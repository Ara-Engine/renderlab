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
    Program::new(Pass::TransparencyGlass, source(material))
}
/// The same material on a ray-traced renderer's head: the pane's sun visibility is a ray
/// its own fragment traces against the frame's scene, not a lookup in the sun map.
pub fn traced_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || traced_program_uncached(material))
}
fn traced_program_uncached(material: &str) -> Result<Program> {
    let head = crate::hardware_shadows::shader_source(
        include_str!("../../shaders/scene.wgsl"),
        crate::SampleCount::One,
    )
    .map_err(|e| anyhow::anyhow!("hardware head: {e}"))?;
    Program::new(
        Pass::TransparencyGlass,
        crate::renderer::transparency_gpu::glass_source(
            &head,
            true,
            material,
            crate::program::declares_parameters(material),
        ),
    )
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
