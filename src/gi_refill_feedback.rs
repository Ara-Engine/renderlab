//! Small nonblocking GPU-time feedback ring for the opt-in refill time target.
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
                        40,
                        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    ),
                    _allocation: ledger.allocate(AllocationKind::Light, 296),
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
                let data = sample.readback.slice(..40).get_mapped_range().map(|view| {
                    let ticks: Vec<u64> = view[..32]
                        .chunks_exact(8)
                        .map(|b| u64::from_ne_bytes(b.try_into().unwrap()))
                        .collect();
                    let claims = u32::from_ne_bytes(view[32..36].try_into().unwrap());
                    let gathers = u32::from_ne_bytes(view[36..40].try_into().unwrap());
                    (ticks, claims, gathers)
                });
                sample.readback.unmap();
                if let Ok((ticks, claims, gathers)) = data {
                    ready.push((
                        sample.frame,
                        claims,
                        gathers,
                        ticks[3].saturating_sub(ticks[2]) as f64 * self.period,
                        ticks[1].saturating_sub(ticks[0]) as f64 * self.period,
                    ));
                }
            }
            sample.done = None;
        }
        ready.sort_by_key(|r| r.0);
        for (frame, claims, gathers, claim_ms, gather_ms) in ready {
            self.controller
                .observe(frame, target, claims, gathers, claim_ms, gather_ms);
        }
    }
    pub fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.frame += 1;
        self.active = self.samples.iter().position(|s| s.done.is_none());
        if let Some(i) = self.active {
            self.samples[i].frame = self.frame;
        }
        self.mark(encoder, 0);
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
    pub fn finish(&mut self, encoder: &mut wgpu::CommandEncoder, work: &wgpu::Buffer) {
        self.mark(encoder, 3);
        let Some(i) = self.active.take() else { return };
        let sample = &mut self.samples[i];
        encoder.resolve_query_set(&sample.queries, 0..4, &sample.resolve, 0);
        encoder.copy_buffer_to_buffer(&sample.resolve, 0, &sample.readback, 0, 32);
        encoder.copy_buffer_to_buffer(work, 7 * 4, &sample.readback, 32, 4);
        encoder.copy_buffer_to_buffer(work, 33 * 4, &sample.readback, 36, 4);
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.map_buffer_on_submit(&sample.readback, wgpu::MapMode::Read, 0..40, move |r| {
            let _ = callback.set(r);
        });
        sample.done = Some(done);
    }
}
