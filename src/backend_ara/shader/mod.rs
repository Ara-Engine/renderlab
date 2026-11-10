//! Shared WGSL-to-heap adaptation; all pass algorithms remain in Renderlab WGSL.
mod array_access;
mod buffer_arrays;
mod heap_interface;
mod mesh_interface;
mod metal;
mod packet_spirv;
pub use heap_interface::{InputKey, InputKind, SampleType, ShaderInput, inputs};
pub(crate) use mesh_interface::MeshResource;
mod particle_interface;
pub(crate) use particle_interface::ParticleResource;
mod skin_interface;
mod spirv;
mod spv_layout;
pub(crate) use skin_interface::SkinResource;

pub use metal::msl;
pub use naga::ShaderStage;
pub use spirv::spirv;

/// The unchanged Renderlab filter source, including the selected bloom kernels.
pub const POST: &str = include_str!("../../../shaders/post.wgsl");
/// The unchanged Renderlab fullscreen vertex and tone fragment source.
pub const TONE: &str = include_str!("../../../shaders/post_tone.wgsl");
/// One pass family in the replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// Shared frontend conformance probe; not a Renderlab rendering pass.
    BufferArrayProbe,
    /// Original software raster of projected cluster triangles into packed visibility.
    VisibilitySoftware,
    /// Original procedural hardware raster of cluster IDs, with reversed-Z depth.
    VisibilityHardware,
    /// Original packed visibility hit to source-triangle receiver resolution.
    VisibilityResolve,
    /// Primary-hit rigid/skinned/sky velocity and view depth for post filters.
    PostPrimarySurface,
    /// Non-RT cluster cut, frustum and two-phase HZB classification.
    VisibilityCull,
    /// Cull output counters to indirect raster/pyramid/cull arguments.
    VisibilityFinish,
    /// Preserve counters and reset working words/header for the next frame.
    VisibilityReset,
    /// Mixed-program tile lists and indirect dispatch arguments.
    ProgramBins,
    /// Base reversed-Z minimum depth from the visibility buffer.
    HzbBase,
    /// Minimum reduction from the previous image mip.
    HzbReduce,
    /// Original sorted procedural or textured particle billboard composition.
    ParticleBillboard,
    /// Original spawn, drag/gravity integration and incident particle lighting.
    ParticleUpdate,
    /// Original programmable toon forward shading, with all optional resource inputs.
    MeshToon,
    /// Opaque raster depth to transparency view-space distance, preserving misses.
    TransparencyDepth,
    /// Original indexed, instanced unlit/diagnostic mesh draw.
    MeshDiagnostic,
    /// Far-to-near2048-key workgroup particle ordering.
    ParticleSort,
    /// Camera velocity and linear depth from a single-sample raster depth image.
    RasterSurface,
    /// The same original kernel reading sample zero of a 4x raster depth image.
    RasterSurfaceFour,
    /// Current/previous skinned vertices, normals and tangent frames.
    SkinVertices,
    /// Copy changed positions into retained source-cluster words.
    SkinClusters,
    /// GGX specular prefilter of the procedural sky, one array mip.
    EnvironmentSpecular,
    /// Cosine-weighted diffuse irradiance of the procedural sky.
    EnvironmentDiffuse,
    /// Fullscreen exposed sky, sun disk and fog backdrop.
    AtmosphereBackdrop,
    /// Thirty-two cumulative aerial perspective slices.
    AtmosphereAerial,
    /// Sixty-four cumulative fog slices from resolved visibility/extinction.
    AtmosphereFogIntegrate,
    /// Forty-step transmittance LUT.
    AtmosphereTransmittance,
    /// Multiple-scattering transfer LUT.
    AtmosphereMultiscatter,
    /// Camera sky-view LUT.
    AtmosphereSky,
    /// Sky solid-angle mean for ambient fog.
    AtmosphereAmbient,
    /// Packed u16 clustered-light lists in an R32Uint image.
    LightCluster,
    /// Display-linear to sRGB transfer into an UNORM presentation image.
    Display,
    /// Nine-tap tent downsample into RGBA16Float.
    Downsample,
    /// Convex 0.65 mix with a tent upsample into RGBA16Float.
    Upsample,
    /// Per-16×16-tile maximum output-space velocity.
    TileMax,
    /// Depth-aware shutter integration along neighbour-max velocity.
    MotionBlur,
    /// Near/far thin-lens gather with foreground coverage.
    DepthOfField,
    /// Fullscreen tone, grading and a sampled 3D LUT.
    Tone,
}
impl Pass {
    pub(super) fn source(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::BufferArrayProbe => include_str!("../shaders/buffer_array_probe.wgsl").into(),
            Self::PostPrimarySurface => {
                let [head, _] = crate::renderer::primary_ray_gpu::non_rt_heads(include_str!(
                    "../../../shaders/scene.wgsl"
                ))
                .expect("embedded ray-free primary composition");
                format!(
                    "{head}{}{}",
                    include_str!("../../../shaders/object_motion.wgsl"),
                    include_str!("../../../shaders/post_surface.wgsl")
                )
                .into()
            }
            Self::VisibilityResolve => crate::renderer::visibility_buffer::resolve_source(
                include_str!("../../../shaders/scene.wgsl"),
            )
            .expect("embedded visibility resolve composition")
            .into(),
            Self::VisibilityHardware => format!(
                "{}\n{}",
                include_str!("../../../shaders/visibility_buffer.wgsl"),
                include_str!("../../../shaders/visibility_buffer_raster.wgsl")
            )
            .into(),
            Self::VisibilitySoftware => format!(
                "{}\n{}",
                include_str!("../../../shaders/visibility_buffer.wgsl"),
                include_str!("../../../shaders/visibility_buffer_sw.wgsl")
            )
            .into(),
            Self::VisibilityCull | Self::VisibilityFinish | Self::VisibilityReset => format!(
                "{}\n{}",
                include_str!("../../../shaders/visibility_buffer.wgsl"),
                if matches!(self, Self::VisibilityReset) {
                    include_str!("../../../shaders/visibility_buffer_reset.wgsl")
                } else {
                    include_str!("../../../shaders/visibility_buffer_cull.wgsl")
                }
            )
            .into(),
            Self::ProgramBins => include_str!("../../../shaders/program_bins.wgsl").into(),
            Self::HzbBase | Self::HzbReduce => {
                include_str!("../../../shaders/visibility_buffer_hzb.wgsl").into()
            }
            Self::ParticleUpdate | Self::ParticleBillboard => format!(
                "{}{}{}{}{}{}{}",
                include_str!("../../../shaders/scene.wgsl"),
                crate::renderer::transparency_gpu::transmission_shadow::api_source(),
                crate::atmosphere::api_source(),
                transparency_section("TP_FRAME"),
                transparency_section("TP_SUN"),
                transparency_section("TP_PARTICLES"),
                transparency_section(if matches!(self, Self::ParticleUpdate) {
                    "TP_PARTICLES_SIM"
                } else {
                    "TP_PARTICLES_DRAW"
                })
            )
            .into(),
            Self::MeshToon => crate::program::compose(
                include_str!("../../../shaders/scene.wgsl"),
                include_str!("../../../shaders/programs/toon.wgsl"),
                crate::program::Stage::Raster,
                true,
            )
            .source
            .into(),
            Self::TransparencyDepth => format!(
                "{}{}",
                transparency_section("TP_FRAME"),
                transparency_section("TP_DEPTH_RASTER")
            )
            .into(),
            Self::MeshDiagnostic => include_str!("../../../shaders/scene.wgsl").into(),
            Self::ParticleSort => {
                let sim = transparency_section("TP_PARTICLES_SIM");
                let sort = sim
                    .split_once("const TP_SORT_THREADS")
                    .expect("particle sort kernel")
                    .1;
                format!("{}{}\n@group(0) @binding(40) var<storage,read_write> tp_particles:array<TpParticle>;\n@group(0) @binding(41) var<storage,read_write> tp_sorted:array<u32>;\nconst TP_SORT_THREADS{}",transparency_section("TP_FRAME"),transparency_section("TP_PARTICLES"),sort).into()
            }
            Self::AtmosphereBackdrop => format!(
                "{}{}",
                crate::atmosphere::api_source(),
                include_str!("../../../shaders/atmosphere_sky.wgsl")
            )
            .into(),
            Self::RasterSurface => include_str!("../../../shaders/post_raster_surface.wgsl").into(),
            Self::RasterSurfaceFour => include_str!("../../../shaders/post_raster_surface.wgsl")
                .replace("texture_depth_2d;", "texture_depth_multisampled_2d;")
                .into(),
            Self::SkinVertices | Self::SkinClusters => {
                include_str!("../../../shaders/skinning.wgsl").into()
            }
            Self::EnvironmentSpecular | Self::EnvironmentDiffuse => {
                let kernels = include_str!("../../../shaders/atmosphere.wgsl");
                let head = kernels
                    .split_once("@compute")
                    .expect("embedded atmosphere starts with bindings")
                    .0;
                let environment = kernels
                    .split_once("@group(0) @binding(16)")
                    .expect("embedded atmosphere has environment declarations")
                    .1;
                format!(
                    "{}{}@group(0) @binding(16){}",
                    include_str!("../../../shaders/atmosphere_common.wgsl"),
                    head,
                    environment
                )
                .into()
            }
            Self::AtmosphereFogIntegrate => {
                let kernels = include_str!("../../../shaders/atmosphere.wgsl");
                let head = kernels
                    .split_once("@compute")
                    .expect("atmosphere declarations")
                    .0;
                let start = kernels
                    .find("@compute @workgroup_size(8,8) fn at_fog_integrate")
                    .expect("fog integrator");
                let end = kernels
                    .find("// ---- Procedural environment source")
                    .expect("environment boundary");
                format!(
                    "{}{}{}",
                    include_str!("../../../shaders/atmosphere_common.wgsl"),
                    head,
                    &kernels[start..end]
                )
                .into()
            }
            Self::AtmosphereAerial => {
                let kernels = include_str!("../../../shaders/atmosphere.wgsl");
                let end = kernels
                    .find("// Low-discrepancy offset")
                    .expect("aerial boundary");
                format!(
                    "{}{}",
                    include_str!("../../../shaders/atmosphere_common.wgsl"),
                    &kernels[..end]
                )
                .into()
            }
            Self::AtmosphereTransmittance
            | Self::AtmosphereMultiscatter
            | Self::AtmosphereSky
            | Self::AtmosphereAmbient => {
                let kernels = include_str!("../../../shaders/atmosphere.wgsl");
                let end = kernels
                    .find("@compute @workgroup_size(8,8) fn at_aerial_volume")
                    .expect("embedded atmosphere source has its aerial entry");
                format!(
                    "{}{}",
                    include_str!("../../../shaders/atmosphere_common.wgsl"),
                    &kernels[..end]
                )
                .into()
            }
            Self::LightCluster => {
                (crate::light_cluster::constants() + crate::light_cluster::KERNEL).into()
            }
            Self::Tone => TONE.into(),
            Self::Display => include_str!("../shaders/display.wgsl").into(),
            _ => POST.into(),
        }
    }
    pub(super) fn raster(self) -> bool {
        matches!(
            self,
            Self::VisibilityHardware
                | Self::Tone
                | Self::Display
                | Self::AtmosphereBackdrop
                | Self::MeshDiagnostic
                | Self::MeshToon
                | Self::ParticleBillboard
        )
    }
    pub(super) fn indirect(self) -> bool {
        matches!(
            self,
            Self::BufferArrayProbe
                | Self::VisibilitySoftware
                | Self::VisibilityResolve
                | Self::PostPrimarySurface
                | Self::VisibilityHardware
                | Self::MeshToon
                | Self::ParticleUpdate
                | Self::ParticleBillboard
                | Self::HzbBase
                | Self::HzbReduce
                | Self::ProgramBins
                | Self::VisibilityCull
                | Self::VisibilityFinish
                | Self::VisibilityReset
        )
    }
    pub(super) fn metal_version(self) -> (u8, u8) {
        if matches!(self, Self::MeshToon) {
            (4, 0)
        } else {
            (3, 0)
        }
    }
    pub(super) fn vertex(self) -> &'static str {
        if matches!(self, Self::ParticleBillboard) {
            "tp_particle_vs"
        } else if matches!(self, Self::AtmosphereBackdrop) {
            "vs_sky"
        } else {
            "vs"
        }
    }
    pub(super) fn depth_input(self) -> bool {
        matches!(
            self,
            Self::RasterSurface | Self::RasterSurfaceFour | Self::TransparencyDepth
        )
    }
    pub(super) fn skin(self) -> bool {
        matches!(self, Self::SkinVertices | Self::SkinClusters)
    }
    pub(super) fn environment(self) -> bool {
        matches!(self, Self::EnvironmentSpecular | Self::EnvironmentDiffuse)
    }
    pub(super) fn atmosphere(self) -> bool {
        matches!(
            self,
            Self::EnvironmentSpecular
                | Self::AtmosphereBackdrop
                | Self::AtmosphereAerial
                | Self::AtmosphereFogIntegrate
                | Self::EnvironmentDiffuse
                | Self::AtmosphereTransmittance
                | Self::AtmosphereMultiscatter
                | Self::AtmosphereSky
                | Self::AtmosphereAmbient
        )
    }
    pub(crate) fn threads(self) -> anyhow::Result<[u32; 3]> {
        let mut module = parse(self.source().as_ref())?;
        select(&mut module, self)?;
        let entry = module
            .entry_points
            .iter()
            .find(|entry| entry.name == self.entry())
            .ok_or_else(|| anyhow::anyhow!("selected WGSL entry missing"))?;
        anyhow::ensure!(
            entry.stage == naga::ShaderStage::Compute
                && entry.workgroup_size.iter().all(|v| *v > 0),
            "selected pass is not a concrete compute entry"
        );
        Ok(entry.workgroup_size)
    }

    pub(super) fn entry(self) -> &'static str {
        match self {
            Self::PostPrimarySurface => "post_primary_surface",
            Self::VisibilityResolve => "resolve",
            Self::VisibilityHardware => "fs",
            Self::VisibilitySoftware => "raster",
            Self::BufferArrayProbe => "probe",
            Self::VisibilityCull => "cull",
            Self::VisibilityFinish => "finish",
            Self::VisibilityReset => "reset",
            Self::ProgramBins => "program_bins",
            Self::HzbBase => "base",
            Self::HzbReduce => "reduce",
            Self::ParticleBillboard => "tp_particle_fs",
            Self::ParticleUpdate => "tp_particles_update",
            Self::MeshToon => "fs_lit",
            Self::TransparencyDepth => "tp_depth_raster",
            Self::MeshDiagnostic => "fs",
            Self::ParticleSort => "tp_particles_sort",
            Self::RasterSurface | Self::RasterSurfaceFour => "surface_raster",
            Self::SkinVertices => "skin_vertices",
            Self::SkinClusters => "skin_clusters",
            Self::EnvironmentSpecular => "at_environment_specular",
            Self::EnvironmentDiffuse => "at_environment_diffuse",
            Self::LightCluster => "lc_build",
            Self::AtmosphereBackdrop => "fs_sky",
            Self::AtmosphereAerial => "at_aerial_volume",
            Self::AtmosphereFogIntegrate => "at_fog_integrate",
            Self::AtmosphereTransmittance => "at_transmittance_lut",
            Self::AtmosphereMultiscatter => "at_multiscatter_lut",
            Self::AtmosphereSky => "at_skyview_lut",
            Self::AtmosphereAmbient => "at_ambient_mean",
            Self::Downsample => "downsample",
            Self::Upsample => "upsample",
            Self::TileMax => "tile_max",
            Self::MotionBlur => "motion_blur",
            Self::DepthOfField => "depth_of_field",
            Self::Tone | Self::Display => "fs",
        }
    }
}
/// Naga-generated MSL with a checked shim that reads Ara's heap from buffer 2.
pub struct MetalProgram {
    /// Complete source except the caller's TopLevel ABI declaration.
    pub source: String,
    /// Selected compute or fragment entry point.
    pub entry: String,
    /// Selected raster vertex entry; mesh variants read the heap through their wrapper.
    pub vertex_entry: Option<String>,
}
/// One SPIR-V entry compiled against Ara's global descriptor bindings and roots.
pub struct SpirvStage {
    /// Native shader stage.
    pub stage: ShaderStage,
    /// Selected WGSL source entry; the adapter renames the native IR entry to `main`.
    pub entry: String,
    /// Little-endian SPIR-V words.
    pub words: Vec<u32>,
}
fn parse(source: &str) -> anyhow::Result<naga::Module> {
    naga::front::wgsl::parse_str(source).map_err(|e| anyhow::anyhow!(e.emit_to_string(source)))
}
fn validate(module: &naga::Module) -> anyhow::Result<naga::valid::ModuleInfo> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(anyhow::Error::from)
}

/// Retain only the selected native program before output, preserving algorithms
/// while avoiding duplicate native entry kernels and their unused interfaces.
pub(super) fn select(module: &mut naga::Module, pass: Pass) -> anyhow::Result<()> {
    module.entry_points.retain(|entry| {
        entry.name == pass.entry()
            || (pass.raster()
                && entry.stage == naga::ShaderStage::Vertex
                && entry.name == pass.vertex())
    });
    anyhow::ensure!(
        module.entry_points.len() == if pass.raster() { 2 } else { 1 },
        "selected shader stages missing"
    );
    anyhow::ensure!(
        module.entry_points.iter().any(|e| e.name == pass.entry()
            && e.stage
                == if pass.raster() {
                    naga::ShaderStage::Fragment
                } else {
                    naga::ShaderStage::Compute
                }),
        "selected entry has the wrong stage"
    );
    validate(module)?;
    naga::compact::compact(module, naga::compact::KeepUnused::No);
    if !module.overrides.is_empty() {
        // Match creation without explicit wgpu pipeline constants. Both native
        // backends see the same evaluated defaults, including workgroup sizes.
        let info = validate(module)?;
        let (resolved, _) = naga::back::pipeline_constants::process_overrides(
            module,
            &info,
            None,
            &naga::back::PipelineConstants::default(),
        )?;
        *module = resolved.into_owned();
    }
    Ok(())
}

fn transparency_section(name: &str) -> &'static str {
    let source = include_str!("../../../shaders/transparency.wgsl");
    let marker = format!("// ---- {name} ----");
    let end = format!("// ---- end {name} ----");
    source
        .split_once(&marker)
        .expect("embedded transparency section")
        .1
        .split_once(&end)
        .expect("embedded transparency section end")
        .0
}

// Match wgpu's Metal data checks. Resource-array indices remain caller-valid,
// as in wgpu; heap-index records are validated when the packet is published.
pub(super) fn bounds_checks() -> naga::proc::BoundsCheckPolicies {
    naga::proc::BoundsCheckPolicies {
        index: naga::proc::BoundsCheckPolicy::Restrict,
        buffer: naga::proc::BoundsCheckPolicy::Restrict,
        image_load: naga::proc::BoundsCheckPolicy::Restrict,
        binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
    }
}
