//! Named GPU stage timings for measurement (P-SHOW2, docs/p-show2.md).
//!
//! With `RENDERLAB_PASS_LOG=<file>` set when the renderer is created (and timestamp
//! queries available), every frame writes a timestamp at the start of each named stage
//! and one at its end, and the renderer appends one JSON line per completed frame:
//! `{"frame":n,"renderer_frame":n,"dropped":false,"out_of_order":false,
//! "span":ms,"passes":{"skinning":ms,...}}`. A stage lasts from its mark to
//! the next mark; marks are empty compute passes, so they need only TIMESTAMP_QUERY.
//! Stages that the GPU overlaps share their boundary, so the stage times always sum to the
//! span. Embedders need no change: harnesses read the file (tools/showcase/showcase.py).
//! Without the variable nothing is created and every call returns at once.
//! `RENDERLAB_PASS_LOG_DETAIL` also keeps a stage's parts (names with a dot, such as
//! `sun_shadow.raster`); without it those marks are skipped, so the part's time stays in
//! the stage before it. The reflection pass's stages (`ALWAYS`) are kept either way: every
//! reflection table reads them from the default log.
//! The readback pool grows to the in-flight high-water mark instead of dropping samples
//! or waiting for the GPU, including captures longer than 600 frames. Completion can be
//! out of order: each entry flags a nonconsecutive emission, and consumers join by
//! `renderer_frame` (`frame` remains its compatibility alias), never by line position.
//! Failed readbacks and frames without timestamps emit `dropped:true` with a reason,
//! rather than fabricated timings. Polling is still needed to collect final readbacks.
//! Each line also carries the CPU time from each stage's mark to the next (`"cpu":{...}`, the
//! stage's encoding and any preparation the renderer does inline there; repeated names add up)
//! and `"cpu_head"`, from the frame's `begin` to its first mark. A mark named `cpu:...` is a CPU
//! point only (no GPU timestamp, no pass): it splits the CPU stages and nothing else.
//! Each line also carries `"compiles"` and `"compile_ms"`: the pipelines created since the previous
//! frame's `finish` and the time their creation took. A frame is submitted in parts, so a compile
//! between two parts leaves the GPU waiting inside the span and the stage being encoded: its times
//! are not GPU work, and consumers drop or flag such frames instead of averaging them in.
//! Measurement only: `RENDERLAB_CPU_SPLIT=1` also finishes and submits the encoder at every mark,
//! so each stage's own `CommandEncoder::finish` (wgpu's replay of its commands) is timed alone
//! (`"finish":{...}`, by the stage the commands were recorded in). It changes when queue writes
//! land against earlier commands, so pictures and GPU times under it are not the default's.
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
    /// Every mark (GPU stages and `cpu:` points) with when it was recorded, and when the frame
    /// began.
    cpu: Vec<(&'static str, std::time::Instant)>,
    begun: Option<std::time::Instant>,
    /// RENDERLAB_CPU_SPLIT: each split segment's finish time, by stage.
    finishes: Vec<(&'static str, std::time::Duration)>,
    mapped: Option<Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>>,
    frame: u64,
    /// Pipelines created for this frame (since the previous frame's finish) and their nanoseconds.
    compiles: (u64, u64),
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
            cpu: vec![],
            begun: None,
            finishes: vec![],
            mapped: None,
            frame: 0,
            compiles: (0, 0),
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

/// Keeps the old frame key while making gaps, reordered completions and failures explicit.
fn write_record(
    log: &mut impl Write,
    last_emitted: &mut u64,
    frame: u64,
    payload: &str,
) -> std::io::Result<()> {
    let out_of_order = frame != last_emitted.saturating_add(1);
    writeln!(
        log,
        "{{\"frame\":{frame},\"renderer_frame\":{frame},\"out_of_order\":{out_of_order},{payload}}}"
    )?;
    *last_emitted = frame;
    Ok(())
}

pub(crate) struct PassTimeline {
    frames: Vec<Frame>,
    /// The pool entry receiving this frame's marks; None outside frame encoding.
    current: Option<usize>,
    next: usize,
    frame: u64,
    last_emitted: u64,
    period: f64,
    /// Whether dotted part names are marked (`RENDERLAB_PASS_LOG_DETAIL`).
    detail: bool,
    log: std::io::BufWriter<std::fs::File>,
    /// RENDERLAB_CPU_SPLIT: the device and queue that finish and submit at each mark.
    split: Option<(wgpu::Device, wgpu::Queue)>,
    /// The pipeline device's compile totals at the previous frame's finish.
    compiled: (u64, u64),
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
            last_emitted: 0,
            period: f64::from(queue.get_timestamp_period()),
            detail: std::env::var_os("RENDERLAB_PASS_LOG_DETAIL").is_some(),
            log: std::io::BufWriter::new(log),
            split: (std::env::var("RENDERLAB_CPU_SPLIT").as_deref() == Ok("1"))
                .then(|| (device.clone(), queue.clone())),
            compiled: (0, 0),
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
        entry.cpu.clear();
        entry.finishes.clear();
        entry.begun = Some(std::time::Instant::now());
        entry.frame = self.frame;
        self.current = Some(i);
        self.next = (i + 1) % self.frames.len();
    }

    /// Whether dotted part names are marked (`RENDERLAB_PASS_LOG_DETAIL`).
    pub(crate) fn detail(&self) -> bool {
        self.detail
    }

    /// Available dotted marks, excluding the final timestamp. Diagnostics can preserve
    /// the frame's remaining stages rather than silently consuming the entire pool.
    pub(crate) fn detail_marks_remaining(&self) -> usize {
        if !self.detail {
            return 0;
        }
        self.current.map_or(0, |i| {
            (MARKS as usize - 1).saturating_sub(self.frames[i].names.len())
        })
    }

    /// Starts stage `name` at this point of `encoder`'s work.
    pub(crate) fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, name: &'static str) {
        let Some(i) = self.current else { return };
        if let (Some((device, queue)), Some(&(stage, _))) = (&self.split, self.frames[i].cpu.last()) {
            let start = std::time::Instant::now();
            let next = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cpu split") });
            let commands = std::mem::replace(encoder, next).finish();
            self.frames[i].finishes.push((stage, start.elapsed()));
            queue.submit([commands]);
        }
        if name.starts_with("cpu:") {
            self.frames[i].cpu.push((name, std::time::Instant::now()));
            return;
        }
        if name.contains('.') && !self.detail && !ALWAYS.contains(&name) {
            return;
        }
        let entry = &mut self.frames[i];
        if entry.names.len() + 1 >= MARKS as usize {
            return;
        }
        let index = entry.names.len() as u32;
        entry.names.push(name);
        entry.cpu.push((name, std::time::Instant::now()));
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(name),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &entry.queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
    }

    /// Ends the frame's last stage and copies its stamps out; encode last. `compiled` is the
    /// pipeline device's compile totals now (`PipelineDevice::compile_totals`): the frame is
    /// booked what was compiled since the previous frame's finish.
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder, compiled: (u64, u64)) {
        let Some(i) = self.current else { return };
        self.frames[i].compiles = (compiled.0 - self.compiled.0, compiled.1 - self.compiled.1);
        self.compiled = compiled;
        if self.frames[i].names.is_empty() {
            let _ = write_record(
                &mut self.log,
                &mut self.last_emitted,
                self.frames[i].frame,
                "\"dropped\":true,\"reason\":\"no_timestamps\"",
            );
            let _ = self.log.flush();
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
                // CPU time from each mark to the next, by name.
                let mut cpu: Vec<(&str, f64)> = vec![];
                for (k, (name, at)) in entry.cpu.iter().enumerate().take(entry.cpu.len().saturating_sub(1)) {
                    let t = (entry.cpu[k + 1].1 - *at).as_secs_f64() * 1e3;
                    match cpu.iter_mut().find(|(n, _)| n == name) {
                        Some((_, total)) => *total += t,
                        None => cpu.push((name, t)),
                    }
                }
                let cpu: Vec<String> = cpu.iter().map(|(n, t)| format!("\"{n}\":{t:.4}")).collect();
                let mut finishes: Vec<(&str, f64)> = vec![];
                for (name, d) in &entry.finishes {
                    let t = d.as_secs_f64() * 1e3;
                    match finishes.iter_mut().find(|(n, _)| n == name) {
                        Some((_, total)) => *total += t,
                        None => finishes.push((name, t)),
                    }
                }
                let finishes: Vec<String> = finishes.iter().map(|(n, t)| format!("\"{n}\":{t:.4}")).collect();
                let head = match (entry.begun, entry.cpu.first()) {
                    (Some(begun), Some(&(_, first))) => (first - begun).as_secs_f64() * 1e3,
                    _ => 0.0,
                };
                let span = ms(stamps[0], *stamps.last().unwrap());
                let (compiles, compile_ns) = entry.compiles;
                let _ = write_record(
                    &mut self.log,
                    &mut self.last_emitted,
                    entry.frame,
                    &format!(
                        "\"dropped\":false,\"span\":{span:.4},\"compiles\":{compiles},\"compile_ms\":{:.3},\"passes\":{{{}}},\"cpu_head\":{head:.4},\"cpu\":{{{}}},\"finish\":{{{}}}",
                        compile_ns as f64 * 1e-6,
                        body.join(","),
                        cpu.join(","),
                        finishes.join(",")
                    ),
                );
            } else {
                let _ = write_record(
                    &mut self.log,
                    &mut self.last_emitted,
                    entry.frame,
                    "\"dropped\":true,\"reason\":\"readback_failed\"",
                );
            }
            let _ = self.log.flush();
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

    #[test]
    fn capture_records_keep_all_600_frame_ids_and_flag_reordered_completions() {
        let mut log = Vec::new();
        let mut last = 0;
        let mut frames: Vec<u64> = (1..=600).collect();
        frames.swap(199, 200);
        for frame in frames {
            write_record(
                &mut log,
                &mut last,
                frame,
                "\"dropped\":false,\"span\":1.25,\"passes\":{\"shading\":1.25}",
            )
            .unwrap();
        }
        let records: Vec<serde_json::Value> = std::str::from_utf8(&log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 600);
        let mut ids: Vec<u64> = records
            .iter()
            .map(|r| {
                assert_eq!(r["frame"], r["renderer_frame"]);
                assert_eq!(r["dropped"], false);
                r["renderer_frame"].as_u64().unwrap()
            })
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=600).collect::<Vec<_>>());
        assert_eq!(records[198]["out_of_order"], false);
        assert_eq!(records[199]["out_of_order"], true); // 201 after 199
        assert_eq!(records[200]["out_of_order"], true); // 200 after 201
        assert_eq!(records[201]["out_of_order"], true); // 202 after 200
        assert_eq!(records[202]["out_of_order"], false);
    }

    #[test]
    fn dropped_records_preserve_frame_identity_without_inventing_timings() {
        let mut log = Vec::new();
        let mut last = 0;
        for (frame, reason) in [(1, "no_timestamps"), (2, "readback_failed")] {
            write_record(
                &mut log,
                &mut last,
                frame,
                &format!("\"dropped\":true,\"reason\":\"{reason}\""),
            )
            .unwrap();
        }
        for (i, line) in std::str::from_utf8(&log).unwrap().lines().enumerate() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["renderer_frame"], i + 1);
            assert_eq!(record["frame"], record["renderer_frame"]);
            assert_eq!(record["dropped"], true);
            assert_eq!(record["out_of_order"], false);
            assert_eq!(record["reason"], ["no_timestamps", "readback_failed"][i]);
            assert!(record.get("span").is_none());
            assert!(record.get("passes").is_none());
        }
    }
}
