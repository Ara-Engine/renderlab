//! M7 G1: the world radiance cache (WRC), sky visibility only (docs/m7-g1.md).
//!
//! World cells keyed like the N8o surface cache (a power-of-two cube by distance plus
//! a normal bin) hold each cell's fixed query point, one visibility bit per fixed
//! direction and the fixed-weight L2 SH of the occluded sky. Shading reads them
//! through `indirect_irradiance` (shaders/gi_api.wgsl). Kernels and layout live in
//! shaders/gi_cells.wgsl and shaders/gi_cache.wgsl; the cache pattern is copied from
//! the shadow surface cache, not shared with it (M7 root decision).
//!
//! Patent design-arounds (docs/m7-p0-claims.md): values are replaced, never counted
//! and averaged (W1, C8); new and invalidated cells are always traced and the rest
//! refresh on a fixed rotation, with no ranking (W2); query points come from the key
//! alone (W2); lookups are unjittered (W3); no resampling anywhere (C1); inserts are
//! atomicMax claims that the winner writes (C4).
use super::*;
use crate::gi_bvh::{self, Blas};
use crate::primary_ray::{TRACED_DRAWS, placement_table};
#[path = "gi_refill_priority.rs"]
mod priority;
#[path = "gi_refill_feedback.rs"]
mod refill_feedback;
#[path = "gi_queue_probe.rs"]
mod queue_probe;

pub(crate) const HEADER_WORDS: u64 = 64;
/// Keep the original admission policy: packing storage must not change the chosen
/// slot count on devices with smaller binding limits. The cold stride is sized below.
const SLOT_WORDS: u64 = 24 + 32;
// P-MEM: the four direction bitsets contain exactly the selected direction count.
pub(super) fn light_cold_offset(directions: u32) -> u32 {
    12 + 4 * (directions / 32) + 3
}
// LC2 light-list plumbing only: cold records end with a visibility bit per light. Keep
// vec4 alignment for the shading view; grow on demand, without a light-count cutoff.
fn light_cold_words(lights: u32, directions: u32) -> u32 {
    (light_cold_offset(directions) + lights.div_ceil(32).max(1)).next_multiple_of(4)
}
/// Work counters, then the per-slot claim words (shaders/gi_cache.wgsl GI_W_CLAIMS).
pub(crate) const COUNTER_WORDS: u64 = 384;
/// Cells the external request list holds (shaders/gi_cache.wgsl GI_EXTERNAL_CAPACITY).
const EXTERNAL_CAPACITY: u64 = 16384;
/// Words of the external list's per-slot bits (shaders/gi_cache.wgsl GI_EXTERNAL_SEEN_WORDS): one
/// bit per slot of the largest table, 2^21.
const EXTERNAL_SEEN_WORDS: u64 = (1 << 21) / 32;
/// The external request list's bytes: 4 header words, 4 words per cell, then the reader's
/// per-slot bits (cells it read, kept alive).
const EXTERNAL_BYTES: u64 = (4 + 4 * EXTERNAL_CAPACITY + EXTERNAL_SEEN_WORDS) * 4;
/// Workgroups of 64 that turn the external list into requests.
const EXTERNAL_GROUPS: u32 = 64;
const MISS_WORDS: u64 = 6;
/// Words per request-table entry: key a, generation band, the chosen key b (shaders/gi_cache.wgsl
/// GI_REQUEST_WORDS).
const REQUEST_WORDS: u64 = 3;
/// WGSL GiFrame: region, counts, offsets, requests, bounce, 256 vec4 box corners, then the
/// boxes' grid: its corner and scale (two vec4) and 512 cells of four mask words.
const FRAME_BYTES: usize = 80 + 256 * 16 + 32 + 512 * 16;
/// The grid's first word in the frame record.
const GRID_WORD: usize = 20 + 256 * 4;
/// Moved-caster boxes a frame holds; more are clustered (cluster_boxes). Glint's play moves
/// about 107 placements a frame: at 32, unions spanned the gaps between movers and every cell
/// inside them was re-probed each frame.
const MAX_BOXES: usize = 128;
/// Claim and resolve repeat in rounds, so collisions retry within the frame.
pub(crate) const CLAIM_ROUNDS: u32 = 3;
/// Ray items per frame (shaders/gi_cache.wgsl GI_RAY_ITEMS): one visibility ray each,
/// traced in parallel. A frame wanting more traces the rest in setup, eight lanes a cell.
pub(crate) const RAY_ITEMS: u32 = 1 << 21;
/// Probe, three claim and resolve rounds, scan, setup, rays, finish and integrate, then
/// bounce's gather, light and cell shading (G2), then the cell-space filter (G3).
const PROFILE_STAMPS: u32 = 2 * (10 + 2 * CLAIM_ROUNDS);
/// Gather workgroups (one cell each) and light workgroups (64 cells), fixed grids like the
/// others.
pub(crate) const GATHER_GROUPS: u32 = 4096;
/// The emitter list (shaders/gi_emitters.wgsl, docs/design-gi-emitters.md): its emitting
/// triangles, 15 words each, then up to EMITTER_DRAWS_MAX emitting draws (stable id, batch, probe
/// radiance, first listed triangle, triangle count, first placement: EMITTER_DRAW_WORDS each),
/// in the table after the slots. Its build pass loops over every emitting draw's triangles on a
/// fixed grid. The list starts with room for EMITTERS_MAX triangles; a build that drops some (the
/// work counters' listed + dropped, read back without waiting) grows it to the next power of two
/// that holds them all, up to EMITTERS_CAP, reallocating the cache once (`grow_emitters`). Glint's
/// 17 emitting draws hold 91,776 triangles: at a fixed 8192 the list held 9% of them, and every
/// hit on the rest found no listed emission.
pub(crate) const EMITTERS_MAX: u32 = 8192;
/// The list's largest capacity (about 21 MB of table and outgoing radiance).
const EMITTERS_CAP: u32 = 1 << 18;
const EMITTER_WORDS: u64 = 15;
const EMITTER_DRAWS_MAX: u32 = 64;
/// Maximum listed emitter triangle count for which the cache's own gathers evaluate emitters
/// analytically (`GI_H_EMITTERS_LIMIT`). Default 64 covers room lights, panels and fixtures so
/// every cache LOD agrees on emitter bounce without random-ray mottling, while huge emitter
/// meshes (e.g. 91k-triangle neon scenes) keep stochastic cache hits. `RENDERLAB_GI_EMITTERS=1`
/// forces analytical gathers for any count (`u32::MAX`), `=0` disables them (`0`).
const GI_DIRECT_EMITTERS_DEFAULT: u32 = 64;
fn emitters_limit() -> u32 {
    static LIMIT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| match std::env::var("RENDERLAB_GI_EMITTERS").as_deref() {
        Ok("1") => u32::MAX,
        Ok("0") => 0,
        Ok(v) => v.parse().unwrap_or(GI_DIRECT_EMITTERS_DEFAULT),
        Err(_) => GI_DIRECT_EMITTERS_DEFAULT,
    })
}
const EMITTER_DRAW_WORDS: u64 = 8;
const EMITTER_GROUPS: u32 = 256;
/// The default per-frame gather budget (`IndirectSettings::gather_cells` 0), in cells, on both
/// paths. G2b sized per-path budgets to about 4 ms of a cut frame's gathers (0.13 us a cell with hardware rays and
/// 0.4 us with software rays on the quarry; docs/m7-g2b.md). Moving frames gather fewer
/// cells than this, so only cuts spread. Software rays had a quarter of the hardware budget,
/// so non-RT bounce took about 100 frames to settle after a cut and lit flat faces showed
/// blotches for the first 8-16+ frames (Ara's Showcase). P-DEFECT gave them the hardware
/// budget (P-GI and P-GATHER made software gathers cheaper since G2b; docs/p-defect.md).
const GATHER_CELLS: u32 = 32768;
/// First fill (gicut, on with hardware rays; `first_fill_on`): the most cells a frame creates
/// (shaders/gi_cache.wgsl gi_fill_bin; after a cut the pixels' parent cells, which cover every
/// receiver, are always created), and software rays' gather budget. Outpost non-RT after a cut: 0.4 us a new cell's
/// sky rays and 1.3 us a gather, so 16k cells and 8k gathers stay near 15 ms over the frame's
/// steady cost; hardware rays: 11 ns and 0.12 us.
const FILL_CELLS_HARDWARE: u32 = 65536;
const FILL_CELLS_SOFTWARE: u32 = 16384;
/// Software rays' gather budget in estimated rays a frame (first fill). Main's steady frames
/// spend at most about 230k (Outpost in motion; Glint's flight peaks at 33k), a cut's frames a
/// median of 880k and up to 1.5M: 2^20 never throttles steady frames and drains a cut at about
/// main's typical rate, without the peaks.
const GATHER_RAYS_SOFTWARE: u32 = 1 << 20;
/// Frames after a cut or clear whose pixels also request their parent cells and whose claims
/// are capped (first fill).
const FILL_PARENT_FRAMES: u32 = 32;
/// Frames after a cut or clear whose claims stay capped: past the parents' window, so the
/// cells deferred until then are not all created on the frame the cap ends.
const FILL_CLAIM_FRAMES: u32 = 64;
/// Whether first fill runs on this path: on with hardware rays, off with software rays (main's
/// claims and gathers; its cut refill there still trades the stall for a longer, dearer tail).
/// `RENDERLAB_GI_FIRST_FILL=1` turns it on for both paths, `=0` off for both.
/// Aimed rays per light a gathering cell holds occluded (shaders/gi_near.wgsl gi_near_gather):
/// `RENDERLAB_GI_NEAR_OCCLUDED=N` (16 or 0 gives every light `near_light_rays`, main's).
fn near_occluded_rays() -> u32 {
    static RAYS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *RAYS.get_or_init(|| {
        std::env::var("RENDERLAB_GI_NEAR_OCCLUDED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(NEAR_OCCLUDED_RAYS)
    })
}
/// Whether shaded cells keep known light visibility (shaders/gi_cache.wgsl gi_light):
/// `RENDERLAB_GI_LIGHT_REUSE=0` re-traces every shaded cell's light rays, as before.
fn light_reuse_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_LIGHT_REUSE").map_or(true, |v| v != "0"))
}
/// Box diagonals around a moved caster within which cells re-trace the directions that cross
/// it (shaders/gi_cache.wgsl gi_reach); RENDERLAB_GI_MOVER_REACH=0 re-traces at any distance.
fn mover_reach() -> f32 {
    static REACH: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *REACH.get_or_init(|| {
        std::env::var("RENDERLAB_GI_MOVER_REACH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(MOVER_REACH)
    })
}
/// One diagonal: Glint's play standing still, non-RT, span 14.1 ms at 4 and 10.8 at 1 with the
/// stale budget and the box grid; live frames at the rings and track showed no difference.
const MOVER_REACH: f32 = 1.0;
/// Turns of a stale cell (shaders/gi_cache.wgsl GI_STALE): one frame in `read` while a pixel
/// reads it, one in `sweep` otherwise. RENDERLAB_GI_STALE=0 re-traces every reached cell at once
/// (both 1); RENDERLAB_GI_STALE_READ and RENDERLAB_GI_STALE_SWEEP set them.
fn stale_periods() -> [u32; 2] {
    static P: std::sync::OnceLock<[u32; 2]> = std::sync::OnceLock::new();
    *P.get_or_init(|| {
        if std::env::var("RENDERLAB_GI_STALE").is_ok_and(|v| v == "0") {
            return [1, 1];
        }
        let get = |name: &str, default: u32| {
            std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default).max(1)
        };
        [get("RENDERLAB_GI_STALE_READ", STALE_READ), get("RENDERLAB_GI_STALE_SWEEP", STALE_SWEEP)]
    })
}
const STALE_READ: u32 = 1;
const STALE_SWEEP: u32 = 16;
/// Stale cells re-traced a frame, about (RENDERLAB_GI_STALE_BUDGET; 0: the fixed periods).
/// RENDERLAB_GI_FIXED=N: the cache spends a fixed N estimated gather rays a frame on cell updates,
/// cells chosen by priority (shaders/gi_cache.wgsl gi_fixed_rank), on both paths (the bench's city
/// and terrain price a hardware ray only 1.2-1.6x below a software one). Unset or 0 keeps the
/// event-driven gather queue.
/// The ara-gpu port does not encode the ranking passes: leave it unset there.
/// RENDERLAB_GI_BORROW=1: readers of a cell's outgoing radiance (gather hits, the final gather,
/// cache-terminated reflections) add, to a cell without a bounce value of its own yet, its albedo
/// times the nearest coarser cell's own bounce (gi_cells.wgsl gi_borrow_*). Off by default;
/// independent of RENDERLAB_GI_FIXED.
/// Cell shading and the cell-space filter follow what changed (gi_cache.wgsl GI_H_TRICKLE): a
/// cell a moved caster reached is shaded only when its light visibility changed, and filtered
/// never (its sky and bounce are unchanged); the reach test ends a local light's ray at the
/// light. Picture-identical; on by default, `RENDERLAB_GI_TRICKLE=0` turns it off.
pub(crate) fn trickle_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_TRICKLE").as_deref() != Ok("0"))
}
/// RENDERLAB_GI_BORROW=0 turns off borrowed coarser bounce for ungathered cells (on by default).
pub(crate) fn borrow_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_BORROW").as_deref() != Ok("0"))
}
/// Default whole-cache ray budgets (gathers + new-cell tracing share the budget).
/// With screen-probe final gather on, pixels never read cache cells directly — probes only sample
/// coarse secondary bounce across a hemisphere, so a smaller cache budget keeps gi_gather fast
/// while holding full visual quality. `RENDERLAB_GI_FIXED=N` overrides (0 restores legacy queue).
const FIXED_RAYS_SOFTWARE: u32 = 98_304;
const FIXED_RAYS_HARDWARE: u32 = 196_608;
const FIXED_RAYS_FG_SOFTWARE: u32 = 98_304;
const FIXED_RAYS_FG_HARDWARE: u32 = 196_608;
pub(crate) fn fixed_rays(software: bool) -> u32 {
    static OVERRIDE: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    let forced = *OVERRIDE
        .get_or_init(|| std::env::var("RENDERLAB_GI_FIXED").ok().and_then(|v| v.parse().ok()));
    forced.unwrap_or_else(|| match (crate::program::final_gather(), software) {
        (true, true) => FIXED_RAYS_FG_SOFTWARE,
        (true, false) => FIXED_RAYS_FG_HARDWARE,
        (false, true) => FIXED_RAYS_SOFTWARE,
        (false, false) => FIXED_RAYS_HARDWARE,
    })
}
/// New-cell tracing shares the fixed ray budget (on by default; `RENDERLAB_GI_FIXED_TRACE=0` off).
fn fixed_trace_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_FIXED_TRACE").as_deref() != Ok("0"))
}
/// Per-cell settling + still-view idle under the fixed budget (on by default; `RENDERLAB_GI_FIXED_REST=0` off).
fn fixed_rest_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_FIXED_REST").as_deref() != Ok("0"))
}
/// Stale cells re-traced per frame from movers (`RENDERLAB_GI_STALE_BUDGET=N`, `RENDERLAB_GI_STALE=0` uncapped).
const STALE_BUDGET_SOFTWARE: u32 = 512;
const STALE_BUDGET_HARDWARE: u32 = 1024;
fn stale_budget(software: bool) -> u32 {
    static OVERRIDE: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    let forced = *OVERRIDE.get_or_init(|| {
        if std::env::var("RENDERLAB_GI_STALE").is_ok_and(|v| v == "0") {
            return Some(0);
        }
        std::env::var("RENDERLAB_GI_STALE_BUDGET").ok().and_then(|v| v.parse().ok())
    });
    forced.unwrap_or(if software { STALE_BUDGET_SOFTWARE } else { STALE_BUDGET_HARDWARE })
}
/// Longest turn of a read stale cell, in frames.
const STALE_TURN_MAX: u32 = 64;
/// Which moved boxes may reach each cell of an 8 x 8 x 8 grid over them (shaders/gi_cache.wgsl
/// gi_box_mask): each box grown by `reach` of its diagonals. The scan tested every slot against
/// every box (Glint's play: about a million slots and 107 boxes a frame); with the grid a slot
/// tests the few boxes near it. None without a reach (every box is tested everywhere).
struct BoxGrid {
    lo: [f32; 3],
    scale: [f32; 3],
    masks: Vec<[u32; 4]>,
}
fn box_grid(boxes: &[[f32; 6]], reach: f32) -> Option<BoxGrid> {
    if reach <= 0.0 || boxes.is_empty() {
        return None;
    }
    let grown: Vec<[f32; 6]> = boxes
        .iter()
        .map(|b| {
            let diagonal = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
            let g = reach * diagonal + 1e-2;
            [b[0] - g, b[1] - g, b[2] - g, b[3] + g, b[4] + g, b[5] + g]
        })
        .collect();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for g in &grown {
        for k in 0..3 {
            lo[k] = lo[k].min(g[k]);
            hi[k] = hi[k].max(g[k + 3]);
        }
    }
    let scale: [f32; 3] = std::array::from_fn(|k| 8.0 / (hi[k] - lo[k]).max(1e-3));
    let cell = |v: f32, k: usize| (((v - lo[k]) * scale[k]).floor() as i32).clamp(0, 7) as usize;
    let mut masks = vec![[0u32; 4]; 512];
    for (i, g) in grown.iter().enumerate() {
        for z in cell(g[2], 2)..=cell(g[5], 2) {
            for y in cell(g[1], 1)..=cell(g[4], 1) {
                for x in cell(g[0], 0)..=cell(g[3], 0) {
                    masks[x + 8 * (y + 8 * z)][i / 32] |= 1 << (i % 32);
                }
            }
        }
    }
    Some(BoxGrid { lo, scale, masks })
}
/// More moved casters than the frame record holds: at most `max` unions of neighbours (the
/// boxes split at the median of their centres along the wider axis, recursively). A single
/// union of every box spanned a level whose movers are spread through it (Glint's obstacles),
/// and re-traced most of the table each frame. RENDERLAB_GI_BOX_CLUSTERS=0 keeps the one union.
fn cluster_boxes(mut boxes: Vec<[f32; 6]>, max: usize) -> Vec<[f32; 6]> {
    fn union(set: &[[f32; 6]]) -> [f32; 6] {
        set.iter().fold(set[0], |u, b| {
            std::array::from_fn(|k| if k < 3 { u[k].min(b[k]) } else { u[k].max(b[k]) })
        })
    }
    fn split(set: &mut [[f32; 6]], groups: usize, out: &mut Vec<[f32; 6]>) {
        if groups <= 1 || set.len() <= 1 {
            out.push(union(set));
            return;
        }
        let centre = |b: &[f32; 6], k: usize| b[k] + b[k + 3];
        let spread = |k: usize| {
            let (lo, hi) = set.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), b| {
                (lo.min(centre(b, k)), hi.max(centre(b, k)))
            });
            hi - lo
        };
        let axis = (0..3).max_by(|&a, &b| spread(a).total_cmp(&spread(b))).unwrap_or(0);
        set.sort_by(|a, b| centre(a, axis).total_cmp(&centre(b, axis)));
        let left = groups / 2;
        let at = (set.len() * left / groups).clamp(1, set.len() - 1);
        let (a, b) = set.split_at_mut(at);
        split(a, left, out);
        split(b, groups - left, out);
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("RENDERLAB_GI_BOX_CLUSTERS").map_or(true, |v| v != "0")) {
        return vec![union(&boxes)];
    }
    let mut out = Vec::with_capacity(max);
    split(&mut boxes, max, &mut out);
    out
}
/// Measurement: with RENDERLAB_GI_MOVED_LOG=<file>, one line per frame with moving casters:
/// how many placements moved, and the first few (index, swept size, largest corner change).
fn moved_log(previous: &[[f32; 6]], bounds: &[[f32; 6]], boxes: &[[f32; 6]]) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let Some(file) = FILE.get_or_init(|| {
        std::env::var_os("RENDERLAB_GI_MOVED_LOG")
            .map(|p| std::sync::Mutex::new(std::fs::File::create(p).expect("moved log")))
    }) else {
        return;
    };
    let mut moved = vec![];
    for i in 0..bounds.len().max(previous.len()) {
        if previous.get(i) != bounds.get(i) {
            let delta = match (previous.get(i), bounds.get(i)) {
                (Some(a), Some(b)) => (0..6).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max),
                _ => f32::INFINITY,
            };
            moved.push((i, delta));
        }
    }
    let sizes: Vec<String> = boxes
        .iter()
        .take(12)
        .map(|b| format!("{:.2}", ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt()))
        .collect();
    let first: Vec<String> = moved.iter().take(12).map(|(i, d)| format!("[{i},{d:.2e}]")).collect();
    let _ = writeln!(
        file.lock().unwrap(),
        "{{\"placements\":{},\"bounds_changed\":{},\"boxes\":{},\"first\":[{}],\"box_diagonals\":[{}]}}",
        bounds.len(),
        moved.len(),
        boxes.len(),
        first.join(","),
        sizes.join(",")
    );
}
/// The default for `near_occluded_rays`.
const NEAR_OCCLUDED_RAYS: u32 = 4;
/// The still-view batch (Planner::finish): one cache frame in STILL_BATCH while a view is still,
/// once STILL_BATCH_SETTLE cache frames have run since the last cut or clear (a cut's backlog
/// drains first). It decided from a read-back count of pending work before, and the readback's
/// timing changed which frames ran: the multi-view test failed on some runs.
/// `RENDERLAB_GI_STILL_BATCH=0` runs every frame.
const STILL_BATCH: u32 = 4;
const STILL_BATCH_SETTLE: u32 = 24;
/// Play loop (sunrt, 2026-11-16): a re-skinned mesh's software geometry is patched in place
/// (`patch_skinned_blas`); `RENDERLAB_GI_SKIN_PATCH=0` repacks every mesh, as before.
/// Play loop (sunrt, 2026-11-16): a re-skinned mesh refits its software tree to each pose
/// (`near_field::refit_bvh`), so the tree keeps its sizes and patches in place; a rebuild cost
/// 6.4 ms per Showcase character per pose and changed the sizes, forcing full repacks.
/// `RENDERLAB_GI_SKIN_REFIT=0` rebuilds it each pose, as before.
fn skin_refit() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_SKIN_REFIT").map_or(true, |v| v != "0"))
}
fn skin_patch() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_SKIN_PATCH").map_or(true, |v| v != "0"))
}
fn still_batch_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_STILL_BATCH").map_or(true, |v| v != "0"))
}
fn first_fill_on(software: bool) -> bool {
    static FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    FORCED
        .get_or_init(|| std::env::var("RENDERLAB_GI_FIRST_FILL").ok().map(|v| v != "0"))
        .unwrap_or(!software)
}
/// `cut_gather_ms` (U-RTVIS): gathers per millisecond of GPU time, software rays (Outpost non-RT
/// after a cut: about 0.95 ms per 1,000, ara-integration perf/table-outpost-view2.md) and
/// hardware rays (assumed four times faster until measured), and the frames after a cut it holds.
const GATHERS_PER_MS_SOFTWARE: f32 = 1000.0;
const GATHERS_PER_MS_HARDWARE: f32 = 4000.0;
const CUT_GATHER_FRAMES: u32 = 64;
/// The per-path budgets of new cells (`IndirectSettings::claim_cells` 0, opt-in): about 3 ms of
/// query points and rays a cut frame at G2b's measured costs (2 px cells: about 60 ns a new
/// cell with hardware rays, 140 ns with software rays; docs/m7-g2b.md).
const CLAIM_CELLS_HARDWARE: u32 = 49152;
const CLAIM_CELLS_SOFTWARE: u32 = 20480;
/// Frames after a cut (a cleared table) whose pixels read cells one level coarser: a quarter of
/// the cells cover the view within the claim budget, and the finer cells that follow fall back
/// to them until they exist (G2b).
const COARSE_CUT_FRAMES: u32 = 4;
/// `cut_refill` (U-RTVIS, docs/design-gi-cut-refill.md): the most frames after a cut that keep
/// the claim cap. The cap defers nothing once a frame's misses fit under it (gi_claim_period),
/// so it ends when the backlog drains; this bounds a scene that never drains.
const CUT_REFILL_FRAMES: u32 = 32;
pub(crate) const LIGHT_GROUPS: u32 = 1024;
/// The cell-space filter's fixed grid (G3): nine items (a changed cell and its eight
/// neighbours) per cell shading queue entry.
pub(crate) const FILTER_GROUPS: u32 = 1024;
/// Unfiltered hot words 1-22 per slot, the filter's sources (shaders/gi_cache.wgsl gi_raw).
const RAW_WORDS: u64 = 22;
/// The fewest slot groups a lighting or caster change spreads relighting over while it keeps
/// changing (and for as many frames after), as ENVIRONMENT_PERIOD does for the sky.
const RELIGHT_PERIOD: u32 = 8;
/// The most slot groups (the shader reads the period and phase as bytes).
const RELIGHT_PERIOD_MAX: u32 = 128;
/// Table slots a continuing change relights a frame by default (`IndirectSettings::relight_cells`
/// 0). With RELIGHT_PERIOD alone a 2^20-slot table relit 131k slots a frame, each reshaded and
/// gathered once, for as long as anything moved or the sun drifted (Outpost's day cycle: GI
/// 9.6 ms RT and 28.7 non-RT, 2026-11-12). Bounce then follows a continuing change within
/// `period` frames (128 on 2^20 slots: about 2 s at 60 Hz); a step after still frames still
/// relights every cell at once.
const RELIGHT_CELLS: u32 = 8192;

/// The relight rotation's period for a table of `slots`: the smallest power of two from
/// RELIGHT_PERIOD to RELIGHT_PERIOD_MAX that holds a slot group to `cells` (0: RELIGHT_CELLS;
/// u32::MAX: RELIGHT_PERIOD, the rate before the budget).
fn relight_period(slots: u32, cells: u32) -> u32 {
    let cells = match cells {
        0 => RELIGHT_CELLS,
        c => c,
    };
    slots
        .div_ceil(cells.max(1))
        .next_power_of_two()
        .clamp(RELIGHT_PERIOD, RELIGHT_PERIOD_MAX)
}
/// Pixels per probe tile side (shaders/gi_cache.wgsl GI_PROBE_TILE).
const PROBE_TILE: u32 = 8;
/// The probe tile side; test builds that screen shader variants (RENDERLAB_GI_SHADERS) may
/// override it with RENDERLAB_GI_PROBE_TILE to match a variant's workgroup.
fn probe_tile() -> u32 {
    #[cfg(test)]
    if let Some(tile) = std::env::var("RENDERLAB_GI_PROBE_TILE")
        .ok()
        .and_then(|t| t.parse().ok())
    {
        return tile;
    }
    PROBE_TILE
}
/// Fixed grids, in 64-thread workgroups, of the passes whose counts only the GPU knows:
/// each kernel reads its count and loops over it. Direct dispatches need no argument
/// passes, and skip the per-dispatch validation pass wgpu adds to indirect dispatches
/// (`InstanceFlags::VALIDATION_INDIRECT_CALL`, on by default); an idle workgroup exits
/// at once. Claims and resolves, setup (eight cells per workgroup), rays (one item per
/// thread), then finish and integrate (sixteen cells per workgroup).
pub(crate) const CLAIM_GROUPS: u32 = 512;
pub(crate) const SETUP_GROUPS: u32 = 4096;
pub(crate) const RAY_GROUPS: u32 = 4096;
pub(crate) const CELL_GROUPS: u32 = 2048;
/// Frames the profile ring holds, and its per-frame stride (resolve offsets are
/// 256-byte aligned).
const PROFILE_RING: u64 = 64;
const PROFILE_STRIDE: u64 = 256;
/// Consecutive identical frames after which a frame runs no cache pass: the orphan-parent
/// chain completes on the third frame (gi_probe's rounds), a split cell's finer levels
/// appear one a frame after it (up to GI_REFINE), and claims lost on the last retry on
/// the next two.
const IDLE_AFTER: u32 = 8;
/// With bounce (G2) the cells a cut or view change makes still gather on later frames: the
/// software refill completes visibility by frame 4, then each cell gathers `gathers` times (and
/// once more if its hits had no value yet), and so do the hit cells its first gather made.
const IDLE_AFTER_BOUNCE: u32 = 16;
/// Slot groups a changing environment map spreads its re-integration over: on frames where
/// the map changed again (a rotating sky), each frame re-integrates the slots with
/// (slot + phase) % ENVIRONMENT_PERIOD == 0, a fixed rotation (W2), and it keeps going for
/// ENVIRONMENT_PERIOD frames after the last change, so every cell is exact again within the
/// 8-frame settle limit. A single change after a still frame re-integrates every cell at
/// once (the next frame equals a fresh cache).
const ENVIRONMENT_PERIOD: u32 = 8;
/// Entries of the frame's request table: twice the miss capacity (a power of two).
fn requests(misses: u32) -> u32 {
    misses * 2
}
/// Word offsets in the work buffer, in its order (`set_indirect` sizes it): the miss list,
/// the trace queue (the integrate queue follows it), the ray items (their staged bits follow),
/// the request table, bounce's two gather queues (a count, then slots) and its cell shading
/// queue (G2).
#[derive(Clone, Copy)]
pub(crate) struct WorkLayout {
    pub(crate) misses: u32,
    pub(crate) traces: u32,
    pub(crate) items: u32,
    pub(crate) requests: u32,
    pub(crate) queues: [u32; 2],
    pub(crate) shade: u32,
}
impl WorkLayout {
    fn of(r: &Resources) -> Self {
        Self::of_counts(r.slots, r.misses)
    }
    pub(crate) fn of_counts(slots: u32, miss_slots: u32) -> Self {
        let misses = COUNTER_WORDS as u32 + slots;
        let traces = misses + miss_slots * MISS_WORDS as u32;
        let items = traces + 2 * slots;
        let requests_at = items + RAY_ITEMS + RAY_ITEMS / 32;
        let queues = requests_at + REQUEST_WORDS as u32 * requests(miss_slots);
        Self {
            misses,
            traces,
            items,
            requests: requests_at,
            queues: [queues, queues + 1 + slots],
            shade: queues + 2 * (1 + slots),
        }
    }
}
/// Blocking readbacks for measurement helpers.
/// The cache's buffers for a table of `slots`: their sizes in bytes, and the counts the
/// sizes follow from.
pub(crate) struct Sizes {
    /// Miss-list entries.
    pub(crate) misses: u32,
    /// The cold record's stride in words, with a visibility bit per light.
    pub(crate) cold_words: u32,
    /// Slots the filter sources hold (1 without a filter).
    pub(crate) raw_slots: u32,
    pub(crate) table: u64,
    pub(crate) work: u64,
    pub(crate) outgoing: u64,
    pub(crate) draws: u64,
    pub(crate) raw: u64,
}
impl Sizes {
    /// `emitters`: the emitter list's capacity, in triangles.
    pub(crate) fn of(slots: u32, lights: u32, settings: &IndirectSettings, emitters: u32) -> Self {
        // A frame lists at most 2^19 misses (a 2 px cut at 1080p makes about 240k cells):
        // the miss list and request table stay at G2's size with the larger tables.
        let misses = slots.min(1 << 19);
        let directions = settings.directions;
        let cold_words = light_cold_words(lights, directions);
        // The slots, then the emitter list (gi_emitters.wgsl): its triangles, then its draws.
        let table_bytes = (HEADER_WORDS
            + u64::from(slots) * (24 + u64::from(cold_words))
            + u64::from(emitters) * EMITTER_WORDS
            + u64::from(EMITTER_DRAWS_MAX) * EMITTER_DRAW_WORDS
            + u64::from(emitters))
            * 4;
        // Counters, claims, misses, trace and integrate queues, ray items and their
        // staged bits, the request table, then bounce's two gather queues (a count, then
        // slots) and the cell shading queue (the frame offsets follow this order).
        let work_bytes = (COUNTER_WORDS
            + u64::from(slots)
            + u64::from(misses) * MISS_WORDS
            + 2 * u64::from(slots)
            + u64::from(RAY_ITEMS + RAY_ITEMS / 32)
            + REQUEST_WORDS * u64::from(requests(misses))
            + 2 * (1 + u64::from(slots))
            + u64::from(slots))
            * 4;
        // The slots' outgoing radiance, then each draw's emission (cell shading's probe).
        // Per slot its outgoing radiance, per draw its emission probe, per listed emitter triangle
        // its mean emission (gi_cells.wgsl gi_emitter_mean_at).
        // With RENDERLAB_GI_BORROW=1, then each slot's borrow record (gi_cells.wgsl gi_borrow_at).
        let borrow = if borrow_on() { u64::from(slots) } else { 0 };
        let outgoing_bytes = (u64::from(slots) + u64::from(TRACED_DRAWS) + u64::from(emitters) + borrow) * 16;
        let draws_bytes = u64::from(TRACED_DRAWS + 1) * 16;
        let raw_slots = if settings.filter != 0 { slots } else { 1 };
        let raw_bytes = u64::from(raw_slots) * RAW_WORDS * 4;
        Self {
            misses,
            cold_words,
            raw_slots,
            table: table_bytes,
            work: work_bytes,
            outgoing: outgoing_bytes,
            draws: draws_bytes,
            raw: raw_bytes,
        }
    }
}
/// The draw table: per draw (batch), its stable id, first placement, instance count and
/// cell-program selector, from `rows` of id, instance count and selector in draw order;
/// the final vec4 carries the active count of `draws`, preserving per-batch indexing.
pub(crate) fn draw_table(draws: usize, rows: impl Iterator<Item = [u32; 3]>) -> Vec<u32> {
    let mut table = vec![0u32; (TRACED_DRAWS as usize + 1) * 4];
    table[TRACED_DRAWS as usize * 4] = draws.min(TRACED_DRAWS as usize) as u32;
    let mut first = 0u32;
    for (b, [id, n, selector]) in rows.enumerate().take(TRACED_DRAWS as usize) {
        table[b * 4..b * 4 + 4].copy_from_slice(&[id, first, n, selector]);
        first += n;
    }
    table
}
const WAIT: wgpu::PollType = wgpu::PollType::Wait {
    submission_index: None,
    timeout: None,
};

/// How a pixel reads cells (W3): the containing cell, or fixed trilinear weights over
/// the eight cells around it. Neither jitters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndirectLookup {
    Nearest,
    #[default]
    Trilinear,
}

/// World radiance cache settings. Off by default at this checkpoint: the standard
/// program then reads the unoccluded environment, as before M7.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndirectSettings {
    pub enabled: bool,
    pub lookup: IndirectLookup,
    /// Target cell edge, in pixels of footprint at the receiver's distance.
    pub cell_pixels: f32,
    /// Edge of level 0 in metres; each level doubles it.
    pub min_edge: f32,
    pub levels: u32,
    /// log2 of the slot count (each slot is 224 bytes in the table, plus 16 of outgoing
    /// radiance and 4 of claim word). With `auto_slots`, the most the table grows to.
    pub slots_log2: u32,
    /// Memory cut 1 (U-FIXED): the table starts at a size from the scene's triangle count
    /// (`scene_slots_log2`) and doubles, emptied, whenever a frame's claims find a bucket full
    /// (the `full` counter, read back without waiting), up to `slots_log2`. Off: the table is
    /// 2^`slots_log2` from the start (the default: each doubling empties the table, three full
    /// refills at Glint's load). `RENDERLAB_GI_SLOTS=N` fixes 2^N either way (A/B).
    pub auto_slots: bool,
    /// Fixed-rotation refresh (W2): a cell re-traces when (slot + frame) % period == 0.
    /// 0 disables it; events alone keep sky visibility current.
    pub rotation_period: u32,
    /// Frames a cell may go unseen before its slot can be reused.
    pub evict_frames: u32,
    /// Trace software rays even when hardware ray queries exist (A/B measurement).
    pub force_software: bool,
    /// W1 value form. 1 replaces all 64 directions on every update (default); S > 1
    /// keeps one slot per direction and re-traces the directions k % S == frame % S,
    /// so a new or invalidated cell settles over S frames (untraced counts as open).
    pub direction_phases: u32,
    /// Exact moved-caster invalidation. Off only to measure the fixed rotation's
    /// staleness alone.
    pub invalidate_casters: bool,
    /// Fixed directions per cell: 64 or 128.
    pub directions: u32,
    /// Half-width, in levels, of the band around each level switch where two levels
    /// mix with a continuous fixed weight (no jitter, W3); 0 switches abruptly.
    pub level_blend: f32,
    /// Rotate each cell's direction set about its normal by an angle its key fixes,
    /// which decorrelates quadrature error between cells (as noise); off, cells with
    /// the same normal share directions, and the error is smooth.
    pub rotate_directions: bool,
    /// Trace visibility rays one per thread (default). Off, each queued cell traces its
    /// rays in one thread (A/B measurement); the bits and values are the same.
    pub parallel_rays: bool,
    /// Cut refill groups S: on the S frames after a reset (`reset_indirect`), a cell
    /// traces one group of its fixed directions (k % S == g, the first untraced group) a
    /// frame, so a cut costs about 1/S of its rays per frame and settles by frame S-1.
    /// Untraced directions take the nearest traced direction's visibility meanwhile. 1
    /// traces every direction of a new cell at once; 0 (the default) picks 4 for software
    /// rays, where a cut costs the most, and 1 for hardware rays (docs/m7-g1b.md).
    pub cut_phases: u32,
    /// Bounce light (M7 G2, docs/m7-g2.md): cells gather the outgoing radiance of the cells
    /// their blocked directions hit, which the frame's program gives through its optional
    /// `scatter`/`emission`. Off, or for programs that declare neither, the cache holds sky
    /// visibility alone, as at G1.
    pub bounce: bool,
    /// Gathers an event schedules for each cell it reaches (1-7): new and re-traced cells,
    /// and every cell at a lighting, environment or program step. Each gather reads the
    /// outgoing radiance the previous ones produced, so G gathers carry G + 1 bounces.
    pub gathers: u32,
    /// The most cells a frame gathers (G2b). When more are queued (a cut queues every visible
    /// cell), a frame gathers those in one fixed slot group (slot % P == frame % P, P the
    /// smallest power of two up to 64 that brings the queue under this count) and the rest
    /// wait, so the cut's bounce work is bounded per frame rather than finished in the fewest
    /// frames. 0 picks the default budget (GATHER_CELLS);
    /// u32::MAX gathers every queued cell each frame.
    pub gather_cells: u32,
    /// The most table slots a continuing lighting or caster change relights a frame (reshaded,
    /// then gathered once): the relight rotation's period is the table's slots over this, a
    /// power of two from 8 to 128. 0 picks the default (RELIGHT_CELLS, 8192); u32::MAX keeps
    /// the period at 8, the rate before the budget.
    pub relight_cells: u32,
    /// Gather stride S (G2b; 1, 2 or 4): a gather traces the blocked directions k % S == 0 of
    /// the cell's fixed set, each weighted S times a direction's solid angle, so it costs 1/S of
    /// the rays. Bounce light is smoother than sky visibility; the subset is fixed per cell.
    pub gather_stride: u32,
    /// The most cells a frame creates (G2b). When more are missing (a cut), a frame creates
    /// those of one fixed key phase and the rest wait, with the first COARSE_CUT_FRAMES frames
    /// after a cut reading cells one level coarser, so a cut's query points and rays are bounded
    /// per frame. 0 picks the per-path budget (CLAIM_CELLS_HARDWARE, CLAIM_CELLS_SOFTWARE);
    /// u32::MAX (the default) creates every missing cell at once and reads no coarser cut level:
    /// on the SV test scene the coarse-to-fine handover settled more slowly than it saved
    /// (docs/m7-g2b.md).
    pub claim_cells: u32,
    /// Cell-space filter (G3, docs/m7-g3.md), with bounce on: bit 0 filters each cell's sky
    /// value, bit 1 its bounce value, over itself and its eight neighbours in its bin's tangent
    /// plane at its level, with fixed weights (`filter_edge`) times geometric ones (query-normal
    /// agreement, distance from the query plane). One pass, no jitter or sample counts. The
    /// unfiltered values (88 bytes a slot) stay the filter's sources. 0 is off. Bit 2 (with bit 0)
    /// marks a cell much darker than an agreeing neighbour split, so its pixels read the next
    /// finer level (contrast refine, for crevice mouths). Bit 3 (with bit 1) gives cells not
    /// gathered yet their gathered neighbours' filtered bounce.
    pub filter: u32,
    /// The filter's tent: weight 1 at the centre, this on the four edge neighbours and its
    /// square on the four corners (0-1).
    pub filter_edge: f32,
    /// Rays a gather aims around each local light in range (U-SPECK, docs/speck.md; 0-64).
    /// The fixed directions cannot resolve a small surface close to a lamp, whose radiance
    /// grows as the inverse square of its distance to the light: the few cells whose
    /// direction met it showed as bright blobs. With this set, hits nearer a light than a
    /// third of the cell's distance to it are left to these rays, which sample that region
    /// where it is bright; the two sets estimate the same integral. 0 turns it off.
    pub near_light_rays: u32,
    /// A miss whose bucket has no free slot takes the bucket's least recently seen cell
    /// (U-SPECK, docs/speck.md): a pixel's cell one unseen for two frames, a gather's hit cell
    /// one unseen for 16 (or 32) frames. Without it a table that fills before `evict_frames`
    /// frees anything, as a moving camera in a large scene fills it, leaves new cells
    /// uncreated and their pixels on lone or coarser cells: a blocky mosaic. Off only to
    /// measure.
    pub pressure_eviction: bool,
    /// Cells across a gather ray's cone at its hit (U-SPECK, docs/speck.md; 0, or 1 to 64). A
    /// fixed direction stands for a cone about 0.36 of the hit's distance wide (64 directions),
    /// so a hit reads and requests a cell that many to the cone's width, never finer than a
    /// pixel there would read. 0 gives every hit the pixels' level, as before: those cells
    /// were most of the table.
    pub hit_cone_cells: f32,
    /// When more cells wait for a gather than a frame's budget (`gather_cells`), the cells
    /// pixels read this frame rotate over the budget alone and the rest share a quarter of it
    /// more; and the rotation's period is the smallest that fits the budget, not the next
    /// power of two (U-SPECK, docs/speck.md). Off (the default), every queued cell waits its
    /// turn alike: after a start or a cut the picture then shows cells without bounce light
    /// beside gathered ones for as long as the whole queue takes.
    pub gather_pixels_first: bool,
    /// Gathers a cell made for a gather's hit gets (GI_SECONDARY, GI_TERTIARY; U-RTVIS). Pixel
    /// cells keep `gathers`. A hit cell feeds other cells' bounce, not the picture directly, and
    /// its own gathers request further cells: at `gathers` (3) a cut's queue fanned out to about
    /// 330k gathers on Outpost's second view and drained only by frame 119. The default is 1
    /// (2026-11-13, RTX on Ara: Outpost's backlog empty at frame 95 instead of 113, 12% off the
    /// post-cut frames; settled pictures equal on Outpost and Glint, within 0.03 and 0.22 of 255).
    /// 0 gives them `gathers`, as before (`RENDERLAB_GI=hit_gathers=0` for the A/B).
    pub hit_gathers: u32,
    /// After a camera cut (U-RTVIS), for CUT_GATHER_FRAMES frames: gathers capped at about this
    /// many milliseconds a frame (GATHERS_PER_MS_*), the cells pixels read first. 0 (the default
    /// until gated) leaves `gather_cells`.
    pub cut_gather_ms: f32,
    /// A new cell takes its parent level's bounce value until its own first gather, so a cell
    /// a capped gather has not reached reads a coarser estimate instead of none (U-RTVIS).
    pub parent_standin: bool,
    /// Levels above the pixels' that a gather hit on something that moved this frame reads
    /// at least (U-SPECK, docs/speck.md; 0 to 4, 0 off). A moving surface leaves a fine cell
    /// before the cell has a value, so its hits found none and it sent no bounce light.
    pub mover_hit_levels: u32,
    /// A camera cut refills like a cleared table (root, docs/design-gi-cut-refill.md; off by
    /// default until measured): until the cut's backlog of missing cells drains (at most
    /// CUT_REFILL_FRAMES frames after `Planner::camera_cut` or a clear), new cells are capped at
    /// the per-path claim budget (CLAIM_CELLS_*); each new cell traces all its directions on
    /// the frame it is claimed (no `cut_phases` spread: partial cells converged visibly slower).
    /// Pixels read their own level throughout; a missing cell takes the lookup's usual
    /// fallbacks. Before this, a camera
    /// cut through `cut_view` kept the table, and Outpost's one cut created 193k cells in one
    /// frame (non-RT gi_update 3 -> 145 ms).
    pub cut_refill: bool,
}
impl Default for IndirectSettings {
    /// On by default since G2b (Islam's overnight push, 2026-11-08; docs/m7-g2b.md): G2's
    /// measured configuration (bounce and ES1's blocked sky specular, 4 px cells, every blocked
    /// direction, three gathers an event) with a per-frame gather cap that bounds camera cuts.
    /// 2 px cells fix the quarry's crevice mouths but speckle flat test-scene surfaces and need a
    /// 448 MiB table, over the non-RT device's 256 MiB buffer limit: an opt-in setting
    /// (`cell_pixels` 2, `slots_log2` 21) where the device allows it.
    /// 2^20 slots since GC1 (224 MiB): at 2^19 the quarry's cells filled buckets, so claims
    /// failed (about 280 a moving frame on RT, 1,100 on non-RT) and lookups walked long buckets;
    /// the larger table cut the quarry's moving GI cost by 0.3-0.5 ms (RT) at the same image. A
    /// device whose storage-binding limit is wgpu's default 128 MiB gets 2^19 (`gi_slots`;
    /// docs/m7-gc1.md).
    /// The cell-space filter with contrast refine and bounce fill since G3 (`filter` 15, 88 more
    /// bytes a slot): quarry TR1x64 pinholes RT 142 -> 65 and non-RT 122 -> 68 (cache-off 74),
    /// every quarry population better on both paths, the cave's mottle and the hollow's streaks
    /// mostly gone (docs/m7-g3.md).
    fn default() -> Self {
        Self {
            enabled: true,
            lookup: IndirectLookup::Trilinear,
            cell_pixels: 4.0,
            min_edge: 1.0 / 512.0,
            levels: 14,
            slots_log2: 20,
            auto_slots: false,
            rotation_period: 0,
            evict_frames: 120,
            force_software: false,
            direction_phases: 1,
            invalidate_casters: true,
            directions: 64,
            // Unrotated, cells with the same normal share their quadrature error, which
            // shows as faint cell-sized rectangles on flat faces; rotated, it becomes a
            // fine fixed mottle (SV frame error 1.93% -> 1.64%), with no extra rays.
            rotate_directions: true,
            level_blend: 0.0,
            parallel_rays: true,
            cut_phases: 0,
            bounce: true,
            gathers: 3,
            gather_cells: 0,
            relight_cells: 0,
            gather_stride: 1,
            claim_cells: u32::MAX,
            // G3 (docs/m7-g3.md): sky and bounce filtered, contrast refine and bounce fill.
            filter: 15,
            filter_edge: 0.5,
            // The CPU replay (tools/speck/replay.py): 16 keep the mean within 1 % and remove
            // the blobs; 8 leave the lintel's bounce 8 % high.
            near_light_rays: 16,
            pressure_eviction: true,
            // Against the path tracer (docs/speck.md section 8): the field walk's error is the
            // same at 0, 16, 8 and 4; the walking hero moves toward the reference as the
            // cells grow, most at 4; and the field walk creates half the cells.
            hit_cone_cells: 4.0,
            // Off until judged on a large scene (docs/speck.md section 9): on the field pose
            // it halves frame 16's distance from the settled frame on RT, for a quarter more
            // gathers over the flight, and non-RT's first eight frames are slightly worse.
            gather_pixels_first: false,
            hit_gathers: 1,
            cut_gather_ms: 0.0,
            parent_standin: false,
            // Off: at 2 the path tracer's walking hero is 1.5-1.9 codes closer underneath at one
            // frame and 0.9-1.4 codes farther at another, and the game scene's spinning cube
            // gains isolated specks (docs/speck.md section 10).
            mover_hit_levels: 0,
            cut_refill: false,
        }
    }
}
impl IndirectSettings {
    /// The cache off: programs read the unoccluded environment, as before M7.
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let valid = self.cell_pixels.is_finite()
            && self.cell_pixels > 0.0
            && self.min_edge.is_finite()
            && self.min_edge > 0.0
            && (1..=24).contains(&self.levels)
            && (10..=21).contains(&self.slots_log2)
            && self.evict_frames != 0
            && (1..=64).contains(&self.direction_phases)
            && [64, 128].contains(&self.directions)
            && (0.0..=0.5).contains(&self.level_blend)
            && self.cut_phases <= 8
            && (1..=7).contains(&self.gathers)
            && [1, 2, 4].contains(&self.gather_stride)
            && self.filter <= 15
            && (0.0..=1.0).contains(&self.filter_edge)
            && self.near_light_rays <= 64
            && self.mover_hit_levels <= 4
            && (self.hit_cone_cells == 0.0 || (1.0..=64.0).contains(&self.hit_cone_cells));
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidInput("invalid indirect settings"))
        }
    }
}

/// Cache resources and the last frame's work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndirectStats {
    pub table_bytes: u64,
    pub work_bytes: u64,
    /// Software-ray geometry (non-RT path), near-field masks included.
    pub geometry_bytes: u64,
    /// Of which cooked near-field masks.
    pub near_bytes: u64,
    /// Instances whose transform matched a cooked near field in the last software frame.
    pub near_instances: u32,
    /// Software-ray instances (U-RTVIS): records the GI cache's rays may visit, the merged
    /// near-field world counting as one. Each ray tested every one before the top level.
    pub instances: u32,
    /// Frames that ran the cache passes.
    pub frames: u64,
    /// The last such frame traced software rays.
    pub software: bool,
    /// Frames whose unchanged inputs ran no cache pass.
    pub idle_frames: u64,
    /// Cache frames that ran the bounce passes (G2).
    pub bounce_frames: u64,
    /// The outgoing-radiance, draw-table and queue buffers bounce adds (in `work_bytes`
    /// for the queues), in bytes.
    pub bounce_bytes: u64,
    /// The cell-space filter's unfiltered sources (G3), in bytes.
    pub filter_bytes: u64,
    /// The table's slots, the times `auto_slots` doubled it, and the largest per-frame
    /// bucket-full count read back since the last doubling.
    pub slots: u32,
    pub slot_doublings: u32,
    pub peak_full: u32,
}

/// GPU microseconds per cache pass of the last profiled frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IndirectProfile {
    pub probe: f64,
    pub claim: f64,
    pub resolve: f64,
    pub scan: f64,
    /// Tracing: query points and ray items, the rays, then the bits and SH per cell.
    pub setup: f64,
    pub rays: f64,
    pub finish: f64,
    pub integrate: f64,
    /// Bounce (G2): gathers, light visibility, then the program's cell shading.
    pub gather: f64,
    pub light: f64,
    pub shade: f64,
    /// The cell-space filter (G3).
    pub filter: f64,
}
impl IndirectProfile {
    pub fn trace(&self) -> f64 {
        self.setup + self.rays + self.finish
    }
    pub fn bounce(&self) -> f64 {
        self.gather + self.light + self.shade
    }
    pub fn total(&self) -> f64 {
        self.probe
            + self.claim
            + self.resolve
            + self.scan
            + self.trace()
            + self.integrate
            + self.bounce()
            + self.filter
    }
}

/// The kernels' counters after a frame (test readback).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndirectCounters {
    pub misses: u32,
    pub traces: u32,
    pub integrates: u32,
    pub miss_overflow: u32,
    pub full: u32,
    pub orphans: u32,
    pub local_overflow: u32,
    pub created: u32,
    pub invalidated: u32,
    pub refreshed: u32,
    /// Claims that lost their slot this frame (they retry next round or frame).
    pub lost: u32,
    /// Visibility rays traced this frame (probe rays excluded).
    pub rays: u32,
    /// Rays requested as parallel items; past RAY_ITEMS the rest trace in setup.
    pub ray_items: u32,
    /// Orphans whose candidates found no qualifying surface, and orphans whose every
    /// qualifying candidate lay inside a solid.
    pub orphans_no_surface: u32,
    pub orphans_inside: u32,
    /// Cells probed this frame whose fixed probe lines each met two surface layers
    /// (shaders/gi_cache.wgsl gi_probe_line); their pixels read finer levels.
    pub splits: u32,
    /// The probe's work, counted on profiled frames only (zero otherwise).
    pub probe: ProbeCounters,
    /// Bounce (G2): cells shaded, cells gathered, their rays, rays whose hit cell had no
    /// value yet, hit cells they requested, light visibility rays, and cells queued for a
    /// gather next frame.
    pub shades: u32,
    pub gathers: u32,
    pub gather_rays: u32,
    pub gather_misses: u32,
    pub gather_requests: u32,
    pub light_rays: u32,
    pub bounce_queued: u32,
    /// Gather rays without a value, by cause: no cell at the hit's level (requested), a
    /// cell not shaded yet, and an orphan whose coarser levels had no value either.
    pub gather_absent: u32,
    /// The emitter list (shaders/gi_emitters.wgsl): triangles listed, emitting draws, visibility
    /// rays the gathers traced toward them, and triangles past the list's capacity.
    pub emitters: u32,
    pub emitter_draws: u32,
    pub emitter_rays: u32,
    pub emitters_dropped: u32,
    /// Listed triangles without an evaluated mean (their draw's probe stands in), and listed
    /// triangles of emitting draws whose radiance is zero.
    pub emitters_unevaluated: u32,
    pub emitters_zero: u32,
    pub gather_unshaded: u32,
    pub gather_orphans: u32,
    /// Gather rays a coarser level answered because the hit level's cell is an orphan, and
    /// because it is absent (gi_hit_radiance).
    pub gather_coarse_orphan: u32,
    pub gather_coarse_absent: u32,
    /// Queued cells a capped gather frame left for the next (`IndirectSettings::gather_cells`).
    pub gather_deferred: u32,
    /// Listed misses a capped claim left for a later frame (`IndirectSettings::claim_cells`).
    pub claim_deferred: u32,
    /// Cells readers outside the cache listed last frame (the final gather's probe hits), and
    /// those past the list's capacity.
    pub external_requests: u32,
    pub external_overflow: u32,
    /// The external list's two reader counters (words 2 and 3): the final gather's rays and
    /// shadow rays, when it counts them.
    pub fg_rays: u32,
    pub fg_shadow_rays: u32,
    /// Cells a full bucket gave up to a miss before their eviction age (U-SPECK,
    /// docs/speck.md), and those of them seen within the last eight frames.
    pub evicted: u32,
    pub evicted_recent: u32,
    /// Pixels' misses that found no slot, once per claim round (`full` counts every band's).
    pub full_pixel: u32,
    /// The gather ray budget (first fill): the queue's estimated rays, and the cuts (1 + the
    /// first bin left for later; 0 none) for pixels' cells and the rest.
    pub gather_estimate: u32,
    pub gather_cuts: [u32; 2],
    /// Gathers' aimed rays, and those at lights the cell holds occluded.
    pub aimed_rays: u32,
    pub aimed_occluded: u32,
    /// Cells created in the secondary and tertiary bands, and live secondary cells evicted.
    pub created_secondary: u32,
    pub created_tertiary: u32,
    pub evicted_secondary: u32,
    /// Moved-caster boxes the scan tested this frame (after clustering).
    pub moved_boxes: u32,
    /// Cells movers re-traced, read by a pixel this frame / last frame; cells movers reshaded
    /// (a light ray crossed), all of them / read this frame / last frame.
    pub invalidated_read: [u32; 2],
    /// Of the cells movers re-traced, those whose cube a moved box overlaps (fully re-probed).
    pub invalidated_cube: u32,
    pub reshaded: [u32; 3],
    /// Of the cells a moved caster reached, those whose light visibility came out unchanged, so
    /// cell shading skipped them (`RENDERLAB_GI_TRICKLE`, gi_cache.wgsl GI_SHADE_SKIP).
    pub reshade_skipped: u32,
    /// Cells queued for cell shading by why (shaders/gi_cache.wgsl gi_queue_shade_cause, once a
    /// cell): after a trace, the sky filter, a relight, a mover crossing a light ray, after a gather.
    pub shade_causes: [u32; 5],
    /// Cells the scan re-traced because they were untraced or partial (`created`, `invalidated`
    /// and `refreshed` count the other traces).
    pub trace_partial: u32,
}

/// What the probe did in one profiled frame (shaders/gi_cache.wgsl GI_W_P_*): tiles with a
/// covered pixel, covered pixels, tile-table insert calls and their probe steps, unique cells
/// from pixels, added for orphans' parents and by refinement, table finds and how many found
/// a cell, refinement walks (pixel, bin and level), tiles that refined, the most unique cells
/// in one tile, and tiles by unique cells (< 32, < 64, < 256, full).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeCounters {
    pub tiles: u32,
    pub pixels: u32,
    pub inserts: u32,
    pub steps: u32,
    pub cells: u32,
    pub parents: u32,
    pub refined: u32,
    pub finds: u32,
    pub found: u32,
    pub walks: u32,
    pub refine_tiles: u32,
    pub max_cells: u32,
    pub tiles_by_cells: [u32; 4],
}

struct Resources {
    table: Owned<wgpu::Buffer>,
    work: Owned<wgpu::Buffer>,
    frame: Owned<wgpu::Buffer>,
    /// Bounce (G2): per slot, its outgoing radiance (vec4, w 1 once written); per draw, its
    /// stable id, first placement and instance count (shaders/gi_cache.wgsl gi_draws).
    outgoing: Owned<wgpu::Buffer>,
    draws: Owned<wgpu::Buffer>,
    /// The cell-space filter's sources (G3): RAW_WORDS per slot while a filter is set, else
    /// one slot's.
    raw: Owned<wgpu::Buffer>,
    /// Cells readers outside the cache met without a value (the final gather's probe hits),
    /// requested on the next frame (shaders/gi_cache.wgsl gi_external_requests).
    external: Owned<wgpu::Buffer>,
    slots: u32,
    cold_words: u32, // LC2: dynamic per-cell light bitset stride.
    directions: u32,
    misses: u32,
    /// Slots the filter sources hold (1 without a filter).
    raw_slots: u32,
    /// The emitter list's capacity, in triangles.
    emitters: u32,
    _allocations: [Allocation; 6],
}
struct Layouts {
    /// Hardware (TLAS at 34) and software (geometry at 35).
    main: [Owned<wgpu::BindGroupLayout>; 2],
    arrays: Owned<wgpu::BindGroupLayout>,
    pipelines: [Owned<wgpu::PipelineLayout>; 2],
    /// Group 1 without the per-draw vertex and index arrays, and the pipeline layouts with it:
    /// for the kernels that never read those arrays (`lean`).
    lean_arrays: Owned<wgpu::BindGroupLayout>,
    lean_pipelines: [Owned<wgpu::PipelineLayout>; 2],
    /// Cell shading's group 0 (G2): the work buffer read-only, the outgoing radiance and
    /// the draw table.
    shade: Owned<wgpu::BindGroupLayout>,
}
struct Pipelines {
    software: bool,
    priority: Option<priority::Pipelines>,
    directions: u32,
    probe: Owned<wgpu::ComputePipeline>,
    external: Owned<wgpu::ComputePipeline>,
    claim: Owned<wgpu::ComputePipeline>,
    resolve: Owned<wgpu::ComputePipeline>,
    resolve_last: Owned<wgpu::ComputePipeline>,
    scan: Owned<wgpu::ComputePipeline>,
    setup: Owned<wgpu::ComputePipeline>,
    rays: Owned<wgpu::ComputePipeline>,
    /// Software-only specialization with `GI_SW_SEE_THROUGH = true` for scenes with static
    /// see-through draws; opaque scenes run `rays`/`gather` with the branch folded out.
    rays_clear: Option<Owned<wgpu::ComputePipeline>>,
    finish: Owned<wgpu::ComputePipeline>,
    integrate: Owned<wgpu::ComputePipeline>,
    gather: Owned<wgpu::ComputePipeline>,
    gather_clear: Option<Owned<wgpu::ComputePipeline>>,
    fill_count: Owned<wgpu::ComputePipeline>,
    fill_select: Owned<wgpu::ComputePipeline>,
    emitters_build: Owned<wgpu::ComputePipeline>,
    emitters_publish: Owned<wgpu::ComputePipeline>,
    emitters_cdf: Owned<wgpu::ComputePipeline>,
    gather_count: Owned<wgpu::ComputePipeline>,
    gather_select: Owned<wgpu::ComputePipeline>,
    /// The fixed update budget's ranking, cut, listing and convergence check (shaders/gi_cache.wgsl gi_fixed_*).
    fixed_rank: Owned<wgpu::ComputePipeline>,
    fixed_select: Owned<wgpu::ComputePipeline>,
    fixed_list: Owned<wgpu::ComputePipeline>,
    fixed_after: Owned<wgpu::ComputePipeline>,
    light: Owned<wgpu::ComputePipeline>,
    filter: Owned<wgpu::ComputePipeline>,
    /// The kernels built on the lean layout (`lean`): their passes bind the lean group 1.
    lean: Vec<wgpu::ComputePipeline>,
}

/// The cache's kernels that never read group 1's per-draw vertex and index arrays (all but the
/// probe and the hit decodes) are built on a group 1 without them and bind it: every dispatch
/// merges each resource of its bound groups into wgpu's usage tracking when the command buffer
/// finishes, and those two arrays are TRACED_DRAWS (256) buffers each, padded. The same
/// kernels, inputs and results. `RENDERLAB_GI_LEAN=0` binds the full group everywhere.
fn lean() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_LEAN").map_or(true, |v| v != "0"))
}
/// One program's cell shading pipeline (G2), keyed by everything its module and layout
/// depend on: the program and its source, the prelude, the ray path and the material
/// group's layout (streamed or not).
/// One program's cell shading pipeline: `key` is (program, its source and the prelude's hash,
/// software tracing, streamed textures); `mixed` selects the entry that shades only its own
/// draws' cells (`mixed_cell_entry`, MP1).
struct ShadePipeline {
    key: (u64, [u8; 32], bool, bool),
    mixed: bool,
    pipeline: Owned<wgpu::ComputePipeline>,
}
/// Group 1: the traced shading ABI's geometry tables, built from the same draws in
/// TLAS placement order, so hit records decode to the receivers shading sees.
struct Arrays {
    /// Group 1 without the vertex and index arrays, for the lean kernels (`lean`).
    lean: Owned<wgpu::BindGroup>,
    keys: Vec<(wgpu::Buffer, wgpu::Buffer, usize)>,
    models: Owned<wgpu::Buffer>,
    normals: Owned<wgpu::Buffer>,
    _placements: Owned<wgpu::Buffer>,
    _absent: Owned<wgpu::Buffer>,
    /// The placements the buffers hold, by name: each draw's collection and accepted-edit count
    /// (`InstanceState::revision`), and the same as bytes for the planner's idle keys. Equal
    /// revisions mean equal transforms, so nothing is packed, compared or hashed per matrix.
    revisions: Vec<(u64, u64)>,
    revision_bytes: Vec<u8>,
    bind: Owned<wgpu::BindGroup>,
}
/// Software-ray geometry: every draw's mesh records and near masks, and the flat instance
/// list. With a merged world (`World`), the world BVH is the last packed mesh and the draws
/// whose instances it covers pack nothing.
struct Packed {
    meshes: Vec<wgpu::Buffer>,
    instances: usize,
    /// Per (draw, instance) in draw order, whether the world BVH covers it (empty: no world).
    covered: Vec<bool>,
    /// The draws' stable ids it was packed for, and the world's placement map (word offset,
    /// placements), G2.
    ids: Vec<u32>,
    map: (u32, u32),
    /// Per draw, its mesh's index among the packed meshes (None: every instance is covered).
    entry: Vec<Option<usize>>,
    offsets: Vec<[u32; 4]>,
    /// Per draw, the base word of each of its mesh's cooked near placements' masks.
    near: Vec<Vec<u32>>,
    near_bytes: u64,
    head: Vec<u32>,
    buffer: Owned<wgpu::Buffer>,
    bytes: u64,
    _allocation: Allocation,
}
/// A cooked near field's placements merged into one world-space BVH (non-RT, G1d), built at
/// `set_indirect_near_field` from fields without masks: every placement's far-field cut in
/// world space. While each placement is drawn with its exact cooked transform (and nothing
/// else draws a field mesh), software rays trace it as one identity instance in place of the
/// placements' own BVHs: the quarry's twelve scans overlap, so a ray otherwise descends each
/// scan's BVH in turn. Any other instance set falls back to per-placement BVHs.
struct World {
    blas: Arc<Blas>,
    source: [u8; 32],
    /// Each placement's field mesh and transform bits.
    placements: Vec<(u32, [u32; 16])>,
    /// Per field mesh, the renderer mesh (vertex buffer) registered for it.
    meshes: Vec<Option<wgpu::Buffer>>,
    /// Per placement, its first position in the merged BVH: a software hit's position index
    /// names the placement whose material it shades (G2).
    bases: Vec<u32>,
}
impl World {
    fn new(field: &crate::asset::near_field::NearField) -> Result<Self, Error> {
        let mut positions = vec![];
        let mut indices = vec![];
        let mut bases = vec![];
        for p in &field.placements {
            let model = Mat4::from_cols_array_2d(&p.transform);
            let mesh = field
                .meshes
                .get(p.mesh as usize)
                .ok_or(Error::InvalidInput("near-field placement mesh"))?;
            let base = positions.len() as u32;
            bases.push(base);
            positions.extend(mesh.positions.iter().map(|q| {
                model
                    .transform_point3(glam::Vec3::new(q[0], q[1], q[2]))
                    .to_array()
            }));
            // A mirroring transform reverses the winding the source-side tests read.
            let flip = model.determinant() < 0.0;
            for t in mesh.indices.chunks_exact(3) {
                let (b, c) = if flip { (t[2], t[1]) } else { (t[1], t[2]) };
                indices.extend_from_slice(&[base + t[0], base + b, base + c]);
            }
        }
        Ok(Self {
            blas: Arc::new(Blas::new(gi_bvh::build(&positions, &indices)?, 0)?),
            source: field.source,
            placements: field
                .placements
                .iter()
                .map(|p| (p.mesh, transform_bits(&p.transform)))
                .collect(),
            meshes: vec![None; field.meshes.len()],
            bases,
        })
    }
    /// Per (draw, instance) of `draws`, whether the world covers it, when its placements
    /// are exactly the drawn instances of the field's meshes, and per placement the (draw,
    /// instance) it is; None otherwise.
    #[allow(clippy::type_complexity)]
    fn covers(&self, draws: &[PreparedDraw<'_>]) -> Option<(Vec<bool>, Vec<(usize, usize)>)> {
        let mut wanted: Vec<Option<(&wgpu::Buffer, [u32; 16])>> = self
            .placements
            .iter()
            .map(|(m, bits)| Some((self.meshes.get(*m as usize)?.as_ref()?, *bits)))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .map(Some)
            .collect();
        let mut covered = vec![];
        let mut drawn = vec![(0, 0); wanted.len()];
        for (di, d) in draws.iter().enumerate() {
            let key: &wgpu::Buffer = &d.mesh.vertex;
            let field_mesh = self.meshes.iter().any(|m| m.as_ref() == Some(key));
            for (ii, model) in d.instances.state.models.iter().enumerate() {
                let bits = model.to_cols_array().map(f32::to_bits);
                let hit = wanted
                    .iter()
                    .position(|w| w.is_some_and(|(k, b)| k == key && b == bits));
                match hit {
                    // The merged world has no per-placement shadow bit: a covered placement
                    // that casts no shadow keeps every placement on its own record.
                    Some(_) if !d.instances.state.casts_shadows(ii) => return None,
                    Some(at) => {
                        wanted[at] = None;
                        drawn[at] = (di, ii);
                        covered.push(true);
                    }
                    // A field mesh drawn elsewhere: its BVH would double the world's.
                    None if field_mesh => return None,
                    None => covered.push(false),
                }
            }
        }
        wanted
            .iter()
            .all(Option::is_none)
            .then_some((covered, drawn))
    }
}
/// A placement's material identity + 1 (shaders/gi_cells.wgsl GI_C_MATERIAL): its draw's
/// stable id and its instance; 0 (none: it reflects nothing) when either does not fit.
fn draw_identity(ids: &[u32], draw: usize, instance: usize) -> u32 {
    match (ids.get(draw), u32::try_from(instance)) {
        (Some(&id), Ok(i)) if id != 0 && id < 0xffff && i < 0x10000 => ((id << 16) | i) + 1,
        _ => 0,
    }
}
/// A placement transform's exact bits, column-major (as `Mat4::to_cols_array`).
fn transform_bits(t: &[[f32; 4]; 4]) -> [u32; 16] {
    Mat4::from_cols_array_2d(t)
        .to_cols_array()
        .map(f32::to_bits)
}
struct Groups {
    key: (wgpu::Texture, wgpu::Texture, u64, bool),
    main: Owned<wgpu::BindGroup>,
    /// Cell shading's group 0 (G2).
    shade: Owned<wgpu::BindGroup>,
}
/// Pass timestamps of the last PROFILE_RING cache frames, resolved per frame into a
/// ring and read back only on request, so measured frames never wait for the CPU.
struct Profile {
    queries: Owned<wgpu::QuerySet>,
    resolve: Owned<wgpu::Buffer>,
    readback: Owned<wgpu::Buffer>,
    period: f32,
    /// Cache frames profiled so far.
    written: u64,
    stamps: u32,
    stride: u64,
    priority: bool,
}
/// What `prepare_gi` decided for this frame.
pub(crate) struct Plan {
    pub(crate) groups: [u32; 2],
    /// Scan workgroups: every slot, or one environment slot group.
    pub(crate) scan_groups: u32,
    pub(crate) scan: bool,
    /// The environment changed: cells re-integrate their stored bits.
    pub(crate) integrate: bool,
    pub(crate) clear: bool,
    /// Only the environment changed, with the view and scene otherwise idle: only the scan
    /// and integrate passes run (and bounce's, which relight).
    pub(crate) environment_only: bool,
    /// Bounce (G2) runs this frame: gathers, light visibility and cell shading. The work
    /// offset (in bytes) of next frame's gather queue count, cleared before the passes.
    pub(crate) bounce: bool,
    pub(crate) next_queue_at: u64,
    /// The cell-space filter (G3) runs this frame (bounce on, a filter bit set).
    pub(crate) filter: bool,
    /// Fixed whole-cache ray budget active for this frame (0: legacy event queue).
    pub(crate) fixed_rays: u32,
    /// Rebuild the emitter list after cell shading (cold frame 1, clear, draw/emission changes)
    /// so final gather sees newly probed/edited emitters on the same frame with zero lag.
    pub(crate) post_emitters: bool,
}
/// What a frame's plan is computed from, apart from the planner's own history: the
/// settings, the allocated table's shape, and the view.
pub(crate) struct PlanConfig {
    pub(crate) settings: IndirectSettings,
    /// Cell slots, miss slots, the cold record's stride and the filter-source slots of
    /// the allocated table.
    pub(crate) slots: u32,
    pub(crate) misses: u32,
    pub(crate) cold_words: u32,
    pub(crate) raw_slots: u32,
    /// The emitter list's capacity, in triangles.
    pub(crate) emitters: u32,
    /// Software rays (the non-RT path) rather than hardware ray queries.
    pub(crate) software: bool,
    /// Backend supports the fixed whole-cache ranking passes (`gi_fixed_*`).
    pub(crate) fixed_budget: bool,
    /// An environment map is selected.
    pub(crate) field: bool,
    pub(crate) profiling: bool,
    /// The software near field's bin count.
    pub(crate) near_bins: u32,
    pub(crate) eye: Vec3,
    pub(crate) fov_y: f32,
    pub(crate) size: [u32; 2],
}

/// The first half of a frame's plan: the table header, and what the second half
/// needs from it.
pub(crate) struct Begun {
    /// The table's header words (written at its start every frame).
    pub(crate) header: [u32; HEADER_WORDS as usize],
    since_clear: u32,
    /// Frames since the later of a clear and (with `cut_refill`) a camera cut.
    since_event: u32,
    cut_phases: u32,
    filter: u32,
}

/// What the second half needs that only the caller's scene and device state give.
pub(crate) struct FrameInputs<'a> {
    /// Every placement's world bounds, in draw order then instance slot.
    pub(crate) bounds: Vec<[f32; 6]>,
    pub(crate) poses: Vec<crate::renderer::skinning::PlacementPose>,
    /// A digest of the environment's generation, content revision and rotation.
    pub(crate) environment: [u8; 32],
    /// With bounce: a digest of everything cell shading reads (lights, decals,
    /// transmission, the environment, the programs, the draws).
    pub(crate) light_key: Option<[u8; 32]>,
    /// The scene's pixel rectangle (`scene_pixels`).
    pub(crate) region: [u32; 4],
    /// The view the idle keys hash: its matrix bytes, then its extent bytes.
    pub(crate) still_matrix: [u8; 64],
    pub(crate) still_extent: Vec<u8>,
    /// Bytes that differ whenever a placement transform does: the idle keys hash them. The
    /// renderer passes each draw's collection and edit count (`InstanceState::revision`) instead
    /// of every matrix; the Ara backend passes the matrices themselves.
    pub(crate) instances: &'a [u8],
    /// Every skinned draw's revision, little-endian, in draw order.
    pub(crate) skin_revisions: Vec<u8>,
    /// True when an emitting placement moved recently: schedules fast fixed-budget gather-only
    /// relights on seen cells without invalidating unchanged analytical light visibility.
    pub(crate) emitter_relight: bool,
    /// True during a moving-emitter relight burst (~4-5 frames after an emitting placement moves):
    /// schedules a bounded moving-emitter relight wave (`GI_RELIGHT_GATHER_DIRTY` -> 3-tier
    /// oldest-first visibility sweep with a 1/8 fill reserve) and clears `GI_SETTLED` on live cells.
    pub(crate) emitter_dirty: bool,
}

/// The second half: the frame record and, unless the cache is idle this frame, the
/// passes to run.
pub(crate) struct Finished {
    /// The `GiFrame` record's bytes.
    pub(crate) frame: Vec<u8>,
    /// None: the cache is idle (view, scene and lighting unchanged for long enough).
    pub(crate) plan: Option<Plan>,
}

/// One view's part of the plan (docs/design-views.md): whether its camera and the scene it
/// sees are unchanged, so its frames can idle. Owned by the view; the cache, its clock and
/// its lighting history are the engine's `Planner`.
#[derive(Default)]
pub(crate) struct ViewPlan {
    idle_key: Option<[u8; 32]>,
    idle_frames: u32,
    view_key: Option<[u8; 32]>,
    view_frames: u32,
}

/// The cache's CPU history between frames, and the per-frame plan made from it:
/// everything `prepare_gi` decides without a device. The wgpu renderer and the ara-gpu
/// backend both plan through `begin` and `finish`, then `advance` once the frame's
/// passes are certain to run.
pub(crate) struct Planner {
    /// Frames since a caster last moved (stale cells' sweep, shaders/gi_cache.wgsl GI_STALE).
    since_movers: u32,
    bounds: Vec<[f32; 6]>,
    poses: Vec<crate::renderer::skinning::PlacementPose>,
    environment: Option<[u8; 32]>,
    /// The frame stamp cells are marked seen with.
    frame: u32,
    /// Empty the cache at the start of the next planned frame.
    pending_clear: bool,
    /// Frames since the cache was last emptied.
    since_clear: u32,
    /// Frames since the last camera cut (`camera_cut`); u32::MAX before any.
    since_cut: u32,
    environment_streak: bool,
    environment_catchup: u32,
    environment_phase: u32,
    bounce_last: Option<bool>,
    bounce_busy: u32,
    /// Last collected frame's stale cells (shaders/gi_cache.wgsl GI_W_STALE_CELLS).
    pub(crate) stale_cells: u32,
    /// Consecutive quiet frames under the fixed budget (no cell created and none moved > 1/2048).
    pub(crate) fixed_quiet: u32,
    /// Last collected ranked rays under the fixed budget (GI_W_FIXED_RANKED).
    pub(crate) fixed_ranked: u32,
    /// Requests readers outside the cache appended on a recent frame (the external list's
    /// count, read back without waiting): while any is unanswered the cache does not idle.
    pub(crate) external_pending: u32,
    parity: bool,
    /// The gather queues (this frame's, next frame's) the tick's first frame used: later
    /// views of the tick trace into the same next queue.
    tick_queues: [u32; 2],
    light_key: Option<[u8; 32]>,
    light_streak: bool,
    relight_catchup: u32,
    relight_phase: u32,
}
impl Default for Planner {
    fn default() -> Self {
        Self {
            since_movers: u32::MAX,
            bounds: vec![],
            poses: vec![],
            environment: None,
            frame: 0,
            pending_clear: false,
            since_clear: u32::MAX,
            since_cut: u32::MAX,
            environment_streak: false,
            environment_catchup: 0,
            environment_phase: 0,
            bounce_last: None,
            bounce_busy: 0,
            stale_cells: 0,
            fixed_quiet: 0,
            fixed_ranked: 0,
            external_pending: 0,
            parity: false,
            tick_queues: [0; 2],
            light_key: None,
            light_streak: false,
            relight_catchup: 0,
            relight_phase: 0,
        }
    }
}
impl Planner {
    /// Whether the next planned frame empties the cache first.
    pub(crate) fn pending_clear(&self) -> bool {
        self.pending_clear
    }
    pub(crate) fn set_pending_clear(&mut self, pending: bool) {
        self.pending_clear = pending;
    }
    /// A camera cut or scene replacement: empty the cache on the next frame and restart
    /// the seen stamps, so a reset sequence repeats bit for bit.
    pub(crate) fn reset(&mut self) {
        self.pending_clear = true;
        self.frame = 0;
        self.fixed_quiet = 0;
        self.fixed_ranked = 0;
    }
    /// A camera cut that keeps the table (`Engine::cut_view`): with `cut_refill`, the next
    /// frames refill as after a clear (coarse cells first, capped claims, cut phases).
    pub(crate) fn camera_cut(&mut self) {
        self.since_cut = 0;
        self.fixed_quiet = 0;
    }
    /// A new table: forget what the old one was planned from and start it empty.
    pub(crate) fn reallocated(&mut self) {
        self.environment = None;
        self.bounds.clear();
        self.pending_clear = true;
        self.fixed_quiet = 0;
        self.fixed_ranked = 0;
    }
    /// Still-view convergence check for the fixed budget: a static view where a full sweep
    /// of ranked cells passed with zero cells created and zero cells moving by > 1/2048.
    fn fixed_settled(&self, still: bool, budget: u32) -> bool {
        if !fixed_rest_on() || !still || budget == 0 {
            return false;
        }
        let turn = self.fixed_ranked.div_ceil(budget).max(4) + 4;
        self.fixed_quiet >= turn
    }
    /// Frames since the cache was last emptied.
    #[cfg(test)]
    pub(crate) fn since_clear(&self) -> u32 {
        self.since_clear
    }
    /// The table header for this frame. `run`: the cache's passes are wanted this frame;
    /// `bounce`: its bounce passes too. `tick`: the first frame of an engine tick, which
    /// does the cache's per-tick work; later views of the tick only probe, claim and trace
    /// their new cells, and change none of the engine's history.
    /// Stale cells' turns [read, sweep] (shaders/gi_cache.wgsl GI_STALE): with a budget, the
    /// power of two that spreads last frame's stale cells to about the budget a frame (1 below
    /// it), unread ones four times slower; else the fixed periods.
    fn stale_turns(&self, software: bool) -> [u32; 2] {
        let [read, sweep] = stale_periods();
        let budget = stale_budget(software);
        if budget == 0 {
            return [read, sweep];
        }
        let turn = self.stale_cells.div_ceil(budget).max(1).next_power_of_two().min(STALE_TURN_MAX);
        [read.max(turn), sweep.max(4 * turn)]
    }
    pub(crate) fn begin(&mut self, c: &PlanConfig, run: bool, bounce: bool, tick: bool) -> Begun {
        if run && tick {
            if self.bounce_last.is_some_and(|b| b != bounce) {
                self.pending_clear = true;
            }
            self.bounce_last = Some(bounce);
        }
        let eye = c.eye;
        let s = c.settings;
        let software = c.software;
        let layout = WorkLayout::of_counts(c.slots, c.misses);
        let mut header = [0u32; HEADER_WORDS as usize];
        header[0] = u32::from(run);
        header[1] = c.slots - 1;
        header[23] = c.cold_words; // LC2 per-cell light visibility stride.
        let field = c.field;
        // Probe statistics (GI_F_STATS) on profiled frames only.
        header[2] = u32::from(s.lookup == IndirectLookup::Trilinear)
            | (u32::from(field) << 1)
            | (u32::from(s.rotate_directions) << 2)
            | (u32::from(c.profiling) << 3);
        header[3] = self.frame;
        // A cut's first frames read cells one level coarser (COARSE_CUT_FRAMES), unless claims
        // are uncapped.
        let since_clear = if self.pending_clear {
            0
        } else {
            self.since_clear
        };
        // With `cut_refill`, a camera cut opens the same refill window as a clear.
        let since_event = if s.cut_refill {
            since_clear.min(self.since_cut)
        } else {
            since_clear
        };
        // U-RTVIS: the cut refill caps claims until its backlog drains (CUT_REFILL_FRAMES) and
        // no longer reads one level coarser (coarse cells showed as blocks); an explicit
        // `claim_cells` keeps G2b's coarse start after a clear.
        let cut_capped = s.cut_refill && since_event < CUT_REFILL_FRAMES;
        let coarse = s.claim_cells != u32::MAX && since_event < COARSE_CUT_FRAMES;
        let footprint = 2.0 * (c.fov_y * 0.5).tan() / c.size[1] as f32
            * s.cell_pixels
            * if coarse { 2.0 } else { 1.0 };
        header[4] = footprint.to_bits();
        header[5] = s.min_edge.to_bits();
        header[6] = s.levels;
        header[7] = s.evict_frames;
        for (i, v) in eye.to_array().into_iter().enumerate() {
            header[8 + i] = v.to_bits();
        }
        header[11] = s.direction_phases;
        header[12] = s.level_blend.to_bits();
        header[13] = if s.parallel_rays { RAY_ITEMS } else { 0 };
        let budget = if c.fixed_budget && bounce { fixed_rays(software) } else { 0 };
        let fixed_trace = budget != 0 && fixed_trace_on();
        let fixed_settle = budget != 0 && fixed_rest_on();
        // The cut refill: S groups over the S cache frames that start at a reset. When the fixed
        // ray budget already paces new-cell tracing (`fixed_trace`), trace every admitted cell's
        // full directions at once instead of holding them GI_PARTIAL for 4 extra frames.
        let cut_phases = match s.cut_phases {
            0 if software && !fixed_trace => 4,
            0 => 1,
            phases => phases,
        };
        // The cut refill traces every new cell's directions at once (U-RTVIS: spreading them
        // left non-RT cells partial for frames, as dark blotches).
        header[14] = if s.cut_refill {
            0
        } else if cut_phases > 1 && since_event < cut_phases {
            cut_phases
        } else {
            0
        };
        header[15] = s.near_light_rays;
        header[55] = near_occluded_rays();
        header[56] = u32::from(light_reuse_on());
        header[61] = mover_reach().to_bits();
        // The emitter list after the slots (gi_emitters.wgsl GI_H_EMITTERS, GI_H_EMITTERS_MAX);
        // its counts (58, 59) are the GPU's, written each frame by gi_emitters_publish.
        header[57] = (HEADER_WORDS + u64::from(c.slots) * (24 + u64::from(c.cold_words))) as u32;
        // Where cell shading writes each listed emitter triangle's mean emission in the outgoing
        // radiance: after the slots' values and the draws' probes (gi_cells.wgsl GI_H_EMITTER_MEANS).
        header[34] = c.slots + TRACED_DRAWS;
        // Ray-hardware cache rays pass see-through placements by their transmission
        // (gi_rays_hardware.wgsl; docs/design-see-through-occlusion.md). RENDERLAB_GI_SEE_THROUGH=0
        // passes them at full strength, as before. Software rays pass them at full strength
        // until the cache traces the reflection BVH4.
        header[35] = u32::from(std::env::var("RENDERLAB_GI_SEE_THROUGH").as_deref() != Ok("0"));
        header[60] = c.emitters;
        // Borrowed bounce for cells not gathered yet (gi_cells.wgsl GI_H_BORROW): its records
        // follow the emitter means, at header[34] + header[60] + slot.
        header[46] = u32::from(borrow_on());
        header[47] = u32::from(trickle_on());
        // Maximum emitter triangle count evaluated analytically by cache gathers (`GI_H_EMITTERS_LIMIT`).
        header[43] = emitters_limit();
        let [read, sweep] = self.stale_turns(software);
        header[62] = read;
        header[63] = sweep;
        // Software rays with cooked near-field masks: their octahedral bins per side (0: none).
        if software {
            header[16] = c.near_bins;
        }
        header[17] = u32::from(bounce);
        header[18] = layout.shade;
        header[19] = s.gathers;
        header[30] = s.hit_gathers;
        header[31] = u32::from(s.parent_standin);
        // U-RTVIS: a cut's gathers held to a time budget, the cells pixels read first.
        let cut_window = s.cut_gather_ms > 0.0 && self.since_cut < CUT_GATHER_FRAMES;
        let per_ms = if software { GATHERS_PER_MS_SOFTWARE } else { GATHERS_PER_MS_HARDWARE };
        // First fill caps a frame's new cells for FILL_CLAIM_FRAMES after a cut or clear
        // (unless claims are set explicitly); steady frames keep main's uncapped claims.
        // Software rays' gathers are held to GATHER_RAYS_SOFTWARE estimated rays a frame
        // instead of a cell count (shaders/gi_cache.wgsl gi_gather_rays).
        let since_fill = since_clear.min(self.since_cut);
        let fill = first_fill_on(software) && s.claim_cells == u32::MAX && since_fill < FILL_CLAIM_FRAMES;
        let ray_budget = first_fill_on(software) && software && s.gather_cells == 0 && !cut_window;
        header[20] = if budget != 0 {
            0
        } else {
            match s.gather_cells {
                _ if cut_window => ((s.cut_gather_ms * per_ms) as u32).max(64),
                0 => GATHER_CELLS,
                u32::MAX => 0,
                cells => cells,
            }
        };
        // With Final Gather on the non-RT software path, pixels never read cache cells directly;
        // striding cache bounce gathers by 2 (32 unbiased hemisphere directions/cell, packed into
        // a single warp) doubles cells updated per frame within the same 98k-ray budget so non-RT
        // dynamic bounce converges in ~3 frames alongside RT.
        header[21] = if software && crate::program::final_gather() && s.gather_stride == 1 {
            2
        } else {
            s.gather_stride
        };
        header[22] = if fixed_trace {
            0
        } else {
            match s.claim_cells {
                0 if software => CLAIM_CELLS_SOFTWARE,
                0 => CLAIM_CELLS_HARDWARE,
                u32::MAX if cut_capped && software => CLAIM_CELLS_SOFTWARE,
                u32::MAX if cut_capped => CLAIM_CELLS_HARDWARE,
                u32::MAX => 0,
                cells => cells,
            }
        };
        // The cell-space filter (G3) runs over cell shading's queue, so only with bounce, and
        // only once its sources are allocated.
        let filter = if bounce && c.raw_slots == c.slots {
            s.filter
        } else {
            0
        };
        header[24] = filter;
        header[25] = s.filter_edge.to_bits();
        header[26] = u32::from(s.pressure_eviction);
        // The cone of one of K equal-solid-angle hemisphere directions: 1 - cos(half-angle) =
        // 1 / K, and its width at distance 1 is twice the half-angle's tangent.
        let cone_width = 2.0 * (1.0 - 1.0 / s.directions as f32).acos().tan();
        let spread = if s.hit_cone_cells > 0.0 {
            cone_width / s.hit_cone_cells
        } else {
            0.0
        };
        header[27] = spread.to_bits();
        header[29] = s.mover_hit_levels;
        // The most cells pixels read in a frame: the view's pixels over a cell's.
        header[48] = match (fill, software) {
            (false, _) => 0,
            (true, true) => FILL_CELLS_SOFTWARE,
            (true, false) => FILL_CELLS_HARDWARE,
        };
        // The pixel cells' gather rotation counts from the last cut or clear, and for
        // FILL_PARENT_FRAMES after it pixels also request their parent cells, which the capped
        // claim admits first (shaders/gi_cache.wgsl gi_fill_bin).
        header[49] = match since_fill {
            u32::MAX => self.frame,
            n => n,
        };
        header[50] = u32::from(fill && since_fill < FILL_PARENT_FRAMES);
        header[54] = u32::from(fill && since_fill == 0);
        header[53] = if ray_budget && budget == 0 { GATHER_RAYS_SOFTWARE } else { 0 };
        // Fixed whole-cache update budget (shaders/gi_cache.wgsl GI_H_FIXED_*): 0 keeps the event queue.
        header[44] = u32::from(fixed_trace);
        header[45] = budget;
        header[51] = u32::from(fixed_settle);
        header[28] = if s.gather_pixels_first || cut_window {
            ((c.size[0] as f32 * c.size[1] as f32) / (s.cell_pixels * s.cell_pixels)).ceil().max(1.0) as u32
        } else {
            0
        };
        Begun {
            header,
            since_clear,
            since_event,
            cut_phases,
            filter,
        }
    }
    /// The frame record and the passes to run. `bounce` and `tick` as given to `begin`;
    /// `view` is the rendering view's plan.
    pub(crate) fn finish(
        &mut self,
        c: &PlanConfig,
        begun: &Begun,
        bounce: bool,
        tick: bool,
        view: &mut ViewPlan,
        inputs: FrameInputs<'_>,
    ) -> Finished {
        let s = c.settings;
        let layout = WorkLayout::of_counts(c.slots, c.misses);
        let (since_event, cut_phases, filter) = (begun.since_event, begun.cut_phases, begun.filter);
        let FrameInputs {
            bounds,
            poses,
            environment,
            light_key,
            region,
            still_matrix,
            still_extent,
            instances,
            skin_revisions,
            emitter_relight,
            emitter_dirty,
        } = inputs;
        let mut boxes: Vec<[f32; 6]> = vec![];
        // Scene, environment and lighting events are the tick's: a later view of the tick
        // sees none, and leaves their history for the next tick.
        let previous = if tick {
            std::mem::take(&mut self.bounds)
        } else {
            vec![]
        };
        let poses = if tick { poses } else { self.poses.clone() };
        let previous_poses = std::mem::replace(&mut self.poses, poses.clone());
        if !previous.is_empty() {
            // A skinned caster can change within identical aggregate bounds; P-SHOW: only a
            // new pose does (one per placement, in `bounds` order, collected once; P-GI:
            // searching the draws per placement made this loop quadratic in placements).
            for i in 0..bounds.len().max(previous.len()) {
                let (a, b) = (previous.get(i), bounds.get(i));
                let same_bounds = match (a, b) {
                    (Some(a), Some(b)) => (0..6).all(|k| (a[k] - b[k]).abs() <= 1e-5),
                    (None, None) => true,
                    _ => false,
                };
                if same_bounds && !crate::renderer::skinning::deformed(&previous_poses, &poses, i) {
                    continue;
                }
                let mut swept = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                for x in [a, b].into_iter().flatten() {
                    for k in 0..3 {
                        swept[k] = swept[k].min(x[k]);
                        swept[k + 3] = swept[k + 3].max(x[k + 3]);
                    }
                }
                boxes.push(swept);
            }
        }
        moved_log(&previous, &bounds, &boxes);
        if tick {
            self.bounds = bounds;
        }
        if boxes.len() > MAX_BOXES {
            boxes = cluster_boxes(boxes, MAX_BOXES);
        }
        // An environment change re-integrates every cell from its bits (no rays):
        // yaw rotates the sky, a new map replaces it. A map's intensity and the constant
        // sky are applied at lookup, so their changes re-integrate nothing.
        let environment_changed = tick && self.environment.is_some_and(|e| e != environment);
        if tick {
            self.environment = Some(environment);
        }
        let mut flags = 0u32;
        // Stale cells left by movers: the scan keeps sweeping them for a full period after the
        // last one stops. Counted in frames, not ticks: a capture's held frames do not tick, and
        // a tick count left the sweep (and the scan) running through every held frame.
        self.since_movers = if boxes.is_empty() {
            self.since_movers.saturating_add(1)
        } else {
            0
        };
        let [_, sweep] = self.stale_turns(c.software);
        if sweep > 1 && self.since_movers < sweep {
            flags |= 64;
        }
        if !boxes.is_empty() && s.invalidate_casters {
            flags |= 1;
        }
        // Partially traced cells re-queue: form 2 always; a clear's phases on the frames after
        // its first, and one frame past its window, where a cell still partial (made late in
        // the window) traces every remaining direction. The cut refill makes no partial cells.
        let refill = !s.cut_refill && cut_phases > 1 && (1..=cut_phases).contains(&since_event);
        if tick && (s.direction_phases > 1 || refill) {
            flags |= 8;
        }
        if tick && s.rotation_period != 0 {
            flags |= 2;
        }
        // A step after a still frame re-integrates every cell; a map changing on
        // consecutive frames, and the ENVIRONMENT_PERIOD frames after it last changed,
        // re-integrate one fixed slot group a frame (ENVIRONMENT_PERIOD).
        let integrate_all = environment_changed && !self.environment_streak;
        let spread =
            tick && !integrate_all && (environment_changed || self.environment_catchup > 0);
        if tick {
            self.environment_catchup = if integrate_all {
                0
            } else if environment_changed {
                ENVIRONMENT_PERIOD
            } else {
                self.environment_catchup.saturating_sub(1)
            };
            self.environment_streak = environment_changed;
        }
        let integrate = integrate_all || spread;
        let environment_group = if spread {
            self.environment_phase = (self.environment_phase + 1) % ENVIRONMENT_PERIOD;
            ENVIRONMENT_PERIOD | (self.environment_phase << 8)
        } else {
            0
        };
        if integrate {
            flags |= 4;
        }
        // Bounce (G2): lighting events. The lights, the environment, the frame's program and
        // its draws' materials (stable ids) form the lighting key. A step after still frames
        // relights every cell (shaded this frame, then `gathers` gathers); a change on
        // consecutive frames and the `period` frames after it relight one fixed slot group a
        // frame, one gather each (W2: a fixed rotation). The period holds that work to about
        // `relight_cells` slots a frame (relight_period). A moved caster relights nothing
        // globally: the scan re-traces the cells whose directions cross its swept bounds
        // (gi_reach) and reshades those whose light rays it crosses (gi_light_reached), and
        // their gathers carry the change to the cells that see them. Before this, any moving
        // or deforming mesh (a skinned character, every frame) relit an eighth of the table.
        let mut relight = 0u32;
        let period = relight_period(c.slots, s.relight_cells);
        // Bounce (gathers, cell shading) is the tick's; later views trace their new cells
        // into the tick's next gather queue.
        let bounce = bounce && tick;
        if bounce {
            let changed = self.light_key.is_some() && self.light_key != light_key;
            let all = changed && !self.light_streak;
            let spread = !all && (changed || self.relight_catchup > 0);
            self.relight_catchup = if all {
                0
            } else if changed {
                period
            } else {
                self.relight_catchup.saturating_sub(1)
            };
            self.light_streak = changed;
            if all {
                relight = 1 << 31;
            } else if spread {
                self.relight_phase = (self.relight_phase + 1) % period;
                relight = period | (self.relight_phase << 8);
            } else if emitter_relight {
                // Moving emitter with unchanged analytical lights: mark cells GI_DIRTY on the
                // frame the emitter moved (bit 28) and clear GI_SETTLED on follow-up frames
                // (bit 30) so both 1st and 2nd bounce converge cleanly across seen cells.
                self.relight_phase = (self.relight_phase + 1) % period;
                relight = (1 << 30)
                    | (u32::from(emitter_dirty) << 28)
                    | period
                    | (self.relight_phase << 8);
            }
            if relight != 0 {
                flags |= 16;
            }
        }
        if tick {
            self.light_key = light_key;
        }
        let budget = begun.header[45];
        let busy_relight = bounce && self.bounce_busy > 0;
        if tick {
            self.bounce_busy = if bounce && relight != 0 {
                s.gathers + 2
            } else {
                self.bounce_busy.saturating_sub(1)
            };
        }
        let mut frame = vec![0u8; FRAME_BYTES];
        let mut put =
            |at: usize, v: u32| frame[at * 4..at * 4 + 4].copy_from_slice(&v.to_le_bytes());
        for (i, v) in region.into_iter().enumerate() {
            put(i, v);
        }
        put(4, boxes.len() as u32);
        put(5, s.rotation_period);
        put(6, flags);
        put(7, c.misses);
        put(8, c.slots);
        put(9, layout.misses);
        put(10, layout.traces);
        put(11, layout.traces + c.slots);
        put(12, layout.requests);
        put(13, requests(c.misses) - 1);
        put(14, layout.items);
        // The environment's slot group: period (0: every slot) and phase; bit 31 is set
        // below on environment-only frames, whose scan visits only that group.
        put(15, environment_group);
        // Bounce (G2): this frame's and next frame's gather queues, the relight event (its
        // request bit is set below once the claim rounds are known to run), the shading queue.
        let [queue, next] = if !tick {
            self.tick_queues
        } else if self.parity {
            [layout.queues[1], layout.queues[0]]
        } else {
            layout.queues
        };
        if tick {
            self.tick_queues = [queue, next];
        }
        put(16, queue);
        put(17, next);
        put(18, relight);
        put(19, layout.shade);
        for (i, b) in boxes.iter().enumerate() {
            for k in 0..3 {
                put(20 + i * 8 + k, b[k].to_bits());
                put(20 + i * 8 + 4 + k, b[k + 3].to_bits());
            }
        }
        if let Some(grid) = box_grid(&boxes, mover_reach()) {
            for k in 0..3 {
                put(GRID_WORD + k, grid.lo[k].to_bits());
                put(GRID_WORD + 4 + k, grid.scale[k].to_bits());
            }
            put(GRID_WORD + 3, 1.0f32.to_bits());
            for (cell, mask) in grid.masks.iter().enumerate() {
                for (w, &m) in mask.iter().enumerate() {
                    put(GRID_WORD + 8 + cell * 4 + w, m);
                }
            }
        }
        let groups = [
            (region[2] - region[0]).div_ceil(probe_tile()),
            (region[3] - region[1]).div_ceil(probe_tile()),
        ];
        let clear = tick && std::mem::take(&mut self.pending_clear);
        // A view whose every input is unchanged has nothing to trace once its cells
        // exist: after IDLE_AFTER identical frames the passes stop (still frames cost
        // nothing). Rotation and partial cells keep working every frame.
        // The placements' digest, once: both keys below read it (about 1 MB of transforms a
        // frame on Outpost was hashed twice).
        let instances: [u8; 32] = Sha256::digest(instances).into();
        let idle_key: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(still_matrix);
            h.update(instances);
            h.update(environment);
            h.update(format!("{s:?}").as_bytes());
            h.update(&still_extent);
            // SK1: a fixed camera and placement can still contain animated geometry.
            h.update(&skin_revisions);
            h.finalize().into()
        };
        if view.idle_key == Some(idle_key) && !clear {
            view.idle_frames = view.idle_frames.saturating_add(1);
        } else {
            view.idle_frames = 0;
        }
        view.idle_key = Some(idle_key);
        // The same view and scene with only the environment changing (a rotating sky over a
        // still camera): once that view is idle, no cell is new, so only the scan and the
        // integrate passes run. Such frames do not advance the cache frame, so seen stamps
        // do not age toward eviction while nothing is probed.
        let view_key: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(still_matrix);
            h.update(instances);
            h.update(format!("{s:?}").as_bytes());
            h.update(&still_extent);
            h.finalize().into()
        };
        if view.view_key == Some(view_key) && !clear {
            view.view_frames = view.view_frames.saturating_add(1);
        } else {
            view.view_frames = 0;
        }
        view.view_key = Some(view_key);
        let still = view.idle_frames >= 8
            && flags == 0
            && !clear
            && !busy_relight
            && self.external_pending == 0;
        if !still {
            self.fixed_quiet = 0;
        }
        let fixed_settled = budget != 0 && self.fixed_settled(still, budget);
        // Once a static view has converged across a full sweep with zero created/moved cells,
        // idle the cache immediately instead of waiting out legacy gather-queue idle_after.
        if fixed_settled {
            return Finished { frame, plan: None };
        }
        let busy = busy_relight
            || self.external_pending > 0
            || (bounce && budget != 0 && self.fixed_quiet < 4);
        // A gather cap spreads a cut's gathers over more frames: still frames keep running
        // until a spread of up to 64 slot groups per gather generation can have drained.
        // Under the fixed budget, static views idle once fixed_settled triggers or after 64
        // consecutive unchanged frames (well past convergence, before first-view windows).
        let idle_after = match (bounce, s.gather_cells) {
            (true, _) if budget != 0 => IDLE_AFTER_BOUNCE + 48,
            (true, _) if begun.header[priority::HEADER_LIMIT] != 0 => IDLE_AFTER_BOUNCE
                + c.slots.div_ceil(begun.header[priority::HEADER_LIMIT]).max(64) * (s.gathers + 1),
            (false, _) => IDLE_AFTER,
            (true, u32::MAX) => IDLE_AFTER_BOUNCE,
            (true, _) => IDLE_AFTER_BOUNCE + 64 * (s.gathers + 1),
        };
        if view.idle_frames >= idle_after && flags == 0 && !busy {
            return Finished { frame, plan: None };
        }
        // The still-view batch: a still view with nothing changing still creates and gathers a
        // few cells a frame (the temporal jitter moves pixels across cell edges: Glint, about
        // 50 a frame; none with the jitter off), and at such counts each pass costs the latency
        // of a few long software rays, not their number. Once the view has been still for
        // STILL_BATCH frames and a cut's backlog has had STILL_BATCH_SETTLE frames, the cache's
        // passes run on one frame in STILL_BATCH only: the cells involved are made and lit at
        // most STILL_BATCH - 1 frames later. Any motion, a relight or a moved caster runs every
        // frame.
        if still_batch_on()
            && view.idle_frames >= STILL_BATCH
            && (budget == 0 || (view.idle_frames >= STILL_BATCH_SETTLE && self.fixed_quiet >= 2))
            && flags == 0
            && !busy
            && self.since_cut.min(self.since_clear) >= STILL_BATCH_SETTLE
            && view.idle_frames % STILL_BATCH != 0
        {
            return Finished { frame, plan: None };
        }
        // With bounce a changing environment relights (flags 16), so frames that are
        // environment-only never run bounce.
        let environment_only = view.view_frames >= IDLE_AFTER && flags == 4;
        let mut scan_groups = c.slots.div_ceil(64);
        if environment_only && spread {
            let word = (environment_group | (1 << 31)).to_le_bytes();
            frame[60..64].copy_from_slice(&word);
            scan_groups = (c.slots / ENVIRONMENT_PERIOD).div_ceil(64);
        }
        let bounce = bounce && !environment_only;
        if bounce {
            // Gathers may request hit cells: the claim rounds run after them.
            let word = (relight | 0x2000_0000).to_le_bytes();
            frame[72..76].copy_from_slice(&word);
        }
        if bounce {
            self.parity = !self.parity;
        }
        Finished {
            frame,
            plan: Some(Plan {
                groups,
                scan_groups,
                scan: flags != 0,
                integrate,
                clear,
                environment_only,
                bounce,
                next_queue_at: u64::from(next) * 4,
                filter: bounce && filter != 0,
                fixed_rays: if bounce { budget } else { 0 },
                post_emitters: bounce && (clear || begun.since_clear == 0 || (relight != 0 && (relight & (1 << 30)) == 0)),
            }),
        }
    }
    /// The planned frame's passes are encoded: count it.
    pub(crate) fn advance(&mut self, begun: &Begun, environment_only: bool) {
        if !environment_only {
            self.since_clear = begun.since_clear.saturating_add(1);
            self.since_cut = self.since_cut.saturating_add(1);
            self.frame = self.frame.wrapping_add(1);
        }
    }
}
enum Retired {
    Resources(#[allow(dead_code)] Resources),
    Arrays(#[allow(dead_code)] Arrays),
    Packed(#[allow(dead_code)] Packed),
    Groups(#[allow(dead_code)] Groups),
}

pub(super) struct GiState {
    pub settings: IndirectSettings,
    /// A header-only table with `enabled` 0: lighting's binding 20 while the cache is
    /// off, and always in the kernels' own lighting group (they write the real table).
    pub dummy: Owned<wgpu::Buffer>,
    _dummy_allocation: Allocation,
    resources: Option<Resources>,
    /// Bumped when the real table is replaced; lighting rebinds then.
    pub generation: u64,
    layouts: Option<Layouts>,
    pipelines: Option<Pipelines>,
    arrays: Option<Arrays>,
    meshes: Vec<(wgpu::Buffer, Arc<Blas>)>,
    /// The octahedral bins per side every cooked mesh's near-field masks share (0: none).
    near_bins: u32,
    /// A mask-free cooked near field merged into one world BVH (`World`).
    world: Option<World>,
    packed: Option<Packed>,
    plan: Option<Plan>,
    /// Frames that ran cache work, across every view: a view's receivers reuse last frame's
    /// lookups only while this is unchanged since they read the table (another view's frame
    /// may have run the cache work in between).
    work_generation: u64,
    profile: Option<Profile>,
    refill_feedback: Option<refill_feedback::State>,
    priority_compact: bool,
    priority_hold: bool,
    queue_probe: Option<queue_probe::State>,
    /// Whether a reader outside the cache appends to the external request list
    /// (`request_external`); until one does, its pass is not encoded.
    external_readers: bool,
    /// Whether a reader outside the cache reads the emitter list (`request_emitters`).
    emitter_readers: bool,
    groups: Option<Groups>,
    retired: Vec<(u64, Retired)>,
    stats: IndirectStats,
    /// Bounce (G2). Stable draw ids: a draw's vertex buffer at index id - 1, so a cell's
    /// material names its placement across draw-order changes (shaders/gi_cells.wgsl
    /// GI_C_MATERIAL); the draw table last uploaded.
    draw_ids: Vec<(wgpu::Buffer, wgpu::Buffer)>,
    draw_table: Vec<u32>,
    /// The light key's parts last frame (lights, environment, programs, draw ids, decals and
    /// transmission), and which of them changed this frame as bits 0-4: what relit the cache
    /// (`RENDERLAB_GI_LOG` field `relight_parts`; measurement only).
    key_parts: Option<[[u8; 32]; 5]>,
    pub(crate) relight_parts: u32,
    /// Cell shading (G2) of every program a frame has shaded or prepare has prepared, single
    /// and mixed entries (MP1), kept while the program is loaded: a program that leaves the
    /// view and comes back compiles nothing. A newer key for a program (an edit) replaces
    /// its entry.
    shading: Vec<ShadePipeline>,
    /// This frame's entries in `shading`, in dispatch order: one single-program entry, or with
    /// `mixed_active` the mixed entry of each reflecting program of the frame.
    frame_shading: Vec<usize>,
    mixed_active: bool,
    /// The CPU history and per-frame plan (shared with the ara-gpu backend).
    planner: Planner,
    /// `auto_slots`: the bucket-full readback and a pending doubling.
    watch: Option<FullWatch>,
    grow: bool,
    /// `RENDERLAB_GI_PROBE_LOG`: the probe's work counters, read back without waiting.
    probe_log: Option<ProbeWatch>,
    /// The still-view batch: misses plus queued gathers of recent cache frames.
    stale_watch: Option<FullWatch>,
    /// Fixed budget convergence readback (GI_W_FIXED_MOVED, GI_W_FIXED_RANKED, GI_W_CREATED).
    fixed_watch: Option<FullWatch>,
    /// The external list's count (word 0) at the start of each encode, idle or not: what
    /// readers appended on the frame before (`Planner::external_pending`).
    external_watch: Option<FullWatch>,
    /// The emitter list's capacity the next allocation takes (EMITTERS_MAX, grown by
    /// `grow_emitters`), and its listed + dropped triangles of recent builds.
    emitters_max: u32,
    emitter_watch: Option<FullWatch>,
    /// Digest of emitting draws' world bounds on the previous tick, plus short catchup counters:
    /// `emitter_moved_frames` (1 frame while moving, for zero-lag direct emitters in final gather)
    /// and `emitter_relight_frames` (3 frames, for 1st/2nd/3rd-bounce convergence in gi_cache/TR1).
    emitter_bounds_hash: Option<[u8; 32]>,
    has_emitters: bool,
    emitter_moved_frames: u32,
    emitter_relight_frames: u32,
}

/// Measurement only: `RENDERLAB_GI_PROBE_LOG=PATH` counts the probe's work on every frame
/// (GI_F_STATS) and appends each frame's work counters (words 0-31; the probe's from
/// GI_W_P_TILES, 16) and the table's slot count to PATH as one JSON line, read a frame or two
/// later from a ring of mappable buffers.
fn probe_log_path() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var("RENDERLAB_GI_PROBE_LOG").ok()).as_deref()
}
pub(crate) struct ProbeWatch {
    ring: Vec<(wgpu::Buffer, WatchState)>,
    frame: u64,
}
impl ProbeWatch {
    const RING: usize = 4;
    const BYTES: u64 = 128;
    fn new(device: &wgpu::Device) -> Self {
        let ring = (0..Self::RING)
            .map(|_| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("indirect cache probe counters readback"),
                    size: Self::BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (buffer, WatchState::Free)
            })
            .collect();
        Self { ring, frame: 0 }
    }
    fn record(&mut self, encoder: &mut wgpu::CommandEncoder, work: &wgpu::Buffer) {
        if let Some((buffer, state)) = self.ring.iter_mut().find(|(_, s)| matches!(s, WatchState::Free)) {
            encoder.copy_buffer_to_buffer(work, 0, buffer, 0, Self::BYTES);
            *state = WatchState::Recorded;
        }
    }
    fn submitted(&mut self) {
        for (buffer, state) in &mut self.ring {
            if matches!(state, WatchState::Recorded) {
                let done = Arc::new(std::sync::OnceLock::new());
                let callback = done.clone();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = callback.set(r.is_ok());
                });
                *state = WatchState::Mapping(done);
            }
        }
    }
    /// Appends every mapped entry's counters to the log.
    fn collect(&mut self, path: &str, slots: u32) {
        use std::io::Write;
        for (buffer, state) in &mut self.ring {
            let WatchState::Mapping(done) = state else { continue };
            let Some(&ok) = done.get() else { continue };
            if ok {
                let words: Vec<u32> = buffer
                    .slice(..)
                    .get_mapped_range()
                    .map(|v| v.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
                    .unwrap_or_default();
                buffer.unmap();
                self.frame += 1;
                let line = format!("{{\"seq\":{},\"slots\":{slots},\"work\":{:?}}}\n", self.frame, words);
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = file.write_all(line.as_bytes());
                }
            }
            *state = WatchState::Free;
        }
    }
}

/// The `full` counter (work word 4) of recent cache frames, copied after their passes into a
/// small ring of mappable buffers and read once mapped, a frame or two later: no frame waits.
pub(crate) struct FullWatch {
    ring: Vec<(wgpu::Buffer, WatchState)>,
    /// The work words copied (summed when read): `full` (word 4) for `auto_slots`; misses
    /// and next frame's queued gathers (words 0 and 38) for the still-view batch.
    words: Vec<u64>,
}
enum WatchState {
    Free,
    Recorded,
    Mapping(Arc<std::sync::OnceLock<bool>>),
}
impl FullWatch {
    const RING: usize = 3;
    fn new(device: &wgpu::Device) -> Self {
        Self::of_words(device, &[4])
    }
    fn of_words(device: &wgpu::Device, words: &[u64]) -> Self {
        let size = (4 * words.len() as u64).max(8);
        let ring = (0..Self::RING)
            .map(|_| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("indirect cache counter readback"),
                    size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (buffer, WatchState::Free)
            })
            .collect();
        Self { ring, words: words.to_vec() }
    }
    /// After the cache passes: copies this frame's counter into a free entry, if any.
    fn record(&mut self, encoder: &mut wgpu::CommandEncoder, work: &wgpu::Buffer) {
        if let Some((buffer, state)) = self
            .ring
            .iter_mut()
            .find(|(_, s)| matches!(s, WatchState::Free))
        {
            for (i, &w) in self.words.iter().enumerate() {
                encoder.copy_buffer_to_buffer(work, w * 4, buffer, 4 * i as u64, 4);
            }
            *state = WatchState::Recorded;
        }
    }
    /// After the frame's submission: maps what it recorded.
    pub(crate) fn submitted(&mut self) {
        for (buffer, state) in &mut self.ring {
            if matches!(state, WatchState::Recorded) {
                let done = Arc::new(std::sync::OnceLock::new());
                let callback = done.clone();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = callback.set(r.is_ok());
                });
                *state = WatchState::Mapping(done);
            }
        }
    }
    /// The largest count among entries mapped since (after a device poll), if any.
    fn collect(&mut self) -> Option<u32> {
        let mut largest = None;
        for (buffer, state) in &mut self.ring {
            let WatchState::Mapping(done) = state else {
                continue;
            };
            let Some(&ok) = done.get() else { continue };
            if ok {
                let n = self.words.len();
                let full = buffer
                    .slice(..)
                    .get_mapped_range()
                    .map(|v| {
                        (0..n).fold(0u32, |a, i| {
                            a.saturating_add(u32::from_le_bytes(v[4 * i..4 * i + 4].try_into().unwrap()))
                        })
                    })
                    .unwrap_or(0);
                buffer.unmap();
                largest = Some(largest.unwrap_or(0).max(full));
            }
            *state = WatchState::Free;
        }
        largest
    }
    /// Per-word maxima across ready ring entries (after a device poll), if any.
    fn collect_words(&mut self) -> Option<Vec<u32>> {
        let mut out: Option<Vec<u32>> = None;
        for (buffer, state) in &mut self.ring {
            let WatchState::Mapping(done) = state else {
                continue;
            };
            let Some(&ok) = done.get() else { continue };
            if ok {
                let n = self.words.len();
                if let Ok(v) = buffer.slice(..).get_mapped_range() {
                    let vals: Vec<u32> = (0..n)
                        .map(|i| u32::from_le_bytes(v[4 * i..4 * i + 4].try_into().unwrap()))
                        .collect();
                    match &mut out {
                        Some(acc) => {
                            for (a, b) in acc.iter_mut().zip(vals) {
                                *a = (*a).max(b);
                            }
                        }
                        None => out = Some(vals),
                    }
                }
                buffer.unmap();
            }
            *state = WatchState::Free;
        }
        out
    }
}

/// `auto_slots`' starting size: about a quarter as many slots as the scene's drawn triangles
/// (instances counted), between 2^16 and `cap`. Showcase (about 380k) starts at 2^17, Outpost
/// (3.1M) and the quarry (13M) at the cap.
pub(crate) fn scene_slots_log2(triangles: u64, cap: u32) -> u32 {
    let log2 = 64 - triangles.max(1).leading_zeros();
    log2.saturating_sub(2).clamp(16, cap.max(16))
}
/// `RENDERLAB_GI_SLOTS=N`: the table fixed at 2^N slots (measurement A/B).
fn forced_slots_log2() -> Option<u32> {
    static FORCED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        std::env::var("RENDERLAB_GI_SLOTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n| (10..=21).contains(n))
    })
}
impl GiState {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let dummy = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("indirect cache header (off)"),
                size: HEADER_WORDS * 4 + SLOT_WORDS * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        Self {
            settings: IndirectSettings::default(),
            _dummy_allocation: ledger.allocate(AllocationKind::Light, dummy.size()),
            dummy,
            resources: None,
            generation: 0,
            layouts: None,
            pipelines: None,
            arrays: None,
            meshes: vec![],
            near_bins: 0,
            world: None,
            packed: None,
            plan: None,
            work_generation: 0,
            planner: Planner::default(),
            profile: None,
            refill_feedback: None,
            priority_compact: false,
            priority_hold: false,
            queue_probe: None,
            external_readers: false,
            emitter_readers: false,
            groups: None,
            retired: vec![],
            stats: IndirectStats::default(),
            draw_ids: vec![],
            draw_table: vec![],
            key_parts: None,
            relight_parts: 0,
            shading: vec![],
            frame_shading: vec![],
            mixed_active: false,
            watch: None,
            grow: false,
            probe_log: None,
            stale_watch: None,
            fixed_watch: None,
            external_watch: None,
            emitters_max: EMITTERS_MAX,
            emitter_watch: None,
            emitter_bounds_hash: None,
            has_emitters: false,
            emitter_moved_frames: 0,
            emitter_relight_frames: 0,
        }
    }
    /// After the frame's submission (Renderer::submit_frame): maps the full-bucket readback.
    pub(crate) fn submitted(&mut self) {
        if let Some(watch) = &mut self.watch {
            watch.submitted();
        }
        if let Some(watch) = &mut self.stale_watch {
            watch.submitted();
        }
        if let Some(watch) = &mut self.fixed_watch {
            watch.submitted();
        }
        if let Some(watch) = &mut self.external_watch {
            watch.submitted();
        }
        if let Some(watch) = &mut self.emitter_watch {
            watch.submitted();
        }
        if let Some(watch) = &mut self.probe_log {
            watch.submitted();
        }
    }
    /// Whether the cache was emptied, the camera cut, or a lighting edit (the light key cell
    /// shading reads) is still relighting the table, within the last RECENT_CHANGE_FRAMES frames:
    /// readers that skip work on settled views (the final gather's stable probes) do all of it then.
    pub(crate) fn recently_changed(&self) -> bool {
        const RECENT_CHANGE_FRAMES: u32 = 8;
        let p = &self.planner;
        p.pending_clear
            || p.since_clear.min(p.since_cut) < RECENT_CHANGE_FRAMES
            || p.light_streak
            || p.relight_catchup > 0
            || p.since_movers <= 2
            || self.emitter_relight_frames > 0
    }
    /// Whether any active draw in this frame uses an emitting program (CPU skip for zero-emitter scenes).
    pub(crate) fn has_emitters(&self) -> bool {
        self.has_emitters
    }
    /// Whether an emitting placement moved on this tick so final gather refreshes direct emitter
    /// radiance with zero temporal lag and no ghost trails.
    pub(crate) fn emitter_moved(&self) -> bool {
        self.emitter_moved_frames > 0
    }
    /// Whether world-space bounce cells are still converging after an emitter moved (3 frames).
    pub(crate) fn emitter_relighting(&self) -> bool {
        self.emitter_relight_frames > 0
    }
    /// The table shading binds (lighting group 3, binding 20).
    pub fn shading_table(&self) -> &wgpu::Buffer {
        self.resources.as_ref().map_or(&*self.dummy, |r| &*r.table)
    }
    /// Every slot's outgoing radiance, which cache-terminated reflection hits read
    /// (shaders/reflection_cache_hits.wgsl); the zero dummy while the cache has no resources.
    pub fn shading_outgoing(&self) -> &wgpu::Buffer {
        self.resources
            .as_ref()
            .map_or(&*self.dummy, |r| &*r.outgoing)
    }
    /// Called by a reader that appends to `external_requests` (the final gather), on every frame
    /// it does: from the next encode on, the cache turns the list into requests.
    pub fn request_external(&mut self) {
        self.external_readers = true;
    }
    /// Called by a reader of the emitter list (the final gather), on every frame it reads it:
    /// from the next encode on, the cache builds the list and its power CDF whenever it gathers.
    pub fn request_emitters(&mut self) {
        self.emitter_readers = true;
    }
    /// The list readers outside the cache append cells without a value to (shaders/gi_cache.wgsl
    /// `gi_external`: count, overflow, two spare words, then per cell level | bin << 8 and its
    /// lattice coordinates); the next frame requests them. The dummy while the cache has no
    /// resources.
    pub fn external_requests(&self) -> &wgpu::Buffer {
        self.resources
            .as_ref()
            .map_or(&*self.dummy, |r| &*r.external)
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
}

/// The kernel module: built-in prelude, primary ray helpers, cell definitions, one ray
/// source and the kernels.
pub(crate) fn kernel_source(software: bool, directions: u32) -> String {
    let priority = software && priority::on();
    probe_cut(priority::source(kernel_source_base(software, directions), priority))
}
/// Measurement only (`RENDERLAB_GI_PROBE_CUT=noinsert|nowalk|nostamp|nofind`, comma list): gi_probe
/// without its pixels' requests, without its walk rounds, or without its seen stamps, to split
/// its time. The cache's later frames are wrong under any of them.
fn probe_cut(source: String) -> String {
    let Ok(list) = std::env::var("RENDERLAB_GI_PROBE_CUT") else {
        return source;
    };
    let mut source = source;
    for cut in list.split(',') {
        let (from, to) = match cut {
            "noinsert" => (
                "            gi_request_cells(gi_request_desc(r.world,pick.lo,b));\n            if pick.hi!=pick.lo { gi_request_cells(gi_request_desc(r.world,pick.hi,b)); }\n",
                "",
            ),
            "nowalk" => (
                "        if round+1u<GI_PROBE_ROUNDS && workgroupUniformLoad(&gi_local_any_flag)!=0u {",
                "        if false && workgroupUniformLoad(&gi_local_any_flag)!=0u {",
            ),
            "nostamp" => ("                gi_table[gi_cold(slot)+GI_C_SEEN]=gi_word(GI_H_FRAME);\n", ""),
            // Rounds that touch no cell: the pixels' inserts, decode and barriers alone.
            "nofind" => (
                "            let i=gi_local_list[k];\n            let entry_def=gi_local_def[i];\n",
                "            let i=gi_local_list[k];\n            if i!=0xffffffffu { continue; }\n            let entry_def=gi_local_def[i];\n",
            ),
            _ => continue,
        };
        assert_eq!(source.matches(from).count(), 1, "RENDERLAB_GI_PROBE_CUT {cut}");
        source = source.replacen(from, to, 1);
    }
    source
}
fn kernel_source_base(software: bool, directions: u32) -> String {
    let enables = if software {
        "enable wgpu_binding_array;\n"
    } else {
        "enable wgpu_binding_array;\nenable wgpu_ray_query;\nenable wgpu_ray_query_vertex_return;\n"
    };
    use crate::program::gi_shader;
    let rays = if software {
        gi_shader(
            "gi_rays_software.wgsl",
            include_str!("../shaders/gi_rays_software.wgsl"),
        )
    } else {
        gi_shader(
            "gi_rays_hardware.wgsl",
            include_str!("../shaders/gi_rays_hardware.wgsl"),
        )
    };
    let cells = gi_shader("gi_cells.wgsl", include_str!("../shaders/gi_cells.wgsl"));
    let line = "const GI_DIRECTIONS:u32=64u;";
    assert_eq!(cells.matches(line).count(), 1);
    let cells = cells.replace(line, &format!("const GI_DIRECTIONS:u32={directions}u;"));
    let kernels = gi_shader("gi_cache.wgsl", include_str!("../shaders/gi_cache.wgsl"));
    assert_eq!(
        kernels
            .matches(&format!("const GI_RAY_ITEMS:u32={RAY_ITEMS}u;"))
            .count(),
        1
    );
    assert_eq!(
        kernels
            .matches(&format!("const GI_W_CLAIMS:u32={COUNTER_WORDS}u;"))
            .count(),
        1
    );
    let line = "const GI_NEIGHBOURS:array<u32,256>=array<u32,256>();";
    assert_eq!(kernels.matches(line).count(), 1);
    let words: Vec<String> = neighbour_table(directions)
        .iter()
        .map(|w| format!("{w}u"))
        .collect();
    let mut kernels = kernels.replace(
        line,
        &format!(
            "const GI_NEIGHBOURS:array<u32,256>=array<u32,256>({});",
            words.join(",")
        ),
    );
    kernels.push('\n');
    kernels.push_str(&gi_shader(
        "gi_near.wgsl",
        include_str!("../shaders/gi_near.wgsl"),
    ));
    kernels.push('\n');
    kernels.push_str(&gi_shader(
        "gi_emitters.wgsl",
        include_str!("../shaders/gi_emitters.wgsl"),
    ));
    // Keep the hardware module unchanged, including its original gather entry.
    if software {
        kernels.push('\n');
        kernels.push_str(&gi_shader(
            "gi_gather_software.wgsl",
            include_str!("../shaders/gi_gather_software.wgsl"),
        ));
    }
    format!(
        "{enables}{}\nconst PRIMARY_BATCHES:u32={TRACED_DRAWS}u;\n{}\n{cells}\n{rays}\n{kernels}",
        include_str!("../shaders/scene.wgsl"),
        include_str!("../shaders/primary_rays.wgsl"),
    )
}

/// Shared source composition for the production cell pipeline and CPU source census.
fn cell_shading_source(
    head: &str,
    source: &str,
    directions: u32,
    mixed: Option<u64>,
) -> crate::program::Composed {
    let parameters = crate::program::declares_parameters(source);
    let mut composed =
        crate::program::compose(head, source, crate::program::Stage::Cell, parameters);
    // Cell shading reads cold records too: retain the tracing kernel's packed layout.
    let line = "const GI_DIRECTIONS:u32=64u;";
    assert_eq!(composed.source.matches(line).count(), 1);
    composed.source = composed
        .source
        .replace(line, &format!("const GI_DIRECTIONS:u32={directions}u;"));
    if let Some(id) = mixed {
        composed.source = mixed_cell_entry(&composed.source, id);
    }
    composed
}

#[cfg(test)]
pub(super) fn compile_structure_cell_source(
    head: &str,
    program: &str,
    directions: u32,
    mixed: bool,
) -> String {
    cell_shading_source(head, program, directions, mixed.then_some(0)).source
}

/// Each fixed direction's eight nearest directions of the unrotated set (the kernels'
/// gi_local_direction, in f64), nearest first with ties to the lower index, as 8-bit
/// indices four to a word, padded to 256 words (128 directions).
fn neighbour_table(directions: u32) -> Vec<u32> {
    let direction = |k: u32| {
        let z = 1.0 - (f64::from(k) + 0.5) / f64::from(directions);
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = f64::from(k) * 2.399963229728653;
        [r * phi.cos(), r * phi.sin(), z]
    };
    let mut words = vec![0u32; 256];
    for k in 0..directions {
        let d = direction(k);
        let mut near: Vec<(f64, u32)> = (0..directions)
            .filter(|&j| j != k)
            .map(|j| {
                let e = direction(j);
                (d[0] * e[0] + d[1] * e[1] + d[2] * e[2], j)
            })
            .collect();
        near.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (i, &(_, j)) in near.iter().take(8).enumerate() {
            let at = (k * 8) as usize + i;
            words[at / 4] |= j << (8 * (at % 4));
        }
    }
    words
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}
fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}
fn uint_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

impl Renderer {
    /// A camera cut that keeps the cache (`Engine::cut_view`): with `cut_refill` on, the
    /// next frames refill coarse-first under the claim budget (docs/design-gi-cut-refill.md).
    /// Which parts of the GI light key changed on the last frame, bits 0-4: lights,
    /// environment, programs, draw ids, decals and transmission (measurement only).
    pub(crate) fn indirect_relight_parts(&self) -> u32 {
        self.gi.relight_parts
    }
    pub fn mark_indirect_cut(&mut self) {
        self.gi.planner.camera_cut();
    }
    // SK1: refresh software-ray geometry in the admitted pose. Keep local swept-box
    // invalidation rather than clearing every cell merely because a mesh deformed.
    pub(super) fn update_skinned_indirect(
        &mut self,
        mesh: &Mesh,
        positions: &[[f32; 3]],
        indices: &[u32],
    ) -> Result<bool, Error> {
        if !self.gi.settings.enabled
            || (self.hardware_shadows.is_some() && !self.gi.settings.force_software)
        {
            return Ok(false);
        }
        let built = std::time::Instant::now();
        // A re-skinned mesh keeps its tree and refits it to the pose (`skin_refit`).
        let key: &wgpu::Buffer = &mesh.vertex;
        let refit = skin_refit()
            .then(|| self.gi.meshes.iter().find(|(k, _)| k == key))
            .flatten()
            .map(|(_, b)| b)
            .filter(|b| b.exact && b.bvh.positions.len() == positions.len() && b.bvh.indices.len() == indices.len())
            .map(|b| {
                let mut bvh = b.bvh.clone();
                let primitives = b.primitives.clone();
                crate::asset::near_field::refit_bvh(&mut bvh, positions).map(|()| (bvh, primitives))
            });
        let refit_ok = matches!(refit, Some(Ok(_)));
        let blas = match refit {
            Some(Ok((bvh, primitives))) => {
                let mut blas = Blas::new(bvh, 0)?;
                blas.exact = true;
                blas.primitives = primitives;
                blas
            }
            _ => Blas::with_primitives(gi_bvh::build(positions, indices)?, indices)?,
        };
        let build_ms = built.elapsed().as_secs_f64() * 1000.;
        // Measurement only (change log): why a pose could not patch.
        let diagnosis = super::change_log::enabled().then(|| {
            let old = self.gi.meshes.iter().find(|(k, _)| k == key).map(|(_, b)| b);
            format!(
                "\"old={} exact={} same_vertices={} same_indices={} packed={} refit={}\"",
                old.is_some(),
                old.is_some_and(|b| b.exact),
                old.is_some_and(|b| b.bvh.positions.len() == positions.len()),
                old.is_some_and(|b| b.bvh.indices.len() == indices.len()),
                self.gi.packed.is_some(),
                refit_ok,
            )
        });
        let unpatched = if skin_patch() {
            self.patch_skinned_blas(mesh, blas)?
        } else {
            Some(blas)
        };
        let patched = unpatched.is_none();
        if super::change_log::enabled() {
            super::change_log::report(
                self.frames + 1,
                "skinned_geometry",
                &[],
                &[
                    ("build_ms", format!("{build_ms:.3}")),
                    ("patched", patched.to_string()),
                    ("state", diagnosis.unwrap_or_default()),
                ],
            );
        }
        if let Some(blas) = unpatched {
            let pending = self.gi.planner.pending_clear();
            self.set_indirect_blas(mesh, blas)?;
            self.gi.planner.set_pending_clear(pending);
        }
        Ok(true)
    }
    /// Play loop (sunrt, 2026-11-16): writes a re-skinned mesh's new software geometry over its
    /// own words in the packed buffer, when the new tree has the old one's node, leaf, position
    /// and index counts and neither has near-field masks, and swaps it in as the mesh's geometry.
    /// Every other packed word is unchanged, and the instance records (bounds, root) are rewritten
    /// by `prepare_gi_geometry` as on any frame. Hands the geometry back, changing nothing, when
    /// it is not packed or a size differs: the caller then repacks everything, as before.
    /// A skinned character otherwise discarded the packed buffer on every pose, so each frame
    /// repacked and re-uploaded every mesh, the static world included (Showcase: 5.8 ms CPU).
    fn patch_skinned_blas(&mut self, mesh: &Mesh, blas: Blas) -> Result<Option<Blas>, Error> {
        let key: &wgpu::Buffer = &mesh.vertex;
        let Some(index) = self.gi.meshes.iter().position(|(k, _)| k == key) else {
            return Ok(Some(blas));
        };
        let old = &self.gi.meshes[index].1;
        let same = |a: &Blas, b: &Blas| {
            a.bvh.nodes.len() == b.bvh.nodes.len()
                && a.bvh.leaves.len() == b.bvh.leaves.len()
                && a.bvh.positions.len() == b.bvh.positions.len()
                && a.bvh.indices.len() == b.bvh.indices.len()
                && a.primitives.len() == b.primitives.len()
                && a.near.is_empty()
                && b.near.is_empty()
        };
        let Some(packed) = self.gi.packed.as_ref() else {
            return Ok(Some(blas));
        };
        if !same(old, &blas) || self.gi.world.iter().flat_map(|w| &w.meshes).any(|m| m.as_ref() == Some(key)) {
            return Ok(Some(blas));
        }
        let regions: Vec<u32> = packed
            .meshes
            .iter()
            .zip(&packed.entry)
            .filter(|(m, _)| *m == key)
            .filter_map(|(_, e)| e.map(|i| packed.offsets[i][0]))
            .collect();
        if regions.is_empty() {
            return Ok(Some(blas));
        }
        let words = gi_bvh::mesh_words(&[&blas], &[[0; 4]], &[vec![]], blas.words());
        let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        for start in regions {
            self.queue.write_buffer(&packed.buffer, u64::from(start) * 4, &raw);
        }
        self.gi.meshes[index].1 = Arc::new(blas);
        Ok(None)
    }
    /// Selects world radiance cache settings. Enabling allocates the table (all
    /// slots empty); a new slot count reallocates it. With `auto_slots` (and no
    /// `RENDERLAB_GI_SLOTS`), the first eligible frame sizes it to its scene instead
    /// (`prepare_gi`), and a table already sized keeps its size, within the new cap. Takes
    /// effect on the next lit traced or visibility-buffer frame whose programs call
    /// `indirect_irradiance`.
    pub fn set_indirect(&mut self, settings: IndirectSettings) -> Result<(), Error> {
        self.check_fault()?;
        settings.validate()?;
        // Switching from a fixed table opts back into scene sizing on the next frame.
        // Retire the old bindings with their table: submitted frames may still use them.
        if settings.auto_slots && !self.gi.settings.auto_slots && forced_slots_log2().is_none() {
            if let Some(old) = self.gi.resources.take() {
                self.gi.retired.push((self.submissions, Retired::Resources(old)));
            }
            if let Some(old) = self.gi.groups.take() {
                self.gi.retired.push((self.submissions, Retired::Groups(old)));
            }
        }
        let slots = match forced_slots_log2() {
            Some(log2) => Some(self.gi_slots(log2)),
            None if settings.auto_slots => self
                .gi
                .resources
                .as_ref()
                .map(|r| r.slots.min(self.gi_slots(settings.slots_log2))),
            None => Some(self.gi_slots(settings.slots_log2)),
        };
        // The filter's sources are sized by the setting (G3).
        let raw_slots = |slots| if settings.filter != 0 { slots } else { 1 };
        let reallocate = slots.filter(|&slots| {
            settings.enabled
                && self.gi.resources.as_ref().is_none_or(|r| {
                    r.slots != slots
                        || r.raw_slots != raw_slots(slots)
                        || r.directions != settings.directions
                })
        });
        if let Some(slots) = reallocate {
            let previous = std::mem::replace(&mut self.gi.settings, settings);
            let allocated = self.allocate_gi(slots);
            self.gi.settings = previous;
            allocated?;
        } else if self.gi.settings.lookup != settings.lookup
            || self.gi.settings.cell_pixels != settings.cell_pixels
            || self.gi.settings.min_edge != settings.min_edge
            || self.gi.settings.levels != settings.levels
            || self.gi.settings.direction_phases != settings.direction_phases
            || self.gi.settings.directions != settings.directions
            || self.gi.settings.rotate_directions != settings.rotate_directions
            // Bounce values left by an earlier setting would still be read.
            || self.gi.settings.bounce != settings.bounce
            // Values filtered, or sources written, under another filter setting (G3).
            || self.gi.settings.filter != settings.filter
        {
            // Keys change meaning: start from an empty table.
            self.gi.planner.set_pending_clear(true);
        }
        if self.gi.settings.force_software != settings.force_software {
            self.gi.planner.set_pending_clear(true);
        }
        self.gi.settings = settings;
        Ok(())
    }
    /// The slots a table of 2^`slots_log2` gets on this device: that many, or the most (a power
    /// of two) whose table fits the device's storage-binding and buffer limits, since shading
    /// binds the whole table. wgpu's default binding limit, 128 MiB, holds 2^19 slots; the
    /// default 2^20 (224 MiB, GC1) needs a device created with a higher limit, as the quarry
    /// harness's are (docs/m7-gc1.md).
    fn gi_slots(&self, slots_log2: u32) -> u32 {
        let limits = self.device.limits();
        let fits = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let mut log2 = slots_log2;
        while log2 > 10 && (HEADER_WORDS + (1u64 << log2) * SLOT_WORDS) * 4 > fits {
            log2 -= 1;
        }
        1 << log2
    }
    /// Allocates the table and work buffers for `slots` (all slots empty), replacing any
    /// earlier ones. The cache is on by default (G2b) and allocates on the first frame whose
    /// program calls an indirect input (`prepare_gi`), so a renderer whose programs never do
    /// holds none of it.
    /// While the emitter list is built: a build that dropped triangles (listed + dropped above the
    /// capacity) grows the list to the next power of two that holds them, up to EMITTERS_CAP, and
    /// reallocates the cache (emptied, as a table doubling empties it). Once per scene that needs it.
    fn grow_emitters(&mut self) -> Result<(), Error> {
        if !(self.gi.emitter_readers || emitters_limit() > 0) {
            return Ok(());
        }
        let device = &self.device;
        let watch = self.gi.emitter_watch.get_or_insert_with(|| FullWatch::of_words(device, &[200, 203]));
        let Some(wanted) = watch.collect() else {
            return Ok(());
        };
        if wanted <= self.gi.emitters_max || self.gi.emitters_max >= EMITTERS_CAP {
            return Ok(());
        }
        self.gi.emitters_max = wanted.next_power_of_two().min(EMITTERS_CAP);
        self.gi.emitter_watch = None;
        let slots = self.gi.resources.as_ref().map_or(0, |r| r.slots);
        if slots != 0 {
            self.allocate_gi(slots)?;
        }
        Ok(())
    }
    fn allocate_gi(&mut self, slots: u32) -> Result<(), Error> {
        {
            let Sizes {
                misses,
                cold_words,
                raw_slots,
                table: table_bytes,
                work: work_bytes,
                outgoing: outgoing_bytes,
                draws: draws_bytes,
                raw: raw_bytes,
            } = Sizes::of(slots, self.lighting.pending.count, &self.gi.settings, self.gi.emitters_max);
            let directions = self.gi.settings.directions;
            let limits = self.device.limits();
            if table_bytes
                > limits
                    .max_buffer_size
                    .min(u64::from(limits.max_storage_buffer_binding_size))
            {
                return Err(Error::InvalidInput(
                    "GI cell light lists exceed device buffer limits; reduce slots_log2 or light count",
                ));
            }
            let buffer = |label, size, usage| {
                self.ledger.own(
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: usage | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ResourceKind::Buffer,
                )
            };
            let storage = wgpu::BufferUsages::STORAGE;
            let resources = Resources {
                table: buffer(
                    "indirect cache table",
                    table_bytes,
                    storage | wgpu::BufferUsages::COPY_SRC,
                ),
                work: buffer(
                    "indirect cache work",
                    work_bytes,
                    storage | wgpu::BufferUsages::COPY_SRC,
                ),
                frame: buffer(
                    "indirect cache frame",
                    FRAME_BYTES as u64,
                    wgpu::BufferUsages::UNIFORM,
                ),
                outgoing: buffer(
                    "indirect cache outgoing radiance",
                    outgoing_bytes,
                    storage | wgpu::BufferUsages::COPY_SRC,
                ),
                draws: buffer("indirect cache draw table", draws_bytes, storage),
                raw: buffer("indirect cache filter sources", raw_bytes, storage | if queue_probe::enabled() {wgpu::BufferUsages::COPY_SRC} else {wgpu::BufferUsages::empty()}),
                external: buffer(
                    "indirect cache external requests",
                    EXTERNAL_BYTES,
                    // COPY_SRC: the count's readback (external_watch).
                    storage | wgpu::BufferUsages::COPY_SRC,
                ),
                slots,
                cold_words,
                directions,
                misses,
                raw_slots,
                emitters: self.gi.emitters_max,
                _allocations: [
                    self.ledger.allocate(AllocationKind::Light, table_bytes),
                    self.ledger.allocate(AllocationKind::Light, work_bytes),
                    self.ledger.allocate(AllocationKind::Light, outgoing_bytes),
                    self.ledger.allocate(AllocationKind::Light, draws_bytes),
                    self.ledger.allocate(AllocationKind::Light, raw_bytes),
                    self.ledger.allocate(AllocationKind::Light, EXTERNAL_BYTES),
                ],
            };
            self.gi.stats.table_bytes = table_bytes;
            self.gi.stats.slots = slots;
            self.gi.stats.work_bytes = work_bytes;
            self.gi.stats.bounce_bytes =
                outgoing_bytes + draws_bytes + (2 * (1 + u64::from(slots)) + u64::from(slots)) * 4;
            self.gi.stats.filter_bytes = raw_bytes;
            self.gi.draw_table.clear();
            if let Some(old) = self.gi.resources.replace(resources) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Resources(old)));
            }
            if let Some(old) = self.gi.groups.take() {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Groups(old)));
            }
            self.gi.queue_probe = None;
            self.gi.generation += 1;
            // LC2: a larger light bitset creates an empty table, including after a long
            // idle view. Reset its normal empty-cache state before any idle shortcut.
            self.gi.planner.reallocated();
        }
        Ok(())
    }
    // LC2 measurement-only access to the light visibility bitset (no raw table dump).
    #[cfg(test)]
    pub(super) fn test_lc2_gi_table(&self) -> Option<&wgpu::Buffer> {
        self.gi.resources.as_ref().map(|r| &*r.table)
    }
    pub fn indirect(&self) -> IndirectSettings {
        self.gi.settings
    }
    pub fn indirect_stats(&self) -> IndirectStats {
        let mut stats = self.gi.stats;
        stats.geometry_bytes = self.gi.packed.as_ref().map_or(0, |p| p.bytes);
        stats.near_bytes = self.gi.packed.as_ref().map_or(0, |p| p.near_bytes);
        stats
    }
    /// Empties the cache on the next frame: a camera cut or scene replacement that
    /// should not reuse earlier cells.
    pub fn reset_indirect(&mut self) {
        self.gi.planner.reset();
    }
    #[cfg(test)]
    pub(crate) fn test_indirect_age(&self) -> u32 {
        self.gi.planner.since_clear()
    }
    /// Software-ray geometry for `mesh` on the non-RT path: a full-detail BVH built now
    /// from the mesh's object-space triangles. Hardware renderers ignore it unless
    /// `force_software` is set.
    pub fn set_indirect_geometry(
        &mut self,
        mesh: &Mesh,
        positions: &[[f32; 3]],
        indices: &[u32],
    ) -> Result<(), Error> {
        let bvh = gi_bvh::build(positions, indices)?;
        let blas = Blas::with_primitives(bvh, indices)?;
        self.set_indirect_blas(mesh, blas)
    }
    /// P-S4B: `mesh`'s software geometry when it is the mesh's own full-detail triangles
    /// (`Blas::exact`).
    pub(super) fn exact_indirect_geometry(&self, mesh: &Mesh) -> Option<&Blas> {
        let key: &wgpu::Buffer = &mesh.vertex;
        self.gi
            .meshes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, b)| &**b)
            .filter(|b| b.exact)
    }
    /// Software-ray geometry from a cooked cluster BVH (an R0c sidecar), traced at the
    /// given level: one complete cut.
    pub fn set_indirect_bvh(
        &mut self,
        mesh: &Mesh,
        bvh: crate::asset::cluster_bvh::Bvh,
        level: usize,
    ) -> Result<(), Error> {
        self.set_indirect_blas(mesh, Blas::new(bvh, level)?)
    }
    /// Software-ray geometry for `mesh` from mesh `index` of a cooked near field
    /// (renderlab_asset::near_field, docs/m7-g1c.md): its far-field cut, traced at runtime in
    /// place of full detail. When the field has masks, each instance whose transform has the
    /// exact bits of one of the field's placements of this mesh reads that placement's masks:
    /// a cell on it also blocks the directions all three vertex masks block. Every cooked mesh
    /// with masks must share the field's `bins`.
    pub fn set_indirect_near_field(
        &mut self,
        mesh: &Mesh,
        field: &crate::asset::near_field::NearField,
        index: usize,
    ) -> Result<(), Error> {
        let bins = field.settings.bins;
        if bins != 0 && self.gi.near_bins != 0 && self.gi.near_bins != bins {
            return Err(Error::InvalidInput(
                "near-field masks must share their bins",
            ));
        }
        let bvh = field
            .meshes
            .get(index)
            .ok_or(Error::InvalidInput("near-field mesh index"))?;
        let mut blas = Blas::new(bvh.clone(), 0)?;
        blas.near = field
            .placements
            .iter()
            .filter(|p| p.mesh as usize == index && !p.masks.is_empty())
            .map(|p| gi_bvh::NearMasks {
                model: Mat4::from_cols_array_2d(&p.transform),
                masks: p.masks.clone(),
            })
            .collect();
        if bins != 0 {
            self.gi.near_bins = bins;
        }
        // Mask-free fields also trace as one merged world BVH where their placements allow.
        let reuse = self.gi.world.as_ref().is_some_and(|w| {
            w.source == field.source
                && w.placements.len() == field.placements.len()
                && w.placements
                    .iter()
                    .zip(&field.placements)
                    .all(|((m, bits), p)| *m == p.mesh && *bits == transform_bits(&p.transform))
        });
        if bins != 0 {
            self.gi.world = None;
        } else if !reuse {
            self.gi.world = Some(World::new(field)?);
        }
        let key: wgpu::Buffer = (**mesh.vertex).clone();
        self.set_indirect_blas(mesh, blas)?;
        if let Some(slot) = self.gi.world.as_mut().and_then(|w| w.meshes.get_mut(index)) {
            *slot = Some(key);
        }
        Ok(())
    }
    pub(crate) fn set_indirect_blas(&mut self, mesh: &Mesh, blas: Blas) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.identity, &mesh.renderer) {
            return Err(Error::InvalidInput("mesh belongs to another renderer"));
        }
        let key: wgpu::Buffer = (**mesh.vertex).clone();
        let before = self.gi.meshes.len();
        self.gi.meshes.retain(|(k, _)| *k != key);
        let replaced = self.gi.meshes.len() != before;
        // Whatever this mesh traced before, it no longer stands for a field mesh.
        for m in self.gi.world.iter_mut().flat_map(|w| &mut w.meshes) {
            if m.as_ref() == Some(&key) {
                *m = None;
            }
        }
        self.gi.meshes.push((key, Arc::new(blas)));
        if self.gi.meshes.iter().all(|(_, b)| b.near.is_empty()) {
            self.gi.near_bins = 0;
        }
        if let Some(old) = self.gi.packed.take() {
            self.gi
                .retired
                .push((self.submissions, Retired::Packed(old)));
        }
        // A replaced mesh changes what placed cells traced. A new mesh is not placed yet (its
        // placements reach the planner as new bounds, which re-trace the cells around them), but
        // the clear stays the default: a full table relit at once (Glint's goal changes its
        // lights as its new meshes appear) queued 633k gathers that ran into the held frames,
        // where main's clear left a small table. RENDERLAB_GI_NEW_MESH_CLEAR=0 keeps the table
        // for new meshes (an embedder that rebuilds batches every frame emptied it each frame).
        static CLEAR_NEW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if replaced || *CLEAR_NEW.get_or_init(|| std::env::var("RENDERLAB_GI_NEW_MESH_CLEAR").map_or(true, |v| v != "0")) {
            self.gi.planner.set_pending_clear(true);
        }
        Ok(())
    }
    /// Per-pass GPU timing of later frames into a readback (`indirect_profile`).
    pub fn set_indirect_profiling(&mut self, enabled: bool) {
        if !enabled
            || !self
                .device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            self.gi.profile = None;
            return;
        }
        if self.gi.profile.is_some() {
            return;
        }
        let priority = priority::on();
        let stamps = PROFILE_STAMPS + if priority { 2 * (2 + 2 * CLAIM_ROUNDS) } else { 0 };
        let stride = (u64::from(stamps) * 8).div_ceil(PROFILE_STRIDE) * PROFILE_STRIDE;
        let queries = self.ledger.own(
            self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("indirect cache passes"),
                ty: wgpu::QueryType::Timestamp,
                count: stamps * PROFILE_RING as u32,
            }),
            ResourceKind::QuerySet,
        );
        let bytes = stride * PROFILE_RING;
        let buffer = |label, usage| {
            self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        self.gi.profile = Some(Profile {
            queries,
            resolve: buffer(
                "indirect cache timestamps",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "indirect cache timestamp readback",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            period: self.queue.get_timestamp_period(),
            written: 0,
            stamps,
            stride,
            priority,
        });
    }
    /// Blocks until the GPU is idle and reads the last profiled frame's pass times.
    pub fn indirect_profile(&mut self) -> Result<Option<IndirectProfile>, Error> {
        Ok(self.indirect_profiles()?.pop())
    }
    /// Blocks until the GPU is idle and reads the pass times of up to the last 64
    /// profiled cache frames, oldest first. Idle frames run no pass and add none.
    pub fn indirect_profiles(&mut self) -> Result<Vec<IndirectProfile>, Error> {
        let Some(profile) = self.gi.profile.as_ref().filter(|p| p.written > 0) else {
            return Ok(vec![]);
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            &profile.resolve,
            0,
            &profile.readback,
            0,
            profile.stride * PROFILE_RING,
        );
        self.queue.submit([encoder.finish()]);
        let slice = profile.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let words: Vec<u64> = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        profile.readback.unmap();
        let period = f64::from(profile.period) / 1000.0;
        let rounds = CLAIM_ROUNDS as usize;
        let first = profile.written.saturating_sub(PROFILE_RING);
        Ok((first..profile.written)
            .map(|frame| {
                let at = ((frame % PROFILE_RING) * profile.stride / 8) as usize;
                let pass = |i: usize| {
                    words[at + 2 * i + 1].saturating_sub(words[at + 2 * i]) as f64 * period
                };
                IndirectProfile {
                    probe: pass(0),
                    claim: (0..rounds).map(|k| pass(1 + 2 * k)).sum::<f64>()
                        + if profile.priority { (0..rounds).map(|k|pass(PROFILE_STAMPS as usize/2+2+2*k)).sum() } else {0.0},
                    resolve: (0..rounds).map(|k| pass(2 + 2 * k)).sum::<f64>()
                        + if profile.priority { (0..rounds).map(|k|pass(PROFILE_STAMPS as usize/2+3+2*k)).sum() } else {0.0},
                    scan: pass(1 + 2 * rounds),
                    setup: pass(2 + 2 * rounds),
                    rays: pass(3 + 2 * rounds),
                    finish: pass(4 + 2 * rounds),
                    integrate: pass(5 + 2 * rounds),
                    gather: pass(6 + 2 * rounds) + if profile.priority {pass(PROFILE_STAMPS as usize/2)+pass(PROFILE_STAMPS as usize/2+1)} else {0.0},
                    light: pass(7 + 2 * rounds),
                    shade: pass(8 + 2 * rounds),
                    filter: pass(9 + 2 * rounds),
                }
            })
            .collect())
    }
    /// Blocks until the GPU is idle and reads the last cache frame's counters.
    pub fn indirect_counters(&mut self) -> Result<Option<IndirectCounters>, Error> {
        let Some(r) = self.gi.resources.as_ref() else {
            return Ok(None);
        };
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect cache counters"),
            size: COUNTER_WORDS * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.work, 0, &readback, 0, COUNTER_WORDS * 4);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let w: Vec<u32> = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Some(IndirectCounters {
            misses: w[0],
            traces: w[1],
            integrates: w[2],
            miss_overflow: w[3],
            full: w[4],
            orphans: w[5],
            local_overflow: w[6],
            created: w[7],
            invalidated: w[8],
            refreshed: w[9],
            lost: w[10],
            rays: w[11],
            ray_items: w[12],
            orphans_no_surface: w[13],
            orphans_inside: w[14],
            splits: w[15],
            probe: ProbeCounters {
                tiles: w[16],
                pixels: w[17],
                inserts: w[18],
                steps: w[19],
                cells: w[20],
                parents: w[21],
                refined: w[22],
                finds: w[23],
                found: w[24],
                walks: w[25],
                refine_tiles: w[26],
                max_cells: w[27],
                tiles_by_cells: [w[28], w[29], w[30], w[31]],
            },
            shades: w[32],
            gathers: w[33],
            gather_rays: w[34],
            gather_misses: w[35],
            gather_requests: w[36],
            light_rays: w[37],
            bounce_queued: w[38],
            gather_absent: w[39],
            emitters: w[200],
            emitter_draws: w[201],
            emitter_rays: w[202],
            emitters_dropped: w[203],
            emitters_unevaluated: w[208],
            emitters_zero: w[209],
            gather_unshaded: w[40],
            gather_orphans: w[41],
            gather_coarse_orphan: w[204],
            gather_coarse_absent: w[205],
            gather_deferred: w[42],
            claim_deferred: w[43],
            external_requests: w[220],
            external_overflow: w[221],
            fg_rays: w[222],
            fg_shadow_rays: w[223],
            evicted: w[44],
            evicted_recent: w[45],
            full_pixel: w[46],
            gather_estimate: w[256..352].iter().fold(0u32, |a, &b| a.saturating_add(b)),
            gather_cuts: [w[352], w[353]],
            aimed_rays: w[250],
            aimed_occluded: w[251],
            created_secondary: w[253],
            created_tertiary: w[254],
            evicted_secondary: w[255],
            moved_boxes: w[252],
            invalidated_read: [w[241], w[242]],
            invalidated_cube: w[246],
            reshaded: [w[243], w[244], w[245]],
            reshade_skipped: w[370],
            shade_causes: [w[210], w[211], w[212], w[213], w[214]],
            trace_partial: w[215],
        }))
    }
    /// Arms an incoming-queue snapshot after pixel probing. Measurement only; requires
    /// RENDERLAB_GI_QUEUE_DIAG before allocation. Reads roughly table+raw+queue bytes.
    pub fn begin_indirect_queue_probe(&mut self) -> Result<(),Error> {
        if !queue_probe::enabled() {return Err(Error::InvalidInput("GI queue diagnostic is unset"));}
        let r=self.gi.resources.as_ref().ok_or(Error::InvalidInput("GI queue diagnostic needs a cache"))?;
        if let Some(probe)=self.gi.queue_probe.as_mut() {probe.arm();}
        else {self.gi.queue_probe=Some(queue_probe::State::new(&self.device,r));}
        Ok(())
    }
    /// First visible, refresh visible, background, ineligible, duplicates, queue entries,
    /// and visible first-fill cells with filtered neighbour data. Zero raw bounce means
    /// never gathered; the nonzero negative-zero sentinel means gathered with zero light.
    pub fn indirect_queue_probe(&mut self) -> Result<[u32;7],Error> {
        self.gi.queue_probe.as_ref().ok_or(Error::InvalidInput("GI queue probe was not armed"))?.read(&self.device)
    }
    /// Blocks until the GPU is idle and copies the whole table (determinism gates).
    pub fn indirect_table(&mut self) -> Result<Option<Vec<u32>>, Error> {
        let words = self.gi.resources.as_ref().map_or(0, |r| r.table.size() / 4);
        self.indirect_table_words(0, words)
    }
    /// Blocks until the GPU is idle and copies `count` table words from word `at` (the header,
    /// the emitter list).
    pub(crate) fn indirect_table_words(&mut self, at: u64, count: u64) -> Result<Option<Vec<u32>>, Error> {
        let Some(r) = self.gi.resources.as_ref() else {
            return Ok(None);
        };
        let size = count * 4;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect cache table readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.table, at * 4, &readback, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let words = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Some(words))
    }

    /// Blocks until the GPU is idle and copies every slot's outgoing radiance (G2: rgb, w 1
    /// once cell shading wrote it), for diagnostics beside `indirect_table`.
    pub fn indirect_outgoing(&mut self) -> Result<Option<Vec<[f32; 4]>>, Error> {
        let Some(r) = self.gi.resources.as_ref() else {
            return Ok(None);
        };
        let size = r.outgoing.size();
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect cache outgoing readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.outgoing, 0, &readback, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let values = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(16)
            .map(|b| {
                std::array::from_fn(|k| f32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap()))
            })
            .collect();
        Ok(Some(values))
    }

    /// Whether any draw's program reads the cache (naga-free token check of its source).
    fn gi_callers(&self, draws: &[PreparedDraw<'_>]) -> bool {
        let standard = self.standard.id();
        draws.iter().any(|d| {
            let id = d.material.map_or(standard, |m| m.program.id());
            crate::program::uses_indirect(&self.program_slot(id).source)
        })
    }

    /// Each draw's stable id (G2): an index + 1 into the (vertex buffer, material) pairs the
    /// cache has seen, so a cell's material names its placement whatever the frame's draw
    /// order. The list only grows; past 0xfffe pairs new ones get 0 and reflect nothing.
    fn gi_draw_ids(&mut self, draws: &[PreparedDraw<'_>]) -> Vec<u32> {
        let default = self.default_material.as_ref();
        let keys: Vec<(wgpu::Buffer, wgpu::Buffer)> = draws
            .iter()
            .filter_map(|d| {
                let m = d.material.or(default)?;
                Some(((**d.mesh.vertex).clone(), (*m._uniform).clone()))
            })
            .collect();
        if keys.len() != draws.len() {
            return vec![0; draws.len()];
        }
        keys.into_iter()
            .map(|key| {
                let ids = &mut self.gi.draw_ids;
                match ids.iter().position(|k| *k == key) {
                    Some(i) => i as u32 + 1,
                    None if ids.len() < 0xfffe => {
                        ids.push(key);
                        ids.len() as u32
                    }
                    None => 0,
                }
            })
            .collect()
    }

    /// Compiles the frame's program's cell shading (G2) when it declares `scatter` or
    /// `emission`; false when it declares neither (it blocks light but reflects none) or the
    /// frame has no traced shading groups. A mixed-program frame (MP1) compiles one cell
    /// pipeline per reflecting program instead; cells of the others reflect nothing.
    fn ensure_gi_shade(&mut self, software: bool) -> Result<bool, Error> {
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(false);
        };
        if state.shading_groups().is_none() {
            return Ok(false);
        }
        let streamed = state.requests;
        let programs = if state.programs.len() > 1 {
            state.programs.clone()
        } else {
            vec![state.program]
        };
        let mixed = programs.len() > 1;
        self.gi.mixed_active = mixed;
        let mut frame = vec![];
        for id in programs {
            if let Some(entry) = self.gi_shade_entry(id, software, streamed, mixed)? {
                frame.push(entry);
            }
        }
        let any = !frame.is_empty();
        self.gi.frame_shading = frame;
        Ok(any)
    }

    /// Program `id`'s cell shading entry in `gi.shading` (single or `mixed`), built if missing;
    /// None when the program reflects no light. Frames and `prepare_gi_shade` call it.
    fn gi_shade_entry(&mut self, id: u64, software: bool, streamed: bool, mixed: bool) -> Result<Option<usize>, Error> {
        if !self.program_slot(id).traits.reflects {
            return Ok(None);
        }
        let key = self.gi_shade_key(id, software, streamed);
        if let Some(i) = self.gi.shading.iter().position(|p| p.key == key && p.mixed == mixed) {
            return Ok(Some(i));
        }
        let pipeline = self.gi_shade_pipeline(id, key, mixed)?;
        // One entry per program and variant: an edited program's old entry is replaced.
        if let Some(i) = self.gi.shading.iter().position(|p| {
            p.mixed == mixed && (p.key.0, p.key.2, p.key.3) == (key.0, key.2, key.3)
        }) {
            self.gi.shading[i] = pipeline;
            return Ok(Some(i));
        }
        self.gi.shading.push(pipeline);
        Ok(Some(self.gi.shading.len() - 1))
    }

    /// Cell shading for every program in `ids`, as a frame drawing only that program and, when
    /// there are several, as a frame drawing all of them would select it. Prepare calls it.
    pub(super) fn prepare_gi_shade(&mut self, ids: &[u64]) -> Result<(), Error> {
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        // Layouts only: the geometry groups a frame binds need not exist yet.
        if !self.gi.settings.enabled || !self.gi.settings.bounce {
            return Ok(());
        }
        let streamed = state.requests;
        let software = self.hardware_shadows.is_none() || self.gi.settings.force_software;
        let mut wanted = vec![];
        for &id in ids {
            if !self.program_slot(id).traits.reflects {
                continue;
            }
            let key = self.gi_shade_key(id, software, streamed);
            for mixed in [false, true].into_iter().take(if ids.len() > 1 { 2 } else { 1 }) {
                if !self.gi.shading.iter().any(|p| p.key == key && p.mixed == mixed) {
                    wanted.push((id, key, mixed));
                }
            }
        }
        if wanted.is_empty() {
            return Ok(());
        }
        let parts = wanted
            .iter()
            .map(|&(id, _, mixed)| self.gi_shade_parts(id, mixed))
            .collect::<Result<Vec<_>, _>>()?;
        let (device, ledger) = (&self.device, &self.ledger);
        let built = super::program_gpu::in_parallel(
            device,
            parts.iter().map(|(module, layout)| move || shade_cells_pipeline(device, ledger, module, layout)),
        );
        for ((_, key, mixed), (pipeline, error)) in wanted.into_iter().zip(built) {
            if let Some(error) = error {
                return Err(Error::Shader(error.to_string()));
            }
            let entry = ShadePipeline { key, mixed, pipeline };
            // As gi_shade_entry: an edited program's old entry for this variant is replaced.
            match self.gi.shading.iter().position(|p| {
                p.mixed == mixed && (p.key.0, p.key.2, p.key.3) == (key.0, key.2, key.3)
            }) {
                Some(i) => self.gi.shading[i] = entry,
                None => self.gi.shading.push(entry),
            }
        }
        Ok(())
    }

    fn gi_shade_key(&self, id: u64, software: bool, streamed: bool) -> (u64, [u8; 32], bool, bool) {
        let hash: [u8; 32] = Sha256::new()
            .chain_update(self.program_slot(id).source.as_bytes())
            .chain_update(self.source.as_bytes())
            .chain_update(self.gi.settings.directions.to_le_bytes())
            .finalize()
            .into();
        (id, hash, software, streamed)
    }

    /// Program `id`'s cell shading pipeline: the single-program entry, or with `mixed`
    /// the entry that shades only its own draws' cells (`mixed_cell_entry`).
    fn gi_shade_pipeline(
        &mut self,
        id: u64,
        key: (u64, [u8; 32], bool, bool),
        mixed: bool,
    ) -> Result<ShadePipeline, Error> {
        let (module, layout) = self.gi_shade_parts(id, mixed)?;
        let scopes = OperationScopes::new(&self.device);
        let pipeline = shade_cells_pipeline(&self.device, &self.ledger, &module, &layout);
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(error.to_string()));
        }
        Ok(ShadePipeline { key, mixed, pipeline })
    }

    /// Program `id`'s cell shading module (with `mixed`, the entry that shades only its own
    /// draws' cells) and its pipeline layout: everything of the pipeline but the driver's
    /// compile (`shade_cells_pipeline`).
    fn gi_shade_parts(
        &mut self,
        id: u64,
        mixed: bool,
    ) -> Result<(Owned<wgpu::ShaderModule>, Owned<wgpu::PipelineLayout>), Error> {
        let source = self.program_slot(id).source.clone();
        self.ensure_gi_layouts();
        let heads = if self.hardware_shadows.is_some() {
            super::primary_ray_gpu::shading_heads(&self.source)?
        } else {
            super::primary_ray_gpu::non_rt_sources(&self.source)?.1
        };
        let composed = cell_shading_source(
            &heads[1],
            &source,
            self.gi.settings.directions,
            mixed.then_some(id),
        );
        let module = self.ledger.own(
            self.device
                // It composes program text: checked, as every program module (P-SPLIT).
                .create_program_module(wgpu::ShaderModuleDescriptor {
                    label: Some("indirect cache cell shading"),
                    source: wgpu::ShaderSource::Wgsl(composed.source.into()),
                }),
            ResourceKind::ShaderModule,
        );
        let state = self.view.primary.as_ref().unwrap();
        let layout = self.ledger.own(
            self.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("indirect cache cell shading ABI"),
                    bind_group_layouts: &[
                        Some(&self.gi.layouts.as_ref().unwrap().shade),
                        Some(state.array_layout()),
                        Some(state.material_layout()),
                        Some(&self.lighting.bindings),
                    ],
                    immediate_size: 0,
                }),
            ResourceKind::PipelineLayout,
        );
        Ok((module, layout))
    }

    fn ensure_gi_layouts(&mut self) {
        if self.gi.layouts.is_some() {
            return;
        }
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            self.ledger.own(
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some(label),
                        entries,
                    }),
                ResourceKind::BindGroupLayout,
            )
        };
        let common = [
            uint_texture(7),
            uint_texture(21),
            storage(30, false),
            storage(31, false),
            entry(
                32,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(FRAME_BYTES as u64),
                },
            ),
            storage(33, false),
            storage(36, true),
            storage(37, false),
            storage(38, false),
        ];
        let tlas = entry(
            34,
            wgpu::BindingType::AccelerationStructure {
                vertex_return: true,
            },
        );
        // Software rays bind their geometry at 35 on every device; the TLAS entry at 34
        // exists only where ray queries do.
        let software = [&common[..], &[storage(35, true)]].concat();
        let rt = self
            .device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        // Ray hardware also binds the see-through table at 40 (gi_rays_hardware.wgsl).
        let hardware = if rt {
            [&common[..], &[tlas, storage(40, true)]].concat()
        } else {
            software.clone()
        };
        let main = [
            group("indirect cache, hardware rays", &hardware),
            group("indirect cache, software rays", &software),
        ];
        let per_draw = |e: wgpu::BindGroupLayoutEntry| wgpu::BindGroupLayoutEntry {
            count: std::num::NonZeroU32::new(TRACED_DRAWS),
            ..e
        };
        let arrays = group(
            "indirect cache geometry",
            &[
                storage(0, true),
                storage(1, true),
                per_draw(storage(2, true)),
                per_draw(storage(3, true)),
                storage(5, true),
            ],
        );
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            self.ledger.own(
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: groups,
                        immediate_size: 0,
                    }),
                ResourceKind::PipelineLayout,
            )
        };
        let lighting = &self.lighting.bindings;
        let pipelines = [
            layout(
                "indirect cache ABI, hardware",
                &[Some(&main[0]), Some(&arrays), None, Some(lighting)],
            ),
            layout(
                "indirect cache ABI, software",
                &[Some(&main[1]), Some(&arrays), None, Some(lighting)],
            ),
        ];
        let lean_arrays = group(
            "indirect cache geometry, lean",
            &[storage(0, true), storage(1, true), storage(5, true)],
        );
        let lean_pipelines = [
            layout(
                "indirect cache ABI, hardware, lean",
                &[Some(&main[0]), Some(&lean_arrays), None, Some(lighting)],
            ),
            layout(
                "indirect cache ABI, software, lean",
                &[Some(&main[1]), Some(&lean_arrays), None, Some(lighting)],
            ),
        ];
        let shade = group(
            "indirect cache cell shading",
            &[storage(31, true), storage(33, false), storage(36, true)],
        );
        self.gi.layouts = Some(Layouts {
            main,
            arrays,
            pipelines,
            lean_arrays,
            lean_pipelines,
            shade,
        });
    }

    /// The cache's kernels for its tracing path, rebuilt when the path or direction count
    /// changes. Called each GI frame and by `prepare_scene_pipelines`.
    pub(super) fn ensure_gi_pipelines(&mut self, software: bool) -> Result<(), Error> {
        let directions = self.gi.settings.directions;
        if self
            .gi
            .pipelines
            .as_ref()
            .is_some_and(|p| p.software == software && p.directions == directions)
        {
            return Ok(());
        }
        self.ensure_gi_layouts();
        let scopes = OperationScopes::new(&self.device);
        let module = self.ledger.own(
            self.device
                .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("indirect cache kernels"),
                    source: wgpu::ShaderSource::Wgsl(kernel_source(software, directions).into()),
                }),
            ResourceKind::ShaderModule,
        );
        let layouts = self.gi.layouts.as_ref().unwrap();
        let compute_with =
            |entry: &str, layout: &wgpu::PipelineLayout, constants: &[(&str, f64)]| {
                self.ledger.own(
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: Some(layout),
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: wgpu::PipelineCompilationOptions {
                                constants,
                                ..Default::default()
                            },
                            cache: None,
                        }),
                    ResourceKind::ComputePipeline,
                )
            };
        let main = &layouts.pipelines[usize::from(software)];
        let lean_layout = &layouts.lean_pipelines[usize::from(software)];
        let lean_list = std::cell::RefCell::new(Vec::new());
        // A kernel goes on the lean layout when it validates there (it reads neither array).
        let build_with = |entry: &str, constants: &[(&str, f64)]| {
            if lean() {
                let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
                let pipeline = compute_with(entry, lean_layout, constants);
                if pollster::block_on(scope.pop()).is_none() {
                    return (pipeline, true);
                }
            }
            (compute_with(entry, main, constants), false)
        };
        let build = |entry: &str| build_with(entry, &[]);
        let priority = software && priority::on();
        // The set's kernels are created at once, as many together as the process's compile slots
        // allow (one in frames; four in prepare and the bench's reload), then taken by name below.
        // A kernel not listed here is created where it is asked for.
        let listed = [
            "gi_probe", "gi_external_requests", if priority { "gi_priority_claim_visible" } else { "gi_claim" },
            "gi_resolve", "gi_resolve_last", "gi_scan", "gi_setup", "gi_rays", "gi_finish", "gi_integrate",
            if priority { "gi_priority_gather_empty" } else if software { "gi_gather_batched" } else { "gi_gather" },
            "gi_fill_count", "gi_fill_select", "gi_emitters_build", "gi_emitters_publish", "gi_emitters_cdf",
            "gi_gather_count", "gi_gather_select", "gi_fixed_rank", "gi_fixed_select", "gi_fixed_list", "gi_fixed_after",
            "gi_light", "gi_filter",
        ];
        let mut first_error = None;
        let mut ready = std::collections::HashMap::new();
        for (name, (built, error)) in listed
            .into_iter()
            .zip(super::program_gpu::in_parallel(&self.device, listed.map(|name| move || build(name))))
        {
            first_error = first_error.or(error);
            ready.insert(name, built);
        }
        let ready = std::cell::RefCell::new(ready);
        let kernel = |entry: &str| {
            let (pipeline, lean) = ready.borrow_mut().remove(entry).unwrap_or_else(|| build(entry));
            if lean {
                lean_list.borrow_mut().push((*pipeline).clone());
            }
            pipeline
        };
        let clear_kernel = |entry: &str| {
            let (pipeline, lean) = build_with(entry, &[("GI_SW_SEE_THROUGH", 1.0)]);
            if lean {
                lean_list.borrow_mut().push((*pipeline).clone());
            }
            pipeline
        };
        let gather_entry = if priority {
            "gi_priority_gather_empty"
        } else if software {
            "gi_gather_batched"
        } else {
            "gi_gather"
        };
        let mut pipelines = Pipelines {
            software,
            priority: priority.then(|| priority::Pipelines::new(|name| kernel(name))),
            directions,
            probe: kernel("gi_probe"),
            external: kernel("gi_external_requests"),
            claim: kernel(if priority { "gi_priority_claim_visible" } else { "gi_claim" }),
            resolve: kernel("gi_resolve"),
            resolve_last: kernel("gi_resolve_last"),
            scan: kernel("gi_scan"),
            setup: kernel("gi_setup"),
            rays: kernel("gi_rays"),
            rays_clear: software.then(|| clear_kernel("gi_rays")),
            finish: kernel("gi_finish"),
            integrate: kernel("gi_integrate"),
            gather: kernel(gather_entry),
            gather_clear: (software && !priority).then(|| clear_kernel(gather_entry)),
            fill_count: kernel("gi_fill_count"),
            fill_select: kernel("gi_fill_select"),
            emitters_build: kernel("gi_emitters_build"),
            emitters_publish: kernel("gi_emitters_publish"),
            emitters_cdf: kernel("gi_emitters_cdf"),
            gather_count: kernel("gi_gather_count"),
            gather_select: kernel("gi_gather_select"),
            fixed_rank: kernel("gi_fixed_rank"),
            fixed_select: kernel("gi_fixed_select"),
            fixed_list: kernel("gi_fixed_list"),
            fixed_after: kernel("gi_fixed_after"),
            light: kernel("gi_light"),
            filter: kernel("gi_filter"),
            lean: vec![],
        };
        pipelines.lean = lean_list.into_inner();
        if let Some(error) = scopes.finish().or(first_error) {
            return Err(Error::Shader(error.to_string()));
        }
        self.gi.pipelines = Some(pipelines);
        Ok(())
    }

    fn prepare_gi_arrays(&mut self, draws: &[PreparedDraw<'_>]) -> Result<(), Error> {
        let keys: Vec<_> = draws
            .iter()
            .map(|d| {
                (
                    (**d.mesh.vertex).clone(),
                    (**d.mesh.index).clone(),
                    d.instances.len(),
                )
            })
            .collect();
        if self.gi.arrays.as_ref().is_none_or(|a| a.keys != keys) {
            let default = self.default_material.as_ref().unwrap();
            let placements = placement_table(draws.iter().map(|d| {
                let m = d.material.unwrap_or(default);
                (
                    d.instances.len(),
                    d.mesh.tangent.is_some(),
                    !m.double_sided,
                    m.thin,
                )
            }))?;
            let count = placements.len().max(1) as u64;
            let buffer = |label, size| {
                self.ledger.own(
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ResourceKind::Buffer,
                )
            };
            let models = buffer("indirect cache placement transforms", count * 64);
            let normals = buffer("indirect cache placement normals", count * 64);
            // The placement words, then each draw's batch word counted back from the end, as the
            // primary pass lays them out: `primary_batch_info` reads a draw's triangle count from
            // there (the emitter list, cell shading's emission probe). Without the tail it read a
            // placement word, and the list took 0 or another draw's count of triangles.
            let batches: Vec<u32> = draws
                .iter()
                .map(|d| {
                    let skinned = d.mesh.skin.as_ref().is_some_and(|s| s.previous.size() > d.mesh.vertex.size());
                    crate::primary_ray::batch_word(d.mesh.index.size() / 4, skinned)
                })
                .collect();
            let placements = crate::primary_ray::placement_words(&placements, &batches);
            let table = buffer("indirect cache placement draws", placements.len() as u64 * 4);
            let absent = buffer("indirect cache absent storage", 48);
            let words: Vec<u8> = placements.iter().flat_map(|v| v.to_ne_bytes()).collect();
            if !words.is_empty() {
                self.queue.write_buffer(&table, 0, &words);
            }
            self.ensure_gi_layouts();
            let slots = crate::primary_ray::bound_slots(&self.device, draws.len());
            fn pad<'a>(
                buffers: impl Iterator<Item = &'a wgpu::Buffer>,
                absent: &'a wgpu::Buffer,
                slots: usize,
            ) -> Vec<wgpu::BufferBinding<'a>> {
                buffers
                    .chain(std::iter::repeat(absent))
                    .take(slots)
                    .map(|b| b.as_entire_buffer_binding())
                    .collect()
            }
            let vertices = pad(draws.iter().map(|d| &**d.mesh.vertex), &absent, slots);
            let indices = pad(draws.iter().map(|d| &**d.mesh.index), &absent, slots);
            let bind = self.ledger.own(
                crate::group_cache::reported(&self.device, &wgpu::BindGroupDescriptor {
                    label: Some("indirect cache geometry"),
                    layout: &self.gi.layouts.as_ref().unwrap().arrays,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: models.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: normals.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::BufferArray(&vertices),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::BufferArray(&indices),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: table.as_entire_binding(),
                        },
                    ],
                }),
                ResourceKind::BindGroup,
            );
            drop(vertices);
            drop(indices);
            let lean = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("indirect cache geometry, lean"),
                    layout: &self.gi.layouts.as_ref().unwrap().lean_arrays,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: models.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: normals.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: table.as_entire_binding(),
                        },
                    ],
                }),
                ResourceKind::BindGroup,
            );
            let next = Arrays {
                lean,
                keys,
                models,
                normals,
                _placements: table,
                _absent: absent,
                revisions: vec![],
                revision_bytes: vec![],
                bind,
            };
            if let Some(old) = self.gi.arrays.replace(next) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Arrays(old)));
            }
        }
        let revisions: Vec<(u64, u64)> = draws
            .iter()
            .map(|d| d.instances.state.revision())
            .collect();
        let a = self.gi.arrays.as_mut().unwrap();
        if a.revisions != revisions {
            // The same bytes the traced shading tables publish, in placement order.
            let mut instances = vec![];
            for d in draws {
                for model in &d.instances.state.models {
                    instances.extend_from_slice(&matrix_bytes(*model));
                }
            }
            for d in draws {
                for normal in &d.instances.state.normals {
                    instances.extend_from_slice(&normal.bytes());
                }
            }
            let (models, normals) = instances.split_at(instances.len() / 2);
            if !models.is_empty() {
                self.queue.write_buffer(&a.models, 0, models);
                self.queue.write_buffer(&a.normals, 0, normals);
            }
            a.revision_bytes = revisions
                .iter()
                .flat_map(|(id, edits)| id.to_ne_bytes().into_iter().chain(edits.to_ne_bytes()))
                .collect();
            a.revisions = revisions;
        }
        Ok(())
    }

    /// Packs every draw's software geometry (plus static see-through draws for non-RT
    /// transmission occlusion); false when an opaque draw has none. `ids` are the draws'
    /// stable ids (`gi_draw_ids`), which name each instance's material (G2).
    fn prepare_gi_geometry(
        &mut self,
        draws: &[PreparedDraw<'_>],
        transparent: &[Draw<'_>],
        ids: &[u32],
    ) -> Result<bool, Error> {
        let mut blases = vec![];
        for d in draws {
            let key: &wgpu::Buffer = &d.mesh.vertex;
            let Some((_, blas)) = self.gi.meshes.iter().find(|(k, _)| k == key) else {
                return Ok(false);
            };
            blases.push(blas.clone());
        }
        let see_through_on = std::env::var("RENDERLAB_GI_SEE_THROUGH").as_deref() != Ok("0");
        let mut clear_draws: Vec<(&Draw<'_>, Arc<Blas>)> = vec![];
        if see_through_on {
            for d in transparent {
                if d.mesh.skin.is_some() {
                    continue;
                }
                let key: &wgpu::Buffer = &d.mesh.vertex;
                if let Some((_, blas)) = self.gi.meshes.iter().find(|(k, _)| k == key) {
                    clear_draws.push((d, blas.clone()));
                }
            }
        }
        let meshes: Vec<wgpu::Buffer> = draws
            .iter()
            .map(|d| (**d.mesh.vertex).clone())
            .chain(clear_draws.iter().map(|(d, _)| (**d.mesh.vertex).clone()))
            .collect();
        let (covered, drawn) = self
            .gi
            .world
            .as_ref()
            .filter(|_| self.gi.near_bins == 0)
            .and_then(|w| w.covers(draws))
            .unwrap_or_default();
        // The world BVH is one identity instance in place of the instances it covers.
        let instances: usize = draws.iter().map(|d| d.instances.len()).sum::<usize>()
            - covered.iter().filter(|&&c| c).count()
            + usize::from(!covered.is_empty())
            + clear_draws.iter().map(|(d, _)| d.instances.len()).sum::<usize>();
        if super::change_log::enabled() {
            let why = match self.gi.packed.as_ref() {
                None => "none".to_owned(),
                Some(p) => format!(
                    "{}{}{}{}",
                    if p.meshes != meshes { "meshes " } else { "" },
                    if p.instances != instances { "instances " } else { "" },
                    if p.covered != covered { "covered " } else { "" },
                    if p.ids != ids { "ids" } else { "" }
                ),
            };
            super::change_log::report(
                self.frames + 1,
                "gi_pack",
                &[],
                &[("repack_because", format!("\"{}\"", why.trim())), ("instances", instances.to_string())],
            );
        }
        if self.gi.packed.as_ref().is_none_or(|p| {
            p.meshes != meshes || p.instances != instances || p.covered != covered || p.ids != ids
        }) {
            // Pack each draw's mesh unless the world covers all its instances, then static
            // see-through meshes, then the world last.
            let mut entry = vec![];
            let mut refs: Vec<&Blas> = vec![];
            let mut at = 0;
            for (d, blas) in draws.iter().zip(&blases) {
                let n = d.instances.len();
                let open = covered.is_empty() || covered[at..at + n].iter().any(|&c| !c);
                at += n;
                entry.push(open.then(|| {
                    refs.push(&**blas);
                    refs.len() - 1
                }));
            }
            for (_, blas) in &clear_draws {
                refs.push(&**blas);
                entry.push(Some(refs.len() - 1));
            }
            let world = self.gi.world.as_ref().filter(|_| !covered.is_empty());
            if let Some(w) = world {
                refs.push(&w.blas);
            }
            let (offsets, near, total) = gi_bvh::layout(&refs, instances);
            let mut words = gi_bvh::mesh_words(&refs, &offsets, &near, total);
            // The merged world's placement map after everything else (G2): per placement, its
            // first position and material identity + 1, in position order
            // (shaders/gi_rays_software.wgsl gi_hit_identity).
            let mut map = (0, 0);
            if let Some(w) = world {
                map = (words.len() as u32, w.bases.len() as u32);
                for (base, &(d, i)) in w.bases.iter().zip(&drawn) {
                    words.extend([*base, draw_identity(ids, d, i)]);
                }
            }
            let bytes = (words.len() * 4) as u64;
            let near_bytes = refs
                .iter()
                .flat_map(|b| &b.near)
                .map(|n| n.masks.len() as u64 * 4)
                .sum();
            let buffer = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("indirect cache software geometry"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            self.queue.write_buffer(&buffer, 0, &raw);
            let packed = Packed {
                meshes,
                instances,
                covered,
                ids: ids.to_vec(),
                map,
                entry,
                offsets,
                near,
                near_bytes,
                head: vec![],
                buffer,
                bytes,
                _allocation: self.ledger.allocate(AllocationKind::Light, bytes),
            };
            if let Some(old) = self.gi.packed.replace(packed) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Packed(old)));
            }
            if let Some(old) = self.gi.groups.take() {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Groups(old)));
            }
        }
        let p = self.gi.packed.as_mut().unwrap();
        let mut records = vec![];
        // Per record, instance words 24-26 (G2): its material identity + 1, or for the merged
        // world zero and its placement map's offset and length; word 27: bit 0 if it casts no
        // shadow (light visibility skips it), bit 1 if see-through (words 28-30 carry packed
        // base colour, coverage and IOR, matching hardware_shadow_gpu::write_see_through).
        let mut extra: Vec<[u32; 8]> = vec![];
        if !p.covered.is_empty() {
            let w = &self.gi.world.as_ref().unwrap().blas;
            records.push((Mat4::IDENTITY, &**w, *p.offsets.last().unwrap(), 0));
            extra.push([0, p.map.0, p.map.1, 0, 0, 0, 0, 0]);
        }
        let mut at = 0;
        for (di, ((d, blas), entry)) in draws.iter().zip(&blases).zip(&p.entry).enumerate() {
            for (ii, model) in d.instances.state.models.iter().enumerate() {
                let placement = at as u32;
                let covered = p.covered.get(at).copied().unwrap_or(false);
                at += 1;
                if covered {
                    continue;
                }
                let e = entry.unwrap();
                let prim_base = if blas.primitives.is_empty() {
                    0
                } else {
                    p.offsets[e][3] + blas.bvh.indices.len() as u32
                };
                records.push((
                    *model,
                    &**blas,
                    p.offsets[e],
                    blas.near_base(model, &p.near[e]),
                ));
                extra.push([
                    draw_identity(ids, di, ii),
                    0,
                    0,
                    u32::from(!d.instances.state.casts_shadows(ii)),
                    prim_base,
                    placement,
                    0,
                    0,
                ]);
            }
        }
        let opaque_count = records.len();
        let pack_f16 = |a: f32, b: f32| {
            u32::from(crate::transparency::f16_bits(a))
                | (u32::from(crate::transparency::f16_bits(b)) << 16)
        };
        for ((d, blas), entry) in clear_draws.iter().zip(&p.entry[draws.len()..]) {
            let e = entry.unwrap();
            let (rgb, coverage, ior) = d.material.map_or(([1.0; 3], 1.0, 1.0), |m| {
                (
                    [m.base_color[0], m.base_color[1], m.base_color[2]],
                    m.see_through_coverage().unwrap_or(1.0),
                    m.transparency.map_or(1.0, |t| t.ior),
                )
            });
            let words = [
                0,
                0,
                0,
                2,
                pack_f16(rgb[0], rgb[1]),
                pack_f16(rgb[2], coverage),
                ior.to_bits(),
                0,
            ];
            for model in &d.instances.state.models {
                records.push((*model, &**blas, p.offsets[e], 0));
                extra.push(words);
            }
        }
        self.gi.stats.near_instances = records.iter().filter(|r| r.3 != 0).count() as u32;
        let mut head = gi_bvh::instance_words(&records)?;
        // Opaque instances live in `0..head[0]` (and feed the top-level BVH); static see-through
        // placements sit after them in `head[0]..head[4]`, checked only when present.
        head[0] = opaque_count as u32;
        head[4] = records.len() as u32;
        for (i, words) in extra.iter().enumerate() {
            let r = gi_bvh::HEADER_WORDS + i * gi_bvh::INSTANCE_WORDS + 24;
            head[r..r + 8].copy_from_slice(words);
        }
        // U-RTVIS: the top level over the records (rebuilt only when they change).
        crate::instance_bvh::place(
            &mut head,
            gi_bvh::HEADER_WORDS,
            gi_bvh::INSTANCE_WORDS,
            records.len(),
            &p.head,
        );
        self.gi.stats.instances = records.len() as u32;
        if p.head != head {
            let raw: Vec<u8> = head.iter().flat_map(|w| w.to_le_bytes()).collect();
            self.queue.write_buffer(&p.buffer, 0, &raw);
            p.head = head;
        }
        Ok(true)
    }

    /// Decides and uploads this frame's cache work. Runs after `prepare_primary`.
    /// `scene_generation` names this frame's TLAS, counting a build prepared but not yet
    /// encoded (as `prepare_primary` does), so hardware rays never trace last frame's scene.
    pub(super) fn prepare_gi(
        &mut self,
        draws: &[PreparedDraw<'_>],
        transparent: &[Draw<'_>],
        camera: &Camera,
        size: [u32; 2],
        eligible: bool,
        scene_generation: u64,
    ) -> Result<(), Error> {
        self.gi.plan = None;
        if self.gi.resources.is_none() {
            // On by default (G2b): the first eligible frame whose programs call an indirect
            // input allocates the table.
            if !(self.gi.settings.enabled && eligible && self.gi_callers(draws)) {
                return Ok(());
            }
            let cap = self.gi.settings.slots_log2;
            let log2 = match forced_slots_log2() {
                Some(n) => n,
                None if self.gi.settings.auto_slots => {
                    let triangles: u64 = draws
                        .iter()
                        .map(|d| {
                            u64::from(d.indices.len() as u32 / 3) * u64::from(d.slots.len() as u32)
                        })
                        .sum();
                    scene_slots_log2(triangles, cap)
                }
                None => cap,
            };
            self.allocate_gi(self.gi_slots(log2))?;
            self.gi.watch = (forced_slots_log2().is_none() && self.gi.settings.auto_slots)
                .then(|| FullWatch::new(&self.device));
            self.gi.grow = false;
        }
        // auto_slots: a full bucket since the last look doubles the table (emptied, so the
        // cache refills as after a cut), up to slots_log2 and the device's binding limit.
        if let Some(path) = probe_log_path() {
            let slots = self.gi.resources.as_ref().map_or(0, |r| r.slots);
            self.gi.probe_log.get_or_insert_with(|| ProbeWatch::new(&self.device)).collect(path, slots);
        }
        // Final Gather reads coarse cone cells via gi_external_requests instead of direct per-pixel
        // cache lookups; mark external_readers before frame 0's encode so gi_probe never floods
        // the cache with unused fine pixel cells.
        if crate::program::final_gather() && super::primary_ray_gpu::gi_receiver_pass::enabled(&self.device) {
            self.gi.request_external();
            self.gi.request_emitters();
        }
        if self.gi.external_readers {
            let device = &self.device;
            let watch = self.gi.external_watch.get_or_insert_with(|| FullWatch::of_words(device, &[0]));
            if let Some(pending) = watch.collect() {
                self.gi.planner.external_pending = pending;
            }
        }
        if let Some(full) = self.gi.watch.as_mut().and_then(FullWatch::collect) {
            self.gi.stats.peak_full = self.gi.stats.peak_full.max(full);
            self.gi.grow |= full > 0;
        }
        self.grow_emitters()?;
        if self.gi.grow {
            self.gi.grow = false;
            let slots = self.gi.resources.as_ref().map_or(0, |r| r.slots);
            let next = self.gi_slots((slots.trailing_zeros() + 1).min(self.gi.settings.slots_log2));
            if next > slots {
                self.allocate_gi(next)?;
                self.gi.stats.slot_doublings += 1;
                self.gi.stats.peak_full = 0;
                self.gi.watch = Some(FullWatch::new(&self.device));
            }
        }
        // LC2: changing light count may outgrow the bitset. Existing in-flight tables
        // retire normally; scheduling and cache lookup are otherwise unchanged.
        if self.gi.settings.enabled
            && self.gi.resources.as_ref().is_some_and(|r| {
                r.cold_words
                    < light_cold_words(self.lighting.pending.count, self.gi.settings.directions)
            })
        {
            let slots = self.gi.resources.as_ref().map_or(0, |r| r.slots);
            self.allocate_gi(slots)?;
        }
        let hardware = self.hardware_shadows.is_some() && !self.gi.settings.force_software;
        let run = self.gi.settings.enabled && eligible && self.gi_callers(draws);
        let software = !hardware;
        if stale_budget(software) != 0 {
            let device = &self.device;
            let watch = self.gi.stale_watch.get_or_insert_with(|| FullWatch::of_words(device, &[247]));
            if let Some(stale) = watch.collect() {
                self.gi.planner.stale_cells = stale;
            }
        }
        if fixed_rays(software) != 0 && fixed_rest_on() {
            let device = &self.device;
            let watch = self
                .gi
                .fixed_watch
                .get_or_insert_with(|| FullWatch::of_words(device, &[355, 356, 7]));
            if let Some(vals) = watch.collect_words() {
                let (moved, ranked, created) = (vals[0], vals[1], vals[2]);
                self.gi.planner.fixed_ranked = ranked;
                // Tolerate the handful of edge cells TR1 sub-pixel jitter creates on static views.
                if moved == 0 && created <= 6 {
                    self.gi.planner.fixed_quiet = self.gi.planner.fixed_quiet.saturating_add(1);
                } else {
                    self.gi.planner.fixed_quiet = 0;
                }
            }
        }
        let ids = self.gi_draw_ids(draws);
        let geometry_start = std::time::Instant::now();
        let run = run && (hardware || self.prepare_gi_geometry(draws, transparent, &ids)?);
        if super::change_log::enabled() {
            super::change_log::report(
                self.frames + 1,
                "gi_geometry",
                &[],
                &[("prepare_gi_geometry_ms", format!("{:.3}", geometry_start.elapsed().as_secs_f64() * 1000.))],
            );
        }
        // Bounce (G2): the setting is on and this frame's program reflects light. A change
        // empties the table, so no cell keeps bounce light from another program's frames.
        let bounce = run && self.gi.settings.bounce && self.ensure_gi_shade(software)?;
        if bounce && software && priority::on() {
            if self.gi.refill_feedback.is_none() {
                if !self.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
                    return Err(Error::InvalidInput("GI soft gather target requires timestamp queries"));
                }
                self.gi.refill_feedback = Some(refill_feedback::State::new(&self.device,&self.queue,&self.ledger));
            }
            let target = match priority::mode() { priority::Mode::Milliseconds(ms) => ms, _ => priority::gather_target() };
            let feedback = self.gi.refill_feedback.as_mut().unwrap();
            feedback.collect(target);
            feedback.set_frame(self.frames + 1);
        } else {
            self.gi.refill_feedback = None;
        }
        let r = self.gi.resources.as_ref().unwrap();
        let config = PlanConfig {
            settings: self.gi.settings,
            slots: r.slots,
            misses: r.misses,
            cold_words: r.cold_words,
            raw_slots: r.raw_slots,
            emitters: r.emitters,
            software,
            fixed_budget: !(software && priority::on()),
            field: self.environment.selected.is_some(),
            profiling: self.gi.profile.is_some() || probe_log_path().is_some(),
            near_bins: self.gi.near_bins,
            eye: camera.view().inverse().w_axis.truncate(),
            fov_y: camera.fov_y(),
            size,
        };
        // The cache's per-tick work runs in the tick's first frame whose plan is not idle, so
        // a moving view beside a still one still relights and gathers.
        let tick = !self.explicit_ticks || self.gi_tick != self.tick;
        let mut begun = self.gi.planner.begin(&config, run, bounce, tick);
        self.gi.priority_compact = false;
        self.gi.priority_hold = false;
        if software && priority::on() {
            let fallback = priority::Controller::default();
            let controller = self.gi.refill_feedback.as_ref().map_or(&fallback,|f|&f.controller);
            controller.header(&mut begun.header, priority::mode());
            priority::measurement_header(&mut begun.header);
            self.gi.priority_compact = priority::compact_available(begun.header[32], r.slots);
            if !self.gi.priority_compact {
                return Err(Error::InvalidInput("GI ordered first-fill lists exceed the existing ray arena"));
            }
            self.gi.priority_hold = matches!(priority::mode(), priority::Mode::Hold(n) if self.gi.planner.since_cut < n);
        }
        let raw: Vec<u8> = begun.header.iter().flat_map(|w| w.to_le_bytes()).collect();
        // Header words 58 and 59 are the emitter list's counts, written by the GPU
        // (gi_emitters_publish); they keep last frame's on frames the list is not rebuilt.
        self.queue.write_buffer(&r.table, 0, &raw[..58 * 4]);
        self.queue.write_buffer(&r.table, 60 * 4, &raw[60 * 4..]);
        if !run {
            return Ok(());
        }
        // Per draw (batch), its stable id, first placement and instance count, which cell
        // shading and hardware setup map cells' materials through (G2). MP1: in mixed
        // frames the fourth word selects the draw's cell program (`mixed_cell_entry`).
        let mixed = self.gi.mixed_active;
        let rows = draws.iter().enumerate().map(|(b, d)| {
            let n = d.instances.len() as u32;
            let program = d.material.map_or(self.standard.id(), |m| m.program.id());
            let selector = if mixed && self.program_slot(program).traits.reflects
            {
                cell_selector(program)
            } else {
                0
            };
            [ids[b], n, selector]
        });
        let table = draw_table(draws.len(), rows);
        let draw_table_changed = self.gi.draw_table != table;
        if draw_table_changed {
            let raw: Vec<u8> = table.iter().flat_map(|w| w.to_le_bytes()).collect();
            self.queue.write_buffer(&r.draws, 0, &raw);
            self.gi.draw_table = table;
        }
        self.ensure_gi_pipelines(software)?;
        self.prepare_gi_arrays(draws)?;
        // Moved casters: placements whose world bounds changed, as swept boxes. Also track the
        // world bounds of placements whose program declares `emission`, so moving an emissive
        // object relights bounce cells across the room and wakes final gather immediately.
        let mut bounds = vec![];
        let mut emitter_hasher = Sha256::new();
        let mut has_emitters = false;
        for d in draws {
            let program = d.material.map_or(self.standard.id(), |m| m.program.id());
            let emits = self.program_slot(program).traits.emits;
            for b in d.instances.state.world_bounds(d.mesh.bounds)?.iter() {
                let raw = [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z];
                bounds.push(raw);
                if emits {
                    has_emitters = true;
                    for c in raw {
                        emitter_hasher.update(c.to_le_bytes());
                    }
                }
            }
        }
        let emitter_bounds_hash: [u8; 32] = emitter_hasher.finalize().into();
        self.gi.has_emitters = has_emitters;
        if tick {
            let first_emitters = self.gi.emitter_bounds_hash.is_none() && has_emitters;
            let moved_emitter = self
                .gi
                .emitter_bounds_hash
                .is_some_and(|prev| prev != emitter_bounds_hash);
            self.gi.emitter_bounds_hash = Some(emitter_bounds_hash);
            self.gi.emitter_moved_frames = if moved_emitter {
                1
            } else {
                self.gi.emitter_moved_frames.saturating_sub(1)
            };
            self.gi.emitter_relight_frames = if moved_emitter || first_emitters {
                // Keep relight + soft-box active across 1st/2nd/3rd bounce convergence on both
                // RT (10 frames) and Non-RT (12 frames), including frame 0->1 when `post_emitters`
                // publishes initial scene emitters after frame 0 gathers already ran.
                if software { 12 } else { 10 }
            } else {
                self.gi.emitter_relight_frames.saturating_sub(1)
            };
        }
        // Temporarily boost fixed fill+gather ray budget during moving-emitter relight catchup
        // (6x RT / 8.5x Non-RT burst at full warp stride, 3x tail — sweeps visible seen_age<=8
        // cells cleanly via gi_fixed_bin's 3 visibility tiers without the 12x software cliff) and
        // cold-cache startup right after table clear (4x for 12 RT / 16 Non-RT frames). Preserves
        // higher overrides.
        let cold_clear = begun.since_clear < if software { 16 } else { 12 };
        let emitter_burst = self.gi.emitter_moved_frames > 0
            || self.gi.emitter_relight_frames >= if software { 8 } else { 6 };
        if (self.gi.emitter_relight_frames > 0 || cold_clear) && begun.header[45] != 0 {
            let base = begun.header[45];
            let boosted = if self.gi.emitter_relight_frames > 0 && emitter_burst {
                if software {
                    base.max(base.saturating_mul(17).saturating_div(2).min(835_584))
                } else {
                    base.max(base.saturating_mul(6).min(1_179_648))
                }
            } else if cold_clear {
                base.max(base.saturating_mul(4).min(if software { 393_216 } else { 786_432 }))
            } else {
                base.max(base.saturating_mul(3).min(if software { 294_912 } else { 589_824 }))
            };
            begun.header[45] = boosted;
            let r = self.gi.resources.as_ref().unwrap();
            self.queue.write_buffer(&r.table, 45 * 4, &boosted.to_le_bytes());
        }
        let poses = crate::renderer::skinning::placement_poses(draws);
        // The environment as the cache sees it: which map, its content and its rotation.
        let environment: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(self.environment.generation.to_le_bytes());
            h.update(self.environment.content_revision.to_le_bytes());
            let u = self.environment.uniform();
            h.update(&u[..8]);
            h.finalize().into()
        };
        // Bounce (G2): the lights, the environment, the frame's program and its draws'
        // materials (stable ids) form the lighting key.
        // Which part of the light key changed since the last frame (measurement only: five
        // more SHA-256 passes over the lights and program text, so only with RENDERLAB_GI_LOG).
        static PARTS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if bounce && *PARTS.get_or_init(|| std::env::var_os("RENDERLAB_GI_LOG").is_some()) {
            let state = self.view.primary.as_ref().unwrap();
            let part = |bytes: &[&[u8]]| -> [u8; 32] {
                let mut h = Sha256::new();
                for b in bytes {
                    h.update(b);
                }
                h.finalize().into()
            };
            let mut sources = self.program_slot(state.program).digest.to_vec();
            for &program in state.programs.iter().skip(1) {
                sources.extend(self.program_slot(program).digest);
            }
            let id_bytes: Vec<u8> = ids.iter().flat_map(|i| i.to_le_bytes()).collect();
            let parts = [
                part(&[format!("{:?}", self.lighting.source_lights).as_bytes()]),
                part(&[&environment]),
                part(&[&sources]),
                part(&[&id_bytes]),
                part(&[&self.lighting.decals.hash, &self.lighting.transmission.hash]),
            ];
            self.gi.relight_parts = match &self.gi.key_parts {
                Some(old) => (0..5).filter(|&i| old[i] != parts[i]).fold(0, |m, i| m | 1 << i),
                None => 0,
            };
            self.gi.key_parts = Some(parts);
        }
        let light_key: Option<[u8; 32]> = bounce.then(|| {
            let state = self.view.primary.as_ref().unwrap();
            let mut h = Sha256::new();
            h.update(format!("{:?}", self.lighting.source_lights).as_bytes());
            // DC1: altered surface inputs relight bounce cells on the existing schedule.
            h.update(self.lighting.decals.hash);
            // TP2: transparent sun attenuation changes the cells' direct illumination.
            h.update(self.lighting.transmission.hash);
            h.update(environment);
            h.update(self.program_slot(state.program).digest);
            // MP1: every program of a mixed frame.
            for &program in state.programs.iter().skip(1) {
                h.update(self.program_slot(program).digest);
            }
            h.update(ids.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<_>>());
            // In-place material edits keep every id: relight on them as on a new material.
            h.update(self.material_edits.bytes.to_le_bytes());
            h.finalize().into()
        });
        let region = self.view.primary.as_ref().unwrap().scene;
        // P-STILL (docs/p-still.md): a still view is the unjittered camera and the target
        // size, decided on the CPU from their bits. TR1's sub-pixel jitter (and the jittered
        // `scene_pixels` rectangle, which follows from the camera, the hashed placements
        // and the size) is sampling, not change; keyed on it, a still view never idled.
        let still = if super::primary_ray_gpu::temporal_reconstruction::p_still("gi-jittered") {
            (*camera, region.map(u32::to_le_bytes).concat())
        } else {
            (camera.unjittered(), size.map(u32::to_le_bytes).concat())
        };
        // SK1: a fixed camera and placement can still contain animated geometry.
        let mut skin_revisions = vec![];
        for d in draws {
            if let Some(s) = &d.mesh.skin {
                skin_revisions.extend(s.revision.to_le_bytes());
            }
        }
        if super::change_log::enabled() {
            let camera_bytes = matrix_bytes(still.0.matrix(size[0], size[1]));
            let lights = format!("{:?}", self.lighting.source_lights);
            let settings = format!("{:?}", self.gi.settings);
            super::change_log::report(
                self.frames + 1,
                "gi",
                &[
                    ("camera", &camera_bytes),
                    ("placements", &self.gi.arrays.as_ref().unwrap().revision_bytes),
                    ("environment", &environment),
                    ("poses", &skin_revisions),
                    ("lights", lights.as_bytes()),
                    ("decals", &self.lighting.decals.hash),
                    ("transmission", &self.lighting.transmission.hash),
                    ("settings", settings.as_bytes()),
                    ("size", &still.1),
                ],
                &[
                    ("bounce", bounce.to_string()),
                    ("tick", tick.to_string()),
                    ("idle_frames", self.view.gi.idle_frames.to_string()),
                    ("view_frames", self.view.gi.view_frames.to_string()),
                ],
            );
        }
        let finished = self.gi.planner.finish(
            &config,
            &begun,
            bounce,
            tick,
            &mut self.view.gi,
            FrameInputs {
                bounds,
                poses,
                environment,
                light_key,
                region,
                still_matrix: matrix_bytes(still.0.matrix(size[0], size[1])),
                still_extent: still.1,
                instances: &self.gi.arrays.as_ref().unwrap().revision_bytes,
                skin_revisions,
                emitter_relight: self.gi.emitter_relight_frames > 0,
                emitter_dirty: emitter_burst,
            },
        );
        let r = self.gi.resources.as_ref().unwrap();
        self.queue.write_buffer(&r.frame, 0, &finished.frame);
        let Some(mut plan) = finished.plan else {
            self.gi.stats.idle_frames += 1;
            return Ok(());
        };
        if draw_table_changed && plan.bounce {
            plan.post_emitters = true;
        }
        let environment_only = plan.environment_only;
        if plan.bounce {
            self.gi.stats.bounce_frames += 1;
        }
        self.gi.plan = Some(plan);
        self.gi.work_generation += 1;
        self.prepare_gi_groups(software, scene_generation)?;
        self.gi.stats.frames += 1;
        self.gi.stats.software = software;
        // The cache's clock (seen stamps, eviction, refill phases) counts ticks.
        if tick {
            self.gi.planner.advance(&begun, environment_only);
            self.gi_tick = self.tick;
        }
        Ok(())
    }

    fn prepare_gi_groups(&mut self, software: bool, scene_generation: u64) -> Result<(), Error> {
        let targets = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap();
        // A new software geometry buffer drops the groups itself; transform changes rewrite it.
        let rays = if software {
            self.gi.packed.as_ref().map_or(0, |p| p.bytes)
        } else {
            scene_generation
        };
        let key = (
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            rays ^ (self.gi.generation << 40),
            software,
        );
        if self.gi.groups.as_ref().is_some_and(|g| g.key == key) {
            return Ok(());
        }
        let layouts = self.gi.layouts.as_ref().unwrap();
        let r = self.gi.resources.as_ref().unwrap();
        let hit = targets.hit.texture.create_view(&Default::default());
        let tiles = targets.tiles.texture.create_view(&Default::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&hit),
            },
            wgpu::BindGroupEntry {
                binding: 21,
                resource: wgpu::BindingResource::TextureView(&tiles),
            },
            wgpu::BindGroupEntry {
                binding: 30,
                resource: r.table.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 31,
                resource: r.work.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 32,
                resource: r.frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 33,
                resource: r.outgoing.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 36,
                resource: r.draws.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 37,
                resource: r.raw.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 38,
                resource: r.external.as_entire_binding(),
            },
        ];
        if software {
            entries.push(wgpu::BindGroupEntry {
                binding: 35,
                resource: self.gi.packed.as_ref().unwrap().buffer.as_entire_binding(),
            });
        } else {
            entries.push(wgpu::BindGroupEntry {
                binding: 34,
                resource: wgpu::BindingResource::AccelerationStructure(
                    self.hardware_shadows.as_ref().unwrap().scene(),
                ),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 40,
                resource: self
                    .hardware_shadows
                    .as_ref()
                    .unwrap()
                    .see_through_table()
                    .unwrap_or(&*self.gi.dummy)
                    .as_entire_binding(),
            });
        }
        let main = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("indirect cache"),
                layout: &layouts.main[usize::from(software)],
                entries: &entries,
            }),
            ResourceKind::BindGroup,
        );
        let shade = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("indirect cache cell shading"),
                layout: &layouts.shade,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 31,
                        resource: r.work.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 33,
                        resource: r.outgoing.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 36,
                        resource: r.draws.as_entire_binding(),
                    },
                ],
            }),
            ResourceKind::BindGroup,
        );
        if let Some(old) = self.gi.groups.replace(Groups { key, main, shade }) {
            self.gi
                .retired
                .push((self.submissions, Retired::Groups(old)));
        }
        Ok(())
    }
}

// ---- MP1 per-program cell shading (docs/m7-mp1.md) ----
/// The draw table's cell-program word for a reflecting program `id` of a mixed frame; 0
/// marks draws whose program reflects nothing.
pub(crate) fn cell_selector(id: u64) -> u32 {
    u32::try_from(id + 1).expect("program ids fit the cell selector")
}
const CELL_PLACEMENT_SEAM: &str = "    if p==0xffffffffu { return vec4<f32>(0.0,0.0,0.0,1.0); }\n";
const CELL_STORE_SEAM: &str = "        gi_shade_out[slot]=gs_shade(slot);\n        if gi_borrow_on() { gi_shade_out[gi_borrow_at(slot)]=gs_borrow; }\n";
const CELL_EMISSION_SEAM: &str = "fn gs_owns_draw(batch:u32) -> i32 { return 1; }";
/// A mixed frame's cell entry for program `id`: it shades only cells whose draw selects
/// it, writes "reflects nothing" for draws whose program has no cell shading (every
/// program's dispatch writes the same value), and leaves other programs' cells to theirs.
pub(crate) fn mixed_cell_entry(source: &str, id: u64) -> String {
    assert_eq!(source.matches(CELL_PLACEMENT_SEAM).count(), 1);
    assert_eq!(source.matches(CELL_STORE_SEAM).count(), 1);
    assert_eq!(source.matches(CELL_EMISSION_SEAM).count(), 1);
    // The emission probe: its own draws, and zero for draws whose program reflects nothing.
    let source = source.replacen(
        CELL_EMISSION_SEAM,
        &format!(
            "fn gs_owns_draw(batch:u32) -> i32 {{ let w=gi_shade_draws[batch].w; if w==0u {{ return 0; }} return select(-1,1,w=={}u); }}",
            cell_selector(id)
        ),
        1,
    );
    let source = source.as_str();
    let check = format!(
        "{CELL_PLACEMENT_SEAM}    let gs_program=gi_shade_draws[primary_placements[p]&0x1fffffffu].w;\n    if gs_program==0u {{ return vec4<f32>(0.0,0.0,0.0,1.0); }}\n    if gs_program!={}u {{ return vec4<f32>(-1.0); }}\n",
        cell_selector(id)
    );
    source.replacen(CELL_PLACEMENT_SEAM, &check, 1).replacen(
        CELL_STORE_SEAM,
        "        let value=gs_shade(slot);\n        if value.w>=0.0 {gi_shade_out[slot]=value; if gi_borrow_on() {gi_shade_out[gi_borrow_at(slot)]=gs_borrow;}}\n",
        1,
    )
}
// ---- end of MP1 per-program cell shading ----

/// What cell shading (G2) binds besides its own group 0: traced shading's geometry (group 1)
/// and materials (group 2), and lighting's cell-shading group (`cell_bind`), whose binding 20
/// is the real table and whose binding 28 is never the outgoing radiance cell shading writes.
pub(super) struct ShadeGroups<'a> {
    pub arrays: &'a wgpu::BindGroup,
    pub materials: &'a wgpu::BindGroup,
    pub lighting: &'a wgpu::BindGroup,
}

impl GiState {
    /// The cache's work generation (`work_generation`), or None without a cache: lookups read at
    /// the same generation read the same table.
    pub(crate) fn table_generation(&self) -> Option<u64> {
        self.resources.is_some().then_some(self.work_generation)
    }
    /// The cache passes, after the primaries' hit records and before shading. `lighting`
    /// is lighting's kernel group (the header-only table at binding 20); `shade` the groups
    /// cell shading runs the frame's program on (G2). `mark` names the stages for the pass
    /// timeline (src/pass_timeline.rs).
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        lighting: &wgpu::BindGroup,
        shade: Option<ShadeGroups<'_>>,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        // Before this frame's pass consumes it, on idle frames too: what readers asked last frame.
        if let (Some(watch), Some(r)) = (self.external_watch.as_mut(), self.resources.as_ref()) {
            watch.record(encoder, &r.external);
        }
        let Some(plan) = self.plan.take() else {
            return;
        };
        mark(encoder, "gi_probe");
        let r = self.resources.as_ref().unwrap();
        let p = self.pipelines.as_ref().unwrap();
        let g = self.groups.as_ref().unwrap();
        let arrays = &self.arrays.as_ref().unwrap().bind;
        let lean_arrays = &self.arrays.as_ref().unwrap().lean;
        let has_clear = self.packed.as_ref().is_some_and(|pk| {
            pk.head.get(4).copied().unwrap_or(0) > pk.head.first().copied().unwrap_or(0)
        });
        let gather_pipe = if has_clear {
            p.gather_clear.as_ref().unwrap_or(&p.gather)
        } else {
            &p.gather
        };
        let rays_pipe = if has_clear {
            p.rays_clear.as_ref().unwrap_or(&p.rays)
        } else {
            &p.rays
        };
        let layout = WorkLayout::of(r);
        if plan.clear {
            encoder.clear_buffer(&r.table, HEADER_WORDS * 4, None);
            encoder.clear_buffer(&r.outgoing, 0, None);
            encoder.clear_buffer(&r.work, (COUNTER_WORDS - 2) * 4, Some(8));
            for at in layout.queues {
                encoder.clear_buffer(&r.work, u64::from(at) * 4, Some(4));
            }
        }
        // The last two counter words (GI_W_LAST_MOVED, GI_W_FILL_PENDING) carry over across frames.
        encoder.clear_buffer(&r.work, 0, Some((COUNTER_WORDS - 2) * 4));
        // Next frame's gather queue starts empty; this frame's holds last frame's appends.
        if plan.bounce {
            encoder.clear_buffer(&r.work, plan.next_queue_at, Some(4));
        }
        // The staged ray bits, after the counters, claims, misses, both queues and the ray
        // items. The request table after them is left clean by the last resolve round
        // (and by finish after a miss-list overflow).
        let staged_at = u64::from(layout.items + RAY_ITEMS) * 4;
        encoder.clear_buffer(&r.work, staged_at, Some(u64::from(RAY_ITEMS / 32) * 4));
        let queries = self.profile.as_ref().map(|p| &*p.queries);
        let ring = self
            .profile
            .as_ref()
            .map_or(0, |p| (p.written % PROFILE_RING) as u32 * p.stamps);
        let stamps = |i: u32| {
            queries.map(|q| wgpu::ComputePassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: Some(ring + 2 * i),
                end_of_pass_write_index: Some(ring + 2 * i + 1),
            })
        };
        let main_pass = |encoder: &mut wgpu::CommandEncoder,
                         label: &str,
                         pipeline: &wgpu::ComputePipeline,
                         stamp: u32,
                         groups: Option<[u32; 2]>| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: if stamp == u32::MAX { None } else { stamps(stamp) },
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*g.main, &[]);
            let group: &wgpu::BindGroup = if p.lean.contains(pipeline) { &**lean_arrays } else { &**arrays };
            pass.set_bind_group(1, group, &[]);
            pass.set_bind_group(3, lighting, &[]);
            if let Some([x, y]) = groups {
                pass.dispatch_workgroups(x, y, 1);
            }
        };
        // Direct fixed grids (CLAIM_GROUPS and the rest): kernels read their counts. Passes
        // with no work this frame (scan and integrate without an event) are not encoded;
        // when profiling they are empty passes, so every stamp is written.
        let profiling = queries.is_some();
        // Environment-only frames run only scan and integrate; when profiling, the other
        // passes are empty, so every stamp is written. When Final Gather owns primary GI
        // (`self.external_readers`), skip gi_probe so the cache only allocates and gathers the
        // coarse cone cells probes actually sample (no 280k-cell fine pixel cache starvation).
        let traced = !plan.environment_only;
        let probe = traced && !self.external_readers;
        if probe || profiling {
            main_pass(
                encoder,
                "indirect cache probe",
                &p.probe,
                0,
                probe.then_some(plan.groups),
            );
        }
        if let Some(probe)=self.queue_probe.as_mut() {
            probe.encode(encoder,r,self.planner.tick_queues[0]);
        }
        // Last frame's external requests join the probe's, for the same claim rounds; the list
        // then starts empty for this frame's appends (the final gather runs after encode).
        if traced && self.external_readers {
            main_pass(encoder, "indirect cache external requests", &p.external, u32::MAX, Some([EXTERNAL_GROUPS, 1]));
            encoder.clear_buffer(&r.external, 0, Some(16));
        }
        // Bounce gathers (G2) read last frame's outgoing radiance and may request hit cells
        // for the claim rounds that follow.
        let b = 6 + 2 * CLAIM_ROUNDS;
        mark(encoder, "gi_gather");
        // Build the emitter list before cache gathers using this frame's updated transforms so
        // both cache gathers (`gi_emitter_gather`) and final gather (`fg_direct_trace`)
        // see moving emitters on the exact frame they move.
        let need_emitters = self.emitter_readers || emitters_limit() > 0;
        if plan.bounce && need_emitters && !plan.clear {
            main_pass(encoder, "indirect emitters", &p.emitters_build, u32::MAX, Some([EMITTER_GROUPS, 1]));
            main_pass(encoder, "indirect emitters publish", &p.emitters_publish, u32::MAX, Some([1, 1]));
            main_pass(encoder, "indirect emitters power", &p.emitters_cdf, u32::MAX, Some([1, 1]));
        }
        if let Some(feedback) = &mut self.refill_feedback { feedback.begin(encoder); }
        if p.priority.is_some() {
            encoder.clear_buffer(&r.work,u64::from(layout.items)*4,Some(16*4));
        }
        if plan.bounce || profiling {
            let grid = plan.bounce.then_some([GATHER_GROUPS, 1]);
            let fixed = plan.fixed_rays != 0;
            if fixed {
                // Compacted fixed update budget: rank active cells into layout.items, cut, list gathers.
                let rank_groups = plan.bounce.then_some([r.slots.div_ceil(64).min(4096), 1]);
                let list_groups = plan.bounce.then_some([1024, 1]);
                mark(encoder, "gi_fixed");
                main_pass(encoder, "indirect fixed rank", &p.fixed_rank, u32::MAX, rank_groups);
                main_pass(encoder, "indirect fixed select", &p.fixed_select, u32::MAX, grid.map(|_| [1, 1]));
                main_pass(encoder, "indirect fixed list", &p.fixed_list, u32::MAX, list_groups);
                mark(encoder, "gi_gather");
            } else {
                // The gather ray budget's admission (no-ops without it).
                main_pass(encoder, "indirect gather count", &p.gather_count, u32::MAX, grid);
                main_pass(encoder, "indirect gather select", &p.gather_select, u32::MAX, grid.map(|_| [1, 1]));
            }
            if let Some(priority) = &p.priority {
                if self.priority_hold {
                    main_pass(encoder,"indirect gather hold",&priority.hold,b,grid);
                } else if self.priority_compact {
                    // Reuse claim scratch and the ray arena, before setup writes ray items.
                    encoder.clear_buffer(&r.work,COUNTER_WORDS*4,Some(u64::from(r.slots)*4));
                    main_pass(encoder,"indirect gather candidates",&priority.compact,u32::MAX,grid);
                    main_pass(encoder,"indirect gather selection",&priority.select,u32::MAX,plan.bounce.then_some([1,1]));
                    main_pass(encoder,"indirect gather deferral",&priority.defer,u32::MAX,grid);
                    main_pass(encoder,"indirect first-fill gather",&priority.gather_first,b,grid);
                    if let Some(feedback) = &self.refill_feedback { feedback.after_first(encoder,&r.work); }
                    main_pass(encoder,"indirect refresh gather",&priority.gather_refresh,PROFILE_STAMPS/2,grid);
                } else {
                    main_pass(encoder,"indirect cache gather",gather_pipe,b,grid);
                }
                let legacy_grid = if self.priority_hold || self.priority_compact { None } else {grid};
                main_pass(encoder,"indirect gather ticket clamp",&priority.gather_clamp,u32::MAX,legacy_grid.map(|_|[1,1]));
                if !self.priority_compact {
                    main_pass(encoder,"indirect visible gather",&priority.gather_later,PROFILE_STAMPS/2,legacy_grid);
                }
                main_pass(encoder,"indirect gather ticket clamp",&priority.gather_clamp,u32::MAX,legacy_grid.map(|_|[1,1]));
                main_pass(encoder,"indirect background gather",&priority.gather_background,PROFILE_STAMPS/2+1,legacy_grid);
            } else {
                main_pass(encoder,"indirect cache gather",gather_pipe,b,grid);
            }
            if fixed {
                main_pass(encoder, "indirect fixed after", &p.fixed_after, u32::MAX, plan.bounce.then_some([256, 1]));
            }
        }
        if let Some(feedback) = &self.refill_feedback {
            if !(plan.bounce || profiling) || !self.priority_compact || self.priority_hold {
                feedback.after_first(encoder,&r.work);
            }
            feedback.after_gathers(encoder,&r.work,layout.items);
        }
        mark(encoder, "gi_claim");
        if p.priority.is_some() { encoder.clear_buffer(&r.work,47*4,Some(4)); }
        if traced || profiling {
            // First fill's admission order, before the rounds (no-ops without its cap).
            main_pass(encoder, "indirect fill count", &p.fill_count, u32::MAX, traced.then_some([CLAIM_GROUPS, 1]));
            main_pass(encoder, "indirect fill select", &p.fill_select, u32::MAX, traced.then_some([1, 1]));
            for k in 0..CLAIM_ROUNDS {
                let grid = traced.then_some([CLAIM_GROUPS, 1]);
                if let Some(priority) = &p.priority {
                    main_pass(encoder,"indirect claim ticket reset",&priority.claim_reset,u32::MAX,traced.then_some([1,1]));
                }
                main_pass(encoder, "indirect cache claim", &p.claim, 1 + 2 * k, grid);
                let last = k + 1 == CLAIM_ROUNDS && p.priority.is_none();
                let resolve = if last { &p.resolve_last } else { &p.resolve };
                main_pass(encoder, "indirect cache resolve", resolve, 2 + 2 * k, grid);
            }
            if let Some(priority) = &p.priority {
                for k in 0..CLAIM_ROUNDS {
                    let grid = traced.then_some([CLAIM_GROUPS,1]);
                    let stamp = PROFILE_STAMPS/2+2+2*k;
                    main_pass(encoder,"indirect claim ticket reset",&priority.claim_reset,u32::MAX,traced.then_some([1,1]));
                    main_pass(encoder,"indirect background claim",&priority.claim_background,stamp,grid);
                    let resolve = if k+1==CLAIM_ROUNDS {&p.resolve_last} else {&p.resolve};
                    main_pass(encoder,"indirect background resolve",resolve,stamp+1,grid);
                }
            }
        }
        let s = 1 + 2 * CLAIM_ROUNDS;
        if let Some(feedback) = &self.refill_feedback {
            feedback.after_claims(encoder,&r.work,layout.items);
        }
        mark(encoder, "gi_update");
        if plan.scan || profiling {
            let groups = plan.scan.then_some([plan.scan_groups, 1]);
            main_pass(encoder, "indirect cache scan", &p.scan, s, groups);
        }
        let cells = Some([CELL_GROUPS, 1]);
        if traced || profiling {
            let setup = traced.then_some([SETUP_GROUPS, 1]);
            main_pass(encoder, "indirect cache setup", &p.setup, s + 1, setup);
            let rays = traced.then_some([RAY_GROUPS, 1]);
            main_pass(encoder, "indirect cache rays", rays_pipe, s + 2, rays);
            let finish = cells.filter(|_| traced);
            main_pass(encoder, "indirect cache finish", &p.finish, s + 3, finish);
        }
        if let Some(feedback) = &mut self.refill_feedback { feedback.finish(encoder,&r.work,&r.table); }
        mark(encoder, "gi_integrate");
        if plan.integrate || profiling {
            let groups = cells.filter(|_| plan.integrate);
            main_pass(
                encoder,
                "indirect cache integrate",
                &p.integrate,
                s + 4,
                groups,
            );
        }
        // The cell-space filter (G3), once every value of the frame is written and before cell
        // shading reads them.
        if plan.filter || profiling {
            let grid = plan.filter.then_some([FILTER_GROUPS, 1]);
            main_pass(
                encoder,
                "indirect cache filter",
                &p.filter,
                9 + 2 * CLAIM_ROUNDS,
                grid,
            );
        }
        // Light visibility at the cells to shade, then the program's cell shading (G2).
        mark(encoder, "gi_cell_shading");
        if plan.bounce || profiling {
            let grid = plan.bounce.then_some([LIGHT_GROUPS, 1]);
            main_pass(encoder, "indirect cache light", &p.light, b + 1, grid);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("indirect cache cell shading"),
                timestamp_writes: stamps(b + 2),
            });
            // MP1: a mixed frame runs each reflecting program over the queue in turn.
            let pipelines: Vec<_> = self.frame_shading.iter().map(|&i| &self.shading[i]).collect();
            if let (true, Some(shade)) = (plan.bounce, shade) {
                pass.set_bind_group(0, &*g.shade, &[]);
                pass.set_bind_group(1, shade.arrays, &[]);
                pass.set_bind_group(2, shade.materials, &[]);
                pass.set_bind_group(3, shade.lighting, &[]);
                for pipeline in pipelines {
                    pass.set_pipeline(&pipeline.pipeline);
                    pass.dispatch_workgroups(LIGHT_GROUPS, 1, 1);
                }
            }
        }
        // On cold frame 1, clears, draw changes, or lighting/material edits, rebuild emitters
        // right after cell shading so newly probed or edited emissions reach final gather on the
        // very same frame with zero startup/edit lag. Clear the emitter build work counters first
        // so a second build in the same frame never double-counts GI_W_EMITTER_DRAWS.
        if plan.bounce && need_emitters && plan.post_emitters {
            encoder.clear_buffer(&r.work, 200 * 4, Some(10 * 4));
            main_pass(encoder, "indirect emitters", &p.emitters_build, u32::MAX, Some([EMITTER_GROUPS, 1]));
            main_pass(encoder, "indirect emitters publish", &p.emitters_publish, u32::MAX, Some([1, 1]));
            main_pass(encoder, "indirect emitters power", &p.emitters_cdf, u32::MAX, Some([1, 1]));
        }
        if let Some(profile) = self.profile.as_ref() {
            if profile.priority && p.priority.is_none() {
                for i in PROFILE_STAMPS/2..profile.stamps/2 {
                    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("indirect priority profile (inactive)"), timestamp_writes: stamps(i),
                    });
                }
            }
        }
        if let Some(profile) = self.profile.as_mut() {
            let slot = profile.written % PROFILE_RING;
            encoder.resolve_query_set(
                &profile.queries,
                ring..ring + profile.stamps,
                &profile.resolve,
                slot * profile.stride,
            );
            profile.written += 1;
        }
        if let (Some(watch), Some(r)) = (self.watch.as_mut(), self.resources.as_ref()) {
            watch.record(encoder, &r.work);
        }
        if let (Some(watch), Some(r)) = (self.probe_log.as_mut(), self.resources.as_ref()) {
            watch.record(encoder, &r.work);
        }
        if let (Some(watch), Some(r)) = (self.stale_watch.as_mut(), self.resources.as_ref()) {
            watch.record(encoder, &r.work);
        }
        if let (Some(watch), Some(r)) = (self.fixed_watch.as_mut(), self.resources.as_ref()) {
            watch.record(encoder, &r.work);
        }
        if let (Some(watch), Some(r)) = (self.emitter_watch.as_mut(), self.resources.as_ref()) {
            watch.record(encoder, &r.work);
        }
    }
}

/// The cell shading pipeline over `module` and `layout` (`gi_shade_parts`): the driver's compile.
fn shade_cells_pipeline(
    device: &PipelineDevice,
    ledger: &Ledger,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> Owned<wgpu::ComputePipeline> {
    ledger.own(
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gi_shade_cells"),
            layout: Some(layout),
            module,
            entry_point: Some("gi_shade_cells"),
            compilation_options: Default::default(),
            cache: None,
        }),
        ResourceKind::ComputePipeline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{Stage, compose, uses_indirect};
    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");

    /// `cut_refill`: a camera cut opens a refill window (the claim cap) for CUT_REFILL_FRAMES
    /// frames, with no coarse read and no direction spread; off, a cut changes no header word.
    #[test]
    fn the_relight_rotation_holds_a_slot_group_to_the_budget() {
        // 2^20 slots at the default 8192: 128 groups of 8192.
        assert_eq!(relight_period(1 << 20, 0), 128);
        // Small tables keep the old period of 8; huge ones stop at 128 (one byte in the shader).
        assert_eq!(relight_period(1 << 14, 0), 8);
        assert_eq!(relight_period(1 << 21, 0), 128);
        // An explicit budget rounds the period up to a power of two.
        assert_eq!(relight_period(1 << 20, 65536), 16);
        assert_eq!(relight_period(1 << 20, 50000), 32);
        // u32::MAX keeps the rate before the budget.
        assert_eq!(relight_period(1 << 20, u32::MAX), 8);
    }

    #[test]
    fn a_camera_cut_opens_the_refill_window_only_with_cut_refill() {
        let config = |cut_refill: bool| PlanConfig {
            settings: IndirectSettings { cut_refill, ..IndirectSettings::default() },
            slots: 1 << 20,
            misses: 1 << 16,
            cold_words: 4,
            raw_slots: 1 << 20,
            emitters: EMITTERS_MAX,
            software: true,
            fixed_budget: false,
            field: false,
            profiling: false,
            near_bins: 0,
            eye: Vec3::ZERO,
            fov_y: 1.0,
            size: [1920, 1080],
        };
        for cut_refill in [false, true] {
            let c = config(cut_refill);
            let mut planner = Planner::default();
            let steady = planner.begin(&c, true, true, true);
            planner.advance(&steady, false);
            planner.camera_cut();
            for frame in 0..CUT_REFILL_FRAMES + 1 {
                let begun = planner.begin(&c, true, true, true);
                let in_window = cut_refill && frame < CUT_REFILL_FRAMES;
                let claims = if in_window { CLAIM_CELLS_SOFTWARE } else { 0 };
                assert_eq!(begun.header[22], claims, "cut_refill {cut_refill}, frame {frame}");
                // No coarse read either way: the footprint is the steady frame's.
                assert_eq!(begun.header[4], steady.header[4], "frame {frame}");
                // No phase spread with the refill; off, the cut is no reset either.
                assert_eq!(begun.header[14], 0, "phases, frame {frame}");
                planner.advance(&begun, false);
            }
        }
    }

    #[test]
    fn fixed_budget_headers_and_settle_idle() {
        let mut settings = IndirectSettings::default();
        settings.enabled = true;
        settings.bounce = true;
        let config = PlanConfig {
            settings,
            slots: 1 << 18,
            misses: 1 << 16,
            cold_words: 24,
            raw_slots: 1 << 18,
            emitters: EMITTERS_MAX,
            software: true,
            fixed_budget: true,
            field: false,
            profiling: false,
            near_bins: 0,
            eye: Vec3::ZERO,
            fov_y: 1.0,
            size: [1920, 1080],
        };
        let mut planner = Planner::default();
        let mut view = ViewPlan::default();
        let inputs = |bounds: Vec<[f32; 6]>| FrameInputs {
            bounds,
            poses: vec![],
            environment: [0; 32],
            light_key: Some([1; 32]),
            region: [0, 0, 1920, 1080],
            still_matrix: [0; 64],
            still_extent: vec![],
            instances: &[],
            skin_revisions: vec![],
            emitter_relight: false,
            emitter_dirty: false,
        };
        let begun = planner.begin(&config, true, true, true);
        assert_eq!(begun.header[45], fixed_rays(true));
        assert_eq!(begun.header[44], 1);
        assert_eq!(begun.header[51], 1);
        // Active work before settling: not idle.
        let finished = planner.finish(&config, &begun, true, true, &mut view, inputs(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]));
        let plan = finished.plan.expect("first frame must run");
        assert_eq!(plan.fixed_rays, fixed_rays(true));
        planner.advance(&begun, false);

        // Once the view is still for 8+ frames and GPU watch reports quiet sweeps with no
        // remaining ranked work, the planner idles the cache (0 ms) and wakes immediately on motion.
        for _ in 0..8 {
            let b = planner.begin(&config, true, true, true);
            let f = planner.finish(&config, &b, true, true, &mut view, inputs(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]));
            planner.advance(&b, f.plan.is_none());
        }
        planner.fixed_quiet = 7;
        planner.fixed_ranked = 0;
        let begun_idle = planner.begin(&config, true, true, true);
        let warming = planner.finish(&config, &begun_idle, true, true, &mut view, inputs(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]));
        assert!(warming.plan.is_some(), "should still run before turn quiet frames");
        planner.fixed_quiet = 8;
        let idle = planner.finish(&config, &begun_idle, true, true, &mut view, inputs(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]));
        assert!(idle.plan.is_none(), "settled static view should idle");
        let moved = planner.finish(&config, &begun_idle, true, true, &mut view, inputs(vec![[2.0, 0.0, 0.0, 3.0, 1.0, 1.0]]));
        assert!(moved.plan.is_some(), "mover must wake the cache immediately");
    }

    /// More moved boxes than the frame holds become at most that many unions, each box inside
    /// one, and far-apart movers stay apart.
    #[test]
    fn moved_boxes_cluster_by_position() {
        let boxes: Vec<[f32; 6]> = (0..100)
            .map(|i| {
                let x = (i % 10) as f32 * 50.0;
                let z = (i / 10) as f32 * 50.0;
                [x, 0.0, z, x + 1.0, 1.0, z + 1.0]
            })
            .collect();
        let clusters = cluster_boxes(boxes.clone(), 32);
        assert!(clusters.len() <= 32);
        for b in &boxes {
            assert!(clusters.iter().any(|c| (0..3).all(|k| c[k] <= b[k] && b[k + 3] <= c[k + 3])));
        }
        // No cluster spans the whole 450 m field (the single union did).
        let largest = clusters
            .iter()
            .map(|c| (c[3] - c[0]).max(c[5] - c[2]))
            .fold(0.0f32, f32::max);
        assert!(largest < 200.0, "largest cluster {largest} m");
    }

    fn validate(label: &str, source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
    }

    #[test]
    fn kernels_validate_on_both_ray_sources() {
        for directions in [64, 128] {
            validate("hardware kernels", &kernel_source(false, directions));
            validate("software kernels", &kernel_source(true, directions));
        }
    }

    #[test]
    fn cold_layout_keeps_every_direction_and_light() {
        for directions in [64, 128] {
            let words = directions / 32;
            let lights_at = light_cold_offset(directions);
            assert_eq!(lights_at, if directions == 64 { 23 } else { 31 });
            for lights in [0u32, 1, 32, 64, 128, 300, 65_535] {
                let stride = light_cold_words(lights, directions);
                assert_eq!(stride % 4, 0);
                let mut used = vec![false; stride as usize];
                let mut occupy = |range: std::ops::Range<u32>| {
                    for i in range {
                        assert!(!std::mem::replace(&mut used[i as usize], true));
                    }
                };
                occupy(0..12); // definition, point and state
                for bitset in 0..4 {
                    occupy(12 + bitset * words..12 + (bitset + 1) * words);
                }
                occupy(lights_at - 3..lights_at); // material, primitive, barycentrics
                occupy(lights_at..lights_at + lights.div_ceil(32).max(1));
            }
        }
    }

    #[test]
    fn only_callers_compose_the_indirect_inputs() {
        let standard = compose(PRELUDE, crate::STANDARD_PROGRAM, Stage::Raster, false);
        assert!(standard.source.contains("var<storage,read> gi_table"));
        crate::program::analyse("standard", crate::STANDARD_PROGRAM, &standard).unwrap();
        for source in
            super::primary_ray_gpu::shading_sources(PRELUDE, crate::STANDARD_PROGRAM).unwrap()
        {
            validate("standard traced", &source);
        }
        let toon = include_str!("../shaders/programs/toon.wgsl");
        assert!(!uses_indirect(toon));
        let composed = compose(PRELUDE, toon, Stage::Raster, true);
        assert!(!composed.source.contains("gi_table"));
        assert!(!uses_indirect(
            "// indirect_irradiance(input,n)\n/* indirect_irradiance */ fn f() {}"
        ));
        assert!(uses_indirect(
            "fn f(i:SurfaceInput)->vec3<f32> { return indirect_irradiance(i,vec3<f32>(0.0,1.0,0.0)); }"
        ));
    }

    /// G2: cell shading composes for programs that declare the bounce hooks, on both ray
    /// paths' shading heads, and validates; a hook with another signature is rejected when
    /// the program is created; programs without hooks reflect nothing.
    #[test]
    fn cell_shading_composes_for_reflecting_programs() {
        use crate::program::{analyse, reflects};
        assert!(reflects(crate::STANDARD_PROGRAM));
        assert!(!reflects(include_str!("../shaders/programs/toon.wgsl")));
        assert!(!reflects("// fn scatter(\nfn surface() {}"));
        let rt = super::primary_ray_gpu::shading_heads(PRELUDE).unwrap();
        let nonrt = super::primary_ray_gpu::non_rt_sources(PRELUDE).unwrap().1;
        for head in [&rt[1], &nonrt[1]] {
            let cell = compose(head, crate::STANDARD_PROGRAM, Stage::Cell, false);
            assert!(cell.source.contains("fn gi_shade_cells("));
            validate("standard cell shading", &cell.source);
            // MP1: a mixed frame's cell entry selects its own draws' cells.
            let mixed = mixed_cell_entry(&cell.source, 3);
            assert!(mixed.contains("if gs_program!=4u"));
            assert!(mixed.contains("if value.w>=0.0 {gi_shade_out[slot]=value; if gi_borrow_on() {gi_shade_out[gi_borrow_at(slot)]=gs_borrow;}}"));
            validate("mixed standard cell shading", &mixed);
            // Cell shading binds only its own group 0 (work, outgoing radiance, draw table):
            // nothing it reaches may read traced shading's group 0 (camera, hit records).
            let module = naga::front::wgsl::parse_str(&cell.source).unwrap();
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            let entry = module
                .entry_points
                .iter()
                .position(|e| e.name == "gi_shade_cells")
                .unwrap();
            let uses = info.get_entry_point(entry);
            for (handle, global) in module.global_variables.iter() {
                if let Some(b) = global.binding.as_ref().filter(|_| !uses[handle].is_empty()) {
                    assert!(
                        b.group != 0 || [31, 33, 36].contains(&b.binding),
                        "cell shading reaches group 0 binding {} ({:?})",
                        b.binding,
                        global.name
                    );
                }
            }
        }
        let emissive = "struct Parameters { glow:vec3<f32> };\nstruct Surface { n:vec3<f32>, albedo:vec3<f32> };\nfn surface(input:SurfaceInput)->Surface { return Surface(input.geometric,sample_base_color(input).rgb); }\nfn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return expose(indirect_irradiance(input,s.n)*s.albedo/PI+params.glow); }\nfn emission(s:Surface,input:SurfaceInput)->vec3<f32> { return params.glow; }\n";
        assert!(reflects(emissive));
        validate(
            "emissive cell shading",
            &compose(&rt[1], emissive, Stage::Cell, true).source,
        );
        analyse(
            "emissive",
            emissive,
            &compose(PRELUDE, emissive, Stage::Raster, true),
        )
        .unwrap();
        let wrong = emissive.replace(
            "fn emission(s:Surface,input:SurfaceInput)",
            "fn emission(s:Surface)",
        );
        let error = analyse(
            "wrong",
            &wrong,
            &compose(PRELUDE, &wrong, Stage::Raster, true),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("bounce light"), "{error}");
    }

    /// G2: the gather's fixed weights (2 pi / K over the cell's hemisphere directions) make
    /// its L1 SH exact for uniform bounce radiance: irradiance pi L at the cell normal, for
    /// any normal and rotation, with no normalization by a count (W1, C8). Cell shading's
    /// sixteen fixed directions give a Lambert program's albedo exactly (sum z_k = Q / 2).
    #[test]
    fn bounce_quadrature_is_exact_for_uniform_light_and_lambert() {
        for (n, rotation) in [
            ([0.0, 1.0, 0.0], 0.0),
            ([0.6, 0.0, -0.8], 1.3),
            ([0.0, 0.0, -1.0], 4.0),
        ] {
            let mut c = [0.0f64; 4];
            for d in directions(n, rotation) {
                let y = basis(d);
                for i in 0..4 {
                    c[i] += std::f64::consts::TAU / 64.0 * y[i];
                }
            }
            let y = basis(n);
            let e: f64 = (0..4).map(|i| band(i) * y[i] * c[i]).sum();
            assert!((e - std::f64::consts::PI).abs() < 1e-5, "{n:?}: {e}");
        }
        let albedo = 0.37;
        let response: f64 = (0..16)
            .map(|k| {
                let z = 1.0 - (k as f64 + 0.5) / 16.0;
                std::f64::consts::TAU / 16.0 * albedo / std::f64::consts::PI * z
            })
            .sum();
        assert!((response - albedo).abs() < 1e-12, "{response}");
    }

    /// CPU mirror of gi_direction, gi_sh_basis and the band weights (f64).
    fn directions(n: [f64; 3], rotation: f64) -> Vec<[f64; 3]> {
        let s = if n[2] >= 0.0 { 1.0 } else { -1.0 };
        let a = -1.0 / (s + n[2]);
        let b = n[0] * n[1] * a;
        let t = [1.0 + s * n[0] * n[0] * a, s * b, -s * n[0]];
        let u = [b, s + n[1] * n[1] * a, -n[1]];
        (0..64)
            .map(|k| {
                let z = 1.0 - (k as f64 + 0.5) / 64.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                let phi = k as f64 * 2.399963229728653 + rotation;
                let (x, y) = (r * phi.cos(), r * phi.sin());
                std::array::from_fn(|i| t[i] * x + u[i] * y + n[i] * z)
            })
            .collect()
    }
    fn basis(d: [f64; 3]) -> [f64; 9] {
        let [x, y, z] = d;
        [
            0.282095,
            0.488603 * y,
            0.488603 * z,
            0.488603 * x,
            1.092548 * x * y,
            1.092548 * y * z,
            0.315392 * (3.0 * z * z - 1.0),
            1.092548 * x * z,
            0.546274 * (x * x - y * y),
        ]
    }
    fn band(i: usize) -> f64 {
        [
            std::f64::consts::PI,
            std::f64::consts::TAU / 3.0,
            std::f64::consts::FRAC_PI_4,
        ][match i {
            0 => 0,
            1..=3 => 1,
            _ => 2,
        }]
    }
    /// Cached irradiance at `shading` from visibility `v` over the fixed directions.
    fn cached(
        normal: [f64; 3],
        rotation: f64,
        shading: [f64; 3],
        v: impl Fn([f64; 3]) -> bool,
    ) -> f64 {
        let mut sh = [0.0; 9];
        for d in directions(normal, rotation) {
            if v(d) {
                for (c, y) in sh.iter_mut().zip(basis(d)) {
                    *c += std::f64::consts::TAU / 64.0 * y;
                }
            }
        }
        let y = basis(shading);
        (0..9).map(|i| band(i) * y[i] * sh[i]).sum::<f64>().max(0.0)
    }
    /// Reference: midpoint quadrature over the upper hemisphere (z, azimuth).
    fn exact(shading: [f64; 3], v: impl Fn([f64; 3]) -> bool) -> f64 {
        let (nz, np) = (800, 1600);
        let mut e = 0.0;
        for i in 0..nz {
            let z = (i as f64 + 0.5) / nz as f64;
            let r = (1.0 - z * z).sqrt();
            for j in 0..np {
                let phi = (j as f64 + 0.5) / np as f64 * std::f64::consts::TAU;
                let d = [r * phi.cos(), r * phi.sin(), z];
                let c = d[0] * shading[0] + d[1] * shading[1] + d[2] * shading[2];
                if c > 0.0 && v(d) {
                    e += c;
                }
            }
        }
        e * std::f64::consts::TAU / (nz * np) as f64
    }

    /// The representation alone (64 fixed directions, L2 SH): exact up to quadrature
    /// on an open plane for any shading normal, so open surfaces are not over-occluded;
    /// bounded error beside walls of known horizon. Spatial interpolation and query
    /// points are measured on the GPU benches (docs/m7-g1.md).
    #[test]
    fn representation_error_on_open_planes_and_walls() {
        let up = [0.0, 0.0, 1.0];
        let mut open: f64 = 0.0;
        for degrees in [0.0f64, 30.0, 60.0, 80.0] {
            let t = degrees.to_radians();
            let n = [t.sin(), 0.0, t.cos()];
            for rotation in [0.0, 1.3, 4.0] {
                let (c, e) = (cached(up, rotation, n, |_| true), exact(n, |_| true));
                open = open.max((c - e).abs() / e);
            }
        }
        // Measured 0.0145 (shading normal 80 degrees from the cell normal).
        eprintln!("G1 representation: worst relative error on an open plane {open:.4}");
        assert!(open < 0.02, "open-plane representation error {open}");
        // A wall at x = -d of height h: blocked when heading -x under the top edge.
        let mut worst: f64 = 0.0;
        for ratio in [0.25f64, 1.0, 4.0, 1e6] {
            let wall = |d: [f64; 3]| !(d[0] < 0.0 && d[2] / -d[0] < ratio);
            for rotation in [0.0, 0.7, 2.1, 3.3] {
                let (c, e) = (cached(up, rotation, up, wall), exact(up, wall));
                worst = worst.max((c - e).abs() / e);
            }
        }
        // Measured 0.0639, all of it quadrature: 1,024 directions give 0.004 and the
        // L2 truncation alone under 0.01 (docs/m7-g1.md).
        eprintln!("G1 representation: worst relative error beside a wall {worst:.4}");
        assert!(worst < 0.07, "wall representation error {worst}");
    }

    /// The merged world (G1d) holds every placement's cut in world space, and a mirroring
    /// placement keeps its triangles' source-winding normals pointing as the instance path's
    /// inverse-transpose normals do, so inside tests read the same faces.
    #[test]
    fn world_merges_placements_and_keeps_winding() {
        use crate::asset::near_field::{NearField, NearPlacement, Settings, build_bvh};
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        // Source winding normal: (b - a) x (c - a) = +x cross +z = -y.
        let mesh = build_bvh(&positions, &[0, 1, 2]).unwrap();
        let shift = Mat4::from_translation(glam::Vec3::new(5.0, 0.0, 0.0));
        let mirror = Mat4::from_scale(glam::Vec3::new(-1.0, 1.0, 1.0));
        let field = NearField {
            settings: Settings {
                lod_error: 0.01,
                reach: 0.0,
                skip: 0.0,
                bins: 0,
            },
            source: [7; 32],
            meshes: vec![mesh],
            placements: [shift, mirror]
                .map(|m| NearPlacement {
                    mesh: 0,
                    transform: m.to_cols_array_2d(),
                    masks: vec![],
                })
                .to_vec(),
        };
        let world = World::new(&field).unwrap();
        let bvh = &world.blas.bvh;
        assert_eq!(bvh.positions.len(), 6);
        assert_eq!(bvh.indices.len(), 6);
        assert_eq!(world.placements.len(), 2);
        assert!(world.meshes.len() == 1 && world.meshes[0].is_none());
        for t in bvh.indices.chunks_exact(3) {
            let p = |i: u32| glam::Vec4::from(bvh.positions[i as usize]).truncate();
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            // Both placements keep the object's -y normal: the shift trivially, the mirror
            // because its winding is reversed (its inverse transpose keeps -y too).
            assert!(n.y < 0.0 && n.x.abs() < 1e-6, "world normal {n}");
            let shifted = p(t[0]).x >= 4.0;
            assert_eq!(shifted, (0..3).all(|k| p(t[k]).x >= 4.0));
        }
    }
}

#[cfg(test)]
#[path = "gi_cache_tests.rs"]
mod gpu_tests;

impl Renderer {
    /// Measurement only (`RENDERLAB_GI_OCCUPANCY=<file>`, at the memory report's frame): copies the
    /// table back once, waiting, and writes its slots, the occupied ones (hot key word nonzero) and
    /// those seen within the last 1, 60 and 600 frames (cold `GI_C_SEEN` against the header's frame).
    pub(super) fn measure_gi_occupancy(&mut self) {
        let Some(path) = std::env::var_os("RENDERLAB_GI_OCCUPANCY") else { return };
        let Some(r) = self.gi.resources.as_ref() else { return };
        let bytes = r.table.size();
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GI occupancy readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.table, 0, &staging, 0, bytes);
        self.queue.submit([encoder.finish()]);
        staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let Ok(data) = staging.slice(..).get_mapped_range() else { return };
        let word = |i: usize| u32::from_ne_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
        let (header, hot) = (64usize, 24usize);
        let slots = word(1) as usize + 1;
        let frame = word(3);
        let cold_words = word(23).max(24) as usize;
        let cold = header + slots * hot;
        let (mut occupied, mut seen) = (0u64, [0u64; 3]);
        for s in 0..slots {
            if word(header + s * hot) == 0 {
                continue;
            }
            occupied += 1;
            let age = frame.wrapping_sub(word(cold + s * cold_words + 2));
            for (k, limit) in [1u32, 60, 600].into_iter().enumerate() {
                seen[k] += u64::from(age <= limit);
            }
        }
        drop(data);
        let _ = std::fs::write(
            path,
            format!("slots {slots} frame {frame} occupied {occupied} seen1 {} seen60 {} seen600 {}\n", seen[0], seen[1], seen[2]),
        );
    }
}

/// The bench's reload (src/bench, feature `bench`, docs/bench.md): the lighting cache's kernels
/// and every program's cell shading, rebuilt from the current shader text (`program::gi_shader`).
/// Each part keeps its old pipelines when its new text fails, and the error returns.
#[cfg(feature = "bench")]
impl Renderer {
    /// The bench's stamp-wrap check (`walk-wrap`): the cache's frame stamp continues from `frame`
    /// instead of 0 after a reset, so a short fixture crosses a wrap of the stamp's packing.
    pub(super) fn bench_start_frame(&mut self, frame: u32) {
        self.gi.planner.frame = frame;
    }
    /// The prelude passes' reload (src/bench/reload.rs): every program's cell shading compiles
    /// again when prepare or a frame next needs it.
    pub(super) fn bench_drop_gi_shading(&mut self) {
        self.gi.shading.clear();
        self.gi.frame_shading.clear();
    }
    pub(super) fn bench_reload_gi(&mut self) -> Result<(), Error> {
        let slots = crate::pipeline_compilation::prepare_compile_slots();
        crate::pipeline_compilation::with_compile_slots(slots, || self.bench_rebuild_gi())
    }
    fn bench_rebuild_gi(&mut self) -> Result<(), Error> {
        if let Some(old) = self.gi.pipelines.take() {
            let software = old.software;
            if let Err(error) = self.ensure_gi_pipelines(software) {
                self.gi.pipelines = Some(old);
                return Err(error);
            }
        }
        let old = std::mem::take(&mut self.gi.shading);
        let mut fresh = Vec::with_capacity(old.len());
        for entry in &old {
            match self.gi_shade_pipeline(entry.key.0, entry.key, entry.mixed) {
                Ok(pipeline) => fresh.push(pipeline),
                Err(error) => {
                    self.gi.shading = old;
                    return Err(error);
                }
            }
        }
        self.gi.shading = fresh;
        Ok(())
    }
}
