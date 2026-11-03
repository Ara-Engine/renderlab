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
    submission: u64,
    frame: Option<FrameId>,
    revision: Revision,
    submitted_at: Instant,
}

#[cfg(test)]
impl Renderer {
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
        bytes
    }
    pub(crate) fn test_failed_map(&mut self) {
        self.test_drain("injected failed mapping setup");
        let done = Arc::new(OnceLock::from(()));
        let mapped = Arc::new(OnceLock::from(Err(wgpu::BufferAsyncError)));
        self.slots[0].flight = Some(Flight {
            done,
            mapped: Some(mapped),
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
struct Slot {
    readback: Option<Owned<wgpu::Buffer>>,
    flight: Option<Flight>,
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
    pipeline: Owned<wgpu::RenderPipeline>,
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
        if !(1..=16).contains(&options.anisotropy_clamp) {
            return Err(Error::InvalidInput("anisotropy_clamp must be in 1..=16"));
        }
        for format in [color_format, wgpu::TextureFormat::Depth32Float] {
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
            && !color_format
                .guaranteed_format_features(device.features())
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
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
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(32),
            },
            count: None,
        }];
        for binding in [1, 3] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        for binding in [2, 4] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
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
        let enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let timing = enabled.then(|| Timing {
            queries: ledger.own(
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("three flights"),
                    ty: wgpu::QueryType::Timestamp,
                    count: (FLIGHTS * 2) as u32,
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
                        size: 16,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ResourceKind::Buffer,
                )
            }),
            flight: None,
        });
        if let Some(error) = scopes.finish() {
            return Err(Error::Fault(Fault(error.to_string())));
        }
        let scopes = OperationScopes::new(&device);
        let pipeline = Self::compile(
            &device,
            &ledger,
            &layout,
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
        if let Some(error) = fault.get() {
            return Err(Error::Fault(error.clone()));
        }
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
            identity: Arc::new(()),
            #[cfg(test)]
            reference_draws: false,
            pipeline,
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
            },
            uploads: UploadBytes::default(),
            submissions: 0,
            completed_submission: 0,
            frames: 0,
            peak_flights: 0,
            busy_uploads: 0,
            busy_frames: 0,
            upload_cpu: CpuFrame::default(),
        };
        // The initial defaults share one immediately submitted flight.
        let scopes = OperationScopes::new(&renderer.device);
        let defaults: [Texture; 2] = [
            asset::TextureRole::BaseColorSrgb,
            asset::TextureRole::MetallicRoughnessLinear,
        ]
        .map(|role| {
            renderer.create_texture(
                &asset::Texture {
                    role,
                    mips: vec![asset::Mip {
                        width: 1,
                        height: 1,
                        rgba8: vec![255; 4],
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
        Ok(renderer)
    }

    fn compile(
        device: &wgpu::Device,
        ledger: &Ledger,
        layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
        samples: SampleCount,
        source: &str,
    ) -> Owned<wgpu::RenderPipeline> {
        let shader = ledger.own(
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("live mesh shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }),
            ResourceKind::ShaderModule,
        );
        ledger.own(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
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
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
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
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples as u32,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        )
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

    pub fn reload(&mut self, wgsl: &str) -> Result<ReloadReport, ReloadError> {
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        let start = Instant::now();
        self.pipelines.attempts += 1;
        let scopes = OperationScopes::new(&self.device);
        let candidate = Self::compile(
            &self.device,
            &self.ledger,
            &self.layout,
            self.format,
            self.options.samples,
            wgsl,
        );
        let error = scopes.finish();
        let compile = start.elapsed();
        if let Some(error) = error {
            return match error {
                wgpu::Error::Validation { .. } => Err(ReloadError::Rejected {
                    diagnostic: error.to_string(),
                    compile,
                }),
                _ => {
                    let _ = self.fault.set(Fault(error.to_string()));
                    Err(ReloadError::Fault(self.fault.get().unwrap().clone()))
                }
            };
        }
        self.pipelines.successes += 1;
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        self.pipeline = candidate;
        self.revision.0 += 1;
        self.pipelines.installations += 1;
        Ok(ReloadReport {
            revision: self.revision,
            compile,
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
        self.check_fault()?;
        let started = Instant::now();
        let sizes = data.validate(self.limits)?;
        let mut preparation = started.elapsed();
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let pack_start = Instant::now();
        let mut bytes = Vec::with_capacity((sizes.0 + sizes.1) as usize);
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
        let bounds = Aabb::from_positions(data.vertices.iter().map(|v| v.position)).unwrap();
        preparation += pack_start.elapsed();
        let start = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        let vertex = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh vertices"),
                size: sizes.0,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let index = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh indices"),
                size: sizes.1,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.check_fault()?;
        self.queue
            .write_buffer(&vertex, 0, &bytes[..sizes.0 as usize]);
        self.uploads.geometry += sizes.0;
        self.uploads.write_calls += 1;
        self.queue
            .write_buffer(&index, 0, &bytes[sizes.0 as usize..]);
        self.uploads.geometry += sizes.1;
        self.uploads.write_calls += 1;
        let upload = start.elapsed();
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
        Ok(Some(Mesh {
            renderer: self.identity.clone(),
            vertex,
            index,
            index_count: data.indices.len() as u32,
            bounds,
        }))
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
                    depth_or_array_layers: 1,
                },
                mip_level_count: data.mips.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: match data.role {
                    asset::TextureRole::BaseColorSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
                    asset::TextureRole::MetallicRoughnessLinear => wgpu::TextureFormat::Rgba8Unorm,
                },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(
            texture.create_view(&Default::default()),
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
                    bytes_per_row: Some(mip.width * 4),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: 1,
                },
            );
            self.uploads.textures += mip.rgba8.len() as u64;
            self.uploads.texture_writes += 1;
            self.uploads.write_calls += 1;
        }
        Texture(Arc::new(crate::material::TextureInner {
            renderer: self.identity.clone(),
            role: data.role,
            _texture: texture,
            view,
            _allocation: self.ledger.allocate(AllocationKind::Texture, bytes),
        }))
    }
    pub fn try_create_material(
        &mut self,
        data: &MaterialDescriptor<'_>,
    ) -> Result<Option<Material>, Error> {
        self.check_fault()?;
        let start = Instant::now();
        data.validate(&self.identity)?;
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
        defaults: &[Texture; 2],
    ) -> Material {
        let textures = [
            data.base_color_texture
                .map_or(&defaults[0], |t| t.texture)
                .clone(),
            data.metallic_roughness_texture
                .map_or(&defaults[1], |t| t.texture)
                .clone(),
        ];
        let samplers = [
            data.base_color_texture
                .map_or(asset::Sampler::default(), |t| t.sampler),
            data.metallic_roughness_texture
                .map_or(asset::Sampler::default(), |t| t.sampler),
        ]
        .map(|s| {
            self.ledger.own(
                self.device
                    .create_sampler(&crate::material::sampler_descriptor(
                        s,
                        self.options.anisotropy_clamp,
                    )),
                ResourceKind::Sampler,
            )
        });
        let uniform = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("immutable material factors"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let bind = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material"),
                layout: &self.material_bindings,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&textures[0].0.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&samplers[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&textures[1].0.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&samplers[1]),
                    },
                ],
            }),
            ResourceKind::BindGroup,
        );
        self.queue.write_buffer(&uniform, 0, &data.bytes());
        self.uploads.materials += 32;
        self.uploads.write_calls += 1;
        Material {
            renderer: self.identity.clone(),
            double_sided: data.double_sided,
            bind,
            _uniform: uniform,
            _samplers: samplers,
            _textures: textures,
            _allocation: self.ledger.allocate(AllocationKind::Material, 32),
        }
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
        self.check_fault()?;
        let preparation_start = Instant::now();
        self.validate_color(color)?;
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
            (view_mode == ViewMode::Normal || draw.material.is_some_and(|m| !m.double_sided))
                && draw.instances.state.invalid_normals > 0
        }) {
            return Err(Error::InvalidInput(
                "normal view and single-sided materials require nonsingular affine transforms with max-element condition estimate <= 1e4",
            ));
        }
        let matrix = camera.matrix(color.width(), color.height());
        if !matrix.is_finite() {
            return Err(Error::InvalidInput(
                "camera aspect produces nonfinite projection",
            ));
        }
        let preparation = preparation_start.elapsed();
        let poll = self.collect()?;
        let Some(slot) = self.free_slot() else {
            self.busy_frames += 1;
            return Ok(None);
        };
        let camera_bytes = matrix_bytes(matrix);
        let mut uploads = UploadBytes::default();
        let mut scene = SceneCounts::default();
        let mut visibility = Duration::ZERO;
        let mut packing = Duration::ZERO;
        let mut upload = Duration::ZERO;
        if self.camera_shadow != Some(camera_bytes) {
            let start = Instant::now();
            self.queue.write_buffer(&self.uniform, 0, &camera_bytes);
            uploads.camera = 64;
            uploads.write_calls += 1;
            upload += start.elapsed();
        }
        if self.view_mode != view_mode {
            let mut bytes = [0; 16];
            bytes[..4].copy_from_slice(&(view_mode as u32).to_ne_bytes());
            let start = Instant::now();
            self.queue.write_buffer(&self.uniform, 64, &bytes);
            uploads.view_settings = 16;
            uploads.write_calls += 1;
            upload += start.elapsed();
        }
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
        self.submit_prepared(
            color,
            camera_bytes,
            view_mode,
            &prepared,
            slot,
            uploads,
            scene,
            CpuFrame {
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

    // Persistent data is prepared once, then borrowed for the shared render pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submit_prepared(
        &mut self,
        color: &wgpu::Texture,
        camera_bytes: [u8; 64],
        view_mode: ViewMode,
        draws: &[PreparedDraw<'_>],
        slot: usize,
        uploads: UploadBytes,
        scene: SceneCounts,
        cpu: CpuFrame,
    ) -> Result<Option<FrameReport>, Error> {
        self.uploads.add(uploads);
        let encode_start = Instant::now();
        if self
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
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
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
        self.check_fault()?;
        let view = self
            .ledger
            .attachment(color.create_view(&Default::default()));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mesh frame"),
            });
        let timed = self.timing_status == TimingStatus::Enabled;
        let timestamps =
            self.timing
                .as_ref()
                .filter(|_| timed)
                .map(|timing| wgpu::RenderPassTimestampWrites {
                    query_set: &timing.queries,
                    beginning_of_pass_write_index: Some(slot as u32 * 2),
                    end_of_pass_write_index: Some(slot as u32 * 2 + 1),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.msaa.as_ref().map_or(&*view, |t| &*t.view),
                    depth_slice: None,
                    resolve_target: self.msaa.as_ref().map(|_| &*view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: CLEAR_COLOR[0],
                            g: CLEAR_COLOR[1],
                            b: CLEAR_COLOR[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().unwrap().view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamps,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &*self.bind, &[]);
            // Pass state persists across draws, so unchanged state is never re-encoded.
            let (mut instances, mut material, mut ids, mut mesh) = (None, None, None, None);
            for draw in draws {
                if draw.slots.is_empty() {
                    continue;
                }
                let bind: &wgpu::BindGroup = &draw.instances.bind;
                if changed(&mut instances, bind) {
                    pass.set_bind_group(1, bind, &[]);
                }
                let bind: &wgpu::BindGroup = &draw
                    .material
                    .unwrap_or(self.default_material.as_ref().unwrap())
                    .bind;
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
                    pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                }
                pass.draw_indexed(draw.indices.clone(), 0, draw.slots.clone());
            }
        }
        let mapped = if timed {
            let timing = self.timing.as_ref().unwrap();
            let readback = self.slots[slot].readback.as_ref().unwrap();
            let offset = slot as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
            encoder.resolve_query_set(
                &timing.queries,
                slot as u32 * 2..slot as u32 * 2 + 2,
                &timing.resolve,
                offset,
            );
            encoder.copy_buffer_to_buffer(&timing.resolve, offset, readback, 0, 16);
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
        let encode = encode_start.elapsed();
        let submitted_at = Instant::now();
        self.queue.submit([commands]);
        let submit = submitted_at.elapsed();
        self.submissions += 1;
        self.frames += 1;
        self.timed_frames += u64::from(timed);
        let frame = FrameId(self.frames);
        self.record_flight(
            slot,
            Flight {
                done,
                mapped,
                submission: self.submissions,
                frame: Some(frame),
                revision: self.revision,
                submitted_at,
            },
        );
        self.check_fault()?;
        self.camera_shadow = Some(camera_bytes);
        self.view_mode = view_mode;
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
        }))
    }

    pub(crate) fn collect(&mut self) -> Result<Duration, Error> {
        self.check_fault()?;
        let start = Instant::now();
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(Error::Poll)?;
        self.check_fault()?;
        for slot in &mut self.slots {
            let Some(flight) = &slot.flight else {
                continue;
            };
            if flight.done.get().is_none()
                || flight.mapped.as_ref().is_some_and(|m| m.get().is_none())
            {
                continue;
            }
            let flight = slot.flight.take().unwrap();
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
                            (begin, end)
                        });
                        readback.unmap();
                        match result {
                            Ok((begin, end)) if end >= begin => {
                                let seconds = (end - begin) as f64
                                    * f64::from(self.timing.as_ref().unwrap().period)
                                    * 1e-9;
                                if let Ok(mesh_pass) = Duration::try_from_secs_f64(seconds) {
                                    gpu = Some(GpuSample {
                                        frame: flight.frame.unwrap(),
                                        revision: flight.revision,
                                        mesh_pass,
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
pub use virtual_gpu::Capture;
