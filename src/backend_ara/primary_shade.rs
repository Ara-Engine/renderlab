//! Original non-RT deferred shading entries, composed with caller material WGSL.
use super::shader::memo::{self, Memo};
use super::shader::{Pass, Program};
use anyhow::Result;
/// Dispatch family for one program's deferred primary shading.
#[derive(Clone, Copy)]
pub enum Entry {
    /// A single material program shades the full primary region and clear blocks.
    Tiles,
    /// A material program shades its GPU-built per-program tile list.
    List,
}
impl Entry {
    /// Compose and reflect the same non-RT program API and entry as wgpu uses.
    /// Programs keep their Surface/Parameters definitions and material hooks.
    pub fn program(self, material: &str) -> Result<Program> {
        let list = matches!(self, Self::List);
        Ok(Program::new(
            if list {
                Pass::PrimaryShadeList
            } else {
                Pass::PrimaryShadeTiles
            },
            source(material, list)?,
        )?
        .checked())
    }
}
pub(crate) fn source(material: &str, list: bool) -> Result<String> {
    let (_, [head, _]) =
        crate::renderer::primary_ray_gpu::non_rt_sources(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    let composed = if list {
        crate::program::compose_list(&head, material, crate::program::Stage::Traced, parameters)
    } else {
        crate::program::compose(&head, material, crate::program::Stage::Traced, parameters)
    };
    Ok(composed.source)
}
/// The tiled entry as `HardwareRt` composes it for soft sun shadows: the hardware
/// soft head, which reads the fresh visibility counts and declares the scene for
/// local-light shadow rays. It needs a ray-query device. Compiled natively on Metal
/// with the standard program; not yet executed against a reference.
pub fn hardware_soft_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || {
        hardware_soft_program_uncached(material)
    })
}
fn hardware_soft_program_uncached(material: &str) -> Result<Program> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    Ok(Program::new(
        Pass::PrimaryShadeTiles,
        crate::program::compose(&head, material, crate::program::Stage::Traced, parameters).source,
    )?
    .checked())
}
/// The list entry on the same hardware soft head: one program's share of a frame of
/// several programs, shading the tiles the binning pass listed for its slot
/// (`program::compose_list`, as `ProgramPipelines::list` compiles it).
pub fn hardware_soft_list_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || list_program_uncached(material))
}
fn list_program_uncached(material: &str) -> Result<Program> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    Ok(Program::new(
        Pass::PrimaryShadeList,
        crate::program::compose_list(&head, material, crate::program::Stage::Traced, parameters)
            .source,
    )?
    .checked())
}
