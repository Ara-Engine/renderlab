//! Original nearest colored transmission for one object per atlas layer.
use super::{
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    mesh::mesh_state,
    shader::{self, InputKey, Pass},
    shadow_raster::ShadowDraw,
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline,
    commands::{Command, Recorder},
    descriptors::Format,
    rendering::{RenderAttachment, RenderTarget},
    texture::{SampleCount, ViewDimension},
};
/// Persistent original tx_vs/tx_fs pipeline, without blend or hardware culling.
pub struct Transmission {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl Transmission {
    /// Compile original HDR color plus forward Less depth state.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let contract = shader::packet_contract(
            Pass::TransmissionShadow,
            &shader::inputs(Pass::TransmissionShadow)?,
        );
        let mut state = mesh_state(Format::Rgba16Float);
        state.raster.vertex_layouts.truncate(1);
        state.raster.vertex_layouts[0]
            .attributes
            .retain(|a| a.location != 1);
        state.raster.depth_compare = ara_gpu::sampler::CompareOp::Less;
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::TransmissionShadow, state)?,
            contract,
        })
    }
    /// Clear color to white/depth to one, then draw this object's instance range.
    /// Each object gets a separate layer as in Renderlab; lighting combines layers.
    pub fn record_layer(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        draw: &ShadowDraw<'_, '_>,
        output: &Texture,
        depth: &Texture,
    ) -> Result<()> {
        ensure!(
            output.format == Format::Rgba16Float
                && depth.format == Format::Depth32Float
                && output.size == depth.size,
            "invalid transmission color/depth format/extent"
        );
        for target in [output, depth] {
            ensure!(
                target.shape.samples == SampleCount::One
                    && target.view.dimension == ViewDimension::D2
                    && target.view.mip_count == 1
                    && target.view.layer_count == 1,
                "transmission needs a single mip/slice"
            );
        }
        ensure!(
            draw.bindings.pass() == Pass::TransmissionShadow
                && draw.bindings.contract() == self.contract,
            "incompatible transmission packet contract"
        );
        let end = draw
            .first_instance
            .checked_add(draw.instance_count)
            .filter(|_| draw.instance_count > 0)
            .context("invalid transmission instance range")?;
        let models = draw
            .bindings
            .buffer(InputKey {
                group: 1,
                binding: 0,
            })
            .context("transmission model table missing")?;
        ensure!(
            u64::from(end) <= models.len / 64,
            "transmission instance range exceeds model table"
        );
        let mut uses = draw.bindings.uses();
        uses.extend(
            draw.geometry
                .native_instance_uses()
                .map(super::graph::ResourceUse::from),
        );
        uses.extend([output.usage(Access::Color), depth.usage(Access::Depth)]);
        graph.node_uses(rec, &uses)?;
        let attachment = |t: &Texture, clear| RenderAttachment {
            target: RenderTarget {
                image: t.image,
                mip: t.view.base_mip,
                layer: t.view.base_layer,
            },
            clear: Some([clear; 4]),
        };
        rec.push(Command::BeginPass("ara::TransmissionShadow".into()));
        rec.push(Command::BeginRenderingSubresources {
            colors: vec![attachment(output, 1.)],
            depth: Some(attachment(depth, 1.)),
            resolves: vec![],
            size: output.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(draw.bindings.roots()));
        draw.geometry
            .draw_native_instances(rec, draw.first_instance, draw.instance_count);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire after all object's layer submissions.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire_pipeline(self.pipeline)?)
    }
}
