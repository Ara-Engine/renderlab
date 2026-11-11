//! Temporal reconstruction (TR1), composed as the wgpu renderer composes it.
//!
//! So far only the resolve kernel's interface: `tr1_resolve` reads the frame's shaded
//! samples, the camera trace's hit records and tiles and three history images, and
//! writes the next history and the reconstructed image. Nothing records it yet.
use super::shader::{Pass, Program};
use anyhow::Result;

/// `temporal_reconstruction::resolve_source` on the standard prelude: the non-RT
/// shading head, the object-motion source and the resolve itself. No ray queries.
pub(crate) fn resolve_source() -> Result<String> {
    Ok(
        crate::renderer::primary_ray_gpu::temporal_reconstruction::resolve_source(include_str!(
            "../../shaders/scene.wgsl"
        ))?,
    )
}

/// The resolve kernel, reflected.
pub fn resolve_program() -> Result<Program> {
    Program::new(Pass::TemporalResolve, resolve_source()?)
}
