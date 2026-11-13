//! Frames-in-flight readback: a ring of host-readable buffers that a per-frame GPU range
//! (counters, a room's requests) is copied into each frame and read once its submission has
//! completed, never waiting. A player keeps several frames in flight, so reading this frame's
//! copy right after submitting it fails (`ARA_GPU_CONTRACT: Readback still in flight`); with a
//! ring the frame records into a free slot and reads the newest completed one, a frame or two
//! late. When every slot still waits on the GPU the frame skips its copy.
use super::graph::Graph;
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Buffer, Device, GpuDevice, Resource, TimelinePoint,
    commands::{Command, Recorder},
};

/// One slot: its buffer, and what it holds.
struct Slot {
    buffer: Buffer,
    state: State,
}
#[derive(Clone, Copy, PartialEq)]
enum State {
    /// Readable or never written: the next `record` may use it.
    Free,
    /// Recorded into this frame's list, not yet submitted.
    Recorded { frame: u64 },
    /// Submitted; its bytes are readable once `point` completes.
    Pending { frame: u64, point: TimelinePoint },
}

/// A ring of `depth` host-readable buffers of `bytes` each.
pub struct ReadbackRing {
    slots: Vec<Slot>,
    bytes: u64,
}

impl ReadbackRing {
    /// `depth` slots (at least two: one in flight while the other is read) of `bytes` each.
    pub fn new(gpu: &mut GpuDevice, bytes: u64, depth: usize) -> Result<Self> {
        ensure!(depth >= 2 && bytes > 0, "a readback ring has two or more non-empty slots");
        let slots = (0..depth)
            .map(|_| {
                Ok(Slot {
                    buffer: gpu.create_readback_buffer(bytes)?,
                    state: State::Free,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { slots, bytes })
    }

    /// Records a copy of `source[offset..offset + bytes]` into a free slot for `frame`.
    /// Returns false, recording nothing, when every slot still waits on the GPU.
    pub fn record(
        &mut self,
        graph: &mut Graph,
        rec: &mut Recorder,
        source: Buffer,
        offset: u64,
        frame: u64,
    ) -> Result<bool> {
        let Some(slot) = self.slots.iter_mut().find(|s| s.state == State::Free) else {
            return Ok(false);
        };
        // The slot was last read by the host (or never used).
        graph.import(Resource::Buffer(slot.buffer), Access::HostRead);
        graph.node(
            rec,
            &[
                (Resource::Buffer(source), Access::CopySource),
                (Resource::Buffer(slot.buffer), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::BeginPass("ara::ReadbackRing".into()));
        rec.push(Command::CopyBufferRange {
            source,
            source_offset: offset,
            destination: slot.buffer,
            destination_offset: 0,
            size: self.bytes,
        });
        rec.push(Command::EndPass);
        graph.node(rec, &[(Resource::Buffer(slot.buffer), Access::HostRead)])?;
        slot.state = State::Recorded { frame };
        Ok(true)
    }

    /// The list holding this frame's `record` was submitted as `point`.
    pub fn submitted(&mut self, point: TimelinePoint) {
        for slot in &mut self.slots {
            if let State::Recorded { frame } = slot.state {
                slot.state = State::Pending { frame, point };
            }
        }
    }

    /// The newest completed copy, with the frame it was recorded for, and every completed
    /// slot freed; `None` while nothing has completed. Never waits.
    pub fn take(&mut self, gpu: &GpuDevice) -> Result<Option<(u64, Vec<u8>)>> {
        let completed = gpu.completed()?;
        let done = |point: TimelinePoint| completed[point.queue.index()] >= point.value;
        let newest = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.state {
                State::Pending { frame, point } if done(point) => Some((frame, i, point)),
                _ => None,
            })
            .max_by_key(|(frame, i, _)| (*frame, *i));
        let Some((frame, at, point)) = newest else {
            return Ok(None);
        };
        // Read against the copy's own submission: `read` would also wait on the other queue.
        let bytes = gpu.read_completed(self.slots[at].buffer, point)?;
        for slot in &mut self.slots {
            if let State::Pending { point, .. } = slot.state
                && done(point)
            {
                slot.state = State::Free;
            }
        }
        Ok(Some((frame, bytes)))
    }

    /// Releases the slots once their submissions have completed.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for slot in self.slots {
            gpu.retire(Resource::Buffer(slot.buffer))?;
        }
        Ok(())
    }
}
