//! The frames-in-flight readback ring (`backend_ara::readback_ring`): each frame copies a GPU
//! range into a free slot and reads the newest completed copy without waiting.
use super::*;
use ara_gpu::{Access, Queue, Resource};
use renderlab::backend_ara::{buffer::StorageBuffer, graph::Graph, readback_ring::ReadbackRing};

/// Eight frames submitted back to back with no wait: every copy read is the frame it was
/// recorded for, frames come back in order, nothing errors with a readback in flight, and
/// after the device idles the newest recorded frame is the one read.
#[test]
fn readback_ring_reads_completed_frames_without_waiting() -> Result<()> {
    let path = root("readback-ring");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let mut ring = ReadbackRing::new(&mut gpu, 16, 3)?;
    let mut seen = vec![];
    let mut last_recorded = None;
    for frame in 0..8u64 {
        let source = StorageBuffer::new(&mut gpu, &[frame as u8; 16])?;
        let mut graph = Graph::default();
        let mut rec = gpu.begin(Queue::Graphics);
        graph.import(Resource::Buffer(source.buffer), Access::HostWrite);
        let recorded = ring.record(&mut graph, &mut rec, source.buffer, 0, frame)?;
        let point = gpu.submit(rec.finish(), &[])?;
        if recorded {
            ring.submitted(point);
            last_recorded = Some(frame);
        }
        source.retire(&mut gpu)?;
        if let Some((f, bytes)) = ring.take(&gpu)? {
            ensure!(bytes == vec![f as u8; 16], "frame {f} read another frame's bytes");
            seen.push(f);
        }
    }
    gpu.wait_idle()?;
    if let Some((f, bytes)) = ring.take(&gpu)? {
        ensure!(bytes == vec![f as u8; 16], "frame {f} read another frame's bytes");
        seen.push(f);
    }
    ensure!(
        seen.windows(2).all(|w| w[0] < w[1]),
        "frames read out of order: {seen:?}"
    );
    ensure!(
        seen.last().copied() == last_recorded,
        "after idling the newest recorded frame ({last_recorded:?}) was not read: {seen:?}"
    );
    ring.retire(&mut gpu)
}
