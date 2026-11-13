//! A few bytes of GPU state read back without stalling a frame: copied after the pass that
//! wrote them into a small ring of mappable buffers, mapped after the frame's submission, and
//! read once the mapping completes (after a later device poll), a frame or two late.

use std::sync::{Arc, OnceLock};

pub(crate) struct ReadbackRing {
    ring: Vec<(wgpu::Buffer, Slot)>,
    bytes: u64,
    /// Sequence number of the next recorded copy, so `collect` returns them in frame order.
    next: u64,
}
enum Slot {
    Free,
    Recorded(u64),
    Mapping(u64, Arc<OnceLock<bool>>),
}

impl ReadbackRing {
    pub(crate) fn new(device: &wgpu::Device, label: &str, bytes: u64, entries: usize) -> Self {
        let ring = (0..entries)
            .map(|_| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (buffer, Slot::Free)
            })
            .collect();
        Self {
            ring,
            bytes,
            next: 0,
        }
    }
    /// Copies the ring's byte count from `source` at `offset` into a free entry; a frame with
    /// every entry still in flight records nothing.
    pub(crate) fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        offset: u64,
    ) {
        if let Some((buffer, slot)) = self.ring.iter_mut().find(|(_, s)| matches!(s, Slot::Free)) {
            encoder.copy_buffer_to_buffer(source, offset, buffer, 0, self.bytes);
            *slot = Slot::Recorded(self.next);
            self.next += 1;
        }
    }
    /// After the submission that holds the recorded copies: maps them.
    pub(crate) fn submitted(&mut self) {
        for (buffer, slot) in &mut self.ring {
            if let Slot::Recorded(sequence) = *slot {
                let done = Arc::new(OnceLock::new());
                let callback = done.clone();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = callback.set(r.is_ok());
                });
                *slot = Slot::Mapping(sequence, done);
            }
        }
    }
    /// The copies mapped since the last call, oldest first; their entries are free again.
    pub(crate) fn collect(&mut self) -> Vec<Vec<u8>> {
        let mut out = vec![];
        for (buffer, slot) in &mut self.ring {
            let Slot::Mapping(sequence, done) = slot else {
                continue;
            };
            let Some(&ok) = done.get() else { continue };
            if ok {
                let bytes = buffer.slice(..).get_mapped_range().map(|v| v.to_vec());
                buffer.unmap();
                if let Ok(bytes) = bytes {
                    out.push((*sequence, bytes));
                }
            }
            *slot = Slot::Free;
        }
        out.sort_unstable_by_key(|(sequence, _)| *sequence);
        out.into_iter().map(|(_, bytes)| bytes).collect()
    }
}
