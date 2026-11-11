//! The reusable post-pass encoder. No device creation, upload or wait inside a pass.
use super::{
    buffer::StorageBuffer, cache::ShaderCache, graph::Graph, image::Texture, shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Buffer, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
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
        let record = StorageBuffer::new(gpu, bytemuck::cast_slice(words))?;
        Ok(Self {
            buffer: record.buffer,
            slot: record.slot,
        })
    }
    /// Schedule settings after earlier reads, without a host overwrite or CPU wait.
    pub fn update(&self, graph: &mut Graph, rec: &mut Recorder, words: &[f32; 20]) -> Result<()> {
        ensure!(
            words.iter().all(|v| v.is_finite()),
            "post settings must be finite"
        );
        graph.node(
            rec,
            &[(Resource::Buffer(self.buffer), Access::CopyDestination)],
        )?;
        rec.push(Command::UpdateBuffer {
            destination: self.buffer,
            offset: 0,
            bytes: bytemuck::cast_slice(words).to_vec(),
        });
        Ok(())
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
/// Sampled resources and settings for the fullscreen tone stage.
pub struct ToneInputs<'a> {
    /// Linear HDR image.
    pub colour: &'a Texture,
    /// Final bloom level.
    pub bloom: &'a Texture,
    /// Three-dimensional colour LUT.
    pub lut: &'a Texture,
    /// Shared 80-byte settings record.
    pub settings: &'a SettingsRecord,
}
/// Persistent pipelines and one immutable linear clamp sampler.
pub struct PostPasses {
    downsample: Pipeline,
    upsample: Pipeline,
    tile_max: Pipeline,
    motion_blur: Pipeline,
    depth_of_field: Pipeline,
    tone: Pipeline,
    display: Pipeline,
    groups: [[u32; 3]; 5],
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
        let groups = [
            Pass::Downsample,
            Pass::Upsample,
            Pass::TileMax,
            Pass::MotionBlur,
            Pass::DepthOfField,
        ]
        .map(Pass::threads)
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .try_into()
        .expect("five compute entries");
        let mut created = Vec::new();
        for pass in [
            Pass::Downsample,
            Pass::Upsample,
            Pass::TileMax,
            Pass::MotionBlur,
            Pass::DepthOfField,
            Pass::Tone,
            Pass::Display,
        ] {
            let state = RenderState {
                color_formats: match pass {
                    Pass::Tone => vec![output_format.vk().as_raw()],
                    Pass::Display => vec![Format::Rgba8.vk().as_raw()],
                    _ => vec![],
                },
                ..Default::default()
            };
            match cache.pipeline(gpu, pass, state) {
                Ok(pipeline) => created.push(pipeline),
                Err(error) => {
                    return Err(error);
                }
            }
        }
        let sampler = match gpu.sampler(&ara_gpu::sampler::SamplerDesc::default()) {
            Ok(slot) => slot.index,
            Err(error) => {
                return Err(error.into());
            }
        };
        let [
            downsample,
            upsample,
            tile_max,
            motion_blur,
            depth_of_field,
            tone,
            display,
        ] = created
            .try_into()
            .expect("one pipeline per fixed post pass");
        Ok(Self {
            downsample,
            upsample,
            tile_max,
            motion_blur,
            depth_of_field,
            tone,
            display,
            sampler,
            output_format,
            groups,
        })
    }

    /// Append one post compute kernel and its graph-generated dependencies.
    pub fn filter(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        pass: Pass,
        inputs: &FilterInputs<'_>,
        output: &Texture,
    ) -> Result<()> {
        let (pipeline, group) = match pass {
            Pass::Downsample => (self.downsample, self.groups[0]),
            Pass::Upsample => (self.upsample, self.groups[1]),
            Pass::TileMax => (self.tile_max, self.groups[2]),
            Pass::MotionBlur => (self.motion_blur, self.groups[3]),
            Pass::DepthOfField => (self.depth_of_field, self.groups[4]),
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
            output.size[0].div_ceil(group[0]),
            output.size[1].div_ceil(group[1]),
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
        inputs: &ToneInputs<'_>,
        output: &Texture,
    ) -> Result<()> {
        let ToneInputs {
            colour,
            bloom,
            lut,
            settings,
        } = *inputs;
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
    /// Encode the display transfer into RGBA8 UNORM; PNG readback and the Mac
    /// presenter's UNORM surface receive already encoded values without CPU grading.
    pub fn display(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        input: &Texture,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            output.format == Format::Rgba8 && output.size == input.size,
            "display output must be same-size RGBA8 UNORM"
        );
        graph.node(
            rec,
            &[
                (Resource::Image(input.image), Access::ShaderRead),
                (Resource::Image(output.image), Access::Color),
            ],
        )?;
        rec.push(Command::BeginPass("ara::DisplayTransfer".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: output.image,
                clear: Some([0.; 4]),
            }],
            depth: None,
            size: output.size,
        });
        rec.push(Command::Pipeline(self.display));
        rec.push(Command::Constants(RootConstants {
            words: [input.sampled, 0, 0, 0, 0, 0, 0, 0],
        }));
        rec.push(Command::Fullscreen);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Nothing of its own to release: its pipelines stay with the shader cache
    /// (`ShaderCache::retire`).
    pub fn retire(self, _: &mut GpuDevice) -> Result<()> {
        Ok(())
    }
}
