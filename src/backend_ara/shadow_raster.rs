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
    /// Family-specific shadow view/frame and model records; source-sided draws
    /// also retain the normal table and material sidedness record.
    pub bindings: &'a HeapBindings<'resources>,
    /// First model/normal-table row, read as builtin(instance_index) by the source.
    pub first_instance: u32,
    /// Number of consecutive rows; noncontiguous rows require separate draws.
    pub instance_count: u32,
}
/// Persistent original depth pipeline with family-specific comparison, clear and bias.
pub struct ShadowRaster {
    pipeline: Pipeline,
    pass: Pass,
    clear: f32,
    contract: [u8; 32],
}
impl ShadowRaster {
    /// Compile the original depth-only state with its single 48-byte vertex stream.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Self::family(gpu, cache, Pass::ShadowRaster)
    }
    /// Compile the original vertex-only TP sun map: Less, clear1, constant/slope bias2.
    pub fn transparency_sun(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Self::family(gpu, cache, Pass::TransparencySunShadow)
    }
    fn family(gpu: &mut GpuDevice, cache: &ShaderCache, pass: Pass) -> Result<Self> {
        let contract = shader::packet_contract(pass, &shader::inputs(pass)?);
        let mut state = mesh_state(Format::Rgba32Float);
        state.color_formats.clear();
        state.raster.vertex_layouts.truncate(1);
        state.raster.vertex_layouts[0].attributes.truncate(1);
        let sun = pass == Pass::TransparencySunShadow;
        if sun {
            state.raster.depth_compare = ara_gpu::sampler::CompareOp::Less;
            state.raster.depth_bias = 2.;
            state.raster.depth_bias_slope = 2.;
        }
        Ok(Self {
            pipeline: cache.pipeline(gpu, pass, state)?,
            pass,
            clear: if sun { 1. } else { 0. },
            contract,
        })
    }
    /// Clear the selected mip/layer to its family's far depth, then retain winners.
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
                draw.bindings.pass() == self.pass && draw.bindings.contract() == self.contract,
                "incompatible shadow packet contract"
            );
            let end = draw
                .first_instance
                .checked_add(draw.instance_count)
                .filter(|_| draw.instance_count > 0)
                .context("invalid shadow instance range")?;
            for binding in 0..if self.pass == Pass::ShadowRaster {
                2
            } else {
                1
            } {
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
                    clear: (index == 0).then_some([self.clear; 4]),
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
