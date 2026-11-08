// AT1: atmosphere LUTs, camera volume and raster sky (docs/m7-at1.md).
#[path = "atmosphere_gpu.rs"]
mod atmosphere_gpu;
#[path = "environment_gpu.rs"]
mod environment_gpu;
#[path = "light_cluster_gpu.rs"]
mod light_cluster_gpu;
#[path = "lighting_gpu.rs"]
mod lighting_gpu;
// DC1: one storage binding beside lighting; no separate render pass.
#[path = "decal_gpu.rs"]
mod decal_gpu;
use lighting_gpu::LightingGpu;
// PP1: presentation-only resources, independent of temporal reconstruction history.
#[path = "post_gpu.rs"]
mod post_gpu;
use post_gpu::PostGpu;
#[path = "shadow_gpu.rs"]
mod shadow_gpu;
use sha2::{Digest, Sha256};
use shadow_gpu::{ShadowDraw, ShadowFrame, ShadowState};
#[path = "hardware_shadow_gpu.rs"]
pub(crate) mod hardware_shadow_gpu;
use hardware_shadow_gpu::HardwareState;
#[path = "fresh_shadow_gpu.rs"]
mod fresh_shadow_gpu;
use fresh_shadow_gpu::{FreshPipelines, FreshState};
#[path = "primary_ray_gpu.rs"]
mod primary_ray_gpu;
use primary_ray_gpu::temporal_reconstruction::{self, ReconstructionState};
pub use primary_ray_gpu::temporal_reconstruction::{
    Reconstruction, ReconstructionShadowBudget, RenderScale,
};
use primary_ray_gpu::{PrimaryPipelines, PrimaryState};
#[path = "mesh_lod.rs"]
mod mesh_lod;
#[path = "virtual_shadow_gpu.rs"]
mod virtual_shadow_gpu;
#[path = "visibility_buffer.rs"]
pub(crate) mod visibility_buffer;
pub use mesh_lod::{LodPackage, LodStats, MeshLod};
pub use virtual_shadow_gpu::{
    VIRTUAL_FEATURES, VirtualLookup, VirtualShadowSettings, VirtualShadowStats, WalkRecords,
};
#[path = "program_gpu.rs"]
mod program_gpu;
pub(crate) use program_gpu::ProgramReload;
use program_gpu::{Preludes, ProgramAbi, ProgramSlot};
#[path = "texture_residency.rs"]
mod texture_residency;
pub use texture_residency::{ResidencyStats, TextureResidency};
#[path = "gi_cache.rs"]
mod gi_cache;
#[path = "skinning.rs"]
pub(crate) mod skinning;
// TP1 transparent pass and particles (docs/m7-tp1.md).
#[path = "transparency_gpu.rs"]
pub(crate) mod transparency_gpu;
pub use gi_cache::{
    IndirectCounters, IndirectLookup, IndirectProfile, IndirectSettings, IndirectStats,
    ProbeCounters,
};
pub(crate) struct PreparedDraw<'a> {
    pub mesh: &'a crate::Mesh,
    pub instances: &'a crate::Instances,
    pub material: Option<&'a crate::Material>,
    pub indices: std::ops::Range<u32>,
    pub slots: std::ops::Range<u32>,
    #[cfg(test)]
    pub reference_ids: Option<&'a wgpu::Buffer>,
}
use crate::{
    instances::InstanceState,
    mesh::matrix_bytes,
    observe::{Allocation, AllocationKind, EventQueue, Ledger, Observation, Owned},
    visibility::Aabb,
    *,
};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

const FLIGHTS: usize = 3;
const CLEAR_COLOR: [f64; 3] = [0.018, 0.025, 0.04];
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];
const TANGENT_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![5 => Float32x4];
const MODEL_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![4 => Uint32];

type MapResult = Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>;
/// Records `next` as the state last set in one pass slot. True when the pass must be told.
/// Identity, not equality: every compared object outlives the pass that references it.
fn changed<'a, T>(slot: &mut Option<&'a T>, next: &'a T) -> bool {
    let changed = !slot.is_some_and(|last| std::ptr::eq(last, next));
    *slot = Some(next);
    changed
}
pub(crate) struct Flight {
    done: Arc<OnceLock<()>>,
    mapped: Option<MapResult>,
    lit: bool,
    shadowed: bool,
    /// Receiver and separate visibility compute timestamps follow the frame's own.
    fresh: bool,
    submission: u64,
    frame: Option<FrameId>,
    revision: Revision,
    submitted_at: Instant,
}

/// Byte-identity evidence for the GPU suites: with `RENDERLAB_TEST_CAPTURE_DIR` set, every
/// test readback is also written there as `<test>-<sequence>-<kind>.bin`, in process order.
#[cfg(test)]
pub(crate) fn test_record_capture(kind: &str, bytes: &[u8]) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let Some(dir) = std::env::var_os("RENDERLAB_TEST_CAPTURE_DIR") else {
        return;
    };
    let index = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("main").replace("::", ".");
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{test}-{index:04}-{kind}.bin")), bytes).unwrap();
}

#[cfg(test)]
impl Renderer {
    /// Reloads the standard program, as edits of its file do.
    pub(crate) fn test_reload_standard(
        &mut self,
        source: &str,
    ) -> Result<ReloadReport, ReloadError> {
        let standard = self.standard.clone();
        self.reload_program(&standard, source)
    }
    /// The standard program's pipelines, every set this renderer can build created
    /// first, which tests swap for probes and references.
    pub(crate) fn test_standard(&mut self) -> &mut program_gpu::ProgramPipelines {
        self.ensure_standard_pipelines();
        &mut self.programs[0].pipelines
    }
    /// A lit pair from a composed raster module, on the hardware-shadow ABI or the plain
    /// lit ABI.
    pub(crate) fn test_lit_pair(
        &self,
        composed: &str,
        hardware: bool,
    ) -> [Owned<wgpu::RenderPipeline>; 2] {
        let scopes = OperationScopes::new(&self.device);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("test hardware lit pair"),
                source: wgpu::ShaderSource::Wgsl(composed.into()),
            });
        let layout = if hardware {
            &self.hardware_shadows.as_ref().unwrap().layout
        } else {
            &self.lighting.layout
        };
        let pair = [false, true].map(|tangent| {
            mesh_pipeline(
                &self.device,
                &self.ledger,
                &module,
                layout,
                wgpu::TextureFormat::Rgba16Float,
                "fs_lit",
                tangent,
                self.options.samples,
                None,
            )
        });
        assert!(scopes.finish().is_none());
        pair
    }
    pub(crate) fn test_drain(&mut self, label: &str) {
        eprintln!("finite_gpu_drain label={label} timeout_seconds=20");
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        self.collect().unwrap();
        assert_eq!(self.occupied(), 0);
    }
    pub(crate) fn test_overlap(&mut self) -> (usize, Stats) {
        self.collect().unwrap();
        // Only unset cells *after* Poll qualify. Unpolled callbacks are not overlap evidence.
        let incomplete = self
            .slots
            .iter()
            .filter_map(|s| s.flight.as_ref())
            .filter(|f| f.frame.is_some() && f.done.get().is_none())
            .count();
        (incomplete, self.stats())
    }
    pub(crate) fn test_reference(&mut self, enabled: bool) {
        self.reference_draws = enabled;
    }
    pub(crate) fn test_capture_color(&mut self, color: &wgpu::Texture) -> Vec<u8> {
        self.test_readback(color, false)
    }
    pub(crate) fn test_capture_full(&mut self, color: &wgpu::Texture) -> (Vec<u8>, Vec<u8>) {
        assert_eq!(
            self.options.samples,
            SampleCount::One,
            "multisample depth cannot be copied"
        );
        let rgba = self.test_readback(color, false);
        let depth = self.depth.as_ref().unwrap().texture.clone();
        (rgba, self.test_readback(&depth, true))
    }
    fn test_readback(&mut self, color: &wgpu::Texture, depth: bool) -> Vec<u8> {
        self.test_drain("capture admission");
        let row = (color.width() * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("test image readback"),
                size: u64::from(row) * u64::from(color.height()),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                aspect: if depth {
                    wgpu::TextureAspect::DepthOnly
                } else {
                    wgpu::TextureAspect::All
                },
                ..color.as_image_copy()
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(color.height()),
                },
            },
            color.size(),
        );
        let mapped = Arc::new(OnceLock::new());
        let callback = mapped.clone();
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            let _ = callback.set(result);
        });
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        let submitted_at = Instant::now();
        self.queue.submit([encoder.finish()]);
        self.submissions += 1;
        let slot = self.free_slot().unwrap();
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
        self.test_drain("capture copy");
        mapped.get().unwrap().as_ref().unwrap();
        let view = buffer.get_mapped_range(..).unwrap();
        let mut bytes = Vec::with_capacity((color.width() * color.height() * 4) as usize);
        for row_bytes in view.chunks_exact(row as usize) {
            bytes.extend_from_slice(&row_bytes[..color.width() as usize * 4]);
        }
        drop(view);
        buffer.unmap();
        test_record_capture(if depth { "depth" } else { "color" }, &bytes);
        bytes
    }
    pub(crate) fn test_failed_map(&mut self) {
        self.test_drain("injected failed mapping setup");
        let done = Arc::new(OnceLock::from(()));
        let mapped = Arc::new(OnceLock::from(Err(wgpu::BufferAsyncError)));
        self.slots[0].flight = Some(Flight {
            done,
            mapped: Some(mapped),
            lit: false,
            shadowed: false,
            fresh: false,
            submission: self.submissions,
            frame: Some(FrameId(self.frames)),
            revision: self.revision,
            submitted_at: Instant::now(),
        });
        self.collect().unwrap();
        assert!(matches!(self.timing_status, TimingStatus::Disabled { .. }));
        self.check_fault().unwrap();
    }
    pub(crate) fn test_busy(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        mesh: &Mesh,
        data: &MeshData<'_>,
        instances: &mut Instances,
        foreign: &mut Instances,
    ) {
        self.test_drain("deterministic admission boundary setup");
        for slot in &mut self.slots {
            slot.flight = Some(Flight {
                done: Arc::new(OnceLock::new()),
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at: Instant::now(),
            });
        }
        instances
            .set(0, Mat4::from_translation(Vec3::new(0.123, 0.0, 0.0)))
            .unwrap();
        let dirty = instances.state.dirty_count;
        let ids = instances.state.ids.clone();
        let before = self.stats();
        assert!(matches!(
            self.try_render(color, camera, &mut [Draw::new(mesh, foreign)]),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(before.busy_frames, self.stats().busy_frames);
        assert!(
            self.try_upload_instances(&[Mat4::IDENTITY])
                .unwrap()
                .is_none()
        );
        assert!(self.try_upload_mesh(data).unwrap().is_none());
        let texture_data = asset::Texture {
            role: asset::TextureRole::BaseColorSrgb,
            mips: vec![asset::Mip {
                width: 1,
                height: 1,
                rgba8: vec![128; 4],
            }],
        };
        assert!(self.try_upload_texture(&texture_data).unwrap().is_none());
        assert!(
            self.try_create_material(&MaterialDescriptor::default())
                .unwrap()
                .is_none()
        );
        assert!(
            self.try_render(color, camera, &mut [Draw::new(mesh, instances)])
                .unwrap()
                .is_none()
        );
        assert_eq!(instances.state.dirty_count, dirty);
        assert_eq!(instances.state.ids, ids);
        let after = self.stats();
        assert_eq!(before.ownership, after.ownership);
        assert_eq!(before.uploads, after.uploads);
        assert_eq!(before.submissions, after.submissions);
        assert_eq!(before.flights, after.flights);
        for slot in &self.slots {
            slot.flight.as_ref().unwrap().done.set(()).unwrap();
        }
        assert!(self.try_upload_mesh(data).unwrap().is_some());
        self.test_drain("deterministic admission boundary cleanup");
        let report = self
            .try_render(color, camera, &mut [Draw::new(mesh, instances)])
            .unwrap()
            .unwrap();
        assert_eq!(report.uploads.models, 64);
        assert_eq!(report.uploads.normals, 64);
        assert_eq!(report.uploads.model_writes, 1);
        assert_eq!(instances.state.dirty_count, 0);
    }
}
/// The engine's own pipelines from the prelude. Lit shading belongs to each program
/// (`ProgramSlot`).
struct Pipelines {
    fresh: Option<FreshPipelines>,
    /// Present exactly while traced primary visibility is selected.
    primary: Option<PrimaryPipelines>,
    shadow: Owned<wgpu::RenderPipeline>,
    diagnostic_tangent: Owned<wgpu::RenderPipeline>,
    diagnostic: Owned<wgpu::RenderPipeline>,
    present: Owned<wgpu::RenderPipeline>,
}
impl Pipelines {
    fn count(&self) -> u64 {
        4 + 2 * u64::from(self.fresh.is_some()) + self.primary.as_ref().map_or(0, |p| p.count())
    }
}
/// A lit or diagnostic mesh pipeline on the 48-byte vertex ABI, with the optional
/// tangent stream, reversed-Z depth and `samples`, through `cache` when given.
#[allow(clippy::too_many_arguments)]
fn mesh_pipeline(
    device: &wgpu::Device,
    ledger: &Ledger,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    entry: &str,
    tangent: bool,
    samples: SampleCount,
    cache: Option<&wgpu::PipelineCache>,
) -> Owned<wgpu::RenderPipeline> {
    ledger.own(
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(if tangent { "vs_tangent" } else { "vs" }),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 48,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 4,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &MODEL_ATTRIBUTES,
                    }),
                    if tangent {
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 16,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &TANGENT_ATTRIBUTES,
                        })
                    } else {
                        None
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples as u32,
                ..Default::default()
            },
            multiview_mask: None,
            cache,
        }),
        ResourceKind::RenderPipeline,
    )
}
/// A frame's visibility and lit-shading choices (`Renderer::frame_paths`).
struct FramePaths {
    lit: bool,
    hardware_shadowed: bool,
    soft: bool,
    /// Raster fresh frames: receiver raster, visibility compute, then raster shading.
    fresh: bool,
    /// Traced Interactive frames fuse camera rays and visibility into one dispatch.
    interactive: bool,
    /// Traced temporal frames: the jittered Interactive trace and film accumulation.
    temporal: bool,
    /// The program pipeline set the frame draws with; None for unlit frames.
    variant: Option<program_gpu::Variant>,
}
struct Slot {
    readback: Option<Owned<wgpu::Buffer>>,
    /// Streamed frames' request and miss counters (`texture_residency.rs`), and for the
    /// frame in flight their mapping and the residency id of each slot word.
    residency_readback: Option<Owned<wgpu::Buffer>>,
    residency: Option<(MapResult, Vec<Option<u64>>)>,
    flight: Option<Flight>,
    /// The frame in flight timed traced receivers without a visibility compute pass.
    traced_receivers: bool,
    /// The frame in flight timed a temporal reconstruction pass (timestamps 4 and 5).
    reconstructed: bool,
}
struct Timing {
    queries: Owned<wgpu::QuerySet>,
    resolve: Owned<wgpu::Buffer>,
    period: f32,
}
struct Depth {
    texture: Owned<wgpu::Texture>,
    view: Owned<wgpu::TextureView>,
    _allocation: Allocation,
}

/// One queue submission owner, three flights, and extent-matched depth/MSAA color targets.
/// Use an sRGB output format for encoded base color; linear targets retain linear values.
/// Alternating target sizes recreates attachments. Separate renderers suit independent streams.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    options: RendererOptions,
    limits: Limits,
    fault: Arc<OnceLock<Fault>>,
    ledger: Ledger,
    layout: Owned<wgpu::PipelineLayout>,
    _bindings: Owned<wgpu::BindGroupLayout>,
    bind: Owned<wgpu::BindGroup>,
    uniform: Owned<wgpu::Buffer>,
    instance_bindings: Owned<wgpu::BindGroupLayout>,
    material_bindings: Owned<wgpu::BindGroupLayout>,
    default_material: Option<Material>,
    identity: Arc<()>,
    #[cfg(test)]
    reference_draws: bool,
    #[cfg(test)]
    fresh_fragment_reference: bool,
    pipeline: Pipelines,
    shader_hash: [u8; 32],
    lighting: LightingGpu,
    post_settings: PostSettings,
    post_lut: Option<ColourLut>,
    post: Option<PostGpu>,
    retired_post: Vec<(u64, PostGpu)>,
    environment: crate::environment::EnvironmentState,
    shadows: ShadowState,
    hardware_shadows: Option<HardwareState>,
    fresh: Option<FreshState>,
    /// Exists on hardware-shadow 1x renderers, which can trace primary visibility.
    primary: Option<PrimaryState>,
    primary_mode: PrimaryVisibility,
    visibility_buffer: Option<visibility_buffer::VisibilityBuffer>,
    cluster_instances: visibility_buffer::ClusterInstanceCache,
    /// N1c cut error bound and software-raster cluster extent, in pixels.
    lod_pixels: f32,
    software_extent: f32,
    lod_lift: f32,
    soft_mode: crate::SoftShadows,
    /// `SoftShadows::Virtual` state, created by its first frame (N8).
    virtual_shadows: Option<virtual_shadow_gpu::VirtualState>,
    virtual_settings: VirtualShadowSettings,
    /// Identity of the traced modules, which also produce the HDR of traced frames.
    primary_hash: [u8; 32],
    /// Last installed prelude WGSL, composed again when traced visibility is selected.
    source: String,
    /// Its hardware-shadow and fresh compositions, where this renderer has those paths.
    hardware_source: Option<String>,
    fresh_source: Option<String>,
    /// While traced is selected: the kernel modules and the program shading heads.
    primary_sources: Option<([String; 2], [String; 2])>,
    /// Shading programs in creation order; the first is the standard program.
    programs: Vec<ProgramSlot>,
    /// The program pipeline sets the last frame drew with, which reloads recreate
    /// eagerly; other sets compile when a frame first needs them.
    hot: Vec<(u64, program_gpu::Variant)>,
    /// Program pipelines go through this driver cache when the device enables
    /// `Features::PIPELINE_CACHE`, so recompiling a source seen before (a revert, an A/B
    /// toggle) is a cache hit; see `program_gpu::ProgramCache`.
    program_cache: program_gpu::ProgramCache,
    standard: crate::Program,
    next_program: u64,
    depth: Option<Depth>,
    msaa: Option<Depth>,
    view_mode: ViewMode,
    camera_shadow: Option<[u8; 64]>,
    scratch: Vec<u8>,
    slots: [Slot; FLIGHTS],
    timing: Option<Timing>,
    timing_status: TimingStatus,
    timing_failures: u64,
    timed_frames: u64,
    gpu_samples: u64,
    events: EventQueue,
    revision: Revision,
    pipelines: PipelineStats,
    uploads: UploadBytes,
    submissions: u64,
    completed_submission: u64,
    frames: u64,
    peak_flights: usize,
    busy_uploads: u64,
    busy_frames: u64,
    upload_cpu: CpuFrame,
    /// Streamed textures' resident levels and requests (`texture_residency.rs`).
    residency: texture_residency::Residency,
    /// M7 world radiance cache (`gi_cache.rs`).
    gi: gi_cache::GiState,
    /// AT1 sky, aerial perspective and fog (`atmosphere_gpu.rs`).
    atmosphere: atmosphere_gpu::AtmosphereGpu,
    /// TR1 temporal reconstruction settings, its history (created by its first frame), the
    /// next frame's sample index, and the index of the frame being prepared while that frame
    /// reconstructs (`temporal_reconstruction.rs`).
    reconstruction: Reconstruction,
    reconstruction_shadow_budget: ReconstructionShadowBudget,
    reconstruction_state: Option<ReconstructionState>,
    reconstruction_index: u64,
    reconstruction_frame: Option<u64>,
    /// TR1b upscaling: the scaled frame's internal colour target and output HDR.
    upscale: Option<temporal_reconstruction::Upscale>,
    /// TP1 transparent draws and particle emitters (`transparency_gpu.rs`).
    transparency: transparency_gpu::TransparencyState,
}

// Native wgpu scope futures are immediately ready. This never polls for GPU completion.
pub(crate) struct OperationScopes([wgpu::ErrorScopeGuard; 3]);
impl OperationScopes {
    fn new(device: &wgpu::Device) -> Self {
        Self([
            device.push_error_scope(wgpu::ErrorFilter::Validation),
            device.push_error_scope(wgpu::ErrorFilter::OutOfMemory),
            device.push_error_scope(wgpu::ErrorFilter::Internal),
        ])
    }
    fn finish(self) -> Option<wgpu::Error> {
        let [validation, oom, internal] = self.0;
        let internal = pollster::block_on(internal.pop());
        let oom = pollster::block_on(oom.pop());
        let validation = pollster::block_on(validation.pop());
        internal.or(oom).or(validation)
    }
}

impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        limits: Limits,
        wgsl: &str,
    ) -> Result<Self, Error> {
        Self::new_with_options(
            device,
            queue,
            color_format,
            limits,
            RendererOptions::default(),
            wgsl,
        )
    }
    pub fn new_with_options(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        limits: Limits,
        options: RendererOptions,
        wgsl: &str,
    ) -> Result<Self, Error> {
        Self::new_configured(device, queue, color_format, limits, options, None, wgsl)
    }
    /// Explicit optional hard shadows. The caller must negotiate the experimental
    /// device feature and its unsafe acknowledgment before calling this safe API.
    /// 4x needs sample-rate shading support; shader/pipeline validation rejects its
    /// absence transactionally. Device-feature/AS-limit checks precede allocations.
    pub fn new_with_hardware_shadows(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        limits: Limits,
        options: RendererOptions,
        hardware: HardwareShadowOptions,
        wgsl: &str,
    ) -> Result<Self, Error> {
        Self::new_configured(
            device,
            queue,
            color_format,
            limits,
            options,
            Some(hardware),
            wgsl,
        )
    }
    fn new_configured(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        limits: Limits,
        options: RendererOptions,
        hardware: Option<HardwareShadowOptions>,
        wgsl: &str,
    ) -> Result<Self, Error> {
        if !device.features().contains(crate::RECEIVER_FEATURES) {
            return Err(Error::InvalidInput(
                "receiver policy requires caller-enabled RECEIVER_FEATURES",
            ));
        }
        if let Some(hardware) = hardware {
            hardware.validate(device.features(), &device.limits())?;
        }
        let hardware_source = hardware
            .map(|_| crate::hardware_shadows::shader_source(wgsl, options.samples))
            .transpose()?;
        let fresh_source = (hardware.is_some() && options.samples == SampleCount::One)
            .then(|| fresh_shadow_gpu::shader_source(wgsl))
            .transpose()?;
        if !(1..=16).contains(&options.anisotropy_clamp) {
            return Err(Error::InvalidInput("anisotropy_clamp must be in 1..=16"));
        }
        for format in [
            color_format,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureFormat::Rgba16Float,
        ] {
            let features = format.guaranteed_format_features(device.features());
            if !features
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                || !features
                    .flags
                    .sample_count_supported(options.samples as u32)
            {
                return Err(Error::InvalidInput(
                    "unsupported render target format/sample configuration",
                ));
            }
        }
        if options.samples == SampleCount::Four
            && [color_format, wgpu::TextureFormat::Rgba16Float]
                .into_iter()
                .any(|f| {
                    !f.guaranteed_format_features(device.features())
                        .flags
                        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
                })
        {
            return Err(Error::InvalidInput(
                "color format does not support multisample resolve",
            ));
        }
        let fault = Arc::new(OnceLock::new());
        let errors = fault.clone();
        device.on_uncaptured_error(Arc::new(move |error| {
            let _ = errors.set(Fault(format!("{error}")));
        }));
        let lost = fault.clone();
        device.set_device_lost_callback(move |reason, message| {
            let _ = lost.set(Fault(format!("device lost ({reason:?}): {message}")));
        });
        limits.validate(&device.limits())?;
        let device_limits = device.limits();
        if device_limits.max_texture_array_layers < 6
            || device_limits.max_bind_groups < 4
            || device_limits.max_storage_buffers_per_shader_stage < 2
            || device_limits.max_sampled_textures_per_shader_stage < 8
            || device_limits.max_uniform_buffer_binding_size < crate::shadows::VIEW_BYTES
        {
            return Err(Error::InvalidInput(
                "device does not support the lighting/shadow binding ABI",
            ));
        }
        let ledger = Ledger::default();
        let scopes = OperationScopes::new(&device);
        let uniform = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("camera"),
                size: 80,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fixed camera ABI"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(80),
                    },
                    count: None,
                }],
            }),
            ResourceKind::BindGroupLayout,
        );
        let bind = ledger.own(
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("camera"),
                layout: &bindings,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            }),
            ResourceKind::BindGroup,
        );
        let instance_bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fixed instance ABI"),
                entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                }),
            }),
            ResourceKind::BindGroupLayout,
        );
        // Compute visibility lets traced primary shading bind each material unchanged.
        let material_stages = wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: material_stages,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(48),
            },
            count: None,
        }];
        for binding in [1, 3, 5] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: material_stages,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: if binding == 5 {
                        wgpu::TextureViewDimension::D2Array
                    } else {
                        wgpu::TextureViewDimension::D2
                    },
                    multisampled: false,
                },
                count: None,
            });
        }
        for binding in [2, 4, 6] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: material_stages,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        // The material's program parameter block, read only by programs declaring one.
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: material_stages,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(crate::program::PARAMETER_BYTES as u64),
            },
            count: None,
        });
        let material_bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fixed material ABI"),
                entries: &entries,
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fixed mesh ABI"),
                bind_group_layouts: &[
                    Some(&bindings),
                    Some(&instance_bindings),
                    Some(&material_bindings),
                ],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let lighting = LightingGpu::new(
            &device,
            &queue,
            &ledger,
            [&bindings, &instance_bindings, &material_bindings],
        );
        let shadows = ShadowState::new(&device, &ledger, &instance_bindings, &material_bindings);
        let gi = gi_cache::GiState::new(&device, &ledger);
        let atmosphere = atmosphere_gpu::AtmosphereGpu::new(&device, &ledger);
        // Traced visibility reads hit-triangle positions back from the shared AS.
        let traced = hardware.is_some()
            && options.samples == SampleCount::One
            && primary_ray_gpu::supported(&device);
        let hardware_shadows = hardware.map(|options| {
            HardwareState::new(
                &device,
                &ledger,
                [&bindings, &instance_bindings, &material_bindings],
                &lighting.bindings,
                options,
                traced,
            )
        });
        let raster_primary =
            options.samples == SampleCount::One && visibility_buffer::supported(&device);
        let fresh = ((hardware.is_some() || raster_primary) && options.samples == SampleCount::One)
            .then(|| {
                FreshState::new(
                    &device,
                    &ledger,
                    [
                        &bindings,
                        &instance_bindings,
                        &material_bindings,
                        &lighting.bindings,
                    ],
                )
            });
        let primary = (traced || raster_primary)
            .then(|| PrimaryState::new(&device, &ledger, &lighting.bindings));
        let enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let timing = enabled.then(|| Timing {
            queries: ledger.own(
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("three flights"),
                    ty: wgpu::QueryType::Timestamp,
                    count: (FLIGHTS * 10) as u32,
                }),
                ResourceKind::QuerySet,
            ),
            resolve: ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("aligned timestamp resolve"),
                    size: FLIGHTS as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            period: queue.get_timestamp_period(),
        });
        let slots = std::array::from_fn(|_| Slot {
            readback: enabled.then(|| {
                ledger.own(
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("flight timestamps"),
                        size: 80,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ResourceKind::Buffer,
                )
            }),
            residency_readback: None,
            residency: None,
            flight: None,
            traced_receivers: false,
            reconstructed: false,
        });
        if let Some(error) = scopes.finish() {
            return Err(Error::Fault(Fault(error.to_string())));
        }
        let scopes = OperationScopes::new(&device);
        let pipeline = Self::compile(
            &device,
            &ledger,
            &layout,
            &lighting,
            &shadows,
            fresh.as_ref().zip(fresh_source.as_deref()),
            None,
            color_format,
            options.samples,
            wgsl,
        );
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => Error::Fault(Fault(error.to_string())),
            });
        }
        let identity = Arc::new(());
        let program_cache = program_gpu::ProgramCache::new(&device);
        let standard = program_gpu::compile_program(
            &ProgramAbi {
                device: &device,
                ledger: &ledger,
                samples: options.samples,
                lit: &lighting.layout,
                hardware: hardware_shadows.as_ref().map(|h| &*h.layout),
                fresh: fresh.as_ref(),
                primary: None,
                cache: None,
                modules: &program_cache.modules,
                vertex_identity: Sha256::digest(wgsl.as_bytes()).into(),
            },
            &Preludes {
                raster: wgsl,
                hardware: hardware_source.as_deref(),
                fresh: fresh_source.as_deref(),
                traced: None,
            },
            "standard",
            crate::STANDARD_PROGRAM,
        )?;
        if let Some(error) = fault.get() {
            return Err(Error::Fault(error.clone()));
        }
        // The standard program has modules only; its pipelines compile at first draw.
        let created = pipeline.count();
        let (standard, slot) = program_gpu::standard_slot(&identity, standard);
        let mut renderer = Self {
            device,
            queue,
            format: color_format,
            options,
            limits,
            fault,
            ledger,
            layout,
            _bindings: bindings,
            bind,
            uniform,
            instance_bindings,
            material_bindings,
            default_material: None,
            identity,
            #[cfg(test)]
            reference_draws: false,
            #[cfg(test)]
            fresh_fragment_reference: false,
            pipeline,
            shader_hash: [0; 32],
            lighting,
            post_settings: PostSettings::default(),
            post_lut: None,
            post: None,
            retired_post: vec![],
            environment: Default::default(),
            shadows,
            hardware_shadows,
            fresh,
            primary,
            primary_mode: PrimaryVisibility::Raster,
            visibility_buffer: None,
            cluster_instances: Default::default(),
            lod_pixels: visibility_buffer::DEFAULT_LOD_PIXELS,
            software_extent: visibility_buffer::DEFAULT_SOFTWARE_EXTENT,
            lod_lift: visibility_buffer::DEFAULT_LOD_LIFT,
            soft_mode: crate::SoftShadows::Exact,
            virtual_shadows: None,
            virtual_settings: VirtualShadowSettings::default(),
            atmosphere,
            gi,
            reconstruction: Reconstruction::default(),
            reconstruction_shadow_budget: ReconstructionShadowBudget::default(),
            reconstruction_state: None,
            reconstruction_index: 0,
            reconstruction_frame: None,
            upscale: None,
            transparency: Default::default(),
            primary_hash: [0; 32],
            source: wgsl.to_owned(),
            hardware_source,
            fresh_source,
            primary_sources: None,
            programs: vec![slot],
            hot: vec![],
            program_cache,
            standard,
            next_program: 1,
            depth: None,
            msaa: None,
            view_mode: ViewMode::BaseColor,
            camera_shadow: None,
            scratch: Vec::new(),
            slots,
            timing,
            timing_status: if enabled {
                TimingStatus::Enabled
            } else {
                TimingStatus::Unavailable
            },
            timing_failures: 0,
            timed_frames: 0,
            gpu_samples: 0,
            events: EventQueue::default(),
            revision: Revision(1),
            pipelines: PipelineStats {
                attempts: 1,
                successes: 1,
                installations: 1,
                created,
            },
            uploads: UploadBytes {
                lut: crate::lighting::DFG.len() as u64,
                environment: 48,
                write_calls: 2,
                ..Default::default()
            },
            submissions: 0,
            completed_submission: 0,
            frames: 0,
            peak_flights: 0,
            busy_uploads: 0,
            busy_frames: 0,
            upload_cpu: CpuFrame::default(),
            residency: Default::default(),
        };
        renderer.update_hashes();
        // The initial defaults share one immediately submitted flight.
        let scopes = OperationScopes::new(&renderer.device);
        let defaults: [Texture; 3] = [
            asset::TextureRole::BaseColorSrgb,
            asset::TextureRole::MetallicRoughnessLinear,
            asset::TextureRole::NormalLinear,
        ]
        .map(|role| {
            renderer.create_texture(
                &asset::Texture {
                    role,
                    mips: vec![asset::Mip {
                        width: 1,
                        height: 1,
                        rgba8: if role == asset::TextureRole::NormalLinear {
                            vec![128, 128, 255, 255]
                        } else {
                            vec![255; 4]
                        },
                    }],
                },
                4,
            )
        });
        let material = renderer.create_material(
            &MaterialDescriptor {
                double_sided: true,
                ..Default::default()
            },
            &defaults,
        );
        if let Some(error) = scopes.finish() {
            return Err(renderer.fail(error.to_string()));
        }
        renderer.submit_upload(renderer.free_slot().unwrap());
        renderer.default_material = Some(material);
        renderer.check_fault()?;
        if traced {
            // RTDEF2: traced visibility enables TR1 and traced reflections. TR1's
            // 256/64/32/8 engine budget keeps cold/moving disk shadows at the retained floor.
            renderer.set_primary_visibility(PrimaryVisibility::Traced)?;
            renderer.set_soft_shadows(crate::SoftShadows::Interactive);
        } else if hardware.is_none()
            && raster_primary
            && renderer.device.features().contains(VIRTUAL_FEATURES)
        {
            renderer.set_primary_visibility(PrimaryVisibility::VisibilityBuffer)?;
            renderer.set_soft_shadows(crate::SoftShadows::Virtual);
        }
        Ok(renderer)
    }

    /// The engine's pipelines from the prelude `source` and its fresh composition and
    /// traced kernel modules; programs compile separately (`program_gpu`).
    #[allow(clippy::too_many_arguments)]
    fn compile(
        device: &wgpu::Device,
        ledger: &Ledger,
        layout: &wgpu::PipelineLayout,
        lighting: &LightingGpu,
        shadows: &ShadowState,
        fresh: Option<(&FreshState, &str)>,
        primary: Option<(&PrimaryState, &[String; 2])>,
        format: wgpu::TextureFormat,
        samples: SampleCount,
        source: &str,
    ) -> Pipelines {
        let shader = ledger.own(
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("live mesh shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }),
            ResourceKind::ShaderModule,
        );
        let mesh = |tangent| {
            mesh_pipeline(
                device, ledger, &shader, layout, format, "fs", tangent, samples, None,
            )
        };
        let diagnostic = mesh(false);
        let diagnostic_tangent = mesh(true);
        let shadow = ledger.own(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("source-sided shadow depth"),
                layout: Some(&shadows.layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 48,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_shadow"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        let present = ledger.own(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("HDR presentation"),
                layout: Some(&lighting.present_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_present"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_present"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        Pipelines {
            fresh: fresh.map(|(f, source)| f.compile(device, ledger, source)),
            primary: primary.map(|(p, sources)| p.compile(device, ledger, sources)),
            shadow,
            diagnostic_tangent,
            diagnostic,
            present,
        }
    }

    pub(crate) fn check_fault(&self) -> Result<(), Error> {
        match self.fault.get() {
            Some(fault) => Err(Error::Fault(fault.clone())),
            None => Ok(()),
        }
    }
    pub(crate) fn fail(&self, message: String) -> Error {
        let _ = self.fault.set(Fault(message));
        Error::Fault(self.fault.get().unwrap().clone())
    }

    /// Replaces the engine prelude (shaders/scene.wgsl): one transaction over the engine
    /// pipelines and every program recomposed against it, each program's module created
    /// for every path. Program pipeline sets the last frame drew with are recreated in
    /// the transaction, concurrently with the engine pipelines; the others compile when a
    /// frame first needs them. Any rejection keeps them all.
    pub fn reload(&mut self, wgsl: &str) -> Result<ReloadReport, ReloadError> {
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        let start = Instant::now();
        self.pipelines.attempts += 1;
        self.purge_programs();
        self.program_cache.refresh(&self.device);
        let rejected = |error: Error| ReloadError::Rejected {
            diagnostic: error.to_string(),
            compile: start.elapsed(),
        };
        let hardware_source = self
            .hardware_shadows
            .as_ref()
            .map(|_| crate::hardware_shadows::shader_source(wgsl, self.options.samples))
            .transpose()
            .map_err(rejected)?;
        let fresh_source = self
            .fresh
            .as_ref()
            .filter(|_| self.hardware_shadows.is_some())
            .map(|_| fresh_shadow_gpu::shader_source(wgsl))
            .transpose()
            .map_err(rejected)?;
        // Traced modules are part of the same transaction while they are selected.
        let primary_sources = self
            .pipeline
            .primary
            .as_ref()
            .map(|_| {
                if self.hardware_shadows.is_none() {
                    return primary_ray_gpu::non_rt_sources(wgsl);
                }
                Ok::<_, Error>((
                    primary_ray_gpu::shader_sources(wgsl)?,
                    primary_ray_gpu::shading_heads(wgsl)?,
                ))
            })
            .transpose()
            .map_err(rejected)?;
        // A temporal kernel already in use recompiles in the transaction, so a rejected
        // prelude keeps it; otherwise the first temporal frame compiles it.
        let film_source = primary_sources
            .as_ref()
            .filter(|_| {
                self.pipeline
                    .primary
                    .as_ref()
                    .is_some_and(|p| p.film.is_some())
            })
            .map(|(kernels, _)| primary_ray_gpu::film_accumulation::kernel_source(&kernels[1]))
            .transpose()
            .map_err(rejected)?;
        let preludes = Preludes {
            raster: wgsl,
            hardware: hardware_source.as_deref(),
            fresh: fresh_source.as_deref(),
            traced: primary_sources.as_ref().map(|(_, heads)| heads),
        };
        let mut abi = self.program_abi();
        abi.vertex_identity = Sha256::digest(wgsl.as_bytes()).into();
        // The engine pipelines compile on their own thread while every program's modules
        // and hot sets (the sets the last frame drew with) compile concurrently here.
        let (device, ledger, layout, lighting, shadows) = (
            &self.device,
            &self.ledger,
            &*self.layout,
            &self.lighting,
            &self.shadows,
        );
        let fresh = self.fresh.as_ref().zip(fresh_source.as_deref());
        let primary = self
            .primary
            .as_ref()
            .zip(primary_sources.as_ref().map(|(k, _)| k));
        let (format, samples) = (self.format, self.options.samples);
        let ((candidate, engine_error), programs) = std::thread::scope(|s| {
            let film_source = film_source.as_deref();
            let engine = s.spawn(move || {
                let scopes = OperationScopes::new(device);
                let mut candidate = Self::compile(
                    device, ledger, layout, lighting, shadows, fresh, primary, format, samples,
                    wgsl,
                );
                if let (Some(source), Some((state, _)), Some(p)) =
                    (film_source, primary, candidate.primary.as_mut())
                {
                    p.film = Some(state.film_kernel(device, ledger, source));
                }
                (candidate, scopes.finish())
            });
            let programs = self
                .programs
                .iter()
                .map(|slot| {
                    program_gpu::compile_program(&abi, &preludes, &slot.label, &slot.source)
                })
                .collect::<Result<Vec<_>, _>>()
                .and_then(|compiled| {
                    let sets: Vec<_> = self
                        .programs
                        .iter()
                        .zip(&compiled)
                        .enumerate()
                        .flat_map(|(i, (slot, c))| {
                            self.hot_sets(slot.id, &c.modules)
                                .map(move |v| (i, (&c.modules, slot.label.as_str(), v)))
                        })
                        .collect();
                    let requests: Vec<_> = sets.iter().map(|(_, request)| *request).collect();
                    let built = program_gpu::create_sets(&abi, &requests)?;
                    let keys: Vec<_> = sets.iter().map(|&(i, (_, _, v))| (i, v)).collect();
                    Ok((keys, built, compiled))
                });
            (engine.join().unwrap(), programs)
        });
        if let Some(error) = engine_error {
            return match error {
                wgpu::Error::Validation { .. } => Err(ReloadError::Rejected {
                    diagnostic: error.to_string(),
                    compile: start.elapsed(),
                }),
                _ => {
                    let _ = self.fault.set(Fault(error.to_string()));
                    Err(ReloadError::Fault(self.fault.get().unwrap().clone()))
                }
            };
        }
        let (keys, built, compiled) = match programs {
            Ok(programs) => programs,
            Err(Error::Fault(Fault(message))) => {
                let _ = self.fault.set(Fault(message));
                return Err(ReloadError::Fault(self.fault.get().unwrap().clone()));
            }
            Err(error) => return Err(rejected(error)),
        };
        let compile = start.elapsed();
        self.pipelines.successes += 1;
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        self.pipelines.created += candidate.count()
            + program_gpu::pipeline_count(&built)
            + 2 * u64::from(film_source.is_some());
        self.program_cache
            .count(program_gpu::pipeline_count(&built));
        self.pipeline = candidate;
        for (slot, c) in self.programs.iter_mut().zip(compiled) {
            slot.install_parameters(c.layout, &self.queue, &mut self.uploads);
            slot.modules = c.modules;
            slot.preview = false;
            slot.pipelines = Default::default();
        }
        for ((i, variant), pair) in keys.into_iter().zip(built) {
            self.programs[i].pipelines.install(variant, pair);
        }
        wgsl.clone_into(&mut self.source);
        self.visibility_buffer = None;
        self.hardware_source = hardware_source;
        self.fresh_source = fresh_source;
        self.primary_sources = primary_sources;
        self.update_hashes();
        self.revision.0 += 1;
        self.pipelines.installations += 1;
        Ok(ReloadReport {
            revision: self.revision,
            compile,
            zeroed_parameters: vec![],
        })
    }

    pub(crate) fn occupied(&self) -> usize {
        self.slots.iter().filter(|s| s.flight.is_some()).count()
    }
    pub(crate) fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.flight.is_none())
    }
    pub(crate) fn record_flight(&mut self, slot: usize, flight: Flight) {
        self.slots[slot].flight = Some(flight);
        self.peak_flights = self.peak_flights.max(self.occupied());
    }

    /// None means real backpressure after a nonblocking collection. Retry on a later wake.
    /// Every admitted upload submits immediately, including callers that never render.
    pub fn try_upload_mesh(&mut self, data: &MeshData<'_>) -> Result<Option<Mesh>, Error> {
        self.try_upload_mesh_with_tangents(data, None)
    }
    /// Upload an optional validated tangent stream in the base mesh's exact vertex domain.
    pub fn try_upload_mesh_with_tangents(
        &mut self,
        data: &MeshData<'_>,
        tangents: Option<&asset::TangentStream>,
    ) -> Result<Option<Mesh>, Error> {
        self.try_upload_mesh_prepared(data, tangents, None, false)
    }

    /// Validate and prepare owned primitives concurrently without submitting GPU
    /// work. The returned order matches the input. Only traced-eligible meshes
    /// allocate adjacency; source vectors move into immutable upload resources.
    pub fn prepare_mesh_uploads(
        &self,
        primitives: Vec<asset::Primitive>,
    ) -> Result<Vec<MeshUpload>, Error> {
        self.check_fault()?;
        #[cfg(test)]
        if crate::receiver_adjacency::LEGACY.with(std::cell::Cell::get) {
            return Ok(crate::receiver_adjacency::prepare_uploads(
                primitives,
                vec![],
            ));
        }
        let eligible = primitives
            .iter()
            .map(|p| {
                let data = MeshData {
                    vertices: &p.vertices,
                    indices: &p.indices,
                };
                let sizes = data.validate(self.limits)?;
                let tangent_bytes =
                    self.validate_mesh_tangents(&data, p.tangents.as_ref(), sizes)?;
                if let Some(hardware) = &self.hardware_shadows {
                    hardware.validate_mesh(
                        &self.ledger,
                        &self.device.limits(),
                        sizes.0,
                        sizes.1,
                    )?;
                }
                Ok(self.receiver_adjacency_fits(sizes, tangent_bytes))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(crate::receiver_adjacency::prepare_uploads(
            primitives, eligible,
        ))
    }

    /// Busy leaves the prepared resource intact for retry. Preparation may also
    /// be used with another renderer; its limits are checked again at upload.
    pub fn try_upload_prepared_mesh(
        &mut self,
        prepared: &MeshUpload,
    ) -> Result<Option<Mesh>, Error> {
        let p = &prepared.primitive;
        self.try_upload_mesh_prepared(
            &MeshData {
                vertices: &p.vertices,
                indices: &p.indices,
            },
            p.tangents.as_ref(),
            Some(prepared),
            false,
        )
    }

    fn receiver_adjacency_fits(&self, sizes: (u64, u64), tangent_bytes: u64) -> bool {
        let adjacency_bytes = sizes.1 + 4;
        self.primary.is_some()
            && sizes.1 + adjacency_bytes <= self.device.limits().max_storage_buffer_binding_size
            && sizes.1 + adjacency_bytes <= self.device.limits().max_buffer_size
            && sizes.0 + sizes.1 + tangent_bytes + adjacency_bytes <= self.limits.max_mesh_bytes
    }

    fn try_upload_mesh_prepared(
        &mut self,
        data: &MeshData<'_>,
        tangents: Option<&asset::TangentStream>,
        prepared: Option<&MeshUpload>,
        skinned: bool,
    ) -> Result<Option<Mesh>, Error> {
        self.check_fault()?;
        let started = Instant::now();
        let sizes = data.validate(self.limits)?;
        let tangent_bytes = self.validate_mesh_tangents(data, tangents, sizes)?;
        let mut preparation = started.elapsed();
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let pack_start = Instant::now();
        if let Some(hardware) = &self.hardware_shadows {
            hardware.validate_mesh(&self.ledger, &self.device.limits(), sizes.0, sizes.1)?;
        }
        // Keep source indices first, followed by three neighbours per triangle and
        // one padding word. The suffix's length modulo three identifies its presence
        // to traced kernels; raster draws and BLASes retain the source index count.
        let adjacency_bytes = sizes.1 + 4;
        let adjacency_fits = self.receiver_adjacency_fits(sizes, tangent_bytes);
        let adjacency_started = Instant::now();
        let prepared = prepared
            .and_then(|p| p.adjacency.as_ref())
            .filter(|_| adjacency_fits);
        let built =
            (adjacency_fits && prepared.is_none()).then(|| crate::receiver_adjacency::build(data));
        let adjacency = prepared.or(built.as_ref());
        let adjacency_build = adjacency_started.elapsed();
        let adjacency_bytes = if adjacency.is_some() {
            adjacency_bytes
        } else {
            0
        };
        let index_bytes = sizes.1 + adjacency_bytes;
        let mut bytes = Vec::with_capacity((sizes.0 + sizes.1 + tangent_bytes) as usize);
        for vertex in data.vertices {
            let normal = glam::DVec3::new(
                vertex.normal[0].into(),
                vertex.normal[1].into(),
                vertex.normal[2].into(),
            )
            .normalize()
            .as_vec3()
            .to_array();
            for value in vertex
                .position
                .into_iter()
                .chain(normal)
                .chain(vertex.uv)
                .chain(vertex.color)
            {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        for index in data.indices {
            bytes.extend_from_slice(&index.to_ne_bytes());
        }
        if let Some(t) = tangents {
            for value in t.values.iter().flatten() {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        let bounds = Aabb::from_positions(data.vertices.iter().map(|v| v.position)).unwrap();
        preparation += pack_start.elapsed();
        let start = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        // Traced shading reads the same buffers as storage; no second geometry copy.
        let traced = if self.primary.is_some() || skinned {
            wgpu::BufferUsages::STORAGE
        } else {
            wgpu::BufferUsages::empty()
        };
        let blas = if self.hardware_shadows.is_some() {
            wgpu::BufferUsages::BLAS_INPUT
        } else {
            wgpu::BufferUsages::empty()
        };
        let vertex = Arc::new(self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh vertices"),
                size: sizes.0,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC
                    | blas
                    | traced,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        ));
        let index = Arc::new(self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh indices"),
                size: index_bytes,
                // COPY_SRC: attaching a cluster LOD hierarchy copies the source indices.
                usage: wgpu::BufferUsages::INDEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC
                    | blas
                    | traced,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        ));
        let tangent = tangents.map(|_| {
            self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mesh tangents"),
                    size: tangent_bytes,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | traced,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        });
        let acceleration = self.hardware_shadows.as_mut().map(|hardware| {
            hardware.create_mesh(
                &self.device,
                &self.ledger,
                &vertex,
                &index,
                data.indices.len() as u32,
                adjacency_bytes,
                skinned,
            )
        });
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.check_fault()?;
        self.queue
            .write_buffer(&vertex, 0, &bytes[..sizes.0 as usize]);
        self.uploads.geometry += sizes.0;
        self.uploads.write_calls += 1;
        self.queue.write_buffer(
            &index,
            0,
            &bytes[sizes.0 as usize..(sizes.0 + sizes.1) as usize],
        );
        if let Some(adjacency) = adjacency {
            let started = Instant::now();
            // Fill staging directly: no second table-sized byte Vec or growth
            // when appending the packed-suffix padding word.
            {
                let mut suffix = self
                    .queue
                    .write_buffer_with(
                        &index,
                        sizes.1,
                        std::num::NonZeroU64::new(adjacency_bytes).unwrap(),
                    )
                    .unwrap();
                let (words, _) = suffix.slice(..).into_chunks::<4>();
                words.write_iter(
                    adjacency
                        .neighbours
                        .iter()
                        .copied()
                        .chain([0])
                        .map(u32::to_ne_bytes),
                );
            }
            let adjacency_upload = started.elapsed();
            self.uploads.geometry += adjacency_bytes;
            self.uploads.write_calls += 1;
            #[cfg(test)]
            eprintln!(
                "receiver_adjacency triangles={} bytes={} boundary_halfedges={} ambiguous_halfedges={} degenerate_halfedges={} build_ms={:.6} upload_call_ms={:.6}",
                data.indices.len() / 3,
                adjacency_bytes,
                adjacency.boundary_halfedges,
                adjacency.ambiguous_halfedges,
                adjacency.degenerate_halfedges,
                adjacency_build.as_secs_f64() * 1000.,
                adjacency_upload.as_secs_f64() * 1000.
            );
            let _ = (
                adjacency_build,
                adjacency_upload,
                adjacency.boundary_halfedges,
                adjacency.ambiguous_halfedges,
                adjacency.degenerate_halfedges,
            );
        }
        if let Some(t) = &tangent {
            self.queue
                .write_buffer(t, 0, &bytes[(sizes.0 + sizes.1) as usize..]);
            self.uploads.geometry += tangent_bytes;
            self.uploads.write_calls += 1;
        }
        self.uploads.geometry += sizes.1;
        self.uploads.write_calls += 1;
        let upload = start.elapsed();
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        let submitted_at = Instant::now();
        if let Some(acceleration) = &acceleration {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            acceleration.encode(&mut encoder);
            self.queue.submit([encoder.finish()]);
        } else {
            self.queue.submit([]);
        }
        self.queue.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        self.submissions += 1;
        if let (Some(hardware), Some(acceleration)) = (&mut self.hardware_shadows, &acceleration) {
            hardware.mesh_submitted(
                &self.device,
                &self.ledger,
                acceleration.clone(),
                self.submissions,
            );
        }
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
        self.upload_cpu = CpuFrame {
            preparation,
            poll,
            upload,
            submit: submitted_at.elapsed(),
            ..CpuFrame::default()
        };
        self.check_fault()?;
        let clusters =
            (self.primary.is_some() && visibility_buffer::supported(&self.device)).then(|| {
                let b = visibility_buffer::source_clusters(data);
                self.upload_clusters(b.clusters, &b.words, vec![[0., 0., 0., 0., f32::MAX]])
            });
        Ok(Some(Mesh {
            skin: None,
            clusters,
            repair_scan: adjacency.is_none_or(|a| a.ambiguous_halfedges != 0),
            acceleration,
            tangent,
            renderer: self.identity.clone(),
            vertex,
            index,
            index_count: data.indices.len() as u32,
            bounds,
        }))
    }

    pub(crate) fn validate_mesh_tangents(
        &self,
        data: &MeshData<'_>,
        tangents: Option<&asset::TangentStream>,
        sizes: (u64, u64),
    ) -> Result<u64, Error> {
        let bytes = if let Some(t) = tangents {
            asset::validate_tangents(data.vertices, data.indices, t)
                .map_err(|e| Error::InvalidInput(e.0))?;
            (t.values.len() as u64)
                .checked_mul(16)
                .ok_or(Error::InvalidInput("tangent byte overflow"))?
        } else {
            0
        };
        if sizes
            .0
            .checked_add(sizes.1)
            .and_then(|n| n.checked_add(bytes))
            .is_none_or(|n| n > self.limits.max_mesh_bytes)
        {
            return Err(Error::InvalidInput("tangent mesh exceeds max_mesh_bytes"));
        }
        Ok(bytes)
    }

    /// Creation follows mesh admission: Busy allocates nothing, accepted uploads submit immediately.
    pub fn try_upload_instances(&mut self, models: &[Mat4]) -> Result<Option<Instances>, Error> {
        self.check_fault()?;
        let started = Instant::now();
        InstanceState::validate(models, self.limits.max_instances)?;
        let preparation = started.elapsed();
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let packing_start = Instant::now();
        let state = InstanceState::new(models);
        self.scratch.clear();
        for &model in models {
            self.scratch.extend_from_slice(&matrix_bytes(model));
        }
        let packing = packing_start.elapsed();
        let upload_start = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        let transforms = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instance transforms"),
                size: models.len() as u64 * 64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let normals = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("normal transforms"),
                size: models.len() as u64 * 64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let id_buffer = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("visible instance IDs"),
                size: models.len() as u64 * 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let bind = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("instances"),
                layout: &self.instance_bindings,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: transforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: normals.as_entire_binding(),
                    },
                ],
            }),
            ResourceKind::BindGroup,
        );
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.check_fault()?;
        self.queue.write_buffer(&transforms, 0, &self.scratch);
        let mut upload = upload_start.elapsed();
        let pack_start = Instant::now();
        self.scratch.clear();
        for normal in &state.normals {
            self.scratch.extend_from_slice(&normal.bytes());
        }
        let packing = packing + pack_start.elapsed();
        let upload_start = Instant::now();
        self.queue.write_buffer(&normals, 0, &self.scratch);
        upload += upload_start.elapsed();
        let pack_start = Instant::now();
        self.scratch.clear();
        for &id in &state.ids {
            self.scratch.extend_from_slice(&id.to_ne_bytes());
        }
        let packing = packing + pack_start.elapsed();
        let upload_start = Instant::now();
        self.queue.write_buffer(&id_buffer, 0, &self.scratch);
        self.uploads.add(UploadBytes {
            instances: models.len() as u64 * 64,
            normals: models.len() as u64 * 64,
            visible_ids: models.len() as u64 * 4,
            visible_id_writes: 1,
            write_calls: 3,
            ..Default::default()
        });
        upload += upload_start.elapsed();
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        let submitted_at = Instant::now();
        self.queue.submit([]);
        self.queue.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        self.submissions += 1;
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
        self.upload_cpu = CpuFrame {
            preparation,
            packing,
            poll,
            upload,
            submit: submitted_at.elapsed(),
            ..Default::default()
        };
        self.check_fault()?;
        Ok(Some(Instances {
            renderer: self.identity.clone(),
            state,
            transforms,
            normals,
            id_buffer,
            bind,
        }))
    }

    pub(crate) fn submit_upload(&mut self, slot: usize) {
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        let submitted_at = Instant::now();
        self.queue.submit([]);
        self.queue.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        self.submissions += 1;
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
    }
    /// Complete prepared mip chain, role-correct storage, bounded immediate submission.
    pub fn try_upload_texture(&mut self, data: &asset::Texture) -> Result<Option<Texture>, Error> {
        self.check_fault()?;
        let started = Instant::now();
        let bytes = crate::material::validate_texture(
            data,
            self.device.limits().max_texture_dimension_2d,
            self.limits.max_texture_bytes,
        )?;
        let preparation = started.elapsed();
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let started = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        let texture = self.create_texture(data, bytes);
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.submit_upload(slot);
        self.upload_cpu = CpuFrame {
            preparation,
            poll,
            upload: started.elapsed(),
            ..Default::default()
        };
        self.check_fault()?;
        Ok(Some(texture))
    }
    fn create_texture(&mut self, data: &asset::Texture, bytes: u64) -> Texture {
        let first = &data.mips[0];
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("immutable material texture"),
                size: wgpu::Extent3d {
                    width: first.width,
                    height: first.height,
                    depth_or_array_layers: data.role.layers(),
                },
                mip_level_count: data.mips.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: texture_residency::texture_format(data.role),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(
            texture.create_view(&texture_residency::texture_view_descriptor(data.role)),
            ResourceKind::TextureView,
        );
        for (level, mip) in data.mips.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    mip_level: level as u32,
                    ..texture.as_image_copy()
                },
                &mip.rgba8,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(
                        mip.width * (data.role.texel_bytes() as u32 / data.role.layers()),
                    ),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: data.role.layers(),
                },
            );
            self.uploads.textures += mip.rgba8.len() as u64;
            self.uploads.texture_writes += 1;
            self.uploads.write_calls += 1;
        }
        Texture(Arc::new(crate::material::TextureInner {
            renderer: self.identity.clone(),
            role: data.role,
            storage: crate::material::TextureStorage::Full(Box::new(
                crate::material::FullTexture {
                    _texture: texture,
                    view,
                    _allocation: self.ledger.allocate(AllocationKind::Texture, bytes),
                },
            )),
        }))
    }
    pub fn try_create_material(
        &mut self,
        data: &MaterialDescriptor<'_>,
    ) -> Result<Option<Material>, Error> {
        self.check_fault()?;
        let start = Instant::now();
        data.validate(&self.identity)?;
        self.validate_program(data)?;
        let preparation = start.elapsed();
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let start = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        let defaults = self.default_material.as_ref().unwrap()._textures.clone();
        let material = self.create_material(data, &defaults);
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.submit_upload(slot);
        self.upload_cpu = CpuFrame {
            preparation,
            poll,
            upload: start.elapsed(),
            ..Default::default()
        };
        self.check_fault()?;
        Ok(Some(material))
    }
    fn create_material(
        &mut self,
        data: &MaterialDescriptor<'_>,
        defaults: &[Texture; 3],
    ) -> Material {
        let textures = [
            data.base_color_texture
                .map_or(&defaults[0], |t| t.texture)
                .clone(),
            data.metallic_roughness_texture
                .map_or(&defaults[1], |t| t.texture)
                .clone(),
            data.normal_texture
                .map_or(&defaults[2], |t| t.texture)
                .clone(),
        ];
        let descriptors = [
            data.base_color_texture
                .map_or(asset::Sampler::default(), |t| t.sampler),
            data.metallic_roughness_texture
                .map_or(asset::Sampler::default(), |t| t.sampler),
            data.normal_texture
                .map_or(asset::Sampler::default(), |t| t.sampler),
        ]
        .map(|s| crate::material::sampler_descriptor(s, self.options.anisotropy_clamp));
        let margins = descriptors.each_ref().map(|d| {
            if d.anisotropy_clamp > 1 {
                texture_residency::ANISOTROPIC_MARGIN
            } else {
                0
            }
        });
        let samplers = descriptors.map(|d| {
            self.ledger
                .own(self.device.create_sampler(&d), ResourceKind::Sampler)
        });
        let uniform = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("immutable material factors"),
                size: 48,
                // Traced shading reads every draw's factors from a storage binding array.
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_DST
                    | if self.primary.is_some() {
                        wgpu::BufferUsages::STORAGE
                    } else {
                        wgpu::BufferUsages::empty()
                    },
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let program = data.program.unwrap_or(&self.standard).clone();
        let parameters = self.material_parameters(&program, data.parameters);
        // Streamed slots bind the 1x1 defaults: this group never retains their levels.
        let streamed =
            |t: &Texture| matches!(t.0.storage, crate::material::TextureStorage::Streamed(_));
        let views = std::array::from_fn(|i| {
            self.texture_view(if streamed(&textures[i]) {
                &defaults[i]
            } else {
                &textures[i]
            })
        });
        let bind = self.material_group(&uniform, views, &samplers, &parameters.buffer);
        self.queue.write_buffer(&uniform, 0, &data.bytes());
        self.uploads.materials += 48;
        self.uploads.write_calls += 1;
        Material {
            normal_mapped: data.normal_texture.is_some(),
            renderer: self.identity.clone(),
            double_sided: data.double_sided,
            thin: data.thin,
            transparency: data.transparency,
            streamed: textures.iter().any(streamed),
            margins,
            bind,
            _uniform: uniform,
            _samplers: samplers,
            _textures: textures,
            program,
            parameters,
            _allocation: self.ledger.allocate(
                AllocationKind::Material,
                48 + crate::program::PARAMETER_BYTES as u64,
            ),
        }
    }

    /// Raster group 2 of one material over the given texture views.
    fn material_group(
        &self,
        uniform: &wgpu::Buffer,
        views: [&wgpu::TextureView; 3],
        samplers: &[Owned<wgpu::Sampler>; 3],
        parameters: &wgpu::Buffer,
    ) -> Owned<wgpu::BindGroup> {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        for (i, (view, sampler)) in views.into_iter().zip(samplers).enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + 2 * i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + 2 * i as u32,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: parameters.as_entire_binding(),
        });
        self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material"),
                layout: &self.material_bindings,
                entries: &entries,
            }),
            ResourceKind::BindGroup,
        )
    }
    /// A streamed material's group over its current levels, for raster passes that
    /// sample; the frame made its textures fully resident first. None for other
    /// materials, whose own group already binds their textures.
    fn sampling_group(&self, material: Option<&Material>) -> Option<Owned<wgpu::BindGroup>> {
        let m = material.filter(|m| m.streamed)?;
        Some(self.material_group(
            &m._uniform,
            std::array::from_fn(|i| self.texture_view(&m._textures[i])),
            &m._samplers,
            &m.parameters.buffer,
        ))
    }

    /// Tone maps the HDR target into the frame's color view.
    fn encode_hdr_present(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let hdr = self.lighting.hdr.as_ref().unwrap();
        // PP1 runs after reconstruction and never modifies its HDR/history targets.
        let mut timestamp_writes = timestamp_writes;
        if let Some(post) = self.post.as_ref().filter(|_| self.post_settings.enabled) {
            if let Some(stamps) = &mut timestamp_writes {
                let _start = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("post start timestamp"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: stamps.query_set,
                        beginning_of_pass_write_index: stamps.beginning_of_pass_write_index.take(),
                        end_of_pass_write_index: None,
                    }),
                });
            }
            post.encode(self, encoder);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HDR present pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes,
            ..Default::default()
        });
        if let Some(post) = self.post.as_ref().filter(|_| self.post_settings.enabled) {
            post.tone(&mut pass);
        } else {
            pass.set_pipeline(&self.pipeline.present);
            // TR1b presents the target-size reconstruction even when post is disabled.
            let bind = self.upscale.as_ref().map_or(&*hdr.bind, |u| &*u.present);
            pass.set_bind_group(0, bind, &[]);
        }
        pass.draw(0..3, 0..1);
    }

    pub(crate) fn validate_color(&self, color: &wgpu::Texture) -> Result<(), Error> {
        if color.format() != self.format
            || color.dimension() != wgpu::TextureDimension::D2
            || color.width() == 0
            || color.height() == 0
            || color.depth_or_array_layers() != 1
            || color.mip_level_count() != 1
            || color.sample_count() != 1
            || !color
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(Error::InvalidInput(
                "color must be a matching single-sample, single-mip/layer D2 render attachment",
            ));
        }
        Ok(())
    }

    pub fn try_render(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        draws: &mut [Draw<'_>],
    ) -> Result<Option<FrameReport>, Error> {
        self.try_render_view(color, camera, draws, ViewMode::BaseColor)
    }
    pub fn try_render_view(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        draws: &mut [Draw<'_>],
        view_mode: ViewMode,
    ) -> Result<Option<FrameReport>, Error> {
        // TP1: transparent draws leave every opaque path for the transparent pass. The
        // split is stable and the caller's order comes back afterwards.
        let (opaque, order) = transparency_gpu::split(draws);
        let (front, transparent) = draws.split_at_mut(opaque);
        let result = self.render_view(color, camera, front, transparent, view_mode);
        if let Some(order) = order {
            transparency_gpu::restore(draws, &order);
        }
        result
    }
    fn render_view(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        draws: &mut [Draw<'_>],
        transparent: &mut [Draw<'_>],
        view_mode: ViewMode,
    ) -> Result<Option<FrameReport>, Error> {
        self.check_fault()?;
        let preparation_start = Instant::now();
        // TP1: the transparent pass runs after reconstruction, with the unjittered camera.
        self.transparency.camera = Some(*camera);
        self.validate_color(color)?;
        // Temporal reconstruction samples traced and visibility-buffer lit frames at the
        // next lattice offset: every matrix of this frame comes from the jittered camera.
        // TR1b (default on): Temporal soft shadows keep their own film history and are never
        // reconstructed, so their frames are not jittered either.
        self.reconstruction_frame = (self.reconstruction.enabled
            && view_mode == ViewMode::Lit
            && self.primary_mode != PrimaryVisibility::Raster
            && self.soft_mode != crate::SoftShadows::Temporal)
            .then_some(self.reconstruction_index);
        let jittered;
        let camera = match self.reconstruction_frame {
            Some(k) => {
                jittered = camera.jittered(temporal_reconstruction::sample_offset(k));
                &jittered
            }
            None => camera,
        };
        // TR1b upscaling (temporal_reconstruction.rs RenderScale): a scaled reconstructed
        // frame renders every pass into an internal target; the reconstruction resolves to
        // the caller's size and present writes the caller's target.
        let internal = self.upscale_target(color);
        let color = internal.as_ref().unwrap_or(color);
        if self.shadows.settings.directional_disk_angle > 0.0 && view_mode == ViewMode::Lit {
            fresh_shadow_gpu::admit_targets(&self.shadows.settings, color.width(), color.height())?;
        }
        if draws.iter().any(|draw| {
            !Arc::ptr_eq(&self.identity, &draw.instances.renderer)
                || !Arc::ptr_eq(&self.identity, &draw.mesh.renderer)
                || draw
                    .material
                    .is_some_and(|m| !Arc::ptr_eq(&self.identity, &m.renderer))
        }) {
            return Err(Error::InvalidInput(
                "draw resource belongs to another renderer",
            ));
        }
        if draws.iter().any(|draw| {
            draw.material.is_some_and(|m| m.normal_mapped) && draw.mesh.tangent.is_none()
        }) {
            return Err(Error::InvalidInput(
                "normal-mapped draw requires tangent stream",
            ));
        }
        if draws.iter().any(|draw| {
            (matches!(view_mode, ViewMode::Normal | ViewMode::Lit)
                || draw.material.is_some_and(|m| !m.double_sided))
                && draw.instances.state.invalid_normals > 0
        }) {
            return Err(Error::InvalidInput(
                "normal/lit views and single-sided materials require nonsingular affine transforms with max-element condition estimate <= 1e4",
            ));
        }
        let matrix = camera.matrix(color.width(), color.height());
        if !matrix.is_finite()
            || (view_mode == ViewMode::Lit && !camera.view().inverse().w_axis.is_finite())
        {
            return Err(Error::InvalidInput(
                "camera aspect produces nonfinite projection",
            ));
        }
        let traced = self.primary_mode != PrimaryVisibility::Raster && view_mode == ViewMode::Lit;
        if traced {
            if self.primary_mode == PrimaryVisibility::VisibilityBuffer
                && matches!(
                    self.soft_mode,
                    SoftShadows::Interactive | SoftShadows::Temporal
                )
            {
                return Err(Error::InvalidInput(
                    "visibility-buffer primaries support Virtual or Exact soft shadows; Interactive and Temporal require traced primaries",
                ));
            }
            if self.primary_mode == PrimaryVisibility::Traced
                && draws.iter().any(|d| d.mesh.acceleration.is_none())
            {
                return Err(Error::InvalidInput(
                    "traced primaries require acceleration structures",
                ));
            }
            if self.hardware_shadows.is_none()
                && self.shadows.enabled(view_mode)
                && (self.soft_mode != crate::SoftShadows::Virtual
                    || self.shadows.settings.directional_disk_angle <= 0.0)
            {
                return Err(Error::InvalidInput(
                    "visibility-buffer shadows without RT require SoftShadows::Virtual and a nonzero disk",
                ));
            }
            primary_ray_gpu::admit_draws(
                draws,
                matrix,
                self.shadows.enabled(view_mode),
                self.standard.id(),
            )?;
            #[cfg(test)]
            assert!(
                !self.reference_draws,
                "traced frames have no per-slot oracle"
            );
        }
        let preparation = preparation_start.elapsed();
        let poll = self.collect()?;
        let Some(slot) = self.free_slot() else {
            self.busy_frames += 1;
            return Ok(None);
        };
        // SK1: palette writes and software-ray updates only after frame admission.
        let skinning_uploads = self.prepare_skinning(draws)?;
        let camera_bytes = matrix_bytes(matrix);
        let shadow_start = Instant::now();
        let mut shadow_frame = if self.shadows.enabled(view_mode) {
            let bounds = draws
                .iter()
                .flat_map(|draw| {
                    draw.instances
                        .state
                        .models
                        .iter()
                        .map(move |&model| shadow_gpu::world_bounds(draw.mesh.bounds, model))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Some(self.shadows.plan(
                camera,
                [color.width(), color.height()],
                &self.lighting.source_lights,
                &bounds,
            )?)
        } else {
            None
        };
        if self.hardware_shadows.is_some()
            && let Some(frame) = &mut shadow_frame
        {
            frame.prepare_hardware_bias(&self.lighting.pending.records)?;
        }
        let mut shadow_preparation = if shadow_frame.is_some() {
            shadow_start.elapsed()
        } else {
            Duration::ZERO
        };
        let acceleration_start = Instant::now();
        // Camera rays and RT volume injection need a TLAS even without surface shadows.
        let hardware_scene = shadow_frame.is_some()
            || traced
            || (view_mode == ViewMode::Lit && self.atmosphere.settings.volumetric_fog.is_some());
        let hardware_build = if hardware_scene {
            if let Some(hardware) = &mut self.hardware_shadows {
                let scopes = OperationScopes::new(&self.device);
                let result = hardware.prepare_scene(
                    (&self.device, &self.queue),
                    &self.ledger,
                    &self.uniform,
                    draws,
                    self.submissions,
                    shadow_frame.is_some(),
                );
                if let Some(error) = scopes.finish() {
                    return Err(self.fail(error.to_string()));
                }
                result?
            } else {
                false
            }
        } else {
            false
        };
        if hardware_scene && self.hardware_shadows.is_some() {
            shadow_preparation += acceleration_start.elapsed();
        }
        // The frame's program pipeline sets, created now if missing: after every admission
        // check (so a rejected frame creates nothing) and before the frame's uploads.
        let variant = self
            .frame_paths(view_mode, shadow_frame.is_some(), traced)
            .variant;
        self.ensure_pipelines(self.frame_sets(draws.iter().map(|d| d.material), variant))?;
        let mut uploads = skinning_uploads;
        let mut scene = SceneCounts::default();
        let mut visibility = Duration::ZERO;
        let mut packing = Duration::ZERO;
        let mut upload = Duration::ZERO;
        for draw in draws.iter_mut() {
            let instances = &mut draw.instances;
            let start = Instant::now();
            let visibility_update =
                instances
                    .state
                    .visibility(matrix, draw.mesh.bounds, draw.visibility);
            visibility += start.elapsed();
            scene.cull_full_tests += visibility_update.full_tests;
            scene.cull_partial_tests += visibility_update.partial_tests;
            if let Some(span) = visibility_update.changed_id_span {
                let start = Instant::now();
                self.scratch.clear();
                for &id in &instances.state.ids[span.clone()] {
                    self.scratch.extend_from_slice(&id.to_ne_bytes());
                }
                packing += start.elapsed();
                let start = Instant::now();
                self.queue
                    .write_buffer(&instances.id_buffer, span.start as u64 * 4, &self.scratch);
                uploads.visible_ids += self.scratch.len() as u64;
                uploads.visible_id_writes += 1;
                uploads.write_calls += 1;
                upload += start.elapsed();
            }
            self.upload_instance_edits(instances, &mut uploads, &mut packing, &mut upload);
            scene.collections += 1;
            scene.normal_cache_bytes += (instances.state.normals.capacity()
                * std::mem::size_of::<crate::normal::NormalTransform>())
                as u64;
            scene.normal_gpu_bytes += instances.normals.size();
            scene.visibility_cache_bytes += instances.state.visibility_cache_bytes() as u64;
            scene.total_instances += instances.len() as u64;
            let visible = instances.state.ids.len() as u64;
            scene.visible_instances += visible;
            scene.submitted_instances += visible;
            scene.draws += u64::from(visible > 0);
            scene.triangles += visible * u64::from(draw.mesh.index_count / 3);
        }
        // TP1: transparent draws get the same visibility and ID publication.
        self.tp_prepare_draws(matrix, transparent, &mut uploads)?;
        // The per-slot oracle intentionally bypasses the production ID publication buffer.
        #[cfg(test)]
        let reference_ids: Vec<_> = if self.reference_draws {
            draws
                .iter()
                .map(|draw| {
                    assert_eq!(draw.visibility, Visibility::Unculled);
                    let buffer = self.ledger.own(
                        self.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("test oracle identity IDs"),
                            size: draw.instances.len() as u64 * 4,
                            usage: wgpu::BufferUsages::VERTEX,
                            mapped_at_creation: true,
                        }),
                        ResourceKind::Buffer,
                    );
                    {
                        let mut bytes = buffer.get_mapped_range_mut(..).unwrap();
                        for slot in 0..draw.instances.len() {
                            bytes
                                .slice(slot * 4..slot * 4 + 4)
                                .copy_from_slice(&(slot as u32).to_ne_bytes());
                        }
                    }
                    buffer.unmap();
                    buffer
                })
                .collect()
        } else {
            vec![]
        };
        let mut prepared = Vec::with_capacity(draws.len());
        #[cfg(test)]
        let mut reference_ids = reference_ids.iter();
        for draw in draws.iter() {
            #[cfg(test)]
            if self.reference_draws {
                assert_eq!(draw.visibility, Visibility::Unculled);
                let reference_ids = reference_ids.next().unwrap();
                for slot in 0..draw.instances.len() as u32 {
                    prepared.push(PreparedDraw {
                        mesh: draw.mesh,
                        instances: draw.instances,
                        material: draw.material,
                        indices: 0..draw.mesh.index_count,
                        slots: slot..slot + 1,
                        reference_ids: Some(reference_ids),
                    });
                }
                scene.draws += draw.instances.len().saturating_sub(1) as u64;
                continue;
            }
            prepared.push(PreparedDraw {
                mesh: draw.mesh,
                instances: draw.instances,
                material: draw.material,
                indices: 0..draw.mesh.index_count,
                slots: 0..draw.instances.state.ids.len() as u32,
                #[cfg(test)]
                reference_ids: None,
            });
        }
        let shadow_start = Instant::now();
        let shadow_draws = if shadow_frame.is_some() {
            draws
                .iter()
                .flat_map(|draw| {
                    (0..draw.instances.len() as u32).map(move |slot| ShadowDraw {
                        mesh: draw.mesh,
                        instances: &*draw.instances,
                        material: draw.material,
                        indices: 0..draw.mesh.index_count,
                        slot,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        if shadow_frame.is_some() {
            shadow_preparation += shadow_start.elapsed();
        }
        self.submit_frame(
            color,
            camera_bytes,
            camera,
            view_mode,
            &prepared,
            transparent,
            shadow_frame,
            &shadow_draws,
            hardware_build,
            traced,
            slot,
            uploads,
            scene,
            CpuFrame {
                shadow_preparation,
                preparation,
                visibility,
                packing,
                upload,
                poll,
                ..CpuFrame::default()
            },
        )
    }

    fn upload_instance_edits(
        &mut self,
        instances: &mut Instances,
        uploads: &mut UploadBytes,
        packing: &mut Duration,
        upload: &mut Duration,
    ) {
        if instances.state.dirty_count > 0 {
            for span in instances.state.dirty_runs() {
                let start = Instant::now();
                self.scratch.clear();
                for &model in &instances.state.models[span.clone()] {
                    self.scratch.extend_from_slice(&matrix_bytes(model));
                }
                *packing += start.elapsed();
                let start = Instant::now();
                self.queue.write_buffer(
                    &instances.transforms,
                    span.start as u64 * 64,
                    &self.scratch,
                );
                uploads.models += self.scratch.len() as u64;
                *upload += start.elapsed();
                let start = Instant::now();
                self.scratch.clear();
                for normal in &instances.state.normals[span.clone()] {
                    self.scratch.extend_from_slice(&normal.bytes());
                }
                *packing += start.elapsed();
                let start = Instant::now();
                self.queue
                    .write_buffer(&instances.normals, span.start as u64 * 64, &self.scratch);
                uploads.normals += self.scratch.len() as u64;
                uploads.write_calls += 1;
                uploads.model_writes += 1;
                uploads.write_calls += 1;
                *upload += start.elapsed();
            }
            instances.state.clear_dirty();
        }
    }

    /// Raster submission for virtual geometry, which never traces primary visibility.
    #[allow(clippy::too_many_arguments)]
    fn submit_prepared(
        &mut self,
        color: &wgpu::Texture,
        camera_bytes: [u8; 64],
        camera: &Camera,
        view_mode: ViewMode,
        draws: &[PreparedDraw<'_>],
        shadow_frame: Option<ShadowFrame>,
        shadow_draws: &[ShadowDraw<'_>],
        hardware_build: bool,
        slot: usize,
        uploads: UploadBytes,
        scene: SceneCounts,
        cpu: CpuFrame,
    ) -> Result<Option<FrameReport>, Error> {
        let variant = self
            .frame_paths(view_mode, shadow_frame.is_some(), false)
            .variant;
        self.ensure_pipelines(self.frame_sets(draws.iter().map(|d| d.material), variant))?;
        self.submit_frame(
            color,
            camera_bytes,
            camera,
            view_mode,
            draws,
            &[],
            shadow_frame,
            shadow_draws,
            hardware_build,
            false,
            slot,
            uploads,
            scene,
            cpu,
        )
    }

    /// How a frame with these choices resolves visibility and lit shading: what
    /// `submit_frame` encodes and which program pipeline set it draws with.
    fn frame_paths(&self, view_mode: ViewMode, shadowed: bool, traced: bool) -> FramePaths {
        use program_gpu::Variant;
        let lit = view_mode == ViewMode::Lit;
        let hardware_shadowed = shadowed
            && (self.hardware_shadows.is_some()
                || (traced
                    && self.primary_mode == PrimaryVisibility::VisibilityBuffer
                    && self.soft_mode == crate::SoftShadows::Virtual));
        let soft = hardware_shadowed && self.shadows.settings.directional_disk_angle > 0.0;
        #[cfg(test)]
        let soft = soft && (traced || !self.fresh_fragment_reference);
        let fresh = soft && !traced;
        let temporal = traced && soft && self.soft_mode == crate::SoftShadows::Temporal;
        FramePaths {
            lit,
            hardware_shadowed,
            soft,
            fresh,
            interactive: traced
                && self.primary_mode == PrimaryVisibility::Traced
                && soft
                && self.soft_mode == crate::SoftShadows::Interactive,
            temporal,
            variant: if temporal {
                Some(Variant::Temporal)
            } else if traced {
                Some(Variant::Traced)
            } else if fresh {
                Some(Variant::Fresh)
            } else if hardware_shadowed {
                Some(Variant::Hardware)
            } else if lit {
                Some(Variant::Lit)
            } else {
                None
            },
        }
    }
    // Persistent data is prepared once, then borrowed for the shared render pass.
    // `traced` frames replace both raster geometry passes with camera rays and
    // deferred compute shading; the TLAS for this draw sequence is current.
    #[allow(clippy::too_many_arguments)]
    fn submit_frame(
        &mut self,
        color: &wgpu::Texture,
        camera_bytes: [u8; 64],
        camera: &Camera,
        view_mode: ViewMode,
        draws: &[PreparedDraw<'_>],
        transparent: &[Draw<'_>],
        shadow_frame: Option<ShadowFrame>,
        shadow_draws: &[ShadowDraw<'_>],
        hardware_build: bool,
        traced: bool,
        slot: usize,
        mut uploads: UploadBytes,
        scene: SceneCounts,
        mut cpu: CpuFrame,
    ) -> Result<Option<FrameReport>, Error> {
        // Both callers have admitted this slot. No camera/mode/lighting staging on busy.
        let start = Instant::now();
        if self.camera_shadow != Some(camera_bytes) {
            self.queue.write_buffer(&self.uniform, 0, &camera_bytes);
            uploads.camera = 64;
            uploads.write_calls += 1;
        }
        if self.view_mode != view_mode {
            let mut bytes = [0; 16];
            bytes[..4].copy_from_slice(&(view_mode as u32).to_ne_bytes());
            self.queue.write_buffer(&self.uniform, 64, &bytes);
            uploads.view_settings = 16;
            uploads.write_calls += 1;
        }
        let FramePaths {
            lit,
            hardware_shadowed,
            soft,
            fresh,
            interactive,
            temporal,
            ..
        } = self.frame_paths(view_mode, shadow_frame.is_some(), traced);
        // Lit traced frames only; raster frames of a reconstructing renderer keep no history.
        let reconstructed = traced && !temporal && self.reconstruction_frame.is_some();
        if !reconstructed {
            self.reconstruction_frame = None;
        }
        // Streamed textures (texture_residency.rs): traced and visibility-buffer frames
        // that bind any outside Full mode shade with pipelines that request their levels.
        // Raster and temporal passes sample without requests, so the textures they bind
        // become fully resident first. Default frames bind none and do none of this.
        let streamed = self.streamed_ids(draws.iter().map(|d| d.material));
        let streaming = traced
            && !temporal
            && !streamed.is_empty()
            && self.residency.mode != TextureResidency::Full;
        if streaming {
            // Admission guarantees one program per traced frame: the first draw's.
            let program = draws
                .first()
                .and_then(|d| d.material)
                .map_or(self.standard.id(), |m| m.program.id());
            self.ensure_streamed(program)?;
        }
        // Feedback growth is spread over frames; raster and temporal frames need theirs now.
        if streaming {
            self.apply_residency(&[], texture_residency::STEP_BYTES);
        } else {
            self.apply_residency(&streamed, u64::MAX);
        }
        let verify =
            streaming && self.residency_verify(&streamed, camera, [color.width(), color.height()]);
        // N8e: another pass writing the soft visibility target ends Virtual's rolling refresh,
        // whose untouched tiles keep the counts it wrote last.
        let virtual_frame = traced
            && soft
            && !temporal
            && !interactive
            && self.soft_mode == crate::SoftShadows::Virtual;
        if soft
            && !virtual_frame
            && let Some(state) = self.virtual_shadows.as_mut()
        {
            state.rolling = false;
        }
        let mut local_lights = None;
        if lit {
            let shadow_start = Instant::now();
            self.upload_shadows(shadow_frame.as_ref(), &mut uploads)?;
            if shadow_frame.is_some() {
                cpu.shadow_upload = shadow_start.elapsed();
            }
            // LC1: shadowed local lights trace where the shading module binds rays
            // (light_cluster::ray_scene), else use pooled maps.
            let maps =
                !(self.hardware_shadows.is_some() && (traced || (hardware_shadowed && !fresh)));
            local_lights = Some(self.prepare_light_clusters(
                camera,
                [color.width(), color.height()],
                draws,
                maps,
                &mut uploads,
            ));
            // AT1: the camera volume and uniform the lighting group binds.
            self.prepare_atmosphere(camera, [color.width(), color.height()], &mut uploads)?;
            // TP2: publish the coloured sun map before binding the frame's lighting.
            self.prepare_transmission(transparent)?;
            self.upload_lighting(camera.view().inverse().w_axis.truncate(), &mut uploads);
        }
        cpu.upload += start.elapsed() - cpu.shadow_upload;
        self.uploads.add(uploads);
        let encode_start = Instant::now();
        // Traced frames resolve visibility without raster depth or MSAA targets.
        if !traced
            && self
                .depth
                .as_ref()
                .is_none_or(|depth| depth.texture.size() != color.size())
        {
            let texture = self.ledger.own(
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("depth"),
                    size: color.size(),
                    mip_level_count: 1,
                    sample_count: self.options.samples as u32,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    // TP1: the transparent pass reads 1x raster depth.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | if cfg!(test) && self.options.samples == SampleCount::One {
                            wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = self.ledger.own(
                texture.create_view(&Default::default()),
                ResourceKind::TextureView,
            );
            let bytes = u64::from(color.width())
                * u64::from(color.height())
                * 4
                * self.options.samples as u64;
            self.depth = Some(Depth {
                texture,
                view,
                _allocation: self.ledger.allocate(AllocationKind::Target, bytes),
            });
            self.msaa = if self.options.samples == SampleCount::Four {
                let texture = self.ledger.own(
                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("MSAA color"),
                        size: color.size(),
                        mip_level_count: 1,
                        sample_count: 4,
                        dimension: wgpu::TextureDimension::D2,
                        format: self.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    }),
                    ResourceKind::Texture,
                );
                let view = self.ledger.own(
                    texture.create_view(&Default::default()),
                    ResourceKind::TextureView,
                );
                Some(Depth {
                    texture,
                    view,
                    _allocation: self.ledger.allocate(
                        AllocationKind::Target,
                        u64::from(color.width())
                            * u64::from(color.height())
                            * 4
                            * u64::from(self.format.block_copy_size(None).unwrap()),
                    ),
                })
            } else {
                None
            };
        }
        if lit {
            self.ensure_hdr(color);
        }
        if soft {
            let scopes = OperationScopes::new(&self.device);
            self.ensure_fresh_targets(color.size(), hardware_build);
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
        }
        if traced {
            self.primary.as_mut().unwrap().requests = streaming;
            let scopes = OperationScopes::new(&self.device);
            let scene_generation = self
                .hardware_shadows
                .as_ref()
                .map_or(0, |h| h.stats.tlas_builds + u64::from(hardware_build));
            let mut primary = UploadBytes::default();
            let result = self.prepare_primary(draws, camera, soft, scene_generation, &mut primary);
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
            result?;
            uploads.add(primary);
            self.uploads.add(primary);
            if self.primary_mode == PrimaryVisibility::VisibilityBuffer {
                self.prepare_visibility_buffer(draws, camera, color.size())?;
            }
            if soft && self.soft_mode == crate::SoftShadows::Virtual {
                let scopes = OperationScopes::new(&self.device);
                let result = self.prepare_virtual(draws, camera, color.size());
                if let Some(error) = scopes.finish() {
                    return Err(self.fail(error.to_string()));
                }
                result?;
            }
            if temporal {
                self.ensure_film_kernel()?;
                let key = self.film_key(&camera_bytes, shadow_frame.as_ref().map(|f| f.records()));
                self.prepare_film(key);
            }
            if let Some(index) = self.reconstruction_frame {
                self.ensure_reconstruction()?;
                self.prepare_reconstruction(camera, index, draws);
            }
        }
        if lit {
            // Traced and visibility-buffer frames feed the cache; every other lit frame
            // publishes it as off, so its programs read the unoccluded environment.
            let scopes = OperationScopes::new(&self.device);
            let scene_generation = self
                .hardware_shadows
                .as_ref()
                .map_or(0, |h| h.stats.tlas_builds + u64::from(hardware_build));
            let result = self.prepare_gi(
                draws,
                camera,
                [color.width(), color.height()],
                traced && !temporal,
                scene_generation,
            );
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
            result?;
        }
        if lit {
            let scopes = OperationScopes::new(&self.device);
            self.prepare_post(camera, traced)?;
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
        }
        self.check_fault()?;
        let view = self
            .ledger
            .attachment(color.create_view(&Default::default()));
        let skinned_bounds = visibility_buffer::refresh_skinned_bounds(self, draws);
        uploads.add(skinned_bounds);
        self.uploads.add(skinned_bounds);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mesh frame"),
            });
        let timed = self.timing_status == TimingStatus::Enabled;
        self.encode_skinning(&mut encoder, draws, hardware_build);
        if let Some(frame) = &local_lights {
            self.encode_light_clusters(&mut encoder, frame, draws);
        }
        if hardware_build {
            let start = Instant::now();
            self.hardware_shadows
                .as_ref()
                .unwrap()
                .encode_scene(&mut encoder);
            cpu.shadow_encode += start.elapsed();
        }
        if let Some(frame) = shadow_frame.as_ref().filter(|_| !hardware_shadowed) {
            let shadow_start = Instant::now();
            self.encode_shadows(&mut encoder, frame, shadow_draws, slot, timed);
            cpu.shadow_encode = shadow_start.elapsed();
        }
        // AT1: LUTs, aerial perspective and froxels, after the TLAS build, before shading.
        if lit {
            // TP2: all opaque paths and GI shading read this independent caster map.
            self.encode_transmission(&mut encoder, transparent);
            self.encode_atmosphere(&mut encoder);
        }
        if fresh {
            self.encode_fresh(&mut encoder, draws, slot, timed);
        }
        if traced {
            let stamps = |first: u32| {
                self.timing
                    .as_ref()
                    .filter(|_| timed)
                    .map(|t| wgpu::ComputePassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: Some(slot as u32 * 10 + first),
                        end_of_pass_write_index: Some(slot as u32 * 10 + first + 1),
                    })
            };
            let p = self.pipeline.primary.as_ref().unwrap();
            if temporal {
                self.encode_film(&mut encoder, stamps(6), stamps(0));
            } else {
                if interactive
                    && reconstructed
                    && p.reconstruction.as_ref().is_some_and(|r| r.trace.is_some())
                    && self.reconstruction.rotate_directions
                {
                    self.encode_reconstruction_interactive(&mut encoder, stamps(6));
                } else if interactive {
                    let tile = primary_ray_gpu::TRACE_TILE;
                    self.encode_primary_interactive(&mut encoder, &p.interactive, tile, stamps(6));
                } else if soft && self.soft_mode == crate::SoftShadows::Virtual {
                    // Traced receivers, then N8 visibility with no ray queries.
                    if self.primary_mode == PrimaryVisibility::VisibilityBuffer {
                        self.encode_visibility_buffer(&mut encoder, true, stamps(6));
                    } else {
                        self.encode_primary_visibility(&mut encoder, true, stamps(6));
                    }
                    self.encode_virtual(&mut encoder, stamps(8));
                } else {
                    if self.primary_mode == PrimaryVisibility::VisibilityBuffer {
                        self.encode_visibility_buffer(&mut encoder, soft, stamps(6));
                    } else {
                        self.encode_primary_visibility(&mut encoder, soft, stamps(6));
                    }
                    if soft {
                        self.encode_primary_fresh(&mut encoder, &p.fresh, stamps(8));
                    }
                }
                let shading = if soft {
                    primary_ray_gpu::Shading::Soft
                } else {
                    primary_ray_gpu::Shading::Hard
                };
                if streaming {
                    self.encode_request_clear(&mut encoder);
                }
                let shade = self.primary.as_ref().and_then(|p| p.shading_groups()).map(
                    |(arrays, materials)| gi_cache::ShadeGroups {
                        arrays,
                        materials,
                        lighting: self.lighting.bind(),
                    },
                );
                self.gi
                    .encode(&mut encoder, self.lighting.kernel_bind(), shade);
                self.encode_primary_shading(&mut encoder, shading, stamps(0));
            }
            if reconstructed {
                self.encode_reconstruction(&mut encoder, stamps(4));
            }
        }
        let residency = streaming.then(|| self.encode_residency_readback(&mut encoder, slot));
        let timestamps =
            self.timing
                .as_ref()
                .filter(|_| timed)
                .map(|timing| wgpu::RenderPassTimestampWrites {
                    query_set: &timing.queries,
                    beginning_of_pass_write_index: Some(slot as u32 * 10),
                    end_of_pass_write_index: Some(slot as u32 * 10 + 1),
                });
        let hdr = self.lighting.hdr.as_ref().filter(|_| lit);
        let msaa = if lit {
            hdr.and_then(|h| h.msaa.as_ref())
        } else {
            self.msaa.as_ref()
        };
        let resolved = hdr.map_or(&*view, |h| &*h.resolved.view);
        let clear = if lit { self.lit_clear() } else { CLEAR_COLOR };
        if !traced {
            // The mesh pass samples: streamed materials bind their current, full levels.
            let sampling: Vec<_> = draws
                .iter()
                .map(|d| self.sampling_group(d.material))
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa.map_or(resolved, |t| &*t.view),
                    depth_slice: None,
                    resolve_target: msaa.map(|_| resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: 1.0,
                        }),
                        store: if lit && msaa.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().unwrap().view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamps,
                ..Default::default()
            });
            // AT1: the sky first, on every sample, without depth; surfaces then cover it.
            if let Some(sky) = self.atmosphere.raster_sky().filter(|_| lit) {
                pass.set_pipeline(sky);
                pass.set_bind_group(3, self.lighting.bind(), &[]);
                pass.draw(0..3, 0..1);
            }
            // Each lit draw uses its material's program; pipelines switch in submission
            // order, which defines equal-depth winners.
            let default = self.default_material.as_ref().unwrap();
            let pipeline_for = |program: u64, tangent: bool| {
                let t = usize::from(tangent);
                let p = &self.program_slot(program).pipelines;
                if fresh {
                    &p.fresh.as_ref().unwrap()[t]
                } else if hardware_shadowed {
                    &p.hardware.as_ref().unwrap()[t]
                } else if lit {
                    &p.lit.as_ref().unwrap()[t]
                } else if tangent {
                    &*self.pipeline.diagnostic_tangent
                } else {
                    &*self.pipeline.diagnostic
                }
            };
            pass.set_pipeline(pipeline_for(self.standard.id(), false));
            if lit {
                pass.set_bind_group(3, self.lighting.bind(), &[]);
            }
            pass.set_bind_group(
                0,
                if fresh {
                    &self.fresh.as_ref().unwrap().targets.as_ref().unwrap().shade
                } else if hardware_shadowed {
                    self.hardware_shadows.as_ref().unwrap().bind()
                } else {
                    &*self.bind
                },
                &[],
            );
            // Pass state persists across draws, so unchanged state is never re-encoded.
            let mut pipeline = None;
            let (mut instances, mut material, mut ids, mut mesh) = (None, None, None, None);
            for (draw, sampling) in draws.iter().zip(&sampling) {
                if draw.slots.is_empty() {
                    continue;
                }
                let candidate = pipeline_for(
                    draw.material.unwrap_or(default).program.id(),
                    draw.mesh.tangent.is_some(),
                );
                if changed(&mut pipeline, candidate) {
                    pass.set_pipeline(candidate);
                }
                let bind: &wgpu::BindGroup = &draw.instances.bind;
                if changed(&mut instances, bind) {
                    pass.set_bind_group(1, bind, &[]);
                }
                let bind: &wgpu::BindGroup = match sampling {
                    Some(group) => group,
                    None => &draw.material.unwrap_or(default).bind,
                };
                if changed(&mut material, bind) {
                    pass.set_bind_group(2, bind, &[]);
                }
                #[cfg(test)]
                let id_buffer: &wgpu::Buffer =
                    draw.reference_ids.unwrap_or(&draw.instances.id_buffer);
                #[cfg(not(test))]
                let id_buffer: &wgpu::Buffer = &draw.instances.id_buffer;
                if changed(&mut ids, id_buffer) {
                    pass.set_vertex_buffer(1, id_buffer.slice(..));
                }
                if changed(&mut mesh, draw.mesh) {
                    pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                    if let Some(t) = &draw.mesh.tangent {
                        pass.set_vertex_buffer(2, t.slice(..));
                    }
                    pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                }
                pass.draw_indexed(draw.indices.clone(), 0, draw.slots.clone());
            }
        }
        // TP1: transparent draws and particles, after opaque shading and reconstruction.
        // Upscaled frames skip them (docs/m7-gaps.md, Transparency).
        if lit && self.upscale.is_none() {
            self.encode_transparency(&mut encoder, draws, transparent, traced)?;
        }
        // TR1b upscaling: present writes the caller's target, not the internal one.
        let output_view = self.upscale.as_ref().map(|u| {
            self.ledger
                .attachment(u.output.create_view(&Default::default()))
        });
        if lit && self.lighting.hdr.is_some() {
            self.encode_hdr_present(
                &mut encoder,
                output_view.as_ref().unwrap_or(&view),
                self.timing
                    .as_ref()
                    .filter(|_| timed)
                    .map(|t| wgpu::RenderPassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: Some(slot as u32 * 10 + 2),
                        end_of_pass_write_index: Some(slot as u32 * 10 + 3),
                    }),
            );
        }
        let mapped = if timed {
            let timing = self.timing.as_ref().unwrap();
            let readback = self.slots[slot].readback.as_ref().unwrap();
            let offset = slot as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
            encoder.resolve_query_set(
                &timing.queries,
                slot as u32 * 10
                    ..slot as u32 * 10
                        + if (shadow_frame.is_some() && !hardware_shadowed) || reconstructed {
                            6
                        } else if lit {
                            4
                        } else {
                            2
                        },
                &timing.resolve,
                offset,
            );
            encoder.copy_buffer_to_buffer(
                &timing.resolve,
                offset,
                readback,
                0,
                if (shadow_frame.is_some() && !hardware_shadowed) || reconstructed {
                    48
                } else if lit {
                    32
                } else {
                    16
                },
            );
            // Receivers (raster or traced), then visibility compute when soft and separate.
            if soft || traced {
                let stamps = if soft && !interactive { 4 } else { 2 };
                encoder.resolve_query_set(
                    &timing.queries,
                    slot as u32 * 10 + 6..slot as u32 * 10 + 6 + stamps,
                    &timing.resolve,
                    offset,
                );
                encoder.copy_buffer_to_buffer(
                    &timing.resolve,
                    offset,
                    readback,
                    48,
                    u64::from(stamps) * 8,
                );
            }
            let mapped = Arc::new(OnceLock::new());
            let callback = mapped.clone();
            encoder.map_buffer_on_submit(readback, wgpu::MapMode::Read, .., move |result| {
                let _ = callback.set(result);
            });
            Some(mapped)
        } else {
            None
        };
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        let commands = encoder.finish();
        self.check_fault()?;
        let encode = encode_start.elapsed() - cpu.shadow_encode;
        let submitted_at = Instant::now();
        let index = self.queue.submit([commands]);
        let submit = submitted_at.elapsed();
        self.submissions += 1;
        if lit {
            self.atmosphere_submitted(camera, [color.width(), color.height()]);
        }
        if hardware_build {
            // SK1: pose updates were encoded before this frame's TLAS and consumers.
            let mut seen = std::collections::HashSet::new();
            for d in draws {
                if d.mesh.skin.is_some()
                    && let Some(a) = &d.mesh.acceleration
                    && seen.insert(Arc::as_ptr(a))
                {
                    self.hardware_shadows.as_mut().unwrap().dynamic_submitted(
                        &self.device,
                        &self.ledger,
                        a.clone(),
                        self.submissions,
                    );
                }
            }
            self.hardware_shadows.as_mut().unwrap().scene_submitted(
                &self.device,
                &self.ledger,
                self.submissions,
            );
        }
        self.frames += 1;
        self.timed_frames += u64::from(timed);
        let frame = FrameId(self.frames);
        self.record_flight(
            slot,
            Flight {
                done,
                mapped,
                lit,
                shadowed: shadow_frame.is_some() && !hardware_shadowed,
                fresh: soft && !interactive && !temporal,
                submission: self.submissions,
                frame: Some(frame),
                revision: self.revision,
                submitted_at,
            },
        );
        self.slots[slot].traced_receivers = traced && (!soft || interactive || temporal);
        self.slots[slot].reconstructed = reconstructed;
        if reconstructed {
            self.reconstruction_index += 1;
            self.reconstruction_frame = None;
        }
        self.slots[slot].residency = residency;
        self.check_fault()?;
        if let Some(frame) = shadow_frame {
            self.stamp_shadows(frame, shadow_draws);
        }
        if lit {
            self.stamp_hdr(
                frame,
                if traced {
                    self.primary_hash
                } else {
                    self.shader_hash
                },
            );
        }
        self.camera_shadow = Some(camera_bytes);
        self.view_mode = view_mode;
        if verify {
            self.verify_frame(slot, index, draws, soft, &view)?;
        }
        Ok(Some(FrameReport {
            frame,
            revision: self.revision,
            cpu: CpuFrame {
                encode,
                submit,
                ..cpu
            },
            uploads,
            timed,
            scene,
            primary: if traced {
                self.primary_mode
            } else {
                PrimaryVisibility::Raster
            },
            soft_shadows: if traced && soft {
                self.soft_mode
            } else {
                crate::SoftShadows::Exact
            },
        }))
    }

    pub(crate) fn collect(&mut self) -> Result<Duration, Error> {
        self.check_fault()?;
        let start = Instant::now();
        self.purge_programs();
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(Error::Poll)?;
        self.check_fault()?;
        let mut observed = vec![];
        for slot in &mut self.slots {
            let Some(flight) = &slot.flight else {
                continue;
            };
            if flight.done.get().is_none()
                || flight.mapped.as_ref().is_some_and(|m| m.get().is_none())
                || slot
                    .residency
                    .as_ref()
                    .is_some_and(|(m, _)| m.get().is_none())
            {
                continue;
            }
            let flight = slot.flight.take().unwrap();
            let traced_receivers = std::mem::take(&mut slot.traced_receivers);
            let reconstructed = std::mem::take(&mut slot.reconstructed);
            if let Some((mapped, ids)) = slot.residency.take()
                && let Some(Ok(())) = mapped.get()
            {
                let readback = slot.residency_readback.as_ref().unwrap();
                observed.push((texture_residency::read_words(readback), ids, flight.frame));
            }
            // Completion of N proves completion of every earlier queue submission.
            self.completed_submission = self.completed_submission.max(flight.submission);
            let mut gpu = None;
            if let Some(mapped) = flight.mapped {
                match mapped.get().unwrap() {
                    Ok(()) => {
                        let readback = slot.readback.as_ref().unwrap();
                        let result = readback.get_mapped_range(..).map(|view| {
                            let begin = u64::from_ne_bytes(view[..8].try_into().unwrap());
                            let end = u64::from_ne_bytes(view[8..16].try_into().unwrap());
                            let present = if flight.lit {
                                Some((
                                    u64::from_ne_bytes(view[16..24].try_into().unwrap()),
                                    u64::from_ne_bytes(view[24..32].try_into().unwrap()),
                                ))
                            } else {
                                None
                            };
                            let words = || {
                                Some((
                                    u64::from_ne_bytes(view[32..40].try_into().unwrap()),
                                    u64::from_ne_bytes(view[40..48].try_into().unwrap()),
                                ))
                            };
                            let shadow = if flight.shadowed { words() } else { None };
                            // Traced frames have no raster shadow pass: the same words then
                            // time temporal reconstruction, after shading, before present.
                            let reconstruction = if reconstructed { words() } else { None };
                            let stamp = |i: usize| {
                                u64::from_ne_bytes(view[48 + i * 8..56 + i * 8].try_into().unwrap())
                            };
                            // Receivers precede any visibility compute; both precede shading.
                            let receivers =
                                (flight.fresh || traced_receivers).then(|| (stamp(0), stamp(1)));
                            let compute = flight.fresh.then(|| (stamp(2), stamp(3)));
                            (
                                begin,
                                end,
                                present,
                                shadow,
                                receivers,
                                compute,
                                reconstruction,
                            )
                        });
                        readback.unmap();
                        match result {
                            Ok((
                                begin,
                                end,
                                present,
                                shadow,
                                receivers,
                                compute,
                                reconstruction,
                            )) if end >= begin
                                && present.is_none_or(|(a, b)| b >= a && a >= end)
                                && shadow.is_none_or(|(a, b)| b >= a && b <= begin)
                                && receivers.is_none_or(|(a, b)| b >= a && begin >= b)
                                && compute.is_none_or(|(c, d)| {
                                    receivers.is_some_and(|(_, b)| c >= b) && d >= c && begin >= d
                                })
                                && reconstruction.is_none_or(|(a, b)| {
                                    a >= end && b >= a && present.is_none_or(|(c, _)| c >= b)
                                }) =>
                            {
                                let seconds = (end - begin) as f64
                                    * f64::from(self.timing.as_ref().unwrap().period)
                                    * 1e-9;
                                let period = f64::from(self.timing.as_ref().unwrap().period);
                                let span = |(a, b): (u64, u64)| {
                                    Duration::from_secs_f64((b - a) as f64 * period * 1e-9)
                                };
                                if let Ok(mesh_pass) = Duration::try_from_secs_f64(seconds) {
                                    gpu = Some(GpuSample {
                                        frame: flight.frame.unwrap(),
                                        revision: flight.revision,
                                        mesh_pass,
                                        receiver_pass: receivers.map(span),
                                        visibility_compute: compute.map(span),
                                        shadow_generation: shadow.map(span),
                                        render_span: span((
                                            receivers.map_or_else(
                                                || shadow.map_or(begin, |(a, _)| a),
                                                |(a, _)| a,
                                            ),
                                            present.map_or(end, |(_, b)| b),
                                        )),
                                        present_pass: present.map(span),
                                        reconstruction_pass: reconstruction.map(span),
                                    });
                                } else {
                                    self.timing_status = TimingStatus::Disabled {
                                        diagnostic: "invalid timestamp period/range".into(),
                                    };
                                    self.timing_failures += 1;
                                }
                            }
                            other => {
                                self.timing_status = TimingStatus::Disabled {
                                    diagnostic: format!("timestamp readback failed: {other:?}"),
                                };
                                self.timing_failures += 1;
                            }
                        }
                    }
                    Err(error) => {
                        // Failed mapping leaves the buffer Idle. Unmapping it would be another error.
                        self.timing_status = TimingStatus::Disabled {
                            diagnostic: format!("timestamp mapping failed: {error}"),
                        };
                        self.timing_failures += 1;
                    }
                }
            }
            self.gpu_samples += u64::from(gpu.is_some());
            self.events.push(Observation {
                completion: Completion {
                    submission: flight.submission,
                    frame: flight.frame,
                    revision: flight.revision,
                    submitted_at: flight.submitted_at,
                    observed_at: Instant::now(),
                },
                gpu,
            });
        }
        for (words, ids, frame) in observed {
            let misses = self.observe_residency(&words, &ids, frame.map_or(0, |f| f.0));
            self.residency.stats.misses += misses;
            self.residency.stats.missed_frames += u64::from(misses > 0);
        }
        self.residency.prune();
        self.lighting.collect(self.completed_submission);
        self.retired_post
            .retain(|(last, _)| *last > self.completed_submission);
        self.atmosphere.collect(self.completed_submission);
        self.gi.collect(self.completed_submission);
        self.environment
            .uploading
            .retain(|(last, _)| *last > self.completed_submission);
        self.shadows.collect(self.completed_submission);
        if let Some(fresh) = &mut self.fresh {
            fresh.collect(self.completed_submission);
        }
        if let Some(state) = &mut self.virtual_shadows {
            state.collect(self.completed_submission);
        }
        if let Some(primary) = &mut self.primary {
            primary.collect(self.completed_submission);
        }
        if let Some(hardware) = &mut self.hardware_shadows {
            hardware.collect(
                self.completed_submission,
                (&self.device, &self.queue),
                &self.ledger,
            );
        }
        self.check_fault()?;
        Ok(start.elapsed())
    }
    /// Nonblocking progress and bounded event delivery, including while the viewer is suspended.
    pub fn poll(&mut self) -> Result<Events, Error> {
        let duration = self.collect()?;
        Ok(self.events.drain(duration))
    }
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn present(&self, frame: wgpu::SurfaceTexture) -> Result<(), Error> {
        self.check_fault()?;
        self.queue.present(frame);
        self.check_fault()
    }
    /// Snapshot only, with no device poll. HAL/allocator observations include external device users.
    pub fn stats(&self) -> Stats {
        let hal = self.device.get_internal_counters().hal;
        Stats {
            ownership: self.ledger.snapshot(),
            hal: HalStats {
                buffers: hal.buffers.read(),
                textures: hal.textures.read(),
                texture_count_reliable: false,
                texture_views: hal.texture_views.read(),
                bind_groups: hal.bind_groups.read(),
                bind_group_layouts: hal.bind_group_layouts.read(),
                pipeline_layouts: hal.pipeline_layouts.read(),
                render_pipelines: hal.render_pipelines.read(),
                shader_modules: hal.shader_modules.read(),
                query_sets: hal.query_sets.read(),
                buffer_bytes: hal.buffer_memory.read(),
                texture_bytes: hal.texture_memory.read(),
                memory_allocations: hal.memory_allocations.read(),
            },
            allocator: self
                .device
                .generate_allocator_report()
                .map(|a| AllocatorStats {
                    allocated_bytes: a.total_allocated_bytes,
                    reserved_bytes: a.total_reserved_bytes,
                }),
            pipelines: self.pipelines,
            uploads: self.uploads,
            submissions: self.submissions,
            frames: self.frames,
            flights: self.occupied(),
            peak_flights: self.peak_flights,
            busy_uploads: self.busy_uploads,
            busy_frames: self.busy_frames,
            timing: self.timing_status.clone(),
            timing_failures: self.timing_failures,
            timed_frames: self.timed_frames,
            gpu_samples: self.gpu_samples,
            events_dropped: self.events.dropped,
            revision: self.revision,
            upload_cpu: self.upload_cpu,
        }
    }
}

#[path = "virtual_gpu.rs"]
mod virtual_gpu;
pub use virtual_gpu::{Capture, CaptureInfo};

#[cfg(test)]
#[path = "fresh_shadow_tests.rs"]
mod fresh_shadow_tests;

#[cfg(test)]
#[path = "specular_aa_tests.rs"]
mod specular_aa_tests;

#[cfg(test)]
#[path = "iq1_tests.rs"]
mod iq1_tests;

#[cfg(test)]
#[path = "skinning_tests.rs"]
mod skinning_tests;

#[cfg(test)]
#[path = "light_cluster_tests.rs"]
mod light_cluster_tests;
