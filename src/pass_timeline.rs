//! Named GPU stage timings for measurement (P-SHOW2, docs/p-show2.md).
//!
//! With `RENDERLAB_PASS_LOG=<file>` set when the renderer is created (and timestamp
//! queries available), every frame writes a timestamp at the start of each named stage
//! and one at its end, and the renderer appends one JSON line per completed frame:
//! `{"frame":n,"span":ms,"passes":{"skinning":ms,...}}`. A stage lasts from its mark to
//! the next mark; marks are empty compute passes, so they need only TIMESTAMP_QUERY.
//! Stages that the GPU overlaps share their boundary, so the stage times always sum to the
//! span. Embedders need no change: harnesses read the file (tools/showcase/showcase.py).
//! Without the variable nothing is created and every call returns at once.
//! `RENDERLAB_PASS_LOG_DETAIL` also keeps a stage's parts (names with a dot, such as
//! `sun_shadow.raster`); without it those marks are skipped, so the part's time stays in
//! the stage before it. The reflection pass's stages (`ALWAYS`) are kept either way: every
//! reflection table reads them from the default log.
//! The readback pool grows to the in-flight high-water mark instead of dropping samples
//! or waiting for the GPU. Completion can be out of order: consumers join by `frame`,
//! never by line position. Polling is still needed to collect the final readbacks.
use std::io::Write;
use std::sync::{Arc, OnceLock};

/// Marks per frame, including the final one. Each frame's stamps resolve at a multiple of
/// QUERY_RESOLVE_BUFFER_ALIGNMENT (256 bytes): 64 marks are 512.
const MARKS: u32 = 64;
const _: () = assert!((MARKS as u64 * 8) % wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT == 0);
/// Dotted stages the default log keeps: the compacted reflection pass's trace, hit shading and
/// composite after `shading` (U-RTVIS; zero-length while the pass is off).
const ALWAYS: [&str; 3] = [
    "reflection.trace",
    "reflection.hits",
    "reflection.composite",
];
/// Initial readback pool; later frames grow it only when every entry is in flight.
const INITIAL_FRAMES: usize = 4;

struct Frame {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    names: Vec<&'static str>,
    mapped: Option<Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>>,
    frame: u64,
}

impl Frame {
    fn new(device: &wgpu::Device) -> Self {
        let bytes = u64::from(MARKS) * 8;
        Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timeline"),
                ty: wgpu::QueryType::Timestamp,
                count: MARKS,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timeline resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timeline readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            names: vec![],
            mapped: None,
            frame: 0,
        }
    }
}

fn available_frame(
    len: usize,
    next: usize,
    mut available: impl FnMut(usize) -> bool,
) -> Option<usize> {
    (0..len)
        .map(|offset| (next + offset) % len)
        .find(|&i| available(i))
}

pub(crate) struct PassTimeline {
    frames: Vec<Frame>,
    /// The pool entry receiving this frame's marks; None outside frame encoding.
    current: Option<usize>,
    next: usize,
    frame: u64,
    period: f64,
    /// Whether dotted part names are marked (`RENDERLAB_PASS_LOG_DETAIL`).
    detail: bool,
    log: std::io::BufWriter<std::fs::File>,
}

impl PassTimeline {
    /// The timeline the environment asks for, if any.
    pub(crate) fn from_env(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let path = std::env::var_os("RENDERLAB_PASS_LOG")?;
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let log = std::fs::File::create(path).ok()?;
        let frames = (0..INITIAL_FRAMES).map(|_| Frame::new(device)).collect();
        Some(Self {
            frames,
            current: None,
            next: 0,
            frame: 0,
            period: f64::from(queue.get_timestamp_period()),
            detail: std::env::var_os("RENDERLAB_PASS_LOG_DETAIL").is_some(),
            log: std::io::BufWriter::new(log),
        })
    }

    /// Reuses a free entry or grows the pool, without skipping a frame or waiting.
    pub(crate) fn begin(&mut self, device: &wgpu::Device) {
        self.frame += 1;
        let i = available_frame(self.frames.len(), self.next, |i| {
            self.frames[i].mapped.is_none()
        })
        .unwrap_or_else(|| {
            let i = self.frames.len();
            self.frames.push(Frame::new(device));
            i
        });
        let entry = &mut self.frames[i];
        entry.names.clear();
        entry.frame = self.frame;
        self.current = Some(i);
        self.next = (i + 1) % self.frames.len();
    }

    /// Whether dotted part names are marked (`RENDERLAB_PASS_LOG_DETAIL`).
    pub(crate) fn detail(&self) -> bool {
        self.detail
    }

    /// Starts stage `name` at this point of `encoder`'s work.
    pub(crate) fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, name: &'static str) {
        let Some(i) = self.current else { return };
        if name.contains('.') && !self.detail && !ALWAYS.contains(&name) {
            return;
        }
        let entry = &mut self.frames[i];
        if entry.names.len() + 1 >= MARKS as usize {
            return;
        }
        let index = entry.names.len() as u32;
        entry.names.push(name);
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(name),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &entry.queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
    }

    /// Ends the frame's last stage and copies its stamps out; encode last.
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(i) = self.current else { return };
        if self.frames[i].names.is_empty() {
            self.current = None;
            return;
        }
        self.mark(encoder, "");
        let entry = &self.frames[i];
        let count = entry.names.len() as u32;
        encoder.resolve_query_set(&entry.queries, 0..count, &entry.resolve, 0);
        encoder.copy_buffer_to_buffer(&entry.resolve, 0, &entry.readback, 0, u64::from(count) * 8);
    }

    /// After the frame's final submission: maps its stamps.
    pub(crate) fn submitted(&mut self) {
        let Some(i) = self.current.take() else { return };
        let entry = &mut self.frames[i];
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        let bytes = entry.names.len() as u64 * 8;
        entry
            .readback
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = callback.set(r);
            });
        entry.mapped = Some(done);
    }

    /// Writes every completed frame's line (call after a device poll).
    pub(crate) fn collect(&mut self) {
        for entry in &mut self.frames {
            let Some(result) = entry.mapped.as_ref().and_then(|m| m.get()) else {
                continue;
            };
            let bytes = entry.names.len() as u64 * 8;
            let stamps: Option<Vec<u64>> = result
                .is_ok()
                .then(|| {
                    let words = entry
                        .readback
                        .slice(..bytes)
                        .get_mapped_range()
                        .map(|view| {
                            view.chunks_exact(8)
                                .map(|c| u64::from_ne_bytes(c.try_into().unwrap()))
                                .collect()
                        });
                    entry.readback.unmap();
                    words.ok()
                })
                .flatten();
            if let Some(stamps) = stamps {
                let ms = |a: u64, b: u64| b.saturating_sub(a) as f64 * self.period * 1e-6;
                // Repeated stage names in one frame add up.
                let mut passes: Vec<(&str, f64)> = vec![];
                for (k, name) in entry.names.iter().enumerate().take(entry.names.len() - 1) {
                    let t = ms(stamps[k], stamps[k + 1]);
                    match passes.iter_mut().find(|(n, _)| n == name) {
                        Some((_, total)) => *total += t,
                        None => passes.push((name, t)),
                    }
                }
                let body: Vec<String> = passes
                    .iter()
                    .map(|(n, t)| format!("\"{n}\":{t:.4}"))
                    .collect();
                let span = ms(stamps[0], *stamps.last().unwrap());
                let _ = writeln!(
                    self.log,
                    "{{\"frame\":{},\"span\":{span:.4},\"passes\":{{{}}}}}",
                    entry.frame,
                    body.join(",")
                );
                let _ = self.log.flush();
            }
            entry.mapped = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_completion_reuses_free_entries_without_overwriting_pending_frames() {
        let mut free = vec![false; INITIAL_FRAMES];
        assert_eq!(available_frame(free.len(), 0, |i| free[i]), None);
        // The old ring checked only entry 0 and dropped the frame even though 2 was free.
        free[2] = true;
        assert_eq!(available_frame(free.len(), 0, |i| free[i]), Some(2));
        assert_eq!(available_frame(free.len(), 3, |i| free[i]), Some(2));
        free[2] = false;
        // Saturation asks the caller to allocate; the same policy works past four entries.
        free.push(true);
        assert_eq!(available_frame(free.len(), 3, |i| free[i]), Some(4));
    }
}
