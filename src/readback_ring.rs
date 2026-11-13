//! A few bytes of GPU state read back without stalling a frame: copied after the pass that
//! wrote them into a small ring of mappable buffers, mapped after the frame's submission, and
//! read once the mapping completes (after a later device poll), a frame or two late. Each copy
//! carries a CPU-side tag `T` from its frame (what the reader needs to age the value).

use std::sync::{Arc, OnceLock};

pub(crate) struct ReadbackRing<T> {
    ring: Vec<(wgpu::Buffer, Slot<T>)>,
    bytes: u64,
    /// Sequence number of the next recorded copy, so `collect` returns them in frame order.
    next: u64,
}
enum Slot<T> {
    Free,
    Recorded(u64, T),
    Mapping(u64, T, Arc<OnceLock<bool>>),
}

impl<T: Copy> ReadbackRing<T> {
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
    /// Copies the ring's byte count from `source` at `offset` into a free entry, tagged; a
    /// frame with every entry still in flight records nothing.
    pub(crate) fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        offset: u64,
        tag: T,
    ) {
        if let Some((buffer, slot)) = self.ring.iter_mut().find(|(_, s)| matches!(s, Slot::Free)) {
            encoder.copy_buffer_to_buffer(source, offset, buffer, 0, self.bytes);
            *slot = Slot::Recorded(self.next, tag);
            self.next += 1;
        }
    }
    /// After the submission that holds the recorded copies: maps them.
    pub(crate) fn submitted(&mut self) {
        for (buffer, slot) in &mut self.ring {
            if let Slot::Recorded(sequence, tag) = *slot {
                let done = Arc::new(OnceLock::new());
                let callback = done.clone();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = callback.set(r.is_ok());
                });
                *slot = Slot::Mapping(sequence, tag, done);
            }
        }
    }
    /// The copies mapped since the last call with their tags, oldest first; their entries are
    /// free again.
    pub(crate) fn collect(&mut self) -> Vec<(T, Vec<u8>)> {
        let mut out = vec![];
        for (buffer, slot) in &mut self.ring {
            let Slot::Mapping(sequence, tag, done) = slot else {
                continue;
            };
            let Some(&ok) = done.get() else { continue };
            if ok {
                let bytes = buffer.slice(..).get_mapped_range().map(|v| v.to_vec());
                buffer.unmap();
                if let Ok(bytes) = bytes {
                    out.push((*sequence, *tag, bytes));
                }
            }
            *slot = Slot::Free;
        }
        out.sort_unstable_by_key(|(sequence, ..)| *sequence);
        out.into_iter()
            .map(|(_, tag, bytes)| (tag, bytes))
            .collect()
    }
}
