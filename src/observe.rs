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
    /// SK1 joint palettes and dispatch parameters, separate from static geometry edits.
    pub skinning: u64,
    pub environment: u64,
    pub shadows: u64,
    pub geometry: u64,
    pub lights: u64,
    pub lighting: u64,
    pub light_writes: u64,
    pub lighting_writes: u64,
    pub lut: u64,
    pub textures: u64,
    pub texture_writes: u64,
    pub materials: u64,
    pub normals: u64,
    pub view_settings: u64,
    pub models: u64,
    pub camera: u64,
    /// Initial transform publication, separate from later model edits.
    pub instances: u64,
    pub visible_ids: u64,
    pub model_writes: u64,
    pub visible_id_writes: u64,
    pub write_calls: u64,
}
impl UploadBytes {
    pub(crate) fn add(&mut self, other: Self) {
        self.skinning += other.skinning;
        self.environment += other.environment;
        self.shadows += other.shadows;
        self.geometry += other.geometry;
        self.lights += other.lights;
        self.lighting += other.lighting;
        self.light_writes += other.light_writes;
        self.lighting_writes += other.lighting_writes;
        self.lut += other.lut;
        self.textures += other.textures;
        self.texture_writes += other.texture_writes;
        self.materials += other.materials;
        self.normals += other.normals;
        self.view_settings += other.view_settings;
        self.models += other.models;
        self.camera += other.camera;
        self.instances += other.instances;
        self.visible_ids += other.visible_ids;
        self.model_writes += other.model_writes;
        self.visible_id_writes += other.visible_id_writes;
        self.write_calls += other.write_calls;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuFrame {
    /// Full submitted bounds, shadow-view fitting and caster assignment. Disjoint from visibility.
    pub shadow_preparation: Duration,
    /// Shadow resource allocation, matrix packing and queue writes, excluded from `upload`.
    pub shadow_upload: Duration,
    /// Shadow pass command encoding, excluded from `encode`.
    pub shadow_encode: Duration,
    /// Input validation and camera preparation. Visibility and byte packing are separate.
    pub preparation: Duration,
    pub visibility: Duration,
    pub packing: Duration,
    pub upload: Duration,
    pub encode: Duration,
    pub submit: Duration,
    pub poll: Duration,
}
#[derive(Debug)]
pub struct FrameReport {
    /// The view this frame rendered.
    pub view: crate::ViewId,
    pub frame: FrameId,
    pub revision: Revision,
    pub cpu: CpuFrame,
    pub uploads: UploadBytes,
    pub timed: bool,
    pub scene: SceneCounts,
    /// The path that resolved this frame's camera visibility and shading.
    pub primary: crate::PrimaryVisibility,
    /// Estimator used by this frame. Exact also denotes hard, unshadowed, raster
    /// and diagnostic frames, where the Interactive preference does not apply.
    pub soft_shadows: crate::SoftShadows,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneCounts {
    pub collections: u64,
    pub total_instances: u64,
    pub visible_instances: u64,
    pub submitted_instances: u64,
    pub draws: u64,
    pub triangles: u64,
    pub cull_full_tests: u64,
    pub cull_partial_tests: u64,
    /// CPU capacity retained for transform-derived visibility bounds, separate from GPU memory.
    pub visibility_cache_bytes: u64,
    /// Retained inverse-transpose CPU capacity and persistent GPU storage.
    pub normal_cache_bytes: u64,
    pub normal_gpu_bytes: u64,
}
#[derive(Debug)]
pub struct ReloadReport {
    pub revision: Revision,
    pub compile: Duration,
    /// Program reloads only: `Parameters` members that are new or retyped, which every
    /// existing material of the program now reads as zero. Members with nonzero
    /// DEFAULT_PARAMETERS values use those defaults and are excluded.
    pub zeroed_parameters: Vec<String>,
}
#[derive(Debug)]
pub struct Completion {
    pub submission: u64,
    pub frame: Option<FrameId>,
    /// The view a frame rendered; None for uploads.
    pub view: Option<crate::ViewId>,
    pub revision: Revision,
    /// Host collection time, not GPU execution time or scanout.
    pub observed_at: Instant,
    pub submitted_at: Instant,
}
#[derive(Debug)]
pub struct GpuSample {
    pub frame: FrameId,
    pub revision: Revision,
    /// Render-pass boundary timestamps, including material shading and configured samples.
    /// Excludes uploads, timestamp-query resolve, acquire and present. Attachment resolve is not isolated.
    /// Traced frames time their deferred compute shading pass here.
    pub mesh_pass: Duration,
    /// Fresh soft-shadow receiver raster, when enabled. Traced frames time their camera
    /// rays here, including exact receivers when soft. Traced Interactive frames fuse
    /// their 8-direction visibility into this pass.
    pub receiver_pass: Option<Duration>,
    /// Separate fresh compute visibility (raster soft and traced Exact frames).
    pub visibility_compute: Option<Duration>,
    /// Separate HDR presentation pass boundaries; excludes acquire/present and uploads.
    pub present_pass: Option<Duration>,
    /// Temporal reconstruction's sample copy and resolve pass (TR1 frames).
    pub reconstruction_pass: Option<Duration>,
    /// First shadow pass begin through final shadow pass end. No CPU wait per frame.
    pub shadow_generation: Option<Duration>,
    /// First rendered pass begin through last pass end, measured directly, not a sum.
    pub render_span: Duration,
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
    Sampler,
    Blas,
    Tlas,
    ComputePipeline,
}
const RESOURCE_KINDS: usize = 13;

/// Renderlab ownership only. No driver residency claim. Caller-owned colors are excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ownership {
    pub persistent: [Count; RESOURCE_KINDS],
    /// Borrowed color attachment views are intentionally short-lived.
    pub attachment_views: Count,
    /// Byte counts by purpose. Includes caller resources while owned; excludes driver retention.
    pub environment_bytes: Count,
    pub environment_creations: u64,
    pub texture_bytes: Count,
    pub material_bytes: Count,
    pub target_bytes: Count,
    pub light_bytes: Count,
    pub shadow_bytes: Count,
    /// Conservative AS admission estimates, separate from observed allocator bytes.
    pub hardware_as_estimated_bytes: Count,
    /// Only allocations positively identified by the safe allocator report.
    pub hardware_as_observed_bytes: Count,
    pub hardware_scratch_estimated_bytes: Count,
    pub lut_bytes: Count,
    pub light_creations: u64,
    pub shadow_creations: u64,
    pub lut_creations: u64,
    pub material_creations: u64,
    pub texture_creations: u64,
    pub target_creations: u64,
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
    pub fn allocate(&self, kind: AllocationKind, bytes: u64) -> Allocation {
        let mut ledger = self.0.lock().unwrap();
        match kind {
            AllocationKind::Environment => ledger.environment_creations += 1,
            AllocationKind::Texture => ledger.texture_creations += 1,
            AllocationKind::Material => ledger.material_creations += 1,
            AllocationKind::Target => ledger.target_creations += 1,
            AllocationKind::Light => ledger.light_creations += 1,
            AllocationKind::Shadow => ledger.shadow_creations += 1,
            AllocationKind::Lut => ledger.lut_creations += 1,
            AllocationKind::HardwareAsEstimate
            | AllocationKind::HardwareAsObserved
            | AllocationKind::HardwareScratchEstimate => {}
        }
        let count = allocation_count(&mut ledger, kind);
        count.created += bytes;
        count.live += bytes;
        count.peak = count.peak.max(count.live);
        Allocation {
            ledger: self.clone(),
            kind,
            bytes,
        }
    }
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
#[derive(Clone, Copy)]
pub(crate) enum AllocationKind {
    Environment,
    Texture,
    Material,
    Target,
    Light,
    Shadow,
    Lut,
    HardwareAsEstimate,
    HardwareAsObserved,
    HardwareScratchEstimate,
}
fn allocation_count(ledger: &mut Ownership, kind: AllocationKind) -> &mut Count {
    match kind {
        AllocationKind::Environment => &mut ledger.environment_bytes,
        AllocationKind::Texture => &mut ledger.texture_bytes,
        AllocationKind::Material => &mut ledger.material_bytes,
        AllocationKind::Target => &mut ledger.target_bytes,
        AllocationKind::Light => &mut ledger.light_bytes,
        AllocationKind::Shadow => &mut ledger.shadow_bytes,
        AllocationKind::Lut => &mut ledger.lut_bytes,
        AllocationKind::HardwareAsEstimate => &mut ledger.hardware_as_estimated_bytes,
        AllocationKind::HardwareAsObserved => &mut ledger.hardware_as_observed_bytes,
        AllocationKind::HardwareScratchEstimate => &mut ledger.hardware_scratch_estimated_bytes,
    }
}
pub(crate) struct Allocation {
    ledger: Ledger,
    kind: AllocationKind,
    bytes: u64,
}
impl Drop for Allocation {
    fn drop(&mut self) {
        let mut ledger = self.ledger.0.lock().unwrap();
        let count = allocation_count(&mut ledger, self.kind);
        count.live -= self.bytes;
        count.dropped += self.bytes;
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
/// Another handle to the same resource, counted as one more live handle: a view's state
/// shares the renderer's layouts this way (`Renderer::create_view`).
impl<T: Clone> Clone for Owned<T> {
    fn clone(&self) -> Self {
        let mut ledger = self.ledger.0.lock().unwrap();
        let count = match self.kind {
            Some(kind) => &mut ledger.persistent[kind as usize],
            None => &mut ledger.attachment_views,
        };
        count.created += 1;
        count.live += 1;
        count.peak = count.peak.max(count.live);
        drop(ledger);
        Self {
            resource: self.resource.clone(),
            ledger: self.ledger.clone(),
            kind: self.kind,
        }
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
    /// Render and compute pipelines created, cumulative: engine pipelines at renderer
    /// creation, reloads and traced selection, and program pipeline sets when a reload
    /// recreates the last frame's or a frame first draws with one.
    pub created: u64,
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

/// Owned lighting generations, not physical driver residency. Replaced resources stay
/// charged until the submission that last used them has completed.
#[derive(Clone, Copy, Debug)]
pub struct LightingStats {
    pub max_lights: u32,
    pub lights: u32,
    pub buffer_generation: u64,
    pub target_generation: u64,
    pub retained_light_generations: usize,
    pub retained_target_generations: usize,
    pub lut_bytes: u64,
}
