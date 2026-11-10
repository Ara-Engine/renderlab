//! Reflected compute entries share one heap packet and graph declaration path.
use super::{
    bindings::HeapBindings, buffer::StorageBuffer, cache::ShaderCache, graph::Graph, shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Command, Recorder},
    cooked::RenderState,
};
/// Persistent compute pipeline for a selected indirect-packet program.
pub struct Kernel {
    pipeline: Pipeline,
    pass: Pass,
    threads: [u32; 3],
}
impl Kernel {
    /// Compile the original selected compute entry; cache its source workgroup size.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, pass: Pass) -> Result<Self> {
        ensure!(
            pass.indirect() && !pass.raster(),
            "entry needs a reflected compute packet"
        );
        let threads = pass.threads()?;
        Ok(Self {
            pipeline: cache.pipeline(gpu, pass, RenderState::default())?,
            pass,
            threads,
        })
    }
    /// Dispatch enough groups for a nonzero logical extent. The original shader
    /// retains responsibility for its padded invocation bounds, as on wgpu.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        extent: [u32; 3],
    ) -> Result<()> {
        ensure!(
            inputs.pass() == self.pass && extent.iter().all(|n| *n > 0),
            "invalid kernel packet or extent"
        );
        graph.node_uses(rec, &inputs.uses())?;
        self.dispatch(
            rec,
            inputs,
            Command::Dispatch(std::array::from_fn(|i| extent[i].div_ceil(self.threads[i]))),
        );
        Ok(())
    }
    /// Consume GPU-produced dispatch arguments without a CPU readback. The source
    /// buffer must declare native indirect usage and cannot alias writable inputs.
    pub fn record_indirect(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        arguments: &StorageBuffer,
        offset: u64,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == self.pass
                && offset.is_multiple_of(4)
                && offset
                    .checked_add(12)
                    .is_some_and(|end| end <= arguments.len),
            "invalid indirect kernel packet/range"
        );
        let mut uses = inputs.uses();
        uses.push((Resource::Buffer(arguments.buffer), Access::Indirect).into());
        graph.node_uses(rec, &uses)?;
        self.dispatch(
            rec,
            inputs,
            Command::DispatchIndirect {
                arguments: arguments.buffer,
                offset,
            },
        );
        Ok(())
    }
    fn dispatch(&self, rec: &mut Recorder, inputs: &HeapBindings<'_>, command: Command) {
        rec.push(Command::BeginPass(format!("ara::{:?}", self.pass)));
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(inputs.roots()));
        rec.push(command);
        rec.push(Command::EndPass);
    }
    /// Retire after submitting every referencing list.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        gpu.retire_pipeline(self.pipeline)?;
        Ok(())
    }
}
