//! Opt-in ara-gpu backend. The existing wgpu renderer remains the default.
//!
//! WGSL algorithms are shared; this tree owns heap adaptation, recording and replay.
pub mod bloom;
pub mod buffer;
pub mod cache;
pub mod graph;
pub mod image;
pub mod light_cluster;
pub mod post;
pub mod shader;
pub mod transfer;
