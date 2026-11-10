//! Original shadow depth pair, preserving source-side winding and native instance IDs.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    mesh::{MeshGeometry, mesh_state},
    shader::{self, InputKey, Pass},
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline,
    commands::{Command, Recorder},
    descriptors::Format,
    rendering::{RenderAttachment, RenderTarget},
    texture::{SampleCount, ViewDimension},
};
/// One indexed mesh and a native contiguous range of model-table instances.
pub struct ShadowDraw<'a, 'resources> {
    /// Source vertex/index storage. Its separate per-instance slot stream is unused.
    pub geometry: &'a MeshGeometry,
    /// ShadowView112B, model/normal tables and the material's sidedness record.
    pub bindings: &'a HeapBindings<'resources>,
    /// First model/normal-table row, read as builtin(instance_index) by the source.
    pub first_instance: u32,
    /// Number of consecutive rows; noncontiguous rows require separate draws.
    pub instance_count: u32,
}
/// Persistent original vs_shadow/fs_shadow pipeline, reversed-depth Greater, no bias.
pub struct ShadowRaster {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl ShadowRaster {
    /// Compile the original depth-only state with its single 48-byte vertex stream.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let contract =
            shader::packet_contract(Pass::ShadowRaster, &shader::inputs(Pass::ShadowRaster)?);
        let mut state = mesh_state(Format::Rgba32Float);
        state.color_formats.clear();
        state.raster.vertex_layouts.truncate(1);
        state.raster.vertex_layouts[0].attributes.truncate(1);
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::ShadowRaster, state)?,
            contract,
        })
    }
    /// Clear selected mip/layer to zero, then retain depth winners across every draw.
    /// All ranges and contracts are checked before recording the first draw.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        draws: &[ShadowDraw<'_, '_>],
        target: &Texture,
    ) -> Result<()> {
        ensure!(
            !draws.is_empty()
                && target.format == Format::Depth32Float
                && target.shape.samples == SampleCount::One
                && target.view.dimension == ViewDimension::D2
                && target.view.mip_count == 1
                && target.view.layer_count == 1,
            "invalid shadow depth target/draws"
        );
        for draw in draws {
            ensure!(
                draw.bindings.pass() == Pass::ShadowRaster
                    && draw.bindings.contract() == self.contract,
                "incompatible shadow packet contract"
            );
            let end = draw
                .first_instance
                .checked_add(draw.instance_count)
                .filter(|_| draw.instance_count > 0)
                .context("invalid shadow instance range")?;
            for binding in [0, 1] {
                let buffer = draw
                    .bindings
                    .buffer(InputKey { group: 1, binding })
                    .context("shadow model/normal table missing")?;
                ensure!(
                    u64::from(end) <= buffer.len / 64,
                    "shadow instance range exceeds model/normal table"
                );
            }
        }
        for (index, draw) in draws.iter().enumerate() {
            let mut uses = draw.bindings.uses();
            uses.extend(
                draw.geometry
                    .native_instance_uses()
                    .map(super::graph::ResourceUse::from),
            );
            uses.push(target.usage(Access::Depth));
            graph.node_uses(rec, &uses)?;
            rec.push(Command::BeginPass("ara::ShadowRaster".into()));
            rec.push(Command::BeginRenderingSubresources {
                colors: vec![],
                depth: Some(RenderAttachment {
                    target: RenderTarget {
                        image: target.image,
                        mip: target.view.base_mip,
                        layer: target.view.base_layer,
                    },
                    clear: (index == 0).then_some([0.; 4]),
                }),
                resolves: vec![],
                size: target.size,
            });
            rec.push(Command::Pipeline(self.pipeline));
            rec.push(Command::Constants(draw.bindings.roots()));
            draw.geometry
                .draw_native_instances(rec, draw.first_instance, draw.instance_count);
            rec.push(Command::EndRendering);
            rec.push(Command::EndPass);
        }
        Ok(())
    }
    /// Retire after the final submitted atlas draw.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire_pipeline(self.pipeline)?)
    }
}
