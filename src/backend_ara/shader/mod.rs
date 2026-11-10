//! Shared WGSL-to-heap adaptation; all pass algorithms remain in Renderlab WGSL.
mod metal;
mod spirv;

pub use metal::msl;
pub use naga::ShaderStage;
pub use spirv::spirv;

/// The unchanged Renderlab filter source, including the selected bloom kernels.
pub const POST: &str = include_str!("../../../shaders/post.wgsl");
/// The unchanged Renderlab fullscreen vertex and tone fragment source.
pub const TONE: &str = include_str!("../../../shaders/post_tone.wgsl");
/// One pass family in the replay.
#[derive(Debug, Clone, Copy)]
pub enum Pass {
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
            Self::LightCluster => {
                (crate::light_cluster::constants() + crate::light_cluster::KERNEL).into()
            }
            Self::Tone => TONE.into(),
            Self::Display => include_str!("../shaders/display.wgsl").into(),
            _ => POST.into(),
        }
    }
    pub(crate) fn threads(self) -> anyhow::Result<[u32; 3]> {
        let module = parse(self.source().as_ref())?;
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
            Self::LightCluster => "lc_build",
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
    /// Fullscreen vertex entry, present for fullscreen raster passes.
    pub vertex_entry: Option<String>,
}
/// One SPIR-V entry compiled against Ara's global descriptor bindings and roots.
pub struct SpirvStage {
    /// Native shader stage.
    pub stage: ShaderStage,
    /// Naga's entry point name.
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
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}
