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
    /// P-FLAT (docs/p-flat-tr1.md): on frames where local lights' intensity or colour changed,
    /// a pixel within such a light's range caps its window at `1 + light_budget / (change *
    /// falloff)`. `change` is the light's largest relative channel change; `falloff` is its
    /// range-windowed inverse square relative to 1 m, the shading's attenuation shape. Where
    /// the cap falls below 2 the pixel restarts, as every pixel did before P-FLAT. Only engine
    /// data and the receiver's position size the cap, never the pixel's colour. History
    /// continues under MQ1's 3x3 box. Zero, the default, restarts every pixel on any such
    /// change, as before P-FLAT. A positive budget trades a measured flicker lag near the light
    /// for history elsewhere (docs/p-flat-tr1.md). More than four changed lights in one frame
    /// also restart.
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
    pub(in crate::renderer) fn validate(&self) -> Result<(), Error> {
        let windows = [
            self.window,
            self.sharp_window,
            self.coverage_window,
            self.start_window,
        ];
        let valid = windows.iter().all(|w| (1.0..=65536.0).contains(w))
            && (0.0..=f32::MAX).contains(&self.blur_budget)
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
const FRAME_BYTES: usize = 448;
/// Changed local lights one frame can cap (`Reconstruction::light_budget`); more restart.
const RELIT_SLOTS: usize = 4;
/// `Reconstruction::light_budget`'s default: zero, so a local light's intensity change
/// restarts every pixel. Every positive budget P-FLAT measured kept flicker lag on the surfaces
/// the light dominates (docs/p-flat-tr1.md, section 3).
const LIGHT_BUDGET: f32 = 0.0;
// Test-only tail permits the immutable MQ1 static-control shader's runtime-array
// minimum binding span. Production has only fixed words; transform history lives
// exclusively in the shared placement buffer.
const FRAME_BUFFER_BYTES: usize = FRAME_BYTES + if cfg!(test) { 64 } else { 0 };
/// `light` with its candela zeroed: a local light's identity for `reconstruction_key`.
fn without_candela(mut light: Light) -> Light {
    if let Light::Point { candela, .. } | Light::Spot { candela, .. } = &mut light {
        *candela = [0.; 3];
    }
    light
}
/// Local lights whose candela changed from `previous` (bits, in order): position, range
/// (f32::MAX when unlimited) and largest relative channel change. The key equals the previous
/// frame's, so the local lights are the same lights in the same order.
fn relit_lights(previous: &[[u32; 3]], lights: &[Light]) -> Vec<(Vec3, f32, f32)> {
    lights
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
            } => Some((position, range.unwrap_or(f32::MAX), candela)),
            Light::Directional { .. } => None,
        })
        .zip(previous)
        .filter_map(|((position, range, candela), old)| {
            let change = candela
                .iter()
                .zip(old.map(f32::from_bits))
                .map(|(&new, old)| (new - old).abs() / new.max(old).max(f32::MIN_POSITIVE))
                .fold(0.0, f32::max);
            (change > 0.0).then_some((position, range, change))
        })
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
pub(in crate::renderer) fn sample_offset(k: u64) -> [f32; 2] {
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
pub(in crate::renderer) fn rotation(k: u64) -> u32 {
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
    pub resolve: Owned<wgpu::ComputePipeline>,
}

pub(super) fn trace_source(soft_kernels: &str) -> Result<String, Error> {
    trace_source_with(soft_kernels, PENUMBRA_LISTING)
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
        return Ok(source
            .replace(
                loop_seam,
                "for(var j=lane;j<primary_frame.sampling.x;j+=1u)",
            )
            .replace("256u/FRESH_DIRECTIONS", "256u/primary_frame.sampling.x"));
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
pub(super) fn resolve_source(wgsl: &str) -> Result<String, Error> {
    let [head, _] = non_rt_heads(wgsl)?;
    // MV1 motion-source seam; history/resolve policy belongs to MQ1.
    Ok(format!(
        "{head}\n{}\n{RESOLVE}",
        include_str!("../shaders/object_motion.wgsl")
    ))
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
                trace: self.interactive_entry(
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
                full: deferred.then(|| self.interactive_pipeline(device, ledger, &soft, &eight)),
                // The penumbra pass follows the retained two-pass repair only.
                penumbra: (deferred && !modern && PENUMBRA_LISTING)
                    .then(|| self.penumbra_pipelines(device, ledger, &soft)),
            }
        });
        Pipelines { trace, resolve }
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
    skin_bounds: Option<crate::shadow_plan::Bounds>,
    skin_revisions: Vec<u64>,
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
    /// The side this frame writes; the other holds the previous frame.
    pub parity: usize,
    previous: Option<Previous>,
    /// The next explicit restart (`cut_reconstruction`).
    pub cut: bool,
    /// The last prepared frame restarted every pixel.
    pub reset: bool,
    held_frames: u32,
    pub shadow_rays: u32,
    /// P-RTS: directions every receiver traces first; below `shadow_rays`, only listed
    /// penumbra pixels trace the full budget (docs/p-rts.md).
    pub probe_rays: u32,
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
        &self.colour[self.parity].texture
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
            parity: 0,
            previous: None,
            cut: false,
            reset: true,
            held_frames: 0,
            shadow_rays: 8,
            probe_rays: 8,
        }
    }
    fn size(&self) -> wgpu::Extent3d {
        self.sample.texture.size()
    }
    /// The colour history this frame wrote (exposed mean, sample weight; Rgba32Float).
    #[cfg(test)]
    pub(in crate::renderer) fn written_colour(&self) -> &wgpu::Texture {
        &self.colour[self.parity].texture
    }
    fn output(&self) -> wgpu::Extent3d {
        self.colour[0].texture.size()
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
            self.reconstruction_state = None;
            self.reconstruction_index = 0;
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
            || self.soft_mode != SoftShadows::Interactive
            || self.shadows.settings.directional_disk_angle == 0.0
            || !self.reconstruction.enabled
        {
            return None;
        }
        self.reconstruction_state.as_ref().map(|s| s.shadow_rays)
    }
    /// This frame's internal colour target when an upscaling preset applies to it (a
    /// reconstructed frame below native scale), allocated for the caller's `color`; None
    /// renders at the caller's size and releases the upscaling targets.
    pub(in crate::renderer) fn upscale_target(
        &mut self,
        color: &wgpu::Texture,
    ) -> Option<wgpu::Texture> {
        let [width, height] = self
            .reconstruction
            .scale
            .internal_size(color.width(), color.height());
        if self.reconstruction_frame.is_none() || [width, height] == [color.width(), color.height()]
        {
            self.upscale = None;
            return None;
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        if self
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
            self.upscale = Some(Upscale {
                color: internal,
                hdr,
                present,
                output: color.clone(),
            });
        }
        let upscale = self.upscale.as_mut().unwrap();
        upscale.output = color.clone();
        Some((*upscale.color.texture).clone())
    }

    /// Identity of every input a reconstructed frame depends on besides the camera, the
    /// eye, exposure, directional-light directions, local-light intensities (P-FLAT), skinned
    /// poses (SK1) and placement transforms (MQ1), which TR1 follows: a change restarts every
    /// pixel. Programs, prelude, geometry, bindings, materials, textures, new placements, the
    /// sun's colour, light kinds and local lights' position, direction, range, cone and source
    /// radius, the environment and every shadow and visibility setting. Before MQ1 transform
    /// edits of existing placements restarted every pixel, so a scene with any animated rigid
    /// object never accumulated; they now reproject through their previous transforms. Before
    /// P-FLAT a flickering local light restarted every pixel every frame in the same way.
    fn reconstruction_key(&self) -> [u8; 32] {
        let u = &self.uploads;
        let state = self.primary.as_ref().unwrap();
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
            .chain_update(format!("{:?}", self.shadows.settings).as_bytes())
            .chain_update(format!("{:?}", self.virtual_settings).as_bytes())
            .chain_update(format!("{:?} {:?}", self.soft_mode, self.primary_mode).as_bytes())
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
            u.environment,
            u.geometry,
            u.lut,
            u.textures,
            u.materials,
            // MV1: transform/derived-normal uploads move surfaces; they no longer
            // reset all history. Placement identity/topology stays in the key above.
            u.instances,
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
        if self
            .pipeline
            .primary
            .as_ref()
            .unwrap()
            .reconstruction
            .as_ref()
            .is_some_and(|p| p.trace.is_some() || !traced)
        {
            return Ok(());
        }
        let resolve = resolve_source(&self.source)?;
        let trace = if traced {
            Some(trace_source(&self.primary_sources.as_ref().unwrap().0[1])?)
        } else {
            None
        };
        let scopes = OperationScopes::new(&self.device);
        let production = self.pipeline.primary.as_ref().unwrap();
        let pipelines = self.primary.as_ref().unwrap().reconstruction_pipelines(
            &self.device,
            &self.ledger,
            &resolve,
            trace.as_deref().map(|s| (s, production)),
        );
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => self.fail(error.to_string()),
            });
        }
        self.pipelines.created += 1 + pipelines.trace.as_ref().map_or(0, |t| {
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
        let pipelines = self.primary.as_ref().unwrap().reconstruction_pipelines(
            &self.device,
            &self.ledger,
            &source,
            None,
        );
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
        let size = self.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let output = self.upscale.as_ref().map_or(size, |u| u.hdr.texture.size());
        if self
            .reconstruction_state
            .as_ref()
            .is_none_or(|s| s.size() != size || s.output() != output)
        {
            let cut = self.reconstruction_state.as_ref().is_some_and(|s| s.cut);
            let mut next = ReconstructionState::new(self, size, output);
            next.cut = cut;
            self.reconstruction_state = Some(next);
        }
        let key = self.reconstruction_key();
        let intensities = local_intensities(&self.lighting.source_lights);
        let settings = self.reconstruction;
        let matrix = camera.unjittered().matrix(size.width, size.height);
        let eye = camera.view().inverse().w_axis.truncate();
        let exposure = self.lighting.pending.exposure;
        let suns: Vec<Vec3> = self
            .lighting
            .source_lights
            .iter()
            .filter_map(|l| match l {
                crate::Light::Directional { travel, .. } => Some(travel.normalize()),
                _ => None,
            })
            .collect();
        let background = self.lit_clear().map(|v| v as f32);
        // AT1: misses also vary per pixel while the atmosphere's sky is on.
        let visible_sky = (self.environment.visible && self.environment.selected.is_some())
            || self.atmosphere_sky_visible();
        let pixel_angle = 2.0 * (0.5 * camera.fov_y()).tan() / output.height as f32;
        // SK1: vertex motion reprojects the character, but its changing shadow also
        // changes otherwise-static receivers. Reject those receivers' lighting history
        // in a conservative world-space swept shadow volume, never a screen-space
        // lighting approximation. This leaves unrelated static pixels accumulating.
        let union = |a: Option<crate::shadow_plan::Bounds>, b: crate::shadow_plan::Bounds| {
            a.map_or(b, |a| crate::shadow_plan::Bounds {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
            })
        };
        let mut scene_bounds = None;
        let mut skin_bounds = None;
        let mut skin_revisions = vec![];
        let mut rigid_motion = false;
        for d in draws {
            if let Some(s) = &d.mesh.skin {
                skin_revisions.push(s.revision);
            }
            for (&model, &previous_model) in d
                .instances
                .state
                .models
                .iter()
                .zip(d.instances.state.previous_models(self.frames))
            {
                let moved = model.to_cols_array().map(f32::to_bits)
                    != previous_model.to_cols_array().map(f32::to_bits);
                rigid_motion |= moved;
                if let Ok(b) = shadow_gpu::world_bounds(d.mesh.bounds, model) {
                    scene_bounds = Some(union(scene_bounds, b));
                    if d.mesh.skin.is_some() {
                        skin_bounds = Some(union(skin_bounds, b));
                    }
                }
            }
        }
        let mut shadow_bounds = None;
        // The shadowed sun's direction toward it and disk half-angle, when `shadow_bounds` is
        // tested as its swept shadow rather than as a box (P-FLAT).
        let mut caster_light: Option<(Vec3, f32)> = None;
        // MQ2: some skinned pose changed since the previous frame.
        let mut posed = false;
        if let (Some(current), Some(previous)) = (
            skin_bounds,
            self.reconstruction_state
                .as_ref()
                .and_then(|s| s.previous.as_ref()),
        ) && previous.skin_revisions != skin_revisions
        {
            posed = true;
            let swept = union(previous.skin_bounds, current);
            let mut affected = swept;
            let distance = scene_bounds.map_or(1., |b: crate::shadow_plan::Bounds| {
                (b.max - b.min).length().max(1.)
            }) * 2.;
            let requested: Vec<&Light> = self
                .lighting
                .source_lights
                .iter()
                .zip(&self.shadows.settings.requests)
                .filter(|(_, r)| !matches!(r, ShadowRequest::None))
                .map(|(l, _)| l)
                .collect();
            // P-FLAT: with one shadowed sun and no other shadow request, receivers are tested
            // against the swept bounds' own shadow (`tr1_caster_shadow`): inside the bounds, or
            // a ray toward the sun within its disk meets them. The box below stretches the
            // bounds twice the scene's size along the sun; under a low sun it held most of the
            // scene's static receivers to one frame of history while a character walked.
            if let [Light::Directional { travel, .. }] = requested[..] {
                caster_light = Some((
                    -travel.normalize(),
                    self.shadows.settings.directional_disk_angle,
                ));
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
        let rendered_frames = self.frames;
        let state = self.reconstruction_state.as_mut().unwrap();
        let cold = state.previous.is_none() || state.cut;
        let continued = state
            .previous
            .as_ref()
            .filter(|p| p.frame == rendered_frames && p.key == key && p.suns.len() == suns.len())
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
        // P-FLAT: local lights whose intensity or colour changed. History continues, held to
        // this frame's 3x3 box as for moving placements and capped near those lights
        // (`light_budget`), so a flicker is followed without restarting every pixel on every
        // frame. A zero budget, or more changed lights than the frame can carry, restarts.
        let changed = match (continued, state.previous.as_ref()) {
            (Some(_), Some(p)) if p.intensities != intensities => {
                relit_lights(&p.intensities, &self.lighting.source_lights)
            }
            _ => vec![],
        };
        let light_restart =
            !changed.is_empty() && (settings.light_budget == 0.0 || changed.len() > RELIT_SLOTS);
        state.reset = continued.is_none() || state.cut || event_window < 1.0 || light_restart;
        state.cut = false;
        // MQ1's clamp event comes from the same submitted transforms that feed
        // TR1/PP1 on the GPU; no second transform history or frame-buffer payload.
        let moved = !state.reset && rigid_motion;
        let relit = !state.reset && !changed.is_empty();
        let jitter = camera.jitter();
        let (previous_matrix, previous_eye, scale, previous_jitter) = match continued {
            Some((m, e, x, _, j)) if !state.reset => (m, e, exposure / x, j),
            _ => (matrix, eye, 1.0, jitter),
        };
        state.parity = 1 - state.parity;
        // MV1 keeps the shared motion source; MQ1/MQ2 clamp rigid or skinned motion.
        let options = u32::from(settings.catmull_rom)
            | u32::from(settings.spatial_start) << 1
            | u32::from((moved || relit || (posed && !state.reset)) && settings.motion_box) << 3
            | u32::from(settings.upscale_interpolation) << 4
            | u32::from(relit) << 5;
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
        let still = !state.reset
            && previous_matrix.to_cols_array().map(f32::to_bits)
                == matrix.to_cols_array().map(f32::to_bits);
        // MV1 motion-source integration: rigid/deforming casters must not enter
        // RTDEF2's settled shadow budget just because the camera is stationary.
        let sampling_still = still && turn == 0.0 && !rigid_motion && shadow_bounds.is_none();
        state.held_frames = if sampling_still {
            state.held_frames.saturating_add(1)
        } else {
            0
        };
        // P-FLAT: frames that drop history somewhere (receivers in a moving caster's shadow, or
        // pixels near a relit light) take the restart budget, as frames that restart every
        // pixel do: the restarted pixels keep main's per-pixel ray count.
        state.shadow_rays = self.reconstruction_shadow_budget.rays(
            cold,
            state.reset || shadow_bounds.is_some() || relit,
            sampling_still,
            state.held_frames,
        );
        // P-RTS: with the penumbra pass compiled and edges deferred, every receiver first
        // traces the held count; only listed pixels trace the rest of the budget.
        let primary = self.primary.as_ref().unwrap();
        let listing = primary.targets.as_ref().is_some_and(|t| t.defer_edges)
            && self.pipeline.primary.as_ref().is_some_and(|p| {
                p.reconstruction
                    .as_ref()
                    .and_then(|r| r.trace.as_ref())
                    .is_some_and(|t| t.penumbra.is_some())
            });
        // Listing pays only where the budget exceeds both probe sets.
        state.probe_rays = if listing && PENUMBRA_LISTING && state.shadow_rays > 2 * PROBE_DISK {
            2 * PROBE_DISK
        } else {
            state.shadow_rays
        };
        // Listing frames probe a cone PROBE_CONE times the sun's; frames without listing
        // keep the exact budget directions (x 1.0 is exact).
        let (disk, widen) = if state.probe_rays < state.shadow_rays {
            (PROBE_DISK, PROBE_CONE)
        } else {
            (0, 1.0)
        };
        // PrimaryFrame.sampling is private engine state; no material/shading ABI changes.
        let primary_frame = &primary.frame.as_ref().unwrap().0.value;
        let mut words = state.shadow_rays.to_ne_bytes().to_vec();
        words.extend_from_slice(&state.probe_rays.to_ne_bytes());
        words.extend_from_slice(&disk.to_ne_bytes());
        words.extend_from_slice(&widen.to_bits().to_ne_bytes());
        self.queue.write_buffer(primary_frame, 112, &words);
        self.uploads.view_settings += 16;
        self.uploads.write_calls += 1;
        for w in [
            u32::from(state.reset),
            options,
            u32::from(still),
            index as u32,
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
        // relit[4]: position and range of each changed light (zero rows unused), then each one's
        // relative change, then the budget.
        let mut relit_words = [0f32; 24];
        if relit {
            for (k, (position, range, change)) in changed.iter().enumerate() {
                relit_words[k * 4..k * 4 + 4]
                    .copy_from_slice(&[position.x, position.y, position.z, *range]);
                relit_words[16 + k] = *change;
            }
            relit_words[20] = settings.light_budget;
        }
        for value in relit_words {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        debug_assert_eq!(bytes.len(), FRAME_BYTES);
        self.queue.write_buffer(&state.frame.value, 0, &bytes);
        self.uploads.view_settings += FRAME_BYTES as u64;
        self.uploads.write_calls += 1;
        state.previous = Some(Previous {
            frame: rendered_frames + 1,
            matrix,
            jitter,
            eye,
            exposure,
            suns,
            key,
            intensities,
            skin_bounds,
            skin_revisions,
        });
        // The resolve writes the frame's HDR target, or the upscaled output.
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let out = self.upscale.as_ref().map_or(hdr, |u| &u.hdr);
        let primary = self.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let frame = &primary.frame.as_ref().unwrap().0.value;
        let state = self.reconstruction_state.as_ref().unwrap();
        let group_key: GroupKey = (
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            (*out.texture).clone(),
            (**frame).clone(),
            (*state.frame.value).clone(),
        );
        if state.groups.as_ref().is_none_or(|(k, _)| *k != group_key) {
            let groups = self.reconstruction_groups();
            self.reconstruction_state.as_mut().unwrap().groups = Some((group_key, groups));
        }
    }

    fn reconstruction_groups(&self) -> [Owned<wgpu::BindGroup>; 2] {
        use wgpu::BindingResource as R;
        let state = self.reconstruction_state.as_ref().unwrap();
        let primary = self.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let frame = &primary.frame.as_ref().unwrap().0.value;
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let out = self.upscale.as_ref().map_or(hdr, |u| &u.hdr);
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
        let state = self.reconstruction_state.as_ref().unwrap();
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        encoder.copy_texture_to_texture(
            hdr.texture.as_image_copy(),
            state.sample.texture.as_image_copy(),
            state.size(),
        );
        let size = state.output();
        let primary = self.primary.as_ref().unwrap();
        let pipelines = self.pipeline.primary.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("temporal reconstruction"),
            timestamp_writes,
        });
        pass.set_pipeline(&pipelines.reconstruction.as_ref().unwrap().resolve);
        pass.set_bind_group(0, &*state.groups.as_ref().unwrap().1[state.parity], &[]);
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
        assert!(off.contains("for(var j=lane;j<primary_frame.sampling.x;j+=1u)"));
        assert!(!off.contains("primary_penumbra_trace"));
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
        for source in [resolve, off, trace, PENUMBRA_LIST.to_owned()] {
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
