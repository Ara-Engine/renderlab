//! Shared WGSL-to-heap adaptation; all pass algorithms remain in Renderlab WGSL.
mod array_access;
mod buffer_arrays;
mod handle_arrays;
mod heap_interface;
/// Renames locals that shadow a resource global before the textual heap rewrite.
pub mod local_shadows;
pub(crate) mod memo;
mod mesh_interface;
mod metal;
mod packet_spirv;
mod program;
mod ray_source;
pub use heap_interface::{InputKey, InputKind, SampleType, ShaderInput, inputs};
pub(crate) use memo::digest;
pub(crate) use mesh_interface::MeshResource;
pub use program::Program;
pub(crate) use program::contract as packet_contract;
mod particle_interface;
pub(crate) use particle_interface::ParticleResource;
mod skin_interface;
mod spirv;
mod spv_acceleration;
mod spv_layout;
mod spv_nonuniform;
mod spv_precise;
pub(crate) use skin_interface::SkinResource;

pub use metal::msl;
pub use naga::ShaderStage;
pub use spirv::{Checks, spirv};

/// The unchanged Renderlab filter source, including the selected bloom kernels.
pub const POST: &str = include_str!("../../../shaders/post.wgsl");
/// The unchanged Renderlab fullscreen vertex and tone fragment source.
pub const TONE: &str = include_str!("../../../shaders/post_tone.wgsl");
/// One pass family in the replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pass {
    /// Shared frontend conformance probe; not a Renderlab rendering pass.
    BufferArrayProbe,
    /// Shared sampled-handle array conformance probe; not a rendering pass.
    HandleArrayProbe,
    /// Original diagnostic closest-hit query, not a production rendering pass.
    PrimaryRayProbe,
    /// Original primary camera trace with portable mesh-position reads.
    PrimaryTrace,
    /// Original camera trace publishing exact receiver world and normals.
    PrimaryTraceReceivers,
    /// The fused Interactive trace, its undeferred form and its repair.
    PrimaryInteractive(crate::backend_ara::primary_trace::Interactive),
    /// One kernel of the world radiance cache's hardware-ray module.
    GiCache(crate::backend_ara::gi_cache::Entry),
    /// Temporal reconstruction's resolve (`tr1_resolve`): history, reprojection and the
    /// reconstructed image.
    TemporalResolve,
    /// Original colored transmission layer from one transparent object.
    TransmissionShadow,
    /// Original vertex-only transparency sun-shadow depth raster.
    TransparencySunShadow,
    /// Original source-sided shadow depth raster.
    ShadowRaster,
    /// Original fresh geometric ray-query shadow visibility.
    FreshVisibility,
    /// Original depth-winning geometric shadow receiver prepass.
    ReceiverRaster,
    /// Original transparency multisample background seed.
    TransparencySeed,
    /// Original caller-programmed transparent pane composition.
    TransparencyGlass,
    /// Original scene fallback Reinhard presenter.
    FallbackPresent,
    /// Original single-program deferred shading of primary tiles.
    PrimaryShadeTiles,
    /// Original mixed-program deferred shading of listed primary tiles.
    PrimaryShadeList,
    /// Original software raster of projected cluster triangles into packed visibility.
    VisibilitySoftware,
    /// Original procedural hardware raster of cluster IDs, with reversed-Z depth.
    VisibilityHardware,
    /// Original packed visibility hit to source-triangle receiver resolution.
    VisibilityResolve,
    /// Primary-hit rigid/skinned/sky velocity and view depth for post filters.
    PostPrimarySurface,
    /// Opaque hit geometry to the transparency pass's signed view distance.
    TransparencyHitDepth,
    /// Ray-free near-edge geometric check and retry-queue construction.
    PrimaryCheck,
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
    /// The pass's WGSL, composed once and kept for the life of the process.
    pub(super) fn source(self) -> std::borrow::Cow<'static, str> {
        use std::sync::{Mutex, OnceLock};
        static KEPT: OnceLock<Mutex<std::collections::HashMap<Pass, &'static str>>> =
            OnceLock::new();
        let table = KEPT.get_or_init(Default::default);
        if let Some(kept) = table.lock().expect("source lock").get(&self) {
            return std::borrow::Cow::Borrowed(kept);
        }
        let made: &'static str = match self.compose() {
            std::borrow::Cow::Borrowed(fixed) => fixed,
            std::borrow::Cow::Owned(composed) => Box::leak(composed.into_boxed_str()),
        };
        table.lock().expect("source lock").insert(self, made);
        std::borrow::Cow::Borrowed(made)
    }
    fn compose(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::TransmissionShadow => {
                crate::renderer::transparency_gpu::transmission_shadow::draw_source(include_str!(
                    "../../../shaders/scene.wgsl"
                ))
                .into()
            }
            Self::TransparencySunShadow => format!(
                "{}{}",
                transparency_section("TP_FRAME"),
                transparency_section("TP_SHADOW")
            )
            .into(),
            Self::ShadowRaster => include_str!("../../../shaders/scene.wgsl").into(),
            Self::GiCache(crate::backend_ara::gi_cache::Entry::ShadeCells) => {
                crate::backend_ara::gi_cache::shade_source(
                    crate::program::STANDARD_PROGRAM,
                    crate::IndirectSettings::default().directions,
                )
                .expect("embedded cell shading composition")
                .into()
            }
            Self::GiCache(_) => {
                crate::backend_ara::gi_cache::source(crate::IndirectSettings::default().directions)
                    .expect("embedded radiance cache composition")
                    .into()
            }
            Self::TemporalResolve => crate::backend_ara::temporal::resolve_source()
                .expect("embedded temporal resolve composition")
                .into(),
            Self::FreshVisibility => crate::backend_ara::fresh_visibility::source(
                crate::backend_ara::fresh_visibility::Mode::Exact,
            )
            .expect("embedded fresh visibility composition")
            .into(),
            Self::ReceiverRaster => crate::renderer::fresh_shadow_gpu::shader_source(include_str!(
                "../../../shaders/scene.wgsl"
            ))
            .expect("embedded fresh receiver composition")
            .into(),
            Self::TransparencyGlass => crate::backend_ara::transparency_glass::source(
                include_str!("../shaders/glass_material.wgsl"),
            )
            .into(),
            Self::TransparencySeed => {
                crate::renderer::transparency_gpu::section("TP_AA_SEED").into()
            }
            Self::FallbackPresent => include_str!("../../../shaders/scene.wgsl").into(),
            Self::PrimaryInteractive(_) => {
                crate::backend_ara::primary_trace::interactive_source(false)
                    .expect("original soft kernel seams remain valid")
                    .into()
            }
            Self::PrimaryTrace | Self::PrimaryTraceReceivers => {
                crate::backend_ara::primary_trace::source()
                    .expect("original primary trace seams remain valid")
                    .into()
            }
            Self::PrimaryRayProbe => include_str!("../../../shaders/primary_ray_probe.wgsl").into(),
            Self::PrimaryShadeTiles | Self::PrimaryShadeList => {
                crate::backend_ara::primary_shade::source(
                    include_str!("../shaders/traced_material.wgsl"),
                    matches!(self, Self::PrimaryShadeList),
                )
                .expect("original traced composition remains valid")
                .into()
            }
            Self::HandleArrayProbe => include_str!("../shaders/handle_array_probe.wgsl").into(),
            Self::BufferArrayProbe => include_str!("../shaders/buffer_array_probe.wgsl").into(),
            Self::PrimaryCheck => {
                let [head, _] = crate::renderer::primary_ray_gpu::non_rt_heads(include_str!(
                    "../../../shaders/scene.wgsl"
                ))
                .expect("embedded primary arithmetic head");
                crate::renderer::primary_ray_gpu::edge_check_source(
                    &(head + include_str!("../../../shaders/primary_trace.wgsl")),
                )
                .into()
            }
            Self::TransparencyHitDepth => {
                crate::renderer::transparency_gpu::hit_depth_source().into()
            }
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
            Self::VisibilityHardware => crate::renderer::visibility_buffer::stage_source(
                include_str!("../../../shaders/visibility_buffer_raster.wgsl"),
            )
            .into(),
            Self::VisibilitySoftware => crate::renderer::visibility_buffer::stage_source(
                include_str!("../../../shaders/visibility_buffer_sw.wgsl"),
            )
            .into(),
            Self::VisibilityCull | Self::VisibilityFinish | Self::VisibilityReset => {
                crate::renderer::visibility_buffer::stage_source(
                    if matches!(self, Self::VisibilityReset) {
                        include_str!("../../../shaders/visibility_buffer_reset.wgsl")
                    } else {
                        include_str!("../../../shaders/visibility_buffer_cull.wgsl")
                    },
                )
                .into()
            }
            Self::ProgramBins => {
                crate::renderer::primary_ray_gpu::program_lists::bin_source().into()
            }
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
            Self::TransmissionShadow
                | Self::TransparencySunShadow
                | Self::ShadowRaster
                | Self::ReceiverRaster
                | Self::TransparencyGlass
                | Self::TransparencySeed
                | Self::FallbackPresent
                | Self::VisibilityHardware
                | Self::Tone
                | Self::Display
                | Self::AtmosphereBackdrop
                | Self::MeshDiagnostic
                | Self::MeshToon
                | Self::ParticleBillboard
        )
    }
    pub(crate) fn program_only(self) -> bool {
        matches!(
            self,
            Self::PrimaryShadeTiles | Self::PrimaryShadeList | Self::TransparencyGlass
        )
    }
    pub(super) fn indirect(self) -> bool {
        matches!(
            self,
            Self::TransmissionShadow
                | Self::TransparencySunShadow
                | Self::ShadowRaster
                | Self::FreshVisibility
                | Self::TemporalResolve
                | Self::GiCache(_)
                | Self::ReceiverRaster
                | Self::TransparencyGlass
                | Self::TransparencySeed
                | Self::FallbackPresent
                | Self::PrimaryTrace
                | Self::PrimaryTraceReceivers
                | Self::PrimaryInteractive(_)
                | Self::PrimaryRayProbe
                | Self::PrimaryShadeTiles
                | Self::PrimaryShadeList
                | Self::HandleArrayProbe
                | Self::BufferArrayProbe
                | Self::VisibilitySoftware
                | Self::VisibilityResolve
                | Self::PostPrimarySurface
                | Self::TransparencyHitDepth
                | Self::PrimaryCheck
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
        if matches!(
            self,
            Self::MeshToon | Self::TransparencyGlass | Self::ReceiverRaster
        ) {
            (4, 0)
        } else {
            (3, 0)
        }
    }
    pub(super) fn vertex_only(self) -> bool {
        matches!(self, Self::TransparencySunShadow)
    }
    pub(super) fn vertex(self) -> &'static str {
        if matches!(self, Self::TransmissionShadow) {
            "tx_vs"
        } else if self.vertex_only() {
            "tp_shadow_vs"
        } else if matches!(self, Self::ShadowRaster) {
            "vs_shadow"
        } else if matches!(self, Self::TransparencySeed) {
            "tp_seed_vs"
        } else if matches!(self, Self::FallbackPresent) {
            "vs_present"
        } else if matches!(self, Self::ParticleBillboard) {
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
        static KEPT: memo::Memo<Pass, [u32; 3]> = memo::Memo::new();
        KEPT.get(self, || threads_of(self, self.source().as_ref()))
    }

    pub(super) fn entry(self) -> &'static str {
        match self {
            Self::TransmissionShadow => "tx_fs",
            Self::TransparencySunShadow => "tp_shadow_vs",
            Self::ShadowRaster => "fs_shadow",
            Self::FreshVisibility => "fresh_compute",
            Self::TemporalResolve => "tr1_resolve",
            Self::GiCache(entry) => entry.name(),
            Self::ReceiverRaster => "fs_receiver",
            Self::TransparencyGlass => "fs_transparent",
            Self::TransparencySeed => "tp_seed_fs",
            Self::FallbackPresent => "fs_present",
            Self::PrimaryTrace => "primary_trace",
            Self::PrimaryTraceReceivers => "primary_trace_receivers",
            Self::PrimaryInteractive(entry) => entry.name(),
            Self::PrimaryRayProbe => "primary_probe",
            Self::PrimaryShadeTiles => "primary_shade_tiles",
            Self::PrimaryShadeList => "primary_shade_list",
            Self::PrimaryCheck => "primary_check_interactive",
            Self::TransparencyHitDepth => "tp_depth_hits",
            Self::PostPrimarySurface => "post_primary_surface",
            Self::VisibilityResolve => "resolve",
            Self::VisibilityHardware => "fs",
            Self::VisibilitySoftware => "raster",
            Self::BufferArrayProbe | Self::HandleArrayProbe => "probe",
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
    /// Native pipeline shape, including a genuinely absent depth-only fragment stage.
    pub entries: MetalEntries,
}
/// Native entry shapes accepted by the shared cooked-pipeline loader.
pub enum MetalEntries {
    /// One compute entry.
    Compute {
        /// Generated function name.
        entry: String,
    },
    /// Ordinary raster or vertex-only depth raster.
    Raster {
        /// Generated vertex name.
        vertex: String,
        /// Generated fragment name; None means no fragment stage.
        fragment: Option<String>,
    },
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
        module.entry_points.len()
            == if pass.raster() && !pass.vertex_only() {
                2
            } else {
                1
            },
        "selected shader stages missing"
    );
    anyhow::ensure!(
        module.entry_points.iter().any(|e| e.name == pass.entry()
            && e.stage
                == if pass.vertex_only() {
                    naga::ShaderStage::Vertex
                } else if pass.raster() {
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

// The Metal emission's data checks: indices and buffer reads clamp, as wgpu's Metal
// backend has them. An image load outside the image reads zero, on Metal and in the
// SPIR-V alike: that is what the wgpu renderer gets on Vulkan, where it leaves the load
// unchecked under `robustImageAccess`, and the renderer is compared on Vulkan.
// Resource-array indices remain caller-valid, as in wgpu; heap-index records are
// validated when the packet is published.
pub(super) fn bounds_checks() -> naga::proc::BoundsCheckPolicies {
    naga::proc::BoundsCheckPolicies {
        index: naga::proc::BoundsCheckPolicy::Restrict,
        buffer: naga::proc::BoundsCheckPolicy::Restrict,
        image_load: naga::proc::BoundsCheckPolicy::ReadZeroSkipWrite,
        binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
    }
}

#[cfg(target_os = "macos")]
pub(crate) use metal::from_source as msl_source;
#[cfg(not(target_os = "macos"))]
pub(crate) use spirv::from_source as spirv_source;
pub(crate) use spirv::variant_name as spirv_variant;
pub(crate) fn threads_source(pass: Pass, source: &str) -> anyhow::Result<[u32; 3]> {
    threads_digest(pass, source, memo::digest(source))
}
/// A caller-authored program's workgroup size, kept by the pass and its source's
/// `digest`.
fn threads_digest(pass: Pass, source: &str, digest: [u8; 32]) -> anyhow::Result<[u32; 3]> {
    static KEPT: memo::Memo<(Pass, [u8; 32]), [u32; 3]> = memo::Memo::new();
    KEPT.get((pass, digest), || threads_of(pass, source))
}
fn threads_of(pass: Pass, source: &str) -> anyhow::Result<[u32; 3]> {
    let mut module = parse(source)?;
    select(&mut module, pass)?;
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == pass.entry())
        .ok_or_else(|| anyhow::anyhow!("selected WGSL entry missing"))?;
    anyhow::ensure!(
        entry.stage == naga::ShaderStage::Compute && entry.workgroup_size.iter().all(|v| *v > 0),
        "selected pass is not a concrete compute entry"
    );
    Ok(entry.workgroup_size)
}
