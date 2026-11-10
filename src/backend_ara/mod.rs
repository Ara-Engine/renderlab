//! Opt-in ara-gpu backend. The existing wgpu renderer remains the default.
//!
//! WGSL algorithms are shared; this tree owns heap adaptation, recording and replay.
pub mod atmosphere;
pub mod bindings;
pub mod bloom;
pub mod buffer;
pub mod cache;
pub mod environment;
pub mod graph;
pub mod image;
pub mod kernel;
pub mod light_cluster;
pub mod mesh;
pub mod particles;
pub mod post;
pub mod post_surface;
pub mod shader;
pub mod skinning;
pub mod sky;
pub mod toon;
pub mod transfer;
pub mod transparency_depth;
pub mod volumes;
