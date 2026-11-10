//! P-STILL (docs/p-still.md): whether the GI cache has drained, from its own work counters
//! read back a few frames late without stalling. A still view may then idle once its last
//! cache frames did no work, instead of waiting out the gather cap's worst-case drain bound
//! (gi_cache.rs `idle_after`: IDLE_AFTER_BOUNCE + 64 slot groups per gather generation).
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Counter words (gi_cache.rs `IndirectCounters`) that are all zero on a cache frame with
/// nothing to do: misses, traces, integrates, created, invalidated, lost, shades, gathers,
/// bounce_queued, and the gather and claim budgets' deferrals.
const WORK_WORDS: [usize; 11] = [0, 1, 2, 7, 8, 10, 32, 33, 38, 42, 43];
const RING: usize = 4;

struct Slot {
    buffer: wgpu::Buffer,
    /// The reset generation the copy was recorded in, and its map completion once requested.
    generation: u64,
    mapped: Option<Arc<AtomicBool>>,
}

pub(super) struct Drain {
    slots: Vec<Slot>,
    free: Vec<usize>,
    /// Recorded slots, oldest first.
    order: VecDeque<usize>,
    generation: u64,
    /// A cache frame ran with no free slot, so its work is unknown.
    missed: bool,
    /// Consecutive read-back cache frames of this generation that did no work.
    pub quiet: u32,
}

impl Drain {
    pub fn new(device: &wgpu::Device, bytes: u64) -> Self {
        let slots = (0..RING)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("indirect cache drain counters"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                generation: 0,
                mapped: None,
            })
            .collect();
        Self {
            slots,
            free: (0..RING).collect(),
            order: VecDeque::new(),
            generation: 0,
            missed: false,
            quiet: 0,
        }
    }

    /// At the end of a cache frame's passes: copy its counters, if a slot is free.
    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder, work: &wgpu::Buffer) {
        let Some(i) = self.free.pop() else {
            self.missed = true;
            return;
        };
        let slot = &mut self.slots[i];
        encoder.copy_buffer_to_buffer(work, 0, &slot.buffer, 0, slot.buffer.size());
        slot.generation = self.generation;
        self.order.push_back(i);
    }

    /// At a frame's start, after the previous frame's submit: request maps of recorded copies
    /// and fold finished ones, oldest first, into `quiet`.
    pub fn poll(&mut self) {
        for &i in &self.order {
            let slot = &mut self.slots[i];
            if slot.mapped.is_none() {
                let done = Arc::new(AtomicBool::new(false));
                let flag = done.clone();
                slot.buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| flag.store(r.is_ok(), Ordering::Release));
                slot.mapped = Some(done);
            }
        }
        if std::mem::take(&mut self.missed) {
            self.quiet = 0;
        }
        while let Some(&i) = self.order.front() {
            let slot = &mut self.slots[i];
            if !slot.mapped.as_ref().is_some_and(|d| d.load(Ordering::Acquire)) {
                break;
            }
            let idle = slot.buffer.slice(..).get_mapped_range().is_ok_and(|bytes| {
                WORK_WORDS.iter().all(|&w| bytes[w * 4..w * 4 + 4] == [0; 4])
            });
            slot.buffer.unmap();
            slot.mapped = None;
            if slot.generation == self.generation {
                self.quiet = if idle { self.quiet + 1 } else { 0 };
            }
            self.order.pop_front();
            self.free.push(i);
        }
    }

    /// The table was emptied: frames recorded before it no longer describe the cache.
    pub fn reset(&mut self) {
        self.generation += 1;
        self.quiet = 0;
    }
}
