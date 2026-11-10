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
use std::io::Write;
use std::sync::{Arc, OnceLock};

/// Marks per frame, including the final one. Each frame's stamps resolve at a multiple of
/// QUERY_RESOLVE_BUFFER_ALIGNMENT (256 bytes): 64 marks are 512.
const MARKS: u32 = 64;
const _: () = assert!((MARKS as u64 * 8) % wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT == 0);
/// Frames in flight before a frame goes untimed rather than waiting.
const RING: usize = 4;

struct Frame {
    readback: wgpu::Buffer,
    names: Vec<&'static str>,
    mapped: Option<Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>>,
    frame: u64,
}

pub(crate) struct PassTimeline {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    frames: Vec<Frame>,
    /// The ring entry this frame's marks go to; None when every entry is still in flight.
    current: Option<usize>,
    next: usize,
    frame: u64,
    period: f64,
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
        let bytes = u64::from(MARKS) * 8;
        let frames = (0..RING)
            .map(|_| Frame {
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pass timeline readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                names: vec![],
                mapped: None,
                frame: 0,
            })
            .collect();
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timeline"),
                ty: wgpu::QueryType::Timestamp,
                count: MARKS * RING as u32,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timeline resolve"),
                size: bytes * RING as u64,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            frames,
            current: None,
            next: 0,
            frame: 0,
            period: f64::from(queue.get_timestamp_period()),
            log: std::io::BufWriter::new(log),
        })
    }

    /// Starts a frame's marks, in the next free ring entry.
    pub(crate) fn begin(&mut self) {
        self.frame += 1;
        let i = self.next;
        let entry = &mut self.frames[i];
        self.current = entry.mapped.is_none().then(|| {
            entry.names.clear();
            entry.frame = self.frame;
            i
        });
        if self.current.is_some() {
            self.next = (i + 1) % RING;
        }
    }

    /// Starts stage `name` at this point of `encoder`'s work.
    pub(crate) fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, name: &'static str) {
        let Some(i) = self.current else { return };
        let entry = &mut self.frames[i];
        if entry.names.len() + 1 >= MARKS as usize {
            return;
        }
        let index = i as u32 * MARKS + entry.names.len() as u32;
        entry.names.push(name);
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(name),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
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
        let first = i as u32 * MARKS;
        let count = entry.names.len() as u32;
        let offset = u64::from(first) * 8;
        encoder.resolve_query_set(&self.queries, first..first + count, &self.resolve, offset);
        encoder.copy_buffer_to_buffer(&self.resolve, offset, &entry.readback, 0, u64::from(count) * 8);
    }

    /// After the frame's final submission: maps its stamps.
    pub(crate) fn submitted(&mut self) {
        let Some(i) = self.current.take() else { return };
        let entry = &mut self.frames[i];
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        let bytes = entry.names.len() as u64 * 8;
        entry.readback.slice(..bytes).map_async(wgpu::MapMode::Read, move |r| {
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
            let stamps: Option<Vec<u64>> = result.is_ok().then(|| {
                let words = entry.readback.slice(..bytes).get_mapped_range().map(|view| {
                    view.chunks_exact(8)
                        .map(|c| u64::from_ne_bytes(c.try_into().unwrap()))
                        .collect()
                });
                entry.readback.unmap();
                words.ok()
            }).flatten();
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
