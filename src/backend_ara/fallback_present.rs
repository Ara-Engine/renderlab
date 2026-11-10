//! Original scene.wgsl fallback presenter, distinct from display transfer or post tone.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    shader::{InputKey, Pass},
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline,
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
    rendering::{RenderAttachment, RenderTarget},
    texture::{SampleCount, ViewDimension},
};
/// Persistent original Reinhard fallback presenter.
pub struct FallbackPresent {
    pipeline: Pipeline,
    format: Format,
}
impl FallbackPresent {
    /// Compile the original scene fullscreen pair for the caller's color format.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, format: Format) -> Result<Self> {
        ensure!(
            matches!(
                format,
                Format::Rgba16Float | Format::Rgba32Float | Format::Rgba8 | Format::Rgba8Srgb
            ),
            "fallback presenter requires float color"
        );
        let state = RenderState {
            color_formats: vec![format.vk().as_raw()],
            ..Default::default()
        };
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::FallbackPresent, state)?,
            format,
        })
    }
    /// Map HDR as rgb/(1+rgb), alpha one. The source and target sizes must match;
    /// neither resampling nor an extra display-transfer shader is inserted.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::FallbackPresent
                && output.format == self.format
                && output.shape.samples == SampleCount::One
                && output.view.dimension == ViewDimension::D2
                && output.view.mip_count == 1
                && output.view.layer_count == 1,
            "invalid fallback presenter packet/target"
        );
        let source = inputs
            .image(InputKey {
                group: 0,
                binding: 1,
            })
            .ok_or_else(|| anyhow::anyhow!("fallback HDR source missing"))?;
        ensure!(
            source.size == output.size,
            "fallback presenter size mismatch"
        );
        let mut uses = inputs.uses();
        uses.push(output.usage(Access::Color));
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::FallbackPresent".into()));
        rec.push(Command::BeginRenderingSubresources {
            colors: vec![RenderAttachment {
                target: RenderTarget {
                    image: output.image,
                    mip: output.view.base_mip,
                    layer: output.view.base_layer,
                },
                clear: Some([0.; 4]),
            }],
            depth: None,
            resolves: vec![],
            size: output.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(inputs.roots()));
        rec.push(Command::Fullscreen);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire after submitting its last draw.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire_pipeline(self.pipeline)?)
    }
}
