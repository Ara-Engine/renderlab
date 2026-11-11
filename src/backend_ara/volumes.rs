//! Independent camera-volume integrators; visibility injection/history are separate.
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
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
};

/// Persistent aerial perspective and resolved-fog integration pipelines.
pub struct VolumePasses {
    pipelines: [Pipeline; 2],
    threads: [[u32; 3]; 2],
    sampler: u32,
}
impl VolumePasses {
    /// Cook unchanged WGSL entries; no placeholder sun visibility is linked.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let stages = [Pass::AtmosphereAerial, Pass::AtmosphereFogIntegrate];
        let threads = [stages[0].threads()?, stages[1].threads()?];
        let sampler = gpu
            .sampler(&ara_gpu::sampler::SamplerDesc::default())?
            .index;
        let first = cache.pipeline(gpu, stages[0], RenderState::default())?;
        let second = match cache.pipeline(gpu, stages[1], RenderState::default()) {
            Ok(p) => p,
            Err(e) => {
                return Err(e);
            }
        };
        Ok(Self {
            pipelines: [first, second],
            threads,
            sampler,
        })
    }
    /// Aerial writes slices64..96; fog writes0..64 of the same caller-owned volume.
    /// The grid must match the camera record; scatter has64 slices, output96.
    /// All images are RGBA16Float. Whole-parent graph edges preserve the other region.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        stage: Pass,
        params: &AtmosphereParameters,
        luts: &AtmosphereTargets,
        scatter: &Texture,
        output: &Texture,
    ) -> Result<()> {
        let i = match stage {
            Pass::AtmosphereAerial => 0,
            Pass::AtmosphereFogIntegrate => 1,
            _ => anyhow::bail!("not a camera-volume integrator"),
        };
        ensure!(
            output.format == Format::Rgba16Float && scatter.format == Format::Rgba16Float,
            "camera volume format mismatch"
        );
        ensure!(output.size == scatter.size, "camera volume grid mismatch");
        ensure!(
            output.shape.dimension == ara_gpu::texture::TextureDimension::D3
                && output.shape.depth == 96
                && scatter.shape.dimension == ara_gpu::texture::TextureDimension::D3
                && scatter.shape.depth == 64,
            "camera volume must have96 output/64 scatter slices"
        );
        ensure!(
            params.grid == Some(output.size),
            "camera parameter grid mismatch"
        );
        let slot = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("camera volume lacks storage view"))?;
        let mut uses = vec![
            (Resource::Buffer(params.record.buffer), Access::ShaderRead),
            (
                Resource::Image(luts.transmittance.image),
                Access::ShaderRead,
            ),
            (Resource::Image(output.image), Access::Storage),
        ];
        if i == 0 {
            uses.push((Resource::Image(luts.multiscatter.image), Access::ShaderRead));
        } else {
            uses.push((Resource::Image(scatter.image), Access::ShaderRead));
        }
        graph.node(rec, &uses)?;
        rec.push(Command::BeginPass(format!("ara::{stage:?}")));
        rec.push(Command::Pipeline(self.pipelines[i]));
        rec.push(Command::Constants(RootConstants {
            words: [
                params.record.slot,
                luts.transmittance.sampled,
                luts.multiscatter.sampled,
                luts.sky.sampled,
                slot,
                self.sampler,
                0,
                scatter.sampled,
            ],
        }));
        rec.push(Command::Dispatch([
            output.size[0].div_ceil(self.threads[i][0]),
            output.size[1].div_ceil(self.threads[i][1]),
            1,
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
}
