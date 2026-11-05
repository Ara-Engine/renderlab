//! Traced primary visibility on hardware-shadow 1x renderers: one closest-hit camera
//! ray per pixel against the shadow TLAS, exact fresh receivers from the hit triangle,
//! and deferred compute shading with the unchanged lit body. Draw geometry and
//! materials are descriptor-indexed, so each stage is one dispatch over the image.
use super::*;
use crate::primary_ray::{TRACED_DRAWS, TRACED_FEATURES, placement_table, traced_limits};
use crate::shadow_plan::Bounds;

const LIT_SIGNATURE: &str = "@fragment fn fs_lit(input:VertexOutput, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {";
const PRESENT_SEAM: &str = "@vertex fn vs_present(";
/// WGSL PrimaryFrame: inverse view-projection, lit clear colour, culling flags and the
/// `scene_pixels` rectangle.
const FRAME_BYTES: usize = 112;
/// Pixels per trace workgroup; the WGSL workgroup sizes must match.
pub(super) const TRACE_TILE: [u32; 2] = [8, 4];
/// Pixels per shading workgroup; the WGSL workgroup size must match.
pub(super) const SHADE_TILE: [u32; 2] = [4, 4];
/// Traced fresh visibility takes receiver coverage from the hit record, so misses
/// never write receivers. The estimator itself is the unchanged production body.
const COVERAGE_SEAM: &str = "    if inside {\n        let normal=textureLoad(fresh_normal,p,0);";
const TANGENT_SEAM: &str =
    "@vertex fn vs_tangent(input:VertexInput, @location(5) tangent:vec4<f32>) -> VertexOutput {";

/// Rewrites a composed lit module for compute. fs_lit's body becomes `primary_lit`,
/// taking the receiver plane, explicit UV gradients and the draw's material slot in
/// place of quad derivatives and bound material globals. Every seam must occur once.
fn compute_lit(composed: &str) -> Result<String, Error> {
    let seams = Error::InvalidInput(
        "traced primary shading requires the unchanged lit, tangent and material seams",
    );
    if composed.matches(LIT_SIGNATURE).count() != 1 || composed.matches(TANGENT_SEAM).count() != 1 {
        return Err(seams);
    }
    let (before, rest) = composed.split_once(LIT_SIGNATURE).unwrap();
    let (body, after) = rest.split_once(PRESENT_SEAM).ok_or(Error::InvalidInput(
        "traced primary shading requires the presentation seam after fs_lit",
    ))?;
    let mut body = body.to_owned();
    for (find, replace) in [
        (fresh_shadow_gpu::GEOMETRIC_SEAM, ""),
        (
            "textureSample(base_color,base_sampler,input.uv)",
            "textureSampleGrad(primary_base_color[primary_index],primary_base_sampler[primary_index],input.uv,dx,dy)",
        ),
        (
            "textureSample(metallic_roughness,mr_sampler,input.uv)",
            "textureSampleGrad(primary_metallic_roughness[primary_index],primary_mr_sampler[primary_index],input.uv,dx,dy)",
        ),
        (
            "textureSample(normal_map,normal_sampler,input.uv)",
            "textureSampleGrad(primary_normal_map[primary_index],primary_normal_sampler[primary_index],input.uv,dx,dy)",
        ),
        // The DFG table has one level, so this equals the raster sample.
        (
            "textureSample(dfg,dfg_sampler,uv)",
            "textureSampleLevel(dfg,dfg_sampler,uv,0.0)",
        ),
        // Unreachable: traced frames admit only double-sided materials.
        (
            "if material.double_sided==0u && !source_front { discard; }",
            "if material.double_sided==0u && !source_front { return vec4<f32>(0.0); }",
        ),
    ] {
        if body.matches(find).count() != 1 {
            return Err(seams);
        }
        body = body.replacen(find, replace, 1);
    }
    if [
        "textureSample(",
        "dpdx",
        "dpdy",
        "(base_color,",
        "(metallic_roughness,",
        "(normal_map,",
    ]
    .iter()
    .any(|bound| body.contains(bound))
    {
        return Err(Error::InvalidInput(
            "traced primary shading cannot use implicit derivatives or bound material textures",
        ));
    }
    // The local shadows the bound material uniform for the whole body. Enable
    // directives precede every declaration, including the composition's own.
    Ok(format!(
        "enable wgpu_binding_array;\nenable wgpu_ray_query_vertex_return;\n{before}fn primary_lit(input:VertexOutput,front:bool,geometric:vec3<f32>,dx:vec2<f32>,dy:vec2<f32>,primary_index:u32)->vec4<f32> {{\n    let material=primary_materials[primary_index].value;{body}{PRESENT_SEAM}{after}"
    )
    .replacen(
        TANGENT_SEAM,
        "fn primary_tangent(input:VertexInput,tangent:vec4<f32>)->VertexOutput {",
        1,
    ))
}

/// The hard/unshadowed module (inline `hard_shadow` rays and the trace) and the soft
/// module (fresh visibility counts and their traced dispatch), from one scene source.
pub(super) fn shader_sources(wgsl: &str) -> Result<[String; 2], Error> {
    let rays = format!(
        "const PRIMARY_BATCHES:u32={TRACED_DRAWS}u;\n{}",
        include_str!("../shaders/primary_rays.wgsl")
    );
    let hard = compute_lit(&crate::hardware_shadows::shader_source(
        wgsl,
        SampleCount::One,
    )?)?;
    let soft = compute_lit(&fresh_shadow_gpu::shader_source(wgsl)?)?;
    if soft.matches(COVERAGE_SEAM).count() != 1 {
        return Err(Error::InvalidInput(
            "traced fresh visibility requires the unchanged receiver coverage seam",
        ));
    }
    let soft = soft.replacen(
        COVERAGE_SEAM,
        &COVERAGE_SEAM.replacen(
            "if inside {",
            "if inside && textureLoad(primary_hit,p,0).x!=0u {",
            1,
        ),
        1,
    );
    let trace = include_str!("../shaders/primary_trace.wgsl");
    Ok([
        format!("{hard}\n{rays}\n{trace}"),
        format!(
            "{soft}\n{rays}\n{trace}\n{}",
            include_str!("../shaders/primary_interactive.wgsl")
        ),
    ])
}

/// Identity of the traced modules, recorded on the HDR images they write.
pub(super) fn sources_hash(sources: &[String; 2]) -> [u8; 32] {
    Sha256::new()
        .chain_update(sources[0].as_bytes())
        .chain_update(sources[1].as_bytes())
        .finalize()
        .into()
}

/// Pixels whose camera ray can reach any placement, as `[x0, y0, x1, y1)`: the bounding
/// rectangle of every conservative world bound projected through `matrix`, 2 px
/// outward. A ray outside it misses every triangle; Interactive frames skip those
/// rays. Bounds reaching behind the eye, or a nonfinite projection, give the image.
pub(super) fn scene_pixels(bounds: &[Bounds], matrix: Mat4, [width, height]: [u32; 2]) -> [u32; 4] {
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
pub(super) fn supported(device: &wgpu::Device) -> bool {
    let limits = device.limits();
    device.features().contains(TRACED_FEATURES) && traced_limits(limits.clone()) == limits
}

/// Rejects a traced frame before any frame write: hardware-shadowed frames need opaque
/// double-sided casters, meshes must have BLASes, the draw count must fit and the
/// camera inverse must exist.
pub(super) fn admit_draws(draws: &[Draw<'_>], matrix: Mat4, shadowed: bool) -> Result<(), Error> {
    if shadowed
        && draws
            .iter()
            .any(|d| d.material.is_some_and(|m| !m.double_sided))
    {
        return Err(Error::InvalidInput(
            "traced primary visibility with hardware shadows requires opaque double-sided materials",
        ));
    }
    if draws.iter().any(|d| d.mesh.acceleration.is_none()) {
        return Err(Error::InvalidInput(
            "traced primary visibility requires meshes uploaded to this hardware renderer",
        ));
    }
    placement_table(draws.iter().map(|d| (d.instances.len(), false, false)))?;
    if !matrix.inverse().is_finite() {
        return Err(Error::InvalidInput("traced camera inverse is nonfinite"));
    }
    Ok(())
}

impl Renderer {
    /// Chooses disk-shadow sampling for traced lit 1x frames. Exact is the default.
    /// The preference survives primary-path changes and reload; raster, hard and
    /// diagnostic frames use Exact, as reported by `FrameReport::soft_shadows`.
    pub fn set_soft_shadows(&mut self, mode: crate::SoftShadows) {
        self.soft_mode = mode;
    }

    pub fn soft_shadows(&self) -> crate::SoftShadows {
        self.soft_mode
    }

    /// Selects how lit frames resolve camera visibility. Traced compiles its modules
    /// from the installed source now, transactionally; failure keeps the current mode.
    /// Raster releases traced pipelines and targets after in-flight frames.
    pub fn set_primary_visibility(&mut self, mode: PrimaryVisibility) -> Result<(), Error> {
        self.check_fault()?;
        if mode == self.primary_mode {
            return Ok(());
        }
        match mode {
            PrimaryVisibility::Raster => {
                self.pipeline.primary = None;
                self.primary.as_mut().unwrap().release(self.submissions);
            }
            PrimaryVisibility::Traced => {
                let state = self.primary.as_ref().ok_or(Error::InvalidInput(
                    "traced primary visibility requires a hardware-shadow renderer at 1x with TRACED_FEATURES and traced_limits",
                ))?;
                let sources = shader_sources(&self.source)?;
                let scopes = OperationScopes::new(&self.device);
                let pipelines = state.compile(&self.device, &self.ledger, &sources);
                if let Some(error) = scopes.finish() {
                    return Err(match error {
                        wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                        _ => self.fail(error.to_string()),
                    });
                }
                self.check_fault()?;
                self.pipeline.primary = Some(pipelines);
                self.primary_hash = sources_hash(&sources);
            }
        }
        self.primary_mode = mode;
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
            view_dimension: wgpu::TextureViewDimension::D2,
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
    fn new(r: &Renderer, label: &str, size: u64, usage: wgpu::BufferUsages) -> Self {
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
    /// Published model then normal-transform bytes, in placement order.
    instances: Vec<u8>,
    arrays: Owned<wgpu::BindGroup>,
    materials: Owned<wgpu::BindGroup>,
}
pub(super) struct Targets {
    /// Placement + 1 (zero is a miss), primitive, f32 barycentrics. Interactive frames
    /// write them, and their counts, only in tiles whose `tiles` flag is set.
    pub hit: Depth,
    /// One flag per `TRACE_TILE` pixels: nonzero when an Interactive camera ray hit.
    pub tiles: Depth,
}
/// Group-0 bindings for one hit/HDR/fresh/TLAS combination.
struct Bindings {
    key: (wgpu::Texture, wgpu::Texture, Option<wgpu::Texture>, u64),
    trace: [Option<Owned<wgpu::BindGroup>>; 2],
    fresh: Option<Owned<wgpu::BindGroup>>,
    /// Soft frames: the fused Interactive trace and visibility.
    interactive: Option<Owned<wgpu::BindGroup>>,
    shade: [Option<Owned<wgpu::BindGroup>>; 3],
}
enum Retired {
    Targets(#[allow(dead_code)] Targets),
    Geometry(#[allow(dead_code)] Geometry),
    Bindings(#[allow(dead_code)] Bindings),
}

/// Compiled lazily while traced visibility is selected, and with every reload then.
pub(super) struct PrimaryPipelines {
    /// Hit record only; hit record plus exact fresh receivers.
    pub trace: [Owned<wgpu::ComputePipeline>; 2],
    /// The production 256-ray estimator with hit-record coverage.
    pub fresh: Owned<wgpu::ComputePipeline>,
    /// Interactive soft frames: camera rays and their 8-direction counts in one dispatch.
    pub interactive: Owned<wgpu::ComputePipeline>,
    /// Indexed by `Shading`: inline hard-shadow rays (or none), fresh counts, or fresh
    /// counts behind Interactive tile flags.
    pub shade: [Owned<wgpu::ComputePipeline>; 3],
}

/// Which traced shading pipeline and bindings a frame uses.
#[derive(Clone, Copy)]
pub(super) enum Shading {
    /// Inline `hard_shadow` rays, or none.
    Hard,
    /// Fresh visibility counts over complete hit records.
    Soft,
    /// Fresh counts; tile flags skip the hit records of uncovered tiles.
    Interactive,
}

pub(super) struct PrimaryState {
    trace_bindings: [Owned<wgpu::BindGroupLayout>; 2],
    shade_bindings: [Owned<wgpu::BindGroupLayout>; 3],
    fresh_bindings: Owned<wgpu::BindGroupLayout>,
    fresh_layout: Owned<wgpu::PipelineLayout>,
    interactive_bindings: Owned<wgpu::BindGroupLayout>,
    interactive_layout: Owned<wgpu::PipelineLayout>,
    array_bindings: Owned<wgpu::BindGroupLayout>,
    material_bindings: Owned<wgpu::BindGroupLayout>,
    trace_layouts: [Owned<wgpu::PipelineLayout>; 2],
    shade_layouts: [Owned<wgpu::PipelineLayout>; 3],
    frame: Option<(Buffer, [u8; FRAME_BYTES])>,
    /// Storage stand-in for absent tangent streams and unused draw slots.
    absent: Option<Buffer>,
    pub targets: Option<Targets>,
    geometry: Option<Geometry>,
    bindings: Option<Bindings>,
    retired: Vec<(u64, Retired)>,
}

impl PrimaryState {
    /// F0's test-only jittered reference uses three existing ABIs. Other entry
    /// points intentionally do not acquire the reference camera scratch fields.
    #[cfg(test)]
    pub(super) fn test_film_pipelines(
        &self,
        device: &wgpu::Device,
        source: &str,
    ) -> [wgpu::ComputePipeline; 3] {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test-only F0 film reference"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        [
            ("primary_trace_receivers", &self.trace_layouts[1]),
            ("fresh_compute", &self.fresh_layout),
            ("primary_shade", &self.shade_layouts[1]),
        ]
        .map(|(entry, layout)| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        })
    }

    #[cfg(test)]
    pub(super) fn test_fresh_bindings(&self) -> (&wgpu::BindGroupLayout, &wgpu::BindGroup) {
        (
            &self.fresh_bindings,
            self.bindings.as_ref().unwrap().fresh.as_ref().unwrap(),
        )
    }

    /// `lighting` is the group-3 layout shared with raster shading.
    pub fn new(device: &wgpu::Device, ledger: &Ledger, lighting: &wgpu::BindGroupLayout) -> Self {
        use wgpu::TextureFormat as F;
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let frame = uniform(10, FRAME_BYTES as u64);
        let camera = uniform(0, 80);
        let trace = [
            &[][..],
            &[camera, image(16, F::Rgba32Float), image(17, F::Rgba32Float)],
        ]
        .map(|extra| {
            let mut entries = vec![scene(TRACE_SCENE), frame, image(13, F::Rgba32Uint)];
            entries.extend_from_slice(extra);
            group("primary trace", &entries)
        });
        let interactive_bindings = group(
            "primary Interactive trace and visibility",
            &[
                scene(TRACE_SCENE),
                scene(2),
                frame,
                image(13, F::Rgba32Uint),
                image(5, F::R32Uint),
                image(20, F::R32Uint),
            ],
        );
        let uint = wgpu::TextureSampleType::Uint;
        let shade_bindings = [
            &[scene(2)][..],
            &[texture(6, uint)],
            &[texture(6, uint), texture(21, uint)],
        ]
        .map(|visibility| {
            let mut entries = vec![camera, texture(7, uint), frame, image(18, F::Rgba16Float)];
            entries.extend_from_slice(visibility);
            group("primary shading", &entries)
        });
        let fresh_bindings = group(
            "primary fresh visibility",
            &[
                scene(2),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                texture(4, wgpu::TextureSampleType::Float { filterable: false }),
                image(5, F::R32Uint),
                texture(7, uint),
            ],
        );
        let array_bindings = group(
            "primary draw geometry",
            &[
                read(0, 64),
                read(1, 64),
                per_draw(read(2, 16)),
                per_draw(read(3, 4)),
                per_draw(read(4, 16)),
                read(5, 4),
            ],
        );
        let filtered = wgpu::TextureSampleType::Float { filterable: true };
        let sampler = |binding| {
            per_draw(entry(
                binding,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ))
        };
        let material_bindings = group(
            "primary draw materials",
            &[
                per_draw(read(10, 48)),
                per_draw(texture(11, filtered)),
                sampler(12),
                per_draw(texture(13, filtered)),
                sampler(14),
                per_draw(texture(15, filtered)),
                sampler(16),
            ],
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
            interactive_bindings,
            fresh_bindings,
            trace_bindings: trace,
            shade_bindings,
            array_bindings,
            material_bindings,
            frame: None,
            absent: None,
            targets: None,
            geometry: None,
            bindings: None,
            retired: vec![],
        }
    }

    /// `sources` are `shader_sources` output: hard module first, soft module second.
    pub fn compile(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        sources: &[String; 2],
    ) -> PrimaryPipelines {
        let [hard, soft] = sources.each_ref().map(|source| {
            ledger.own(
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
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
        PrimaryPipelines {
            fresh: self.fresh_pipeline(device, ledger, &soft, "fresh_compute", &[]),
            interactive: self.interactive_pipeline(
                device,
                ledger,
                &soft,
                &[("FRESH_DIRECTIONS", 8.0)],
            ),
            trace: [
                pipeline(&hard, "primary_trace", &self.trace_layouts[0]),
                pipeline(&hard, "primary_trace_receivers", &self.trace_layouts[1]),
            ],
            shade: [
                pipeline(&hard, "primary_shade", &self.shade_layouts[0]),
                pipeline(&soft, "primary_shade", &self.shade_layouts[1]),
                pipeline(&soft, "primary_shade_tiles", &self.shade_layouts[2]),
            ],
        }
    }

    /// `fresh_compute` of a soft primary module on the traced fresh ABI. Tests seed or
    /// replace the estimator through `constants` and variant modules.
    pub fn fresh_pipeline(
        &self,
        device: &wgpu::Device,
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

    /// `primary_trace_interactive` of a soft primary module on its fused ABI. Tests seed
    /// the estimator through `constants`.
    pub fn interactive_pipeline(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        constants: &[(&str, f64)],
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("traced Interactive camera rays and visibility"),
                layout: Some(&self.interactive_layout),
                module,
                entry_point: Some("primary_trace_interactive"),
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
            self.retired.push((last, Retired::Targets(t)));
        }
        if let Some(g) = self.geometry.take() {
            self.retired.push((last, Retired::Geometry(g)));
        }
        if let Some(b) = self.bindings.take() {
            self.retired.push((last, Retired::Bindings(b)));
        }
        self.frame = None;
        self.absent = None;
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
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
        let size = self.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let default = self.default_material.as_ref().unwrap();
        let placements = placement_table(draws.iter().map(|d| {
            let single_sided = !d.material.unwrap_or(default).double_sided;
            (d.instances.len(), d.mesh.tangent.is_some(), single_sided)
        }))?;
        let sided = placements.iter().any(|p| p & 1 << 30 != 0);
        let matrix = camera.matrix(size.width, size.height);
        let inverse = matrix.inverse();
        if !inverse.is_finite() {
            return Err(Error::InvalidInput("traced camera inverse is nonfinite"));
        }
        // Only the Interactive trace reads the rectangle; it costs O(placements) per frame.
        let scene = if soft && self.soft_mode == crate::SoftShadows::Interactive {
            let mut bounds = vec![];
            for d in draws {
                for &model in &d.instances.state.models {
                    bounds.push(shadow_gpu::world_bounds(d.mesh.bounds, model)?);
                }
            }
            scene_pixels(&bounds, matrix, [size.width, size.height])
        } else {
            [0, 0, size.width, size.height]
        };
        let mut bytes = [0; FRAME_BYTES];
        bytes[..64].copy_from_slice(&matrix_bytes(inverse));
        for (dst, value) in bytes[64..80]
            .chunks_exact_mut(4)
            .zip(self.lit_clear().into_iter().chain([1.0]))
        {
            dst.copy_from_slice(&(value as f32).to_ne_bytes());
        }
        bytes[80..84].copy_from_slice(&u32::from(sided).to_ne_bytes());
        for (dst, value) in bytes[96..].chunks_exact_mut(4).zip(scene) {
            dst.copy_from_slice(&value.to_ne_bytes());
        }
        if self.primary.as_ref().unwrap().frame.is_none() {
            let frame = Buffer::new(
                self,
                "primary camera inverse, clear and culling",
                FRAME_BYTES as u64,
                wgpu::BufferUsages::UNIFORM,
            );
            self.primary.as_mut().unwrap().frame = Some((frame, [0; FRAME_BYTES]));
        }
        if self.primary.as_ref().unwrap().absent.is_none() {
            let absent = Buffer::new(
                self,
                "primary absent storage",
                48,
                wgpu::BufferUsages::STORAGE,
            );
            self.primary.as_mut().unwrap().absent = Some(absent);
        }
        let (frame, shadow) = self.primary.as_mut().unwrap().frame.as_mut().unwrap();
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
                index: (**d.mesh.index).clone(),
                tangent: d.mesh.tangent.as_ref().map(|t| (**t).clone()),
                material: (*d.material.unwrap_or(default).bind).clone(),
                instances: d.instances.len(),
            })
            .collect();
        let state = self.primary.as_ref().unwrap();
        if state.geometry.as_ref().is_none_or(|g| g.keys != keys) {
            let storage = wgpu::BufferUsages::STORAGE;
            let instance_bytes = (placements.len().max(1) * 64) as u64;
            let models = Buffer::new(
                self,
                "primary placement transforms",
                instance_bytes,
                storage,
            );
            let normals = Buffer::new(self, "primary placement normals", instance_bytes, storage);
            let table = Buffer::new(
                self,
                "primary placement draws",
                (placements.len().max(1) * 4) as u64,
                storage,
            );
            let words: Vec<u8> = placements.iter().flat_map(|v| v.to_ne_bytes()).collect();
            if !words.is_empty() {
                self.queue.write_buffer(&table.value, 0, &words);
                uploads.shadows += words.len() as u64;
                uploads.write_calls += 1;
            }
            let state = self.primary.as_ref().unwrap();
            let absent: &wgpu::Buffer = &state.absent.as_ref().unwrap().value;
            let slots = TRACED_DRAWS as usize;
            let vertices = padded(draws.iter().map(|d| &**d.mesh.vertex), absent);
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
                ],
            );
            let used: Vec<&Material> = draws
                .iter()
                .map(|d| d.material.unwrap_or(default))
                .chain(std::iter::repeat(default))
                .take(slots)
                .collect();
            let factors: Vec<_> = used
                .iter()
                .map(|m| m._uniform.as_entire_buffer_binding())
                .collect();
            let views: [Vec<&wgpu::TextureView>; 3] =
                std::array::from_fn(|i| used.iter().map(|m| &*m._textures[i].0.view).collect());
            let samplers: [Vec<&wgpu::Sampler>; 3] =
                std::array::from_fn(|i| used.iter().map(|m| &*m._samplers[i]).collect());
            let materials = bind(
                "primary draw materials",
                &state.material_bindings,
                &[
                    (10, R::BufferArray(&factors)),
                    (11, R::TextureViewArray(&views[0])),
                    (12, R::SamplerArray(&samplers[0])),
                    (13, R::TextureViewArray(&views[1])),
                    (14, R::SamplerArray(&samplers[1])),
                    (15, R::TextureViewArray(&views[2])),
                    (16, R::SamplerArray(&samplers[2])),
                ],
            );
            let next = Geometry {
                keys,
                _placements: table,
                models,
                normals,
                instances: vec![],
                arrays,
                materials,
            };
            let state = self.primary.as_mut().unwrap();
            if let Some(old) = state.geometry.replace(next) {
                state
                    .retired
                    .push((self.submissions, Retired::Geometry(old)));
            }
        }
        // Instance edits keep the tables; only their transforms are republished.
        let mut instances = Vec::with_capacity(placements.len() * 128);
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
        let g = self.primary.as_mut().unwrap().geometry.as_mut().unwrap();
        if g.instances != instances {
            let (models, normals) = instances.split_at(instances.len() / 2);
            if !models.is_empty() {
                self.queue.write_buffer(&g.models.value, 0, models);
                self.queue.write_buffer(&g.normals.value, 0, normals);
                uploads.models += models.len() as u64;
                uploads.normals += normals.len() as u64;
                uploads.write_calls += 2;
            }
            g.instances = instances;
        }
        if self
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .is_none_or(|t| t.hit.texture.size() != size)
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
            let next = Targets {
                hit: target("primary hit records", size, wgpu::TextureFormat::Rgba32Uint),
                tiles: target("primary tile coverage", tiles, wgpu::TextureFormat::R32Uint),
            };
            let state = self.primary.as_mut().unwrap();
            if let Some(old) = state.targets.replace(next) {
                state
                    .retired
                    .push((self.submissions, Retired::Targets(old)));
            }
        }
        let state = self.primary.as_ref().unwrap();
        let t = state.targets.as_ref().unwrap();
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let fresh = soft.then(|| self.fresh.as_ref().unwrap().targets.as_ref().unwrap());
        let key = (
            (*t.hit.texture).clone(),
            (*hdr.texture).clone(),
            fresh.map(|f| (*f.world.texture).clone()),
            scene_generation,
        );
        if state.bindings.as_ref().is_none_or(|b| b.key != key) {
            use wgpu::BindingResource as R;
            let hardware = self.hardware_shadows.as_ref().unwrap();
            let frame = &state.frame.as_ref().unwrap().0.value;
            let group = |layout: &wgpu::BindGroupLayout, entries: &[(u32, R<'_>)]| {
                let entries: Vec<_> = entries
                    .iter()
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
            let scene = (2, hardware.scene().as_binding());
            let trace_scene = (TRACE_SCENE, hardware.scene().as_binding());
            let uniform = (10, frame.as_entire_binding());
            let hit = R::TextureView(&t.hit.view);
            let trace = [
                Some(group(
                    &state.trace_bindings[0],
                    &[trace_scene.clone(), uniform.clone(), (13, hit.clone())],
                )),
                fresh.map(|f| {
                    group(
                        &state.trace_bindings[1],
                        &[
                            camera.clone(),
                            trace_scene.clone(),
                            uniform.clone(),
                            (13, hit.clone()),
                            (16, R::TextureView(&f.world.view)),
                            (17, R::TextureView(&f.normal.view)),
                        ],
                    )
                }),
            ];
            let shade = |shading: Shading, visibility: &[(u32, R<'_>)]| {
                let mut entries = vec![
                    camera.clone(),
                    (7, hit.clone()),
                    uniform.clone(),
                    (18, R::TextureView(&hdr.view)),
                ];
                entries.extend_from_slice(visibility);
                Some(group(&state.shade_bindings[shading as usize], &entries))
            };
            let tiles = R::TextureView(&t.tiles.view);
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
                        ],
                    )
                }),
                shade: match fresh {
                    Some(f) => {
                        let counts = (6, R::TextureView(&f.visibility.view));
                        [
                            None,
                            shade(Shading::Soft, std::slice::from_ref(&counts)),
                            shade(Shading::Interactive, &[counts, (21, tiles)]),
                        ]
                    }
                    None => [
                        shade(Shading::Hard, std::slice::from_ref(&scene)),
                        None,
                        None,
                    ],
                },
                key,
            };
            let state = self.primary.as_mut().unwrap();
            if let Some(old) = state.bindings.replace(next) {
                state
                    .retired
                    .push((self.submissions, Retired::Bindings(old)));
            }
        }
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
    /// The trace dispatch with `pipeline`; tests screen variant kernels on one ABI.
    pub(super) fn encode_primary_trace(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        soft: bool,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.primary.as_ref().unwrap();
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
        self.encode_primary_fresh_tile(encoder, pipeline, fresh_shadow_gpu::TILE, timestamp_writes);
    }

    /// Interactive soft frames: camera rays, hit records and 8-direction counts in one
    /// dispatch, replacing the trace and fresh visibility passes. Production passes
    /// `TRACE_TILE`; tests screen other workgroup shapes on the same ABI.
    pub(super) fn encode_primary_interactive(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.primary.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let bindings = state.bindings.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced Interactive visibility"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &**bindings.interactive.as_ref().unwrap(), &[]);
        pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(
            size.width.div_ceil(tile[0]),
            size.height.div_ceil(tile[1]),
            1,
        );
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
        let state = self.primary.as_ref().unwrap();
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

    /// Deferred material shading of the whole image into the HDR target: one dispatch.
    pub(super) fn encode_primary_shading(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let p = self.pipeline.primary.as_ref().unwrap();
        let pipeline = &p.shade[shading as usize];
        self.encode_primary_shade(encoder, pipeline, SHADE_TILE, shading, timestamp_writes);
    }
    /// The shading dispatch with `pipeline`; tests screen variant kernels on one ABI.
    pub(super) fn encode_primary_shade(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        shading: Shading,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.primary.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let bindings = state.bindings.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced deferred shading"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(
            0,
            &**bindings.shade[shading as usize].as_ref().unwrap(),
            &[],
        );
        pass.set_bind_group(1, &*g.arrays, &[]);
        pass.set_bind_group(2, &*g.materials, &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(
            size.width.div_ceil(tile[0]),
            size.height.div_ceil(tile[1]),
            1,
        );
    }
}
