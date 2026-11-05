//! Persistent indexed meshes and transform collections with bounded submissions.
//! Draw order, then ascending instance slot, defines equal-depth winners. Windowing belongs to the caller.
//! Use a dedicated device and queue. External queue clones/submissions are outside accounting.
#[cfg(test)]
extern crate self as renderlab;
mod environment;
mod hardware_shadows;
mod instances;
mod lighting;
mod material;
mod mesh;
mod normal;
mod observe;
mod primary_ray;
mod program;
mod renderer;
mod shadow_plan;
mod shadows;
pub mod virtual_geometry;
mod visibility;

pub use environment::{EnvironmentLimits, EnvironmentMap, EnvironmentParameters};
pub use glam::{Mat4, Vec3};
pub use hardware_shadows::{HardwareShadowOptions, HardwareShadowStats};
pub use instances::{Instances, Visibility};
pub use lighting::{Light, Lighting, decode_f16};
pub use material::{
    Material, MaterialDescriptor, RendererOptions, SampleCount, Texture, TextureBinding, ViewMode,
};
pub use mesh::{Camera, Draw, Limits, Mesh, MeshData};
pub use observe::*;
pub use primary_ray::{PrimaryVisibility, TRACED_DRAWS, TRACED_FEATURES, traced_limits};
pub use program::{Program, ProgramDescriptor, STANDARD_PROGRAM, Value};
pub use renderer::Renderer;
pub use renderer::{Capture, CaptureInfo};
pub use renderlab_asset::{self as asset, Vertex};
pub use shadows::SoftShadows;
pub use shadows::{ShadowLimits, ShadowRequest, ShadowStats, Shadows};
pub use wgpu;

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
