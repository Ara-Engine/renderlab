//! Original four-sample transparency background seed, before pane composition.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    shader::{InputKey, Pass},
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Device, GpuDevice, Image, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::{Format, ImageDesc},
    raster::RasterState,
    texture::{ImageShape, SampleCount, TextureDesc, ViewDimension},
};
/// Backed four-sample HDR color, retained across seed, pane draws and resolves.
pub struct MsaaColor {
    image: Image,
    size: [u32; 2],
}
impl MsaaColor {
    /// Allocate the original transparency format and sample count, without a heap view.
    pub fn new(gpu: &mut GpuDevice, size: [u32; 2]) -> Result<Self> {
        let image = gpu.create_texture(&TextureDesc {
            image: ImageDesc {
                size,
                format: Format::Rgba16Float,
                sampled: false,
                storage: false,
                attachment: true,
                copy_source: true,
                copy_destination: false,
            },
            shape: ImageShape {
                samples: SampleCount::Four,
                ..Default::default()
            },
        })?;
        Ok(Self { image, size })
    }
    /// Attachment identity for ordered transparent draws.
    pub fn image(&self) -> Image {
        self.image
    }
    /// Attachment viewport.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    /// Resolve without discarding the backed multisample contents. The destination
    /// must have attachment/copy-destination usage, as provided by Texture::new.
    pub fn resolve_to(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            output.size == self.size
                && output.format == Format::Rgba16Float
                && output.view.dimension == ViewDimension::D2
                && output.shape.samples == SampleCount::One
                && output.allocation.mips == 1
                && output.allocation.layers == 1
                && output.view.mip_count == 1
                && output.view.layer_count == 1
                && output.view.base_mip == 0
                && output.view.base_layer == 0,
            "invalid transparency resolve target"
        );
        graph.require_initialized(Resource::Image(self.image))?;
        graph.node(
            rec,
            &[
                (Resource::Image(self.image), Access::CopySource),
                (Resource::Image(output.image), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::ResolveImage {
            source: self.image,
            destination: output.image,
        });
        Ok(())
    }
    /// Retire after submitting all seed/draw/resolve uses.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Image(self.image));
        Ok(gpu.retire(Resource::Image(self.image))?)
    }
}
/// Persistent original TP_AA_SEED fullscreen pipeline.
pub struct Seed {
    pipeline: Pipeline,
}
impl Seed {
    /// Compile the original vertex/fragment pair with four backed coverage samples.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let state = RenderState {
            color_formats: vec![Format::Rgba16Float.vk().as_raw()],
            raster: RasterState {
                samples: SampleCount::Four,
                ..Default::default()
            },
            ..Default::default()
        };
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::TransparencySeed, state)?,
        })
    }
    /// Seed every coverage sample from the single-sample HDR background. No upload,
    /// device creation, submission or host wait occurs while recording.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        target: &MsaaColor,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::TransparencySeed,
            "wrong transparency seed packet"
        );
        let source = inputs
            .image(InputKey {
                group: 0,
                binding: 31,
            })
            .ok_or_else(|| anyhow::anyhow!("transparency seed source missing"))?;
        ensure!(
            source.size == target.size,
            "transparency seed size mismatch"
        );
        let mut uses = inputs.uses();
        uses.push((Resource::Image(target.image), Access::Color).into());
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::TransparencySeed".into()));
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
        rec.push(Command::Fullscreen);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire after submitting its final seed.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire_pipeline(self.pipeline)?)
    }
}
