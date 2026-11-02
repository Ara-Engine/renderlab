//! Persistent indexed meshes with bounded submissions. Windowing belongs to the caller.
//! Use a dedicated device and queue. External queue clones/submissions are outside accounting.
mod mesh;
mod observe;
mod renderer;
#[cfg(test)]
mod visibility;

pub use glam::{Mat4, Vec3};
pub use mesh::{Camera, Draw, Limits, Mesh, MeshData, Vertex};
pub use observe::*;
pub use renderer::Renderer;
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
