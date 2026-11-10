//! Opt-in ara-gpu backend. The existing wgpu renderer remains the default.
//!
//! WGSL algorithms are shared; this tree owns heap adaptation, recording and replay.
pub mod cache;
pub mod graph;
pub mod image;
pub mod post;
pub mod shader;
