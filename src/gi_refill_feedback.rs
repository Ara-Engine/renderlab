//! Nonblocking feedback for a soft gather target from the previous frame's measured cost.
use super::*;
use std::sync::OnceLock;

struct Sample {
    queries: Owned<wgpu::QuerySet>,
    resolve: Owned<wgpu::Buffer>,
    readback: Owned<wgpu::Buffer>,
    _allocation: Allocation,
    done: Option<Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>>,
    frame: u64,
}
pub(super) struct State {
    samples: Vec<Sample>,
    active: Option<usize>,
    frame: u64,
    period: f64,
    pub controller: priority::Controller,
}
impl State {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, ledger: &Ledger) -> Self {
        let samples = (0..4)
            .map(|_| {
                let buffer = |label, size, usage| {
                    ledger.own(
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some(label),
                            size,
                            usage,
                            mapped_at_creation: false,
                        }),
                        ResourceKind::Buffer,
                    )
                };
                Sample {
                    queries: ledger.own(
                        device.create_query_set(&wgpu::QuerySetDescriptor {
                            label: Some("GI refill feedback"),
                            ty: wgpu::QueryType::Timestamp,
                            count: 4,
                        }),
                        ResourceKind::QuerySet,
                    ),
                    resolve: buffer(
                        "GI refill resolve",
                        256,
                        wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    ),
                    readback: buffer(
                        "GI refill feedback",
                        112,
                        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    ),
                    _allocation: ledger.allocate(AllocationKind::Light, 368),
                    done: None,
                    frame: 0,
                }
            })
            .collect();
        Self {
            samples,
            active: None,
            frame: 0,
            period: f64::from(queue.get_timestamp_period()) * 1e-6,
            controller: Default::default(),
        }
    }
    pub fn collect(&mut self, target: f64) {
        let mut ready = Vec::new();
        for sample in &mut self.samples {
            let Some(result) = sample.done.as_ref().and_then(|d| d.get()) else {
                continue;
            };
            if result.is_ok() {
                let data = sample.readback.slice(..112).get_mapped_range().map(|view| {
                    let ticks: Vec<u64> = view[..32]
                        .chunks_exact(8)
                        .map(|b| u64::from_ne_bytes(b.try_into().unwrap()))
                        .collect();
                    let counts: Vec<u32> = view[32..]
                        .chunks_exact(4)
                        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
                        .collect();
                    (ticks, counts)
                });
                sample.readback.unmap();
                if let Ok((ticks, counts)) = data {
                    ready.push((
                        sample.frame,
                        counts,
                        ticks[3].saturating_sub(ticks[2]) as f64 * self.period,
                        ticks[1].saturating_sub(ticks[0]) as f64 * self.period,
                        ticks[2].saturating_sub(ticks[1]) as f64 * self.period,
                    ));
                }
            }
            sample.done = None;
        }
        ready.sort_by_key(|r| r.0);
        for (frame, c, claim_ms, first_ms, refresh_ms) in ready {
            let (claims, gathers, first) = (c[0], c[1], c[2]);
            self.controller.observe(
                frame,
                target,
                claims,
                gathers,
                claim_ms,
                first_ms + refresh_ms,
            );
            self.controller.observe_split(
                frame,
                priority::gather_target(),
                first,
                gathers.saturating_sub(first),
                first_ms,
                refresh_ms,
            );
            if let Some(path) = std::env::var_os("RENDERLAB_GI_REFILL_LOG") {
                use std::io::Write;
                let line = format!(
                    "{{\"frame\":{frame},\"cache_frame\":{},\"created\":{claims},\"gathers\":{gathers},\"first_gathers\":{first},\"refresh_gathers\":{},\"gather_rays\":{},\"visible_never_gathered\":{},\"visible_with_raw_data\":{},\"off_screen\":{},\"first_selected\":{},\"refresh_selected\":{},\"background_selected\":{},\"first_inflow\":{},\"visible_filtered_only\":{},\"estimated_floor_overrun\":{},\"first_ms\":{first_ms},\"refresh_ms\":{refresh_ms},\"claim_update_ms\":{claim_ms},\"soft_gather_target_ms\":{},\"actual_first_overrun\":{},\"soft_gather_target_active\":{},\"admitted_claim_attempts\":{},\"claim_budget\":{}}}\n",
                    c[13],
                    gathers.saturating_sub(first),
                    c[12],
                    c[14],
                    c[15],
                    c[16],
                    c[6],
                    c[7],
                    c[8],
                    c[9],
                    c[10],
                    c[11],
                    priority::gather_target(),
                    first_ms > priority::gather_target(),
                    c[17] != 0
                );
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                {
                    let _ = file.write_all(line.as_bytes());
                }
            }
        }
    }
    pub fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.active = self.samples.iter().position(|s| s.done.is_none());
        if let Some(i) = self.active {
            self.samples[i].frame = self.frame;
        }
        self.mark(encoder, 0);
    }
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    pub fn after_first(&self, encoder: &mut wgpu::CommandEncoder, work: &wgpu::Buffer) {
        self.mark(encoder, 1);
        if let Some(i) = self.active {
            encoder.copy_buffer_to_buffer(work, 33 * 4, &self.samples[i].readback, 40, 4);
        }
    }
    pub fn after_gathers(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        work: &wgpu::Buffer,
        arena: u32,
    ) {
        self.mark(encoder, 2);
        if let Some(i) = self.active {
            let readback = &self.samples[i].readback;
            encoder.copy_buffer_to_buffer(work, 33 * 4, readback, 36, 4);
            encoder.copy_buffer_to_buffer(work, 34 * 4, readback, 80, 4);
            encoder.copy_buffer_to_buffer(work, u64::from(arena) * 4, readback, 44, 24);
            encoder.copy_buffer_to_buffer(work, u64::from(arena + 7) * 4, readback, 68, 12);
            encoder.copy_buffer_to_buffer(work, u64::from(arena + 10) * 4, readback, 88, 12);
        }
    }
    pub fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        let Some(i) = self.active else { return };
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GI refill feedback stamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.samples[i].queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
    }
    pub fn after_claims(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        work: &wgpu::Buffer,
        arena: u32,
    ) {
        if let Some(i) = self.active {
            encoder.copy_buffer_to_buffer(
                work,
                u64::from(arena + 13) * 4,
                &self.samples[i].readback,
                104,
                4,
            );
        }
    }
    pub fn finish(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        work: &wgpu::Buffer,
        table: &wgpu::Buffer,
    ) {
        self.mark(encoder, 3);
        let Some(i) = self.active.take() else { return };
        let sample = &mut self.samples[i];
        encoder.resolve_query_set(&sample.queries, 0..4, &sample.resolve, 0);
        encoder.copy_buffer_to_buffer(&sample.resolve, 0, &sample.readback, 0, 32);
        encoder.copy_buffer_to_buffer(work, 7 * 4, &sample.readback, 32, 4);
        encoder.copy_buffer_to_buffer(table, 3 * 4, &sample.readback, 84, 4);
        encoder.copy_buffer_to_buffer(table, 41 * 4, &sample.readback, 100, 4);
        encoder.copy_buffer_to_buffer(table, 33 * 4, &sample.readback, 108, 4);
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.map_buffer_on_submit(&sample.readback, wgpu::MapMode::Read, 0..112, move |r| {
            let _ = callback.set(r);
        });
        sample.done = Some(done);
    }
}
