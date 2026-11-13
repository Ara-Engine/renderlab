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
    hardware_soft_part(material, ALL_TERMS)
}
/// `hardware_soft_program` keeping only the `terms` shading terms: the pass log's detail
/// parts (src/shade_detail.rs).
pub(crate) fn hardware_soft_part(material: &str, terms: u32) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(&format!("{terms}:{material}")), || {
        hardware_soft_program_uncached(material, terms)
    })
}
/// Every shading term (`RL_SHADE_TERMS` as shaders/program_api.wgsl declares it).
const ALL_TERMS: u32 = u32::MAX;
fn hardware_soft_program_uncached(material: &str, terms: u32) -> Result<Program> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    let source =
        crate::program::compose(&head, material, crate::program::Stage::Traced, parameters).source;
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
    hardware_soft_list_part(material, ALL_TERMS)
}
/// `hardware_soft_list_program` keeping only the `terms` shading terms.
pub(crate) fn hardware_soft_list_part(material: &str, terms: u32) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(&format!("{terms}:{material}")), || {
        list_program_uncached(material, terms)
    })
}
fn list_program_uncached(material: &str, terms: u32) -> Result<Program> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let parameters = crate::program::declares_parameters(material);
    Ok(Program::new(
        Pass::PrimaryShadeList,
        with_terms(
            crate::program::compose_list(
                &head,
                material,
                crate::program::Stage::Traced,
                parameters,
            )
            .source,
            terms,
        ),
    )?
    .checked())
}

#[cfg(test)]
mod tests {
    use crate::program::{Composed, Stage, compose, compose_hits, compose_list};

    /// Every traced shading module the port compiles for the standard program (the hardware
    /// soft head's tiles, list and hit modules, and the non-RT head's) validates, under
    /// whatever composition switches the process runs with (`RENDERLAB_RL_DEFERRED=1`,
    /// `RENDERLAB_HIT_LOD`, ...); with `RL_DEFERRED` the deferred modules carry its outputs.
    /// Each module's program range names the program's text (not yet with `RL_DEFERRED`: its
    /// range shift is U-RTVIS's fix on t3/rtvis; drop the condition once that is merged).
    #[test]
    fn port_traced_modules_validate_under_the_process_switches() {
        let scene = include_str!("../../shaders/scene.wgsl");
        let [_, soft] = crate::renderer::primary_ray_gpu::shading_heads(scene).unwrap();
        let (_, [non_rt, _]) = crate::renderer::primary_ray_gpu::non_rt_sources(scene).unwrap();
        let program = crate::STANDARD_PROGRAM;
        let modules: [(&str, Composed); 5] = [
            ("soft tiles", compose(&soft, program, Stage::Traced, false)),
            (
                "soft list",
                compose_list(&soft, program, Stage::Traced, false),
            ),
            (
                "soft hits",
                compose_hits(&soft, program, Stage::Traced, false, false),
            ),
            (
                "non-RT tiles",
                compose(&non_rt, program, Stage::Traced, false),
            ),
            (
                "non-RT list",
                compose_list(&non_rt, program, Stage::Traced, false),
            ),
        ];
        for (name, composed) in &modules {
            let source = &composed.source;
            if !crate::program::rl_deferred() {
                assert_eq!(
                    &source[composed.program.clone()],
                    program,
                    "{name}: program range"
                );
            }
            if crate::program::rl_deferred() && !name.ends_with("hits") {
                assert!(
                    source.contains("rl_deferred_store(pixel,input.world);"),
                    "{name}"
                );
            }
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
        }
    }
}
