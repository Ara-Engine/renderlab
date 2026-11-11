//! Procedural environment leaf kernels; the sky input can come from either replay or AT1.
use super::{
    atmosphere::AtmosphereParameters, cache::ShaderCache, graph::Graph, image::Texture,
    shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
};
/// Persistent GGX and cosine-weighted prefilter pipelines.
pub struct EnvironmentPasses {
    pipelines: [Pipeline; 2],
    threads: [[u32; 3]; 2],
    sampler: u32,
}
impl EnvironmentPasses {
    /// Compile the unchanged environment portion of the atmosphere source.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let stages = [Pass::EnvironmentSpecular, Pass::EnvironmentDiffuse];
        let threads = stages
            .map(Pass::threads)
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("two environment entries");
        let first = cache.pipeline(gpu, stages[0], RenderState::default())?;
        let second = match cache.pipeline(gpu, stages[1], RenderState::default()) {
            Ok(p) => p,
            Err(e) => {
                return Err(e);
            }
        };
        let sampler = gpu
            .sampler(&ara_gpu::sampler::SamplerDesc::default())?
            .index;
        Ok(Self {
            pipelines: [first, second],
            threads,
            sampler,
        })
    }
    /// Write all six faces of one storage mip. Call specular for each original
    /// size 128,64,32,16,8; diffuse accepts the selected cube resolution.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        stage: Pass,
        params: &AtmosphereParameters,
        sky: &Texture,
        output: &Texture,
    ) -> Result<()> {
        let i = match stage {
            Pass::EnvironmentSpecular => 0,
            Pass::EnvironmentDiffuse => 1,
            _ => anyhow::bail!("not an environment pass"),
        };
        ensure!(
            output.format == Format::Rgba32Float
                && output.size[0] == output.size[1]
                && output.shape.dimension == ara_gpu::texture::TextureDimension::D2
                && output.shape.layers == 6
                && output.shape.mip_levels == 1
                && output.shape.samples == ara_gpu::texture::SampleCount::One,
            "environment output must be a square RGBA32Float array view"
        );
        ensure!(
            i != 0 || [128, 64, 32, 16, 8].contains(&output.size[0]),
            "specular mip extent would change the original roughness steps"
        );
        let storage = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("environment output needs a narrow storage view"))?;
        graph.node(
            rec,
            &[
                (Resource::Buffer(params.record.buffer), Access::ShaderRead),
                (Resource::Image(sky.image), Access::ShaderRead),
                (Resource::Image(output.image), Access::Storage),
            ],
        )?;
        rec.push(Command::BeginPass(format!("ara::{stage:?}")));
        rec.push(Command::Pipeline(self.pipelines[i]));
        rec.push(Command::Constants(RootConstants {
            words: [
                params.record.slot,
                0,
                0,
                sky.sampled,
                storage,
                self.sampler,
                0,
                0,
            ],
        }));
        rec.push(Command::Dispatch([
            output.size[0].div_ceil(self.threads[i][0]),
            output.size[1].div_ceil(self.threads[i][1]),
            6u32.div_ceil(self.threads[i][2]),
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
}
