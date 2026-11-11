//! Original ray-free near-edge arithmetic and retry-queue construction.
use super::{
    bindings::HeapBindings,
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    kernel::Kernel,
    shader::{InputKey, Pass},
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Resource,
    commands::{Command, Recorder},
};
/// Persistent geometric check, including the per-invocation retry-header reset.
pub struct PrimaryCheck {
    kernel: Kernel,
}
impl PrimaryCheck {
    /// Compile the original production arithmetic-only composition.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Ok(Self {
            kernel: Kernel::new(gpu, cache, Pass::PrimaryCheck)?,
        })
    }
    /// Direct workgroups use the original 256-column dispatch grid and 32 lanes.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        groups: [u32; 3],
    ) -> Result<()> {
        ensure!(
            (1..=256).contains(&groups[0])
                && groups[1] > 0
                && groups[2] == 1
                && (groups[1] == 1 || groups[0] == 256),
            "invalid primary-check dispatch grid"
        );
        self.reset(graph, rec, inputs)?;
        self.kernel
            .record(graph, rec, inputs, [groups[0] * 32, groups[1], 1])
    }
    /// Consume the original separate PrimaryRepairArgs record, without reading its
    /// GPU count on the CPU. It must follow the same 256-column grid convention.
    pub fn record_indirect(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        arguments: &StorageBuffer,
    ) -> Result<()> {
        ensure!(arguments.len >= 12, "primary-check arguments are too short");
        self.reset(graph, rec, inputs)?;
        self.kernel
            .record_indirect(graph, rec, inputs, arguments, 0)
    }
    fn reset(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::PrimaryCheck,
            "wrong primary-check packet"
        );
        let retries = inputs
            .buffer(InputKey {
                group: 0,
                binding: 28,
            })
            .context("primary retry queue missing")?;
        ensure!(retries.len >= 16, "primary retry header is too short");
        graph.node(
            rec,
            &[(Resource::Buffer(retries.buffer), Access::CopyDestination)],
        )?;
        rec.push(Command::BeginPass("ara::Fill".into()));
        rec.push(Command::FillBuffer {
            destination: retries.buffer,
            offset: 0,
            size: 16,
            value: 0,
        });
        rec.push(Command::EndPass);
        Ok(())
    }
}
