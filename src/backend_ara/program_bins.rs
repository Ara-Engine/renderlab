//! Original mixed-program tile binning, preserving atomic append semantics.
use super::{
    bindings::HeapBindings,
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
/// CPU dispatch metadata matching the packet's original RlList and region records.
pub struct BinDispatch {
    /// Number of frame-program slots, including slot zero; source mask limit32.
    pub slots: u32,
    /// Words per slot, including its count header, from RlList.capacity.
    pub capacity: u32,
    /// Workgroups covering the region tiles and additional16x16 clear-block rows.
    pub groups: [u32; 3],
}
/// Persistent original4x4-workgroup tile binning kernel.
pub struct ProgramBins {
    kernel: Kernel,
}
impl ProgramBins {
    /// Compile the unchanged standalone source and its shared atomic workgroup mask.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Ok(Self {
            kernel: Kernel::new(gpu, cache, Pass::ProgramBins)?,
        })
    }
    /// Reset GPU counters/arguments, then bin into the existing packet's buffers.
    /// Slot/capacity/groups must match its source records. Unused list words remain
    /// intact; concurrent append order is unspecified, just as on the wgpu path.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        dispatch: &BinDispatch,
    ) -> Result<()> {
        // MEASUREMENT ONLY: with RENDERLAB_ARA_BINS_TWICE set, the binning runs twice,
        // the first time under its own label. The second starts from cleared counters,
        // so the lists are the same. A first dispatch much slower than the second is
        // paying for something other than the kernel (memory the frame just made).
        if std::env::var_os("RENDERLAB_ARA_BINS_TWICE").is_some() {
            self.record_labelled(
                graph,
                rec,
                inputs,
                dispatch,
                "ara::ProgramBins(first)".into(),
            )?;
        }
        self.record_labelled(graph, rec, inputs, dispatch, "ara::ProgramBins".into())
    }
    fn record_labelled(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        dispatch: &BinDispatch,
        label: String,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::ProgramBins
                && (1..=32).contains(&dispatch.slots)
                && dispatch.capacity > 1
                && dispatch.groups[0] > 0
                && dispatch.groups[1] > 0
                && dispatch.groups[2] == 1,
            "invalid bin dispatch metadata"
        );
        let entries = inputs
            .buffer(InputKey {
                group: 0,
                binding: 5,
            })
            .context("bin entries missing")?;
        let args = inputs
            .buffer(InputKey {
                group: 0,
                binding: 6,
            })
            .context("bin arguments missing")?;
        let stride = u64::from(dispatch.capacity) * 4;
        ensure!(
            entries.len >= stride * u64::from(dispatch.slots)
                && args.len >= u64::from(dispatch.slots) * 16,
            "bin output buffer too short"
        );
        let extent = [
            dispatch.groups[0]
                .checked_mul(4)
                .context("bin dispatch width overflow")?,
            dispatch.groups[1]
                .checked_mul(4)
                .context("bin dispatch height overflow")?,
            1,
        ];
        graph.node(
            rec,
            &[
                (Resource::Buffer(entries.buffer), Access::CopyDestination),
                (Resource::Buffer(args.buffer), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::BeginPass("ara::Fill".into()));
        rec.push(Command::FillBuffer {
            destination: args.buffer,
            offset: 0,
            size: u64::from(dispatch.slots) * 16,
            value: 0,
        });
        for slot in 0..dispatch.slots {
            rec.push(Command::FillBuffer {
                destination: entries.buffer,
                offset: u64::from(slot) * stride,
                size: 4,
                value: 0,
            });
        }
        rec.push(Command::EndPass);
        self.kernel
            .record_labelled(graph, rec, inputs, extent, label)
    }
}
