//! The local-light visibility pass on the port (`RENDERLAB_LIGHT_VIS_PASS=1`,
//! src/light_vis_pass.rs): before shading, a kernel traces each covered pixel's shadowed local
//! lights and stores their visibility in binding 73; shading's light loop reads it there. With
//! the material record on (`RENDERLAB_MATERIAL_PASS=1`), the kernel's pixels come from the
//! record's receiver layers (`program::light_vis_from_record`) instead of decoding the hit.
use super::shader::memo::Memo;
use super::shader::{Pass, Program};
use anyhow::{Context, Result, ensure};

/// The pass's kernel, composed on the hardware soft head; `record` reads the receiver layers.
pub fn program(record: bool) -> Result<Program> {
    static KEPT: Memo<bool, Program> = Memo::new();
    KEPT.get(record, || Program::new(Pass::LightVisibility, source(record)?))
}
/// The kernel's source: the engine's pass-only program composed with the pass on.
pub(crate) fn source(record: bool) -> Result<String> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let composed = crate::program::compose(
        &head,
        crate::renderer::primary_ray_gpu::light_vis_pass::PASS_PROGRAM,
        crate::program::Stage::Traced,
        false,
    );
    ensure!(
        composed.source.contains("fn light_vis_tiles("),
        "the light visibility pass is off"
    );
    if record {
        crate::program::light_vis_from_record(composed.source).context("light visibility seam changed")
    } else {
        Ok(composed.source)
    }
}
