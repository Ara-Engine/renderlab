//! TP1 transparent pass on the GPU (docs/m7-tp1.md, shaders/transparency.wgsl). Lit frames
//! with transparent draws or live emitters run, after opaque shading and temporal
//! reconstruction and before presentation, into the resolved HDR target:
//! 1. opaque view depth: from the hit records on traced and visibility-buffer frames, from
//!    the raster depth on 1x raster frames;
//! 2. the transparent receivers' sun shadow map, rendered only when the sun, the receivers'
//!    footprint or the opaque casters change;
//! 3. particle simulation and a one-workgroup bitonic sort per emitter;
//! 4. transparent draws and emitters back to front by object. A transparent draw copies
//!    the HDR colour behind it first, then composites its program's `shade` over the
//!    refracted copy.
//!
//! Transparent draws compose the material's own program (`surface`, `shade` and the
//! optional `transmittance` hook). Their sun visibility is the renderer's hardware ray on
//! RT renderers and the shadow map otherwise; particles use the map on both.
use super::*;
use crate::transparency::{
    Emitter, EmitterDescriptor, EmitterShared, Forces, MAX_CURVE_KEYS, PlaneCollision, SpawnShape,
    TransparencyStats,
};
use glam::Vec4;

#[path = "transparency_reflection.rs"]
pub(crate) mod reflection;
#[path = "transparency_shadow.rs"]
pub(crate) mod transmission_shadow;
#[path = "transparency_panes.rs"]
mod panes;

const SHADER: &str = include_str!("../shaders/transparency.wgsl");
/// Square sun shadow map of the transparent receivers' footprint.
pub(crate) const SHADOW_SIZE: u32 = 2048;
/// Transparent draws per frame: one 256-byte dynamic slot each.
const MAX_TRANSPARENT_DRAWS: usize = crate::TRACED_DRAWS as usize;
pub(crate) const SLOT: u64 = 256;
/// The camera block at 0, then `TpFrame` at 256.
pub(crate) const FRAME_OFFSET: u64 = 256;
pub(crate) const FRAME_BYTES: u64 = 336;
/// WGSL `TpEmitter`: ten vec4s of TP1, then shape, velocity box, forces, collision,
/// curve counts, the colour, emission and size curves, and the sprite's alignment and
/// flipbook (`EmitterParams::uniform`).
const EMITTER_BYTES: u64 = 512;
/// Substeps a stepped prewarm runs at most (`EmitterParams::steps_prewarm`).
const MAX_PREWARM_STEPS: usize = 600;
const PARTICLE_BYTES: u64 = 48;

fn glass_samples() -> u32 {
    #[cfg(test)]
    if std::env::var("RENDERLAB_TP2_PROFILE").is_ok_and(|s| !s.is_empty()) {
        return 1;
    }
    4
}

/// One `// ---- NAME ----` section of the shader.
pub(crate) fn section(name: &str) -> &'static str {
    let start = SHADER.find(&format!("// ---- {name} ----")).unwrap();
    let end = SHADER.find(&format!("// ---- end {name} ----")).unwrap();
    &SHADER[start..end]
}

/// `source` with its `fn name(` function replaced by `replacement`.
fn replace_function(source: &str, name: &str, replacement: &str) -> String {
    let start = source.find(&format!("fn {name}(")).unwrap();
    let brace = start + source[start..].find('{').unwrap();
    let mut depth = 1;
    let mut end = brace + 1;
    let bytes = source.as_bytes();
    while depth != 0 {
        depth += i32::from(bytes[end] == b'{') - i32::from(bytes[end] == b'}');
        end += 1;
    }
    format!("{}{replacement}{}", &source[..start], &source[end..])
}

/// The non-RT transparent head's sun visibility: the cached map, for the frame's sun.
const MAP_SHADOW: &str = "fn hard_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    if light.kind.x!=0u || dot(l,tp.sun.xyz)<0.9999 { return 1.0; }
    return tp_sun_visibility(receiver.world);
}";

/// A transparent material's module: its program composed on `head` (the hardware prelude
/// on RT renderers; otherwise the prelude with the map's sun visibility), then the pass's
/// entry with the program's composite.
pub(crate) fn glass_source(head: &str, rt: bool, program: &str, parameters: bool) -> String {
    let head = if rt {
        head.to_owned()
    } else {
        replace_function(head, "hard_shadow", MAP_SHADOW)
    };
    let mut composed =
        crate::program::compose(&head, program, crate::program::Stage::Raster, parameters);
    // TP2: transparent receivers create no cache cells. Searching unrelated opaque
    // cells cost ~0.95 ms on the quarry; use the open environment, as R1 hit shading does.
    #[cfg(not(test))]
    let open = true;
    #[cfg(test)]
    let open = std::env::var("RENDERLAB_TP2_PROFILE").map_or(true, |v| v.is_empty());
    if open && composed.source.contains("fn gi_prepare(") {
        composed.source = replace_function(
            &composed.source,
            "gi_prepare",
            "fn gi_prepare(input:SurfaceInput) { gi_rx_valid=false; }",
        );
    }
    // TP2: paired profiling variants change only this pass, in the same test binary.
    #[cfg(test)]
    let composed = {
        let mut composed = composed;
        let variant = std::env::var("RENDERLAB_TP2_PROFILE").unwrap_or_default();
        if variant.contains("open") && composed.source.contains("fn gi_prepare(") {
            composed.source = replace_function(
                &composed.source,
                "gi_prepare",
                "fn gi_prepare(input:SurfaceInput) { gi_rx_valid=false; }",
            );
        }
        if variant.contains("map") {
            composed.source = replace_function(&composed.source, "hard_shadow", MAP_SHADOW);
        }
        if variant.contains("nofog") {
            composed.source = replace_function(
                &composed.source,
                "atmosphere_surface",
                "fn atmosphere_surface(c:vec3<f32>, p:vec3<f32>) -> vec3<f32> { return c; }",
            );
        }
        composed
    };
    // The surface's own light and the per-channel share of the light behind it.
    let (surface, transmit) = if crate::program::declares_transmittance(program) {
        ("lit", "transmittance(surface_output,surface_input)")
    } else {
        ("lit*coverage", "vec3<f32>(1.0-coverage)")
    };
    let source = format!(
        "{}\n{}{}{}",
        composed.source,
        section("TP_FRAME"),
        section("TP_SUN"),
        section("TP_GLASS")
            .replace("TP_SURFACE", surface)
            .replace("TP_TRANSMIT", transmit)
    );
    // TP2: a material's own lobe is rasterized separately from its final composite.
    let glass = section("TP_GLASS");
    let body = &glass[glass.find("    let source_front=").unwrap()
        ..glass
            .find("    rl_indirect_prepare(surface_input);")
            .unwrap()];
    let lobe = if crate::program::declares_reflection(program) {
        "let lobe=reflection_lobe(surface_output,surface_input);\nif lobe.weight<=0.0 {discard;}\nreturn TpLobeOutput(vec4<f32>(input.world,surface_input.receiver.epsilon),vec4<f32>(lobe.normal,lobe.alpha),vec4<f32>(surface_input.receiver.geometric,0.0));"
    } else {
        "return TpLobeOutput(vec4<f32>(0.0),vec4<f32>(0.0),vec4<f32>(0.0));"
    };
    let source = format!(
        "{source}\nstruct TpLobeOutput {{ @location(0) position:vec4<f32>, @location(1) normal:vec4<f32>, @location(2) geometric:vec4<f32> }};\n@fragment fn fs_tp_lobe(input:RasterInput, triangle:RasterTriangle, @builtin(front_facing) front:bool)->TpLobeOutput {{\n{body}\n{lobe}\n}}\n"
    );
    if crate::program::declares_reflection(program) {
        replace_function(
            &source,
            "specular_incident",
            "fn specular_incident(input:SurfaceInput)->vec4<f32> { if tp.misc.w==0.0 {return vec4<f32>(0.0);} return textureLoad(tp_reflected,vec2<i32>(input.pixel),0); }",
        )
    } else {
        source
    }
}
/// A film's glass module: `glass` (from `glass_source`, for a program that declares films,
/// `crate::program::declares_films`) with `fs_transparent` ending in what hardware blending
/// adds over the colour behind it, instead of reading that colour: the surface's fogged light
/// less the fog its transmittance lets through, with alpha `1 - transmittance`, so
/// `src + (1 - alpha) * behind` is the copy path's `light + transmittance * (behind - fog)`.
/// The blend state is premultiplied alpha (`src * 1 + dst * (1 - src.a)`).
pub(crate) fn film_source(glass: &str) -> String {
    let behind = "    let behind=tp_behind(input.world,refraction_normal,geometric,pixel,depth,transparent_ior(material));\n";
    let composite = "    return vec4<f32>(rl_finite(atmosphere_surface(rl_finite(lit),input.world)+passed*max(behind-fog,vec3<f32>(0.0))),1.0);\n";
    let film = "    return vec4<f32>(rl_finite(atmosphere_surface(rl_finite(lit),input.world)-passed*fog),1.0-passed.x);\n";
    assert_eq!(glass.matches(behind).count(), 1, "film seam: the colour behind");
    assert_eq!(glass.matches(composite).count(), 1, "film seam: the composite");
    glass.replacen(behind, "", 1).replacen(composite, film, 1)
}
/// The particle simulation and draw modules, after `prelude` for its environment helpers
/// and AT1's camera fog and aerial perspective.
fn particle_sources(prelude: &str) -> [String; 2] {
    let common = format!(
        "{prelude}\n{}{}{}{}{}",
        transmission_shadow::api_source(),
        crate::atmosphere::api_source(),
        section("TP_FRAME"),
        section("TP_SUN"),
        section("TP_PARTICLES")
    );
    ["TP_PARTICLES_SIM", "TP_PARTICLES_DRAW"].map(|s| format!("{common}{}", section(s)))
}
/// Share the original hit-to-view-distance composition across native backends.
pub(crate) fn hit_depth_source() -> String {
    format!(
        "enable wgpu_binding_array;\nconst PRIMARY_BATCHES:u32={}u;\n{}{}",
        crate::TRACED_DRAWS,
        section("TP_FRAME"),
        section("TP_DEPTH")
    )
}

/// Final production compositions, exposed only to the CPU source census.
#[cfg(test)]
pub(super) fn compile_structure_sources(
    prelude: &str,
    program: &str,
    rt: bool,
) -> Result<Vec<(&'static str, String)>, Error> {
    let head = if rt {
        crate::hardware_shadows::shader_source(prelude, SampleCount::One)?
    } else {
        prelude.to_owned()
    };
    let parameters = crate::program::declares_parameters(program);
    let [sim, draw] = particle_sources(prelude);
    let mut sources = vec![
        ("glass", glass_source(&head, rt, program, parameters)),
        ("particles-sim", sim),
        ("particles-draw", draw),
        ("hit-depth", hit_depth_source()),
        (
            "raster-depth",
            format!("{}{}", section("TP_FRAME"), section("TP_DEPTH_RASTER")),
        ),
        (
            "shadow",
            format!("{}{}", section("TP_FRAME"), section("TP_SHADOW")),
        ),
        ("aa-seed", section("TP_AA_SEED").to_owned()),
        ("transmission", transmission_shadow::draw_source(prelude)),
    ];
    if rt {
        let head = primary_ray_gpu::transparent_reflection_head(prelude)?;
        sources.push(("reflection", reflection::source(&head, program, parameters)));
    }
    Ok(sources)
}

#[cfg(test)]
#[path = "transparency_tests.rs"]
mod tp1_tests;

#[derive(Default)]
pub(super) struct TransparencyState {
    #[cfg(test)]
    profile: Option<String>,
    /// Per-pixel panes for this renderer; None reads `RENDERLAB_PANES` (`panes::wanted`).
    #[cfg(test)]
    pub(super) pane_pixels: Option<bool>,
    pub stats: TransparencyStats,
    /// Particle simulation step per lit frame, seconds.
    pub step: f32,
    /// The frame's camera before any reconstruction jitter.
    pub camera: Option<Camera>,
    emitters: Vec<EmitterSlot>,
    next_emitter: u64,
    gpu: Option<Gpu>,
    glass: Vec<GlassPipelines>,
    shadow_key: Option<[u8; 32]>,
    /// The sun map fit last computed and what it was computed from (`encode_transparency`).
    fit_memo: Option<([u8; 32], Option<ShadowFit>)>,
}

struct Gpu {
    frame: Owned<wgpu::Buffer>,
    draws: Owned<wgpu::Buffer>,
    sampler: Owned<wgpu::Sampler>,
    compare: Owned<wgpu::Sampler>,
    shadow: Depth,
    white: Owned<wgpu::TextureView>,
    _white: Owned<wgpu::Texture>,
    /// Indexed by RT (hardware sun rays) or not.
    /// Indexed as above; the RT entry exists only on hardware-shadow renderers.
    glass_bindings: Vec<Owned<wgpu::BindGroupLayout>>,
    glass_layouts: Vec<Owned<wgpu::PipelineLayout>>,
    /// Per-pixel panes (`panes`): the glass group plus the pane's own bindings, by path.
    pane_bindings: Vec<Owned<wgpu::BindGroupLayout>>,
    pane_layouts: Vec<Owned<wgpu::PipelineLayout>>,
    shadow_group: Owned<wgpu::BindGroup>,
    shadow_pipeline: Owned<wgpu::RenderPipeline>,
    raster_depth_bindings: Owned<wgpu::BindGroupLayout>,
    raster_depth: Owned<wgpu::ComputePipeline>,
    /// Created with the first traced or visibility-buffer frame.
    hit_depth: Option<(Owned<wgpu::BindGroupLayout>, Owned<wgpu::ComputePipeline>)>,
    sim_bindings: Owned<wgpu::BindGroupLayout>,
    draw_bindings: Owned<wgpu::BindGroupLayout>,
    /// Built from the prelude they were composed with.
    particles: Option<ParticlePipelines>,
    empty: Owned<wgpu::BindGroup>,
    empty_bindings: Owned<wgpu::BindGroupLayout>,
    // TP2: seed the multisampled glass attachment with its exact background.
    aa_bindings: Owned<wgpu::BindGroupLayout>,
    aa_seed: Owned<wgpu::RenderPipeline>,
}
struct ParticlePipelines {
    prelude: [u8; 32],
    update: Owned<wgpu::ComputePipeline>,
    sort: Owned<wgpu::ComputePipeline>,
    draw: Owned<wgpu::RenderPipeline>,
}
pub(super) struct Targets {
    color: Depth,
    depth: Depth,
    aa: Depth,
    /// Per-pixel panes' four-sample triangle ids (`panes`), when they are on.
    pane_ids: Option<Depth>,
    generation: u64,
}
impl Targets {
    /// The view's opaque view depth (R32Float, `dot(world - eye, forward)` of the
    /// unjittered camera), after a transparent pass ran; the ID pass tests glass against it.
    pub(super) fn opaque_depth(&self) -> &wgpu::TextureView {
        &self.depth.view
    }
}
struct GlassPipelines {
    program: u64,
    key: [u8; 32],
    /// By tangent stream.
    pipelines: [Owned<wgpu::RenderPipeline>; 2],
    lobe: [Owned<wgpu::RenderPipeline>; 2],
    /// Films (`crate::program::declares_films`): `film_source`'s module blended premultiplied
    /// over the target, by tangent stream; None for programs that do not declare films.
    films: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    /// Per-pixel pane pipelines, built the first time a frame shades panes per pixel.
    pane: Option<panes::PanePipelines>,
}
struct EmitterSlot {
    id: u64,
    handle: std::sync::Weak<EmitterShared>,
    params: EmitterParams,
    sprite: Option<Texture>,
    uniform: Owned<wgpu::Buffer>,
    sim: Owned<wgpu::BindGroup>,
    particles: Owned<wgpu::Buffer>,
    sorted: Owned<wgpu::Buffer>,
    spawned: u64,
    carry: f64,
    /// Seconds since the emitter started, prewarm included, at the end of its last step:
    /// particles' birth times for `Forces::curl_phase_rate`.
    clock: f64,
    /// A stepped prewarm's uniforms, one per substep, then this frame's own: run in order
    /// before the frame's simulation (`EmitterParams::steps_prewarm`).
    warm_steps: Option<Vec<u8>>,
    /// Seconds of history the first frame spawns (`EmitterDescriptor::prewarm`), and the
    /// burst it adds; both zero once spawned.
    warm: f32,
    burst: u32,
    _allocation: Allocation,
}
/// An emitter descriptor without its texture borrow.
#[derive(Clone, Copy)]
struct EmitterParams {
    position: Vec3,
    spawn_radius: f32,
    spawn_shape: SpawnShape,
    spawn_rate: f32,
    lifetime: [f32; 2],
    velocity: Vec3,
    velocity_spread: f32,
    velocity_box: Vec3,
    acceleration: Vec3,
    drag: f32,
    forces: Forces,
    collision: Option<PlaneCollision>,
    size: [f32; 2],
    color: [[f32; 4]; 2],
    emission: [[f32; 3]; 2],
    /// Curves copied out of the descriptor's borrows: key count, then (t, value) keys.
    color_keys: (usize, [crate::transparency::ColorKey; MAX_CURVE_KEYS]),
    size_keys: (usize, [crate::transparency::SizeKey; MAX_CURVE_KEYS]),
    fade: [f32; 2],
    soft_distance: f32,
    flipbook: Option<crate::transparency::Flipbook>,
    alignment: crate::transparency::SpriteAlignment,
    aspect: f32,
    random_rotation: bool,
    capacity: u32,
    seed: u32,
}
impl EmitterParams {
    fn new(d: &EmitterDescriptor<'_>) -> Self {
        Self {
            position: d.position,
            spawn_radius: d.spawn_radius,
            spawn_shape: d.spawn_shape,
            spawn_rate: d.spawn_rate,
            lifetime: d.lifetime,
            velocity: d.velocity,
            velocity_spread: d.velocity_spread,
            velocity_box: d.velocity_box,
            acceleration: d.acceleration,
            drag: d.drag,
            forces: d.forces,
            collision: d.collision,
            size: d.size,
            color: d.color,
            emission: d.emission,
            color_keys: (
                d.color_keys.len(),
                std::array::from_fn(|i| d.color_keys.get(i).copied().unwrap_or_default()),
            ),
            size_keys: (
                d.size_keys.len(),
                std::array::from_fn(|i| d.size_keys.get(i).copied().unwrap_or_default()),
            ),
            fade: d.fade,
            soft_distance: d.soft_distance,
            flipbook: d.flipbook,
            alignment: d.alignment,
            aspect: d.aspect,
            random_rotation: d.random_rotation,
            capacity: d.capacity,
            seed: d.seed,
        }
    }
    /// Whether a prewarm must be stepped: forces and collisions act along the way, which
    /// the closed-form placement of prewarmed particles (drag and acceleration) leaves out.
    fn steps_prewarm(&self) -> bool {
        let f = &self.forces;
        f.curl[1] != 0.0
            || f.vortex_axis != Vec3::ZERO
            || f.attractor_strength[0] != 0.0
            || self.collision.is_some()
    }
    /// A world box every live particle stays inside.
    fn bounds(&self) -> (Vec3, Vec3) {
        let life = self.lifetime[1];
        let shape = match self.spawn_shape {
            SpawnShape::Sphere => 0.0,
            SpawnShape::Box { half_extent } => half_extent.length(),
            SpawnShape::Cone { axis } => axis.length(),
        };
        // Forces bound by their strength (curl's field is at most 2 per axis); a vortex
        // or attractor is bounded by the distance it could carry a particle in a life.
        let f = &self.forces;
        let pull = 2.0 * 3f32.sqrt() * f.curl[1].abs()
            + f.vortex_axis.length()
                * (self.spawn_radius + shape + self.position.distance(f.vortex_center))
            + f.attractor_strength[0].abs() / f.attractor_strength[1].max(1e-6).powf(1.5);
        let largest = self.size_keys.1[..self.size_keys.0]
            .iter()
            .fold(self.size[0].max(self.size[1]), |m, k| m.max(k.size));
        let reach = self.spawn_radius
            + shape
            + (self.velocity.length() + self.velocity_spread + self.velocity_box.length()) * life
            + 0.5 * (self.acceleration.length() + pull) * life * life
            + 0.5 * largest;
        (
            self.position - Vec3::splat(reach),
            self.position + Vec3::splat(reach),
        )
    }
    /// `window`: the span this step's spawns are spread over, seconds (0: the step).
    /// `burst`: how many of this step's spawns (the last, youngest) are burst births,
    /// aged within the step rather than across `window`. `clock`: seconds since the
    /// emitter started, at the end of this step (`EmitterSlot::clock`). `dt`: this step's
    /// length when it is a prewarm substep (0: the frame's particle step).
    #[allow(clippy::too_many_arguments)]
    fn uniform(
        &self,
        first: u32,
        spawned: u32,
        burst: u32,
        textured: bool,
        window: f32,
        clock: f32,
        dt: f32,
    ) -> [u8; EMITTER_BYTES as usize] {
        const FLOATS: usize = EMITTER_BYTES as usize / 4;
        let mut f = [0f32; FLOATS];
        let v4 = |f: &mut [f32; FLOATS], at: usize, v: [f32; 4]| f[at..at + 4].copy_from_slice(&v);
        v4(
            &mut f,
            0,
            self.position.extend(self.spawn_radius).to_array(),
        );
        v4(
            &mut f,
            4,
            self.velocity.extend(self.velocity_spread).to_array(),
        );
        v4(&mut f, 8, self.acceleration.extend(self.drag).to_array());
        v4(&mut f, 12, self.color[0]);
        v4(&mut f, 16, self.color[1]);
        let [e0, e1] = self.emission;
        v4(&mut f, 20, [e0[0], e0[1], e0[2], self.size[0]]);
        v4(&mut f, 24, [e1[0], e1[1], e1[2], self.size[1]]);
        v4(
            &mut f,
            28,
            [
                self.lifetime[0],
                self.lifetime[1],
                self.fade[0],
                self.fade[1],
            ],
        );
        let sorted = self.capacity.next_power_of_two() as f32;
        v4(
            &mut f,
            36,
            [
                self.soft_distance,
                sorted,
                f32::from(u8::from(textured)),
                window,
            ],
        );
        // 40: shape (kind, then the box's half extents or the cone's height).
        v4(
            &mut f,
            40,
            match self.spawn_shape {
                SpawnShape::Sphere => [0.0; 4],
                SpawnShape::Box { half_extent } => {
                    [1.0, half_extent.x, half_extent.y, half_extent.z]
                }
                SpawnShape::Cone { axis } => [2.0, axis.x, axis.y, axis.z],
            },
        );
        let fo = &self.forces;
        // The curl field's origin rides in the w lanes of 44, 52 and 56; its phase rate in
        // 48's (shaders/transparency.wgsl `tp_curl_origin`).
        let o = fo.curl_origin;
        v4(&mut f, 44, self.velocity_box.extend(o.x).to_array());
        v4(
            &mut f,
            48,
            [fo.curl[0], fo.curl[1], burst as f32, fo.curl_phase_rate],
        );
        v4(&mut f, 52, fo.vortex_center.extend(o.y).to_array());
        v4(&mut f, 56, fo.vortex_axis.extend(o.z).to_array());
        v4(
            &mut f,
            60,
            fo.attractor.extend(fo.attractor_strength[0]).to_array(),
        );
        let (normal, offset, restitution) = self.collision.map_or((Vec3::ZERO, 0.0, 0.0), |c| {
            (c.normal.normalize(), c.offset, c.restitution)
        });
        v4(&mut f, 64, normal.extend(offset).to_array());
        v4(
            &mut f,
            68,
            [
                restitution,
                fo.attractor_strength[1],
                self.color_keys.0 as f32,
                self.size_keys.0 as f32,
            ],
        );
        // 72: colour key times; 76: colours; 92: emissions; 108: size key times; 112: sizes.
        let (colors, sizes) = (&self.color_keys.1, &self.size_keys.1);
        v4(&mut f, 72, colors.map(|k| k.t));
        for (i, k) in colors.iter().enumerate() {
            v4(&mut f, 76 + 4 * i, k.color);
            v4(
                &mut f,
                92 + 4 * i,
                [k.emission[0], k.emission[1], k.emission[2], 0.0],
            );
        }
        v4(&mut f, 108, sizes.map(|k| k.t));
        v4(&mut f, 112, sizes.map(|k| k.size));
        // 116: alignment (mode, stretch or the plane's unit normal); 120: flipbook.
        use crate::transparency::SpriteAlignment;
        v4(
            &mut f,
            116,
            match self.alignment {
                SpriteAlignment::Camera => [0.0; 4],
                SpriteAlignment::Velocity { stretch } => [1.0, stretch, 0.0, 0.0],
                SpriteAlignment::Plane { normal } => {
                    let n = normal.normalize();
                    [2.0, n.x, n.y, n.z]
                }
            },
        );
        v4(
            &mut f,
            120,
            self.flipbook.map_or([1.0, 1.0, 0.0, 0.0], |b| {
                let play = if b.over_life { 2.0 } else { 1.0 };
                [b.grid[0] as f32, b.grid[1] as f32, b.fps, play]
            }),
        );
        // 124: sprite aspect, random rotation, the emitter's clock and a substep's length.
        v4(
            &mut f,
            124,
            [
                self.aspect,
                f32::from(u8::from(self.random_rotation)),
                clock,
                dt,
            ],
        );
        let mut bytes = [0u8; EMITTER_BYTES as usize];
        for (dst, v) in bytes.chunks_exact_mut(4).zip(f) {
            dst.copy_from_slice(&v.to_ne_bytes());
        }
        for (k, v) in [self.capacity, first, spawned, self.seed]
            .into_iter()
            .enumerate()
        {
            bytes[128 + 4 * k..132 + 4 * k].copy_from_slice(&v.to_ne_bytes());
        }
        bytes
    }
}

fn entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}
fn uniform(dynamic: bool, size: u64) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: dynamic,
        min_binding_size: wgpu::BufferSize::new(size),
    }
}
fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}
fn texture(sample_type: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}
const FLOAT: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };
const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
const FRAGMENT: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;
const COMPUTE: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;

impl Renderer {
    fn tp_check_limits(&self) -> Result<(), Error> {
        // Four pass textures, three material textures and twelve lighting textures.
        if self.device.limits().max_sampled_textures_per_shader_stage < 19 {
            return Err(Error::InvalidInput(
                "transparent rendering requires 19 sampled textures per shader stage",
            ));
        }
        Ok(())
    }
    fn tp_layout(
        &self,
        label: &str,
        entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Owned<wgpu::BindGroupLayout> {
        self.ledger.own(
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
            ResourceKind::BindGroupLayout,
        )
    }
    fn tp_pipeline_layout(
        &self,
        label: &str,
        groups: &[&wgpu::BindGroupLayout],
    ) -> Owned<wgpu::PipelineLayout> {
        let groups: Vec<_> = groups.iter().map(|g| Some(*g)).collect();
        self.ledger.own(
            self.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &groups,
                    immediate_size: 0,
                }),
            ResourceKind::PipelineLayout,
        )
    }
    fn tp_module(&self, label: &str, source: &str) -> Owned<wgpu::ShaderModule> {
        self.ledger.own(
            self.device
                .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                }),
            ResourceKind::ShaderModule,
        )
    }
    fn tp_compute(
        &self,
        label: &str,
        layout: &wgpu::PipelineLayout,
        module: &wgpu::ShaderModule,
        entry: &str,
    ) -> Owned<wgpu::ComputePipeline> {
        self.ledger.own(
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(layout),
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            ResourceKind::ComputePipeline,
        )
    }

    /// The pass's fixed resources and kernels, created with its first frame.
    fn tp_gpu(&self) -> Gpu {
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
        let frame = buffer(
            "TP1 frame",
            FRAME_OFFSET + FRAME_BYTES,
            wgpu::BufferUsages::UNIFORM,
        );
        let draws = buffer(
            "TP1 draws",
            SLOT * MAX_TRANSPARENT_DRAWS as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let sampler = self.ledger.own(
            self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("TP1 linear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        let compare = self.ledger.own(
            self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("TP1 shadow compare"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        let shadow = self.tp_target(
            "TP1 sun shadow map",
            wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            4,
        );
        let white_texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("TP1 white sprite"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        self.queue.write_texture(
            white_texture.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white = self.ledger.own(
            white_texture.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let all = vf | COMPUTE;
        let glass_entries = |rt: bool| {
            let mut e = vec![
                entry(0, vf, uniform(false, 80)),
                entry(30, all, uniform(false, FRAME_BYTES)),
                entry(31, FRAGMENT, texture(FLOAT)),
                entry(32, FRAGMENT, texture(UNFILTERED)),
                entry(
                    33,
                    FRAGMENT,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    34,
                    FRAGMENT,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    35,
                    FRAGMENT,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                ),
                entry(37, FRAGMENT, uniform(true, 16)),
                entry(38, FRAGMENT, texture(UNFILTERED)),
            ];
            if rt {
                e.push(entry(
                    2,
                    FRAGMENT,
                    wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
                ));
            }
            e
        };
        let paths: &[bool] = if self.hardware_shadows.is_some() {
            &[false, true]
        } else {
            &[false]
        };
        let glass_bindings: Vec<_> = paths
            .iter()
            .map(|&rt| self.tp_layout("TP1 transparent group 0", &glass_entries(rt)))
            .collect();
        let glass_layouts: Vec<_> = glass_bindings
            .iter()
            .map(|bindings| {
                self.tp_pipeline_layout(
                    "TP1 transparent",
                    &[
                        bindings,
                        &self.instance_bindings,
                        &self.material_bindings,
                        &self.lighting.bindings,
                    ],
                )
            })
            .collect();
        let pane_bindings: Vec<_> = paths
            .iter()
            .map(|&rt| self.tp_layout("TP1 pane group 0", &panes::layout_entries(glass_entries(rt))))
            .collect();
        let pane_layouts: Vec<_> = pane_bindings
            .iter()
            .map(|bindings| {
                self.tp_pipeline_layout(
                    "TP1 pane shading",
                    &[
                        bindings,
                        &self.instance_bindings,
                        &self.material_bindings,
                        &self.lighting.bindings,
                    ],
                )
            })
            .collect();
        // Shadow map: depth only, every opaque instance.
        let shadow_bindings = self.tp_layout(
            "TP1 shadow group 0",
            &[entry(30, all, uniform(false, FRAME_BYTES))],
        );
        let shadow_group = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 shadow"),
                layout: &shadow_bindings,
                entries: &[wgpu::BindGroupEntry {
                    binding: 30,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &frame,
                        offset: FRAME_OFFSET,
                        size: wgpu::BufferSize::new(FRAME_BYTES),
                    }),
                }],
            }),
            ResourceKind::BindGroup,
        );
        let shadow_layout =
            self.tp_pipeline_layout("TP1 shadow", &[&shadow_bindings, &self.instance_bindings]);
        let module = self.tp_module(
            "TP1 shadow",
            &format!("{}{}", section("TP_FRAME"), section("TP_SHADOW")),
        );
        let shadow_pipeline = self.ledger.own(
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("TP1 shadow"),
                    layout: Some(&shadow_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("tp_shadow_vs"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: 48,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                        })],
                    },
                    fragment: None,
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Less),
                        stencil: Default::default(),
                        bias: wgpu::DepthBiasState {
                            constant: 2,
                            slope_scale: 2.0,
                            clamp: 0.0,
                        },
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
            ResourceKind::RenderPipeline,
        );
        let raster_depth_bindings = self.tp_layout(
            "TP1 raster depth",
            &[
                entry(30, all, uniform(false, FRAME_BYTES)),
                entry(
                    52,
                    COMPUTE,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(
                    53,
                    COMPUTE,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
            ],
        );
        let layout = self.tp_pipeline_layout("TP1 raster depth", &[&raster_depth_bindings]);
        let module = self.tp_module(
            "TP1 raster depth",
            &format!("{}{}", section("TP_FRAME"), section("TP_DEPTH_RASTER")),
        );
        let raster_depth = self.tp_compute("TP1 raster depth", &layout, &module, "tp_depth_raster");
        let depth_entry = entry(
            34,
            all,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        );
        let compare_entry = entry(
            35,
            all,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        );
        let sim_bindings = self.tp_layout(
            "TP1 particle simulation",
            &[
                entry(30, all, uniform(false, FRAME_BYTES)),
                depth_entry,
                compare_entry,
                entry(40, COMPUTE, storage(false)),
                entry(41, COMPUTE, storage(false)),
                entry(42, all, uniform(false, EMITTER_BYTES)),
            ],
        );
        let draw_bindings = self.tp_layout(
            "TP1 particle draw",
            &[
                entry(30, all, uniform(false, FRAME_BYTES)),
                entry(32, FRAGMENT, texture(UNFILTERED)),
                entry(
                    33,
                    FRAGMENT,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(36, FRAGMENT, texture(FLOAT)),
                entry(40, wgpu::ShaderStages::VERTEX, storage(true)),
                entry(41, wgpu::ShaderStages::VERTEX, storage(true)),
                entry(42, all, uniform(false, EMITTER_BYTES)),
            ],
        );
        let empty_bindings = self.tp_layout("TP1 empty", &[]);
        let empty = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 empty"),
                layout: &empty_bindings,
                entries: &[],
            }),
            ResourceKind::BindGroup,
        );
        let aa_bindings =
            self.tp_layout("TP2 glass AA seed", &[entry(31, FRAGMENT, texture(FLOAT))]);
        let aa_layout = self.tp_pipeline_layout("TP2 glass AA seed", &[&aa_bindings]);
        let aa_module = self.tp_module("TP2 glass AA seed", section("TP_AA_SEED"));
        let aa_seed = self.ledger.own(
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("TP2 glass AA seed"),
                    layout: Some(&aa_layout),
                    vertex: wgpu::VertexState {
                        module: &aa_module,
                        entry_point: Some("tp_seed_vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &aa_module,
                        entry_point: Some("tp_seed_fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState {
                        count: glass_samples(),
                        ..Default::default()
                    },
                    multiview_mask: None,
                    cache: None,
                }),
            ResourceKind::RenderPipeline,
        );
        Gpu {
            aa_bindings,
            aa_seed,
            frame,
            draws,
            sampler,
            compare,
            shadow,
            white,
            _white: white_texture,
            glass_bindings,
            glass_layouts,
            pane_bindings,
            pane_layouts,
            shadow_group,
            shadow_pipeline,
            raster_depth_bindings,
            raster_depth,
            hit_depth: None,
            sim_bindings,
            draw_bindings,
            particles: None,
            empty,
            empty_bindings,
        }
    }

    fn tp_target(
        &self,
        label: &str,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        texel: u64,
    ) -> Depth {
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(
            texture.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        Depth {
            texture,
            view,
            _allocation: self.ledger.allocate(
                AllocationKind::Target,
                u64::from(size.width) * u64::from(size.height) * texel,
            ),
        }
    }

    /// Particle kernels, composed after the current prelude for its environment helpers.
    fn tp_particle_pipelines(&self) -> ParticlePipelines {
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let [sim, draw] = particle_sources(&self.source);
        let sim = self.tp_module("TP1 particle simulation", &sim);
        let layout = self.tp_pipeline_layout(
            "TP1 particle simulation",
            &[
                &gpu.sim_bindings,
                &gpu.empty_bindings,
                &gpu.empty_bindings,
                &self.lighting.bindings,
            ],
        );
        let update = self.tp_compute("TP1 particle update", &layout, &sim, "tp_particles_update");
        let sort = self.tp_compute("TP1 particle sort", &layout, &sim, "tp_particles_sort");
        let module = self.tp_module("TP1 particle draw", &draw);
        let layout = self.tp_pipeline_layout(
            "TP1 particle draw",
            &[
                &gpu.draw_bindings,
                &gpu.empty_bindings,
                &gpu.empty_bindings,
                &self.lighting.bindings,
            ],
        );
        let draw = self.ledger.own(
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("TP1 particle billboards"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("tp_particle_vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("tp_particle_fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba16Float,
                            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
            ResourceKind::RenderPipeline,
        );
        ParticlePipelines {
            prelude: self.source_digest,
            update,
            sort,
            draw,
        }
    }

    /// The module source of `program`'s transparent pipelines on this renderer's
    /// sun-visibility path, and its identity.
    fn tp_glass_source(&self, program: u64, rt: bool) -> String {
        let slot = self.program_slot(program);
        let head = if rt {
            self.hardware_source.as_deref().unwrap()
        } else {
            &self.source
        };
        let source = glass_source(head, rt, &slot.source, slot.layout.is_some());
        #[cfg(test)]
        let source = {
            let mut source = source;
            if let Some(variant) = &self.transparency.profile {
                if variant.contains("open") && source.contains("fn gi_prepare(") {
                    source = replace_function(
                        &source,
                        "gi_prepare",
                        "fn gi_prepare(input:SurfaceInput) { gi_rx_valid=false; }",
                    );
                }
                if variant.contains("map") {
                    source = replace_function(&source, "hard_shadow", MAP_SHADOW);
                }
                if variant.contains("nofog") {
                    source = replace_function(
                        &source,
                        "atmosphere_surface",
                        "fn atmosphere_surface(c:vec3<f32>, p:vec3<f32>)->vec3<f32> {return c;}",
                    );
                }
            }
            source
        };
        source
    }
    /// The identity of `program`'s transparent pipelines on this renderer's sun-visibility
    /// path, from the digests kept with the sources: the composed source is only built when
    /// the pipelines are (this runs every frame for every transparent program).
    fn tp_glass_key(&self, program: u64, rt: bool) -> [u8; 32] {
        let slot = self.program_slot(program);
        let head = if rt {
            self.hardware_digest.unwrap()
        } else {
            self.source_digest
        };
        let mut hash = Sha256::new();
        hash.update(head);
        hash.update(slot.digest);
        hash.update([u8::from(rt), u8::from(slot.layout.is_some())]);
        #[cfg(test)]
        if let Some(variant) = &self.transparency.profile {
            hash.update(variant.as_bytes());
        }
        hash.finalize().into()
    }
    /// The transparent pipelines of `program` from its module source.
    fn tp_glass_pipelines(
        &self,
        program: u64,
        rt: bool,
        source: &str,
        key: [u8; 32],
    ) -> Result<GlassPipelines, Error> {
        let slot = self.program_slot(program);
        let scopes = OperationScopes::new(&self.device);
        let module = self.ledger.own(
            {
                let label = format!("TP1 transparent {}", slot.label);
                let descriptor = wgpu::ShaderModuleDescriptor {
                    label: Some(&label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                };
                // Program text compiles checked, the standard program included (P-SPLIT).
                self.device.create_program_module(descriptor)
            },
            ResourceKind::ShaderModule,
        );
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let layout = &gpu.glass_layouts[usize::from(rt)];
        let make_with = |module: &wgpu::ShaderModule, tangent: bool, lobe: bool, blend: bool| {
            let targets = if lobe {
                vec![
                    Some(wgpu::TextureFormat::Rgba32Float.into()),
                    Some(wgpu::TextureFormat::Rgba16Float.into()),
                    Some(wgpu::TextureFormat::Rgba16Float.into()),
                ]
            } else {
                vec![Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })]
            };
            self.ledger.own(
                self.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("TP1 transparent"),
                        layout: Some(layout),
                        vertex: wgpu::VertexState {
                            module,
                            entry_point: Some(if tangent { "vs_tangent" } else { "vs" }),
                            compilation_options: Default::default(),
                            buffers: &[
                                Some(wgpu::VertexBufferLayout {
                                    array_stride: 48,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &VERTEX_ATTRIBUTES,
                                }),
                                Some(wgpu::VertexBufferLayout {
                                    array_stride: 4,
                                    step_mode: wgpu::VertexStepMode::Instance,
                                    attributes: &MODEL_ATTRIBUTES,
                                }),
                                tangent.then_some(wgpu::VertexBufferLayout {
                                    array_stride: 16,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &TANGENT_ATTRIBUTES,
                                }),
                            ],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module,
                            entry_point: Some(if lobe { "fs_tp_lobe" } else { "fs_transparent" }),
                            compilation_options: Default::default(),
                            targets: &targets,
                        }),
                        primitive: wgpu::PrimitiveState {
                            cull_mode: None,
                            ..Default::default()
                        },
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState {
                            count: if lobe { 1 } else { glass_samples() },
                            ..Default::default()
                        },
                        multiview_mask: None,
                        cache: None,
                    }),
                ResourceKind::RenderPipeline,
            )
        };
        let make = |tangent: bool, lobe: bool| make_with(&module, tangent, lobe, false);
        let pipelines = [false, true].map(|t| make(t, false));
        let lobe = [false, true].map(|t| make(t, true));
        let films = crate::program::declares_films(&slot.source).then(|| {
            let label = format!("TP1 films {}", slot.label);
            let films = self.ledger.own(
                self.device
                    .create_program_module(wgpu::ShaderModuleDescriptor {
                        label: Some(&label),
                        source: wgpu::ShaderSource::Wgsl(film_source(source).into()),
                    }),
                ResourceKind::ShaderModule,
            );
            [false, true].map(|t| make_with(&films, t, false, true))
        });
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(format!(
                    "program `{}`, transparent pipelines: {error}",
                    slot.label
                )),
                _ => Error::Fault(Fault(error.to_string())),
            });
        }
        Ok(GlassPipelines {
            program,
            key,
            pipelines,
            lobe,
            films,
            pane: None,
        })
    }

    /// Creates a particle emitter. It simulates and draws in every lit frame until the
    /// returned handle drops.
    pub fn try_create_emitter(&mut self, d: &EmitterDescriptor<'_>) -> Result<Emitter, Error> {
        self.check_fault()?;
        self.tp_check_limits()?;
        d.validate()?;
        if d.texture
            .is_some_and(|t| !Arc::ptr_eq(&self.identity, &t.0.renderer))
        {
            return Err(Error::InvalidInput(
                "emitter texture belongs to another renderer",
            ));
        }
        if self.transparency.gpu.is_none() {
            self.transparency.gpu = Some(self.tp_gpu());
        }
        if self.transparency.step == 0.0 {
            self.transparency.step = 1.0 / 60.0;
        }
        let scopes = OperationScopes::new(&self.device);
        let sorted_len = u64::from(d.capacity.next_power_of_two());
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
        // wgpu zero-initializes buffers: every slot starts dead (lifetime 0).
        // COPY_SRC: tests read particles back.
        let particles = buffer(
            "TP1 particles",
            u64::from(d.capacity) * PARTICLE_BYTES,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let sorted = buffer(
            "TP1 particle order",
            sorted_len * 4,
            wgpu::BufferUsages::STORAGE,
        );
        let uniform = buffer("TP1 emitter", EMITTER_BYTES, wgpu::BufferUsages::UNIFORM);
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let sim = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 particle simulation"),
                layout: &gpu.sim_bindings,
                entries: &[
                    frame_entry(&gpu.frame),
                    wgpu::BindGroupEntry {
                        binding: 34,
                        resource: wgpu::BindingResource::TextureView(&gpu.shadow.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 35,
                        resource: wgpu::BindingResource::Sampler(&gpu.compare),
                    },
                    wgpu::BindGroupEntry {
                        binding: 40,
                        resource: particles.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 41,
                        resource: sorted.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 42,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            }),
            ResourceKind::BindGroup,
        );
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        let id = self.transparency.next_emitter;
        self.transparency.next_emitter += 1;
        let shared = Arc::new(EmitterShared {
            id,
            renderer: self.identity.clone(),
        });
        self.transparency.emitters.push(EmitterSlot {
            id,
            handle: Arc::downgrade(&shared),
            params: EmitterParams::new(d),
            sprite: d.texture.cloned(),
            uniform,
            sim,
            particles,
            sorted,
            spawned: 0,
            carry: 0.0,
            clock: 0.0,
            warm_steps: None,
            warm: d.prewarm,
            burst: d.burst,
            _allocation: self.ledger.allocate(
                AllocationKind::Target,
                u64::from(d.capacity) * PARTICLE_BYTES + sorted_len * 4 + EMITTER_BYTES,
            ),
        });
        Ok(Emitter(shared))
    }

    /// Changes a live emitter's parameters; its particles continue. The capacity and
    /// texture presence must stay the same.
    pub fn try_update_emitter(
        &mut self,
        emitter: &Emitter,
        d: &EmitterDescriptor<'_>,
    ) -> Result<(), Error> {
        d.validate()?;
        if !Arc::ptr_eq(&self.identity, &emitter.0.renderer) {
            return Err(Error::InvalidInput("emitter belongs to another renderer"));
        }
        let slot = self
            .transparency
            .emitters
            .iter_mut()
            .find(|s| s.id == emitter.0.id)
            .unwrap();
        if slot.params.capacity != d.capacity || slot.sprite.is_some() != d.texture.is_some() {
            return Err(Error::InvalidInput(
                "emitter updates keep capacity and texture presence",
            ));
        }
        slot.params = EmitterParams::new(d);
        if let Some(texture) = d.texture {
            slot.sprite = Some(texture.clone());
        }
        Ok(())
    }

    /// Particle simulation time per lit frame, seconds (default 1/60). Zero freezes
    /// particles in place.
    pub fn set_particle_step(&mut self, seconds: f32) -> Result<(), Error> {
        if !(seconds.is_finite() && (0.0..=1.0).contains(&seconds)) {
            return Err(Error::InvalidInput("particle step must be in [0, 1] s"));
        }
        // Zero is stored as a tiny step so the default is not reapplied; spawns need dt>0.
        self.transparency.step = if seconds == 0.0 {
            f32::MIN_POSITIVE
        } else {
            seconds
        };
        Ok(())
    }

    pub fn transparency_stats(&self) -> TransparencyStats {
        self.transparency.stats
    }

    /// Visibility and ID publication for transparent draws, as opaque draws get them.
    pub(super) fn tp_prepare_draws(
        &mut self,
        matrix: Mat4,
        transparent: &mut [Draw<'_>],
        uploads: &mut UploadBytes,
    ) -> Result<(), Error> {
        if transparent.iter().any(|d| {
            !Arc::ptr_eq(&self.identity, &d.instances.renderer)
                || !Arc::ptr_eq(&self.identity, &d.mesh.renderer)
        }) {
            return Err(Error::InvalidInput(
                "draw resource belongs to another renderer",
            ));
        }
        if transparent
            .iter()
            .any(|d| d.material.is_some_and(|m| m.normal_mapped) && d.mesh.tangent.is_none())
        {
            return Err(Error::InvalidInput(
                "normal-mapped draw requires tangent stream",
            ));
        }
        let (mut packing, mut upload) = (Duration::ZERO, Duration::ZERO);
        for draw in transparent.iter_mut() {
            let update = draw
                .instances
                .state
                .visibility(matrix, draw.mesh.bounds, draw.visibility);
            if let Some(span) = update.changed_id_span {
                self.scratch.clear();
                for &id in &draw.instances.state.ids[span.clone()] {
                    self.scratch.extend_from_slice(&id.to_ne_bytes());
                }
                self.queue.write_buffer(
                    &draw.instances.id_buffer,
                    span.start as u64 * 4,
                    &self.scratch,
                );
                uploads.visible_ids += self.scratch.len() as u64;
                uploads.visible_id_writes += 1;
                uploads.write_calls += 1;
            }
            self.upload_instance_edits(draw.instances, uploads, &mut packing, &mut upload);
        }
        Ok(())
    }

    /// Encodes the transparent pass into `encoder` after opaque shading and before
    /// presentation. `opaque` are the frame's opaque draws (shadow-map casters).
    pub(super) fn encode_transparency(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        opaque: &[PreparedDraw<'_>],
        transparent: &[Draw<'_>],
        traced: bool,
    ) -> Result<(), Error> {
        self.transparency
            .emitters
            .retain(|e| e.handle.strong_count() > 0);
        let transparent: Vec<&Draw<'_>> = transparent
            .iter()
            .filter(|d| !d.instances.state.ids.is_empty())
            .collect();
        if transparent.is_empty() && self.transparency.emitters.is_empty() {
            return Ok(());
        }
        self.tp_check_limits()?;
        // 4x raster frames have no single-sample depth to read (docs/m7-tp1.md gaps).
        if !traced && self.options.samples != SampleCount::One {
            return Ok(());
        }
        let Some(size) = self.view.hdr.as_ref().map(|h| h.resolved.texture.size()) else {
            return Ok(());
        };
        if self.transparency.gpu.is_none() {
            self.transparency.gpu = Some(self.tp_gpu());
        }
        if self.transparency.step == 0.0 {
            self.transparency.step = 1.0 / 60.0;
        }
        let scopes = OperationScopes::new(&self.device);
        // Targets at the HDR size: the colour behind a transparent draw and opaque depth.
        if self
            .view
            .transparency_targets
            .as_ref()
            .is_none_or(|t| t.color.texture.size() != size)
        {
            let generation = self
                .view
                .transparency_targets
                .as_ref()
                .map_or(0, |t| t.generation + 1);
            let color = self.tp_target(
                "TP1 colour behind",
                size,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                8,
            );
            let depth = self.tp_target(
                "TP1 opaque view depth",
                size,
                wgpu::TextureFormat::R32Float,
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                4,
            );
            let texture = self.ledger.own(
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("TP2 multisampled glass"),
                    size,
                    mip_level_count: 1,
                    sample_count: 4,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = self.ledger.own(
                texture.create_view(&Default::default()),
                ResourceKind::TextureView,
            );
            let aa = Depth {
                texture,
                view,
                _allocation: self.ledger.allocate(
                    AllocationKind::Target,
                    u64::from(size.width) * u64::from(size.height) * 32,
                ),
            };
            let pane_ids = self.tp_pane_pixels().then(|| {
                let texture = self.ledger.own(
                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("TP pane ids"),
                        size,
                        mip_level_count: 1,
                        sample_count: 4,
                        dimension: wgpu::TextureDimension::D2,
                        format: panes::ID_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    }),
                    ResourceKind::Texture,
                );
                let view = self.ledger.own(
                    texture.create_view(&Default::default()),
                    ResourceKind::TextureView,
                );
                Depth {
                    texture,
                    view,
                    _allocation: self.ledger.allocate(
                        AllocationKind::Target,
                        u64::from(size.width) * u64::from(size.height) * 16,
                    ),
                }
            });
            self.view.transparency_targets = Some(Targets {
                color,
                depth,
                aa,
                pane_ids,
                generation,
            });
        }
        // The prelude-composed particle kernels follow prelude reloads.
        let prelude: [u8; 32] = self.source_digest;
        if !self.transparency.emitters.is_empty()
            && self
                .transparency
                .gpu
                .as_ref()
                .unwrap()
                .particles
                .as_ref()
                .is_none_or(|p| p.prelude != prelude)
        {
            let pipelines = self.tp_particle_pipelines();
            self.transparency.gpu.as_mut().unwrap().particles = Some(pipelines);
        }
        if traced && self.transparency.gpu.as_ref().unwrap().hit_depth.is_none() {
            let primary = self.view.primary.as_ref().unwrap();
            let bindings = self.tp_layout(
                "TP1 hit depth",
                &[
                    entry(30, COMPUTE, uniform(false, FRAME_BYTES)),
                    entry(50, COMPUTE, texture(wgpu::TextureSampleType::Uint)),
                    entry(51, COMPUTE, texture(wgpu::TextureSampleType::Uint)),
                    entry(
                        52,
                        COMPUTE,
                        wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                    ),
                ],
            );
            let layout =
                self.tp_pipeline_layout("TP1 hit depth", &[&bindings, primary.array_layout()]);
            let module = self.tp_module("TP1 hit depth", &hit_depth_source());
            let pipeline = self.tp_compute("TP1 hit depth", &layout, &module, "tp_depth_hits");
            self.transparency.gpu.as_mut().unwrap().hit_depth = Some((bindings, pipeline));
        }
        // Transparent programs' pipelines, by program and sun-visibility path.
        let rt = self
            .hardware_shadows
            .as_ref()
            .is_some_and(|h| h.active_scene().is_some());
        let reflect = rt
            && traced
            && transparent.iter().any(|draw| {
                let program = draw.material.map_or(self.standard.id(), |m| m.program.id());
                self.program_slot(program).traits.declares_reflection
            });
        #[cfg(test)]
        let reflect =
            reflect && std::env::var("RENDERLAB_TP2_PROFILE").map_or(true, |s| s.is_empty());
        if reflect {
            let primary = self.view.primary.as_ref().unwrap();
            let program = self.program_slot(primary.program);
            let key = reflection::key(&self.source_digest, &program.digest, program.layout.is_some());
            if self
                .view
                .transparency_reflections
                .as_ref()
                .is_none_or(|r| r.position.texture.size() != size || r.key != key)
            {
                self.view.transparency_reflections = Some(self.tp_reflections(size)?);
            }
            // MP1 integration: R1 shades foreign-program hits with the first opaque
            // program's response and parameters, as MP1's existing fallback does.
            let mut slots = [0u32; crate::TRACED_DRAWS as usize];
            for (slot, draw) in opaque.iter().enumerate() {
                let id = draw.material.map_or(self.standard.id(), |m| m.program.id());
                slots[slot] = u32::from(id != primary.program);
            }
            let bytes: Vec<u8> = slots.iter().flat_map(|s| s.to_ne_bytes()).collect();
            self.queue.write_buffer(
                &self
                    .view
                    .transparency_reflections
                    .as_ref()
                    .unwrap()
                    .program_slots,
                0,
                &bytes,
            );
        }
        let default_program = self.standard.id();
        // Sources are composed once per program and frame; pipelines only when they change
        // (a program or prelude reload, or the TLAS appearing).
        let mut programs: Vec<u64> = transparent
            .iter()
            .map(|d| d.material.map_or(default_program, |m| m.program.id()))
            .collect();
        programs.sort_unstable();
        programs.dedup();
        for program in programs {
            let key = self.tp_glass_key(program, rt);
            if self
                .transparency
                .glass
                .iter()
                .any(|g| g.program == program && g.key == key)
            {
                continue;
            }
            let source = self.tp_glass_source(program, rt);
            let pipelines = match self.tp_glass_pipelines(program, rt, &source, key) {
                Err(Error::Fault(Fault(message))) => return Err(self.fail(message)),
                other => other?,
            };
            self.transparency.glass.retain(|g| g.program != program);
            self.transparency.glass.push(pipelines);
        }
        // Per-pixel panes: the id raster and shading of each program, once its glass set exists.
        let pixel_panes = self.tp_pane_pixels()
            && traced
            && self.device.features().contains(wgpu::Features::PRIMITIVE_INDEX);
        if pixel_panes {
            for at in 0..self.transparency.glass.len() {
                if self.transparency.glass[at].pane.is_some() {
                    continue;
                }
                let program = self.transparency.glass[at].program;
                let source = self.tp_glass_source(program, rt);
                let pane = match self.tp_pane_pipelines(program, rt, &source) {
                    Err(Error::Fault(Fault(message))) => return Err(self.fail(message)),
                    other => other?,
                };
                self.transparency.glass[at].pane = Some(pane);
            }
        }
        // ---- CPU: the frame block, the sun map fit and the draw order ----
        let camera = self.transparency.camera.unwrap();
        let matrix = camera.matrix(size.width, size.height);
        let inverse_view = camera.view().inverse();
        let eye = inverse_view.w_axis.truncate();
        let forward = -inverse_view.z_axis.truncate().normalize();
        let sun = self.lighting.source_lights.iter().find_map(|l| match l {
            crate::Light::Directional { travel, lux } => Some((travel.normalize(), *lux)),
            _ => None,
        });
        let mut receivers: Vec<(Vec3, Vec3)> = Vec::new();
        for draw in &transparent {
            for b in draw.instances.state.world_bounds(draw.mesh.bounds)?.iter() {
                receivers.push((b.min, b.max));
            }
        }
        receivers.extend(self.transparency.emitters.iter().map(|e| e.params.bounds()));
        // What the opaque casters are: each draw's mesh and placements by name (the collection
        // and its edit count), not matrix by matrix (8,195 matrices on Ara's Outpost).
        let mut casters_key = Sha256::new();
        for draw in opaque {
            casters_key.update((Arc::as_ptr(&draw.mesh.vertex) as usize).to_ne_bytes());
            let (id, edits) = draw.instances.state.revision();
            casters_key.update(id.to_ne_bytes());
            casters_key.update(edits.to_ne_bytes());
        }
        let casters_key: [u8; 32] = casters_key.finalize().into();
        // The fit depends on the sun, the receivers and the casters only: reused while those are
        // the same (a moving camera changes none of them).
        let fit_inputs: [u8; 32] = {
            let mut h = Sha256::new();
            if let Some((travel, _)) = sun {
                h.update(bytes_of(&travel.to_array()));
            }
            for (min, max) in &receivers {
                h.update(bytes_of(&min.to_array()));
                h.update(bytes_of(&max.to_array()));
            }
            h.update(casters_key);
            h.finalize().into()
        };
        let fit = match &self.transparency.fit_memo {
            Some((inputs, fit)) if *inputs == fit_inputs => *fit,
            _ => {
                // Every opaque caster's world bounds, in draw then instance order.
                let mut casters: Vec<(Vec3, Vec3)> = Vec::new();
                for draw in opaque {
                    if let Ok(boxes) = draw.instances.state.world_bounds(draw.mesh.bounds) {
                        casters.extend(boxes.iter().map(|b| (b.min, b.max)));
                    }
                }
                let fit = sun.and_then(|(travel, _)| shadow_fit(travel, &receivers, &casters));
                self.transparency.fit_memo = Some((fit_inputs, fit));
                fit
            }
        };
        let shadow = fit.map_or(Mat4::IDENTITY, |f| f.matrix);
        let mut key = Sha256::new();
        key.update(bytes_of(&shadow.to_cols_array()));
        key.update(casters_key);
        let key: [u8; 32] = key.finalize().into();
        let render_shadow = fit.is_some() && self.transparency.shadow_key != Some(key);
        let bytes = frame_block(&FrameInputs {
            camera,
            size: [size.width, size.height],
            sun,
            fit,
            // A later view of the tick moves nothing: its update only relights the
            // particles for its own sun map fit (`tp_incident`).
            step: if self.tick_work {
                self.transparency.step
            } else {
                0.0
            },
            reflect,
            scene: if traced {
                self.view.primary.as_ref().unwrap().scene
            } else {
                [0; 4]
            },
        });
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let mut camera_block = [0u8; 80];
        camera_block[..64].copy_from_slice(&matrix_bytes(matrix));
        camera_block[64..68].copy_from_slice(&(ViewMode::Lit as u32).to_ne_bytes());
        self.queue.write_buffer(&gpu.frame, 0, &camera_block);
        self.queue.write_buffer(&gpu.frame, FRAME_OFFSET, &bytes);
        // Items far to near: transparent draws by their visible instances' mean bounds
        // centre, emitters by their spawn centre.
        enum Item {
            Draw(usize),
            Emitter(usize),
        }
        let mut items: Vec<(f32, Item)> = Vec::new();
        for (i, draw) in transparent.iter().enumerate() {
            let mut centre = Vec3::ZERO;
            for &id in &draw.instances.state.ids {
                let b = shadow_gpu::world_bounds(
                    draw.mesh.bounds,
                    draw.instances.state.models[id as usize],
                )?;
                centre += (b.min + b.max) * 0.5;
            }
            centre /= draw.instances.state.ids.len() as f32;
            items.push(((centre - eye).dot(forward), Item::Draw(i)));
        }
        for (i, e) in self.transparency.emitters.iter().enumerate() {
            items.push(((e.params.position - eye).dot(forward), Item::Emitter(i)));
        }
        items.sort_by(|a, b| b.0.total_cmp(&a.0));
        if transparent.len() > MAX_TRANSPARENT_DRAWS {
            return Err(Error::InvalidInput(
                "more than TRACED_DRAWS transparent draws in one frame",
            ));
        }
        let mut draw_slots = vec![0u8; SLOT as usize * transparent.len().max(1)];
        for (i, draw) in transparent.iter().enumerate() {
            let thickness = draw
                .material
                .and_then(|m| m.transparency)
                .map_or(0.0, |t| t.thickness);
            draw_slots[i * SLOT as usize..i * SLOT as usize + 4]
                .copy_from_slice(&thickness.to_ne_bytes());
        }
        self.queue.write_buffer(&gpu.draws, 0, &draw_slots);
        // Emitters' spawns for this step: once per tick (`Renderer::begin_tick`); a later view
        // of the same tick spawns nothing.
        let step = f64::from(self.transparency.step);
        // A stepped prewarm's substep: the frame's particle step, or 60 Hz while it is 0.
        let substep = if step > 0.0 { step } else { 1.0 / 60.0 };
        for e in &mut self.transparency.emitters {
            let textured = e.sprite.is_some();
            if !self.tick_work {
                let none = e
                    .params
                    .uniform(e.spawned as u32, 0, 0, textured, 0.0, e.clock as f32, 0.0);
                self.queue.write_buffer(&e.uniform, 0, &none);
                continue;
            }
            // A new emitter whose forces or collision the closed-form prewarm cannot place
            // steps its prewarm window instead: each substep spawns its share of the rate
            // and moves every live particle, as a running emitter would have.
            if e.warm > 0.0 && e.params.steps_prewarm() {
                let capacity = u64::from(e.params.capacity);
                let window = f64::from(e.warm.min(e.params.lifetime[1]));
                let count = ((window / substep).ceil() as usize).clamp(1, MAX_PREWARM_STEPS);
                let dt = window / count as f64;
                let mut blobs = Vec::with_capacity((count + 1) * EMITTER_BYTES as usize);
                for i in 0..count {
                    let burst = if i == 0 { u64::from(e.burst).min(capacity) } else { 0 };
                    e.clock += dt;
                    e.carry += f64::from(e.params.spawn_rate) * dt;
                    let births = (e.carry.floor() as u64).min(capacity - burst);
                    e.carry -= e.carry.floor();
                    let first = e.spawned as u32;
                    let spawned = births + burst;
                    e.spawned += spawned;
                    blobs.extend_from_slice(&e.params.uniform(
                        first,
                        spawned as u32,
                        burst as u32,
                        textured,
                        0.0,
                        e.clock as f32,
                        dt as f32,
                    ));
                }
                (e.warm, e.burst) = (0.0, 0);
                e.warm_steps = Some(blobs);
            }
            // A new emitter's first step spawns its prewarm span (no longer than a life:
            // older particles would be dead), spread over that span, plus its burst.
            // The burst is born this step; rate births fill the prewarm window (or the
            // step). Past the ring's capacity only the newest births are kept: the
            // window shrinks to the span they cover, and the spawn ordinal skips the
            // older births as a running ring would have overwritten them.
            let capacity = u64::from(e.params.capacity);
            let burst = u64::from(e.burst).min(capacity);
            let mut window = f64::from(e.warm.min(e.params.lifetime[1]));
            e.clock += if window > 0.0 { window } else { step };
            e.carry += f64::from(e.params.spawn_rate) * if window > 0.0 { window } else { step };
            let births = e.carry.floor() as u64;
            e.carry -= e.carry.floor();
            let kept = births.min(capacity - burst);
            if window > 0.0 && births > kept {
                window *= kept as f64 / births as f64;
                e.spawned += births - kept;
            }
            (e.warm, e.burst) = (0.0, 0);
            let first = e.spawned as u32;
            let spawned = kept + burst;
            e.spawned += spawned;
            let uniform = e.params.uniform(
                first,
                spawned as u32,
                burst as u32,
                textured,
                window as f32,
                e.clock as f32,
                0.0,
            );
            self.queue.write_buffer(&e.uniform, 0, &uniform);
            // The prewarm's substeps overwrite the uniform on the GPU; the frame's own
            // follows them.
            if let Some(blobs) = &mut e.warm_steps {
                blobs.extend_from_slice(&uniform);
            }
        }
        // ---- GPU ----
        let hdr = self.view.hdr.as_ref().unwrap();
        let targets = self.view.transparency_targets.as_ref().unwrap();
        // 1. Opaque view depth.
        {
            let group = if traced {
                let primary = self.view.primary.as_ref().unwrap();
                let t = primary.targets.as_ref().unwrap();
                let (bindings, _) = gpu.hit_depth.as_ref().unwrap();
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP1 hit depth"),
                    layout: bindings,
                    entries: &[
                        frame_entry(&gpu.frame),
                        wgpu::BindGroupEntry {
                            binding: 50,
                            resource: wgpu::BindingResource::TextureView(&t.hit.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 51,
                            resource: wgpu::BindingResource::TextureView(&t.tiles.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 52,
                            resource: wgpu::BindingResource::TextureView(&targets.depth.view),
                        },
                    ],
                });
                Some(group)
            } else {
                let depth_view = self
                    .view
                    .depth
                    .as_ref()
                    .unwrap()
                    .texture
                    .create_view(&Default::default());
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP1 raster depth"),
                    layout: &gpu.raster_depth_bindings,
                    entries: &[
                        frame_entry(&gpu.frame),
                        wgpu::BindGroupEntry {
                            binding: 52,
                            resource: wgpu::BindingResource::TextureView(&targets.depth.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 53,
                            resource: wgpu::BindingResource::TextureView(&depth_view),
                        },
                    ],
                }))
            };
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("TP1 opaque depth"),
                timestamp_writes: None,
            });
            if traced {
                let arrays = self
                    .view
                    .primary
                    .as_ref()
                    .unwrap()
                    .virtual_geometry()
                    .unwrap()
                    .1;
                pass.set_pipeline(&gpu.hit_depth.as_ref().unwrap().1);
                pass.set_bind_group(1, arrays, &[]);
            } else {
                pass.set_pipeline(&gpu.raster_depth);
            }
            pass.set_bind_group(0, group.as_ref().unwrap(), &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        }
        // 2. The transparent receivers' sun shadow map, when its inputs changed.
        if render_shadow {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TP1 sun shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.shadow.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&gpu.shadow_pipeline);
            pass.set_bind_group(0, &*gpu.shadow_group, &[]);
            for draw in opaque {
                pass.set_bind_group(1, &*draw.instances.bind, &[]);
                pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..draw.mesh.index_count, 0, 0..draw.instances.len() as u32);
            }
        }
        // 3. Particles: stepped prewarms first (each substep's uniform copied in, then
        // simulated), then this frame's simulation, lighting and sort.
        if let Some(p) = &gpu.particles {
            for e in &mut self.transparency.emitters {
                let Some(blobs) = e.warm_steps.take() else {
                    continue;
                };
                let staging = wgpu::util::DeviceExt::create_buffer_init(
                    &*self.device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("TP1 stepped prewarm"),
                        contents: &blobs,
                        usage: wgpu::BufferUsages::COPY_SRC,
                    },
                );
                let steps = blobs.len() as u64 / EMITTER_BYTES;
                for i in 0..steps {
                    encoder.copy_buffer_to_buffer(
                        &staging,
                        i * EMITTER_BYTES,
                        &e.uniform,
                        0,
                        EMITTER_BYTES,
                    );
                    // The last blob is the frame's own uniform, simulated below.
                    if i + 1 == steps {
                        break;
                    }
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("TP1 prewarm step"),
                        timestamp_writes: None,
                    });
                    pass.set_bind_group(0, &*e.sim, &[]);
                    pass.set_bind_group(1, &*gpu.empty, &[]);
                    pass.set_bind_group(2, &*gpu.empty, &[]);
                    pass.set_bind_group(3, self.lighting.bind(), &[]);
                    pass.set_pipeline(&p.update);
                    pass.dispatch_workgroups(e.params.capacity.div_ceil(64), 1, 1);
                }
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("TP1 particles"),
                timestamp_writes: None,
            });
            pass.set_bind_group(1, &*gpu.empty, &[]);
            pass.set_bind_group(2, &*gpu.empty, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            for e in &self.transparency.emitters {
                pass.set_bind_group(0, &*e.sim, &[]);
                pass.set_pipeline(&p.update);
                pass.dispatch_workgroups(e.params.capacity.div_ceil(64), 1, 1);
                pass.set_pipeline(&p.sort);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        // Per-frame groups: the TLAS, targets and shadow map they bind change between frames.
        let rt_scene = self
            .hardware_shadows
            .as_ref()
            .and_then(|h| h.active_scene())
            .filter(|_| rt);
        let mut glass_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &gpu.frame,
                    offset: 0,
                    size: wgpu::BufferSize::new(80),
                }),
            },
            frame_entry(&gpu.frame),
            wgpu::BindGroupEntry {
                binding: 31,
                resource: wgpu::BindingResource::TextureView(&targets.color.view),
            },
            wgpu::BindGroupEntry {
                binding: 32,
                resource: wgpu::BindingResource::TextureView(&targets.depth.view),
            },
            wgpu::BindGroupEntry {
                binding: 33,
                resource: wgpu::BindingResource::Sampler(&gpu.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 34,
                resource: wgpu::BindingResource::TextureView(&gpu.shadow.view),
            },
            wgpu::BindGroupEntry {
                binding: 35,
                resource: wgpu::BindingResource::Sampler(&gpu.compare),
            },
            wgpu::BindGroupEntry {
                binding: 38,
                resource: wgpu::BindingResource::TextureView(
                    self.view
                        .transparency_reflections
                        .as_ref()
                        .filter(|_| reflect)
                        .map_or(&targets.color.view, |r| &r.radiance.view),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 37,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &gpu.draws,
                    offset: 0,
                    size: wgpu::BufferSize::new(16),
                }),
            },
        ];
        if let Some(scene) = rt_scene {
            glass_entries.push(wgpu::BindGroupEntry {
                binding: 2,
                resource: scene.as_binding(),
            });
        }
        let glass_group = (!transparent.is_empty()).then(|| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 transparent group 0"),
                layout: &gpu.glass_bindings[usize::from(rt)],
                entries: &glass_entries,
            })
        });
        // TP2: reflection hits use the existing opaque geometry, materials and R1 shader.
        let reflection_group = reflect.then(|| {
            let reflection = self.view.transparency_reflections.as_ref().unwrap();
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP2 glass reflections"),
                layout: &reflection.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &gpu.frame,
                            offset: 0,
                            size: wgpu::BufferSize::new(80),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: rt_scene.unwrap().as_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: self
                            .view
                            .primary
                            .as_ref()
                            .unwrap()
                            .frame_buffer()
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 18,
                        resource: wgpu::BindingResource::TextureView(&reflection.radiance.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 31,
                        resource: wgpu::BindingResource::TextureView(&reflection.position.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 32,
                        resource: wgpu::BindingResource::TextureView(&reflection.normal.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 33,
                        resource: wgpu::BindingResource::TextureView(&reflection.geometric.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 38,
                        resource: reflection.program_slots.as_entire_binding(),
                    },
                ],
            })
        });
        let draw_groups: Vec<wgpu::BindGroup> = self
            .transparency
            .emitters
            .iter()
            .map(|e| {
                let sprite = e
                    .sprite
                    .as_ref()
                    .map_or(&*gpu.white, |t| self.texture_view(t));
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP1 particle draw"),
                    layout: &gpu.draw_bindings,
                    entries: &[
                        frame_entry(&gpu.frame),
                        wgpu::BindGroupEntry {
                            binding: 32,
                            resource: wgpu::BindingResource::TextureView(&targets.depth.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 33,
                            resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 36,
                            resource: wgpu::BindingResource::TextureView(sprite),
                        },
                        wgpu::BindGroupEntry {
                            binding: 40,
                            resource: e.particles.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 41,
                            resource: e.sorted.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 42,
                            resource: e.uniform.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        // Per-pixel panes (`panes`): a record (rectangle and model slot) per instance of each
        // glass draw, at its dynamic offset, and a pane group per draw. Draws with a tangent
        // stream keep the raster: the pane shading builds vertices without tangents.
        // Films (IOR one, no thickness) of programs that declare them blend in hardware:
        // consecutive films share one pass, each draw's instances far to near (`film_ids`).
        let films: Vec<bool> = transparent
            .iter()
            .map(|draw| {
                let program = draw.material.map_or(default_program, |m| m.program.id());
                draw.material
                    .and_then(|m| m.transparency)
                    .is_some_and(|t| t.ior <= 1.0 && t.thickness == 0.0)
                    && self
                        .transparency
                        .glass
                        .iter()
                        .any(|g| g.program == program && g.films.is_some())
            })
            .collect();
        let mut pane_records: Vec<Vec<(u32, [u32; 4])>> = vec![Vec::new(); transparent.len()];
        let mut pane_groups: Vec<Option<wgpu::BindGroup>> =
            (0..transparent.len()).map(|_| None).collect();
        let mut pane_bytes = Vec::new();
        if pixel_panes && targets.pane_ids.is_some() {
            for (i, draw) in transparent.iter().enumerate() {
                if films[i] || draw.mesh.tangent.is_some() {
                    continue;
                }
                let state = &draw.instances.state;
                for (model, slot) in state.models.iter().zip(&state.ids) {
                    let rect = panes::rect(&draw.mesh.bounds, *model, matrix, [size.width, size.height]);
                    let at = pane_bytes.len();
                    pane_bytes.extend(panes::record(rect, *slot));
                    pane_bytes.resize(at + panes::RECORD as usize, 0);
                    pane_records[i].push((at as u32, rect));
                }
            }
        }
        let pane_buffer = (!pane_bytes.is_empty()).then(|| {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("TP pane records"),
                size: pane_bytes.len() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, &pane_bytes);
            buffer
        });
        if let (Some(buffer), Some(ids)) = (&pane_buffer, &targets.pane_ids) {
            for (i, draw) in transparent.iter().enumerate() {
                if pane_records[i].is_empty() {
                    continue;
                }
                let mut entries = glass_entries.clone();
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 70,
                        resource: draw.mesh.vertex.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 71,
                        resource: draw.mesh.index.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 72,
                        resource: wgpu::BindingResource::TextureView(&ids.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 76,
                        resource: wgpu::BindingResource::TextureView(&hdr.resolved.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 77,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(panes::RECORD_BYTES),
                        }),
                    },
                ]);
                pane_groups[i] = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP pane group 0"),
                    layout: &gpu.pane_bindings[usize::from(rt)],
                    entries: &entries,
                }));
            }
        }
        // 4. Back to front. Each transparent draw first copies the colour behind it.
        let hdr_view = &hdr.resolved.view;
        let aa_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TP2 glass AA seed"),
            layout: &gpu.aa_bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 31,
                resource: wgpu::BindingResource::TextureView(&targets.color.view),
            }],
        });
        // The colour behind a draw is the HDR target as the draws before it left it. After the
        // first full copy, only what those draws wrote can differ, so later draws copy just that
        // region (`dirty`, the union of their conservative screen rectangles; the whole view for
        // particles and for a draw with a corner behind the eye). The same bytes reach every
        // pixel any draw reads, refraction offsets included (audit 2026-11-12, 3.2: a full-frame
        // copy per transparent draw).
        let view_rect = [0, 0, size.width, size.height];
        let union = |a: [u32; 4], b: [u32; 4]| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
        let draw_rects: Vec<[u32; 4]> = transparent
            .iter()
            .map(|draw| {
                let state = &draw.instances.state;
                state
                    .models
                    .iter()
                    .map(|model| panes::rect(&draw.mesh.bounds, *model, matrix, [size.width, size.height]))
                    .reduce(union)
                    .unwrap_or([0, 0, 0, 0])
            })
            .collect();
        // A film draw's instances far to near, so overlapping instances blend in order.
        let film_ids: Vec<Option<wgpu::Buffer>> = transparent
            .iter()
            .enumerate()
            .map(|(i, draw)| {
                let state = &draw.instances.state;
                (films[i] && state.ids.len() > 1).then(|| {
                    let depth = |k: usize| (state.models[k].w_axis.truncate() - eye).dot(forward);
                    let mut order: Vec<usize> = (0..state.ids.len()).collect();
                    order.sort_by(|a, b| depth(*b).total_cmp(&depth(*a)));
                    let ids: Vec<u32> = order.iter().map(|&k| state.ids[k]).collect();
                    wgpu::util::DeviceExt::create_buffer_init(
                        &*self.device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("TP film instance order"),
                            contents: bytemuck::cast_slice(&ids),
                            usage: wgpu::BufferUsages::VERTEX,
                        },
                    )
                })
            })
            .collect();
        let film_at = |k: usize| matches!(items.get(k).map(|item| &item.1), Some(Item::Draw(j)) if films[*j]);
        let mut copied = false;
        let mut dirty: Option<[u32; 4]> = None;
        let mut index = 0;
        while index < items.len() {
            let glass = matches!(items[index].1, Item::Draw(_));
            let aa = glass && glass_samples() == 4;
            if glass {
                let region = if copied { dirty } else { Some(view_rect) };
                if let Some(r) = region.filter(|r| r[2] > r[0] && r[3] > r[1]) {
                    let origin = wgpu::Origin3d { x: r[0], y: r[1], z: 0 };
                    let mut from = hdr.resolved.texture.as_image_copy();
                    from.origin = origin;
                    let mut to = targets.color.texture.as_image_copy();
                    to.origin = origin;
                    let extent = wgpu::Extent3d {
                        width: r[2] - r[0],
                        height: r[3] - r[1],
                        depth_or_array_layers: 1,
                    };
                    encoder.copy_texture_to_texture(from, to, extent);
                }
                copied = true;
                dirty = None;
                if let (Item::Draw(first), Some(group)) = (&items[index].1, &reflection_group) {
                    let reflection = self.view.transparency_reflections.as_ref().unwrap();
                    // A run of films shares one lobe pass and one trace: where films overlap,
                    // each reads the reflection of the one drawn last there.
                    let run: Vec<usize> = if films[*first] {
                        (index..items.len())
                            .take_while(|&k| film_at(k))
                            .map(|k| match items[k].1 {
                                Item::Draw(j) => j,
                                Item::Emitter(_) => unreachable!("film_at"),
                            })
                            .collect()
                    } else {
                        vec![*first]
                    };
                    {
                        let attachments = [
                            &reflection.position.view,
                            &reflection.normal.view,
                            &reflection.geometric.view,
                        ]
                        .map(|view| {
                            Some(wgpu::RenderPassColorAttachment {
                                view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })
                        });
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("TP2 glass reflection lobe"),
                            color_attachments: &attachments,
                            ..Default::default()
                        });
                        pass.set_bind_group(3, self.lighting.bind(), &[]);
                        for &i in &run {
                            let draw = transparent[i];
                            let program = draw.material.map_or(default_program, |m| m.program.id());
                            let set = self
                                .transparency
                                .glass
                                .iter()
                                .find(|g| g.program == program)
                                .unwrap();
                            pass.set_pipeline(&set.lobe[usize::from(draw.mesh.tangent.is_some())]);
                            pass.set_bind_group(
                                0,
                                glass_group.as_ref().unwrap(),
                                &[(i as u64 * SLOT) as u32],
                            );
                            pass.set_bind_group(1, &*draw.instances.bind, &[]);
                            pass.set_bind_group(2, &*draw.material.unwrap().bind, &[]);
                            pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                            pass.set_vertex_buffer(1, match &film_ids[i] {
                                Some(order) => order.slice(..),
                                None => draw.instances.id_buffer.slice(..),
                            });
                            if let Some(t) = &draw.mesh.tangent {
                                pass.set_vertex_buffer(2, t.slice(..));
                            }
                            pass.set_index_buffer(
                                draw.mesh.index.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(
                                0..draw.mesh.index_count,
                                0,
                                0..draw.instances.state.ids.len() as u32,
                            );
                        }
                    }
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("TP2 R1 glass reflection"),
                        timestamp_writes: None,
                    });
                    let (arrays, materials) = self
                        .view
                        .primary
                        .as_ref()
                        .unwrap()
                        .shading_groups()
                        .unwrap();
                    pass.set_pipeline(&reflection.pipeline);
                    pass.set_bind_group(0, group, &[]);
                    pass.set_bind_group(1, arrays, &[]);
                    pass.set_bind_group(2, materials, &[]);
                    pass.set_bind_group(3, self.lighting.bind(), &[]);
                    pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
                }
            }
            // Per-pixel panes: each instance in draw order, its ids then its shading over what
            // the previous instance left; the first one's colour behind was copied above.
            let pane_group = match &items[index].1 {
                Item::Draw(i) => pane_groups[*i].as_ref().map(|group| (*i, group)),
                Item::Emitter(_) => None,
            };
            if let Some((i, group)) = pane_group {
                let draw = transparent[i];
                let program = draw.material.map_or(default_program, |m| m.program.id());
                let set = self
                    .transparency
                    .glass
                    .iter()
                    .find(|g| g.program == program)
                    .unwrap();
                let pane = set.pane.as_ref().unwrap();
                let material = draw
                    .material
                    .unwrap_or(self.default_material.as_ref().unwrap());
                let ids = targets.pane_ids.as_ref().unwrap();
                for (k, &(at, rect)) in pane_records[i].iter().enumerate() {
                    if rect[2] <= rect[0] || rect[3] <= rect[1] {
                        continue;
                    }
                    if k > 0 {
                        let origin = wgpu::Origin3d { x: rect[0], y: rect[1], z: 0 };
                        let extent = wgpu::Extent3d {
                            width: rect[2] - rect[0],
                            height: rect[3] - rect[1],
                            depth_or_array_layers: 1,
                        };
                        let mut from = hdr.resolved.texture.as_image_copy();
                        from.origin = origin;
                        let mut to = targets.color.texture.as_image_copy();
                        to.origin = origin;
                        encoder.copy_texture_to_texture(from, to, extent);
                    }
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("TP pane ids"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &ids.view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        });
                        pass.set_pipeline(&pane.ids[0]);
                        pass.set_bind_group(0, glass_group.as_ref().unwrap(), &[(i as u64 * SLOT) as u32]);
                        pass.set_bind_group(1, &*draw.instances.bind, &[]);
                        pass.set_bind_group(2, &*material.bind, &[]);
                        pass.set_bind_group(3, self.lighting.bind(), &[]);
                        pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                        pass.set_vertex_buffer(1, draw.instances.id_buffer.slice(..));
                        pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..draw.mesh.index_count, 0, k as u32..k as u32 + 1);
                    }
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("TP pane shading"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pane.shade);
                    pass.set_bind_group(0, group, &[(i as u64 * SLOT) as u32, at]);
                    pass.set_bind_group(1, &*draw.instances.bind, &[]);
                    pass.set_bind_group(2, &*material.bind, &[]);
                    pass.set_bind_group(3, self.lighting.bind(), &[]);
                    pass.dispatch_workgroups(
                        (rect[2] - rect[0]).div_ceil(8),
                        (rect[3] - rect[1]).div_ceil(8),
                        1,
                    );
                }
                dirty = Some(dirty.map_or(draw_rects[i], |d| union(d, draw_rects[i])));
                index += 1;
                continue;
            }
            // A glass draw's instances share one pass until one overlaps an instance already
            // in it: each span then draws over what the spans before it left, as per-pixel panes
            // and the ara-gpu port's raster do. In one pass a front pane overwrote the pane
            // behind it with the colour from before the draw.
            let spans = match items[index].1 {
                Item::Draw(i)
                    if glass
                        && !films[i]
                        && transparent[i].instances.state.models.len() <= SPAN_INSTANCES =>
                {
                    let draw = transparent[i];
                    let screen = [size.width, size.height];
                    let rects: Vec<_> = (draw.instances.state.models.iter())
                        .map(|m| panes::rect(&draw.mesh.bounds, *m, matrix, screen))
                        .collect();
                    instance_spans(&rects)
                }
                _ => vec![(0..u32::MAX, view_rect)],
            };
            let first = index;
            for (s, (span, _)) in spans.iter().enumerate() {
                index = first;
                if s > 0 {
                    let r = spans[s - 1].1;
                    if r[2] > r[0] && r[3] > r[1] {
                        let origin = wgpu::Origin3d {
                            x: r[0],
                            y: r[1],
                            z: 0,
                        };
                        let mut from = hdr.resolved.texture.as_image_copy();
                        from.origin = origin;
                        let mut to = targets.color.texture.as_image_copy();
                        to.origin = origin;
                        let extent = wgpu::Extent3d {
                            width: r[2] - r[0],
                            height: r[3] - r[1],
                            depth_or_array_layers: 1,
                        };
                        encoder.copy_texture_to_texture(from, to, extent);
                    }
                }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("TP1 transparent"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: if aa { &targets.aa.view } else { hdr_view },
                        depth_slice: None,
                        resolve_target: aa.then_some(hdr_view),
                        ops: wgpu::Operations {
                            load: if aa {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: if aa {
                                wgpu::StoreOp::Discard
                            } else {
                                wgpu::StoreOp::Store
                            },
                        },
                    })],
                    ..Default::default()
                });
                if aa {
                    pass.set_pipeline(&gpu.aa_seed);
                    pass.set_bind_group(0, &aa_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                pass.set_bind_group(3, self.lighting.bind(), &[]);
                loop {
                    // What this pass writes: the draw's rectangle, the whole view for particles.
                    let written = match items[index].1 {
                        Item::Draw(i) => draw_rects[i],
                        Item::Emitter(_) => view_rect,
                    };
                    dirty = Some(dirty.map_or(written, |d| union(d, written)));
                    match items[index].1 {
                        Item::Draw(i) => {
                            let draw = transparent[i];
                            let program = draw.material.map_or(default_program, |m| m.program.id());
                            let set = self
                                .transparency
                                .glass
                                .iter()
                                .find(|g| g.program == program)
                                .unwrap();
                            let material = draw
                                .material
                                .unwrap_or(self.default_material.as_ref().unwrap());
                            let pipelines = match (&set.films, films[i]) {
                                (Some(films), true) => films,
                                _ => &set.pipelines,
                            };
                            pass.set_pipeline(&pipelines[usize::from(draw.mesh.tangent.is_some())]);
                            pass.set_bind_group(
                                0,
                                glass_group.as_ref().unwrap(),
                                &[(i as u64 * SLOT) as u32],
                            );
                            pass.set_bind_group(1, &*draw.instances.bind, &[]);
                            pass.set_bind_group(2, &*material.bind, &[]);
                            pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                            pass.set_vertex_buffer(1, match &film_ids[i] {
                                Some(order) => order.slice(..),
                                None => draw.instances.id_buffer.slice(..),
                            });
                            if let Some(t) = &draw.mesh.tangent {
                                pass.set_vertex_buffer(2, t.slice(..));
                            }
                            pass.set_index_buffer(
                                draw.mesh.index.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            let count = draw.instances.state.ids.len() as u32;
                            pass.draw_indexed(
                                0..draw.mesh.index_count,
                                0,
                                span.start.min(count)..span.end.min(count),
                            );
                        }
                        Item::Emitter(i) => {
                            let e = &self.transparency.emitters[i];
                            pass.set_pipeline(&gpu.particles.as_ref().unwrap().draw);
                            pass.set_bind_group(0, &draw_groups[i], &[]);
                            pass.set_bind_group(1, &*gpu.empty, &[]);
                            pass.set_bind_group(2, &*gpu.empty, &[]);
                            pass.draw(0..6, 0..e.params.capacity);
                        }
                    }
                    index += 1;
                    // A transparent draw next needs the colour this pass has written so far,
                    // unless this pass and the next draw are both films: they blend over it.
                    let films_on = film_at(index - 1) && film_at(index);
                    if index == items.len()
                        || (glass && !films_on)
                        || (!glass && matches!(items[index].1, Item::Draw(_)))
                    {
                        break;
                    }
                }
            }
        }
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        if render_shadow {
            self.transparency.shadow_key = Some(key);
            self.transparency.stats.shadow_map_renders += 1;
        }
        let stats = &mut self.transparency.stats;
        stats.transparent_draws = transparent.len() as u32;
        stats.emitters = self.transparency.emitters.len() as u32;
        stats.particle_capacity = self
            .transparency
            .emitters
            .iter()
            .map(|e| e.params.capacity)
            .sum();
        stats.frames += 1;
        Ok(())
    }
}

/// Instances above which a glass draw is not split into spans (see `instance_spans`).
const SPAN_INSTANCES: usize = 64;
/// Spans above which a glass draw is drawn in one pass instead.
const MAX_SPANS: usize = 4;

/// A glass draw's instances in draw order, split into spans that one raster pass can draw
/// together: a span ends before the first instance whose pixel rectangle (`panes::rect`)
/// overlaps one already in it. Each span comes with the union of its rectangles, which the
/// next span copies from the HDR target as the colour behind it.
///
/// Each span is a render pass with a region copy and, under MSAA, a full-target seed and
/// resolve. So a draw with more than `SPAN_INSTANCES` instances (foliage, fences) or more
/// than `MAX_SPANS` spans is drawn in one pass, as before RL#72. Unbounded, Ara Outpost RT
/// went from 35 to 163 ms a frame (2026-11-13).
fn instance_spans(rects: &[[u32; 4]]) -> Vec<(std::ops::Range<u32>, [u32; 4])> {
    let overlaps =
        |a: &[u32; 4], b: &[u32; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    let cover = |rects: &[[u32; 4]]| {
        let union = |a: [u32; 4], b: [u32; 4]| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        };
        rects.iter().copied().reduce(union).unwrap_or_default()
    };
    let one = || vec![(0..rects.len() as u32, cover(rects))];
    if rects.len() > SPAN_INSTANCES {
        return one();
    }
    let mut spans = Vec::new();
    let mut start = 0;
    for (k, rect) in rects.iter().enumerate() {
        if rects[start..k].iter().any(|r| overlaps(r, rect)) {
            if spans.len() + 1 == MAX_SPANS {
                return one();
            }
            spans.push((start as u32..k as u32, cover(&rects[start..k])));
            start = k;
        }
    }
    let cover = cover(&rects[start..]);
    spans.push((start as u32..rects.len() as u32, cover));
    spans
}

fn frame_entry(frame: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding: 30,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: frame,
            offset: FRAME_OFFSET,
            size: wgpu::BufferSize::new(FRAME_BYTES),
        }),
    }
}

fn bytes_of(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// What `TpFrame` is packed from.
pub(crate) struct FrameInputs {
    /// The frame's unjittered camera, and the HDR target's size.
    pub(crate) camera: Camera,
    pub(crate) size: [u32; 2],
    /// The first directional light: its unit travel direction and its lux.
    pub(crate) sun: Option<(Vec3, [f32; 3])>,
    /// The sun map's fit over the transparent receivers, when a sun and receivers exist.
    pub(crate) fit: Option<ShadowFit>,
    /// The particle simulation's step in seconds.
    pub(crate) step: f32,
    /// R1 reflections run for this frame's transparent draws.
    pub(crate) reflect: bool,
    /// The traced frame's `scene_pixels` rectangle; zero on a raster frame.
    pub(crate) scene: [u32; 4],
}

/// WGSL `TpFrame`, `FRAME_BYTES` long: the camera matrix and its inverse, the sun map's
/// matrix, the eye and the view's axes, the size and its reciprocal, the direction toward
/// the sun (w: a map exists), its lux (w: the map's texel), then at float 80 the step, the
/// map's bias and size and whether reflections run, and at byte 304 the scene rectangle.
/// The wgpu renderer and the ara-gpu backend both pack through it.
pub(crate) fn frame_block(i: &FrameInputs) -> Vec<u8> {
    let matrix = i.camera.matrix(i.size[0], i.size[1]);
    let inverse_view = i.camera.view().inverse();
    let eye = inverse_view.w_axis.truncate();
    let forward = -inverse_view.z_axis.truncate().normalize();
    let right = inverse_view.x_axis.truncate().normalize();
    let up = inverse_view.y_axis.truncate().normalize();
    let (shadow, bias, texel) = i
        .fit
        .map_or((Mat4::IDENTITY, 0.0, 0.0), |f| (f.matrix, f.bias, f.texel));
    let [width, height] = i.size;
    let mut frame = [0f32; (FRAME_BYTES / 4) as usize];
    frame[0..16].copy_from_slice(&matrix.to_cols_array());
    frame[16..32].copy_from_slice(&matrix.inverse().to_cols_array());
    frame[32..48].copy_from_slice(&shadow.to_cols_array());
    frame[48..52].copy_from_slice(&eye.extend(1.0).to_array());
    frame[52..56].copy_from_slice(&forward.extend(0.0).to_array());
    frame[56..60].copy_from_slice(&right.extend(0.0).to_array());
    frame[60..64].copy_from_slice(&up.extend(0.0).to_array());
    frame[64..68].copy_from_slice(&[
        width as f32,
        height as f32,
        1.0 / width as f32,
        1.0 / height as f32,
    ]);
    let (toward, lux) = i.sun.map_or((Vec3::Z, [0.0; 3]), |(t, l)| (-t, l));
    frame[68..72].copy_from_slice(
        &toward
            .extend(f32::from(u8::from(i.fit.is_some())))
            .to_array(),
    );
    frame[72..76].copy_from_slice(&[lux[0], lux[1], lux[2], texel]);
    frame[80..84].copy_from_slice(&[
        i.step,
        bias,
        SHADOW_SIZE as f32,
        f32::from(u8::from(i.reflect)),
    ]);
    let mut bytes = vec![0u8; FRAME_BYTES as usize];
    for (dst, v) in bytes.chunks_exact_mut(4).zip(frame) {
        dst.copy_from_slice(&v.to_ne_bytes());
    }
    for (k, v) in i.scene.into_iter().enumerate() {
        bytes[304 + 4 * k..308 + 4 * k].copy_from_slice(&v.to_ne_bytes());
    }
    bytes
}

/// The sun map's fit: world to its clip space, the depth bias and one texel's size in
/// metres.
#[derive(Clone, Copy)]
pub(crate) struct ShadowFit {
    pub(crate) matrix: Mat4,
    pub(crate) bias: f32,
    pub(crate) texel: f32,
}
/// An orthographic sun map over the receivers' footprint, deep enough toward the sun to
/// hold every opaque caster: square texels, depth 0 at the most sunward caster.
/// `receivers` and `casters` are world bounds (least and greatest corner).
pub(crate) fn shadow_fit(
    travel: Vec3,
    receivers: &[(Vec3, Vec3)],
    casters: &[(Vec3, Vec3)],
) -> Option<ShadowFit> {
    if receivers.is_empty() || !travel.is_finite() {
        return None;
    }
    let w = travel;
    let helper = if w.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let u = helper.cross(w).normalize();
    let v = w.cross(u);
    let corners = |(min, max): (Vec3, Vec3)| {
        (0..8).map(move |i| {
            Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        })
    };
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for &r in receivers {
        for p in corners(r) {
            let q = Vec3::new(p.dot(u), p.dot(v), p.dot(w));
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    let mut near = lo.z;
    for &caster in casters {
        for p in corners(caster) {
            near = near.min(p.dot(w));
        }
    }
    let margin = 0.25;
    let extent = (hi.x - lo.x).max(hi.y - lo.y) + 2.0 * margin;
    let centre = [(lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5];
    let near = near.max(lo.z - 2000.0) - margin;
    let far = hi.z + margin;
    let depth = (far - near).max(1e-3);
    let half = extent * 0.5;
    let rows = [
        (u / half).extend(-centre[0] / half),
        (v / half).extend(-centre[1] / half),
        (w / depth).extend(-near / depth),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    ];
    let texel = extent / SHADOW_SIZE as f32;
    Some(ShadowFit {
        matrix: Mat4::from_cols(rows[0], rows[1], rows[2], rows[3]).transpose(),
        bias: 1.5 * texel / depth,
        texel,
    })
}

/// Moves transparent draws after opaque ones, keeping each group's order. Returns the
/// opaque count and, when anything moved, each position's original index.
pub(super) fn split(draws: &mut [Draw<'_>]) -> (usize, Option<Vec<usize>>) {
    let transparent = |d: &Draw<'_>| d.material.is_some_and(|m| m.transparency.is_some());
    let opaque = draws.iter().filter(|d| !transparent(d)).count();
    if draws[..opaque].iter().all(|d| !transparent(d)) {
        return (opaque, None);
    }
    let mut order: Vec<usize> = (0..draws.len()).collect();
    order.sort_by_key(|&i| transparent(&draws[i]));
    apply(draws, &order);
    (opaque, Some(order))
}
/// Restores the caller's order after `split`.
pub(super) fn restore(draws: &mut [Draw<'_>], order: &[usize]) {
    let mut inverse = vec![0; order.len()];
    for (position, &original) in order.iter().enumerate() {
        inverse[original] = position;
    }
    apply(draws, &inverse);
}
/// Reorders in place so position k holds the element at `order[k]`.
fn apply(draws: &mut [Draw<'_>], order: &[usize]) {
    let mut done = vec![false; order.len()];
    for start in 0..order.len() {
        if done[start] {
            continue;
        }
        let mut k = start;
        loop {
            done[k] = true;
            let next = order[k];
            if next == start {
                break;
            }
            draws.swap(k, next);
            // After the swap, position `next` holds what position k held before.
            k = next;
        }
    }
}
