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
pub(crate) const COUNTER_WORDS: u64 = 44;
const MISS_WORDS: u64 = 6;
/// Words per request-table entry: key a, generation band, the chosen key b (shaders/gi_cache.wgsl
/// GI_REQUEST_WORDS).
const REQUEST_WORDS: u64 = 3;
/// WGSL GiFrame: region, counts, offsets, requests, bounce, then 64 vec4 box corners.
const FRAME_BYTES: usize = 80 + 64 * 16;
const MAX_BOXES: usize = 32;
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
/// The default per-frame gather budget (`IndirectSettings::gather_cells` 0), in cells, on both
/// paths. G2b sized per-path budgets to about 4 ms of a cut frame's gathers (0.13 us a cell with hardware rays and
/// 0.4 us with software rays on the quarry; docs/m7-g2b.md). Moving frames gather fewer
/// cells than this, so only cuts spread. Software rays had a quarter of the hardware budget,
/// so non-RT bounce took about 100 frames to settle after a cut and lit flat faces showed
/// blotches for the first 8-16+ frames (Ara's Showcase). P-DEFECT gave them the hardware
/// budget (P-GI and P-GATHER made software gathers cheaper since G2b; docs/p-defect.md).
const GATHER_CELLS: u32 = 32768;
/// The per-path budgets of new cells (`IndirectSettings::claim_cells` 0, opt-in): about 3 ms of
/// query points and rays a cut frame at G2b's measured costs (2 px cells: about 60 ns a new
/// cell with hardware rays, 140 ns with software rays; docs/m7-g2b.md).
const CLAIM_CELLS_HARDWARE: u32 = 49152;
const CLAIM_CELLS_SOFTWARE: u32 = 20480;
/// Frames after a cut (a cleared table) whose pixels read cells one level coarser: a quarter of
/// the cells cover the view within the claim budget, and the finer cells that follow fall back
/// to them until they exist (G2b).
const COARSE_CUT_FRAMES: u32 = 4;
pub(crate) const LIGHT_GROUPS: u32 = 1024;
/// The cell-space filter's fixed grid (G3): nine items (a changed cell and its eight
/// neighbours) per cell shading queue entry.
pub(crate) const FILTER_GROUPS: u32 = 1024;
/// Unfiltered hot words 1-22 per slot, the filter's sources (shaders/gi_cache.wgsl gi_raw).
const RAW_WORDS: u64 = 22;
/// Slot groups a lighting, environment or caster change spreads relighting over while it
/// keeps changing (and for as many frames after), as ENVIRONMENT_PERIOD does for the sky.
const RELIGHT_PERIOD: u32 = 8;
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
    pub(crate) fn of(slots: u32, lights: u32, settings: &IndirectSettings) -> Self {
        // A frame lists at most 2^19 misses (a 2 px cut at 1080p makes about 240k cells):
        // the miss list and request table stay at G2's size with the larger tables.
        let misses = slots.min(1 << 19);
        let directions = settings.directions;
        let cold_words = light_cold_words(lights, directions);
        let table_bytes = (HEADER_WORDS + u64::from(slots) * (24 + u64::from(cold_words))) * 4;
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
        let outgoing_bytes = u64::from(slots) * 16;
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
    /// radiance and 4 of claim word).
    pub slots_log2: u32,
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
            gather_stride: 1,
            claim_cells: u32::MAX,
            // G3 (docs/m7-g3.md): sky and bounce filtered, contrast refine and bounce fill.
            filter: 15,
            filter_edge: 0.5,
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
            && (0.0..=1.0).contains(&self.filter_edge);
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
    pub gather_unshaded: u32,
    pub gather_orphans: u32,
    /// Queued cells a capped gather frame left for the next (`IndirectSettings::gather_cells`).
    pub gather_deferred: u32,
    /// Listed misses a capped claim left for a later frame (`IndirectSettings::claim_cells`).
    pub claim_deferred: u32,
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
    slots: u32,
    cold_words: u32, // LC2: dynamic per-cell light bitset stride.
    directions: u32,
    misses: u32,
    /// Slots the filter sources hold (1 without a filter).
    raw_slots: u32,
    _allocations: [Allocation; 5],
}
struct Layouts {
    /// Hardware (TLAS at 34) and software (geometry at 35).
    main: [Owned<wgpu::BindGroupLayout>; 2],
    arrays: Owned<wgpu::BindGroupLayout>,
    pipelines: [Owned<wgpu::PipelineLayout>; 2],
    /// Cell shading's group 0 (G2): the work buffer read-only, the outgoing radiance and
    /// the draw table.
    shade: Owned<wgpu::BindGroupLayout>,
}
struct Pipelines {
    software: bool,
    directions: u32,
    probe: Owned<wgpu::ComputePipeline>,
    claim: Owned<wgpu::ComputePipeline>,
    resolve: Owned<wgpu::ComputePipeline>,
    resolve_last: Owned<wgpu::ComputePipeline>,
    scan: Owned<wgpu::ComputePipeline>,
    setup: Owned<wgpu::ComputePipeline>,
    rays: Owned<wgpu::ComputePipeline>,
    finish: Owned<wgpu::ComputePipeline>,
    integrate: Owned<wgpu::ComputePipeline>,
    gather: Owned<wgpu::ComputePipeline>,
    light: Owned<wgpu::ComputePipeline>,
    filter: Owned<wgpu::ComputePipeline>,
}
/// One program's cell shading pipeline (G2), keyed by everything its module and layout
/// depend on: the program and its source, the prelude, the ray path and the material
/// group's layout (streamed or not).
struct ShadePipeline {
    key: (u64, [u8; 32], bool, bool),
    pipeline: Owned<wgpu::ComputePipeline>,
}
/// Group 1: the traced shading ABI's geometry tables, built from the same draws in
/// TLAS placement order, so hit records decode to the receivers shading sees.
struct Arrays {
    keys: Vec<(wgpu::Buffer, wgpu::Buffer, usize)>,
    models: Owned<wgpu::Buffer>,
    normals: Owned<wgpu::Buffer>,
    _placements: Owned<wgpu::Buffer>,
    _absent: Owned<wgpu::Buffer>,
    instances: Vec<u8>,
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
    /// Software rays (the non-RT path) rather than hardware ray queries.
    pub(crate) software: bool,
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
    /// The placement transforms' bytes as the geometry arrays hold them.
    pub(crate) instances: &'a [u8],
    /// Every skinned draw's revision, little-endian, in draw order.
    pub(crate) skin_revisions: Vec<u8>,
}

/// The second half: the frame record and, unless the cache is idle this frame, the
/// passes to run.
pub(crate) struct Finished {
    /// The `GiFrame` record's bytes.
    pub(crate) frame: Vec<u8>,
    /// None: the cache is idle (view, scene and lighting unchanged for long enough).
    pub(crate) plan: Option<Plan>,
}

/// The cache's CPU history between frames, and the per-frame plan made from it:
/// everything `prepare_gi` decides without a device. The wgpu renderer and the ara-gpu
/// backend both plan through `begin` and `finish`, then `advance` once the frame's
/// passes are certain to run.
pub(crate) struct Planner {
    bounds: Vec<[f32; 6]>,
    poses: Vec<crate::renderer::skinning::PlacementPose>,
    environment: Option<[u8; 32]>,
    /// The frame stamp cells are marked seen with.
    frame: u32,
    /// Empty the cache at the start of the next planned frame.
    pending_clear: bool,
    /// Frames since the cache was last emptied.
    since_clear: u32,
    idle_key: Option<[u8; 32]>,
    idle_frames: u32,
    view_key: Option<[u8; 32]>,
    view_frames: u32,
    environment_streak: bool,
    environment_catchup: u32,
    environment_phase: u32,
    bounce_last: Option<bool>,
    bounce_busy: u32,
    parity: bool,
    light_key: Option<[u8; 32]>,
    light_streak: bool,
    relight_catchup: u32,
    relight_phase: u32,
}
impl Default for Planner {
    fn default() -> Self {
        Self {
            bounds: vec![],
            poses: vec![],
            environment: None,
            frame: 0,
            pending_clear: false,
            since_clear: u32::MAX,
            idle_key: None,
            idle_frames: 0,
            view_key: None,
            view_frames: 0,
            environment_streak: false,
            environment_catchup: 0,
            environment_phase: 0,
            bounce_last: None,
            bounce_busy: 0,
            parity: false,
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
    }
    /// A new table: forget what the old one was planned from and start it empty.
    pub(crate) fn reallocated(&mut self) {
        self.environment = None;
        self.bounds.clear();
        self.pending_clear = true;
    }
    /// Frames since the cache was last emptied.
    #[cfg(test)]
    pub(crate) fn since_clear(&self) -> u32 {
        self.since_clear
    }
    /// The table header for this frame. `run`: the cache's passes are wanted this frame;
    /// `bounce`: its bounce passes too.
    pub(crate) fn begin(&mut self, c: &PlanConfig, run: bool, bounce: bool) -> Begun {
        if run {
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
        let coarse = s.claim_cells != u32::MAX && since_clear < COARSE_CUT_FRAMES;
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
        // The cut refill: S groups over the S cache frames that start at a reset.
        let cut_phases = match s.cut_phases {
            0 if software => 4,
            0 => 1,
            phases => phases,
        };
        let spread = cut_phases > 1 && since_clear < cut_phases;
        header[14] = if spread { cut_phases } else { 0 };
        // Software rays with cooked near-field masks: their octahedral bins per side (0: none).
        if software {
            header[16] = c.near_bins;
        }
        header[17] = u32::from(bounce);
        header[18] = layout.shade;
        header[19] = s.gathers;
        header[20] = match s.gather_cells {
            0 => GATHER_CELLS,
            u32::MAX => 0,
            cells => cells,
        };
        header[21] = s.gather_stride;
        header[22] = match s.claim_cells {
            0 if software => CLAIM_CELLS_SOFTWARE,
            0 => CLAIM_CELLS_HARDWARE,
            u32::MAX => 0,
            cells => cells,
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
        Begun {
            header,
            since_clear,
            cut_phases,
            filter,
        }
    }
    /// The frame record and the passes to run. `bounce` as given to `begin`.
    pub(crate) fn finish(
        &mut self,
        c: &PlanConfig,
        begun: &Begun,
        bounce: bool,
        inputs: FrameInputs<'_>,
    ) -> Finished {
        let s = c.settings;
        let layout = WorkLayout::of_counts(c.slots, c.misses);
        let (since_clear, cut_phases, filter) = (begun.since_clear, begun.cut_phases, begun.filter);
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
        } = inputs;
        let mut boxes: Vec<[f32; 6]> = vec![];
        let previous = std::mem::take(&mut self.bounds);
        let previous_poses = std::mem::replace(&mut self.poses, poses.clone());
        if !previous.is_empty() {
            // A skinned caster can change within identical aggregate bounds; P-SHOW: only a
            // new pose does (one per placement, in `bounds` order, collected once; P-GI:
            // searching the draws per placement made this loop quadratic in placements).
            for i in 0..bounds.len().max(previous.len()) {
                let (a, b) = (previous.get(i), bounds.get(i));
                if a == b && !crate::renderer::skinning::deformed(&previous_poses, &poses, i) {
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
        self.bounds = bounds;
        if boxes.len() > MAX_BOXES {
            let all = boxes.iter().fold(boxes[0], |u, b| {
                std::array::from_fn(|k| {
                    if k < 3 {
                        u[k].min(b[k])
                    } else {
                        u[k].max(b[k])
                    }
                })
            });
            boxes = vec![all];
        }
        // An environment change re-integrates every cell from its bits (no rays):
        // yaw rotates the sky, a new map replaces it. A map's intensity and the constant
        // sky are applied at lookup, so their changes re-integrate nothing.
        let environment_changed = self.environment.is_some_and(|e| e != environment);
        self.environment = Some(environment);
        let mut flags = 0u32;
        if !boxes.is_empty() && s.invalidate_casters {
            flags |= 1;
        }
        // Partially traced cells re-queue: form 2 always; the cut refill on the frames after
        // its first, and one frame past its window, where a cell still partial (made late in
        // the window) traces every remaining direction.
        let refill = cut_phases > 1 && (1..=cut_phases).contains(&since_clear);
        if s.direction_phases > 1 || refill {
            flags |= 8;
        }
        if s.rotation_period != 0 {
            flags |= 2;
        }
        // A step after a still frame re-integrates every cell; a map changing on
        // consecutive frames, and the ENVIRONMENT_PERIOD frames after it last changed,
        // re-integrate one fixed slot group a frame (ENVIRONMENT_PERIOD).
        let integrate_all = environment_changed && !self.environment_streak;
        let spread = !integrate_all && (environment_changed || self.environment_catchup > 0);
        self.environment_catchup = if integrate_all {
            0
        } else if environment_changed {
            ENVIRONMENT_PERIOD
        } else {
            self.environment_catchup.saturating_sub(1)
        };
        self.environment_streak = environment_changed;
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
        // consecutive frames, a moved caster, and the RELIGHT_PERIOD frames after either
        // relight one fixed slot group a frame, one gather each (W2: a fixed rotation).
        let mut relight = 0u32;
        if bounce {
            let changed = self.light_key.is_some() && self.light_key != light_key;
            let caster = !boxes.is_empty() && s.invalidate_casters;
            let all = changed && !self.light_streak;
            let spread = !all && (changed || caster || self.relight_catchup > 0);
            self.relight_catchup = if all {
                0
            } else if changed || caster {
                RELIGHT_PERIOD
            } else {
                self.relight_catchup.saturating_sub(1)
            };
            self.light_streak = changed;
            if all {
                relight = 1 << 31;
            } else if spread {
                self.relight_phase = (self.relight_phase + 1) % RELIGHT_PERIOD;
                relight = RELIGHT_PERIOD | (self.relight_phase << 8);
            }
            if relight != 0 {
                flags |= 16;
            }
        }
        self.light_key = light_key;
        // Gathers scheduled by a relight run on the frames after it, even where the view is
        // otherwise idle.
        let busy = bounce && self.bounce_busy > 0;
        self.bounce_busy = if bounce && relight != 0 {
            s.gathers + 2
        } else {
            self.bounce_busy.saturating_sub(1)
        };
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
        let [queue, next] = if self.parity {
            [layout.queues[1], layout.queues[0]]
        } else {
            layout.queues
        };
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
        let groups = [
            (region[2] - region[0]).div_ceil(probe_tile()),
            (region[3] - region[1]).div_ceil(probe_tile()),
        ];
        let clear = std::mem::take(&mut self.pending_clear);
        // A view whose every input is unchanged has nothing to trace once its cells
        // exist: after IDLE_AFTER identical frames the passes stop (still frames cost
        // nothing). Rotation and partial cells keep working every frame.
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
        if self.idle_key == Some(idle_key) && !clear {
            self.idle_frames = self.idle_frames.saturating_add(1);
        } else {
            self.idle_frames = 0;
        }
        self.idle_key = Some(idle_key);
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
        if self.view_key == Some(view_key) && !clear {
            self.view_frames = self.view_frames.saturating_add(1);
        } else {
            self.view_frames = 0;
        }
        self.view_key = Some(view_key);
        // A gather cap spreads a cut's gathers over more frames: still frames keep running
        // until a spread of up to 64 slot groups per gather generation can have drained.
        let idle_after = match (bounce, s.gather_cells) {
            (false, _) => IDLE_AFTER,
            (true, u32::MAX) => IDLE_AFTER_BOUNCE,
            (true, _) => IDLE_AFTER_BOUNCE + 64 * (s.gathers + 1),
        };
        if self.idle_frames >= idle_after && flags == 0 && !busy {
            return Finished { frame, plan: None };
        }
        // With bounce a changing environment relights (flags 16), so frames that are
        // environment-only never run bounce.
        let environment_only = self.view_frames >= IDLE_AFTER && flags == 4;
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
            }),
        }
    }
    /// The planned frame's passes are encoded: count it.
    pub(crate) fn advance(&mut self, begun: &Begun, environment_only: bool) {
        if !environment_only {
            self.since_clear = begun.since_clear.saturating_add(1);
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
    profile: Option<Profile>,
    groups: Option<Groups>,
    retired: Vec<(u64, Retired)>,
    stats: IndirectStats,
    /// Bounce (G2). Stable draw ids: a draw's vertex buffer at index id - 1, so a cell's
    /// material names its placement across draw-order changes (shaders/gi_cells.wgsl
    /// GI_C_MATERIAL); the draw table last uploaded.
    draw_ids: Vec<(wgpu::Buffer, wgpu::Buffer)>,
    draw_table: Vec<u32>,
    shade: Option<ShadePipeline>,
    /// MP1: mixed-program frames' cell shading, one pipeline per reflecting program of the
    /// frame (`mixed_cell_entry`), used instead of `shade` while `mixed_active`.
    mixed: Vec<ShadePipeline>,
    mixed_active: bool,
    /// The CPU history and per-frame plan (shared with the ara-gpu backend).
    planner: Planner,
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
            planner: Planner::default(),
            profile: None,
            groups: None,
            retired: vec![],
            stats: IndirectStats::default(),
            draw_ids: vec![],
            draw_table: vec![],
            shade: None,
            mixed: vec![],
            mixed_active: false,
        }
    }
    /// The table shading binds (lighting group 3, binding 20).
    pub fn shading_table(&self) -> &wgpu::Buffer {
        self.resources.as_ref().map_or(&*self.dummy, |r| &*r.table)
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
}

/// The kernel module: built-in prelude, primary ray helpers, cell definitions, one ray
/// source and the kernels.
pub(crate) fn kernel_source(software: bool, directions: u32) -> String {
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
        let pending = self.gi.planner.pending_clear();
        self.set_indirect_geometry(mesh, positions, indices)?;
        self.gi.planner.set_pending_clear(pending);
        Ok(true)
    }
    /// Selects world radiance cache settings. Enabling allocates the table (all
    /// slots empty); a new slot count reallocates it. Takes effect on the next lit
    /// traced or visibility-buffer frame whose programs call `indirect_irradiance`.
    pub fn set_indirect(&mut self, settings: IndirectSettings) -> Result<(), Error> {
        self.check_fault()?;
        settings.validate()?;
        let slots = self.gi_slots(settings.slots_log2);
        // The filter's sources are sized by the setting (G3).
        let raw_slots = if settings.filter != 0 { slots } else { 1 };
        let reallocate = settings.enabled
            && self.gi.resources.as_ref().is_none_or(|r| {
                r.slots != slots || r.raw_slots != raw_slots || r.directions != settings.directions
            });
        if reallocate {
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
            } = Sizes::of(slots, self.lighting.pending.count, &self.gi.settings);
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
                raw: buffer("indirect cache filter sources", raw_bytes, storage),
                slots,
                cold_words,
                directions,
                misses,
                raw_slots,
                _allocations: [
                    self.ledger.allocate(AllocationKind::Light, table_bytes),
                    self.ledger.allocate(AllocationKind::Light, work_bytes),
                    self.ledger.allocate(AllocationKind::Light, outgoing_bytes),
                    self.ledger.allocate(AllocationKind::Light, draws_bytes),
                    self.ledger.allocate(AllocationKind::Light, raw_bytes),
                ],
            };
            self.gi.stats.table_bytes = table_bytes;
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
        let mut blas = Blas::new(bvh, 0)?;
        blas.exact = true;
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
    fn set_indirect_blas(&mut self, mesh: &Mesh, blas: Blas) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.identity, &mesh.renderer) {
            return Err(Error::InvalidInput("mesh belongs to another renderer"));
        }
        let key: wgpu::Buffer = (**mesh.vertex).clone();
        self.gi.meshes.retain(|(k, _)| *k != key);
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
        self.gi.planner.set_pending_clear(true);
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
        let queries = self.ledger.own(
            self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("indirect cache passes"),
                ty: wgpu::QueryType::Timestamp,
                count: PROFILE_STAMPS * PROFILE_RING as u32,
            }),
            ResourceKind::QuerySet,
        );
        let bytes = PROFILE_STRIDE * PROFILE_RING;
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
            PROFILE_STRIDE * PROFILE_RING,
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
                let at = ((frame % PROFILE_RING) * PROFILE_STRIDE / 8) as usize;
                let pass = |i: usize| {
                    words[at + 2 * i + 1].saturating_sub(words[at + 2 * i]) as f64 * period
                };
                IndirectProfile {
                    probe: pass(0),
                    claim: (0..rounds).map(|k| pass(1 + 2 * k)).sum(),
                    resolve: (0..rounds).map(|k| pass(2 + 2 * k)).sum(),
                    scan: pass(1 + 2 * rounds),
                    setup: pass(2 + 2 * rounds),
                    rays: pass(3 + 2 * rounds),
                    finish: pass(4 + 2 * rounds),
                    integrate: pass(5 + 2 * rounds),
                    gather: pass(6 + 2 * rounds),
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
            gather_unshaded: w[40],
            gather_orphans: w[41],
            gather_deferred: w[42],
            claim_deferred: w[43],
        }))
    }
    /// Blocks until the GPU is idle and copies the whole table (determinism gates).
    pub fn indirect_table(&mut self) -> Result<Option<Vec<u32>>, Error> {
        let Some(r) = self.gi.resources.as_ref() else {
            return Ok(None);
        };
        let size = r.table.size();
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect cache table readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.table, 0, &readback, 0, size);
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
        let id = state.program;
        let programs = state.programs.clone();
        self.gi.mixed_active = programs.len() > 1;
        if programs.len() > 1 {
            let mut built = vec![];
            for id in programs {
                if !crate::program::reflects(&self.program_slot(id).source) {
                    continue;
                }
                let mut existing = std::mem::take(&mut self.gi.mixed);
                let key = self.gi_shade_key(id, software, streamed);
                let pipeline = match existing.iter().position(|p| p.key == key) {
                    Some(i) => Ok(existing.swap_remove(i)),
                    None => self.gi_shade_pipeline(id, key, true),
                };
                self.gi.mixed = existing;
                built.push(pipeline?);
            }
            let any = !built.is_empty();
            self.gi.mixed = built;
            return Ok(any);
        }
        if !crate::program::reflects(&self.program_slot(id).source) {
            return Ok(false);
        }
        let key = self.gi_shade_key(id, software, streamed);
        if self.gi.shade.as_ref().is_some_and(|p| p.key == key) {
            return Ok(true);
        }
        self.gi.shade = Some(self.gi_shade_pipeline(id, key, false)?);
        Ok(true)
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
        let scopes = OperationScopes::new(&self.device);
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
        let pipeline = self.ledger.own(
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("gi_shade_cells"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some("gi_shade_cells"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            ResourceKind::ComputePipeline,
        );
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(error.to_string()));
        }
        Ok(ShadePipeline { key, pipeline })
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
        let hardware = if rt {
            [&common[..], &[tlas]].concat()
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
        let shade = group(
            "indirect cache cell shading",
            &[storage(31, true), storage(33, false), storage(36, true)],
        );
        self.gi.layouts = Some(Layouts {
            main,
            arrays,
            pipelines,
            shade,
        });
    }

    fn ensure_gi_pipelines(&mut self, software: bool) -> Result<(), Error> {
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
        let compute = |entry: &str, layout: &wgpu::PipelineLayout| {
            self.ledger.own(
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                ResourceKind::ComputePipeline,
            )
        };
        let main = &layouts.pipelines[usize::from(software)];
        let pipelines = Pipelines {
            software,
            directions,
            probe: compute("gi_probe", main),
            claim: compute("gi_claim", main),
            resolve: compute("gi_resolve", main),
            resolve_last: compute("gi_resolve_last", main),
            scan: compute("gi_scan", main),
            setup: compute("gi_setup", main),
            rays: compute("gi_rays", main),
            finish: compute("gi_finish", main),
            integrate: compute("gi_integrate", main),
            gather: compute(
                if software {
                    "gi_gather_batched"
                } else {
                    "gi_gather"
                },
                main,
            ),
            light: compute("gi_light", main),
            filter: compute("gi_filter", main),
        };
        if let Some(error) = scopes.finish() {
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
            let table = buffer("indirect cache placement draws", count * 4);
            let absent = buffer("indirect cache absent storage", 48);
            let words: Vec<u8> = placements.iter().flat_map(|v| v.to_ne_bytes()).collect();
            if !words.is_empty() {
                self.queue.write_buffer(&table, 0, &words);
            }
            self.ensure_gi_layouts();
            fn pad<'a>(
                buffers: impl Iterator<Item = &'a wgpu::Buffer>,
                absent: &'a wgpu::Buffer,
            ) -> Vec<wgpu::BufferBinding<'a>> {
                buffers
                    .chain(std::iter::repeat(absent))
                    .take(TRACED_DRAWS as usize)
                    .map(|b| b.as_entire_buffer_binding())
                    .collect()
            }
            let vertices = pad(draws.iter().map(|d| &**d.mesh.vertex), &absent);
            let indices = pad(draws.iter().map(|d| &**d.mesh.index), &absent);
            let bind = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            let next = Arrays {
                keys,
                models,
                normals,
                _placements: table,
                _absent: absent,
                instances: vec![],
                bind,
            };
            if let Some(old) = self.gi.arrays.replace(next) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Arrays(old)));
            }
        }
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
        let a = self.gi.arrays.as_mut().unwrap();
        if a.instances != instances {
            let (models, normals) = instances.split_at(instances.len() / 2);
            if !models.is_empty() {
                self.queue.write_buffer(&a.models, 0, models);
                self.queue.write_buffer(&a.normals, 0, normals);
            }
            a.instances = instances;
        }
        Ok(())
    }

    /// Packs every draw's software geometry; false when a draw has none.
    /// Packs every draw's software geometry; false when a draw has none. `ids` are the draws'
    /// stable ids (`gi_draw_ids`), which name each instance's material (G2).
    fn prepare_gi_geometry(
        &mut self,
        draws: &[PreparedDraw<'_>],
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
        let meshes: Vec<wgpu::Buffer> = draws.iter().map(|d| (**d.mesh.vertex).clone()).collect();
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
            + usize::from(!covered.is_empty());
        if self.gi.packed.as_ref().is_none_or(|p| {
            p.meshes != meshes || p.instances != instances || p.covered != covered || p.ids != ids
        }) {
            // Pack each draw's mesh unless the world covers all its instances, then the world.
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
        // world zero and its placement map's offset and length.
        let mut identities = vec![];
        if !p.covered.is_empty() {
            let w = &self.gi.world.as_ref().unwrap().blas;
            records.push((Mat4::IDENTITY, &**w, *p.offsets.last().unwrap(), 0));
            identities.push([0, p.map.0, p.map.1]);
        }
        let mut at = 0;
        for (di, ((d, blas), entry)) in draws.iter().zip(&blases).zip(&p.entry).enumerate() {
            for (ii, model) in d.instances.state.models.iter().enumerate() {
                let covered = p.covered.get(at).copied().unwrap_or(false);
                at += 1;
                if covered {
                    continue;
                }
                let e = entry.unwrap();
                records.push((
                    *model,
                    &**blas,
                    p.offsets[e],
                    blas.near_base(model, &p.near[e]),
                ));
                identities.push([draw_identity(ids, di, ii), 0, 0]);
            }
        }
        self.gi.stats.near_instances = records.iter().filter(|r| r.3 != 0).count() as u32;
        let mut head = gi_bvh::instance_words(&records)?;
        for (i, words) in identities.iter().enumerate() {
            let r = gi_bvh::HEADER_WORDS + i * gi_bvh::INSTANCE_WORDS + 24;
            head[r..r + 3].copy_from_slice(words);
        }
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
            self.allocate_gi(self.gi_slots(self.gi.settings.slots_log2))?;
        }
        // LC2: changing light count may outgrow the bitset. Existing in-flight tables
        // retire normally; scheduling and cache lookup are otherwise unchanged.
        if self.gi.settings.enabled
            && self.gi.resources.as_ref().is_some_and(|r| {
                r.cold_words
                    < light_cold_words(self.lighting.pending.count, self.gi.settings.directions)
            })
        {
            self.allocate_gi(self.gi_slots(self.gi.settings.slots_log2))?;
        }
        let hardware = self.hardware_shadows.is_some() && !self.gi.settings.force_software;
        let run = self.gi.settings.enabled && eligible && self.gi_callers(draws);
        let software = !hardware;
        let ids = self.gi_draw_ids(draws);
        let run = run && (hardware || self.prepare_gi_geometry(draws, &ids)?);
        // Bounce (G2): the setting is on and this frame's program reflects light. A change
        // empties the table, so no cell keeps bounce light from another program's frames.
        let bounce = run && self.gi.settings.bounce && self.ensure_gi_shade(software)?;
        let r = self.gi.resources.as_ref().unwrap();
        let config = PlanConfig {
            settings: self.gi.settings,
            slots: r.slots,
            misses: r.misses,
            cold_words: r.cold_words,
            raw_slots: r.raw_slots,
            software,
            field: self.environment.selected.is_some(),
            profiling: self.gi.profile.is_some(),
            near_bins: self.gi.near_bins,
            eye: camera.view().inverse().w_axis.truncate(),
            fov_y: camera.fov_y(),
            size,
        };
        let begun = self.gi.planner.begin(&config, run, bounce);
        let raw: Vec<u8> = begun.header.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.queue.write_buffer(&r.table, 0, &raw);
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
            let selector = if mixed && crate::program::reflects(&self.program_slot(program).source)
            {
                cell_selector(program)
            } else {
                0
            };
            [ids[b], n, selector]
        });
        let table = draw_table(draws.len(), rows);
        if self.gi.draw_table != table {
            let raw: Vec<u8> = table.iter().flat_map(|w| w.to_le_bytes()).collect();
            self.queue.write_buffer(&r.draws, 0, &raw);
            self.gi.draw_table = table;
        }
        self.ensure_gi_pipelines(software)?;
        self.prepare_gi_arrays(draws)?;
        // Moved casters: placements whose world bounds changed, as swept boxes.
        let mut bounds = vec![];
        for d in draws {
            for &model in &d.instances.state.models {
                let b = shadow_gpu::world_bounds(d.mesh.bounds, model)?;
                bounds.push([b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z]);
            }
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
        let light_key: Option<[u8; 32]> = bounce.then(|| {
            let state = self.view.primary.as_ref().unwrap();
            let mut h = Sha256::new();
            h.update(format!("{:?}", self.lighting.source_lights).as_bytes());
            // DC1: altered surface inputs relight bounce cells on the existing schedule.
            h.update(self.lighting.decals.hash);
            // TP2: transparent sun attenuation changes the cells' direct illumination.
            h.update(self.lighting.transmission.hash);
            h.update(environment);
            h.update(self.program_slot(state.program).source.as_bytes());
            // MP1: every program of a mixed frame.
            for &program in state.programs.iter().skip(1) {
                h.update(self.program_slot(program).source.as_bytes());
            }
            h.update(ids.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<_>>());
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
        let finished = self.gi.planner.finish(
            &config,
            &begun,
            bounce,
            FrameInputs {
                bounds,
                poses,
                environment,
                light_key,
                region,
                still_matrix: matrix_bytes(still.0.matrix(size[0], size[1])),
                still_extent: still.1,
                instances: &self.gi.arrays.as_ref().unwrap().instances,
                skin_revisions,
            },
        );
        let r = self.gi.resources.as_ref().unwrap();
        self.queue.write_buffer(&r.frame, 0, &finished.frame);
        let Some(plan) = finished.plan else {
            self.gi.stats.idle_frames += 1;
            return Ok(());
        };
        let environment_only = plan.environment_only;
        if plan.bounce {
            self.gi.stats.bounce_frames += 1;
        }
        self.gi.plan = Some(plan);
        self.prepare_gi_groups(software, scene_generation)?;
        self.gi.stats.frames += 1;
        self.gi.stats.software = software;
        self.gi.planner.advance(&begun, environment_only);
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
const CELL_STORE_SEAM: &str = "        gi_shade_out[slot]=gs_shade(slot);\n";
/// A mixed frame's cell entry for program `id`: it shades only cells whose draw selects
/// it, writes "reflects nothing" for draws whose program has no cell shading (every
/// program's dispatch writes the same value), and leaves other programs' cells to theirs.
pub(crate) fn mixed_cell_entry(source: &str, id: u64) -> String {
    assert_eq!(source.matches(CELL_PLACEMENT_SEAM).count(), 1);
    assert_eq!(source.matches(CELL_STORE_SEAM).count(), 1);
    let check = format!(
        "{CELL_PLACEMENT_SEAM}    let gs_program=gi_shade_draws[primary_placements[p]&0x1fffffffu].w;\n    if gs_program==0u {{ return vec4<f32>(0.0,0.0,0.0,1.0); }}\n    if gs_program!={}u {{ return vec4<f32>(-1.0); }}\n",
        cell_selector(id)
    );
    source.replacen(CELL_PLACEMENT_SEAM, &check, 1).replacen(
        CELL_STORE_SEAM,
        "        let value=gs_shade(slot);\n        if value.w>=0.0 {gi_shade_out[slot]=value;}\n",
        1,
    )
}
// ---- end of MP1 per-program cell shading ----

/// What cell shading (G2) binds besides its own group 0: traced shading's geometry (group 1)
/// and materials (group 2), and lighting's shading group, whose binding 20 is the real table.
pub(super) struct ShadeGroups<'a> {
    pub arrays: &'a wgpu::BindGroup,
    pub materials: &'a wgpu::BindGroup,
    pub lighting: &'a wgpu::BindGroup,
}

impl GiState {
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
        let Some(plan) = self.plan.take() else {
            return;
        };
        mark(encoder, "gi_probe");
        let r = self.resources.as_ref().unwrap();
        let p = self.pipelines.as_ref().unwrap();
        let g = self.groups.as_ref().unwrap();
        let arrays = &self.arrays.as_ref().unwrap().bind;
        let layout = WorkLayout::of(r);
        if plan.clear {
            encoder.clear_buffer(&r.table, HEADER_WORDS * 4, None);
            encoder.clear_buffer(&r.outgoing, 0, None);
            for at in layout.queues {
                encoder.clear_buffer(&r.work, u64::from(at) * 4, Some(4));
            }
        }
        encoder.clear_buffer(&r.work, 0, Some(COUNTER_WORDS * 4));
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
            .map_or(0, |p| (p.written % PROFILE_RING) as u32 * PROFILE_STAMPS);
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
                timestamp_writes: stamps(stamp),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*g.main, &[]);
            pass.set_bind_group(1, &**arrays, &[]);
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
        // passes are empty, so every stamp is written.
        let traced = !plan.environment_only;
        if traced || profiling {
            main_pass(
                encoder,
                "indirect cache probe",
                &p.probe,
                0,
                traced.then_some(plan.groups),
            );
        }
        // Bounce gathers (G2) read last frame's outgoing radiance and may request hit cells
        // for the claim rounds that follow.
        let b = 6 + 2 * CLAIM_ROUNDS;
        mark(encoder, "gi_gather");
        if plan.bounce || profiling {
            let grid = plan.bounce.then_some([GATHER_GROUPS, 1]);
            main_pass(encoder, "indirect cache gather", &p.gather, b, grid);
        }
        mark(encoder, "gi_claim");
        if traced || profiling {
            for k in 0..CLAIM_ROUNDS {
                let grid = traced.then_some([CLAIM_GROUPS, 1]);
                main_pass(encoder, "indirect cache claim", &p.claim, 1 + 2 * k, grid);
                let last = k + 1 == CLAIM_ROUNDS;
                let resolve = if last { &p.resolve_last } else { &p.resolve };
                main_pass(encoder, "indirect cache resolve", resolve, 2 + 2 * k, grid);
            }
        }
        let s = 1 + 2 * CLAIM_ROUNDS;
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
            main_pass(encoder, "indirect cache rays", &p.rays, s + 2, rays);
            let finish = cells.filter(|_| traced);
            main_pass(encoder, "indirect cache finish", &p.finish, s + 3, finish);
        }
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
            let pipelines: Vec<_> = if self.mixed_active {
                self.mixed.iter().collect()
            } else {
                self.shade.iter().collect()
            };
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
        if let Some(profile) = self.profile.as_mut() {
            let slot = profile.written % PROFILE_RING;
            encoder.resolve_query_set(
                &profile.queries,
                ring..ring + PROFILE_STAMPS,
                &profile.resolve,
                slot * PROFILE_STRIDE,
            );
            profile.written += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{Stage, compose, uses_indirect};
    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");

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
