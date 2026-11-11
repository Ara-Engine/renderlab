//! Original depth-winning world/normal receiver raster, before shadow visibility.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    mesh::{MeshGeometry, mesh_state},
    post_surface::RasterDepth,
    shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    descriptors::Format,
    rendering::{RenderAttachment, RenderTarget},
    texture::{SampleCount, ViewDimension},
};
/// One original indexed/instanced receiver draw, borrowing a reflected input packet.
pub struct ReceiverDraw<'a, 'resources> {
    /// Persistent source geometry and instance slots.
    pub geometry: &'a MeshGeometry,
    /// Original camera, model, normal and material records.
    pub bindings: &'a HeapBindings<'resources>,
}
/// Two float32 receiver attachments and the original single-sample reversed depth.
pub struct ReceiverTargets<'a> {
    /// Interpolated world XYZ and precision epsilon.
    pub world: &'a Texture,
    /// Raw world-triangle normal and thin-material sign.
    pub normal: &'a Texture,
    /// Greater wins; clear depth zero.
    pub depth: &'a RasterDepth,
}
/// Persistent original vs/fs_receiver pipeline. No ray query executes in this prepass.
pub struct Receivers {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl Receivers {
    /// Compile both original stages, two float32 MRTs, and single-sample depth writes.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let contract = super::shader::packet_contract(
            Pass::ReceiverRaster,
            &super::shader::inputs(Pass::ReceiverRaster)?,
        );
        let mut state = mesh_state(Format::Rgba32Float);
        state.color_formats.push(Format::Rgba32Float.vk().as_raw());
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::ReceiverRaster, state)?,
            contract,
        })
    }
    /// Clear once, then preserve depth-winning receivers across all ordered draws.
    /// Color outputs select one mip/layer each; their viewport must match depth.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        draws: &[ReceiverDraw<'_, '_>],
        targets: ReceiverTargets<'_>,
    ) -> Result<()> {
        ensure!(!draws.is_empty(), "receiver prepass needs a draw");
        for output in [targets.world, targets.normal] {
            ensure!(
                output.format == Format::Rgba32Float
                    && output.size == targets.depth.size()
                    && output.shape.samples == SampleCount::One
                    && output.view.dimension == ViewDimension::D2
                    && output.view.mip_count == 1
                    && output.view.layer_count == 1,
                "invalid receiver attachment"
            );
        }
        ensure!(
            targets.depth.samples() == SampleCount::One,
            "receiver depth must be single-sampled"
        );
        ensure!(
            draws
                .iter()
                .all(|d| d.bindings.pass() == Pass::ReceiverRaster
                    && d.bindings.contract() == self.contract),
            "incompatible receiver packet contract"
        );
        for (index, draw) in draws.iter().enumerate() {
            let mut uses = draw.bindings.uses();
            uses.extend(
                draw.geometry
                    .raster_uses()
                    .map(super::graph::ResourceUse::from),
            );
            uses.extend([
                targets.world.usage(Access::Color),
                targets.normal.usage(Access::Color),
                (Resource::Image(targets.depth.image()), Access::Depth).into(),
            ]);
            graph.node_uses(rec, &uses)?;
            rec.push(Command::BeginPass("ara::ReceiverRaster".into()));
            rec.push(Command::BeginRenderingSubresources {
                colors: [targets.world, targets.normal]
                    .into_iter()
                    .map(|output| RenderAttachment {
                        target: RenderTarget {
                            image: output.image,
                            mip: output.view.base_mip,
                            layer: output.view.base_layer,
                        },
                        clear: (index == 0).then_some([0.; 4]),
                    })
                    .collect(),
                depth: Some(
                    Attachment {
                        image: targets.depth.image(),
                        clear: (index == 0).then_some([0.; 4]),
                    }
                    .into(),
                ),
                resolves: vec![],
                size: targets.depth.size(),
            });
            rec.push(Command::Pipeline(self.pipeline));
            rec.push(Command::Constants(draw.bindings.roots()));
            draw.geometry.draw(rec);
            rec.push(Command::EndRendering);
            rec.push(Command::EndPass);
        }
        Ok(())
    }
    /// Nothing of its own to release: its pipelines stay with the shader cache
    /// (`ShaderCache::retire`).
    pub fn retire(self, _: &mut GpuDevice) -> Result<()> {
        Ok(())
    }
}
