//! Original programmable transparent pane shading into seeded coverage samples.
use super::shader::memo::{self, Memo};
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    mesh::{MeshGeometry, mesh_state},
    shader::{InputKey, Pass, Program},
    transparency_seed::MsaaColor,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    descriptors::Format,
    texture::SampleCount,
};
/// Compose the production non-RT transparent material, preserving its authored hooks.
pub fn program(material: &str) -> Result<Program> {
    Ok(Program::new(Pass::TransparencyGlass, source(material))?.checked())
}
/// The same material on a ray-traced renderer's head: the pane's sun visibility is a ray
/// its own fragment traces against the frame's scene, not a lookup in the sun map.
pub fn traced_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || traced_program_uncached(material))
}
fn traced_program_uncached(material: &str) -> Result<Program> {
    Ok(Program::new(Pass::TransparencyGlass, traced_source(material)?)?.checked())
}
/// The glass module on the ray-traced head (`traced_program`'s source).
fn traced_source(material: &str) -> Result<String> {
    let head = crate::hardware_shadows::shader_source(
        include_str!("../../shaders/scene.wgsl"),
        crate::SampleCount::One,
    )
    .map_err(|e| anyhow::anyhow!("hardware head: {e}"))?;
    Ok(crate::renderer::transparency_gpu::glass_source(
        &head,
        true,
        material,
        crate::program::declares_parameters(material),
    ))
}

/// One section of shaders/pane_pixels.wgsl (`crate::pane_pixels::section`).
fn pane_section(name: &str) -> Result<&'static str> {
    crate::pane_pixels::section(name).map_err(|e| anyhow::anyhow!(e))
}
/// The traced glass material's pane id raster: which of the pane's triangles covers each
/// of a pixel's four samples (the glass module's `vs`, `tp_pane_id_fs`).
pub fn pane_id_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || {
        Ok(Program::new(Pass::PaneId, pane_id_source(material)?)?.checked())
    })
}
pub(crate) fn pane_id_source(material: &str) -> Result<String> {
    // `tp_pane_id_fs` reads the triangle's index, an extension WGSL enables up front.
    Ok(format!(
        "enable primitive_index;\n{}\n{}\n",
        traced_source(material)?,
        pane_section("PANE_RASTER")?
    ))
}
/// The traced glass material's per-pixel pane shading (`tp_pane_shade`): `fs_transparent`'s
/// own body as a function of explicit derivatives, on a compute stage.
pub fn pane_shade_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || {
        Ok(Program::new(Pass::PaneShade, pane_shade_source(material)?)?.checked())
    })
}
pub(crate) fn pane_shade_source(material: &str) -> Result<String> {
    let glass = crate::pane_pixels::explicit_sampling(&traced_source(material)?)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(format!(
        "{glass}\n{}\n{}\n{}\n",
        crate::pane_pixels::fragment(&glass).map_err(|e| anyhow::anyhow!(e))?,
        pane_section("PANE_COMMON")?,
        pane_section("PANE_COMPUTE")?
    ))
}
pub(crate) fn source(material: &str) -> String {
    crate::renderer::transparency_gpu::glass_source(
        include_str!("../../shaders/scene.wgsl"),
        false,
        material,
        crate::program::declares_parameters(material),
    )
}
/// Persistent four-sample original vs/fs_transparent pipeline. Vulkan needs the
/// same fragment-barycentric capability as the opaque programmable raster entry.
pub struct Glass {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl Glass {
    /// Compile caller-authored material WGSL with depth disabled, as on Renderlab.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, program: &Program) -> Result<Self> {
        ensure!(
            program.pass() == Pass::TransparencyGlass,
            "wrong transparent program family"
        );
        let mut state = mesh_state(Format::Rgba16Float);
        state.depth_format = None;
        state.depth_test = false;
        state.depth_write = false;
        state.blend = false;
        state.raster.samples = SampleCount::Four;
        Ok(Self {
            pipeline: cache.program_state(gpu, program, state)?,
            contract: program.contract(),
        })
    }
    /// Draw one sorted pane, loading coverage samples seeded by Seed. Resolve the
    /// current color into the packet's behind image before another overlapping pane.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        geometry: &MeshGeometry,
        target: &MsaaColor,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::TransparencyGlass && inputs.contract() == self.contract,
            "incompatible transparent packet"
        );
        for binding in [31, 32] {
            let image = inputs
                .image(InputKey { group: 0, binding })
                .ok_or_else(|| anyhow::anyhow!("transparent color/depth input missing"))?;
            ensure!(
                image.size == target.size(),
                "transparent color/depth viewport mismatch"
            );
        }
        graph.require_initialized(Resource::Image(target.image()))?;
        let mut uses = inputs.uses();
        uses.extend(geometry.raster_uses().map(super::graph::ResourceUse::from));
        uses.push((Resource::Image(target.image()), Access::Color).into());
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::TransparentPane".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: target.image(),
                clear: None,
            }],
            depth: None,
            size: target.size(),
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(inputs.roots()));
        geometry.draw(rec);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
}

/// The pane id raster's pipeline (`pane_id_program`): triangle ids into a four-sample R32Uint
/// target, no depth, no blending: the glass raster's coverage.
pub struct PaneIds {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl PaneIds {
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, program: &Program) -> Result<Self> {
        ensure!(
            program.pass() == Pass::PaneId,
            "wrong pane id program family"
        );
        let mut state = mesh_state(Format::R32Uint);
        state.depth_format = None;
        state.depth_test = false;
        state.depth_write = false;
        state.blend = false;
        state.raster.samples = SampleCount::Four;
        Ok(Self {
            pipeline: cache.program_state(gpu, program, state)?,
            contract: program.contract(),
        })
    }
    /// Draws one pane into `target` (four-sample R32Uint at the frame's size), cleared to 0
    /// first.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        geometry: &MeshGeometry,
        target: &super::image::Texture,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::PaneId && inputs.contract() == self.contract,
            "incompatible pane id packet"
        );
        let mut uses = inputs.uses();
        uses.extend(geometry.raster_uses().map(super::graph::ResourceUse::from));
        uses.push((Resource::Image(target.image), Access::Color).into());
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::PaneIds".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: target.image,
                clear: Some([0.; 4]),
            }],
            depth: None,
            size: target.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(inputs.roots()));
        geometry.draw(rec);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pane entries compose with the standard program on the traced head, validate, and
    /// reflect what the frame binds: the id raster the opaque depth, the shading its pane
    /// geometry, the id raster's image, the output and its record.
    #[test]
    fn pane_programs_compose_validate_and_reflect_their_inputs() {
        let ids = pane_id_program(crate::STANDARD_PROGRAM).unwrap();
        let shade = pane_shade_program(crate::STANDARD_PROGRAM).unwrap();
        let bound = |p: &Program, binding: u32| {
            p.inputs()
                .iter()
                .any(|i| i.key.group == 0 && i.key.binding == binding)
        };
        assert!(bound(&ids, 32));
        for binding in [70, 71, 72, 76, 77] {
            assert!(bound(&shade, binding), "the shading binds {binding}");
        }
    }
}
