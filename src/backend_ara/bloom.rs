//! Persistent bloom pyramid matching the six-level wgpu post topology.
use super::{
    graph::Graph,
    image::Texture,
    post::{FilterInputs, PostPasses, SettingsRecord},
    shader::Pass,
};
use anyhow::Result;
use ara_gpu::{Device, GpuDevice, Resource, commands::Recorder, descriptors::Format};
/// Reusable down/up resources; record after motion/DOF and before tone.
pub struct Bloom {
    down: Vec<Texture>,
    up: Vec<Texture>,
}
impl Bloom {
    /// Allocate the same ceil-halved, at-most-six levels as Renderlab's post chain.
    pub fn new(gpu: &mut GpuDevice, size: [u32; 2]) -> Result<Self> {
        let mut result = Self {
            down: Vec::new(),
            up: Vec::new(),
        };
        let mut size = size;
        for _ in 0..6 {
            size = size.map(|n| n.div_ceil(2));
            result
                .down
                .push(Texture::new(gpu, size, Format::Rgba16Float, true, false)?);
            result
                .up
                .push(Texture::new(gpu, size, Format::Rgba16Float, true, false)?);
            if size == [1, 1] {
                break;
            }
        }
        Ok(result)
    }
    /// The final bloom texture, valid after recording this chain.
    pub fn output(&self) -> &Texture {
        &self.up[0]
    }
    /// Record every dependent kernel as a graph node, including the coarsest self-tent.
    pub fn record(
        &self,
        passes: &PostPasses,
        graph: &mut Graph,
        rec: &mut Recorder,
        colour: &Texture,
        settings: &SettingsRecord,
    ) -> Result<()> {
        for (i, out) in self.down.iter().enumerate() {
            let input = if i == 0 { colour } else { &self.down[i - 1] };
            passes.filter(
                graph,
                rec,
                Pass::Downsample,
                &FilterInputs {
                    colour: input,
                    second: colour,
                    auxiliary: colour,
                    velocity_tiles: colour,
                    settings,
                },
                out,
            )?;
        }
        for i in (0..self.up.len()).rev() {
            let second = if i + 1 == self.up.len() {
                &self.down[i]
            } else {
                &self.up[i + 1]
            };
            passes.filter(
                graph,
                rec,
                Pass::Upsample,
                &FilterInputs {
                    colour: &self.down[i],
                    second,
                    auxiliary: colour,
                    velocity_tiles: colour,
                    settings,
                },
                &self.up[i],
            )?;
        }
        Ok(())
    }
    /// Retire all levels after their recorded lists have been submitted.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for t in self.down.into_iter().chain(self.up) {
            gpu.retire(Resource::Image(t.image))?;
        }
        Ok(())
    }
}
