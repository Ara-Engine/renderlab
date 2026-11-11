//! Raster sky backdrop, using the same atmosphere record and prepared LUT/volume.
use super::{
    atmosphere::{AtmosphereParameters, AtmosphereTargets},
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
};
/// Persistent fullscreen sky pipeline; draw before opaque meshes on the same target.
pub struct SkyPass {
    pipeline: Pipeline,
    sampler: u32,
    format: Format,
}
impl SkyPass {
    /// Compile unchanged API and raster entries, without depth testing or blending.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, format: Format) -> Result<Self> {
        let sampler = gpu
            .sampler(&ara_gpu::sampler::SamplerDesc::default())?
            .index;
        let state = RenderState {
            color_formats: vec![format.vk().as_raw()],
            ..Default::default()
        };
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::AtmosphereBackdrop, state)?,
            sampler,
            format,
        })
    }
    /// Draw the backdrop. The volume must be initialized even when its flags are off.
    /// Output size must match the viewport used to construct `params`.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        params: &AtmosphereParameters,
        luts: &AtmosphereTargets,
        volume: &Texture,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            params.viewport == Some(output.size),
            "sky viewport mismatch"
        );
        ensure!(
            output.format == self.format,
            "sky attachment format mismatch"
        );
        ensure!(
            volume.shape.dimension == ara_gpu::texture::TextureDimension::D3
                && volume.shape.depth == 96,
            "sky requires a packed96-slice volume"
        );
        graph.node(
            rec,
            &[
                (Resource::Buffer(params.record.buffer), Access::ShaderRead),
                (
                    Resource::Image(luts.transmittance.image),
                    Access::ShaderRead,
                ),
                (Resource::Image(luts.sky.image), Access::ShaderRead),
                (Resource::Image(volume.image), Access::ShaderRead),
                (Resource::Image(output.image), Access::Color),
            ],
        )?;
        rec.push(Command::BeginPass("ara::SkyBackdrop".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: output.image,
                clear: Some([0.; 4]),
            }],
            depth: None,
            size: output.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(RootConstants {
            words: [
                params.record.slot,
                luts.transmittance.sampled,
                0,
                luts.sky.sampled,
                0,
                self.sampler,
                0,
                volume.sampled,
            ],
        }));
        rec.push(Command::Fullscreen);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
}
