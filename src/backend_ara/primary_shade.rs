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
    hardware_soft_part(material, ALL_TERMS, false)
}
/// `hardware_soft_program` keeping only the `terms` shading terms: the pass log's detail
/// parts (src/shade_detail.rs). `lean`: the module for frames whose `program::FrameInputs`
/// are lean (`program::lean_variant`).
pub(crate) fn hardware_soft_part(material: &str, terms: u32, lean: bool) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(&format!("{terms}:{lean}:{material}")), || {
        hardware_soft_program_uncached(material, terms, lean)
    })
}
/// Measurement: the uncomposed-then-compiled standard traced module, full or lean, and its
/// composed text's length (tests/ara_backend lean_shading). Bypasses the memo.
pub fn lean_probe(material: &str, lean: bool) -> Result<(Program, usize)> {
    let program = hardware_soft_program_uncached(material, ALL_TERMS, lean)?;
    let len = program.source().len();
    Ok((program, len))
}
/// Every shading term (`RL_SHADE_TERMS` as shaders/program_api.wgsl declares it).
pub(crate) const ALL_TERMS: u32 = u32::MAX;
fn hardware_soft_program_uncached(material: &str, terms: u32, lean: bool) -> Result<Program> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    let composed = crate::program::compose(&head, material, crate::program::Stage::Traced, parameters);
    let source = if lean { crate::program::lean_variant(composed) } else { composed }.source;
    Ok(Program::new(Pass::PrimaryShadeTiles, with_terms(source, terms))?.checked())
}
/// `source` unchanged with every term, else through `shade_detail::without_terms`.
fn with_terms(source: String, terms: u32) -> String {
    if terms == ALL_TERMS {
        source
    } else {
        crate::renderer::shade_detail::without_terms(&source, terms)
    }
}
/// A program's reflection-hit module on the hardware soft head (`program::compose_hits`):
/// the traced module with the `reflection_hits` entry, which shades one bin of H.
pub fn hardware_soft_hits_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || {
        Ok(Program::new(Pass::ReflectionHits, hits_source(material)?)?.checked())
    })
}
/// The hit module's text (`Pass::ReflectionHits`).
pub(crate) fn hits_source(material: &str) -> Result<String> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    Ok(crate::program::compose_hits(
        &head,
        material,
        crate::program::Stage::Traced,
        parameters,
        false,
    )
    .source)
}
/// The list entry on the same hardware soft head: one program's share of a frame of
/// several programs, shading the tiles the binning pass listed for its slot
/// (`program::compose_list`, as `ProgramPipelines::list` compiles it).
pub fn hardware_soft_list_program(material: &str) -> Result<Program> {
    hardware_soft_list_part(material, ALL_TERMS, false)
}
/// `hardware_soft_list_program` keeping only the `terms` shading terms; `lean` as for
/// `hardware_soft_part`.
pub(crate) fn hardware_soft_list_part(material: &str, terms: u32, lean: bool) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(&format!("{terms}:{lean}:{material}")), || {
        list_program_uncached(material, terms, lean)
    })
}
fn list_program_uncached(material: &str, terms: u32, lean: bool) -> Result<Program> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    Ok(Program::new(
        Pass::PrimaryShadeList,
        with_terms(
            {
                let composed = crate::program::compose_list(
                    &head,
                    material,
                    crate::program::Stage::Traced,
                    parameters,
                );
                if lean { crate::program::lean_variant(composed) } else { composed }.source
            },
            terms,
        ),
    )?
    .checked())
}
/// RENDERLAB_MATERIAL_PASS (program.rs `material_variant`): the hardware soft head's tiles or
/// list module of `material` as the material pass (`Write`) or as record-reading shading
/// (`Read`), or None for a program without the record mapping (it keeps its single kernel).
pub(crate) fn hardware_soft_material(
    material: &str,
    list: bool,
    role: crate::program::MaterialRole,
) -> Result<Option<Program>> {
    static KEPT: Memo<[u8; 32], Option<Program>> = Memo::new();
    KEPT.get(memo::digest(&format!("{list}:{role:?}:{material}")), || {
        let [_, head] = crate::renderer::primary_ray_gpu::shading_heads(include_str!(
            "../../shaders/scene.wgsl"
        ))?;
        let parameters = crate::program::declares_parameters(material);
        let composed = if list {
            crate::program::compose_list(&head, material, crate::program::Stage::Traced, parameters)
        } else {
            crate::program::compose(&head, material, crate::program::Stage::Traced, parameters)
        };
        let pass = if list {
            Pass::PrimaryShadeList
        } else {
            Pass::PrimaryShadeTiles
        };
        // The variant carries the trace's edge repair, which the port runs without
        // hit-vertex return (primary_trace.rs `portable_trace`).
        crate::program::material_variant(composed, material, role)
            .map(|c| {
                let source = super::primary_trace::without_vertex_return(&c.source);
                Ok(Program::new(pass, source)?.checked())
            })
            .transpose()
    })
}
/// The material pass's record writer for `material` (`hardware_soft_material`, `Write`).
pub fn hardware_soft_material_writer(material: &str, list: bool) -> Result<Option<Program>> {
    hardware_soft_material(material, list, crate::program::MaterialRole::Write)
}
/// The record-reading shading for `material` (`hardware_soft_material`, `Read`).
pub fn hardware_soft_material_reader(material: &str, list: bool) -> Result<Option<Program>> {
    hardware_soft_material(material, list, crate::program::MaterialRole::Read)
}
