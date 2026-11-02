use crate::{
    instances::InstanceState,
    mesh::matrix_bytes,
    observe::{EventQueue, Ledger, Observation, Owned},
    visibility::Aabb,
    *,
};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

const FLIGHTS: usize = 3;
const CLEAR_COLOR: [f64; 3] = [0.018, 0.025, 0.04];
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const MODEL_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![2 => Uint32];

type MapResult = Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>;
struct Flight {
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
    pub(crate) fn test_capture_full(&mut self, color: &wgpu::Texture) -> (Vec<u8>, Vec<u8>) {
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
}

/// One queue submission owner, three flights, and one depth texture keyed by color extent.
/// Alternating target sizes recreates depth. Separate renderers suit independent target streams.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    limits: Limits,
    fault: Arc<OnceLock<Fault>>,
    ledger: Ledger,
    layout: Owned<wgpu::PipelineLayout>,
    _bindings: Owned<wgpu::BindGroupLayout>,
    bind: Owned<wgpu::BindGroup>,
    uniform: Owned<wgpu::Buffer>,
    instance_bindings: Owned<wgpu::BindGroupLayout>,
    identity: Arc<()>,
    #[cfg(test)]
    reference_draws: bool,
    pipeline: Owned<wgpu::RenderPipeline>,
    depth: Option<Depth>,
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
    frames: u64,
    peak_flights: usize,
    busy_uploads: u64,
    busy_frames: u64,
    upload_cpu: CpuFrame,
}

// Native wgpu scope futures are immediately ready. This never polls for GPU completion.
struct OperationScopes([wgpu::ErrorScopeGuard; 3]);
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
                size: 64,
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
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
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
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                }],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fixed mesh ABI"),
                bind_group_layouts: &[Some(&bindings), Some(&instance_bindings)],
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
        let pipeline = Self::compile(&device, &ledger, &layout, color_format, wgsl);
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => Error::Fault(Fault(error.to_string())),
            });
        }
        if let Some(error) = fault.get() {
            return Err(Error::Fault(error.clone()));
        }
        Ok(Self {
            device,
            queue,
            format: color_format,
            limits,
            fault,
            ledger,
            layout,
            _bindings: bindings,
            bind,
            uniform,
            instance_bindings,
            identity: Arc::new(()),
            #[cfg(test)]
            reference_draws: false,
            pipeline,
            depth: None,
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
            frames: 0,
            peak_flights: 0,
            busy_uploads: 0,
            busy_frames: 0,
            upload_cpu: CpuFrame::default(),
        })
    }

    fn compile(
        device: &wgpu::Device,
        ledger: &Ledger,
        layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
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
                            array_stride: 24,
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
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        )
    }

    fn check_fault(&self) -> Result<(), Error> {
        match self.fault.get() {
            Some(fault) => Err(Error::Fault(fault.clone())),
            None => Ok(()),
        }
    }
    fn fail(&self, message: String) -> Error {
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
        let candidate = Self::compile(&self.device, &self.ledger, &self.layout, self.format, wgsl);
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

    fn occupied(&self) -> usize {
        self.slots.iter().filter(|s| s.flight.is_some()).count()
    }
    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.flight.is_none())
    }
    fn record_flight(&mut self, slot: usize, flight: Flight) {
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
            for value in vertex.position.into_iter().chain(vertex.color) {
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
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: transforms.as_entire_binding(),
                }],
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
        for &id in &state.ids {
            self.scratch.extend_from_slice(&id.to_ne_bytes());
        }
        let packing = packing + pack_start.elapsed();
        let upload_start = Instant::now();
        self.queue.write_buffer(&id_buffer, 0, &self.scratch);
        self.uploads.add(UploadBytes {
            instances: models.len() as u64 * 64,
            visible_ids: models.len() as u64 * 4,
            visible_id_writes: 1,
            write_calls: 2,
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
            id_buffer,
            bind,
        }))
    }

    fn validate_color(&self, color: &wgpu::Texture) -> Result<(), Error> {
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
        self.check_fault()?;
        let preparation_start = Instant::now();
        self.validate_color(color)?;
        if draws
            .iter()
            .any(|draw| !Arc::ptr_eq(&self.identity, &draw.instances.renderer))
        {
            return Err(Error::InvalidInput("instances belong to another renderer"));
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
            if instances.state.dirty_count > 0 {
                for span in instances.state.dirty_runs() {
                    let start = Instant::now();
                    self.scratch.clear();
                    for &model in &instances.state.models[span.clone()] {
                        self.scratch.extend_from_slice(&matrix_bytes(model));
                    }
                    packing += start.elapsed();
                    let start = Instant::now();
                    self.queue.write_buffer(
                        &instances.transforms,
                        span.start as u64 * 64,
                        &self.scratch,
                    );
                    uploads.models += self.scratch.len() as u64;
                    uploads.model_writes += 1;
                    uploads.write_calls += 1;
                    upload += start.elapsed();
                }
                instances.state.clear_dirty();
            }
            scene.collections += 1;
            scene.visibility_cache_bytes += instances.state.visibility_cache_bytes() as u64;
            scene.total_instances += instances.len() as u64;
            let visible = instances.state.ids.len() as u64;
            scene.visible_instances += visible;
            scene.submitted_instances += visible;
            scene.draws += u64::from(visible > 0);
            scene.triangles += visible * u64::from(draw.mesh.index_count / 3);
        }
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
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | if cfg!(test) {
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
            self.depth = Some(Depth { texture, view });
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
        #[cfg(test)]
        let unculled_identity_buffers: Vec<_> = if self.reference_draws {
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
            Vec::new()
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
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
            #[cfg(test)]
            let mut unculled_identity_buffers = unculled_identity_buffers.iter();
            for draw in draws.iter() {
                let count = draw.instances.state.ids.len() as u32;
                #[cfg(test)]
                let count = if self.reference_draws {
                    draw.instances.len() as u32
                } else {
                    count
                };
                if count == 0 {
                    continue;
                }
                pass.set_bind_group(1, &*draw.instances.bind, &[]);
                pass.set_vertex_buffer(1, draw.instances.id_buffer.slice(..));
                pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                #[cfg(test)]
                if self.reference_draws {
                    pass.set_vertex_buffer(1, unculled_identity_buffers.next().unwrap().slice(..));
                    for slot in 0..draw.instances.len() as u32 {
                        pass.draw_indexed(0..draw.mesh.index_count, 0, slot..slot + 1);
                    }
                    scene.draws += u64::from(count) - 1;
                    continue;
                }
                pass.draw_indexed(0..draw.mesh.index_count, 0, 0..count);
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
        Ok(Some(FrameReport {
            frame,
            revision: self.revision,
            cpu: CpuFrame {
                preparation,
                visibility,
                packing,
                upload,
                encode,
                submit,
                poll,
            },
            uploads,
            timed,
            scene,
        }))
    }

    fn collect(&mut self) -> Result<Duration, Error> {
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
