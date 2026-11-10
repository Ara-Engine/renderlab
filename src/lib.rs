//! Renderlab: a Rust/wgpu renderer with first-class hardware-RT and non-RT paths.
//!
//! Games build against [`engine`]: one owned [`engine::Engine`] with typed resource and
//! object handles, coarse settings, and blocking-only-for-backpressure frames
//! (docs/api.md). The crate root is the low-level renderer that the engine wraps:
//! persistent indexed meshes and transform collections with bounded submissions. Draw
//! order, then ascending instance slot, defines equal-depth winners. Windowing belongs to
//! the caller. Use a dedicated device and queue. External queue clones/submissions are
//! outside accounting.
#[cfg(test)]
extern crate self as renderlab;
pub mod animation;
/// Optional ara-gpu renderer, independently constructed from the wgpu renderer.
#[cfg(feature = "backend-ara")]
pub mod backend_ara;
// AT1: physical sky, aerial perspective and fog (docs/m7-at1.md).
mod atmosphere;
/// TP1 alpha-tested (cutout) geometry, cut at load (docs/m7-tp1.md).
pub mod cutout;
pub mod engine;
mod environment;
// DC1: owned projected decals and the programmable surface-input API.
mod decal;
mod environment_lobes;
mod gi_bvh;
mod hardware_shadows;
mod instances;
mod light_cluster;
mod lighting;
mod material;
mod mesh;
mod normal;
mod observe;
mod pass_timeline;
// PP1 public camera/display settings; implementation stays in the tone stage.
mod post;
mod primary_ray;
// PC1: shared pipeline caching and startup instrumentation.
pub mod pipeline_compilation;
mod program;
pub mod program_watch;
mod receiver_adjacency;
/// M7 reference path tracer (measurement tooling for the GI and reflection gates).
pub mod reference;
mod reflection_software;
mod reflections;
mod renderer;
#[path = "program_vulkan.rs"]
pub mod shader_compiler;
mod shadow_plan;
mod shadows;
mod transparency;
pub mod virtual_geometry;
mod visibility;

pub use atmosphere::{Atmosphere, AtmosphereMedium, HeightFog, VolumetricFog};
pub use decal::{
    Decal, DecalBlend, DecalSettings, DecalStats, DecalTexture, DecalTextureKind, DecalTextures,
};
pub use environment::{EnvironmentLimits, EnvironmentMap, EnvironmentParameters};
pub use glam::{Mat4, Vec3};
pub use hardware_shadows::{HardwareShadowOptions, HardwareShadowStats};
pub use instances::{Instances, Visibility};
pub use light_cluster::{LightClusterStats, LocalShadows};
pub use lighting::{Light, Lighting, decode_f16};
pub use material::{
    Material, MaterialDescriptor, RendererOptions, SampleCount, Texture, TextureBinding, ViewMode,
};
pub use mesh::{Camera, Draw, Limits, Mesh, MeshData};
pub use observe::*;
pub use post::{ColourLut, DepthOfField, PostSettings, Tonemapper};
pub use primary_ray::{
    PrimaryVisibility, TRACED_DRAWS, TRACED_FEATURES, VISIBILITY_FEATURES,
    VISIBILITY_RASTER_FEATURES, traced_limits, visibility_limits,
};
pub use program::{Program, ProgramDescriptor, STANDARD_PROGRAM, Value};
pub use receiver_adjacency::MeshUpload;
pub use reflections::ReflectionSettings;
pub use renderer::skinning::SkinnedMeshInstance;
pub use renderer::{Capture, CaptureInfo};
pub use renderer::{
    IndirectCounters, IndirectLookup, IndirectProfile, IndirectSettings, IndirectStats,
    ProbeCounters,
};
pub use renderer::{LodPackage, LodStats, MeshLod};
pub use renderer::{Reconstruction, ReconstructionShadowBudget, RenderScale, Renderer};
pub use renderer::{ResidencyStats, TextureResidency};
pub use renderer::{
    VIRTUAL_FEATURES, VirtualLookup, VirtualShadowSettings, VirtualShadowStats, WalkRecords,
};
pub use renderlab_asset::{self as asset, Vertex};
pub use shadows::SoftShadows;
pub use shadows::{ShadowLimits, ShadowRequest, ShadowStats, Shadows};
pub use transparency::{
    ColorKey, Emitter, EmitterDescriptor, Flipbook, Forces, MAX_CURVE_KEYS, MAX_PARTICLES,
    PlaneCollision, SizeKey, SpawnShape, SpriteAlignment, Transparency, TransparencyStats,
};
pub use wgpu;

/// Raster shadow receivers read the actual triangle's world-space vertices.
/// Callers must request this native Vulkan device feature.
pub const RECEIVER_FEATURES: wgpu::Features = wgpu::Features::SHADER_PER_VERTEX;

/// Device memory hints Renderlab recommends for `wgpu::DeviceDescriptor::memory_hints`:
/// wgpu's 8-64 MiB blocks. Smaller blocks saved more space but regressed quarry
/// frame times; retain this policy (docs/memory-m1.md, docs/non-rt-n8h.md).
pub const MEMORY_HINTS: wgpu::MemoryHints = wgpu::MemoryHints::MemoryUsage;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault(pub String);

#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    Shader(String),
    Fault(Fault),
    Poll(wgpu::PollError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => f.write_str(message),
            Self::Shader(message) => f.write_str(message),
            Self::Fault(Fault(message)) => {
                write!(f, "GPU fault; recreate the renderer or restart: {message}")
            }
            Self::Poll(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug)]
pub enum ReloadError {
    Rejected {
        diagnostic: String,
        compile: std::time::Duration,
    },
    Fault(Fault),
}
impl std::fmt::Display for ReloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { diagnostic, .. } => {
                write!(f, "reload rejected; active pipeline retained: {diagnostic}")
            }
            Self::Fault(Fault(message)) => write!(f, "GPU fault; restart required: {message}"),
        }
    }
}
impl std::error::Error for ReloadError {}

#[cfg(test)]
mod tests;
