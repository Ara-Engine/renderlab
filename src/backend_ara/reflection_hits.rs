//! The reflection hit pass on the port (src/reflection_hits.rs,
//! docs/design-reflection-pass.md): H's records binned by their hit's program. The
//! per-program hit dispatches follow once U-RTVIS's `rl_trace` writes H here.
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

/// One binning dispatch: the frame's program slots, each bin's words (its length, then up to
/// one index per record), and the workgroups covering H's records (64 lanes each).
pub struct HitBinDispatch {
    pub slots: u32,
    pub stride: u32,
    pub groups: [u32; 3],
}

/// The binning kernel (shaders/reflection_hit_bins.wgsl), compiled once.
pub struct HitBins {
    kernel: Kernel,
}
impl HitBins {
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Ok(Self {
            kernel: Kernel::new(gpu, cache, Pass::ReflectionHitBins)?,
        })
    }
    /// Clears each bin's length and every slot's dispatch arguments, then bins. The packet
    /// binds H (0), the placements (1), the bins uniform (2), the order (3) and the
    /// arguments (4).
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        dispatch: &HitBinDispatch,
    ) -> Result<()> {
        let buffer = |binding| {
            inputs
                .buffer(InputKey { group: 0, binding })
                .context("hit bin output missing")
        };
        let (order, args) = (buffer(3)?, buffer(4)?);
        let stride = u64::from(dispatch.stride) * 4;
        ensure!(
            (1..=32).contains(&dispatch.slots)
                && dispatch.stride > 1
                && order.len >= stride * u64::from(dispatch.slots)
                && args.len >= u64::from(dispatch.slots) * 16
                && dispatch.groups[0] > 0
                && dispatch.groups[2] == 1,
            "invalid hit bin dispatch"
        );
        graph.node(
            rec,
            &[
                (Resource::Buffer(order.buffer), Access::CopyDestination),
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
                destination: order.buffer,
                offset: u64::from(slot) * stride,
                size: 4,
                value: 0,
            });
        }
        rec.push(Command::EndPass);
        let extent = [
            dispatch.groups[0] * crate::renderer::reflection_hits::LANES,
            dispatch.groups[1],
            1,
        ];
        self.kernel.record(graph, rec, inputs, extent)
    }
}
