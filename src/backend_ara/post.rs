//! The reusable post-pass encoder. No device creation, upload or wait inside a pass.
use super::{cache::ShaderCache, graph::Graph, image::Texture, shader::Pass};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Buffer, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
    vk,
};

/// Exact 80-byte Settings record shared with the original post WGSL.
pub struct SettingsRecord {
    /// Owned storage buffer identity; the caller retires it with the device.
    pub buffer: Buffer,
    pub(crate) slot: u32,
}
impl SettingsRecord {
    /// Publish immutable initial settings. Update only after GPU completion or via queued upload.
    pub fn new(gpu: &mut GpuDevice, words: &[f32; 20]) -> Result<Self> {
        ensure!(
            words.iter().all(|v| v.is_finite()),
            "post settings must be finite"
        );
        let buffer = gpu.create_buffer(80, vk::BufferUsageFlags::STORAGE_BUFFER, true)?;
        gpu.upload(buffer, bytemuck::cast_slice(words))?;
        let slot = gpu.storage_buffer(buffer)?.index;
        Ok(Self { buffer, slot })
    }
}
/// Direct heap inputs; their image identities also supply graph reachability.
pub struct FilterInputs<'a> {
    /// Primary filter input.
    pub colour: &'a Texture,
    /// Coarser level for bloom upsample.
    pub second: &'a Texture,
    /// Output-space velocity and linear depth.
    pub auxiliary: &'a Texture,
    /// Maximum velocity per 16×16 tile.
    pub velocity_tiles: &'a Texture,
    /// Validated shared settings record.
    pub settings: &'a SettingsRecord,
}
/// Persistent pipelines and one immutable linear clamp sampler.
pub struct PostPasses {
    downsample: Pipeline,
    upsample: Pipeline,
    tone: Pipeline,
    sampler: u32,
    output_format: Format,
}
impl PostPasses {
    /// Build/load once at startup. Failure leaves the caller's previous instance usable.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, output_format: Format) -> Result<Self> {
        ensure!(
            matches!(output_format, Format::Rgba32Float | Format::Rgba8),
            "unsupported tone output format"
        );
        let downsample = cache.pipeline(gpu, Pass::Downsample, RenderState::default())?;
        let upsample = cache.pipeline(gpu, Pass::Upsample, RenderState::default())?;
        let tone = cache.pipeline(
            gpu,
            Pass::Tone,
            RenderState {
                color_formats: vec![output_format.vk().as_raw()],
                ..Default::default()
            },
        )?;
        let sampler = gpu
            .sampler(&ara_gpu::sampler::SamplerDesc::default())?
            .index;
        Ok(Self {
            downsample,
            upsample,
            tone,
            sampler,
            output_format,
        })
    }
    /// Append one bloom kernel and its graph-generated dependencies.
    pub fn filter(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        pass: Pass,
        inputs: &FilterInputs<'_>,
        output: &Texture,
    ) -> Result<()> {
        let pipeline = match pass {
            Pass::Downsample => self.downsample,
            Pass::Upsample => self.upsample,
            _ => anyhow::bail!("not a filter pass"),
        };
        ensure!(
            output.format == Format::Rgba16Float,
            "filter output must be RGBA16Float"
        );
        let slot = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("filter output has no storage view"))?;
        graph.node(
            rec,
            &[
                (Resource::Image(inputs.colour.image), Access::ShaderRead),
                (Resource::Image(inputs.second.image), Access::ShaderRead),
                (Resource::Image(inputs.auxiliary.image), Access::ShaderRead),
                (
                    Resource::Image(inputs.velocity_tiles.image),
                    Access::ShaderRead,
                ),
                (Resource::Buffer(inputs.settings.buffer), Access::ShaderRead),
                (Resource::Image(output.image), Access::Storage),
            ],
        )?;
        rec.push(Command::BeginPass(format!("ara::{pass:?}")));
        rec.push(Command::Pipeline(pipeline));
        rec.push(Command::Constants(RootConstants {
            words: [
                inputs.colour.sampled,
                inputs.second.sampled,
                inputs.auxiliary.sampled,
                inputs.velocity_tiles.sampled,
                slot,
                inputs.settings.slot,
                self.sampler,
                0,
            ],
        }));
        rec.push(Command::Dispatch([
            output.size[0].div_ceil(8),
            output.size[1].div_ceil(8),
            1,
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Append fullscreen tone/grade/LUT output without a wgpu pass or descriptor set.
    pub fn tone(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        colour: &Texture,
        bloom: &Texture,
        lut: &Texture,
        settings: &SettingsRecord,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            output.format == self.output_format && output.size == colour.size,
            "tone output shape/format mismatch"
        );
        graph.node(
            rec,
            &[
                (Resource::Image(colour.image), Access::ShaderRead),
                (Resource::Image(bloom.image), Access::ShaderRead),
                (Resource::Image(lut.image), Access::ShaderRead),
                (Resource::Buffer(settings.buffer), Access::ShaderRead),
                (Resource::Image(output.image), Access::Color),
            ],
        )?;
        rec.push(Command::BeginPass("ara::Tone".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: output.image,
                clear: Some([0.; 4]),
            }],
            depth: None,
            size: output.size,
        });
        rec.push(Command::Pipeline(self.tone));
        rec.push(Command::Constants(RootConstants {
            words: [
                colour.sampled,
                bloom.sampled,
                0,
                0,
                0,
                settings.slot,
                self.sampler,
                lut.sampled,
            ],
        }));
        rec.push(Command::Fullscreen);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire pipelines only after their last recorded lists have been submitted.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for pipeline in [self.downsample, self.upsample, self.tone] {
            gpu.retire_pipeline(pipeline)?;
        }
        Ok(())
    }
}
