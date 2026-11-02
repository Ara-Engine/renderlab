use std::{
    collections::VecDeque,
    ops::Deref,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadBytes {
    pub geometry: u64,
    pub models: u64,
    pub camera: u64,
    pub write_calls: u64,
}
impl UploadBytes {
    pub(crate) fn add(&mut self, other: Self) {
        self.geometry += other.geometry;
        self.models += other.models;
        self.camera += other.camera;
        self.write_calls += other.write_calls;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuFrame {
    pub preparation: Duration,
    pub upload: Duration,
    pub encode: Duration,
    pub submit: Duration,
    pub poll: Duration,
}
#[derive(Debug)]
pub struct FrameReport {
    pub frame: FrameId,
    pub revision: Revision,
    pub cpu: CpuFrame,
    pub uploads: UploadBytes,
    pub timed: bool,
}
#[derive(Debug)]
pub struct ReloadReport {
    pub revision: Revision,
    pub compile: Duration,
}
#[derive(Debug)]
pub struct Completion {
    pub submission: u64,
    pub frame: Option<FrameId>,
    pub revision: Revision,
    /// Host collection time, not GPU execution time or scanout.
    pub observed_at: Instant,
    pub submitted_at: Instant,
}
#[derive(Debug)]
pub struct GpuSample {
    pub frame: FrameId,
    pub revision: Revision,
    /// Only the mesh render pass. Excludes uploads, resolve, acquire and present.
    pub mesh_pass: Duration,
}
#[derive(Debug, Default)]
pub struct Events {
    pub completed: Vec<Completion>,
    pub gpu: Vec<GpuSample>,
    /// Cumulative observations dropped because the caller did not drain events.
    pub dropped: u64,
    pub poll_cpu: Duration,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimingStatus {
    Enabled,
    /// TIMESTAMP_QUERY was not enabled on the supplied device.
    Unavailable,
    Disabled {
        diagnostic: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub created: u64,
    pub dropped: u64,
    pub live: u64,
    pub peak: u64,
}
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum ResourceKind {
    Buffer,
    Texture,
    TextureView,
    BindGroup,
    BindGroupLayout,
    PipelineLayout,
    RenderPipeline,
    ShaderModule,
    QuerySet,
}
const RESOURCE_KINDS: usize = 9;

/// Renderlab ownership only. No driver residency claim. Caller-owned colors are excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ownership {
    pub persistent: [Count; RESOURCE_KINDS],
    /// Borrowed color attachment views are intentionally short-lived.
    pub attachment_views: Count,
}
impl Ownership {
    pub fn get(&self, kind: ResourceKind) -> Count {
        self.persistent[kind as usize]
    }
    pub fn created(&self) -> u64 {
        self.persistent.iter().map(|c| c.created).sum()
    }
}

#[derive(Clone, Default)]
pub(crate) struct Ledger(Arc<Mutex<Ownership>>);
impl Ledger {
    pub fn own<T>(&self, resource: T, kind: ResourceKind) -> Owned<T> {
        let mut ledger = self.0.lock().unwrap();
        let count = &mut ledger.persistent[kind as usize];
        count.created += 1;
        count.live += 1;
        count.peak = count.peak.max(count.live);
        Owned {
            resource,
            ledger: self.clone(),
            kind: Some(kind),
        }
    }
    pub fn attachment(&self, resource: wgpu::TextureView) -> Owned<wgpu::TextureView> {
        let mut ledger = self.0.lock().unwrap();
        let count = &mut ledger.attachment_views;
        count.created += 1;
        count.live += 1;
        count.peak = count.peak.max(count.live);
        Owned {
            resource,
            ledger: self.clone(),
            kind: None,
        }
    }
    pub fn snapshot(&self) -> Ownership {
        self.0.lock().unwrap().clone()
    }
}
pub(crate) struct Owned<T> {
    resource: T,
    ledger: Ledger,
    kind: Option<ResourceKind>,
}
impl<T> Deref for Owned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.resource
    }
}
impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        let mut ledger = self.ledger.0.lock().unwrap();
        let count = match self.kind {
            Some(kind) => &mut ledger.persistent[kind as usize],
            None => &mut ledger.attachment_views,
        };
        count.live -= 1;
        count.dropped += 1;
    }
}

/// Native backend live objects and attributed bytes, including retained submitted resources.
/// These values are not physical driver memory usage. Availability is backend dependent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HalStats {
    pub buffers: isize,
    /// Raw wgpu 30 Vulkan value. Ordinary creation misses an increment in wgpu-hal.
    pub textures: isize,
    pub texture_count_reliable: bool,
    pub texture_views: isize,
    pub bind_groups: isize,
    pub bind_group_layouts: isize,
    pub pipeline_layouts: isize,
    pub render_pipelines: isize,
    pub shader_modules: isize,
    pub query_sets: isize,
    pub buffer_bytes: isize,
    pub texture_bytes: isize,
    /// Raw backend counter, not maintained by wgpu 30 Vulkan; use allocator bytes there.
    pub memory_allocations: isize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocatorStats {
    pub allocated_bytes: u64,
    pub reserved_bytes: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineStats {
    pub attempts: u64,
    pub successes: u64,
    pub installations: u64,
}
#[derive(Clone, Debug)]
pub struct Stats {
    pub ownership: Ownership,
    pub hal: HalStats,
    pub allocator: Option<AllocatorStats>,
    pub pipelines: PipelineStats,
    pub uploads: UploadBytes,
    pub submissions: u64,
    pub frames: u64,
    pub flights: usize,
    pub peak_flights: usize,
    pub busy_uploads: u64,
    pub busy_frames: u64,
    pub timing: TimingStatus,
    pub timing_failures: u64,
    pub timed_frames: u64,
    /// Successfully collected samples, including observations later dropped from the event queue.
    pub gpu_samples: u64,
    pub events_dropped: u64,
    pub revision: Revision,
    pub upload_cpu: CpuFrame,
}

pub(crate) struct Observation {
    pub completion: Completion,
    pub gpu: Option<GpuSample>,
}
#[derive(Default)]
pub(crate) struct EventQueue {
    items: VecDeque<Observation>,
    pub dropped: u64,
}
impl EventQueue {
    pub fn push(&mut self, event: Observation) {
        if self.items.len() == 16 {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back(event);
    }
    pub fn drain(&mut self, poll_cpu: Duration) -> Events {
        let mut events = Events {
            dropped: self.dropped,
            poll_cpu,
            ..Events::default()
        };
        for event in self.items.drain(..) {
            events.completed.push(event.completion);
            if let Some(gpu) = event.gpu {
                events.gpu.push(gpu);
            }
        }
        events
    }
}
