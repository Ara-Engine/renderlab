//! Traced primary visibility on hardware-shadow 1x renderers: one closest-hit camera
//! ray per pixel against the shadow TLAS, exact fresh receivers from the hit triangle,
//! and deferred compute shading with each material's shading program. Draw geometry and
//! materials are descriptor-indexed, so each stage is one dispatch over the image.
//! Trace and visibility kernels hold no program code and compile once per prelude; each
//! program compiles only its own shading modules (`shading_heads`).
// PC1: only the device type changes; pipeline creation uses the shared cache.
use super::*;
use crate::pipeline_compilation::PipelineDevice;
use crate::primary_ray::{
    TRACED_DRAWS, TRACED_FEATURES, batch_word, placement_table, placement_words, traced_limits,
};
use crate::shadow_plan::Bounds;

#[path = "film_accumulation.rs"]
pub(super) mod film_accumulation;
use film_accumulation::{FilmLayouts, FilmState};
#[path = "shadow_visibility_filter.rs"]
mod shadow_visibility_filter;
#[path = "temporal_reconstruction.rs"]
pub(crate) mod temporal_reconstruction;
// MP1: per-program deferred shading of mixed-program frames.
#[path = "program_lists.rs"]
pub(crate) mod program_lists;
// U-RTVIS: the reflection pass's rl_trace (docs/design-reflection-pass.md).
#[path = "reflection_pass.rs"]
pub(crate) mod reflection_pass;
#[path = "gi_receiver_pass.rs"]
pub(crate) mod gi_receiver_pass;
// The Mac thread's reflection_hits after rl_trace (docs/design-reflection-pass.md).
#[path = "reflection_hits_gpu.rs"]
mod reflection_hits_gpu;
use program_lists::{ListLayouts, MAX_FRAME_PROGRAMS, ProgramLists};
// Lean shading's background-compiled pipelines (program::lean_variant).
#[path = "lean_gpu.rs"]
mod lean_gpu;

/// WGSL PrimaryFrame: inverse view-projection, lit clear colour, culling flags, the
/// `scene_pixels` rectangle, the sampling words, the reflection bands
/// (`ReflectionSettings::frame_words`) and TR1's restart zones (`FRAME_ZONES`).
// TP2 integration: post-processing shares this ABI instead of freezing its byte size.
pub(super) const FRAME_BYTES: usize = FRAME_ZONES + temporal_reconstruction::ZONES_BYTES;
/// `PrimaryFrame::zones`: written by `prepare_reconstruction` on reconstructed frames, zero
/// otherwise.
pub(super) const FRAME_ZONES: usize = 144;
/// Pixels per trace workgroup; the WGSL workgroup sizes must match.
pub(super) const TRACE_TILE: [u32; 2] = [8, 4];
/// Pixels per shading workgroup; the WGSL workgroup size must match.
pub(super) const SHADE_TILE: [u32; 2] = [8, 4];
/// Pixels per clear block in the shading dispatch; WGSL PRIMARY_CLEAR_TILE must match.
pub(super) const CLEAR_TILE: [u32; 2] = [16, 16];
/// Traced fresh visibility takes receiver coverage from the tile flag and hit record,
/// so misses never write receivers and uncovered tiles never write hit records. The
/// estimator itself is the unchanged production body; this engine kernel seam is in
/// shaders/fresh_shadows.wgsl, not in any program.
const COVERAGE_SEAM: &str = "    if inside {\n        let normal=textureLoad(fresh_normal,p,0);";
/// Traced shading reconstructs tangent frames with the prelude's own vertex function,
/// which the hardware composition pins (the affine vertex block must be unchanged).
const TANGENT_SEAM: &str =
    "@vertex fn vs_tangent(input:VertexInput, @location(5) tangent:vec4<f32>) -> VertexOutput {";
/// Enable directives precede every declaration, including the composition's own.
const ENABLES: &str = "enable wgpu_binding_array;\nenable wgpu_ray_query_vertex_return;\n";

/// The hardware (inline `hard_shadow` rays) and fresh (counts) compositions of one
/// prelude, each followed by the shared ray helpers.
fn heads(wgsl: &str) -> Result<[String; 2], Error> {
    let rays = format!(
        "const PRIMARY_BATCHES:u32={TRACED_DRAWS}u;\n{}",
        include_str!("../shaders/primary_rays.wgsl")
    );
    let hard = crate::hardware_shadows::shader_source(wgsl, SampleCount::One)?;
    let soft = fresh_shadow_gpu::shader_source(wgsl)?;
    Ok([hard, soft].map(|c| format!("{ENABLES}{c}\n{rays}")))
}

// TP2: glass reflection hits use R1's ordinary traced hit shading and geometry ABI.
pub(crate) fn transparent_reflection_head(wgsl: &str) -> Result<String, Error> {
    Ok(finish_shading_heads(heads(wgsl)?)?[0].clone())
}

/// Same engine seams and material ABI, with no traversal declarations at all.
pub(crate) fn non_rt_heads(wgsl: &str) -> Result<[String; 2], Error> {
    // Validate the affine/visibility seams even though this composition has no rays.
    crate::hardware_shadows::shader_source(wgsl, SampleCount::One)?;
    let (before, rest) = wgsl
        .split_once(crate::hardware_shadows::VISIBILITY_BEGIN)
        .unwrap();
    let (_, after) = rest
        .split_once(crate::hardware_shadows::VISIBILITY_END)
        .unwrap();
    let fresh = include_str!("../shaders/fresh_shadows.wgsl");
    let hash =
        &fresh[fresh.find("fn fresh_hash(").unwrap()..fresh.find("fn fresh_direction(").unwrap()];
    let soft = &fresh
        [fresh.find("@group(0) @binding(6)").unwrap()..fresh.find("struct FreshReceiver").unwrap()];
    let hard = "fn hard_shadow(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {return 1.0;}";
    Ok([hard, soft].map(|body| format!("enable wgpu_binding_array;\n{before}\n{body}\n{after}\n{hash}\nconst PRIMARY_BATCHES:u32={TRACED_DRAWS}u;\n{}", include_str!("../shaders/primary_rays.wgsl"))))
}

pub(crate) fn non_rt_sources(wgsl: &str) -> Result<([String; 2], [String; 2]), Error> {
    let heads = non_rt_heads(wgsl)?;
    let kernels = heads.clone().map(|h| format!("{h}\noverride FRESH_DIRECTIONS:u32=256u;\n@compute @workgroup_size(1) fn primary_trace() {{}}\n@compute @workgroup_size(1) fn primary_trace_receivers() {{}}\n@compute @workgroup_size(1) fn primary_trace_interactive() {{}}\n@compute @workgroup_size(1) fn fresh_compute() {{}}"));
    // M7-R1n: non-RT shading heads declare the software reflection geometry.
    let shading = finish_shading_heads(heads)?
        .map(|h| format!("{h}\n{}", crate::program::SOFTWARE_REFLECTION_GEOMETRY));
    Ok((kernels, shading))
}

/// The engine's trace and visibility kernel modules: hard (camera rays and exact
/// receivers) and soft (traced fresh visibility and the fused Interactive pass).
pub(crate) fn shader_sources(wgsl: &str) -> Result<[String; 2], Error> {
    let [hard, soft] = heads(wgsl)?;
    if soft.matches(COVERAGE_SEAM).count() != 1 {
        return Err(Error::InvalidInput(
            "traced fresh visibility requires the unchanged receiver coverage seam",
        ));
    }
    let soft = soft.replacen(
        COVERAGE_SEAM,
        &COVERAGE_SEAM.replacen(
            "if inside {",
            "if inside && textureLoad(primary_tiles,id/PRIMARY_TILE,0).x!=0u && textureLoad(primary_hit,p,0).x!=0u {",
            1,
        ),
        1,
    );
    let trace = include_str!("../shaders/primary_trace.wgsl");
    let interactive = include_str!("../shaders/primary_interactive.wgsl");
    let queue = if interactive.contains("fn primary_retry_interactive(") {
        include_str!("../shaders/primary_edge_queue.wgsl")
    } else {
        ""
    };
    Ok([
        format!("{hard}\n{trace}"),
        format!("{soft}\n{trace}\n{queue}\n{interactive}"),
    ])
}

/// The check pipeline shares the actual geometry arithmetic, including test rays,
/// but has no traversal types, builtins or lighting estimator in its module.
pub(crate) fn edge_check_source(kernels: &str) -> String {
    use std::collections::{BTreeMap, BTreeSet};
    let kernels = kernels.replace("\r\n", "\n");
    let mut functions = BTreeMap::new();
    for (at, _) in kernels.match_indices("\nfn ") {
        let start = at + 1;
        let name = &kernels[start + 3..kernels[start..].find('(').unwrap() + start];
        let brace = kernels[start..].find('{').unwrap() + start;
        let mut depth = 1;
        let mut end = brace + 1;
        while depth != 0 {
            depth += i32::from(kernels.as_bytes()[end] == b'{')
                - i32::from(kernels.as_bytes()[end] == b'}');
            end += 1;
        }
        functions.insert(name.to_owned(), kernels[start..end].to_owned());
    }
    let repair = functions.get_mut("primary_repair").unwrap();
    *repair = repair.replace("var query:ray_query<vertex_return>;", "");
    let start = repair
        .find("            if neighbour==0xffffffffu {")
        .unwrap();
    let end = repair[start..]
        .find("            var direct_pending=true;")
        .unwrap()
        + start;
    repair.replace_range(
        start..end,
        "            if neighbour==0xffffffffu {primary_check_fallback=true;continue;}\n",
    );
    let start = repair
        .find("                if neighbour!=0xffffffffu {")
        .unwrap();
    let end = repair[start..]
        .find("                var common_vertices=0u;")
        .unwrap()
        + start;
    repair.replace_range(start..end,"                if !direct_pending {break;}direct_pending=false;\n                v=primary_triangle_vertices(batch,neighbour);\n                if !primary_probe_intersects(v,slot,start,probe_direction,reach) {continue;}\n                if !visible {visible=primary_edge_dot(primary_geometric(v[0],v[1],v[2],slot),direction)<0.0;}\n");
    let mut selected = BTreeSet::new();
    let mut pending = vec![
        "primary_repair".to_owned(),
        "primary_ray".to_owned(),
        "primary_receiver".to_owned(),
    ];
    while let Some(name) = pending.pop() {
        if !selected.insert(name.clone()) {
            continue;
        }
        let body = &functions[&name];
        for dependency in functions.keys() {
            if dependency != &name && body.contains(&format!("{dependency}(")) {
                pending.push(dependency.clone());
            }
        }
    }
    let mut source = format!(
        "enable wgpu_binding_array;\nconst PRIMARY_BATCHES:u32={TRACED_DRAWS}u;\n{}\n",
        include_str!("../shaders/primary_edge_queue.wgsl")
    );
    source.push_str("struct Receiver {world:vec3<f32>,geometric:vec3<f32>,pixel:vec2<i32>,slot:u32,epsilon:f32,thin:bool};\nstruct NormalTransform {x:vec4<f32>,y:vec4<f32>,z:vec4<f32>,info:vec4<f32>};\n@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;\n@group(1) @binding(1) var<storage,read> normals:array<NormalTransform>;\nstruct PrimaryFrame {inverse:mat4x4<f32>,clear:vec4<f32>,sided:vec4<u32>,scene:vec4<u32>,sampling:vec4<u32>};\n@group(0) @binding(7) var primary_hit:texture_2d<u32>;\n@group(0) @binding(10) var<uniform> primary_frame:PrimaryFrame;\n@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;\nstruct PrimaryRay {near:vec3<f32>,far:vec3<f32>,dx_near:vec3<f32>,dy_near:vec3<f32>,dx_far:vec3<f32>,dy_far:vec3<f32>};\n");
    let start = kernels.find("struct PrimaryTrace {").unwrap();
    let end = kernels[start..]
        .find("fn primary_triangle_vertices(")
        .unwrap()
        + start;
    source.push_str(&kernels[start..end]);
    for name in selected {
        source.push_str(&functions[&name]);
        source.push('\n');
    }
    source.push_str(include_str!("../shaders/primary_edge_check.wgsl"));
    assert!(!source.contains("rayQuery") && !source.contains("ray_query"));
    source
}

/// Heads of every program's traced shading modules, indexed by `Shading`: Hard (inline
/// `hard_shadow` rays) and Soft (fresh counts). `program::compose` appends the API, the
/// program and the shading entry.
pub(crate) fn shading_heads(wgsl: &str) -> Result<[String; 2], Error> {
    let mut heads = heads(wgsl)?;
    // Hard shading reuses the exact epsilon published by its camera-hit pass.
    // Keep this engine helper separate from every user's shading program.
    let epsilon = "primary_epsilon(v,placement)";
    if heads[0].matches(epsilon).count() != 1 {
        return Err(Error::InvalidInput(
            "traced hard shading requires the engine receiver precision helper",
        ));
    }
    heads[0] = heads[0].replacen(
        epsilon,
        "textureLoad(primary_receiver_epsilon,pixel,0).x",
        1,
    );
    heads[0].push_str("\n@group(0) @binding(23) var primary_receiver_epsilon:texture_2d<f32>;\n");
    // RT1d integration seam: visibility input only; no hit-record/shading changes.
    shadow_visibility_filter::compose(&mut heads);
    finish_shading_heads(heads)
}
fn finish_shading_heads(heads: [String; 2]) -> Result<[String; 2], Error> {
    if heads.iter().any(|h| h.matches(TANGENT_SEAM).count() != 1) {
        return Err(Error::InvalidInput(
            "traced primary shading requires the unchanged tangent vertex function",
        ));
    }
    Ok(heads.map(|h| {
        h.replacen(
            TANGENT_SEAM,
            "fn primary_tangent(input:VertexInput,tangent:vec4<f32>)->VertexOutput {",
            1,
        )
    }))
}

/// One program's traced shading modules, indexed by `Shading`.
#[cfg(test)]
pub(super) fn shading_sources(wgsl: &str, program: &str) -> Result<[String; 2], Error> {
    let parameters = crate::program::declares_parameters(program);
    Ok(shading_heads(wgsl)?.map(|head| {
        crate::program::compose(&head, program, crate::program::Stage::Traced, parameters).source
    }))
}

#[cfg(test)]
pub(super) fn compile_structure_reconstruction_trace(kernels: &str) -> Result<String, Error> {
    temporal_reconstruction::trace_source_held(kernels)
}

#[cfg(test)]
pub(super) fn compile_structure_reconstruction_resolve(prelude: &str) -> Result<String, Error> {
    temporal_reconstruction::resolve_source(prelude)
}

/// Identity of the traced kernel modules, combined with the programs' on the HDR images
/// they write.
pub(super) fn sources_hash(sources: &[String; 2]) -> [u8; 32] {
    Sha256::new()
        .chain_update(sources[0].as_bytes())
        .chain_update(sources[1].as_bytes())
        .finalize()
        .into()
}

/// Pixels whose camera ray can reach any placement, as `[x0, y0, x1, y1)`: the bounding
/// rectangle of every conservative world bound projected through `matrix`, 2 px
/// outward. A ray outside it misses every triangle; traced frames skip those rays.
/// Bounds reaching behind the eye, or a nonfinite projection, give the image.
pub(crate) fn scene_pixels(bounds: &[Bounds], matrix: Mat4, [width, height]: [u32; 2]) -> [u32; 4] {
    let matrix = matrix.as_dmat4();
    let (mut lower, mut upper) = (glam::DVec2::INFINITY, glam::DVec2::NEG_INFINITY);
    for b in bounds {
        for i in 0..8 {
            let corner = glam::DVec3::from_array(std::array::from_fn(|axis| {
                f64::from(if i & (1 << axis) == 0 {
                    b.min[axis]
                } else {
                    b.max[axis]
                })
            }));
            let clip = matrix * corner.extend(1.0);
            // Pixel coordinates with y down; pixel (i, j) has its centre at (i + 0.5, j + 0.5).
            let pixel = glam::DVec2::new(clip.x / clip.w + 1.0, 1.0 - clip.y / clip.w)
                * 0.5
                * glam::DVec2::new(f64::from(width), f64::from(height));
            if clip.w <= 0.0 || !pixel.is_finite() {
                return [0, 0, width, height];
            }
            lower = lower.min(pixel);
            upper = upper.max(pixel);
        }
    }
    if bounds.is_empty() {
        return [0; 4];
    }
    let clamp = |v: f64, end: u32| v.clamp(0.0, f64::from(end)) as u32;
    let (lower, upper) = ((lower - 2.0).floor(), (upper + 2.0).ceil());
    [
        clamp(lower.x, width),
        clamp(lower.y, height),
        clamp(upper.x, width),
        clamp(upper.y, height),
    ]
}

/// Whether `device` enables the binding arrays traced visibility indexes per draw.
pub(super) fn supported(device: &PipelineDevice) -> bool {
    let limits = device.limits();
    device.features().contains(TRACED_FEATURES) && traced_limits(limits.clone()) == limits
}

/// Rejects a traced frame before any frame write: hardware-shadowed frames need opaque
/// double-sided casters, meshes must have BLASes, the draw count must fit, the camera
/// inverse must exist, and at most `MAX_FRAME_PROGRAMS` shading programs (`standard` for
/// draws without a material) may shade it, per program (MP1). Temporal film frames
/// (`single_program`) still shade with one program.
pub(super) fn admit_draws(
    draws: &[Draw<'_>],
    matrix: Mat4,
    shadowed: bool,
    standard: u64,
    single_program: bool,
) -> Result<(), Error> {
    let (programs, _) = program_lists::frame_slots(
        draws
            .iter()
            .map(|d| d.material.map_or(standard, |m| m.program.id())),
    );
    if single_program && programs.len() > 1 {
        return Err(Error::InvalidInput(
            "temporal film frames support one shading program per frame; mixed-program frames need another soft-shadow mode",
        ));
    }
    if programs.len() > MAX_FRAME_PROGRAMS {
        return Err(Error::InvalidInput(
            "a traced or visibility-buffer frame supports at most 32 shading programs",
        ));
    }
    if shadowed
        && draws
            .iter()
            .any(|d| d.material.is_some_and(|m| !m.double_sided))
    {
        return Err(Error::InvalidInput(
            "traced primary visibility with hardware shadows requires opaque double-sided materials",
        ));
    }
    placement_table(
        draws
            .iter()
            .map(|d| (d.instances.len(), false, false, false)),
    )?;
    if !matrix.inverse().is_finite() {
        return Err(Error::InvalidInput("traced camera inverse is nonfinite"));
    }
    Ok(())
}

/// The sun an RT renderer gets under the `RENDERLAB_RT_PRIMARY` measurement switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RtPrimarySun {
    /// The Interactive sun from its receivers: `primary_vb_sun` with TR1's budget, rotation
    /// and stops, or the 8-direction fresh prefix without TR1.
    Interactive,
    /// Hardware sun rays from the visibility buffer's receivers (the fresh kernel), this many
    /// disk directions per pixel: 8, 32 or 256.
    Rays(u32),
    /// The non-RT sun: the cached clipmap lookup (`SoftShadows::Virtual`).
    Virtual,
}

/// U-FIXED step 1 measurement switch (docs/frame-cost-ara-scenes.md section 0):
/// `RENDERLAB_RT_PRIMARY=vb` gives the RT path the raster visibility buffer instead of camera
/// rays, with the Interactive sun traced from its receivers (`primary_vb_sun`); `vb:8`,
/// `vb:32` and `vb:256` trace that many fixed directions per pixel instead (no rotation or
/// stops), `vb:virtual` takes the non-RT clipmap sun. Unset (or anything else): traced
/// primaries.
pub(crate) fn rt_primary_switch() -> Option<RtPrimarySun> {
    let value = std::env::var("RENDERLAB_RT_PRIMARY").ok()?;
    match value.as_str() {
        "vb" => Some(RtPrimarySun::Interactive),
        "vb:virtual" => Some(RtPrimarySun::Virtual),
        other => other
            .strip_prefix("vb:")?
            .parse()
            .ok()
            .filter(|n| [8, 32, 256].contains(n))
            .map(RtPrimarySun::Rays),
    }
}

impl super::view::ViewState {
    /// This view's TR1 history restarts on its next reconstructed frame.
    pub(super) fn cut_reconstruction(&mut self) {
        if let Some(state) = &mut self.reconstruction_state {
            state.plan.cut = true;
        }
    }
}
impl Renderer {
    /// Chooses disk-shadow sampling for traced lit 1x frames. Traced-capable RT
    /// renderers start with Interactive and TR1's adaptive ray budget; Exact remains
    /// the portable selector default and the explicit historical reference setting.
    /// The preference survives primary-path changes and reload; raster, hard and
    /// diagnostic frames use Exact, as reported by `FrameReport::soft_shadows`.
    /// Leaving `Temporal` releases its film history after in-flight frames.
    pub fn set_soft_shadows(&mut self, mode: crate::SoftShadows) {
        // U-FIXED step 1: a visibility-buffer RT renderer has no Interactive kernels; the
        // measurement switch names the sun it gets instead.
        let mode = match rt_primary_switch() {
            Some(sun)
                if mode == crate::SoftShadows::Interactive
                    && self.primary_mode == PrimaryVisibility::VisibilityBuffer
                    && self.hardware_shadows.is_some() =>
            {
                match sun {
                    RtPrimarySun::Interactive => mode,
                    RtPrimarySun::Rays(_) => crate::SoftShadows::Exact,
                    RtPrimarySun::Virtual => crate::SoftShadows::Virtual,
                }
            }
            _ => mode,
        };
        if mode != crate::SoftShadows::Temporal
            && let Some(state) = &mut self.view.primary
            && let Some(film) = state.film.take()
        {
            state
                .retired
                .push((self.submissions, Retired::Film(Box::new(film))));
        }
        self.view.soft_mode = mode;
    }

    pub fn soft_shadows(&self) -> crate::SoftShadows {
        self.view.soft_mode
    }

    /// Selects TR1 temporal reconstruction (docs/m7-tr1.md) for traced and visibility-buffer
    /// lit frames. Changing it restarts every pixel; disabling releases the history after
    /// in-flight frames.
    pub fn set_reconstruction(&mut self, settings: Reconstruction) -> Result<(), Error> {
        settings.validate()?;
        if settings != self.view.reconstruction {
            // A new history starts the sample sequence again, so equal settings replay
            // equal frames.
            self.view.reconstruction_state = None;
            self.view.reconstruction_index = 0;
        }
        self.view.reconstruction = settings;
        Ok(())
    }
    pub fn reconstruction(&self) -> Reconstruction {
        self.view.reconstruction
    }
    /// Global reflection controls (src/reflections.rs): R1's traced reflections and R1n's
    /// software rays trace only while enabled, and only up to `max_roughness`. Default on.
    pub fn set_reflections(&mut self, settings: crate::ReflectionSettings) -> Result<(), Error> {
        settings.validate()?;
        self.view.reflections = settings;
        Ok(())
    }
    pub fn reflections(&self) -> crate::ReflectionSettings {
        self.view.reflections
    }
    /// The next reconstructed frame restarts every pixel: a camera cut the engine cannot
    /// infer from the camera alone.
    /// The selected view's history restarts on its next reconstructed frame.
    pub fn cut_reconstruction(&mut self) {
        self.view.cut_reconstruction();
    }

    /// Selects how lit frames resolve camera visibility. Traced compiles its kernels and
    /// every program's traced modules from the installed source now, transactionally;
    /// failure keeps the current mode. Program traced pipelines compile when a traced
    /// frame first draws with the program. Raster releases traced pipelines and targets
    /// after in-flight frames.
    pub fn set_primary_visibility(&mut self, mode: PrimaryVisibility) -> Result<(), Error> {
        self.check_fault()?;
        if mode == self.primary_mode {
            return Ok(());
        }
        match mode {
            PrimaryVisibility::Raster => {
                self.pipeline.primary = None;
                for slot in &mut self.programs {
                    slot.modules.traced = None;
                    slot.modules.temporal = None;
                    slot.modules.streamed = None;
                    slot.pipelines.traced = Default::default();
                    slot.pipelines.hits = Default::default();
                    slot.pipelines.temporal = None;
                    slot.pipelines.streamed = Default::default();
                    slot.pipelines.list = Default::default();
                    slot.pipelines.lean = Default::default();
                }
                self.primary_sources = None;
                let submissions = self.submissions;
                for view in self.views_mut() {
                    if let Some(primary) = &mut view.primary {
                        primary.release(submissions);
                    }
                }
            }
            PrimaryVisibility::Traced | PrimaryVisibility::VisibilityBuffer => {
                if mode == PrimaryVisibility::Traced
                    && (self.hardware_shadows.is_none() || !supported(&self.device))
                {
                    return Err(Error::InvalidInput(
                        "traced primary visibility requires hardware ray queries and TRACED_FEATURES",
                    ));
                }
                if mode == PrimaryVisibility::VisibilityBuffer
                    && !visibility_buffer::supported(&self.device)
                {
                    return Err(Error::InvalidInput(
                        "visibility-buffer primaries require VISIBILITY_FEATURES, VISIBILITY_RASTER_FEATURES and visibility_limits",
                    ));
                }
                let state = self.view.primary.as_ref().ok_or(Error::InvalidInput(
                    "traced primary visibility requires a hardware-shadow renderer at 1x with TRACED_FEATURES and traced_limits",
                ))?;
                let (sources, heads) = if self.hardware_shadows.is_some() {
                    (shader_sources(&self.source)?, shading_heads(&self.source)?)
                } else {
                    non_rt_sources(&self.source)?
                };
                let scopes = OperationScopes::new(&self.device);
                let pipelines = state.compile(&self.device, &self.ledger, &sources);
                if let Some(error) = scopes.finish() {
                    return Err(match error {
                        wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                        _ => self.fail(error.to_string()),
                    });
                }
                let traced = self.compile_traced_modules(&heads)?;
                self.check_fault()?;
                self.pipelines.created += pipelines.count();
                for (slot, (modules, temporal)) in self.programs.iter_mut().zip(traced) {
                    slot.modules.traced = Some(modules);
                    slot.modules.temporal = Some(temporal);
                    slot.modules.streamed = None;
                    slot.pipelines.traced = Default::default();
                    slot.pipelines.hits = Default::default();
                    slot.pipelines.temporal = None;
                    slot.pipelines.streamed = Default::default();
                    slot.pipelines.list = Default::default();
                    slot.pipelines.lean = Default::default();
                }
                self.pipeline.primary = Some(pipelines);
                self.primary_sources = Some((sources, heads));
                self.update_hashes();
            }
        }
        self.primary_mode = mode;
        if mode != PrimaryVisibility::VisibilityBuffer {
            for view in self.views_mut() {
                view.visibility_buffer = None;
            }
        }
        Ok(())
    }
    pub fn primary_visibility(&self) -> PrimaryVisibility {
        self.primary_mode
    }
}

/// One binding per traced draw slot; unused slots bind stand-ins no placement names.
fn padded<'a>(
    buffers: impl Iterator<Item = &'a wgpu::Buffer>,
    absent: &'a wgpu::Buffer,
) -> Vec<wgpu::BufferBinding<'a>> {
    buffers
        .chain(std::iter::repeat(absent))
        .take(TRACED_DRAWS as usize)
        .map(|b| b.as_entire_buffer_binding())
        .collect()
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}
/// One slot per traced draw.
fn per_draw(entry: wgpu::BindGroupLayoutEntry) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        count: std::num::NonZeroU32::new(TRACED_DRAWS),
        ..entry
    }
}
fn buffer(binding: u32, ty: wgpu::BufferBindingType, size: u64) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
    )
}
fn uniform(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    buffer(binding, wgpu::BufferBindingType::Uniform, size)
}
fn read(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    buffer(
        binding,
        wgpu::BufferBindingType::Storage { read_only: true },
        size,
    )
}
fn texture(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension: if binding == 15 {
                wgpu::TextureViewDimension::D2Array
            } else {
                wgpu::TextureViewDimension::D2
            },
            multisampled: false,
        },
    )
}
fn image(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}
/// WGSL binding of the camera rays' TLAS view, with hit-triangle positions.
const TRACE_SCENE: u32 = 19;
/// M7-R1n: non-RT shading's software reflection geometry
/// (`program::SOFTWARE_REFLECTION_GEOMETRY`).
const SOFTWARE_REFLECTIONS: u32 = 36;
/// The shared TLAS: binding 2 for shadow rays, `TRACE_SCENE` for camera rays.
fn scene(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::AccelerationStructure {
            vertex_return: binding == TRACE_SCENE,
        },
    )
}

pub(super) struct Buffer {
    pub value: Owned<wgpu::Buffer>,
    _allocation: Allocation,
}
impl Buffer {
    pub(super) fn new(r: &Renderer, label: &str, size: u64, usage: wgpu::BufferUsages) -> Self {
        Self {
            value: r.ledger.own(
                r.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            _allocation: r.ledger.allocate(AllocationKind::Target, size),
        }
    }
}

/// Identity of everything one draw binds; any change rebuilds the tables.
#[derive(PartialEq)]
struct DrawKey {
    vertex: wgpu::Buffer,
    placements: wgpu::Buffer,
    // SK1: static draws alias the current stream; deforming draws retain last frame.
    previous: wgpu::Buffer,
    index: wgpu::Buffer,
    tangent: Option<wgpu::Buffer>,
    material: wgpu::BindGroup,
    instances: usize,
}
/// Per-scene tables and descriptor-indexed bindings. Rebuilt only when the submitted
/// draw sequence changes; instance transforms are republished when they change.
struct Geometry {
    keys: Vec<DrawKey>,
    _placements: Buffer,
    models: Buffer,
    normals: Buffer,
    /// Published current/previous models, then normals, in placement order.
    instances: Vec<u8>,
    arrays: Owned<wgpu::BindGroup>,
    /// Group 2, rebuilt alone when a streamed texture's resident range changes, so a
    /// residency change never replaces the tables other passes key on.
    materials: Option<Owned<wgpu::BindGroup>>,
    /// Per slot and role, the bound texture's identity and residency generation.
    material_key: Vec<(usize, u64)>,
}
/// Streamed-texture state of traced shading (`texture_residency.rs`): per slot and role
/// the residency word, then the frame's request and miss counters; the shared level
/// probe; and which residency id each slot word describes.
pub(super) struct ResidencyGpu {
    pub buffer: Buffer,
    _probe: Owned<wgpu::Texture>,
    probe: Owned<wgpu::TextureView>,
    _probe_allocation: Allocation,
    words: Vec<u32>,
    pub ids: Vec<Option<u64>>,
}
pub(super) struct Targets {
    /// Placement + 1 (zero is a miss), primitive, f32 barycentrics. Traced frames write
    /// them, with their receivers or Interactive counts, only in tiles whose `tiles`
    /// flag is set.
    pub hit: Depth,
    /// Exact hard-receiver epsilon, computed from the returned hit vertices.
    pub epsilon: Depth,
    /// One flag per `TRACE_TILE` pixels: nonzero when a camera ray in the tile hit.
    pub tiles: Depth,
    pub repairs: Buffer,
    pub repair_args: Buffer,
    pub retries: Buffer,
    /// P-RTS: pixels listed for the full shadow budget (header, then pixel indices).
    pub penumbra: Buffer,
    defer_edges: bool,
    /// U-RTVIS: the held budget's per-pixel history (R32Uint, ping-ponged by the
    /// reconstruction's parity) and its frame record; traced primaries only.
    pub held: Option<HeldTargets>,
    /// Deferred reflections (`program::deferred_variant`, `RENDERLAB_RL_DEFERRED=1`): each
    /// glossy pixel's reflectance with its traced fraction, and its untraced specular,
    /// written by shading at bindings 62 and 63 for the composite after it.
    pub deferred: Option<[Depth; 2]>,
    /// U-RTVIS: the reflection history (shaders/reflections_history.wgsl), both paths.
    pub rl_history: RlHistoryTargets,
    /// U-RTVIS: the reflection pass's L, H, R and D while RENDERLAB_RL_PASS=1.
    pub rl_pass: Option<reflection_pass::PassTargets>,
    pub gi_receiver: Option<gi_receiver_pass::Targets>,
}
/// Per pixel two halves of four r32uint layers, and the frame record (written by
/// `prepare_reconstruction`; zero, so never read, without reconstruction).
pub(super) struct RlHistoryTargets {
    pub history: Depth,
    pub frame: Buffer,
    /// The resolve's shading phase (binding 60) and its two values (0, then 1, 16 bytes each).
    pub phase: Buffer,
    pub phases: Buffer,
}
/// The reflection history's group-0 bindings in primary shading. U-RTVIS's shading-entry
/// resources take 54-63; the Mac's reflection_hits takes 64-67 (docs/design-reflection-pass.md).
const RL_HISTORY_FRAME: u32 = 55;
const RL_HISTORY: u32 = 54;
const RL_PHASE: u32 = 60;
/// Two halves of four, rl_resolve's sample and seen-history layers (8-15), the halves' hit
/// distances (16, 17), the sample's (18) and the a-trous filter's two buffers (19-22).
const RL_HISTORY_LAYERS: u32 = 23;
pub(super) struct HeldTargets {
    pub history: [Depth; 2],
    pub frame: Buffer,
}
/// Group-0 bindings for one hit/HDR/fresh/TLAS combination.
struct Bindings {
    /// Hit records, HDR, fresh targets, TLAS and list-buffer generations, then M7-R1n's
    /// software reflection geometry generation.
    key: (
        wgpu::Texture,
        wgpu::Texture,
        Option<wgpu::Texture>,
        u64,
        u64,
        u64,
    ),
    trace: [Option<Owned<wgpu::BindGroup>>; 2],
    fresh: Option<Owned<wgpu::BindGroup>>,
    /// Soft frames: the fused Interactive trace and visibility.
    interactive: Option<Owned<wgpu::BindGroup>>,
    /// U-RTVIS: `interactive` plus the held history; entry k writes history k and reads
    /// the other (the reconstruction's parity).
    held: Option<[Owned<wgpu::BindGroup>; 2]>,
    /// U-FIXED: `primary_vb_sun`'s groups; entry k writes history k and reads the other.
    vb_sun: Option<[Owned<wgpu::BindGroup>; 2]>,
    repair: Option<Owned<wgpu::BindGroup>>,
    check: Option<Owned<wgpu::BindGroup>>,
    /// P-RTS: penumbra list and trace.
    penumbra: Option<[Owned<wgpu::BindGroup>; 2]>,
    shade: [Option<Owned<wgpu::BindGroup>>; 2],
    /// MP1: `shade` plus the program list bindings, while list buffers exist.
    list: [Option<Owned<wgpu::BindGroup>>; 2],
    /// U-RTVIS: rl_trace's group 0 while RENDERLAB_RL_PASS=1.
    rl_trace: Option<Owned<wgpu::BindGroup>>,
    /// U-RTVIS: rl_resolve's group while RENDERLAB_RL_RESOLVE_PASS=1.
    rl_resolve: Option<Owned<wgpu::BindGroup>>,
}
enum Retired {
    Targets(#[allow(dead_code)] Box<Targets>),
    Geometry(#[allow(dead_code)] Geometry),
    Bindings(#[allow(dead_code)] Bindings),
    Film(#[allow(dead_code)] Box<FilmState>),
    FilmGroups(#[allow(dead_code)] Box<[[Owned<wgpu::BindGroup>; 3]; 2]>),
}

/// The engine's traced kernels, compiled while traced visibility is selected and with
/// every prelude reload then. Shading pipelines belong to each program (indexed by
/// `Shading`; both read the trace's tile flags first).
pub(super) struct PrimaryPipelines {
    /// Hit record only; hit record plus exact fresh receivers.
    pub trace: [Owned<wgpu::ComputePipeline>; 2],
    /// The production 256-ray estimator with hit-record coverage.
    pub fresh: Owned<wgpu::ComputePipeline>,
    /// U-FIXED: the same estimator's 8-direction prefix (8x4 receivers per group), the
    /// Interactive sun of visibility-buffer RT frames without TR1.
    pub fresh_eight: Owned<wgpu::ComputePipeline>,
    /// Interactive soft frames: camera rays and their 8-direction counts in one dispatch.
    pub interactive: Owned<wgpu::ComputePipeline>,
    /// Temporal soft frames: the jittered, direction-rotated Interactive trace and the
    /// reset frames' history start (`film_init`), compiled on the first such frame
    /// (`ensure_film_kernel`).
    pub film: Option<[Owned<wgpu::ComputePipeline>; 2]>,
    /// Temporal reconstruction's resolve and rotated Interactive kernels, compiled by its
    /// first frame after each prelude (`ensure_reconstruction`).
    pub reconstruction: Option<temporal_reconstruction::Pipelines>,
    pub interactive_repair: Option<Owned<wgpu::ComputePipeline>>,
    pub interactive_retry: Option<Owned<wgpu::ComputePipeline>>,
    pub interactive_full: Option<Owned<wgpu::ComputePipeline>>,
}
impl PrimaryPipelines {
    pub(super) fn count(&self) -> u64 {
        5 + u64::from(self.interactive_repair.is_some())
            + u64::from(self.interactive_retry.is_some())
            + u64::from(self.interactive_full.is_some())
    }
}

/// The passes after a frame's Interactive trace: the near-edge check (modern) or repair,
/// the retry, and the full kernel used when edges are not deferred.
#[derive(Clone, Copy)]
pub(super) struct InteractiveKernels<'a> {
    check: Option<&'a wgpu::ComputePipeline>,
    repair: Option<&'a wgpu::ComputePipeline>,
    retry: Option<&'a wgpu::ComputePipeline>,
    full: Option<&'a wgpu::ComputePipeline>,
    /// P-RTS: list and trace, on frames whose budget exceeds their probe count.
    penumbra: Option<[&'a wgpu::ComputePipeline; 2]>,
    /// U-RTVIS: the trace pass's group 0 when it is not `Bindings::interactive`.
    group: Option<&'a wgpu::BindGroup>,
}

/// Which traced shading pipeline and bindings a frame uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shading {
    /// Inline `hard_shadow` rays, or none.
    Hard,
    /// Fresh visibility counts, Exact or Interactive.
    Soft,
}

pub(super) struct PrimaryState {
    #[cfg(test)]
    pub test_split_interactive_timestamps: bool,
    #[cfg(test)]
    pub test_fresh_tile: Option<[u32; 2]>,
    /// Temporal history kept in every tile instead of only where rays have hit (tests).
    #[cfg(test)]
    pub test_film_materialize: bool,
    trace_bindings: [Owned<wgpu::BindGroupLayout>; 2],
    shade_bindings: [Owned<wgpu::BindGroupLayout>; 2],
    /// U-RTVIS: rl_trace's layouts while RENDERLAB_RL_PASS=1, and its kernels per `Shading`.
    rl_trace: Option<reflection_pass::TraceAbi>,
    rl_pipelines: [Option<reflection_pass::TracePipelines>; 2],
    /// U-RTVIS: rl_resolve while RENDERLAB_RL_RESOLVE_PASS=1.
    rl_resolve: Option<reflection_pass::ResolveKit>,
    fresh_bindings: Owned<wgpu::BindGroupLayout>,
    fresh_layout: Owned<wgpu::PipelineLayout>,
    interactive_bindings: Owned<wgpu::BindGroupLayout>,
    interactive_layout: Owned<wgpu::PipelineLayout>,
    /// U-RTVIS: `interactive_bindings` plus the held budget's record and history pair.
    held_bindings: Owned<wgpu::BindGroupLayout>,
    held_layout: Owned<wgpu::PipelineLayout>,
    /// U-FIXED: `primary_vb_sun` (the visibility buffer's receivers, the frame words and the
    /// held record and history pair), the per-draw arrays and lighting.
    vb_sun_bindings: Owned<wgpu::BindGroupLayout>,
    vb_sun_layout: Owned<wgpu::PipelineLayout>,
    repair_bindings: Owned<wgpu::BindGroupLayout>,
    repair_layout: Owned<wgpu::PipelineLayout>,
    check_bindings: Owned<wgpu::BindGroupLayout>,
    check_layout: Owned<wgpu::PipelineLayout>,
    /// P-RTS: the penumbra list (probe counts, hits, tiles, queue) and its trace.
    penumbra_list_bindings: Owned<wgpu::BindGroupLayout>,
    penumbra_list_layout: Owned<wgpu::PipelineLayout>,
    penumbra_trace_bindings: Owned<wgpu::BindGroupLayout>,
    penumbra_trace_layout: Owned<wgpu::PipelineLayout>,
    array_bindings: Owned<wgpu::BindGroupLayout>,
    material_bindings: Owned<wgpu::BindGroupLayout>,
    /// `material_bindings` plus the texture residency bindings 18 and 19.
    streamed_bindings: Owned<wgpu::BindGroupLayout>,
    streamed_layouts: [Owned<wgpu::PipelineLayout>; 2],
    trace_layouts: [Owned<wgpu::PipelineLayout>; 2],
    shade_layouts: [Owned<wgpu::PipelineLayout>; 2],
    frame: Option<(Buffer, [u8; FRAME_BYTES])>,
    /// The last prepared frame's `scene_pixels` rectangle.
    pub scene: [u32; 4],
    /// That rectangle widened to whole `TRACE_TILE`s: shading dispatches only its groups
    /// and clears the rest. Uploaded only when it changes.
    region: [u32; 4],
    /// `region` as storage for WGSL `primary_region`. A uniform-derived region made
    /// long-lived shading pipelines change their arithmetic mid-run on NVIDIA 617.14
    /// (docs/m6-interactive-climb-2.md), so it is not part of the frame uniform.
    region_buffer: Option<Buffer>,
    /// Storage stand-in for absent tangent streams and unused draw slots.
    absent: Option<Buffer>,
    /// The first draw's program in the last prepared frame: its slot 0.
    pub program: u64,
    /// MP1: the last prepared frame's distinct programs in first-draw order (its slots).
    /// More than one makes it a mixed frame, shaded per program (`program_lists`).
    pub programs: Vec<u64>,
    list_layouts: ListLayouts,
    /// The mixed frames' list buffers, kept for later mixed frames.
    lists: Option<ProgramLists>,
    /// The binning kernel, compiled by the first mixed frame.
    bins: Option<Owned<wgpu::ComputePipeline>>,
    pub targets: Option<Targets>,
    geometry: Option<Geometry>,
    /// M7-R1n: non-RT frames' software reflection geometry (reflection_software.rs).
    reflections: Option<SoftwareReflections>,
    bindings: Option<Bindings>,
    /// Counts rebuilds of `geometry` and `bindings`: temporal history depends on both.
    geometry_generation: u64,
    bindings_generation: u64,
    // SK1: TLAS-only rebinding changes poses, not reconstruction target identity.
    reconstruction_bindings_generation: u64,
    film_layouts: FilmLayouts,
    /// Temporal reconstruction's group 0 and pipeline layouts.
    reconstruction: (Owned<wgpu::BindGroupLayout>, Owned<wgpu::PipelineLayout>),
    /// Temporal frames' film history, while `SoftShadows::Temporal` is selected.
    pub film: Option<FilmState>,
    /// Created by the first frame that streams textures.
    pub residency: Option<ResidencyGpu>,
    /// This frame streams textures: it shades with the program's streamed pipelines,
    /// whose helpers record residency requests, and binds the residency state.
    pub requests: bool,
    /// The lean pipeline compiling in the background and those that failed (lean_gpu.rs).
    lean: lean_gpu::LeanState,
    retired: Vec<(u64, Retired)>,
}

impl PrimaryState {
    /// F0's test-only jittered reference uses three existing ABIs. Other entry
    /// points intentionally do not acquire the reference camera scratch fields.
    #[cfg(test)]
    pub(super) fn test_film_pipelines(
        &self,
        device: &PipelineDevice,
        kernels: &str,
        shading: &str,
    ) -> [wgpu::ComputePipeline; 3] {
        let [kernels, shading] = [kernels, shading].map(|source| {
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("test-only F0/R1 film reference"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        });
        [
            ("primary_trace_receivers", &self.trace_layouts[1], &kernels),
            ("fresh_compute", &self.fresh_layout, &kernels),
            ("primary_shade_tiles", &self.shade_layouts[1], &shading),
        ]
        .map(|(entry, layout, module)| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        })
    }

    /// The traced geometry tables' layout, which N8 visibility binds as group 1.
    pub(super) fn array_layout(&self) -> &wgpu::BindGroupLayout {
        &self.array_bindings
    }
    pub(super) fn frame_buffer(&self) -> &wgpu::Buffer {
        &self.frame.as_ref().unwrap().0.value
    }
    /// The prepared frame's placement-ordered models and geometry tables (N8).
    pub(super) fn virtual_geometry(&self) -> Option<(&wgpu::Buffer, &wgpu::BindGroup)> {
        self.geometry
            .as_ref()
            .map(|g| (&*g.models.value, &*g.arrays))
    }
    /// The prepared frame's traced shading groups 1 (geometry tables) and 2 (materials), on
    /// which the world radiance cache's cell shading runs the frame's program (M7 G2).
    pub(super) fn shading_groups(&self) -> Option<(&wgpu::BindGroup, &wgpu::BindGroup)> {
        let g = self.geometry.as_ref()?;
        Some((&*g.arrays, &**g.materials.as_ref()?))
    }
    /// The layout of this frame's material group: with the texture residency bindings
    /// while it streams textures.
    pub(super) fn material_layout(&self) -> &wgpu::BindGroupLayout {
        if self.requests {
            &self.streamed_bindings
        } else {
            &self.material_bindings
        }
    }

    #[cfg(test)]
    pub(super) fn test_fresh_bindings(&self) -> (&wgpu::BindGroupLayout, &wgpu::BindGroup) {
        (
            &self.fresh_bindings,
            self.bindings.as_ref().unwrap().fresh.as_ref().unwrap(),
        )
    }
    /// `lighting` is the group-3 layout shared with raster shading.
    pub fn new(device: &PipelineDevice, ledger: &Ledger, lighting: &wgpu::BindGroupLayout) -> Self {
        use wgpu::TextureFormat as F;
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            let entries: Vec<_> = entries
                .iter()
                .copied()
                .filter(|e| {
                    !matches!(e.ty, wgpu::BindingType::AccelerationStructure { .. })
                        || device
                            .features()
                            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
                })
                .collect();
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries: &entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let frame = uniform(10, FRAME_BYTES as u64);
        let camera = uniform(0, 80);
        let trace = [
            &[image(23, F::R32Float)][..],
            &[camera, image(16, F::Rgba32Float), image(17, F::Rgba32Float)],
        ]
        .map(|extra| {
            let mut entries = vec![
                scene(TRACE_SCENE),
                frame,
                image(13, F::Rgba32Uint),
                image(20, F::R32Uint),
            ];
            entries.extend_from_slice(extra);
            group("primary trace", &entries)
        });
        let interactive_entries = [
            scene(TRACE_SCENE),
            scene(2),
            frame,
            image(13, F::Rgba32Uint),
            image(5, F::R32Uint),
            image(20, F::R32Uint),
            buffer(
                24,
                wgpu::BufferBindingType::Storage { read_only: false },
                52,
            ),
            buffer(
                27,
                wgpu::BufferBindingType::Storage { read_only: false },
                16,
            ),
        ];
        let interactive_bindings = group(
            "primary Interactive trace and visibility",
            &interactive_entries,
        );
        // U-RTVIS: the reconstruction's trace entries also read the previous held history
        // and write this frame's (shaders/rtvis_held.wgsl).
        let held_bindings = group(
            "primary Interactive trace, held shadow budget",
            &[
                &interactive_entries[..],
                &[
                    uniform(50, temporal_reconstruction::HELD_BYTES as u64),
                    texture(51, wgpu::TextureSampleType::Uint),
                    image(52, F::R32Uint),
                ],
            ]
            .concat(),
        );
        let vb_sun_bindings = group(
            "primary visibility-buffer sun, held shadow budget",
            &[
                scene(2),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                texture(4, wgpu::TextureSampleType::Float { filterable: false }),
                image(5, F::R32Uint),
                texture(7, wgpu::TextureSampleType::Uint),
                frame,
                texture(21, wgpu::TextureSampleType::Uint),
                uniform(50, temporal_reconstruction::HELD_BYTES as u64),
                texture(51, wgpu::TextureSampleType::Uint),
                image(52, F::R32Uint),
            ],
        );
        let repair_bindings = group(
            "primary deferred edge repair",
            &[
                scene(TRACE_SCENE),
                scene(2),
                frame,
                image(13, F::Rgba32Uint),
                image(5, F::R32Uint),
                image(20, F::R32Uint),
                buffer(26, wgpu::BufferBindingType::Storage { read_only: true }, 52),
                buffer(28, wgpu::BufferBindingType::Storage { read_only: true }, 20),
            ],
        );
        let uint = wgpu::TextureSampleType::Uint;
        let penumbra_list_bindings = group(
            "P-RTS penumbra list",
            &[
                texture(6, uint),
                texture(7, uint),
                texture(21, uint),
                buffer(
                    29,
                    wgpu::BufferBindingType::Storage { read_only: false },
                    20,
                ),
            ],
        );
        let penumbra_trace_bindings = group(
            "P-RTS penumbra trace",
            &[
                scene(TRACE_SCENE),
                scene(2),
                frame,
                image(5, F::R32Uint),
                read(29, 20),
            ],
        );
        let check_bindings = group(
            "primary arithmetic edge checks",
            &[
                frame,
                buffer(
                    24,
                    wgpu::BufferBindingType::Storage { read_only: false },
                    52,
                ),
                buffer(
                    28,
                    wgpu::BufferBindingType::Storage { read_only: false },
                    20,
                ),
            ],
        );
        let shade_entries = [scene(2), texture(6, uint)].map(|visibility| {
            let mut entries = vec![
                camera,
                texture(7, uint),
                frame,
                image(18, F::Rgba16Float),
                texture(21, uint),
                read(22, 16),
                visibility,
            ];
            if visibility.binding == 2 {
                entries.push(texture(
                    23,
                    wgpu::TextureSampleType::Float { filterable: false },
                ));
            } else {
                // M7-R1: Soft shading traces reflection rays on the shared TLAS.
                entries.push(scene(2));
            }
            if gi_receiver_pass::on() { entries.push(gi_receiver_pass::entry()); }
            // U-RTVIS: the reflection history (program::with_reflection_history).
            entries.push(uniform(
                RL_HISTORY_FRAME,
                temporal_reconstruction::RL_HISTORY_BYTES as u64,
            ));
            entries.push(entry(
                RL_HISTORY,
                wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::ReadWrite,
                    format: F::R32Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                },
            ));
            // U-RTVIS: the reflection resolve's shading phase (reflections_history.wgsl).
            entries.push(uniform(RL_PHASE, 16));
            // U-RTVIS: the reflection pass's lobe output (reflection_pass.rs).
            if reflection_pass::on() {
                entries.extend(reflection_pass::shade_entries());
                // The Mac thread's reflection_hits: H and R (reflection_hits_gpu.rs).
                entries.extend(super::reflection_hits::shade_entries());
            }
            // M7-R1n: non-RT shading traces them in software (reflection_software.rs), read
            // as vec4 words (P-NRT), so a binding holds at least one.
            if !device
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            {
                entries.push(read(SOFTWARE_REFLECTIONS, 16));
            }
            // Deferred reflections' outputs (`Targets::deferred`).
            if crate::program::rl_deferred() {
                entries.extend([image(62, F::Rgba16Float), image(63, F::Rgba16Float)]);
            }
            entries
        });
        let shade_bindings = shade_entries
            .each_ref()
            .map(|e| group("primary shading", e));
        let fresh_bindings = group(
            "primary fresh visibility",
            &[
                scene(2),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                texture(4, wgpu::TextureSampleType::Float { filterable: false }),
                image(5, F::R32Uint),
                texture(7, uint),
                texture(21, uint),
            ],
        );
        let array_entries = [
            read(0, 64),
            read(1, 64),
            per_draw(read(2, 16)),
            per_draw(read(3, 4)),
            per_draw(read(4, 16)),
            read(5, 4),
            per_draw(read(6, 16)),
        ]
        .map(|e| wgpu::BindGroupLayoutEntry {
            visibility: wgpu::ShaderStages::COMPUTE
                | if e.binding == 4 {
                    wgpu::ShaderStages::empty()
                } else {
                    wgpu::ShaderStages::VERTEX
                },
            ..e
        });
        let array_bindings = group("primary draw geometry", &array_entries);
        let filtered = wgpu::TextureSampleType::Float { filterable: true };
        let sampler = |binding| {
            per_draw(entry(
                binding,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ))
        };
        let material_entries = [
            per_draw(read(10, 48)),
            per_draw(texture(11, filtered)),
            sampler(12),
            per_draw(texture(13, filtered)),
            sampler(14),
            per_draw(texture(15, filtered)),
            sampler(16),
            // Each draw's program parameter block; read only by programs declaring one.
            per_draw(read(17, crate::program::PARAMETER_BYTES as u64)),
        ];
        let material_bindings = group("primary draw materials", &material_entries);
        // Frames that stream textures add the residency words and counters and the
        // shared level probe (texture_residency.rs); default frames never bind them.
        let streamed_bindings = group(
            "primary draw materials, texture residency",
            &[
                &material_entries[..],
                &[
                    buffer(
                        18,
                        wgpu::BufferBindingType::Storage { read_only: false },
                        (texture_residency::WORDS * 4) as u64,
                    ),
                    texture(19, filtered),
                ],
            ]
            .concat(),
        );
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        Self {
            #[cfg(test)]
            test_split_interactive_timestamps: false,
            #[cfg(test)]
            test_fresh_tile: None,
            #[cfg(test)]
            test_film_materialize: false,
            trace_layouts: [&trace[0], &trace[1]]
                .map(|g| layout("primary trace ABI", &[Some(g), Some(&array_bindings)])),
            shade_layouts: shade_bindings.each_ref().map(|g| {
                layout(
                    "primary shading ABI",
                    &[
                        Some(g),
                        Some(&array_bindings),
                        Some(&material_bindings),
                        Some(lighting),
                    ],
                )
            }),
            streamed_layouts: shade_bindings.each_ref().map(|g| {
                layout(
                    "primary streamed shading ABI",
                    &[
                        Some(g),
                        Some(&array_bindings),
                        Some(&streamed_bindings),
                        Some(lighting),
                    ],
                )
            }),
            list_layouts: ListLayouts::new(
                device,
                ledger,
                &shade_entries,
                &array_bindings,
                [&material_bindings, &streamed_bindings],
                lighting,
            ),
            streamed_bindings,
            fresh_layout: layout(
                "primary fresh visibility ABI",
                &[Some(&fresh_bindings), None, None, Some(lighting)],
            ),
            interactive_layout: layout(
                "primary Interactive trace ABI",
                &[
                    Some(&interactive_bindings),
                    Some(&array_bindings),
                    None,
                    Some(lighting),
                ],
            ),
            held_layout: layout(
                "primary Interactive trace ABI, held shadow budget",
                &[
                    Some(&held_bindings),
                    Some(&array_bindings),
                    None,
                    Some(lighting),
                ],
            ),
            held_bindings,
            vb_sun_layout: layout(
                "primary visibility-buffer sun ABI",
                &[
                    Some(&vb_sun_bindings),
                    Some(&array_bindings),
                    None,
                    Some(lighting),
                ],
            ),
            vb_sun_bindings,
            film_layouts: FilmLayouts::new(
                device,
                ledger,
                &array_bindings,
                &material_bindings,
                lighting,
            ),
            reconstruction: Self::reconstruction_layout(device, ledger, &array_bindings),
            repair_layout: layout(
                "primary edge repair ABI",
                &[
                    Some(&repair_bindings),
                    Some(&array_bindings),
                    None,
                    Some(lighting),
                ],
            ),
            repair_bindings,
            check_layout: layout(
                "primary arithmetic edge check ABI",
                &[Some(&check_bindings), Some(&array_bindings)],
            ),
            check_bindings,
            penumbra_list_layout: layout(
                "P-RTS penumbra list ABI",
                &[Some(&penumbra_list_bindings)],
            ),
            penumbra_list_bindings,
            penumbra_trace_layout: layout(
                "P-RTS penumbra trace ABI",
                &[
                    Some(&penumbra_trace_bindings),
                    Some(&array_bindings),
                    None,
                    Some(lighting),
                ],
            ),
            penumbra_trace_bindings,
            interactive_bindings,
            fresh_bindings,
            trace_bindings: trace,
            shade_bindings,
            rl_trace: reflection_pass::on()
                .then(|| reflection_pass::TraceAbi::new(device, ledger, &array_bindings, lighting)),
            rl_pipelines: [None, None],
            rl_resolve: crate::program::rl_resolve_pass_on()
                .then(|| reflection_pass::ResolveKit::new(device, ledger)),
            array_bindings,
            material_bindings,
            frame: None,
            scene: [0; 4],
            region: [0; 4],
            region_buffer: None,
            absent: None,
            program: 0,
            programs: vec![],
            lean: Default::default(),
            lists: None,
            bins: None,
            targets: None,
            geometry: None,
            reflections: None,
            bindings: None,
            geometry_generation: 0,
            bindings_generation: 0,
            reconstruction_bindings_generation: 0,
            film: None,
            residency: None,
            requests: false,
            retired: vec![],
        }
    }

    /// A new view's state: these layouts (pipelines compiled against them serve every
    /// view) and the binning kernel; its targets, geometry tables, bindings and film start
    /// empty and are built by its first traced frame.
    pub fn fork(&self) -> Self {
        Self {
            #[cfg(test)]
            test_split_interactive_timestamps: self.test_split_interactive_timestamps,
            #[cfg(test)]
            test_fresh_tile: self.test_fresh_tile,
            #[cfg(test)]
            test_film_materialize: self.test_film_materialize,
            trace_bindings: self.trace_bindings.clone(),
            shade_bindings: self.shade_bindings.clone(),
            rl_trace: self.rl_trace.clone(),
            rl_pipelines: self.rl_pipelines.clone(),
            rl_resolve: self.rl_resolve.clone(),
            fresh_bindings: self.fresh_bindings.clone(),
            fresh_layout: self.fresh_layout.clone(),
            interactive_bindings: self.interactive_bindings.clone(),
            interactive_layout: self.interactive_layout.clone(),
            held_bindings: self.held_bindings.clone(),
            held_layout: self.held_layout.clone(),
            vb_sun_bindings: self.vb_sun_bindings.clone(),
            vb_sun_layout: self.vb_sun_layout.clone(),
            repair_bindings: self.repair_bindings.clone(),
            repair_layout: self.repair_layout.clone(),
            check_bindings: self.check_bindings.clone(),
            check_layout: self.check_layout.clone(),
            penumbra_list_bindings: self.penumbra_list_bindings.clone(),
            penumbra_list_layout: self.penumbra_list_layout.clone(),
            penumbra_trace_bindings: self.penumbra_trace_bindings.clone(),
            penumbra_trace_layout: self.penumbra_trace_layout.clone(),
            array_bindings: self.array_bindings.clone(),
            material_bindings: self.material_bindings.clone(),
            streamed_bindings: self.streamed_bindings.clone(),
            streamed_layouts: self.streamed_layouts.clone(),
            trace_layouts: self.trace_layouts.clone(),
            shade_layouts: self.shade_layouts.clone(),
            list_layouts: self.list_layouts.clone(),
            film_layouts: self.film_layouts.clone(),
            reconstruction: self.reconstruction.clone(),
            bins: self.bins.clone(),
            frame: None,
            scene: [0; 4],
            region: [0; 4],
            region_buffer: None,
            absent: None,
            program: 0,
            programs: vec![],
            lean: Default::default(),
            lists: None,
            targets: None,
            geometry: None,
            reflections: None,
            bindings: None,
            geometry_generation: 0,
            bindings_generation: 0,
            reconstruction_bindings_generation: 0,
            film: None,
            residency: None,
            requests: false,
            retired: vec![],
        }
    }
    /// `sources` are `shader_sources` output: hard kernels first, soft kernels second.
    pub fn compile(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        sources: &[String; 2],
    ) -> PrimaryPipelines {
        let [hard, soft] = sources.each_ref().map(|source| {
            ledger.own(
                device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("traced primary visibility and deferred shading"),
                    source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
                }),
                ResourceKind::ShaderModule,
            )
        });
        let pipeline = |module: &wgpu::ShaderModule, entry, layout: &wgpu::PipelineLayout| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let modern = sources[1].contains("fn primary_retry_interactive(");
        let deferred = modern || sources[1].contains("fn primary_trace_interactive_deferred(");
        let check = modern.then(|| {
            ledger.own(
                device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("primary arithmetic edge checks; no traversal"),
                    source: wgpu::ShaderSource::Wgsl(edge_check_source(&sources[1]).into()),
                }),
                ResourceKind::ShaderModule,
            )
        });
        PrimaryPipelines {
            interactive_repair: deferred.then(|| {
                self.interactive_entry(
                    device,
                    ledger,
                    if modern {
                        check.as_ref().unwrap()
                    } else {
                        &soft
                    },
                    if modern {
                        "primary_check_interactive"
                    } else {
                        "primary_repair_interactive"
                    },
                    if modern {
                        &[]
                    } else {
                        &[("FRESH_DIRECTIONS", 8.0)]
                    },
                )
            }),
            interactive_retry: modern.then(|| {
                self.interactive_entry(
                    device,
                    ledger,
                    &soft,
                    "primary_retry_interactive",
                    &[("FRESH_DIRECTIONS", 8.0)],
                )
            }),
            interactive_full: deferred.then(|| {
                self.interactive_pipeline(device, ledger, &soft, &[("FRESH_DIRECTIONS", 8.0)])
            }),
            // Under the U-FIXED measurement switch the fresh pass traces fewer directions.
            fresh: match rt_primary_switch() {
                Some(RtPrimarySun::Rays(n)) if n != 256 => self.fresh_pipeline(
                    device,
                    ledger,
                    &soft,
                    "fresh_compute",
                    &[("FRESH_DIRECTIONS", f64::from(n))],
                ),
                _ => self.fresh_pipeline(device, ledger, &soft, "fresh_compute", &[]),
            },
            fresh_eight: self.fresh_pipeline(
                device,
                ledger,
                &soft,
                "fresh_compute",
                &[("FRESH_DIRECTIONS", 8.0)],
            ),
            interactive: self.interactive_entry(
                device,
                ledger,
                &soft,
                if deferred {
                    "primary_trace_interactive_deferred"
                } else {
                    "primary_trace_interactive"
                },
                &[("FRESH_DIRECTIONS", 8.0)],
            ),
            trace: [
                pipeline(&hard, "primary_trace", &self.trace_layouts[0]),
                pipeline(&hard, "primary_trace_receivers", &self.trace_layouts[1]),
            ],
            film: None,
            reconstruction: None,
        }
    }

    /// The temporal trace and reset (`film_init`) pipelines from one
    /// `film_accumulation::kernel_source` module.
    pub fn film_kernel(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        source: &str,
    ) -> [Owned<wgpu::ComputePipeline>; 2] {
        let module = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal film trace"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        [
            ("primary_trace_interactive", &self.film_layouts.trace),
            ("film_init", &self.film_layouts.init),
        ]
        .map(|(entry, layout)| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("FRESH_DIRECTIONS", 8.0)],
                        ..Default::default()
                    },
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        })
    }

    /// A program's temporal shading pipeline from its `film_accumulation::compose_shading`
    /// module, through `cache` when given.
    pub fn film_shade_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("temporal program shading"),
                layout: Some(&self.film_layouts.shade),
                module,
                entry_point: Some("primary_shade_tiles"),
                compilation_options: Default::default(),
                cache,
            }),
            ResourceKind::ComputePipeline,
        )
    }

    /// One program's shading pipelines from its composed modules, indexed by `Shading`.
    #[cfg(test)]
    pub fn shade_pipelines(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        modules: [&wgpu::ShaderModule; 2],
    ) -> [Owned<wgpu::ComputePipeline>; 2] {
        [0, 1].map(|i| self.shade_pipeline(device, ledger, modules[i], i, None))
    }
    /// The pipeline of `Shading` index `shading` from its module, so the pair can compile
    /// concurrently, through `cache` when given.
    pub fn shade_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        shading: usize,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Owned<wgpu::ComputePipeline> {
        self.shading_pipeline(device, ledger, module, &self.shade_layouts[shading], cache)
    }
    /// `shade_pipeline` of a streamed (`Stage::TracedStreamed`) module.
    pub fn streamed_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        shading: usize,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Owned<wgpu::ComputePipeline> {
        self.shading_pipeline(
            device,
            ledger,
            module,
            &self.streamed_layouts[shading],
            cache,
        )
    }
    fn shading_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        layout: &wgpu::PipelineLayout,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("traced program shading"),
                layout: Some(layout),
                module,
                entry_point: Some("primary_shade_tiles"),
                compilation_options: Default::default(),
                cache,
            }),
            ResourceKind::ComputePipeline,
        )
    }

    /// `fresh_compute` of a soft primary module on the traced fresh ABI. Tests seed or
    /// replace the estimator through `constants` and variant modules.
    pub fn fresh_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        entry: &str,
        constants: &[(&str, f64)],
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("traced fresh 256 visibility rays"),
                layout: Some(&self.fresh_layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..Default::default()
                },
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        )
    }

    /// P-RTS: the standalone penumbra list and `primary_penumbra_trace` of TR1's soft
    /// module `module` (docs/p-rts.md).
    pub(super) fn penumbra_pipelines(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
    ) -> [Owned<wgpu::ComputePipeline>; 2] {
        let list = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("P-RTS penumbra list"),
            source: wgpu::ShaderSource::Wgsl(temporal_reconstruction::PENUMBRA_LIST.into()),
        });
        let pipeline = |layout, module, entry| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        [
            pipeline(&*self.penumbra_list_layout, &list, "primary_penumbra_list"),
            pipeline(
                &*self.penumbra_trace_layout,
                module,
                "primary_penumbra_trace",
            ),
        ]
    }

    /// `primary_trace_interactive` of a soft primary module on its fused ABI. Tests seed
    /// the estimator through `constants`.
    pub fn interactive_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        constants: &[(&str, f64)],
    ) -> Owned<wgpu::ComputePipeline> {
        self.interactive_entry(
            device,
            ledger,
            module,
            "primary_trace_interactive",
            constants,
        )
    }
    fn interactive_entry(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        entry: &str,
        constants: &[(&str, f64)],
    ) -> Owned<wgpu::ComputePipeline> {
        let layout = match entry {
            "primary_check_interactive" => &self.check_layout,
            "primary_retry_interactive" | "primary_repair_interactive" => &self.repair_layout,
            _ => &self.interactive_layout,
        };
        self.entry_on(device, ledger, module, entry, constants, layout)
    }
    /// U-RTVIS: a trace entry of `trace_source_held`'s module, on the held layout.
    pub(super) fn held_entry(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        entry: &str,
        constants: &[(&str, f64)],
    ) -> Owned<wgpu::ComputePipeline> {
        self.entry_on(device, ledger, module, entry, constants, &self.held_layout)
    }
    /// U-FIXED: `primary_vb_sun` of `vb_sun_source`'s module, on the VB sun layout.
    pub(super) fn vb_sun_entry(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
    ) -> Owned<wgpu::ComputePipeline> {
        self.entry_on(device, ledger, module, "primary_vb_sun", &[], &self.vb_sun_layout)
    }
    fn entry_on(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        entry: &str,
        constants: &[(&str, f64)],
        layout: &wgpu::PipelineLayout,
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..Default::default()
                },
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        )
    }

    /// Retires every frame resource after `last`; the next traced frame recreates them.
    pub fn release(&mut self, last: u64) {
        if let Some(t) = self.targets.take() {
            self.retired.push((last, Retired::Targets(Box::new(t))));
        }
        if let Some(g) = self.geometry.take() {
            self.retired.push((last, Retired::Geometry(g)));
        }
        if let Some(b) = self.bindings.take() {
            self.retired.push((last, Retired::Bindings(b)));
        }
        if let Some(f) = self.film.take() {
            self.retired.push((last, Retired::Film(Box::new(f))));
        }
        self.frame = None;
        self.region_buffer = None;
        self.absent = None;
        self.lists = None;
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
    /// Workgroups of `tile` pixels covering `region`.
    pub(super) fn region_groups(&self, tile: [u32; 2]) -> [u32; 2] {
        let r = self.region;
        std::array::from_fn(|i| (r[i + 2] - r[i]).div_ceil(tile[i]))
    }
}

impl Renderer {
    /// Publishes one traced frame's tables, targets and bindings. Runs after flight
    /// admission, when the HDR and fresh targets and the TLAS generation are current.
    pub(super) fn prepare_primary(
        &mut self,
        draws: &[PreparedDraw<'_>],
        camera: &Camera,
        soft: bool,
        scene_generation: u64,
        uploads: &mut UploadBytes,
    ) -> Result<(), Error> {
        let size = self.view.hdr.as_ref().unwrap().resolved.texture.size();
        let default = self.default_material.as_ref().unwrap();
        let placements = placement_table(draws.iter().map(|d| {
            let material = d.material.unwrap_or(default);
            (
                d.instances.len(),
                d.mesh.tangent.is_some(),
                !material.double_sided,
                material.thin,
            )
        }))?;
        // Slot 0 is the first draw's program (standard for an empty frame, which shades
        // nothing); MP1 shades a frame with several programs per program.
        let (programs, slots) = program_lists::frame_slots(
            draws
                .iter()
                .map(|d| d.material.unwrap_or(default).program.id()),
        );
        self.view.primary.as_mut().unwrap().program =
            programs.first().copied().unwrap_or(self.standard.id());
        // One single-sided draw makes every primary ray non-opaque (each candidate runs the
        // facing test in the shader). `RENDERLAB_DIAG_ALL_OPAQUE=1` traces as if every draw
        // were double-sided, to measure that cost through an embedder (measurement only:
        // single-sided back faces then show).
        static ALL_OPAQUE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let all_opaque = *ALL_OPAQUE
            .get_or_init(|| std::env::var("RENDERLAB_DIAG_ALL_OPAQUE").is_ok_and(|v| v == "1"));
        let sided = !all_opaque && placements.iter().any(|p| p & 1 << 30 != 0);
        let matrix = camera.matrix(size.width, size.height);
        let inverse = matrix.inverse();
        if !inverse.is_finite() {
            return Err(Error::InvalidInput("traced camera inverse is nonfinite"));
        }
        // Every traced frame's camera rays read the rectangle: O(placements) per frame.
        let mut bounds = vec![];
        for d in draws {
            bounds.extend_from_slice(&d.instances.state.world_bounds(d.mesh.bounds)?);
        }
        let scene = scene_pixels(&bounds, matrix, [size.width, size.height]);
        // Whole trace tiles, so shading reads only tile flags its trace wrote.
        let region = {
            let t = TRACE_TILE;
            let lo = [scene[0] / t[0] * t[0], scene[1] / t[1] * t[1]];
            [
                lo[0],
                lo[1],
                lo[0] + (scene[2] - lo[0]).div_ceil(t[0]) * t[0],
                lo[1] + (scene[3] - lo[1]).div_ceil(t[1]) * t[1],
            ]
        };
        if self.view.primary.as_ref().unwrap().region_buffer.is_none() {
            let buffer = Buffer::new(
                self,
                "primary shading region",
                16,
                wgpu::BufferUsages::STORAGE,
            );
            let state = self.view.primary.as_mut().unwrap();
            state.region_buffer = Some(buffer);
            // No real region is all ones, so the new buffer is always written.
            state.region = [u32::MAX; 4];
        }
        let state = self.view.primary.as_mut().unwrap();
        state.scene = scene;
        if state.region != region {
            let words: Vec<u8> = region.iter().flat_map(|v| v.to_ne_bytes()).collect();
            self.queue
                .write_buffer(&state.region_buffer.as_ref().unwrap().value, 0, &words);
            state.region = region;
            uploads.camera += 16;
            uploads.write_calls += 1;
        }
        let mut bytes = [0; FRAME_BYTES];
        // sampling.x: TR1's shadow budget; y: its probe directions; z: the probes inside the
        // sun's disk before wide ones (0: no split); w: the wide probes' cone scale as f32
        // bits (P-RTS; 0 and 1.0 here and on every frame that lists no penumbrae).
        bytes[112..116].copy_from_slice(&8u32.to_ne_bytes());
        bytes[116..120].copy_from_slice(&8u32.to_ne_bytes());
        bytes[124..128].copy_from_slice(&1f32.to_bits().to_ne_bytes());
        bytes[..64].copy_from_slice(&matrix_bytes(inverse));
        for (dst, value) in bytes[64..80]
            .chunks_exact_mut(4)
            .zip(self.lit_clear().into_iter().chain([1.0]))
        {
            dst.copy_from_slice(&(value as f32).to_ne_bytes());
        }
        bytes[80..84].copy_from_slice(&u32::from(sided).to_ne_bytes());
        // Reconstructed frames' Interactive direction rotation (temporal_reconstruction.rs);
        // production kernels never read it.
        let rotation = self
            .view
            .reconstruction_frame
            .filter(|_| self.view.reconstruction.rotate_directions)
            .map_or(0, temporal_reconstruction::rotation);
        bytes[84..88].copy_from_slice(&rotation.to_ne_bytes());
        // Visible sky (Renderer::set_visible_environment): misses sample the selected map,
        // at the radiance scale whose f32 bits this word holds (0: off; see primary_miss).
        let visible = self.environment.visible && self.environment.selected.is_some();
        let scale = self
            .environment
            .visible_scale
            .unwrap_or(self.environment.parameters.intensity);
        bytes[92..96].copy_from_slice(&(if visible { scale.to_bits() } else { 0 }).to_ne_bytes());
        // RT1d uses a spare frame word; all geometry/visibility bindings already exist.
        let filter = self.primary_mode == PrimaryVisibility::Traced
            && self.view.soft_mode == SoftShadows::Interactive
            && self
                .hardware_shadows
                .as_ref()
                .is_some_and(|h| h.visibility_filter);
        bytes[88..92].copy_from_slice(&u32::from(filter).to_ne_bytes());
        for (dst, value) in bytes[96..112].chunks_exact_mut(4).zip(scene) {
            dst.copy_from_slice(&value.to_ne_bytes());
        }
        // M7-R1/R1n: the global reflection bands, on both paths. A non-RT frame traces only
        // when every drawn mesh has software geometry: a ray through an uncovered mesh would
        // show the sky behind it, a leak worse than the untraced estimate. Skinned meshes
        // (SK1) are left out of the software geometry, not counted as uncovered: their GPU
        // vertices move every frame, so a bind-pose tree would reflect them in the wrong place.
        let covered = self.hardware_shadows.is_some()
            || draws
                .iter()
                .all(|d| d.mesh.reflection.is_some() || d.mesh.skin.is_some());
        let bands = if covered {
            self.view.reflections.frame_words()
        } else {
            crate::ReflectionSettings::off().frame_words()
        };
        for (dst, value) in bytes[128..144].chunks_exact_mut(4).zip(bands) {
            dst.copy_from_slice(&value.to_ne_bytes());
        }
        if self.view.primary.as_ref().unwrap().frame.is_none() {
            let frame = Buffer::new(
                self,
                "primary camera inverse, clear and culling",
                FRAME_BYTES as u64,
                wgpu::BufferUsages::UNIFORM,
            );
            self.view.primary.as_mut().unwrap().frame = Some((frame, [0; FRAME_BYTES]));
        }
        if self.view.primary.as_ref().unwrap().absent.is_none() {
            let absent = Buffer::new(
                self,
                "primary absent storage",
                48,
                wgpu::BufferUsages::STORAGE,
            );
            self.view.primary.as_mut().unwrap().absent = Some(absent);
        }
        let (frame, shadow) = self.view.primary.as_mut().unwrap().frame.as_mut().unwrap();
        if *shadow != bytes {
            self.queue.write_buffer(&frame.value, 0, &bytes);
            *shadow = bytes;
            uploads.camera += FRAME_BYTES as u64;
            uploads.write_calls += 1;
        }
        let keys: Vec<_> = draws
            .iter()
            .map(|d| DrawKey {
                vertex: (**d.mesh.vertex).clone(),
                placements: (*d.instances.transforms).clone(),
                previous: d
                    .mesh
                    .skin
                    .as_ref()
                    .map_or_else(|| (**d.mesh.vertex).clone(), |s| (**s.previous).clone()),
                index: (**d.mesh.index).clone(),
                tangent: d.mesh.tangent.as_ref().map(|t| (**t).clone()),
                material: (*d.material.unwrap_or(default).bind).clone(),
                instances: d.instances.len(),
            })
            .collect();
        let state = self.view.primary.as_ref().unwrap();
        if state.geometry.as_ref().is_none_or(|g| g.keys != keys) {
            let storage = wgpu::BufferUsages::STORAGE;
            let instance_bytes = (placements.len().max(1) * 64) as u64;
            if instance_bytes * 2 > u64::from(self.device.limits().max_storage_buffer_binding_size)
            {
                return Err(Error::InvalidInput(
                    "current/previous placement transforms exceed storage binding limit",
                ));
            }
            let models = Buffer::new(
                self,
                "primary placement transforms",
                instance_bytes * 2,
                storage,
            );
            let normals = Buffer::new(self, "primary placement normals", instance_bytes, storage);
            // A skinned draw binds its own previous stream, one padding vec4 longer than
            // the current one; a static draw aliases the current stream.
            let batches: Vec<u32> = draws
                .iter()
                .map(|d| {
                    let skinned = d
                        .mesh
                        .skin
                        .as_ref()
                        .is_some_and(|s| s.previous.size() > d.mesh.vertex.size());
                    batch_word(d.mesh.index.size() / 4, skinned)
                })
                .collect();
            let words: Vec<u8> = placement_words(&placements, &batches)
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect();
            let table = Buffer::new(self, "primary placement draws", words.len() as u64, storage);
            self.queue.write_buffer(&table.value, 0, &words);
            uploads.shadows += words.len() as u64;
            uploads.write_calls += 1;
            let state = self.view.primary.as_ref().unwrap();
            let absent: &wgpu::Buffer = &state.absent.as_ref().unwrap().value;
            let vertices = padded(draws.iter().map(|d| &**d.mesh.vertex), absent);
            let previous = padded(
                draws.iter().map(|d| {
                    d.mesh
                        .skin
                        .as_ref()
                        .map_or(&**d.mesh.vertex, |s| &**s.previous)
                }),
                absent,
            );
            let indices = padded(draws.iter().map(|d| &**d.mesh.index), absent);
            let tangents = padded(
                draws
                    .iter()
                    .map(|d| d.mesh.tangent.as_deref().unwrap_or(absent)),
                absent,
            );
            let bind = |label, layout, entries: &[(u32, wgpu::BindingResource<'_>)]| {
                let entries: Vec<_> = entries
                    .iter()
                    .map(|(binding, resource)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: resource.clone(),
                    })
                    .collect();
                self.ledger.own(
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(label),
                        layout,
                        entries: &entries,
                    }),
                    ResourceKind::BindGroup,
                )
            };
            use wgpu::BindingResource as R;
            let arrays = bind(
                "primary draw geometry",
                &state.array_bindings,
                &[
                    (0, models.value.as_entire_binding()),
                    (1, normals.value.as_entire_binding()),
                    (2, R::BufferArray(&vertices)),
                    (3, R::BufferArray(&indices)),
                    (4, R::BufferArray(&tangents)),
                    (5, table.value.as_entire_binding()),
                    (6, R::BufferArray(&previous)),
                ],
            );
            let next = Geometry {
                keys,
                _placements: table,
                models,
                normals,
                instances: vec![],
                arrays,
                materials: None,
                material_key: vec![],
            };
            let state = self.view.primary.as_mut().unwrap();
            state.geometry_generation += 1;
            if let Some(old) = state.geometry.replace(next) {
                state
                    .retired
                    .push((self.submissions, Retired::Geometry(old)));
            }
        }
        // Instance edits keep the tables; only their transforms are republished.
        let mut instances = Vec::with_capacity(placements.len() * 192);
        for d in draws {
            for model in &d.instances.state.models {
                instances.extend_from_slice(&matrix_bytes(*model));
            }
        }
        for d in draws {
            for model in d
                .instances
                .state
                .previous_models(self.view_key.0, self.view.last_frame)
            {
                instances.extend_from_slice(&matrix_bytes(*model));
            }
        }
        for d in draws {
            for normal in &d.instances.state.normals {
                instances.extend_from_slice(&normal.bytes());
            }
        }
        let g = self
            .view
            .primary
            .as_mut()
            .unwrap()
            .geometry
            .as_mut()
            .unwrap();
        if g.instances != instances {
            let (models, normals) = instances.split_at(instances.len() / 3 * 2);
            if !models.is_empty() {
                self.queue.write_buffer(&g.models.value, 0, models);
                self.queue.write_buffer(&g.normals.value, 0, normals);
                uploads.models += models.len() as u64;
                uploads.normals += normals.len() as u64;
                uploads.write_calls += 2;
            }
            g.instances = instances;
        }
        self.prepare_primary_materials(draws, uploads);
        // M7-R1n: non-RT frames trace reflections in software.
        if self.hardware_shadows.is_none() {
            self.prepare_software_reflections(draws, uploads);
        }
        // Only traced Interactive visibility dispatches deferred edge repair. Other
        // modes bind ABI-sized placeholders; switching back restores full capacity.
        let repair_bytes = 16 + u64::from(size.width) * u64::from(size.height) * 36;
        let retry_bytes = 16 + u64::from(size.width) * u64::from(size.height) * 4;
        let defer_edges = self.primary_mode == PrimaryVisibility::Traced
            && self.view.soft_mode == SoftShadows::Interactive
            && repair_bytes <= self.device.limits().max_storage_buffer_binding_size
            && repair_bytes <= self.device.limits().max_buffer_size;
        // The held budget's history: traced frames, and (U-FIXED) visibility-buffer RT frames.
        let wants_held = self.primary_mode == PrimaryVisibility::Traced
            || (self.primary_mode == PrimaryVisibility::VisibilityBuffer
                && self.hardware_shadows.is_some());
        if self
            .view
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .is_none_or(|t| {
                t.hit.texture.size() != size
                    || t.defer_edges != defer_edges
                    || t.held.is_some() != wants_held
            })
        {
            let target = |label, size: wgpu::Extent3d, format: wgpu::TextureFormat| {
                let texture = self.ledger.own(
                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::STORAGE_BINDING
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | if cfg!(test) {
                                wgpu::TextureUsages::COPY_SRC
                            } else {
                                wgpu::TextureUsages::empty()
                            },
                        view_formats: &[],
                    }),
                    ResourceKind::Texture,
                );
                let view = self.ledger.own(
                    texture.create_view(&Default::default()),
                    ResourceKind::TextureView,
                );
                let bytes = u64::from(size.width)
                    * u64::from(size.height)
                    * u64::from(format.block_copy_size(None).unwrap());
                Depth {
                    texture,
                    view,
                    _allocation: self.ledger.allocate(AllocationKind::Target, bytes),
                }
            };
            let tiles = wgpu::Extent3d {
                width: size.width.div_ceil(TRACE_TILE[0]),
                height: size.height.div_ceil(TRACE_TILE[1]),
                depth_or_array_layers: 1,
            };
            let repairs = Buffer::new(
                self,
                "primary edge worklist",
                if defer_edges { repair_bytes } else { 52 },
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_SRC,
            );
            let repair_args = Buffer::new(
                self,
                "primary edge dispatch arguments",
                16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            );
            let retries = Buffer::new(
                self,
                "primary sparse fallback and changed receivers",
                if defer_edges { retry_bytes } else { 20 },
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_SRC,
            );
            let penumbra = Buffer::new(
                self,
                "P-RTS penumbra list",
                if defer_edges && temporal_reconstruction::PENUMBRA_LISTING {
                    retry_bytes
                } else {
                    20
                },
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            );
            let held = wants_held.then(|| HeldTargets {
                history: ["held shadow history A", "held shadow history B"]
                    .map(|label| target(label, size, wgpu::TextureFormat::R32Uint)),
                frame: Buffer::new(
                    self,
                    "held shadow budget record",
                    temporal_reconstruction::HELD_BYTES as u64,
                    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                ),
            });
            let rl_history = {
                let layers = wgpu::Extent3d {
                    depth_or_array_layers: RL_HISTORY_LAYERS,
                    ..size
                };
                let texture = self.ledger.own(
                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("reflection history"),
                        size: layers,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::R32Uint,
                        usage: wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    }),
                    ResourceKind::Texture,
                );
                let view = self.ledger.own(
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        ..Default::default()
                    }),
                    ResourceKind::TextureView,
                );
                let bytes = u64::from(size.width) * u64::from(size.height) * 4 * u64::from(RL_HISTORY_LAYERS);
                RlHistoryTargets {
                    history: Depth {
                        texture,
                        view,
                        _allocation: self.ledger.allocate(AllocationKind::Target, bytes),
                    },
                    frame: Buffer::new(
                        self,
                        "reflection history record",
                        temporal_reconstruction::RL_HISTORY_BYTES as u64,
                        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    ),
                    phase: Buffer::new(
                        self,
                        "reflection shading phase",
                        16,
                        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    ),
                    phases: {
                        let phases = Buffer::new(
                            self,
                            "reflection shading phases",
                            32,
                            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                        );
                        let words: Vec<u8> = [0u32, 0, 0, 0, 1, 0, 0, 0].iter().flat_map(|w| w.to_ne_bytes()).collect();
                        self.queue.write_buffer(&phases.value, 0, &words);
                        phases
                    },
                }
            };
            let next = Targets {
                repairs,
                repair_args,
                retries,
                penumbra,
                defer_edges,
                held,
                rl_history,
                rl_pass: reflection_pass::on().then(|| self.rl_pass_targets(size)),
                gi_receiver: gi_receiver_pass::on().then(|| self.gi_receiver_targets(size)),
                hit: target("primary hit records", size, wgpu::TextureFormat::Rgba32Uint),
                epsilon: target(
                    "primary hard receiver epsilon",
                    size,
                    wgpu::TextureFormat::R32Float,
                ),
                tiles: target("primary tile coverage", tiles, wgpu::TextureFormat::R32Uint),
                deferred: crate::program::rl_deferred().then(|| {
                    ["deferred reflectance", "deferred untraced specular"]
                        .map(|label| target(label, size, wgpu::TextureFormat::Rgba16Float))
                }),
            };
            let state = self.view.primary.as_mut().unwrap();
            if let Some(old) = state.targets.replace(next) {
                state
                    .retired
                    .push((self.submissions, Retired::Targets(Box::new(old))));
            }
        }
        self.prepare_program_lists(programs, &slots, [size.width, size.height], uploads);
        let state = self.view.primary.as_ref().unwrap();
        let t = state.targets.as_ref().unwrap();
        let hdr = &self.view.hdr.as_ref().unwrap().resolved;
        let fresh = soft.then(|| self.view.fresh.as_ref().unwrap().targets.as_ref().unwrap());
        let lists = state.lists.as_ref();
        let key = (
            (*t.hit.texture).clone(),
            (*hdr.texture).clone(),
            fresh.map(|f| (*f.world.texture).clone()),
            scene_generation,
            lists.map_or(0, |l| l.generation),
            state.reflections.as_ref().map_or(0, |r| r.generation),
        );
        if state.bindings.as_ref().is_none_or(|b| b.key != key) {
            let reconstruction_changed = state
                .bindings
                .as_ref()
                .is_none_or(|b| b.key.0 != key.0 || b.key.1 != key.1 || b.key.2 != key.2);
            use wgpu::BindingResource as R;
            let hardware = self.hardware_shadows.as_ref();
            let frame = &state.frame.as_ref().unwrap().0.value;
            let region = &state.region_buffer.as_ref().unwrap().value;
            let group = |layout: &wgpu::BindGroupLayout, entries: &[(u32, R<'_>)]| {
                let entries: Vec<_> = entries
                    .iter()
                    .filter(|(binding, _)| {
                        hardware.is_some() || ![2, TRACE_SCENE].contains(binding)
                    })
                    .map(|(binding, resource)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: resource.clone(),
                    })
                    .collect();
                self.ledger.own(
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("primary frame bindings"),
                        layout,
                        entries: &entries,
                    }),
                    ResourceKind::BindGroup,
                )
            };
            let camera = (0, self.uniform.as_entire_binding());
            let scene = (
                2,
                hardware.map_or_else(|| frame.as_entire_binding(), |h| h.scene().as_binding()),
            );
            let trace_scene = (TRACE_SCENE, scene.1.clone());
            let uniform = (10, frame.as_entire_binding());
            let hit = R::TextureView(&t.hit.view);
            let tiles = R::TextureView(&t.tiles.view);
            let trace = [
                Some(group(
                    &state.trace_bindings[0],
                    &[
                        trace_scene.clone(),
                        uniform.clone(),
                        (13, hit.clone()),
                        (20, tiles.clone()),
                        (23, R::TextureView(&t.epsilon.view)),
                    ],
                )),
                fresh.map(|f| {
                    group(
                        &state.trace_bindings[1],
                        &[
                            camera.clone(),
                            trace_scene.clone(),
                            uniform.clone(),
                            (13, hit.clone()),
                            (20, tiles.clone()),
                            (16, R::TextureView(&f.world.view)),
                            (17, R::TextureView(&f.normal.view)),
                        ],
                    )
                }),
            ];
            let reflection_scene = scene.clone();
            let reflection_geometry = hardware
                .is_none()
                .then(|| &*state.reflections.as_ref().unwrap().buffer.value);
            let shade = |shading: Shading,
                         visibility: (u32, R<'_>)|
             -> [Option<Owned<wgpu::BindGroup>>; 2] {
                let mut entries = vec![
                    camera.clone(),
                    (7, hit.clone()),
                    uniform.clone(),
                    (18, R::TextureView(&hdr.view)),
                    (21, tiles.clone()),
                    (22, region.as_entire_binding()),
                    visibility,
                    (RL_HISTORY_FRAME, t.rl_history.frame.value.as_entire_binding()),
                    (RL_HISTORY, R::TextureView(&t.rl_history.history.view)),
                    (RL_PHASE, t.rl_history.phase.value.as_entire_binding()),
                ];
                if let Some(gi) = &t.gi_receiver { entries.push((gi_receiver_pass::BINDING, gi.values.value.as_entire_binding())); }
                // U-RTVIS: the reflection pass's lobe output.
                if let Some(pass) = &t.rl_pass {
                    entries.push((reflection_pass::LOBE_A, R::TextureView(&pass.lobe_a.view)));
                    entries.push((reflection_pass::LOBE_B, R::TextureView(&pass.lobe_b.view)));
                    entries.push((reflection_pass::PASS_FRAME, pass.frame.value.as_entire_binding()));
                    // The Mac thread's reflection_hits reads H and writes R.
                    entries.push((super::reflection_hits::HITS, pass.hits.value.as_entire_binding()));
                    entries.push((super::reflection_hits::SAMPLES, R::TextureView(&pass.samples.view)));
                }
                if matches!(shading, Shading::Hard) {
                    entries.push((23, R::TextureView(&t.epsilon.view)));
                } else {
                    // M7-R1: reflection rays (shaders/reflections.wgsl).
                    entries.push(reflection_scene.clone());
                }
                // M7-R1n: software reflection geometry on non-RT renderers.
                if let Some(geometry) = &reflection_geometry {
                    entries.push((SOFTWARE_REFLECTIONS, geometry.as_entire_binding()));
                }
                // Deferred reflections' outputs (`Targets::deferred`).
                if let Some([reflectance, untraced]) = &t.deferred {
                    entries.push((62, R::TextureView(&reflectance.view)));
                    entries.push((63, R::TextureView(&untraced.view)));
                }
                // MP1: the same group plus the slot table and tile lists.
                let list = lists.map(|l| {
                    let mut entries = entries.clone();
                    entries.extend(l.shade_entries());
                    group(state.list_shade_layout(shading), &entries)
                });
                [
                    Some(group(&state.shade_bindings[shading as usize], &entries)),
                    list,
                ]
            };
            let [shade, list] = match fresh {
                Some(f) => {
                    let [s, l] = shade(Shading::Soft, (6, R::TextureView(&f.visibility.view)));
                    [[None, s], [None, l]]
                }
                None => {
                    let [s, l] = shade(Shading::Hard, scene.clone());
                    [[s, None], [l, None]]
                }
            };
            let next = Bindings {
                trace,
                fresh: fresh.map(|f| {
                    group(
                        &state.fresh_bindings,
                        &[
                            scene.clone(),
                            (3, R::TextureView(&f.world.view)),
                            (4, R::TextureView(&f.normal.view)),
                            (5, R::TextureView(&f.visibility.view)),
                            (7, hit.clone()),
                            (21, tiles.clone()),
                        ],
                    )
                }),
                interactive: fresh.map(|f| {
                    group(
                        &state.interactive_bindings,
                        &[
                            trace_scene.clone(),
                            scene.clone(),
                            uniform.clone(),
                            (13, hit.clone()),
                            (5, R::TextureView(&f.visibility.view)),
                            (20, tiles.clone()),
                            (24, t.repairs.value.as_entire_binding()),
                            (27, t.repair_args.value.as_entire_binding()),
                        ],
                    )
                }),
                held: fresh.zip(t.held.as_ref()).map(|(f, h)| {
                    [0, 1].map(|k| {
                        group(
                            &state.held_bindings,
                            &[
                                trace_scene.clone(),
                                scene.clone(),
                                uniform.clone(),
                                (13, hit.clone()),
                                (5, R::TextureView(&f.visibility.view)),
                                (20, tiles.clone()),
                                (24, t.repairs.value.as_entire_binding()),
                                (27, t.repair_args.value.as_entire_binding()),
                                (50, h.frame.value.as_entire_binding()),
                                (51, R::TextureView(&h.history[1 - k].view)),
                                (52, R::TextureView(&h.history[k].view)),
                            ],
                        )
                    })
                }),
                vb_sun: fresh.zip(t.held.as_ref()).map(|(f, h)| {
                    [0, 1].map(|k| {
                        group(
                            &state.vb_sun_bindings,
                            &[
                                scene.clone(),
                                (3, R::TextureView(&f.world.view)),
                                (4, R::TextureView(&f.normal.view)),
                                (5, R::TextureView(&f.visibility.view)),
                                (7, hit.clone()),
                                uniform.clone(),
                                (21, tiles.clone()),
                                (50, h.frame.value.as_entire_binding()),
                                (51, R::TextureView(&h.history[1 - k].view)),
                                (52, R::TextureView(&h.history[k].view)),
                            ],
                        )
                    })
                }),
                repair: fresh.map(|f| {
                    group(
                        &state.repair_bindings,
                        &[
                            trace_scene.clone(),
                            scene.clone(),
                            uniform.clone(),
                            (13, hit.clone()),
                            (5, R::TextureView(&f.visibility.view)),
                            (20, tiles.clone()),
                            (26, t.repairs.value.as_entire_binding()),
                            (28, t.retries.value.as_entire_binding()),
                        ],
                    )
                }),
                check: fresh.map(|_| {
                    group(
                        &state.check_bindings,
                        &[
                            uniform.clone(),
                            (24, t.repairs.value.as_entire_binding()),
                            (28, t.retries.value.as_entire_binding()),
                        ],
                    )
                }),
                penumbra: fresh.filter(|_| hardware.is_some()).map(|f| {
                    [
                        group(
                            &state.penumbra_list_bindings,
                            &[
                                (6, R::TextureView(&f.visibility.view)),
                                (7, hit.clone()),
                                (21, tiles.clone()),
                                (29, t.penumbra.value.as_entire_binding()),
                            ],
                        ),
                        group(
                            &state.penumbra_trace_bindings,
                            &[
                                trace_scene.clone(),
                                scene.clone(),
                                uniform.clone(),
                                (5, R::TextureView(&f.visibility.view)),
                                (29, t.penumbra.value.as_entire_binding()),
                            ],
                        ),
                    ]
                }),
                shade,
                list,
                // U-RTVIS: rl_trace's group (reflection_pass.rs).
                rl_trace: state.rl_trace.as_ref().zip(t.rl_pass.as_ref()).map(|(abi, pass)| {
                    let geometry = match &reflection_geometry {
                        Some(g) => (SOFTWARE_REFLECTIONS, g.as_entire_binding()),
                        None => reflection_scene.clone(),
                    };
                    self.rl_trace_group(abi, pass, frame.as_entire_binding(), geometry)
                }),
                rl_resolve: state
                    .rl_resolve
                    .as_ref()
                    .map(|kit| self.rl_resolve_group(kit, &t.rl_history)),
                key,
            };
            let state = self.view.primary.as_mut().unwrap();
            state.bindings_generation += 1;
            state.reconstruction_bindings_generation += u64::from(reconstruction_changed);
            if let Some(old) = state.bindings.replace(next) {
                state
                    .retired
                    .push((self.submissions, Retired::Bindings(old)));
            }
        }
        self.prepare_bins_group();
        Ok(())
    }

    /// Closest-hit camera rays, plus exact fresh receivers when `soft`: one dispatch.
    pub(super) fn encode_primary_visibility(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        soft: bool,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let p = self.pipeline.primary.as_ref().unwrap();
        let pipeline = &p.trace[usize::from(soft)];
        self.encode_primary_trace(encoder, pipeline, TRACE_TILE, soft, timestamp_writes);
    }
    /// The trace dispatch with `pipeline` over the image; tests screen variant kernels on
    /// one ABI.
    pub(super) fn encode_primary_trace(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        soft: bool,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.view.primary.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let bindings = state.bindings.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced primary visibility"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(
            0,
            &**bindings.trace[usize::from(soft)].as_ref().unwrap(),
            &[],
        );
        pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
        pass.dispatch_workgroups(
            size.width.div_ceil(tile[0]),
            size.height.div_ceil(tile[1]),
            1,
        );
    }

    /// The production 256-ray estimator over the traced receivers, coverage from hits.
    pub(super) fn encode_primary_fresh(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        // U-FIXED: visibility-buffer RT frames' Interactive sun from the buffer's receivers.
        if self.primary_mode == PrimaryVisibility::VisibilityBuffer
            && self.hardware_shadows.is_some()
            && self.view.soft_mode == SoftShadows::Interactive
        {
            self.encode_vb_sun(encoder, timestamp_writes);
            return;
        }
        // fresh_compute's 8-direction variant covers 8x4 receivers per workgroup.
        let tile = if rt_primary_switch() == Some(RtPrimarySun::Rays(8)) {
            [8, 4]
        } else {
            fresh_shadow_gpu::TILE
        };
        #[cfg(test)]
        let tile = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .test_fresh_tile
            .unwrap_or(tile);
        self.encode_primary_fresh_tile(encoder, pipeline, tile, timestamp_writes);
    }

    /// Visibility-buffer RT frames' Interactive sun: `primary_vb_sun` on reconstructed frames
    /// (TR1's budget, rotation, cold and held stops, history `parity`), else the 8-direction
    /// fresh prefix. Both read the receivers the buffer's resolve wrote.
    fn encode_vb_sun(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let p = self.pipeline.primary.as_ref().unwrap();
        let state = self.view.primary.as_ref().unwrap();
        let reconstructed = p
            .reconstruction
            .as_ref()
            .and_then(|r| r.vb_sun.as_deref())
            .filter(|_| self.view.reconstruction_frame.is_some())
            .zip(state.bindings.as_ref().and_then(|b| b.vb_sun.as_ref()))
            .zip(self.view.reconstruction_state.as_ref());
        let Some(((pipeline, groups), r)) = reconstructed else {
            self.encode_primary_fresh_tile(encoder, &p.fresh_eight, [8, 4], timestamp_writes);
            return;
        };
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("visibility-buffer RT sun"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*groups[r.plan.parity], &[]);
        pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(4), 1);
    }

    /// Interactive soft frames: camera rays, hit records and 8-direction counts in one
    /// dispatch over the image, replacing the trace and fresh visibility passes.
    /// Production passes `TRACE_TILE`; tests screen other workgroup shapes on the ABI.
    pub(super) fn encode_primary_interactive(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let p = self.pipeline.primary.as_ref().unwrap();
        let kernels = InteractiveKernels {
            check: p.interactive_repair.as_deref(),
            repair: p.interactive_repair.as_deref(),
            retry: p.interactive_retry.as_deref(),
            full: p.interactive_full.as_deref(),
            penumbra: None,
            group: None,
        };
        self.encode_interactive_kernels(
            encoder,
            pipeline,
            kernels,
            tile,
            timestamp_writes,
            &mut |_, _| {},
        );
    }
    /// The reconstruction's rotated Interactive kernels on production's passes and groups;
    /// the near-edge check traces no direction, so it stays production's. `mark` names each
    /// pass after the fused trace as a `visibility.*` part (`RENDERLAB_PASS_LOG_DETAIL`).
    pub(super) fn encode_reconstruction_interactive(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        let p = self.pipeline.primary.as_ref().unwrap();
        let t = p.reconstruction.as_ref().unwrap().trace.as_ref().unwrap();
        // P-RTS: the probe kernels traced fewer directions than the budget only when the
        // frame words say so (ReconstructionState::probe_rays).
        let listed = self
            .view
            .reconstruction_state
            .as_ref()
            .is_some_and(|s| s.plan.probe_rays < s.plan.shadow_rays);
        let kernels = InteractiveKernels {
            check: p.interactive_repair.as_deref(),
            repair: t.repair.as_deref(),
            retry: t.retry.as_deref(),
            full: t.full.as_deref(),
            penumbra: t
                .penumbra
                .as_ref()
                .filter(|_| listed)
                .map(|[list, trace]| [&**list, &**trace]),
            // U-RTVIS: the held trace entries write history `parity` and read the other.
            group: self
                .view
                .primary
                .as_ref()
                .and_then(|s| s.bindings.as_ref()?.held.as_ref())
                .zip(self.view.reconstruction_state.as_ref())
                .map(|(groups, r)| &*groups[r.plan.parity]),
        };
        self.encode_interactive_kernels(
            encoder,
            &t.trace,
            kernels,
            TRACE_TILE,
            timestamp_writes,
            mark,
        );
    }
    fn encode_interactive_kernels(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        kernels: InteractiveKernels<'_>,
        tile: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        let state = self.view.primary.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let bindings = state.bindings.as_ref().unwrap();
        let targets = state.targets.as_ref().unwrap();
        let deferred = targets.defer_edges && kernels.check.is_some();
        #[cfg(not(test))]
        let split_timestamps = false;
        #[cfg(test)]
        let split_timestamps = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .test_split_interactive_timestamps;
        let pipeline = if deferred {
            pipeline
        } else {
            kernels.full.unwrap_or(pipeline)
        };
        if deferred {
            encoder.clear_buffer(&targets.repairs.value, 0, Some(16));
            encoder.clear_buffer(&targets.repair_args.value, 0, Some(16));
            encoder.clear_buffer(&targets.retries.value, 0, Some(16));
        }
        let penumbra = kernels
            .penumbra
            .filter(|_| deferred && kernels.retry.is_none());
        if penumbra.is_some() {
            encoder.clear_buffer(&targets.penumbra.value, 0, Some(16));
        }
        let begin = timestamp_writes
            .as_ref()
            .map(|t| wgpu::ComputePassTimestampWrites {
                query_set: t.query_set,
                beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                end_of_pass_write_index: if deferred && !split_timestamps {
                    None
                } else {
                    t.end_of_pass_write_index
                },
            });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced Interactive visibility"),
            timestamp_writes: begin,
        });
        pass.set_pipeline(pipeline);
        let group = kernels
            .group
            .unwrap_or_else(|| bindings.interactive.as_ref().unwrap());
        pass.set_bind_group(0, group, &[]);
        pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(
            size.width.div_ceil(tile[0]),
            size.height.div_ceil(tile[1]),
            1,
        );
        drop(pass);
        if deferred && kernels.retry.is_none() {
            // The frame's span ends on its last pass: the repair, or P-RTS's trace.
            let end = |t: &wgpu::ComputePassTimestampWrites<'_>| {
                t.end_of_pass_write_index
                    .map(|i| i + if split_timestamps { 2 } else { 0 })
            };
            let stamps = timestamp_writes
                .as_ref()
                .map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: if split_timestamps {
                        t.beginning_of_pass_write_index.map(|i| i + 2)
                    } else {
                        None
                    },
                    end_of_pass_write_index: if penumbra.is_some() { None } else { end(t) },
                })
                .filter(|s| {
                    s.beginning_of_pass_write_index.is_some() || s.end_of_pass_write_index.is_some()
                });
            mark(encoder, "visibility.repair");
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained two-pass receiver correction"),
                timestamp_writes: stamps,
            });
            pass.set_pipeline(kernels.repair.unwrap());
            pass.set_bind_group(0, &**bindings.repair.as_ref().unwrap(), &[]);
            pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            pass.dispatch_workgroups_indirect(&targets.repairs.value, 0);
            drop(pass);
            if let Some([list, trace]) = penumbra {
                let [list_group, trace_group] = bindings.penumbra.as_ref().unwrap();
                mark(encoder, "visibility.penumbra");
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("P-RTS penumbra list"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(list);
                pass.set_bind_group(0, &**list_group, &[]);
                pass.dispatch_workgroups(
                    size.width.div_ceil(TRACE_TILE[0]),
                    size.height.div_ceil(TRACE_TILE[1]),
                    1,
                );
                drop(pass);
                let stamps = timestamp_writes.as_ref().and_then(|t| {
                    end(t).map(|i| wgpu::ComputePassTimestampWrites {
                        query_set: t.query_set,
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(i),
                    })
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("P-RTS full-budget penumbra visibility"),
                    timestamp_writes: stamps,
                });
                pass.set_pipeline(trace);
                pass.set_bind_group(0, &**trace_group, &[]);
                pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
                pass.set_bind_group(3, self.lighting.bind(), &[]);
                pass.dispatch_workgroups_indirect(&targets.penumbra.value, 0);
            }
            return;
        }

        if deferred {
            let end = timestamp_writes
                .as_ref()
                .filter(|_| split_timestamps)
                .map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: if split_timestamps {
                        t.beginning_of_pass_write_index.map(|i| i + 2)
                    } else {
                        None
                    },
                    end_of_pass_write_index: if split_timestamps {
                        t.end_of_pass_write_index.map(|i| i + 2)
                    } else {
                        None
                    },
                });
            mark(encoder, "visibility.check");
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("near-edge arithmetic check"),
                timestamp_writes: end,
            });
            pass.set_pipeline(kernels.check.unwrap());
            pass.set_bind_group(0, &**bindings.check.as_ref().unwrap(), &[]);
            pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
            pass.dispatch_workgroups_indirect(&targets.repair_args.value, 0);
            drop(pass);
            let stamps = timestamp_writes.map(|t| wgpu::ComputePassTimestampWrites {
                query_set: t.query_set,
                beginning_of_pass_write_index: if split_timestamps {
                    t.beginning_of_pass_write_index.map(|i| i + 4)
                } else {
                    None
                },
                end_of_pass_write_index: t
                    .end_of_pass_write_index
                    .map(|i| i + if split_timestamps { 4 } else { 0 }),
            });
            mark(encoder, "visibility.retry");
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sparse fallback and replacement visibility"),
                timestamp_writes: stamps,
            });
            pass.set_pipeline(kernels.retry.unwrap());
            pass.set_bind_group(0, &**bindings.repair.as_ref().unwrap(), &[]);
            pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            pass.dispatch_workgroups_indirect(&targets.retries.value, 0);
        }
    }

    /// Fresh visibility over the traced receivers with `tile` receivers per group; tests
    /// screen the separate 8-direction layouts with it.
    pub(super) fn encode_primary_fresh_tile(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.view.primary.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced fresh visibility"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        let bindings = state.bindings.as_ref().unwrap();
        pass.set_bind_group(0, &**bindings.fresh.as_ref().unwrap(), &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(
            size.width.div_ceil(tile[0]),
            size.height.div_ceil(tile[1]),
            1,
        );
    }

    /// Group 2 of traced shading: every slot's material factors, current texture levels,
    /// samplers and parameters, plus the residency words the sampling helpers decode.
    /// Rebuilt only when a bound texture or its resident range changes.
    pub(super) fn prepare_primary_materials(
        &mut self,
        draws: &[PreparedDraw<'_>],
        uploads: &mut UploadBytes,
    ) {
        use crate::material::TextureStorage;
        use texture_residency::{SLOTS, WORDS};
        let requests = self.view.primary.as_ref().unwrap().requests;
        if requests && self.view.primary.as_ref().unwrap().residency.is_none() {
            let buffer = Buffer::new(
                self,
                "texture residency words and requests",
                (WORDS * 4) as u64,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );
            let (probe, view, allocation) = self.level_probe();
            self.view.primary.as_mut().unwrap().residency = Some(ResidencyGpu {
                buffer,
                _probe: probe,
                probe: view,
                _probe_allocation: allocation,
                // No word is ever all ones, so the first words are always written.
                words: vec![u32::MAX; SLOTS],
                ids: vec![None; SLOTS],
            });
        }
        let default = self.default_material.as_ref().unwrap();
        let used: Vec<&Material> = draws
            .iter()
            .map(|d| d.material.unwrap_or(default))
            .chain(std::iter::repeat(default))
            .take(TRACED_DRAWS as usize)
            .collect();
        // Whether group 2 binds the residency state or its stand-ins.
        let mut key = vec![(usize::MAX, u64::from(requests))];
        let mut words = Vec::with_capacity(SLOTS);
        let mut ids = Vec::with_capacity(SLOTS);
        let survey = self.residency.survey;
        for m in &used {
            for (t, &margin) in m._textures.iter().zip(&m.margins) {
                match t.0.storage {
                    TextureStorage::Full(_) => {
                        key.push((Arc::as_ptr(&t.0) as usize, 0));
                        words.push(0);
                        ids.push(None);
                    }
                    TextureStorage::Streamed(id) => {
                        let e = self.residency.entry(id);
                        key.push((Arc::as_ptr(&t.0) as usize, e.generation));
                        words.push(e.word(margin, survey));
                        ids.push(Some(id));
                    }
                }
            }
        }
        let state = self.view.primary.as_ref().unwrap();
        let residency = state.residency.as_ref().filter(|_| requests);
        if let Some(residency) = residency
            && residency.words != words
        {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
            self.queue.write_buffer(&residency.buffer.value, 0, &bytes);
            uploads.materials += bytes.len() as u64;
            uploads.write_calls += 1;
        }

        let g = state.geometry.as_ref().unwrap();
        let materials = if g.materials.is_none() || g.material_key != key {
            use wgpu::BindingResource as R;
            let factors: Vec<_> = used
                .iter()
                .map(|m| m._uniform.as_entire_buffer_binding())
                .collect();
            let views: [Vec<&wgpu::TextureView>; 3] = std::array::from_fn(|i| {
                used.iter()
                    .map(|m| self.texture_view(&m._textures[i]))
                    .collect()
            });
            let samplers: [Vec<&wgpu::Sampler>; 3] =
                std::array::from_fn(|i| used.iter().map(|m| &*m._samplers[i]).collect());
            let parameters: Vec<_> = used
                .iter()
                .map(|m| m.parameters.buffer.as_entire_buffer_binding())
                .collect();
            let mut entries: Vec<_> = [
                (10, R::BufferArray(&factors)),
                (11, R::TextureViewArray(&views[0])),
                (12, R::SamplerArray(&samplers[0])),
                (13, R::TextureViewArray(&views[1])),
                (14, R::SamplerArray(&samplers[1])),
                (15, R::TextureViewArray(&views[2])),
                (16, R::SamplerArray(&samplers[2])),
                (17, R::BufferArray(&parameters)),
            ]
            .into_iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
            .collect();
            if let Some(r) = residency {
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 18,
                        resource: r.buffer.value.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 19,
                        resource: R::TextureView(&r.probe),
                    },
                ]);
            }
            Some(self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("primary draw materials"),
                    layout: if residency.is_some() {
                        &state.streamed_bindings
                    } else {
                        &state.material_bindings
                    },
                    entries: &entries,
                }),
                ResourceKind::BindGroup,
            ))
        } else {
            None
        };
        let state = self.view.primary.as_mut().unwrap();
        if let Some(residency) = state.residency.as_mut().filter(|_| requests) {
            residency.words = words;
            residency.ids = ids;
        }
        let g = state.geometry.as_mut().unwrap();
        if let Some(materials) = materials {
            // Queued frames keep the group they recorded.
            g.materials = Some(materials);
            g.material_key = key;
        }
    }
    /// Zeroes the frame's request and miss counters before its shading.
    pub(super) fn encode_request_clear(&self, encoder: &mut wgpu::CommandEncoder) {
        let residency = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .residency
            .as_ref()
            .unwrap();
        encoder.clear_buffer(
            &residency.buffer.value,
            texture_residency::REQUEST_OFFSET,
            Some(texture_residency::REQUEST_BYTES),
        );
    }

    /// Deferred shading of the scene region into the HDR target with the frame's program,
    /// and the clear colour everywhere else, in one dispatch; per program in mixed frames.
    pub(super) fn encode_primary_shading(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.view.primary.as_ref().unwrap();
        if state.mixed() {
            self.encode_program_lists(encoder, shading, timestamp_writes);
            return;
        }
        let program = self.program_slot(state.program);
        let set = if state.requests {
            &program.pipelines.streamed
        } else {
            &program.pipelines.traced
        };
        // The lean module once it landed; until then the full one, the same picture.
        let pipeline = self
            .lean_pipeline(state.program, false, shading as usize)
            .unwrap_or_else(|| set[shading as usize].as_ref().unwrap());
        self.encode_primary_shade(encoder, pipeline, SHADE_TILE, shading, timestamp_writes);
    }
    /// The shading pass: `pipeline` over the scene region by `tile`, plus clear blocks
    /// for the rest of the image. Tests screen variant shading kernels on one ABI.
    pub(super) fn encode_primary_shade(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        shading: Shading,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let bindings = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .bindings
            .as_ref()
            .unwrap();
        let group = bindings.shade[shading as usize].as_ref().unwrap();
        self.encode_shade_group(encoder, pipeline, group, tile, timestamp_writes);
    }
    /// `encode_primary_shade` with an explicit group 0, which temporal shading extends.
    pub(super) fn encode_shade_group(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        group: &wgpu::BindGroup,
        tile: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.view.primary.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced deferred shading"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_bind_group(1, &*g.arrays, &[]);
        pass.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        // Rows of region shading groups, then rows of CLEAR_TILE clear blocks over the
        // image, as WGSL primary_shade_tiles decodes them.
        let [x, y] = state.region_groups(tile);
        let [bx, by] = [0, 1].map(|i| [size.width, size.height][i].div_ceil(CLEAR_TILE[i]));
        pass.dispatch_workgroups(x.max(bx), y + by, 1);
    }
}

// ---- M7-R1n software reflections (docs/m7-r1n.md) ----
/// Non-RT frames' packed software reflection geometry (`crate::reflection_software`): each
/// distinct drawn mesh's BVH once, behind the frame's instance records. Repacked when the
/// drawn meshes or the number of traced instances change; transform edits rewrite only the
/// records.
pub(super) struct SoftwareReflections {
    buffer: Buffer,
    meshes: Vec<Arc<crate::reflection_software::Blas>>,
    bases: Vec<[u32; 2]>,
    records: Vec<u32>,
    /// Counts repacks; shading groups rebind on a change.
    generation: u64,
}
impl Renderer {
    fn prepare_software_reflections(
        &mut self,
        draws: &[PreparedDraw<'_>],
        uploads: &mut UploadBytes,
    ) {
        let mut meshes: Vec<Arc<crate::reflection_software::Blas>> = vec![];
        let mut traced = vec![];
        let mut slot = 0u32;
        for d in draws {
            // Skinned meshes neither appear in nor block non-RT reflection rays (gap).
            let rigid = d.mesh.reflection.as_ref().filter(|_| d.mesh.skin.is_none());
            let mesh = rigid.map(|b| {
                meshes
                    .iter()
                    .position(|m| Arc::ptr_eq(m, b))
                    .unwrap_or_else(|| {
                        meshes.push(b.clone());
                        meshes.len() - 1
                    })
            });
            let state = &d.instances.state;
            for (i, model) in state.models.iter().enumerate() {
                if let Some(mesh) = mesh {
                    traced.push((slot, *model, mesh, state.casts_shadows(i)));
                }
                slot += 1;
            }
        }
        let state = self.view.primary.as_ref().unwrap();
        let same = state.reflections.as_ref().is_some_and(|r| {
            r.meshes.len() == meshes.len()
                && r.meshes.iter().zip(&meshes).all(|(a, b)| Arc::ptr_eq(a, b))
                && r.records.len() == crate::reflection_software::records_words(traced.len())
        });
        let previous = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .reflections
            .as_ref()
            .map_or(vec![], |r| r.records.clone());
        let records = |bases: &[[u32; 2]]| {
            let rows: Vec<_> = traced
                .iter()
                .map(|&(slot, model, mesh, casts)| {
                    (slot, model, &*meshes[mesh], bases[mesh], casts)
                })
                .collect();
            let mut words = crate::reflection_software::instance_words(&rows);
            // Records of singular models stay reserved (and untraced).
            words.resize(
                crate::reflection_software::HEADER_WORDS
                    + traced.len() * crate::reflection_software::INSTANCE_WORDS,
                0,
            );
            // U-RTVIS: the top level over the written records (src/instance_bvh.rs).
            crate::instance_bvh::place(
                &mut words,
                crate::reflection_software::HEADER_WORDS,
                crate::reflection_software::INSTANCE_WORDS,
                traced.len(),
                &previous,
            );
            words
        };
        if same {
            let r = self
                .view
                .primary
                .as_ref()
                .unwrap()
                .reflections
                .as_ref()
                .unwrap();
            let words = records(&r.bases);
            if words != r.records {
                let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
                self.queue.write_buffer(&r.buffer.value, 0, &bytes);
                // MQ2: same meshes and slots, so only placement transforms changed. Count
                // them as transform edits, which TR1 reprojects (MQ1); `geometry` is in
                // TR1's restart key and restarted every non-RT pixel on every animated frame.
                uploads.models += (words.len() * 4) as u64;
                uploads.write_calls += 1;
                self.view
                    .primary
                    .as_mut()
                    .unwrap()
                    .reflections
                    .as_mut()
                    .unwrap()
                    .records = words;
            }
            return;
        }
        let refs: Vec<_> = meshes.iter().map(|m| &**m).collect();
        let mut packed = crate::reflection_software::pack(&refs, traced.len());
        let words = records(&packed.bases);
        packed.words[..words.len()].copy_from_slice(&words);
        let buffer = Buffer::new(
            self,
            "software reflection geometry",
            (packed.words.len() * 4) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let bytes: Vec<u8> = packed.words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        self.queue.write_buffer(&buffer.value, 0, &bytes);
        uploads.geometry += (packed.words.len() * 4) as u64;
        uploads.write_calls += 1;
        let state = self.view.primary.as_mut().unwrap();
        let generation = state.reflections.as_ref().map_or(0, |r| r.generation) + 1;
        state.reflections = Some(SoftwareReflections {
            buffer,
            meshes,
            bases: packed.bases,
            records: words,
            generation,
        });
    }
}
// ---- end of M7-R1n software reflections ----
#[cfg(test)]
#[path = "reflection_software_tests.rs"]
mod reflection_software_tests;
