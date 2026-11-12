//! TR1 temporal reconstruction of the final lit image (docs/m7-tr1.md), on by default and set
//! through `Renderer::set_reconstruction`. Each traced or visibility-buffer lit frame samples every
//! pixel at one sub-pixel offset of a rank-1 lattice, carried by the camera projection
//! (`Camera::jittered`), so both primary paths and every program's shading follow it with
//! no shader edit. On RT renderers, Interactive's eight disk directions also rotate by the
//! frame through the unused `PrimaryFrame.sided.y` word. After the program's shading, one
//! pass (shaders/temporal_resolve.wgsl) continues each pixel's history where its current
//! sample's surface point was in the previous frame, accepted by surface identity and
//! engine events only, and starts revealed pixels from this frame's samples. Every colour
//! history is one surface's; a silhouette pixel keeps a second slot for the other surface of
//! its 3x3 and the occluder's coverage of its footprint (TR1b, docs/m7-tr1b.md).
//!
//! Its histories, weights, coverages and anchors are read only by that pass, next frame
//! (P0's W5); no later stage takes them as input.
// PC1: only the device type changes; pipeline creation uses the shared cache.
use super::*;
use crate::pipeline_compilation::PipelineDevice;

/// Temporal reconstruction settings (TR1). On by default since TR1b (docs/m7-tr1b.md):
/// traced and visibility-buffer lit frames at 1x are jittered and accumulate; raster,
/// diagnostic and Temporal-shadow frames are unaffected. `Reconstruction::off()` is the named
/// switch back to unjittered single frames with no history. History costs 40 bytes per pixel
/// plus an 8-byte copy of the frame's samples. The other defaults are the variant measured
/// best on the Q-walk and visible-sky screens (docs/m7-tr1b.md); the default still misses
/// TSR Epic on moving edges and revealed pixels (docs/m7-gaps.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reconstruction {
    pub enabled: bool,
    /// The history's largest sample weight on still pixels.
    pub window: f32,
    /// Bilinear resampling blur budget v_max (px²): a moving pixel's window is
    /// `1 + blur_budget / sum f(1 - f)` at fractional fetch offset f (T1b's rule).
    pub blur_budget: f32,
    /// Window of moving pixels whose 4x4 history taps are all accepted, which resample by
    /// Catmull-Rom (second-moment blur free) instead of bilinear.
    pub sharp_window: f32,
    /// P-FLICKER (docs/p-flicker.md): Catmull-Rom resampling also blurs, by an amount that
    /// changes with the fetch fraction f every frame, so a fixed `sharp_window` shimmers on
    /// moving texture. A positive budget caps that window at `1 + sharp_blur_budget / sum
    /// f(1 - f)`, as `blur_budget` does for bilinear (default 2). Zero keeps the fixed window.
    pub sharp_blur_budget: f32,
    /// Window of a moving silhouette's occluder coverage, which the occluder's motion
    /// carries; still silhouettes use `window`.
    pub coverage_window: f32,
    /// Largest effective sample count a revealed pixel's spatial start may claim.
    pub start_window: f32,
    /// Tent radius (pixels) of a revealed pixel's spatial start over the 3x3 samples of its
    /// surface; 0 starts from its own sample. 1 (the default) interpolates to the pixel
    /// centre, so a cold first frame (k = 0, unjittered) is the unreconstructed image; 2
    /// blurs it (docs/m7-tr1b.md §4.6).
    pub start_radius: f32,
    /// Sun turn (radians per frame) at which the window falls to one frame; smaller turns
    /// cap it at `turn_angle / turn`. Zero restarts every pixel on any sun turn. 0.064 rad:
    /// Q-sun-fast's 10°/s turns keep about 22 frames (docs/m7-tr1b.md §4.3); TR1's 0.004
    /// restarted them every frame.
    pub turn_angle: f32,
    /// Catmull-Rom history resampling where all 16 taps are accepted.
    pub catmull_rom: bool,
    /// Revealed pixels start from this frame's 3x3 samples of their surface; otherwise
    /// from their own sample.
    pub spatial_start: bool,
    /// Rotate the sun-disk directions through the pixel's 256-direction sequence, 8 per
    /// frame: RT Interactive's, and non-RT Virtual's Exact and Surface lookups (P-NSQ).
    pub rotate_directions: bool,
    /// MQ1 (docs/m7-mq1.md): on frames where a placement's transform changed, clamp every
    /// continued history to the plain min/max box of this frame's 3x3 samples (all nine, no
    /// statistic), so light that moving objects change on other surfaces (shadows, reflections,
    /// bounce) is not carried as a trail. Frames where nothing moved never clamp.
    pub motion_box: bool,
    /// MQ2 (docs/m7-mq2.md): view budget, in pixels of a reflected image weighted by the
    /// program's declared view-dependent share (`fn view_dependence`, src/program.rs). While a
    /// surface's view direction turns, its reflections slide across it; its window is capped
    /// at `1 + view_budget / drift`, drift being that share-weighted slide per frame, so
    /// surface-identity history does not trail them. Zero turns the cap off. Programs that
    /// declare nothing are unaffected.
    pub view_budget: f32,
    /// P-RELIT (docs/p-relit.md): the exposed illuminance (lux times the frame's exposure) of
    /// stale local light a continued history may hold. On a frame where a local light's
    /// intensity or colour changed, a pixel restarts where that light's swing could put more
    /// than this on it, and continues under MQ1's 3x3 box elsewhere. The swing is the light's
    /// largest channel distance from its current candela to any candela it had since every
    /// pixel last restarted, so to any intensity a history can hold; it reaches a pixel through
    /// the shading's own attenuation (inverse square, range window). A restarted pixel follows
    /// the light exactly, as every pixel did when any such change restarted the whole frame;
    /// a continued one is within the budget of it. Only engine light data, the exposure and
    /// the receiver's position decide it, never the pixel's colour. Zero restarts every pixel
    /// in the light's range.
    pub light_budget: f32,
    /// TR2 (docs/m7-tr2.md): upscaled frames weight every internal sample of the pixel's
    /// surface by a tent whose radius shrinks from the internal sample spacing (young history
    /// interpolates the internal grid instead of repeating its nearest sample, which stepped
    /// the finest moving highlights) to one output pixel as the history reaches
    /// `sharp_window`. Off: the nearest sample alone (TR1b). Native frames are unaffected.
    pub upscale_interpolation: bool,
    /// Internal resolution of reconstructed frames: every pass renders at this fraction of
    /// the target size and TR1 reconstructs the target from the jittered samples.
    pub scale: RenderScale,
}

/// Upscaling presets of reconstructed frames (TR1b). Raster, diagnostic and unreconstructed
/// frames always render at the target size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderScale {
    /// The target size.
    #[default]
    Native,
    /// Two thirds of the target in each axis (about 44 % of its pixels).
    Quality,
    /// Half the target in each axis (a quarter of its pixels).
    Performance,
}
impl RenderScale {
    /// The internal size for a target size, each axis rounded to the nearest pixel.
    pub fn internal_size(self, width: u32, height: u32) -> [u32; 2] {
        let (n, d) = match self {
            Self::Native => (1, 1),
            Self::Quality => (2, 3),
            Self::Performance => (1, 2),
        };
        [width, height].map(|v| ((v * n + d / 2) / d).max(1))
    }
}
impl Default for Reconstruction {
    fn default() -> Self {
        Self {
            enabled: true,
            window: 256.0,
            blur_budget: 1.0,
            sharp_window: 16.0,
            // P-FLICKER (docs/p-flicker.md): quarry flicker 4.39 -> 4.32 RT, 4.44 -> 4.38 non-RT.
            sharp_blur_budget: 2.0,
            coverage_window: 16.0,
            start_window: 4.0,
            start_radius: 1.0,
            turn_angle: 0.064,
            catmull_rom: true,
            spatial_start: true,
            rotate_directions: true,
            motion_box: true,
            view_budget: 0.25,
            light_budget: LIGHT_BUDGET,
            upscale_interpolation: true,
            scale: RenderScale::Native,
        }
    }
}
impl Reconstruction {
    /// TR1 with its measured settings: the default, named for harnesses that enable it
    /// whatever the default is.
    pub fn identity() -> Self {
        Self::default()
    }
    /// No temporal reconstruction: unjittered single frames with no history (the pre-TR1b
    /// default, and every exact-repeat certification's setting).
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
    /// The quality upscaling preset: TR1 reconstructing the target from two thirds of it.
    pub fn quality() -> Self {
        Self {
            scale: RenderScale::Quality,
            ..Self::default()
        }
    }
    /// The performance upscaling preset: TR1 reconstructing the target from half of it.
    pub fn performance() -> Self {
        Self {
            scale: RenderScale::Performance,
            ..Self::default()
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let windows = [
            self.window,
            self.sharp_window,
            self.coverage_window,
            self.start_window,
        ];
        let valid = windows.iter().all(|w| (1.0..=65536.0).contains(w))
            && (0.0..=f32::MAX).contains(&self.blur_budget)
            && (0.0..=f32::MAX).contains(&self.sharp_blur_budget)
            && (0.0..=f32::MAX).contains(&self.turn_angle)
            && (0.0..=f32::MAX).contains(&self.view_budget)
            && (0.0..=f32::MAX).contains(&self.light_budget)
            && (0.0..=2.0).contains(&self.start_radius);
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidInput(
                "reconstruction windows in [1, 65536]; blur budget, turn angle and view budget finite and nonnegative; start radius in [0, 2]",
            ))
        }
    }
}

/// WGSL Tr1Frame.
const FRAME_BYTES: usize = ZONES_OFFSET + ZONES_BYTES + 16;
/// WGSL Tr1Zones (shaders/primary_rays.wgsl) and its place in Tr1Frame: where this frame's
/// resolve restarts static receivers' histories.
pub(crate) const ZONES_BYTES: usize = 48 + RELIT_SLOTS * 16 + 16;
const ZONES_OFFSET: usize = 304;
/// Restart spheres a frame carries around changed local lights; the smallest ones beyond
/// that share one bounding sphere (`relit_zones`).
const RELIT_SLOTS: usize = 8;
/// `Reconstruction::light_budget`'s default: 0.05 exposed lux, about 0.8 % of display white on
/// a mid-grey diffuse surface. In the courtyard it restarts the 5.4-6.7 m around the brazier,
/// where the fire-lit wall and ground follow the flicker as closely as a full restart did
/// (docs/p-relit.md, section 3).
const LIGHT_BUDGET: f32 = 0.05;
// Test-only tail permits the immutable MQ1 static-control shader's runtime-array
// minimum binding span. Production has only fixed words; transform history lives
// exclusively in the shared placement buffer.
const FRAME_BUFFER_BYTES: usize = FRAME_BYTES + if cfg!(test) { 64 } else { 0 };
/// `light` with its candela zeroed: a local light's identity for `reconstruction_key`.
pub(crate) fn without_candela(mut light: Light) -> Light {
    if let Light::Point { candela, .. } | Light::Spot { candela, .. } = &mut light {
        *candela = [0.; 3];
    }
    light
}
/// Each local light's candela range per channel, `[lo, hi]`, since every pixel last
/// restarted: every intensity a history can hold.
type Swing = [[f32; 3]; 2];
/// `swings` (one per local light, in order) widened by this frame's candela; a fresh range
/// for each light when `swings` is `None`.
fn widen_swings(swings: Option<&[Swing]>, intensities: &[[u32; 3]]) -> Vec<Swing> {
    intensities
        .iter()
        .enumerate()
        .map(|(i, bits)| {
            let now = bits.map(f32::from_bits);
            swings
                .and_then(|s| s.get(i))
                .map_or([now, now], |[lo, hi]| {
                    [
                        std::array::from_fn(|c| lo[c].min(now[c])),
                        std::array::from_fn(|c| hi[c].max(now[c])),
                    ]
                })
        })
        .collect()
}
/// The radius around a local light inside which its attenuation (program_api.wgsl
/// `light_sample`: inverse square, times the range window `(1 - (d / range)^4)^2`) is at
/// least `1 / level`; rounded up. Zero when no distance reaches it: the shading clamps the
/// distance at 1 cm, so the attenuation never passes 1e4.
fn restart_radius(range: Option<f32>, level: f32) -> f32 {
    if level.is_nan() || level < 1e-4 {
        return 0.0;
    }
    let Some(range) = range else {
        return level.sqrt();
    };
    if level == f32::INFINITY {
        return range;
    }
    // The attenuation falls with distance, so the radius is its one crossing of 1 / level.
    let reaches = |d: f32| {
        let window = (1.0 - (d / range).powi(4)).max(0.0);
        level * window * window >= d * d
    };
    let (mut lo, mut hi) = (0.0, range.min(level.sqrt()));
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        if reaches(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}
/// The restart spheres (centre, squared radius) of the local lights whose candela changed
/// from `previous` (bits, in order): each holds every point where the light's swing, the
/// largest channel distance from its current candela to either end of its `swings` range,
/// could change the exposed illuminance by `budget` or more. The key equals the previous
/// frame's, so the local lights are the same lights in the same order. At most `RELIT_SLOTS`
/// spheres: the smallest ones beyond that are replaced by one sphere bounding them.
fn relit_zones(
    previous: &[[u32; 3]],
    swings: &[Swing],
    lights: &[Light],
    exposure: f32,
    budget: f32,
) -> Vec<[f32; 4]> {
    let mut spheres: Vec<(Vec3, f32)> = lights
        .iter()
        .filter_map(|l| match *l {
            Light::Point {
                position,
                candela,
                range,
                ..
            }
            | Light::Spot {
                position,
                candela,
                range,
                ..
            } => Some((position, range, candela)),
            Light::Directional { .. } => None,
        })
        .zip(previous.iter().zip(swings))
        .filter(|((_, _, candela), (old, _))| candela.map(f32::to_bits) != **old)
        .map(|((position, range, candela), (_, [lo, hi]))| {
            let swing = (0..3)
                .map(|c| (candela[c] - lo[c]).max(hi[c] - candela[c]))
                .fold(0.0, f32::max);
            (position, restart_radius(range, swing * exposure / budget))
        })
        .filter(|&(_, radius)| radius > 0.0)
        .collect();
    if spheres.len() > RELIT_SLOTS {
        spheres.sort_by(|a, b| b.1.total_cmp(&a.1));
        let rest = spheres.split_off(RELIT_SLOTS - 1);
        let centre = rest.iter().map(|s| s.0).sum::<Vec3>() / rest.len() as f32;
        let radius = rest
            .iter()
            .map(|&(c, r)| (c - centre).length() + r)
            .fold(0.0, f32::max);
        // Rounded up: the bound must hold every sphere it replaces.
        spheres.push((centre, radius * (1.0 + 1e-5)));
    }
    spheres
        .into_iter()
        .map(|(c, r)| [c.x, c.y, c.z, (r * r).min(f32::MAX)])
        .collect()
}
/// Local lights' candela bits in order: what `reconstruction_key` leaves out of a light.
fn local_intensities(lights: &[Light]) -> Vec<[u32; 3]> {
    lights
        .iter()
        .filter_map(|l| match l {
            Light::Point { candela, .. } | Light::Spot { candela, .. } => {
                Some(candela.map(f32::to_bits))
            }
            Light::Directional { .. } => None,
        })
        .collect()
}
/// The rank-1 lattice generator of the sample sequence (film_accumulation.wgsl's).
const GENERATOR: u32 = 71;
/// The lattice's shifts in 1/256 px: x as T1's; y chosen so the 256 points are
/// point-symmetric about the pixel centre (point j's partner is 239 - j).
const SHIFT: [u32; 2] = [8, 91];
/// Lattice point of each sample k = 1..=254: T1's visiting order (bit-reversed k, the first
/// three swapped), each newly visited point followed by its point-symmetric partner.
const SEQUENCE: [u8; 254] = {
    let mut out = [0u8; 254];
    let mut used = [false; 256];
    let (mut n, mut k) = (0, 1u32);
    while n < 254 {
        let order = if k < 4 { (0x0231 >> (4 * k)) & 0xf } else { k };
        let j = order.reverse_bits() >> 24;
        let partner = (239 + 256 - j) & 0xff;
        if !used[j as usize] {
            used[j as usize] = true;
            used[partner as usize] = true;
            out[n] = j as u8;
            out[n + 1] = partner as u8;
            n += 2;
        }
        k += 1;
    }
    out
};
/// Interactive's disk directions continue by the frame's rotation word. The estimator body
/// is otherwise production's; only the reconstruction's own trace modules carry this edit.
const ROTATION_SEAM: (&str, &str) = (
    "let direction=fresh_direction(j,shift,l,u,v,view.texel.z);",
    "let direction=fresh_direction(j+primary_frame.sided.y,shift,l,u,v,view.texel.z);",
);
const RESOLVE: &str = include_str!("../shaders/temporal_resolve.wgsl");

/// Engine-side sampling for reconstructed Interactive shadows. New targets/cuts use
/// `cold_rays`, scene-history restarts use `reset_rays`, and moving/young history uses
/// `active_rays`. After `settle_frames` unchanged frames it uses `held_rays`.
/// Materials still receive the same visibility value and shading inputs.
/// P-RTS penumbra listing (`PENUMBRA_LISTING`, off by default; docs/p-rts.md) would trace
/// the budget only near shadow edges; with it off every receiver traces the budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconstructionShadowBudget {
    pub cold_rays: u32,
    pub reset_rays: u32,
    pub active_rays: u32,
    pub held_rays: u32,
    pub settle_frames: u32,
}
impl Default for ReconstructionShadowBudget {
    fn default() -> Self {
        Self {
            cold_rays: 256,
            reset_rays: 64,
            active_rays: 32,
            held_rays: 8,
            settle_frames: 16,
        }
    }
}
impl ReconstructionShadowBudget {
    fn validate(self) -> Result<(), Error> {
        if [
            self.cold_rays,
            self.reset_rays,
            self.active_rays,
            self.held_rays,
        ]
        .into_iter()
        .any(|n| !(8..=256).contains(&n) || !n.is_power_of_two())
        {
            return Err(Error::InvalidInput(
                "reconstruction shadow rays must be powers of two in 8..=256",
            ));
        }
        Ok(())
    }
    fn rays(self, cold: bool, reset: bool, still: bool, held_frames: u32) -> u32 {
        if cold {
            self.cold_rays
        } else if reset {
            self.reset_rays
        } else if still && held_frames >= self.settle_frames {
            self.held_rays
        } else {
            self.active_rays
        }
    }
}

/// Sample k's offset from the pixel centre in pixels (y down), one offset for the whole
/// image, which a projection can carry. k = 0 and 255 are the centre; k = 1..=254 are 127
/// point-symmetric pairs (odd k, then its negation) of a 256-point rank-1 lattice, in T1's
/// lattice order (film_accumulation.wgsl `film_offset`). Every odd-length prefix from k = 0
/// has a mean offset of exactly zero, so a still pixel's mean is first-order exact on smooth
/// shading (the sky's glow) every other frame, and off by at most one offset over n
/// otherwise (docs/m7-tr1b.md §4.6).
pub(crate) fn sample_offset(k: u64) -> [f32; 2] {
    let k = (k & 0xff) as usize;
    if k == 0 || k == 255 {
        return [0.0; 2];
    }
    let j = u32::from(SEQUENCE[k - 1]);
    [j + SHIFT[0], GENERATOR * j + SHIFT[1]].map(|v| ((v & 0xff) as f32 + 0.5) / 256.0 - 0.5)
}
/// P-STILL measurement knobs (docs/p-still.md): `RENDERLAB_P_STILL` is a comma list of
/// `nojitter` (offset 0, the jitter attribution baseline) and `gi-jittered` (GI idleness keyed
/// on the jittered camera, main before P-STILL). Unset in every shipping run.
pub(crate) fn p_still(flag: &str) -> bool {
    static FLAGS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FLAGS
        .get_or_init(|| std::env::var("RENDERLAB_P_STILL").unwrap_or_default())
        .split(',')
        .any(|f| f == flag)
}
/// Interactive's direction rotation for sample k: directions 8 (k mod 32) onward.
pub(crate) fn rotation(k: u64) -> u32 {
    8 * (k & 31) as u32
}

/// The reconstruction's interactive trace kernels: production's entries compiled from the
/// soft kernel module with `ROTATION_SEAM`. The near-edge check reads no direction, so
/// frames keep production's.
pub(in crate::renderer) struct TraceKernels {
    pub trace: Owned<wgpu::ComputePipeline>,
    pub repair: Option<Owned<wgpu::ComputePipeline>>,
    pub retry: Option<Owned<wgpu::ComputePipeline>>,
    pub full: Option<Owned<wgpu::ComputePipeline>>,
    /// P-RTS: the penumbra list and its full-budget trace, after the deferred repair.
    pub penumbra: Option<[Owned<wgpu::ComputePipeline>; 2]>,
}
/// Pipelines of the reconstruction, compiled by its first frame after each prelude.
pub(in crate::renderer) struct Pipelines {
    pub trace: Option<TraceKernels>,
    /// U-FIXED: visibility-buffer RT frames' sun (`vb_sun_source`), on the VB sun layout.
    pub vb_sun: Option<Owned<wgpu::ComputePipeline>>,
    pub resolve: Owned<wgpu::ComputePipeline>,
}

/// The Ara port's trace module (src/backend_ara); the wgpu renderer uses `trace_source_held`.
#[cfg_attr(not(feature = "backend-ara"), allow(dead_code))]
pub(crate) fn trace_source(soft_kernels: &str) -> Result<String, Error> {
    trace_source_with(soft_kernels, PENUMBRA_LISTING)
}
/// `trace_source` with the held budget's per-pixel history (HELD_SOURCE) in its two trace
/// entries, on their HELD_* bindings: the wgpu renderer's module. Without listing only.
pub(crate) fn trace_source_held(soft_kernels: &str) -> Result<String, Error> {
    let source = trace_source_with(soft_kernels, false)?;
    let estimate = "        visible=fresh_estimate(receiver,id.xy,size.x,0u);\n    }\n    textureStore(fresh_output,p,vec4<u32>(select((256u/tr1_rays)*visible,256u,visible==FRESH_SKIP)));";
    let returns = [
        "if !inside || covered==0u {return;}",
        "if !in_bounds || covered==0u {return;}",
    ];
    if source.matches(estimate).count() != 2
        || returns.iter().any(|r| source.matches(r).count() != 1)
    {
        return Err(Error::InvalidInput(
            "the held shadow budget requires the Interactive trace entries' seams",
        ));
    }
    let mut source = source
        .replace(
            estimate,
            &estimate
                .replacen(
                    "        visible=",
                    "        rtvis_hold(receiver,trace.record.x-1u);\n        visible=",
                    1,
                )
                .replacen(
                    "    textureStore(",
                    "    rtvis_store(p,visible);\n    textureStore(",
                    1,
                ),
        )
        .replacen(
            "        let direction=fresh_direction(j+",
            &format!("        {HELD_STOP}\n        let direction=fresh_direction(j+"),
            1,
        );
    for r in returns {
        let inside = if r.contains("in_bounds") {
            "in_bounds"
        } else {
            "inside"
        };
        source = source.replacen(
            r,
            &r.replacen(
                "{return;}",
                &format!("{{if {inside} {{rtvis_store(vec2<i32>(id.xy),FRESH_SKIP);}}return;}}"),
                1,
            ),
            1,
        );
    }
    Ok(format!("{source}\n{HELD_SOURCE}"))
}
/// U-FIXED: `trace_source_held`'s module plus `primary_vb_sun`, the RT sun traced from the
/// visibility buffer's receivers with the same budget, rotation and stops.
pub(crate) fn vb_sun_source(soft_kernels: &str) -> Result<String, Error> {
    Ok(format!(
        "{}\n{}",
        trace_source_held(soft_kernels)?,
        include_str!("../shaders/primary_vb_sun.wgsl")
    ))
}
/// The reconstruction's trace module; `listing` adds P-RTS's probes and penumbra pass,
/// otherwise the module is main's (every receiver traces the budget).
fn trace_source_with(soft_kernels: &str, listing: bool) -> Result<String, Error> {
    let (find, replace) = ROTATION_SEAM;
    if soft_kernels.matches(find).count() != 1 {
        return Err(Error::InvalidInput(
            "temporal reconstruction requires the unchanged Interactive direction seam",
        ));
    }
    let source = soft_kernels.replacen(find, replace, 1);
    let loop_seam =
        "for(var j=lane;j<FRESH_DIRECTIONS;j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u))";
    if source.matches(loop_seam).count() != 1 {
        return Err(Error::InvalidInput(
            "reconstruction shadow budget requires the fresh ray loop",
        ));
    }
    if !listing {
        // Only TR1's fused modules use the runtime budget. The legacy eight-ray and
        // Exact pipelines keep their override, direction sequence and normalization.
        // P-RELIT: each receiver traces the frame's budget, or the restart count where the
        // resolve will restart it (tr1_zone_rays); its count is normalised by its own rays.
        // U-RTVIS: below the budget, a receiver whose first sampling.y directions agree
        // stops there (COLD_PROBE_STOP).
        return Ok(format!(
            "{}\nvar<private> tr1_rays:u32=8u;\n{}",
            source
                .replace(
                    loop_seam,
                    "tr1_rays=tr1_zone_rays(world,primary_frame.sampling.x);for(var j=lane;j<tr1_rays;j+=1u)",
                )
                .replacen(
                    ROTATION_SEAM.1,
                    &format!("{COLD_PROBE_STOP}\n        {}", ROTATION_SEAM.1),
                    1,
                )
                .replace("256u/FRESH_DIRECTIONS", "256u/tr1_rays"),
            zones_source("primary_frame.zones")
        ));
    }
    // P-RTS: the full-budget estimator for listed penumbra pixels is this module's own
    // fresh_estimate text, renamed, with lanes striding the frame's budget.
    let begin = source.find("fn fresh_estimate(").unwrap();
    let end = begin + source[begin..].find("\n}\n").unwrap() + 3;
    let budget = source[begin..end]
        .replacen("fn fresh_estimate(", "fn fresh_estimate_budget(", 1)
        .replacen(
            loop_seam,
            "for(var j=lane;j<primary_frame.sampling.x;j+=PENUMBRA_LANES)",
            1,
        );
    // Probe directions: on listing frames (sampling.z = PROBE_DISK, else 0) the first
    // sampling.z are the budget's own, then the same sequence over a cone sampling.w
    // times the sun's. Other frames keep j and an exact x 1.0 slope.
    let source = source.replacen(
        ROTATION_SEAM.1,
        "let wide=primary_frame.sampling.z!=0u && j>=primary_frame.sampling.z;
        let direction=fresh_direction(select(j,j-primary_frame.sampling.z,wide)+primary_frame.sided.y,shift,l,u,v,view.texel.z*select(1.0,bitcast<f32>(primary_frame.sampling.w),wide));",
        1,
    );
    // Only TR1's fused modules use the runtime budget. The legacy eight-ray and
    // Exact pipelines keep their override, direction sequence and normalization.
    // The probe kernels trace sampling.y (= the budget unless P-RTS lists penumbrae).
    Ok(format!(
        "{}\n{budget}\n{PENUMBRA_TRACE}",
        source
            .replace(
                loop_seam,
                "for(var j=lane;j<primary_frame.sampling.y;j+=1u)",
            )
            .replace("256u/FRESH_DIRECTIONS", "256u/primary_frame.sampling.y")
    ))
}
const PENUMBRA_TRACE: &str = include_str!("../shaders/primary_penumbra.wgsl");
/// U-RTVIS (docs/rtvis.md): on frames whose probe count (sampling.y) is below the budget
/// (sampling.x), a receiver stops after the probes when they all agree, and its count is
/// normalised by the probes alone (0 or 256). Others continue to the whole budget, in
/// main's direction order, so their estimate is main's exactly.
const COLD_PROBE_STOP: &str = "if j==primary_frame.sampling.y && primary_frame.sampling.y<primary_frame.sampling.x && (visible==0u || visible==j) {tr1_rays=j;break;}";
/// U-RTVIS measurement switch: `RENDERLAB_COLD_PROBES=0` traces every receiver's whole cold
/// budget on cuts and first frames, as before the probe stop.
fn cold_probes() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_COLD_PROBES").map_or(true, |v| v != "0"))
}
/// U-RTVIS held budget (docs/rtvis.md): on frames that continue history, a still receiver
/// whose reprojected 3x3 shadow history has been unanimously lit or unanimously shadowed for
/// HELD_STREAK frames traces HELD_PROBES directions first, and stops there when they agree
/// with that history. Every other receiver traces the frame's budget, in main's order.
pub(crate) const HELD_PROBES: u32 = 8;
pub(crate) const HELD_STREAK: u32 = 4;
/// The held budget's frame record (HELD_SOURCE `RtvisHeld`): the previous unjittered camera
/// matrix, the words (enabled, probes, streak), and the moving placements' swept bounds with
/// the sun's direction and widened disk tangent.
pub(crate) const HELD_BYTES: usize = 128;
const HELD_STOP: &str = "if j==tr1_held_probes && tr1_rays==primary_frame.sampling.x && visible==select(0u,j,tr1_held_lit) {tr1_rays=j;break;}";
const HELD_SOURCE: &str = include_str!("../shaders/rtvis_held.wgsl");
/// U-RTVIS measurement switch: `RENDERLAB_HELD_PROBES=0` traces every receiver's whole moving
/// budget, as before the held stop. The history is written either way.
fn held_probes() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_HELD_PROBES").map_or(true, |v| v != "0"))
}
/// P-RTS (docs/p-rts.md). Listing frames probe PROBE_DISK of the budget's own directions
/// and PROBE_DISK more over a cone PROBE_CONE times the sun's. The wide probes find thin
/// slivers at a small sun's rim (an occluder clipping the disk also covers a large cap of
/// the wider disk); the disk probes keep full density when the light is large and a
/// blocker's shadow is no wider than the probe cone. Probes only decide which pixels trace
/// the budget; a non-listed pixel takes its window's unanimous 0 or 256.
const PROBE_DISK: u32 = 8;
const PROBE_CONE: f32 = 2.0;
/// P-RTS listing. Off by default: on RTDEF2's S2 fixture (0.04 rad light) the listed
/// frames still left near-unanimous fringe pixels unlisted (up to 0.125 visibility on
/// moving frames), which fails the zero-loss bar. Off, the trace module, pipelines and
/// targets are main's exactly. On, ARA1's eight non-quarry scenes are byte-identical and
/// the cut costs 1.1-1.7 ms instead of 9.7-17.5 ms (docs/p-rts.md).
pub(super) const PENUMBRA_LISTING: bool = false;
/// P-RTS's standalone penumbra list pass (docs/p-rts.md).
pub(in crate::renderer) const PENUMBRA_LIST: &str =
    include_str!("../shaders/primary_penumbra_list.wgsl");
/// The restart-zone tests (shaders/temporal_zones.wgsl) over `record`, the including
/// module's `Tr1Zones` member.
pub(crate) fn zones_source(record: &str) -> String {
    include_str!("../shaders/temporal_zones.wgsl").replace("ZONES", record)
}
pub(crate) fn resolve_source(wgsl: &str) -> Result<String, Error> {
    let [head, _] = non_rt_heads(wgsl)?;
    let mode = ["tr1-mask", "tr1-raw", "tr1-window"]
        .into_iter()
        .find(|m| p_still(m));
    let mut resolve = match mode {
        Some(mode) => mask_resolve(mode),
        None => RESOLVE.to_owned(),
    };
    if let Ok(k) = std::env::var("RENDERLAB_P_SPLIT_FRESH_CLAMP") {
        resolve = fresh_clamp_resolve(
            &resolve,
            k.parse()
                .map_err(|_| Error::InvalidInput("RENDERLAB_P_SPLIT_FRESH_CLAMP: a number"))?,
        );
    }
    // MV1 motion-source seam; history/resolve policy belongs to MQ1.
    Ok(format!(
        "{head}\n{}\n{resolve}\n{}",
        include_str!("../shaders/object_motion.wgsl"),
        zones_source("tr1.zones")
    ))
}

/// Measurement only (P-SPLIT crack flashes): a continued history's fresh sample is clamped to
/// its 8 same-surface neighbours' range, widened by `k` times that range on each side.
fn fresh_clamp_resolve(source: &str, k: f32) -> String {
    let seam = "var x=sample;";
    assert_eq!(source.matches(seam).count(), 1, "TR1 fresh clamp seam");
    source.replace(seam, "var x=tr1_fresh_clamp(p,s.id,sample,size);")
        + &format!(
            "
fn tr1_fresh_clamp(p:vec2<i32>,id:u32,x:vec3<f32>,size:vec2<u32>)->vec3<f32> {{
    var lo=vec3<f32>(3.4e38);var hi=vec3<f32>(-3.4e38);var n=0u;
    for(var i=0;i<9;i+=1) {{
        let t=p+vec2<i32>(i%3,i/3)-vec2<i32>(1);
        if i==4 || !tr1_inside(t,size) || tr1_ring(i)!=id {{continue;}}
        let c=textureLoad(tr1_sample,tr1_map(t),0).xyz;
        lo=min(lo,c);hi=max(hi,c);n+=1u;
    }}
    if n<2u {{return x;}}
    let r=(hi-lo)*{k:?};
    return clamp(x,lo-r,hi+r);
}}
"
        )
}

/// Measurement only (P-SPLIT crack flashes): `RENDERLAB_P_STILL=tr1-mask|tr1-raw|tr1-window`
/// writes how TR1 continued each pixel into the output's alpha, which nothing after TR1 reads
/// as colour. History and every other target are production's; production compiles
/// `RESOLVE` unchanged. Alpha: 1 restart (cut), 2 still, 3 Catmull-Rom (16 taps), 4 bilinear
/// (4 taps), 5 bilinear (1-3 taps), 6 no tap (spatial start), 7 rejected (caster or relit
/// light); +16 on silhouettes; 32 a third-surface pixel. `tr1-raw` writes this frame's sample
/// as the colour, `tr1-window` the history weight and window before the sample (r, g).
fn mask_resolve(mode: &str) -> String {
    let colour = match mode {
        "tr1-raw" => "textureLoad(tr1_sample,tr1_map(p),0).xyz",
        "tr1-window" => "vec3<f32>(tr1_mask_history,0.0)",
        _ => "",
    };
    let film = |film: &str| {
        if colour.is_empty() {
            film.to_owned()
        } else {
            colour.to_owned()
        }
    };
    let edits = [
        (
            "if shown==0u || total<=0.0 {return h;}",
            "if shown==0u || total<=0.0 {return h;}\n    tr1_mask_last=select(select(5.0,4.0,shown==4u),2.0,tr1_still(s));"
                .to_owned(),
        ),
        (
            "h.colour=clamp(sharp,lo,hi);",
            "h.colour=clamp(sharp,lo,hi);tr1_mask_last=3.0;".to_owned(),
        ),
        (
            "if tr1.flags.x==0u {h=tr1_history(p,s,size);}",
            "tr1_mask_last=select(6.0,1.0,tr1.flags.x!=0u);if tr1.flags.x==0u {h=tr1_history(p,s,size);}\n    tr1_mask_own=tr1_mask_last;tr1_mask_history=vec2<f32>(h.weight,h.window);".to_owned(),
        ),
        (
            "textureStore(tr1_output,p,vec4<f32>(film.xyz,1.0));",
            format!("textureStore(tr1_output,p,vec4<f32>({},tr1_mask_own));", film("film.xyz")),
        ),
        (
            "textureStore(tr1_output,p,vec4<f32>(film,1.0));",
            format!("textureStore(tr1_output,p,vec4<f32>({},tr1_mask_own+16.0));", film("film")),
        ),
        (
            "textureStore(tr1_output,p,vec4<f32>(mix(theirs,mine.xyz,own.coverage),1.0));",
            format!(
                "textureStore(tr1_output,p,vec4<f32>({},32.0));",
                film("mix(theirs,mine.xyz,own.coverage)")
            ),
        ),
    ];
    let mut source = RESOLVE.replace(
        "var rejected:Tr1History;return rejected;",
        "tr1_mask_last=7.0;var rejected:Tr1History;return rejected;",
    );
    for (from, to) in edits {
        assert_eq!(source.matches(from).count(), 1, "TR1 mask seam: {from}");
        source = source.replace(from, &to);
    }
    source
        + "\nvar<private> tr1_mask_last:f32;\nvar<private> tr1_mask_own:f32;\nvar<private> tr1_mask_history:vec2<f32>;\n"
}

impl PrimaryState {
    pub(super) fn reconstruction_layout(
        device: &PipelineDevice,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
    ) -> (Owned<wgpu::BindGroupLayout>, Owned<wgpu::PipelineLayout>) {
        use wgpu::TextureFormat as F;
        let uint = wgpu::TextureSampleType::Uint;
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("temporal reconstruction"),
                entries: &[
                    texture(7, uint),
                    uniform(10, super::FRAME_BYTES as u64),
                    texture(21, uint),
                    read(40, FRAME_BUFFER_BYTES as u64),
                    texture(41, float),
                    texture(42, float),
                    texture(43, uint),
                    image(44, F::Rgba32Float),
                    image(45, F::Rg32Uint),
                    image(46, F::Rgba16Float),
                    texture(47, uint),
                    image(48, F::Rgba32Uint),
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("temporal reconstruction ABI"),
                bind_group_layouts: &[Some(&bindings), Some(arrays)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        (bindings, layout)
    }

    /// The resolve pipeline, and on RT renderers the rotated Interactive kernels for the
    /// production entries `production` has.
    pub(super) fn reconstruction_pipelines(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        resolve: &str,
        trace: Option<(&str, &PrimaryPipelines)>,
        vb_sun: Option<&str>,
    ) -> Pipelines {
        let module = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal reconstruction"),
            source: wgpu::ShaderSource::Wgsl(resolve.into()),
        });
        let resolve = ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("tr1_resolve"),
                layout: Some(&self.reconstruction.1),
                module: &module,
                entry_point: Some("tr1_resolve"),
                compilation_options: Default::default(),
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        );
        let trace = trace.map(|(source, production)| {
            let soft = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("temporal reconstruction Interactive trace"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let eight = [("FRESH_DIRECTIONS", 8.0)];
            let modern = production.interactive_retry.is_some();
            let deferred = production.interactive_repair.is_some();
            TraceKernels {
                // U-RTVIS: both trace entries carry the held history (trace_source_held).
                trace: self.held_entry(
                    device,
                    ledger,
                    &soft,
                    if deferred {
                        "primary_trace_interactive_deferred"
                    } else {
                        "primary_trace_interactive"
                    },
                    &eight,
                ),
                // The modern near-edge check traces no direction: production's is reused.
                repair: (deferred && !modern).then(|| {
                    self.interactive_entry(
                        device,
                        ledger,
                        &soft,
                        "primary_repair_interactive",
                        &eight,
                    )
                }),
                retry: modern.then(|| {
                    self.interactive_entry(
                        device,
                        ledger,
                        &soft,
                        "primary_retry_interactive",
                        &eight,
                    )
                }),
                full: deferred.then(|| {
                    self.held_entry(device, ledger, &soft, "primary_trace_interactive", &eight)
                }),
                // The penumbra pass follows the retained two-pass repair only.
                penumbra: (deferred && !modern && PENUMBRA_LISTING)
                    .then(|| self.penumbra_pipelines(device, ledger, &soft)),
            }
        });
        let vb_sun = vb_sun.map(|source| {
            let module = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("visibility-buffer RT sun"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            self.vb_sun_entry(device, ledger, &module)
        });
        Pipelines {
            trace,
            vb_sun,
            resolve,
        }
    }
}

/// The previous reconstructed frame's inputs.
struct Previous {
    // MV1: match the instance snapshot's rendered frame, excluding upload submissions.
    frame: u64,
    matrix: Mat4,
    jitter: [f32; 2],
    eye: Vec3,
    exposure: f32,
    /// Directional lights' travel directions.
    suns: Vec<Vec3>,
    key: [u8; 32],
    /// Local lights' candela bits (P-FLAT): a change continues history under MQ1's box.
    intensities: Vec<[u32; 3]>,
    /// Local lights' candela ranges since the last restart of every pixel (P-RELIT).
    swings: Vec<Swing>,
    skin_bounds: Option<crate::shadow_plan::Bounds>,
    skin_revisions: Vec<u64>,
}

/// The world-space union of an optional box and a box.
fn union(
    a: Option<crate::shadow_plan::Bounds>,
    b: crate::shadow_plan::Bounds,
) -> crate::shadow_plan::Bounds {
    a.map_or(b, |a| crate::shadow_plan::Bounds {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    })
}

/// What a frame's plan is computed from, apart from the planner's own history.
pub(crate) struct PlanInputs<'a> {
    pub(crate) settings: Reconstruction,
    pub(crate) budget: ReconstructionShadowBudget,
    /// The renderer frame this view rendered last: history continues only from it.
    pub(crate) previous_frame: u64,
    /// This frame's renderer frame (`FrameId`).
    pub(crate) frame: u64,
    /// The reconstruction sample index.
    pub(crate) index: u64,
    /// A digest of everything that restarts history when it changes
    /// (`reconstruction_key`).
    pub(crate) key: [u8; 32],
    /// The lights as given, and their shadow requests.
    pub(crate) lights: &'a [Light],
    pub(crate) requests: &'a [ShadowRequest],
    pub(crate) directional_disk_angle: f32,
    /// The unjittered camera matrix at the internal size, the eye and this frame's jitter.
    pub(crate) matrix: Mat4,
    pub(crate) eye: Vec3,
    pub(crate) jitter: [f32; 2],
    pub(crate) exposure: f32,
    /// The clear colour misses show, and whether they show the visible sky instead.
    pub(crate) background: [f32; 3],
    pub(crate) visible_sky: bool,
    /// One output pixel's angle.
    pub(crate) pixel_angle: f32,
    /// The draws' world bounds, the skinned ones', every skinned draw's revision in draw
    /// order, and whether any placement moved since the previous frame.
    pub(crate) scene_bounds: Option<crate::shadow_plan::Bounds>,
    pub(crate) skin_bounds: Option<crate::shadow_plan::Bounds>,
    pub(crate) skin_revisions: Vec<u64>,
    pub(crate) rigid_motion: bool,
    /// U-RTVIS: the world bounds of every placement that moved or is skinned, at both its
    /// previous and current transforms; no held stop inside their swept sun shadow.
    pub(crate) moving_bounds: Option<crate::shadow_plan::Bounds>,
    /// P-RTS: the penumbra pass is compiled and edges are deferred.
    pub(crate) listing: bool,
}

/// A frame's plan: the `Tr1Frame` record, and the primary frame record's sampling words
/// (bytes 112..128: the shadow ray budget, the probe count, the probe disk and its cone).
pub(crate) struct Planned {
    pub(crate) frame: Vec<u8>,
    pub(crate) sampling: Vec<u8>,
    /// U-RTVIS: the held budget's `RtvisHeld` record (HELD_BYTES).
    pub(crate) held: Vec<u8>,
    /// U-RTVIS: the reflection history's `RlHistoryFrame` record (RL_HISTORY_BYTES).
    pub(crate) rl_history: Vec<u8>,
}
/// U-RTVIS: `RlHistoryFrame` (shaders/reflections_history.wgsl): the previous unjittered
/// view-projection, then whether this frame continues history, the half it writes, the frame
/// counter and the exposure factor carrying stored radiance to this frame.
pub(crate) const RL_HISTORY_BYTES: usize = 80;
impl Planned {
    /// The frame's `Tr1Zones` record (`ZONES_BYTES`), which the sun-shadow passes also bind:
    /// `PrimaryFrame::zones` and `VsmFrame::zones`.
    pub(crate) fn zones(&self) -> &[u8] {
        &self.frame[ZONES_OFFSET..ZONES_OFFSET + ZONES_BYTES]
    }
}

/// The reconstruction's CPU history between frames and the per-frame plan made from it:
/// everything `prepare_reconstruction` decides without a device. The wgpu renderer and
/// the ara-gpu backend both plan through `prepare`.
pub(crate) struct Planner {
    /// The side this frame writes; the other holds the previous frame.
    pub parity: usize,
    previous: Option<Previous>,
    /// The next explicit restart (`cut_reconstruction`).
    pub cut: bool,
    /// The last prepared frame restarted every pixel.
    pub reset: bool,
    held_frames: u32,
    /// Sun-shadow directions every receiver evaluates this frame.
    pub shadow_rays: u32,
    /// P-RELIT: directions a receiver in a restart zone evaluates instead (`Tr1Zones`
    /// counts.y); 0 when no zone is up or the frame's count is already as large.
    pub restart_rays: u32,
    /// P-RTS: directions every receiver traces first; below `shadow_rays`, only listed
    /// penumbra pixels trace the full budget (docs/p-rts.md). Without listing (U-RTVIS,
    /// docs/rtvis.md), cold frames probe `active_rays` and receivers whose probes disagree
    /// trace the full budget.
    pub probe_rays: u32,
}
impl Planner {
    pub(crate) fn new() -> Self {
        Self {
            parity: 0,
            previous: None,
            cut: false,
            reset: true,
            held_frames: 0,
            shadow_rays: 8,
            restart_rays: 0,
            probe_rays: 8,
        }
    }
    /// Plans the next frame and advances the history.
    pub(crate) fn prepare(&mut self, i: PlanInputs<'_>) -> Planned {
        let settings = i.settings;
        let (key, matrix, eye, exposure) = (i.key, i.matrix, i.eye, i.exposure);
        let (background, visible_sky, pixel_angle) = (i.background, i.visible_sky, i.pixel_angle);
        let (scene_bounds, skin_bounds, rigid_motion) =
            (i.scene_bounds, i.skin_bounds, i.rigid_motion);
        let (skin_revisions, listing) = (i.skin_revisions, i.listing);
        let intensities = local_intensities(i.lights);
        let suns: Vec<Vec3> = i
            .lights
            .iter()
            .filter_map(|l| match l {
                crate::Light::Directional { travel, .. } => Some(travel.normalize()),
                _ => None,
            })
            .collect();
        // SK1: vertex motion reprojects the character, but its changing shadow also
        // changes otherwise-static receivers. Reject those receivers' lighting history
        // in a conservative world-space swept shadow volume, never a screen-space
        // lighting approximation. This leaves unrelated static pixels accumulating.
        let mut shadow_bounds = None;
        // The shadowed sun's direction toward it and disk half-angle, when `shadow_bounds` is
        // tested as its swept shadow rather than as a box (P-FLAT).
        let mut caster_light: Option<(Vec3, f32)> = None;
        // MQ2: some skinned pose changed since the previous frame.
        let mut posed = false;
        if let (Some(current), Some(previous)) = (skin_bounds, self.previous.as_ref())
            && previous.skin_revisions != skin_revisions
        {
            posed = true;
            let swept = union(previous.skin_bounds, current);
            let mut affected = swept;
            let distance = scene_bounds.map_or(1., |b: crate::shadow_plan::Bounds| {
                (b.max - b.min).length().max(1.)
            }) * 2.;
            let requested: Vec<&Light> = i
                .lights
                .iter()
                .zip(i.requests)
                .filter(|(_, r)| !matches!(r, ShadowRequest::None))
                .map(|(l, _)| l)
                .collect();
            // P-FLAT: with one shadowed sun and no other shadow request, receivers are tested
            // against the swept bounds' own shadow (`tr1_caster_shadow`): inside the bounds, or
            // a ray toward the sun within its disk meets them. The box below stretches the
            // bounds twice the scene's size along the sun; under a low sun it held most of the
            // scene's static receivers to one frame of history while a character walked.
            if let [Light::Directional { travel, .. }] = requested[..] {
                caster_light = Some((-travel.normalize(), i.directional_disk_angle));
            } else {
                for light in requested {
                    if let Light::Directional { travel, .. } = light {
                        affected = union(
                            Some(affected),
                            crate::shadow_plan::Bounds {
                                min: swept.min + travel.normalize() * distance,
                                max: swept.max + travel.normalize() * distance,
                            },
                        );
                    } else if let Some(scene) = scene_bounds {
                        affected = union(Some(affected), scene);
                    }
                }
            }
            // Sky occlusion and bounce the character changes outside its direct shadow are
            // held to this frame's 3x3 box (MQ2: skinned frames set TR1_BOX), as MQ1 holds
            // rigid motion's; SK1 rejected every receiver's history scene-wide while GI was
            // on, so an animated character restarted the whole image on every frame.
            let margin = Vec3::splat(1e-4) + affected.min.abs().max(affected.max.abs()) * 1e-5;
            affected.min -= margin;
            affected.max += margin;
            shadow_bounds = Some(affected);
        }
        let cold = self.previous.is_none() || self.cut;
        let continued = self
            .previous
            .as_ref()
            .filter(|p| p.frame == i.previous_frame && p.key == key && p.suns.len() == suns.len())
            .map(|p| {
                // The largest directional-light turn since the previous frame.
                let turn = p
                    .suns
                    .iter()
                    .zip(&suns)
                    // Chord angle: exactly zero for an unchanged direction, where acos of
                    // a rounded unit dot product is not.
                    .map(|(a, b)| 2.0 * (0.5 * (*a - *b).length()).min(1.0).asin())
                    .fold(0.0, f32::max);
                (p.matrix, p.eye, p.exposure, turn, p.jitter)
            });
        // A sun turn caps the window at turn_angle / turn; a turn past turn_angle (any turn
        // when it is zero) restarts every pixel.
        let turn = continued.map_or(0.0, |c| c.3);
        let event_window = if turn > 0.0 {
            settings.turn_angle / turn
        } else {
            settings.window
        };
        self.reset = continued.is_none() || self.cut || event_window < 1.0;
        self.cut = false;
        // P-RELIT: local lights whose intensity or colour changed never restart the frame.
        // History continues, held to this frame's 3x3 box as for moving placements, except
        // inside each such light's restart sphere, where its swing since the histories began
        // could leave `light_budget` of stale light: those pixels restart and follow the
        // light exactly (`relit_zones`).
        let previous = continued.and(self.previous.as_ref());
        let swings = widen_swings(
            previous
                .filter(|_| !self.reset)
                .map(|p| p.swings.as_slice()),
            &intensities,
        );
        let relit = !self.reset && previous.is_some_and(|p| p.intensities != intensities);
        let relit_spheres = match previous {
            Some(p) if relit => relit_zones(
                &p.intensities,
                &swings,
                i.lights,
                exposure,
                settings.light_budget,
            ),
            _ => vec![],
        };
        // MQ1's clamp event comes from the same submitted transforms that feed
        // TR1/PP1 on the GPU; no second transform history or frame-buffer payload.
        let moved = !self.reset && rigid_motion;
        let jitter = i.jitter;
        let (previous_matrix, previous_eye, scale, previous_jitter) = match continued {
            Some((m, e, x, _, j)) if !self.reset => (m, e, exposure / x, j),
            _ => (matrix, eye, 1.0, jitter),
        };
        self.parity = 1 - self.parity;
        // MV1 keeps the shared motion source; MQ1/MQ2 clamp rigid or skinned motion.
        let options = u32::from(settings.catmull_rom)
            | u32::from(settings.spatial_start) << 1
            | u32::from((moved || relit || (posed && !self.reset)) && settings.motion_box) << 3
            | u32::from(settings.upscale_interpolation) << 4;
        let mut bytes = Vec::with_capacity(FRAME_BYTES);
        bytes.extend_from_slice(&matrix_bytes(previous_matrix));
        bytes.extend_from_slice(&matrix_bytes(matrix));
        bytes.extend_from_slice(&matrix_bytes(previous_matrix.inverse()));
        let floats: [f32; 24] = [
            previous_eye.x,
            previous_eye.y,
            previous_eye.z,
            pixel_angle,
            eye.x,
            eye.y,
            eye.z,
            previous_jitter[0],
            jitter[0],
            jitter[1],
            scale,
            previous_jitter[1],
            background[0],
            background[1],
            background[2],
            // 1: misses show the constant clear colour; 0: the visible sky, which misses
            // reconstruct like a surface keyed by direction.
            if visible_sky { 0.0 } else { 1.0 },
            settings.window,
            event_window.max(1.0),
            settings.coverage_window,
            settings.blur_budget,
            settings.sharp_window,
            settings.start_window,
            settings.start_radius,
            settings.view_budget,
        ];
        for f in floats {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        // Camera stillness; MV1/SK1's surface source handles moving placements/vertices.
        let still = !self.reset
            && previous_matrix.to_cols_array().map(f32::to_bits)
                == matrix.to_cols_array().map(f32::to_bits);
        // MV1 motion-source integration: rigid casters must not enter RTDEF2's settled
        // shadow budget just because the camera is stationary. A deforming caster's swept
        // shadow does not hold the whole frame at the moving count (audit 3.3): receivers it
        // can reach trace the restart count per pixel (`restart_rays`, tr1_zone_rays) and
        // every other receiver settles.
        let sampling_still = still && turn == 0.0 && !rigid_motion;
        self.held_frames = if sampling_still {
            self.held_frames.saturating_add(1)
        } else {
            0
        };
        // P-RELIT: the restart budget is per pixel. A frame that restarts every pixel takes
        // it frame-wide. A frame that restarts only some (receivers in a moving caster's
        // shadow, or pixels in a changed light's restart sphere) keeps the moving or held
        // count, and the receivers in those zones trace `restart_rays` (tr1_zone_rays): they
        // keep the per-pixel ray count a full restart gave them. One frame-wide word put the
        // whole frame on 64 rays for every frame of a flickering fire, which cost the game
        // scene 24 ms a frame on the non-RT path (docs/p-relit.md).
        self.shadow_rays = i
            .budget
            .rays(cold, self.reset, sampling_still, self.held_frames);
        let zoned = shadow_bounds.is_some() || !relit_spheres.is_empty();
        self.restart_rays = if zoned && i.budget.reset_rays > self.shadow_rays {
            i.budget.reset_rays
        } else {
            0
        };
        // Listing pays only where the budget exceeds both probe sets.
        let listed = listing && PENUMBRA_LISTING;
        self.probe_rays = if listed && self.shadow_rays > 2 * PROBE_DISK {
            2 * PROBE_DISK
        } else if !listed && cold && cold_probes() && i.budget.active_rays < self.shadow_rays {
            // U-RTVIS: a cut or first frame traces the moving count everywhere, and the cold
            // budget only where those directions disagree (COLD_PROBE_STOP).
            i.budget.active_rays
        } else {
            self.shadow_rays
        };
        // Listing frames probe a cone PROBE_CONE times the sun's; frames without listing
        // keep the exact budget directions (x 1.0 is exact).
        let (disk, widen) = if listed && self.probe_rays < self.shadow_rays {
            (PROBE_DISK, PROBE_CONE)
        } else {
            (0, 1.0)
        };
        let mut words = self.shadow_rays.to_ne_bytes().to_vec();
        words.extend_from_slice(&self.probe_rays.to_ne_bytes());
        words.extend_from_slice(&disk.to_ne_bytes());
        words.extend_from_slice(&widen.to_bits().to_ne_bytes());
        for w in [
            u32::from(self.reset),
            options,
            u32::from(still),
            i.index as u32,
        ] {
            bytes.extend_from_slice(&w.to_ne_bytes());
        }
        // shadow_lo.w enables rejection; shadow_hi.w selects the swept-shadow test, whose
        // shadow_light is the unit direction toward the sun (no zero component, so the slab
        // test's reciprocals stay finite) and the tangent of its disk half-angle, widened.
        let toward = caster_light.map_or(Vec3::ZERO, |(l, _)| {
            l.map(|c| {
                if c.abs() < 1e-6 {
                    1e-6f32.copysign(c)
                } else {
                    c
                }
            })
        });
        let spread = caster_light.map_or(0., |(_, angle)| angle.tan() * 1.01 + 1e-4);
        for value in shadow_bounds.map_or([0.; 12], |b| {
            [
                b.min.x,
                b.min.y,
                b.min.z,
                1.,
                b.max.x,
                b.max.y,
                b.max.z,
                if caster_light.is_some() { 1. } else { 0. },
                toward.x,
                toward.y,
                toward.z,
                spread,
            ]
        }) {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        // Tr1Zones.relit: each restart sphere (zero rows unused), then their count.
        for row in 0..RELIT_SLOTS {
            for value in relit_spheres.get(row).copied().unwrap_or_default() {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        for count in [relit_spheres.len() as u32, self.restart_rays, 0, 0] {
            bytes.extend_from_slice(&count.to_ne_bytes());
        }
        debug_assert_eq!(bytes.len(), ZONES_OFFSET + ZONES_BYTES);
        // resample.x: P-FLICKER's Catmull-Rom blur budget.
        for value in [settings.sharp_blur_budget, 0.0, 0.0, 0.0] {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        debug_assert_eq!(bytes.len(), FRAME_BYTES);
        let held = self.held_record(
            (i.moving_bounds, i.directional_disk_angle),
            previous_matrix,
            turn,
            &suns,
            listed,
        );
        let mut rl_history = Vec::with_capacity(RL_HISTORY_BYTES);
        rl_history.extend_from_slice(&matrix_bytes(previous_matrix));
        let continues = continued.is_some() && !self.reset;
        for w in [u32::from(continues), (i.frame & 1) as u32, i.frame as u32, scale.to_bits()] {
            rl_history.extend_from_slice(&w.to_ne_bytes());
        }
        self.previous = Some(Previous {
            frame: i.frame,
            matrix,
            jitter,
            eye,
            exposure,
            suns,
            key,
            intensities,
            swings,
            skin_bounds,
            skin_revisions,
        });
        Planned {
            frame: bytes,
            sampling: words,
            held,
            rl_history,
        }
    }
    /// U-RTVIS: the `RtvisHeld` record. The stop is on for a frame that continues every
    /// pixel's history (no reset or cut), with the sun unturned, one shadowing sun, no P-RTS
    /// listing and a budget above HELD_PROBES.
    fn held_record(
        &self,
        (moving_bounds, disk_angle): (Option<crate::shadow_plan::Bounds>, f32),
        previous_matrix: Mat4,
        turn: f32,
        suns: &[Vec3],
        listed: bool,
    ) -> Vec<u8> {
        let enabled = !self.reset
            && turn == 0.0
            && !listed
            && suns.len() == 1
            && held_probes()
            && HELD_PROBES < self.shadow_rays;
        let mut bytes = Vec::with_capacity(HELD_BYTES);
        bytes.extend_from_slice(&matrix_bytes(previous_matrix));
        for w in [u32::from(enabled), HELD_PROBES, HELD_STREAK, 0] {
            bytes.extend_from_slice(&w.to_ne_bytes());
        }
        // The swept-shadow test of shaders/temporal_zones.wgsl over the moving bounds.
        let toward = suns.first().map_or(Vec3::ONE, |t| {
            (-*t).map(|c| {
                if c.abs() < 1e-6 {
                    1e-6f32.copysign(c)
                } else {
                    c
                }
            })
        });
        let spread = disk_angle.tan() * 1.01 + 1e-4;
        let values = moving_bounds.map_or([0.; 12], |b| {
            let margin = Vec3::splat(1e-4) + b.min.abs().max(b.max.abs()) * 1e-5;
            let (lo, hi) = (b.min - margin, b.max + margin);
            [
                lo.x, lo.y, lo.z, 1., hi.x, hi.y, hi.z, 1., toward.x, toward.y, toward.z, spread,
            ]
        });
        for v in values {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        debug_assert_eq!(bytes.len(), HELD_BYTES);
        bytes
    }
}

/// Per-pixel history at the HDR size, ping-ponged by frame: exposed mean and weight
/// (Rgba32Float) and anchors (Rg32Uint), plus the copy of the frame's shaded samples.
pub(in crate::renderer) struct ReconstructionState {
    colour: [Depth; 2],
    anchor: [Depth; 2],
    /// Silhouettes' second slot and occluder coverage (Rgba32Uint).
    layer: [Depth; 2],
    sample: Depth,
    /// Fixed frame words; shared placement history is bound with the geometry.
    frame: Buffer,
    /// Resolve groups per written side, keyed by the textures and frame buffers they bind.
    groups: Option<(GroupKey, [Owned<wgpu::BindGroup>; 2])>,
    /// The CPU history and per-frame plan (shared with the ara-gpu backend).
    pub plan: Planner,
    /// The last planned frame's `Tr1Zones` record.
    pub zones: [u8; ZONES_BYTES],
}
type GroupKey = (
    wgpu::Texture,
    wgpu::Texture,
    wgpu::Texture,
    wgpu::Buffer,
    wgpu::Buffer,
);

/// A sampled 2D target owned through the ledger (copyable in tests).
fn target(
    r: &Renderer,
    label: &str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> Depth {
    let texture = r.ledger.own(
        r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | usage
                | if cfg!(test) {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = r.ledger.own(
        texture.create_view(&Default::default()),
        ResourceKind::TextureView,
    );
    let bytes = u64::from(size.width)
        * u64::from(size.height)
        * u64::from(format.block_copy_size(None).unwrap());
    Depth {
        texture,
        view,
        _allocation: r.ledger.allocate(AllocationKind::Target, bytes),
    }
}

/// An upscaled frame's targets (`RenderScale`): the internal colour target every pass renders
/// into in place of the caller's, and the reconstruction's output HDR at the caller's size,
/// which present tone-maps into the caller's target.
pub(in crate::renderer) struct Upscale {
    color: Depth,
    // PP1 consumes only the output image, never TR1's history or coverage state.
    pub(in crate::renderer) hdr: Depth,
    pub present: Owned<wgpu::BindGroup>,
    /// This frame's caller target, which present writes.
    pub output: wgpu::Texture,
}

impl ReconstructionState {
    #[cfg(test)]
    pub(in crate::renderer) fn test_history(&self) -> &wgpu::Texture {
        &self.colour[self.plan.parity].texture
    }
    /// Histories at the output size; the copy of the frame's samples at the internal size
    /// (equal at native resolution).
    fn new(r: &Renderer, size: wgpu::Extent3d, output: wgpu::Extent3d) -> Self {
        use wgpu::{TextureFormat as F, TextureUsages as U};
        let history = |label, format| target(r, label, output, format, U::STORAGE_BINDING);
        let colour = || history("TR1 history colour", F::Rgba32Float);
        let anchor = || history("TR1 history anchors", F::Rg32Uint);
        let layer = || history("TR1 silhouette slots", F::Rgba32Uint);
        Self {
            colour: [colour(), colour()],
            anchor: [anchor(), anchor()],
            layer: [layer(), layer()],
            sample: target(r, "TR1 frame samples", size, F::Rgba16Float, U::COPY_DST),
            frame: Buffer::new(
                r,
                "TR1 frame state",
                FRAME_BUFFER_BYTES as u64,
                wgpu::BufferUsages::STORAGE,
            ),
            groups: None,
            plan: Planner::new(),
            zones: [0; ZONES_BYTES],
        }
    }
    fn size(&self) -> wgpu::Extent3d {
        self.sample.texture.size()
    }
    /// The colour history this frame wrote (exposed mean, sample weight; Rgba32Float).
    #[cfg(test)]
    pub(in crate::renderer) fn written_colour(&self) -> &wgpu::Texture {
        &self.colour[self.plan.parity].texture
    }
    fn output(&self) -> wgpu::Extent3d {
        self.colour[0].texture.size()
    }
    /// The anchors and silhouette slots this frame's resolve writes, for the ID pass
    /// (src/id_output.rs), which runs after it and reads which surface settled.
    pub(in crate::renderer) fn written_slots(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        let side = self.plan.parity;
        (&self.anchor[side].view, &self.layer[side].view)
    }
}

impl Renderer {
    /// Changes reconstructed Interactive shadow sampling without changing user programs
    /// or compiling pipelines. A policy change starts a fresh reconstruction history.
    pub fn set_reconstruction_shadow_budget(
        &mut self,
        budget: ReconstructionShadowBudget,
    ) -> Result<(), Error> {
        budget.validate()?;
        if budget != self.reconstruction_shadow_budget {
            for view in self.views_mut() {
                view.reconstruction_state = None;
                view.reconstruction_index = 0;
            }
        }
        self.reconstruction_shadow_budget = budget;
        Ok(())
    }
    pub fn reconstruction_shadow_budget(&self) -> ReconstructionShadowBudget {
        self.reconstruction_shadow_budget
    }
    /// Last prepared reconstructed frame's Interactive ray count, for telemetry.
    pub fn reconstruction_shadow_rays(&self) -> Option<u32> {
        if self.primary_mode != PrimaryVisibility::Traced
            || self.view.soft_mode != SoftShadows::Interactive
            || self.view.shadows.settings.directional_disk_angle == 0.0
            || !self.view.reconstruction.enabled
        {
            return None;
        }
        self.view
            .reconstruction_state
            .as_ref()
            .map(|s| s.plan.shadow_rays)
    }
    /// This frame's internal colour target when an upscaling preset applies to it (a
    /// reconstructed frame below native scale), allocated for the caller's `color`; None
    /// renders at the caller's size and releases the upscaling targets.
    pub(in crate::renderer) fn upscale_target(
        &mut self,
        color: &wgpu::Texture,
    ) -> Option<wgpu::Texture> {
        let [width, height] = self
            .view
            .reconstruction
            .scale
            .internal_size(color.width(), color.height());
        if self.view.reconstruction_frame.is_none()
            || [width, height] == [color.width(), color.height()]
        {
            self.view.upscale = None;
            return None;
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        if self
            .view
            .upscale
            .as_ref()
            .is_none_or(|u| u.color.texture.size() != size || u.hdr.texture.size() != color.size())
        {
            use wgpu::{TextureFormat as F, TextureUsages as U};
            let internal = target(
                self,
                "upscaled frame colour",
                size,
                self.format,
                U::RENDER_ATTACHMENT | U::COPY_SRC,
            );
            let hdr = target(
                self,
                "upscaled reconstruction HDR",
                color.size(),
                F::Rgba16Float,
                U::STORAGE_BINDING | U::COPY_SRC,
            );
            let present = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("upscaled HDR presentation"),
                    layout: &self.lighting.present_bindings,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&hdr.view),
                    }],
                }),
                ResourceKind::BindGroup,
            );
            self.view.upscale = Some(Upscale {
                color: internal,
                hdr,
                present,
                output: color.clone(),
            });
        }
        let upscale = self.view.upscale.as_mut().unwrap();
        upscale.output = color.clone();
        Some((*upscale.color.texture).clone())
    }

    /// Identity of every input a reconstructed frame depends on besides the camera, the
    /// eye, exposure, directional-light directions, local-light intensities (P-RELIT), skinned
    /// poses (SK1) and placement transforms (MQ1), which TR1 follows: a change restarts every
    /// pixel. Programs, prelude, geometry, bindings, materials, textures, new placements, the
    /// sun's colour, light kinds and local lights' position, direction, range, cone and source
    /// radius, the environment and every shadow and visibility setting. Before MQ1 transform
    /// edits of existing placements restarted every pixel, so a scene with any animated rigid
    /// object never accumulated; they now reproject through their previous transforms. Before
    /// P-RELIT a flickering local light restarted every pixel every frame in the same way;
    /// it now restarts only its restart sphere (`relit_zones`).
    fn reconstruction_key(&self) -> [u8; 32] {
        // The scene's uploads and this view's own, not other views' primary tables.
        let u = &self.uploads;
        let (others, mine) = (&self.primary_uploads, &self.view.primary_uploads);
        let own = |all: u64, others: u64, mine: u64| all - others + mine;
        let state = self.view.primary.as_ref().unwrap();
        let lights: Vec<_> = self
            .lighting
            .source_lights
            .iter()
            .map(|light| match *light {
                crate::Light::Directional { lux, .. } => format!("directional {lux:?}"),
                // Candela (intensity and colour) is in `local_intensities`, not the key.
                local => format!("{:?}", without_candela(local)),
            })
            .collect();
        let mut hash = Sha256::new()
            .chain_update(self.revision.0.to_ne_bytes())
            .chain_update(state.program.to_ne_bytes())
            // MP1: a mixed frame's other programs (none for single-program frames).
            .chain_update(
                state
                    .programs
                    .iter()
                    .skip(1)
                    .flat_map(|p| p.to_ne_bytes())
                    .collect::<Vec<_>>(),
            )
            .chain_update(state.geometry_generation.to_ne_bytes())
            .chain_update(state.reconstruction_bindings_generation.to_ne_bytes())
            .chain_update(format!("{:?}", self.view.shadows.settings).as_bytes())
            .chain_update(format!("{:?}", self.view.virtual_settings).as_bytes())
            .chain_update(format!("{:?} {:?}", self.view.soft_mode, self.primary_mode).as_bytes())
            .chain_update(format!("{lights:?}").as_bytes())
            .chain_update(
                self.lighting
                    .pending
                    .environment
                    .map(f32::to_bits)
                    .map(u32::to_ne_bytes)
                    .concat(),
            )
            .chain_update(self.environment.uniform())
            .chain_update([u8::from(self.environment.visible)])
            // AT1: the atmosphere sky on misses (atmosphere_gpu.rs).
            .chain_update([u8::from(self.atmosphere_sky_visible())]);
        // Not `models` or `normals` (MQ1): transform edits of existing placements keep the
        // primary tables, so placement slots are stable; new placements change `instances`
        // and `geometry_generation`.
        for v in [
            own(u.environment, others.environment, mine.environment),
            own(u.geometry, others.geometry, mine.geometry),
            own(u.lut, others.lut, mine.lut),
            own(u.textures, others.textures, mine.textures),
            own(u.materials, others.materials, mine.materials),
            // MV1: transform/derived-normal uploads move surfaces; they no longer
            // reset all history. Placement identity/topology stays in the key above.
            own(u.instances, others.instances, mine.instances),
        ] {
            hash.update(v.to_ne_bytes());
        }
        hash.finalize().into()
    }

    /// Compiles the reconstruction's pipelines for the installed prelude on its first frame
    /// (and again after a prelude reload), so other frames never pay for them.
    pub(in crate::renderer) fn ensure_reconstruction(&mut self) -> Result<(), Error> {
        // Traced primaries on an RT renderer also need the rotated Interactive kernels,
        // which a visibility-buffer frame compiled before did not build.
        let traced =
            self.hardware_shadows.is_some() && self.primary_mode == PrimaryVisibility::Traced;
        // U-FIXED: visibility-buffer RT frames trace their sun with `primary_vb_sun`.
        let vb = self.hardware_shadows.is_some()
            && self.primary_mode == PrimaryVisibility::VisibilityBuffer;
        if self
            .pipeline
            .primary
            .as_ref()
            .unwrap()
            .reconstruction
            .as_ref()
            .is_some_and(|p| (p.trace.is_some() || !traced) && (p.vb_sun.is_some() || !vb))
        {
            return Ok(());
        }
        let resolve = resolve_source(&self.source)?;
        let soft = &self.primary_sources.as_ref().unwrap().0[1];
        let trace = if traced {
            Some(trace_source_held(soft)?)
        } else {
            None
        };
        let vb_sun = if vb {
            Some(vb_sun_source(soft)?)
        } else {
            None
        };
        let scopes = OperationScopes::new(&self.device);
        let production = self.pipeline.primary.as_ref().unwrap();
        let pipelines = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .reconstruction_pipelines(
                &self.device,
                &self.ledger,
                &resolve,
                trace.as_deref().map(|s| (s, production)),
                vb_sun.as_deref(),
            );
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => self.fail(error.to_string()),
            });
        }
        self.pipelines.created += 1 + u64::from(pipelines.vb_sun.is_some()) + pipelines.trace.as_ref().map_or(0, |t| {
            1 + u64::from(t.repair.is_some())
                + u64::from(t.retry.is_some())
                + u64::from(t.full.is_some())
                + 2 * u64::from(t.penumbra.is_some())
        });
        self.pipeline.primary.as_mut().unwrap().reconstruction = Some(pipelines);
        Ok(())
    }

    /// Static-image control against an immutable pre-merge MQ1 resolve shader. Keep
    /// traced-primary kernels installed; only the resolve is exchanged by the GPU test.
    #[cfg(test)]
    pub(in crate::renderer) fn test_resolve_source(&mut self, mq1: Option<&str>) {
        let source = match mq1 {
            Some(source) => format!("{}\n{source}", non_rt_heads(&self.source).unwrap()[0]),
            None => resolve_source(&self.source).unwrap(),
        };
        let scopes = OperationScopes::new(&self.device);
        let pipelines = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .reconstruction_pipelines(&self.device, &self.ledger, &source, None, None);
        assert!(
            scopes.finish().is_none(),
            "static-control resolve must compile"
        );
        self.pipeline
            .primary
            .as_mut()
            .unwrap()
            .reconstruction
            .as_mut()
            .unwrap()
            .resolve = pipelines.resolve;
    }

    /// Publishes one reconstructed frame after `prepare_primary` (and the visibility
    /// buffer's): allocates history at the HDR size, decides restarts and window caps from
    /// engine state, writes the frame words and binds the frame's targets. `camera` is the
    /// frame's jittered camera; `index` its sample.
    pub(in crate::renderer) fn prepare_reconstruction(
        &mut self,
        camera: &Camera,
        index: u64,
        draws: &[PreparedDraw<'_>],
    ) {
        let size = self.view.hdr.as_ref().unwrap().resolved.texture.size();
        let output = self
            .view
            .upscale
            .as_ref()
            .map_or(size, |u| u.hdr.texture.size());
        if self
            .view
            .reconstruction_state
            .as_ref()
            .is_none_or(|s| s.size() != size || s.output() != output)
        {
            let cut = self
                .view
                .reconstruction_state
                .as_ref()
                .is_some_and(|s| s.plan.cut);
            let mut next = ReconstructionState::new(self, size, output);
            next.plan.cut = cut;
            self.view.reconstruction_state = Some(next);
        }
        let key = self.reconstruction_key();
        let matrix = camera.unjittered().matrix(size.width, size.height);
        let eye = camera.view().inverse().w_axis.truncate();
        let exposure = self.lighting.pending.exposure;
        let background = self.lit_clear().map(|v| v as f32);
        // AT1: misses also vary per pixel while the atmosphere's sky is on.
        let visible_sky = (self.environment.visible && self.environment.selected.is_some())
            || self.atmosphere_sky_visible();
        let pixel_angle = 2.0 * (0.5 * camera.fov_y()).tan() / output.height as f32;
        let mut scene_bounds = None;
        let mut skin_bounds = None;
        let mut skin_revisions = vec![];
        let mut rigid_motion = false;
        let mut moving_bounds = None;
        for d in draws {
            if let Some(s) = &d.mesh.skin {
                skin_revisions.push(s.revision);
            }
            for (&model, &previous_model) in d.instances.state.models.iter().zip(
                d.instances
                    .state
                    .previous_models(self.view_key.0, self.view.last_frame),
            ) {
                let moved = model.to_cols_array().map(f32::to_bits)
                    != previous_model.to_cols_array().map(f32::to_bits);
                rigid_motion |= moved;
                // U-RTVIS: no held stop in a moving or skinned placement's swept shadow.
                if moved || d.mesh.skin.is_some() {
                    for m in [model, previous_model] {
                        if let Ok(b) = shadow_gpu::world_bounds(d.mesh.bounds, m) {
                            moving_bounds = Some(union(moving_bounds, b));
                        }
                    }
                }
                if let Ok(b) = shadow_gpu::world_bounds(d.mesh.bounds, model) {
                    scene_bounds = Some(union(scene_bounds, b));
                    if d.mesh.skin.is_some() {
                        skin_bounds = Some(union(skin_bounds, b));
                    }
                }
            }
        }
        // P-RTS: with the penumbra pass compiled and edges deferred, every receiver first
        // traces the held count; only listed pixels trace the rest of the budget.
        let primary = self.view.primary.as_ref().unwrap();
        let listing = primary.targets.as_ref().is_some_and(|t| t.defer_edges)
            && self.pipeline.primary.as_ref().is_some_and(|p| {
                p.reconstruction
                    .as_ref()
                    .and_then(|r| r.trace.as_ref())
                    .is_some_and(|t| t.penumbra.is_some())
            });
        let planned = self
            .view
            .reconstruction_state
            .as_mut()
            .unwrap()
            .plan
            .prepare(PlanInputs {
                settings: self.view.reconstruction,
                budget: self.reconstruction_shadow_budget,
                previous_frame: self.view.last_frame,
                frame: self.frames + 1,
                index,
                key,
                lights: &self.lighting.source_lights,
                requests: &self.view.shadows.settings.requests,
                directional_disk_angle: self.view.shadows.settings.directional_disk_angle,
                matrix,
                eye,
                jitter: camera.jitter(),
                exposure,
                background,
                visible_sky,
                pixel_angle,
                scene_bounds,
                skin_bounds,
                skin_revisions,
                rigid_motion,
                moving_bounds,
                listing,
            });
        // PrimaryFrame.sampling is private engine state; no material/shading ABI changes.
        let primary_frame = &primary.frame.as_ref().unwrap().0.value;
        self.queue
            .write_buffer(primary_frame, 112, &planned.sampling);
        self.queue
            .write_buffer(primary_frame, super::FRAME_ZONES as u64, planned.zones());
        self.uploads.view_settings += 16 + ZONES_BYTES as u64;
        self.uploads.write_calls += 2;
        // The non-RT sun lookup reads the same zones (prepare_virtual_sampling).
        let zones: [u8; ZONES_BYTES] = planned.zones().try_into().unwrap();
        self.view.reconstruction_state.as_mut().unwrap().zones = zones;
        let state = self.view.reconstruction_state.as_ref().unwrap();
        self.queue
            .write_buffer(&state.frame.value, 0, &planned.frame);
        self.uploads.view_settings += FRAME_BYTES as u64;
        self.uploads.write_calls += 1;
        if let Some(history) = primary.targets.as_ref().map(|t| &t.rl_history) {
            self.queue.write_buffer(&history.frame.value, 0, &planned.rl_history);
            self.uploads.view_settings += RL_HISTORY_BYTES as u64;
            self.uploads.write_calls += 1;
        }
        if let Some(held) = primary.targets.as_ref().and_then(|t| t.held.as_ref()) {
            self.queue.write_buffer(&held.frame.value, 0, &planned.held);
            self.uploads.view_settings += HELD_BYTES as u64;
            self.uploads.write_calls += 1;
        }
        // The resolve writes the frame's HDR target, or the upscaled output.
        let hdr = &self.view.hdr.as_ref().unwrap().resolved;
        let out = self.view.upscale.as_ref().map_or(hdr, |u| &u.hdr);
        let primary = self.view.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let frame = &primary.frame.as_ref().unwrap().0.value;
        let state = self.view.reconstruction_state.as_ref().unwrap();
        let group_key: GroupKey = (
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            (*out.texture).clone(),
            (**frame).clone(),
            (*state.frame.value).clone(),
        );
        if state.groups.as_ref().is_none_or(|(k, _)| *k != group_key) {
            let groups = self.reconstruction_groups();
            self.view.reconstruction_state.as_mut().unwrap().groups = Some((group_key, groups));
        }
    }

    fn reconstruction_groups(&self) -> [Owned<wgpu::BindGroup>; 2] {
        use wgpu::BindingResource as R;
        let state = self.view.reconstruction_state.as_ref().unwrap();
        let primary = self.view.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let frame = &primary.frame.as_ref().unwrap().0.value;
        let hdr = &self.view.hdr.as_ref().unwrap().resolved;
        let out = self.view.upscale.as_ref().map_or(hdr, |u| &u.hdr);
        [0, 1].map(|written| {
            let read = 1 - written;
            let entries = [
                (7, R::TextureView(&targets.hit.view)),
                (10, frame.as_entire_binding()),
                (21, R::TextureView(&targets.tiles.view)),
                (40, state.frame.value.as_entire_binding()),
                (41, R::TextureView(&state.sample.view)),
                (42, R::TextureView(&state.colour[read].view)),
                (43, R::TextureView(&state.anchor[read].view)),
                (44, R::TextureView(&state.colour[written].view)),
                (45, R::TextureView(&state.anchor[written].view)),
                (47, R::TextureView(&state.layer[read].view)),
                (48, R::TextureView(&state.layer[written].view)),
                (46, R::TextureView(&out.view)),
            ]
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
            self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("temporal reconstruction"),
                    layout: &primary.reconstruction.0,
                    entries: &entries,
                }),
                ResourceKind::BindGroup,
            )
        })
    }

    /// The reconstruction pass after the frame's shading: copies the shaded samples, then
    /// resolves every target pixel into the HDR target (the upscaled output when a preset
    /// applies) and the written history side.
    pub(in crate::renderer) fn encode_reconstruction(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.view.reconstruction_state.as_ref().unwrap();
        let hdr = &self.view.hdr.as_ref().unwrap().resolved;
        encoder.copy_texture_to_texture(
            hdr.texture.as_image_copy(),
            state.sample.texture.as_image_copy(),
            state.size(),
        );
        let size = state.output();
        let primary = self.view.primary.as_ref().unwrap();
        let pipelines = self.pipeline.primary.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("temporal reconstruction"),
            timestamp_writes,
        });
        pass.set_pipeline(&pipelines.reconstruction.as_ref().unwrap().resolve);
        pass.set_bind_group(
            0,
            &*state.groups.as_ref().unwrap().1[state.plan.parity],
            &[],
        );
        pass.set_bind_group(1, primary.virtual_geometry().unwrap().1, &[]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P-FLAT: a flicker (candela only) keeps a local light's history identity and is the
    /// event `local_intensities` reports; moving or reshaping the light changes the identity.
    #[test]
    fn local_light_identity_ignores_only_candela() {
        let spot =
            |y: f32, candela: [f32; 3], range: f32, outer: f32, source_radius: f32| Light::Spot {
                cast_shadows: None,
                position: Vec3::new(3.0, y, 3.0),
                travel: Vec3::NEG_Y,
                candela,
                range: Some(range),
                inner: 0.3,
                outer,
                source_radius,
            };
        let bright = [26_000.0, 25_000.0, 23_000.0];
        let lamp = spot(3.0, bright, 12.0, 0.6, 0.1);
        let dimmer = spot(3.0, [13_000.0, 9_000.0, 4_000.0], 12.0, 0.6, 0.1);
        let sun = Light::Directional {
            travel: Vec3::NEG_Y,
            lux: [1.0; 3],
        };
        let identity = |l: Light| format!("{:?}", without_candela(l));
        assert_eq!(identity(lamp), identity(dimmer));
        assert_ne!(
            local_intensities(&[sun, lamp]),
            local_intensities(&[sun, dimmer])
        );
        assert_eq!(local_intensities(&[sun, lamp]).len(), 1);
        for moved in [
            spot(2.9, bright, 12.0, 0.6, 0.1),
            spot(3.0, bright, 11.0, 0.6, 0.1),
            spot(3.0, bright, 12.0, 0.5, 0.1),
            spot(3.0, bright, 12.0, 0.6, 0.0),
        ] {
            assert_ne!(identity(lamp), identity(moved), "{moved:?}");
        }
        assert_eq!(identity(sun), format!("{sun:?}"));
    }

    /// A moving camera's plan inputs over `lights`, frame `frame` of an unchanged scene.
    fn inputs(lights: &[Light], frame: u64) -> PlanInputs<'_> {
        PlanInputs {
            settings: Reconstruction::default(),
            budget: ReconstructionShadowBudget::default(),
            previous_frame: frame,
            frame: frame + 1,
            index: frame,
            key: [7; 32],
            lights,
            requests: &[],
            directional_disk_angle: 0.0,
            matrix: Mat4::from_translation(Vec3::new(frame as f32, 0.0, 0.0)),
            eye: Vec3::new(frame as f32, 1.0, 0.0),
            jitter: [0.0; 2],
            exposure: 2f32.powi(-11),
            background: [0.0; 3],
            visible_sky: false,
            pixel_angle: 1e-3,
            scene_bounds: None,
            skin_bounds: None,
            skin_revisions: vec![],
            rigid_motion: false,
            moving_bounds: None,
            listing: false,
        }
    }
    /// The courtyard's fire at `scale` of its peak.
    fn fire(scale: f32) -> Light {
        Light::Point {
            position: Vec3::new(-3.8, 0.95, 2.0),
            candela: [24_000.0 * scale, 12_000.0 * scale, 4_000.0 * scale],
            range: Some(10.0),
            source_radius: 0.25,
            cast_shadows: None,
        }
    }

    /// The restart spheres in a planned frame's `Tr1Zones`: (centre, radius).
    fn spheres(planned: &Planned) -> Vec<(Vec3, f32)> {
        let zones = &planned.frame[ZONES_OFFSET..ZONES_OFFSET + ZONES_BYTES];
        let word = |at: usize| u32::from_ne_bytes(zones[at..at + 4].try_into().unwrap());
        let float = |at: usize| f32::from_bits(word(at));
        (0..word(48 + RELIT_SLOTS * 16) as usize)
            .map(|k| 48 + k * 16)
            .map(|at| {
                (
                    Vec3::new(float(at), float(at + 4), float(at + 8)),
                    float(at + 12).sqrt(),
                )
            })
            .collect()
    }

    /// P-RELIT: a flickering local light never restarts the frame nor selects the restart
    /// shadow budget; it restarts a sphere sized by its swing since the histories began, not
    /// by its change since the last frame. A cut and a key change still restart every pixel.
    #[test]
    fn light_intensity_restarts_a_sphere_not_the_frame() {
        let budget = ReconstructionShadowBudget::default();
        let mut plan = Planner::new();
        let cold = plan.prepare(inputs(&[fire(1.0)], 0));
        assert!(plan.reset && spheres(&cold).is_empty());
        assert_eq!(plan.shadow_rays, budget.cold_rays);
        let mut radii = vec![];
        // 1.0 -> 0.7 -> 0.9 -> 0.91: the last step is one percent, but the oldest history may
        // still hold 1.0 and 0.7, so its sphere is as wide as the 0.9 frame's.
        for (frame, scale) in [(1, 0.7), (2, 0.9), (3, 0.91)] {
            let planned = plan.prepare(inputs(&[fire(scale)], frame));
            assert!(!plan.reset, "frame {frame}");
            // The frame keeps the moving count; receivers in the sphere trace the restart count.
            assert_eq!(plan.shadow_rays, budget.active_rays, "frame {frame}");
            assert_eq!(plan.restart_rays, budget.reset_rays, "frame {frame}");
            assert_eq!(
                planned.zones()[52 + RELIT_SLOTS * 16..][..4],
                budget.reset_rays.to_ne_bytes()
            );
            let [(centre, radius)] = spheres(&planned)[..] else {
                panic!("one sphere, frame {frame}")
            };
            assert_eq!(centre, Vec3::new(-3.8, 0.95, 2.0));
            radii.push(radius);
        }
        // The courtyard: 0.3 x 24,000 cd at 2^-11 leaves 0.05 exposed lux at 6.7 m.
        assert!((radii[0] - 6.7).abs() < 0.05, "{radii:?}");
        assert!(radii[1] > 5.5 && radii[1] < radii[0], "{radii:?}");
        assert!(
            radii[2] > radii[1] && radii[2] - radii[1] < 0.15,
            "{radii:?}"
        );
        // An unchanged light restarts nothing; its box and spheres are off.
        let steady = plan.prepare(inputs(&[fire(0.91)], 4));
        assert!(spheres(&steady).is_empty());
        assert_eq!(plan.restart_rays, 0);
        let options = u32::from_ne_bytes(steady.frame[292..296].try_into().unwrap());
        assert_eq!(options & 8, 0);
        // A key change restarts every pixel and the swing: the next flicker starts small.
        plan.prepare(PlanInputs {
            key: [8; 32],
            ..inputs(&[fire(0.91)], 5)
        });
        assert!(plan.reset);
        assert_eq!(
            (plan.shadow_rays, plan.restart_rays),
            (budget.reset_rays, 0)
        );
        let after = plan.prepare(PlanInputs {
            key: [8; 32],
            ..inputs(&[fire(0.9)], 6)
        });
        assert!(spheres(&after)[0].1 < 1.6, "{:?}", spheres(&after));
        plan.cut = true;
        let cut = plan.prepare(PlanInputs {
            key: [8; 32],
            ..inputs(&[fire(0.9)], 7)
        });
        assert_eq!(plan.shadow_rays, budget.cold_rays);
        // U-RTVIS: the cut probes the moving count first (sampling.y), with no P-RTS cone.
        assert_eq!(plan.probe_rays, budget.active_rays);
        let word =
            |k: usize| u32::from_ne_bytes(cut.sampling[4 * k..4 * k + 4].try_into().unwrap());
        assert_eq!(
            [word(0), word(1), word(2), word(3)],
            [budget.cold_rays, budget.active_rays, 0, 1f32.to_bits()]
        );
        plan.prepare(PlanInputs {
            key: [8; 32],
            ..inputs(&[fire(0.9)], 8)
        });
        assert_eq!(plan.probe_rays, plan.shadow_rays);
    }

    /// U-RTVIS: the held stop is on only on frames that continue every pixel's history under
    /// an unturned sun, and carries the previous matrix and the moving placements' bounds.
    #[test]
    fn held_record_follows_history_and_sun() {
        let sun = |x: f32| Light::Directional {
            travel: Vec3::new(x, -1.0, 0.3),
            lux: [1.0; 3],
        };
        let enabled = |p: &Planned| u32::from_ne_bytes(p.held[64..68].try_into().unwrap());
        let mut plan = Planner::new();
        let first = plan.prepare(inputs(&[sun(0.2)], 0));
        assert_eq!((enabled(&first), first.held.len()), (0, HELD_BYTES));
        let moving = plan.prepare(PlanInputs {
            moving_bounds: Some(crate::shadow_plan::Bounds {
                min: Vec3::splat(-1.0),
                max: Vec3::splat(1.0),
            }),
            ..inputs(&[sun(0.2)], 1)
        });
        assert_eq!(enabled(&moving), 1);
        // The previous frame's matrix, then probes and streak, then the bounds' flag.
        assert_eq!(
            moving.held[..64],
            matrix_bytes(Mat4::from_translation(Vec3::ZERO))
        );
        let word = |p: &Planned, k: usize| {
            u32::from_ne_bytes(p.held[64 + 4 * k..68 + 4 * k].try_into().unwrap())
        };
        assert_eq!(
            [word(&moving, 1), word(&moving, 2)],
            [HELD_PROBES, HELD_STREAK]
        );
        assert_eq!(
            f32::from_ne_bytes(moving.held[92..96].try_into().unwrap()),
            1.0
        );
        let turned = plan.prepare(inputs(&[sun(0.25)], 2));
        assert_eq!(enabled(&turned), 0);
        plan.cut = true;
        let cut = plan.prepare(inputs(&[sun(0.25)], 3));
        assert_eq!(enabled(&cut), 0);
    }

    /// P-RELIT: a light switched off restarts everything it lit above the budget, and a zero
    /// budget its whole range; a light too dim to show at this exposure restarts nothing.
    #[test]
    fn restart_spheres_follow_the_swing_and_the_exposure() {
        let radius = |from: f32, to: f32, budget: f32, exposure: f32| {
            let mut plan = Planner::new();
            let settings = Reconstruction {
                light_budget: budget,
                ..Default::default()
            };
            let (before, after) = ([fire(from)], [fire(to)]);
            plan.prepare(PlanInputs {
                settings,
                exposure,
                ..inputs(&before, 0)
            });
            let planned = plan.prepare(PlanInputs {
                settings,
                exposure,
                ..inputs(&after, 1)
            });
            assert!(!plan.reset);
            // History continues under MQ1's box on a relit frame.
            let options = u32::from_ne_bytes(planned.frame[292..296].try_into().unwrap());
            assert_eq!(options & 8, 8);
            spheres(&planned).first().map_or(0.0, |s| s.1)
        };
        let dusk = 2f32.powi(-11);
        let off = radius(1.0, 0.0, LIGHT_BUDGET, dusk);
        assert!(off > 8.0 && off < 10.0, "{off}");
        assert_eq!(radius(1.0, 0.0, 0.0, dusk), 10.0);
        assert!(radius(1.0, 0.99, LIGHT_BUDGET, dusk) < 1.6);
        // At a noon exposure the same flicker reaches less than half as far.
        let noon = radius(1.0, 0.7, LIGHT_BUDGET, 2f32.powi(-14));
        assert!(noon > 2.7 && noon < 3.0, "{noon}");
        assert_eq!(radius(1e-9, 0.0, LIGHT_BUDGET, dusk), 0.0);
        // The radius is where the shading's attenuation crosses the level, rounded up.
        for (range, level) in [(Some(10.0), 2.3), (Some(4.0), 900.0), (None, 2.3)] {
            let r = restart_radius(range, level);
            let attenuation = |d: f32| {
                let window = range.map_or(1.0, |range: f32| 1.0 - (d / range).powi(4));
                window * window / (d * d)
            };
            assert!(level * attenuation(r * 1.001) < 1.0, "{range:?} {level}");
            assert!(level * attenuation(r * 0.999) >= 1.0, "{range:?} {level}");
        }
    }

    /// P-RELIT: more changed lights than the frame has rows keep the largest spheres and
    /// bound the rest by one.
    #[test]
    fn many_relit_lights_share_a_bounding_sphere() {
        let torch = |k: usize, scale: f32| Light::Point {
            position: Vec3::new(3.0 * k as f32, 2.0, 0.0),
            candela: [(2_000.0 + 500.0 * k as f32) * scale; 3],
            range: Some(8.0),
            source_radius: 0.05,
            cast_shadows: None,
        };
        let hall = |scale: f32| (0..12).map(|k| torch(k, scale)).collect::<Vec<_>>();
        let mut plan = Planner::new();
        let (bright, dim) = (hall(1.0), hall(0.6));
        plan.prepare(inputs(&bright, 0));
        let planned = plan.prepare(inputs(&dim, 1));
        assert!(!plan.reset);
        let zones = spheres(&planned);
        assert_eq!(zones.len(), RELIT_SLOTS);
        // Every light's own sphere lies inside some uploaded one.
        let previous = local_intensities(&bright);
        let swings = widen_swings(
            Some(&widen_swings(None, &previous)),
            &local_intensities(&dim),
        );
        for (k, light) in dim.iter().enumerate() {
            let own = relit_zones(
                &previous[k..=k],
                &swings[k..=k],
                std::slice::from_ref(light),
                2f32.powi(-11),
                LIGHT_BUDGET,
            )[0];
            let (centre, radius) = (Vec3::new(own[0], own[1], own[2]), own[3].sqrt());
            assert!(
                zones
                    .iter()
                    .any(|&(c, r)| (c - centre).length() + radius <= r * 1.0001),
                "light {k}"
            );
        }
    }

    #[test]
    fn samples_are_balanced_lattice_pairs() {
        assert_eq!(sample_offset(0), [0.0; 2]);
        assert_eq!(sample_offset(255), [0.0; 2]);
        let mut seen = std::collections::HashSet::new();
        let mut sum = [0.0f64; 2];
        for k in 0..512 {
            let o = sample_offset(k);
            assert!(o.iter().all(|v| v.abs() < 0.5), "k={k}");
            assert_eq!(o, sample_offset(k + 256));
            if k % 2 == 1 && k < 255 {
                assert_eq!(sample_offset(k + 1), o.map(|v| -v), "k={k}");
            }
            if (1..255).contains(&k) {
                assert!(seen.insert(o.map(f32::to_bits)), "repeated offset at k={k}");
            }
            sum = [sum[0] + f64::from(o[0]), sum[1] + f64::from(o[1])];
            // Every odd-length prefix of the cycle is balanced.
            if k < 256 && k % 2 == 0 {
                assert!(sum.iter().all(|s| s.abs() < 1e-9), "k={k}: {sum:?}");
            }
        }
        assert_eq!(rotation(0), 0);
        assert_eq!(rotation(33), 8);
    }

    #[test]
    fn jittered_projection_moves_each_pixel_sample_by_the_offset() {
        let camera = Camera::look_at(
            Vec3::new(0.3, 1.0, 4.0),
            Vec3::ZERO,
            Vec3::Y,
            0.8,
            0.05,
            200.0,
        )
        .unwrap();
        let [w, h] = [64u32, 36];
        assert_eq!(
            camera
                .jittered([0.0; 2])
                .matrix(w, h)
                .to_cols_array()
                .map(f32::to_bits),
            camera.matrix(w, h).to_cols_array().map(f32::to_bits)
        );
        let offset = [0.25, -0.375];
        let jittered = camera.jittered(offset);
        assert_eq!(jittered.jitter(), offset);
        assert_eq!(jittered.unjittered().jitter(), [0.0; 2]);
        let pixel = |m: Mat4, p: Vec3| {
            let c = m * p.extend(1.0);
            [
                (c.x / c.w + 1.0) * 0.5 * w as f32,
                (1.0 - c.y / c.w) * 0.5 * h as f32,
            ]
        };
        for p in [
            Vec3::ZERO,
            Vec3::new(0.4, -0.2, 0.7),
            Vec3::new(-1.0, 0.5, -2.0),
        ] {
            let [a, b] = [
                pixel(camera.matrix(w, h), p),
                pixel(jittered.matrix(w, h), p),
            ];
            // The point appears `offset` up-left, so the pixel centre that sees it now
            // sampled it at centre + offset.
            assert!((a[0] - b[0] - offset[0]).abs() < 1e-4, "{a:?} {b:?}");
            assert!((a[1] - b[1] - offset[1]).abs() < 1e-4, "{a:?} {b:?}");
        }
    }

    #[test]
    fn reconstruction_modules_validate_without_ray_queries() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let resolve = resolve_source(prelude).unwrap();
        assert!(!resolve.contains("ray_query") && !resolve.contains("rayQuery"));
        let kernels = shader_sources(prelude).unwrap();
        // Main's module when listing is off; P-RTS's seams when it is on.
        let off = trace_source_with(&kernels[1], false).unwrap();
        // P-RELIT: each receiver traces and normalises by its own count.
        assert!(off.contains(
            "tr1_rays=tr1_zone_rays(world,primary_frame.sampling.x);for(var j=lane;j<tr1_rays;j+=1u)"
        ));
        assert!(off.contains("256u/tr1_rays") && !off.contains("256u/FRESH_DIRECTIONS"));
        // U-RTVIS: the probe stop is the budget loop's first statement.
        assert!(off.contains(&format!(
            "j<tr1_rays;j+=1u) {{\n        {COLD_PROBE_STOP}\n        {}",
            ROTATION_SEAM.1
        )));
        assert!(!off.contains("primary_penumbra_trace"));
        // U-RTVIS: the held module asks and records in both trace entries, stops after the
        // cold probes' check, and records uncovered pixels as no history.
        let held = trace_source_held(&kernels[1]).unwrap();
        assert_eq!(
            held.matches("rtvis_hold(receiver,trace.record.x-1u);")
                .count(),
            2
        );
        assert_eq!(held.matches("rtvis_store(p,visible);").count(), 2);
        assert_eq!(
            held.matches("rtvis_store(vec2<i32>(id.xy),FRESH_SKIP);")
                .count(),
            2
        );
        assert!(held.contains(&format!(
            "{COLD_PROBE_STOP}\n        {HELD_STOP}\n        {}",
            ROTATION_SEAM.1
        )));
        let trace = trace_source_with(&kernels[1], true).unwrap();
        assert!(trace.contains(
            "fresh_direction(select(j,j-primary_frame.sampling.z,wide)+primary_frame.sided.y"
        ));
        // P-RTS: probes trace sampling.y; listed pixels stride the whole budget.
        assert_eq!(trace.matches("j<primary_frame.sampling.y;j+=1u").count(), 1);
        assert_eq!(
            trace
                .matches("bitcast<f32>(primary_frame.sampling.w),wide)")
                .count(),
            1
        );
        assert_eq!(
            trace
                .matches("j<primary_frame.sampling.x;j+=PENUMBRA_LANES")
                .count(),
            1
        );
        // The measurement-only mask modes (mask_resolve) must stay valid as TR1 changes.
        let masks = ["tr1-mask", "tr1-raw", "tr1-window"]
            .map(|m| resolve.replace(RESOLVE, &mask_resolve(m)));
        // U-FIXED: the visibility-buffer sun asks and records once more, on the budget's estimate.
        let vb_sun = vb_sun_source(&kernels[1]).unwrap();
        assert_eq!(vb_sun.matches("rtvis_hold(receiver,slot);").count(), 1);
        assert_eq!(vb_sun.matches("rtvis_store(p,visible);").count(), 3);
        let resolve_clone = resolve.clone();
        for source in [resolve, off, held, trace, PENUMBRA_LIST.to_owned(), vb_sun]
            .into_iter()
            .chain(masks)
            .chain([fresh_clamp_resolve(&resolve_clone, 0.5)])
        {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
        // Production modules never see the rotation.
        assert!(!kernels[1].contains("sided.y"));
    }

    #[test]
    fn render_scales() {
        assert_eq!(RenderScale::Native.internal_size(1920, 1080), [1920, 1080]);
        assert_eq!(RenderScale::Quality.internal_size(1920, 1080), [1280, 720]);
        assert_eq!(
            RenderScale::Performance.internal_size(1920, 1080),
            [960, 540]
        );
        assert_eq!(RenderScale::Performance.internal_size(1, 3), [1, 2]);
        assert_eq!(Reconstruction::quality().scale, RenderScale::Quality);
        assert_eq!(Reconstruction::default().scale, RenderScale::Native);
    }

    #[test]
    fn settings_validate() {
        assert!(Reconstruction::default().validate().is_ok());
        assert!(Reconstruction::default().enabled);
        assert_eq!(Reconstruction::identity(), Reconstruction::default());
        assert!(!Reconstruction::off().enabled);
        assert!(Reconstruction::off().validate().is_ok());
        for bad in [
            Reconstruction {
                window: 0.5,
                ..Default::default()
            },
            Reconstruction {
                blur_budget: f32::NAN,
                ..Default::default()
            },
            Reconstruction {
                turn_angle: -1.0,
                ..Default::default()
            },
            Reconstruction {
                view_budget: f32::INFINITY,
                ..Default::default()
            },
            Reconstruction {
                light_budget: f32::NAN,
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }

    /// MQ2: a program declaring `view_dependence` gets its struct on every stage and the hint
    /// on traced stages; any other program's traced entry stores the view-independent 1.
    #[test]
    fn view_dependence_composition() {
        use crate::program::{Stage, compose};
        let toon = include_str!("../shaders/programs/toon.wgsl");
        for stage in [
            Stage::Raster,
            Stage::Traced,
            Stage::TracedStreamed,
            Stage::Cell,
        ] {
            let traced = matches!(stage, Stage::Traced | Stage::TracedStreamed);
            let standard = compose("", crate::STANDARD_PROGRAM, stage, false).source;
            let other = compose("", toon, stage, true).source;
            assert!(standard.contains("struct ViewDependence {"));
            assert!(!other.contains("ViewDependence"));
            assert_eq!(standard.contains("let d=view_dependence(s,input);"), traced);
            for source in [&standard, &other] {
                assert_eq!(
                    source.matches("fn tr1_view_hint(").count(),
                    usize::from(traced)
                );
                assert_eq!(
                    source
                        .matches("tr1_view_hint(surface_output,surface_input,id,size)")
                        .count(),
                    usize::from(traced)
                );
            }
        }
    }

    #[test]
    fn shadow_budget_keeps_resets_and_motion_at_the_active_count() {
        let budget = ReconstructionShadowBudget {
            cold_rays: 256,
            reset_rays: 64,
            active_rays: 32,
            held_rays: 8,
            settle_frames: 16,
        };
        assert!(budget.validate().is_ok());
        assert_eq!(budget.rays(true, true, true, 100), 256);
        assert_eq!(budget.rays(false, true, true, 100), 64);
        assert_eq!(budget.rays(false, false, false, 0), 32);
        assert_eq!(budget.rays(false, false, false, 100), 32);
        assert_eq!(budget.rays(false, false, true, 15), 32);
        assert_eq!(budget.rays(false, false, true, 16), 8);
        for rays in [0, 7, 12, 257] {
            assert!(
                ReconstructionShadowBudget {
                    active_rays: rays,
                    ..budget
                }
                .validate()
                .is_err()
            );
        }
    }
}
