//! Opt-in ara-gpu backend. The existing wgpu renderer remains the default.
//!
//! WGSL algorithms are shared; this tree owns heap adaptation, recording and replay.
pub mod atmosphere;
pub mod bloom;
pub mod buffer;
pub mod cache;
pub mod environment;
pub mod graph;
pub mod image;
pub mod light_cluster;
pub mod post;
pub mod post_surface;
pub mod shader;
pub mod skinning;
pub mod transfer;
pub mod volumes;
