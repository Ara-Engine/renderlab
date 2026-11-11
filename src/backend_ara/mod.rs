//! Opt-in ara-gpu backend. The existing wgpu renderer remains the default.
//!
//! WGSL algorithms are shared; this tree owns heap adaptation, recording and replay.
pub mod acceleration;
pub mod atmosphere;
pub mod bindings;
pub mod bloom;
pub mod buffer;
pub mod cache;
pub mod environment;
pub mod fallback_present;
pub mod graph;
pub mod hzb;
pub mod image;
pub mod kernel;
pub mod light_cluster;
pub mod mesh;
pub mod particle_billboard;
pub mod particles;
pub mod post;
pub mod post_surface;
pub mod primary_check;
pub mod primary_shade;
pub mod primary_trace;
pub mod program_bins;
pub mod shader;
pub mod skinning;
pub mod sky;
pub mod toon;
pub mod transfer;
pub mod transparency_depth;
pub mod transparency_glass;
pub mod transparency_seed;
pub mod visibility_cull;
pub mod visibility_raster;
pub mod volumes;

/// Depth-winning geometric receivers for the fresh visibility path.
pub mod receiver_raster;

/// Fresh directional shadow queries from geometric receiver images.
pub mod fresh_visibility;

/// Original source-sided shadow depth raster into selected atlas subresources.
pub mod shadow_raster;

/// Per-object colored sun-transmission atlas layers.
pub mod transmission_shadow;

/// The forward lit frame assembled from the ported passes.
pub mod forward;

/// The engine-level entry: `engine::Scene` over the frames above.
pub mod engine;

/// The world radiance cache's kernels.
pub mod gi_cache;
pub mod resolve_probe;
pub mod temporal;
