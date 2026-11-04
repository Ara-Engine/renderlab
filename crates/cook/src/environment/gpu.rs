//! Opt-in offline Vulkan preparation. No importer reaches the renderer, and failure never starts CPU convolution.
use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;

pub const KERNEL: &str = include_str!("convolve.wgsl");
pub const KERNEL_SHA256: &str = "96865924434439bafd5b82ea33f4eb1f28071ac4bcc61f60bcb13f43d88571f1";
pub const RECIPE: &[u8] =
    b"renderlab-rle2-quadratic65-spatial128-r4-ggx-sqrt-mis16384-f64-host-compensated-f32-gpu-v1";
const TILE: u32 = 1024;

#[derive(Clone, Copy, Debug)]
pub struct GpuPolicy {
    pub wait_timeout: Duration,
    pub max_tile_time: Duration,
    pub max_wall_time: Duration,
}
impl Default for GpuPolicy {
    fn default() -> Self {
        Self {
            wait_timeout: Duration::from_secs(30),
            max_tile_time: Duration::from_secs(1),
            max_wall_time: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ConvolutionTexel {
    pub level: u32,
    pub face: CubeFace,
    pub x: u32,
    pub y: u32,
}

#[derive(Default, Debug)]
pub struct PreparationReport {
    pub adapter: String,
    pub timestamps: bool,
    pub device_ms: f64,
    pub pipeline_ms: f64,
    pub cpu_tables_ms: f64,
    pub cpu_queries_ms: f64,
    pub cpu_diffuse_delta_ms: f64,
    pub upload_ms: f64,
    pub encode_submit_ms: f64,
    pub completion_map_wait_ms: f64,
    pub readback_host_ms: f64,
    pub dispatch_readback_ms: f64,
    pub gpu_ms: f64,
    pub completed_texels: u64,
    pub sample_evaluations: u64,
    pub gpu_owned_bytes: u64,
    pub total_ms: f64,
}

/// One bounded preparation session. Wall time counts from construction, including
/// idle time, and sample evaluations accumulate across calls. An error can leave
/// readback mapped or pending; discard the preparer after any error and create a
/// new session to retry.
pub struct GpuPreparer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    queries: wgpu::Buffer,
    output: wgpu::Buffer,
    dispatch: wgpu::Buffer,
    staging: wgpu::Buffer,
    timestamps: Option<(wgpu::QuerySet, wgpu::Buffer)>,
    // Bind groups retain common buffer handles; keep explicit ownership visible too.
    _common: [wgpu::Buffer; 4],
    limits: CookLimits,
    policy: GpuPolicy,
    start: Instant,
    pub report: PreparationReport,
}

fn failure(what: impl std::fmt::Display) -> Error {
    Error(format!(
        "offline environment GPU: {what}; no automatic CPU fallback"
    ))
}

impl GpuPreparer {
    pub fn new(input: &LinearEnvironment, limits: CookLimits, policy: GpuPolicy) -> Result<Self> {
        source_texels(input.width, input.height, limits)?;
        if policy.wait_timeout.is_zero()
            || policy.max_tile_time.is_zero()
            || policy.max_wall_time.is_zero()
            || input.width > (1 << 24)
            || input.height > (1 << 24)
            || input.pixels.len() > i32::MAX as usize
        {
            return Err(failure(
                "invalid finite wait policy or shader source addressing",
            ));
        }
        let common_bytes = input.pixels.len() as u64 * 16 + 8192 * 32 + 64 * 8192 * 16 + 64;
        let working =
            common_bytes + input.pixels.len() as u64 * 20 + 8192 * 88 + u64::from(TILE) * 76;
        let preprocessing = input.pixels.len() as u64 * 2 + 8192;
        if working > limits.max_working_bytes || preprocessing > limits.max_sample_evaluations {
            return Err(failure("common input working/work policy"));
        }
        let start = Instant::now();
        let mut report = PreparationReport {
            sample_evaluations: preprocessing,
            ..Default::default()
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::debugging(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).map_err(failure)?;
        report.adapter = format!("{:?}", adapter.get_info());
        eprintln!(
            "offline_environment_adapter={} backend=Vulkan fallback=error",
            report.adapter
        );
        let supported = adapter.limits();
        let largest = (input.pixels.len() as u64 * 16).max(64 * 8192 * 16);
        if largest > supported.max_storage_buffer_binding_size
            || largest > supported.max_buffer_size
            || supported.max_storage_buffers_per_shader_stage < 5
            || supported.max_compute_workgroup_size_x < 128
            || supported.max_compute_invocations_per_workgroup < 128
            || supported.max_compute_workgroup_storage_size < 2048
            || supported.max_compute_workgroups_per_dimension < TILE
        {
            return Err(failure("adapter compute/storage limits"));
        }
        let features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        report.timestamps = !features.is_empty();
        let mut device_limits = wgpu::Limits::default();
        device_limits.max_storage_buffer_binding_size =
            device_limits.max_storage_buffer_binding_size.max(largest);
        device_limits.max_buffer_size = device_limits.max_buffer_size.max(largest);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("offline environment preparer"),
            required_features: features,
            required_limits: device_limits,
            ..Default::default()
        }))
        .map_err(failure)?;
        report.device_ms = start.elapsed().as_secs_f64() * 1000.0;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let at = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("accepted compensated GGX MIS v1"),
            source: wgpu::ShaderSource::Wgsl(KERNEL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("offline environment"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        report.pipeline_ms = at.elapsed().as_secs_f64() * 1000.0;
        let at = Instant::now();
        let tables = specular::gpu_tables(input)?;
        report.cpu_tables_ms = at.elapsed().as_secs_f64() * 1000.0;
        let at = Instant::now();
        let common = std::array::from_fn(|i| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("f64-generated rounded shared tables"),
                contents: &tables[i],
                usage: if i == 3 {
                    wgpu::BufferUsages::UNIFORM
                } else {
                    wgpu::BufferUsages::STORAGE
                },
            })
        });
        drop(tables);
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let queries = buffer(
            "bounded tile frames",
            u64::from(TILE) * 48,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let output = buffer(
            "bounded tile output",
            u64::from(TILE) * 16,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let dispatch = buffer(
            "tile offset",
            16,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let staging = buffer(
            "tile readback and timestamps",
            u64::from(TILE) * 16 + 16,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let buffers = [
            &common[0], &common[1], &common[2], &queries, &output, &common[3], &dispatch,
        ];
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("offline environment"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let timestamps = report.timestamps.then(|| {
            (
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("tile GPU time"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                buffer(
                    "timestamp resolve",
                    256,
                    wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                ),
            )
        });
        queue.write_buffer(&dispatch, 0, &[0; 16]);
        queue.submit([]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(policy.wait_timeout),
            })
            .map_err(failure)?;
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(failure(e));
        }
        report.upload_ms = at.elapsed().as_secs_f64() * 1000.0;
        report.gpu_owned_bytes = common_bytes
            + u64::from(TILE) * (48 + 16 + 16)
            + 32
            + if timestamps.is_some() { 256 } else { 0 };
        Ok(Self {
            device,
            queue,
            pipeline,
            bind,
            queries,
            output,
            dispatch,
            staging,
            timestamps,
            _common: common,
            limits,
            policy,
            start,
            report,
        })
    }

    /// Sparse early gates and full production use this same bounded kernel and f64 host path.
    pub fn prepare_texels(
        &mut self,
        texels: &[ConvolutionTexel],
        scale: f64,
    ) -> Result<Vec<[f32; 3]>> {
        check_scale(scale)?;
        let evaluations = (texels.len() as u64)
            .checked_mul(16384)
            .and_then(|n| n.checked_add(self.report.sample_evaluations))
            .ok_or_else(|| failure("sample counter overflow"))?;
        if texels.len() > 6 * 128 * 128
            || evaluations > self.limits.max_sample_evaluations
            || texels.len() as u64 * 12 > self.limits.max_working_bytes
            || texels
                .iter()
                .any(|t| !(1..65).contains(&t.level) || t.x >= 128 || t.y >= 128)
        {
            return Err(failure("texel count, coordinates or work policy"));
        }
        let mut result = Vec::new();
        result.try_reserve_exact(texels.len()).map_err(failure)?;
        for tile in texels.chunks(TILE as usize) {
            if self.start.elapsed() > self.policy.max_wall_time {
                return Err(failure(format!(
                    "wall limit after {} texels",
                    self.report.completed_texels
                )));
            }
            let at = Instant::now();
            let mut frames = Vec::with_capacity(tile.len() * 48);
            for t in tile {
                let n = t.face.direction([
                    2.0 * (f64::from(t.x) + 0.5) / 128.0 - 1.0,
                    2.0 * (f64::from(t.y) + 0.5) / 128.0 - 1.0,
                ]);
                specular::gpu_query(&mut frames, n, t.level)?;
            }
            self.report.cpu_queries_ms += at.elapsed().as_secs_f64() * 1000.0;
            let at = Instant::now();
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            self.queue.write_buffer(&self.queries, 0, &frames);
            self.queue.write_buffer(&self.dispatch, 0, &[0; 16]);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("bounded 1024 texel GGX MIS"),
                    timestamp_writes: self.timestamps.as_ref().map(|(set, _)| {
                        wgpu::ComputePassTimestampWrites {
                            query_set: set,
                            beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: Some(1),
                        }
                    }),
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.dispatch_workgroups(tile.len() as u32, 1, 1);
            }
            if let Some((set, resolve)) = &self.timestamps {
                encoder.resolve_query_set(set, 0..2, resolve, 0);
                encoder.copy_buffer_to_buffer(resolve, 0, &self.staging, u64::from(TILE) * 16, 16);
            }
            encoder.copy_buffer_to_buffer(
                &self.output,
                0,
                &self.staging,
                0,
                tile.len() as u64 * 16,
            );
            self.queue.submit([encoder.finish()]);
            self.report.encode_submit_ms += at.elapsed().as_secs_f64() * 1000.0;
            let wait_start = Instant::now();
            let (tx, rx) = mpsc::channel();
            self.staging.map_async(wgpu::MapMode::Read, .., move |r| {
                let _ = tx.send(r);
            });
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(self.policy.wait_timeout),
                })
                .map_err(failure)?;
            rx.recv_timeout(self.policy.wait_timeout)
                .map_err(failure)?
                .map_err(failure)?;
            self.report.completion_map_wait_ms += wait_start.elapsed().as_secs_f64() * 1000.0;
            if let Some(e) = pollster::block_on(scope.pop()) {
                return Err(failure(e));
            }
            let mapped = self.staging.get_mapped_range(..).map_err(failure)?;
            let read_start = Instant::now();
            for p in mapped[..tile.len() * 16].chunks_exact(16) {
                if f32::from_le_bytes(p[12..16].try_into().unwrap()) != 16384.0 {
                    return Err(failure("sample coverage marker"));
                }
                let rgb: [f64; 3] = std::array::from_fn(|i| {
                    f64::from(f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())) * scale
                });
                if rgb.iter().any(|v| {
                    !v.is_finite()
                        || *v < 0.0
                        || *v > f64::from(f32::MAX)
                        || (*v > 0.0 && *v as f32 == 0.0)
                }) {
                    return Err(failure("output outside RGB32F range"));
                }
                result.push(rgb.map(|v| v as f32));
            }
            let wall = at.elapsed();
            let duration = if self.timestamps.is_some() {
                let t = &mapped[TILE as usize * 16..TILE as usize * 16 + 16];
                let a = u64::from_le_bytes(t[..8].try_into().unwrap());
                let b = u64::from_le_bytes(t[8..].try_into().unwrap());
                let ns = b.checked_sub(a).ok_or_else(|| failure("timestamp order"))? as f64
                    * f64::from(self.queue.get_timestamp_period());
                self.report.gpu_ms += ns / 1e6;
                Duration::from_secs_f64(ns / 1e9)
            } else {
                wall
            };
            drop(mapped);
            self.staging.unmap();
            self.report.readback_host_ms += read_start.elapsed().as_secs_f64() * 1000.0;
            self.report.dispatch_readback_ms += wall.as_secs_f64() * 1000.0;
            self.report.completed_texels += tile.len() as u64;
            self.report.sample_evaluations += tile.len() as u64 * 16384;
            if duration > self.policy.max_tile_time {
                return Err(failure(format!(
                    "tile time limit after {} completed texels",
                    self.report.completed_texels
                )));
            }
        }
        self.report.total_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        Ok(result)
    }
}

/// Explicit full-field operation. Call only after the sparse precision gate passes for this binary.
pub fn prepare_environment_gpu(
    input: &LinearEnvironment,
    mut metadata: Metadata,
    limits: CookLimits,
    policy: GpuPolicy,
) -> Result<(PreparedEnvironment, PreparationReport)> {
    metadata.validate()?;
    let cost = estimate_field_cost(input, limits)?;
    let start = Instant::now();
    let mut hash = Sha256::new();
    hash.update(RECIPE);
    hash.update(Sha256::digest(KERNEL.as_bytes()));
    for v in [
        input.width,
        input.height,
        16,
        128,
        16384,
        65,
        metadata.source_radiance_scale.to_bits(),
        metadata.rotation_y_radians.to_bits(),
    ] {
        hash.update(v.to_le_bytes());
    }
    hash.update([match metadata.units {
        asset::RadianceUnits::CandelasPerSquareMetre => 1,
        asset::RadianceUnits::Relative => 2,
    }]);
    for r in asset::roughness_knots() {
        hash.update(r.to_le_bytes());
    }
    for p in &input.pixels {
        for c in p {
            hash.update(c.to_le_bytes());
        }
    }
    metadata.generator_hash = hash.finalize().into();
    let mut gpu = GpuPreparer::new(input, limits, policy)?;
    let cpu_start = Instant::now();
    let diffuse = convolve_diffuse_irradiance(
        input,
        FIELD_SETTINGS,
        f64::from(metadata.source_radiance_scale),
        limits,
    )?;
    gpu.report.sample_evaluations += 6 * 16 * 16 * 16384 + 16384 + 6 * 128 * 128;
    let mut levels = Vec::with_capacity(asset::FIELD_LEVELS);
    levels.push(make_cube(128, limits, |n| {
        input
            .sample(n)
            .map(|v| v * f64::from(metadata.source_radiance_scale))
    })?);
    gpu.report.cpu_diffuse_delta_ms = cpu_start.elapsed().as_secs_f64() * 1000.0;
    for level in 1..65 {
        let mut values = Vec::with_capacity(6 * 128 * 128);
        let mut records = Vec::with_capacity(TILE as usize);
        for face in CubeFace::ALL {
            for y in 0..128 {
                for x in 0..128 {
                    records.push(ConvolutionTexel { level, face, x, y });
                    if records.len() == TILE as usize {
                        values.extend(
                            gpu.prepare_texels(
                                &records,
                                f64::from(metadata.source_radiance_scale),
                            )?,
                        );
                        records.clear();
                    }
                }
            }
        }
        if !records.is_empty() {
            values.extend(gpu.prepare_texels(&records, f64::from(metadata.source_radiance_scale))?);
        }
        levels.push(CubeLevel::new(128, values, limits.prepared)?);
        eprintln!(
            "offline_environment_level={level}/64 completed_evaluations={}",
            gpu.report.sample_evaluations
        );
    }
    gpu.report.sample_evaluations = cost.sample_evaluations;
    gpu.report.total_ms = start.elapsed().as_secs_f64() * 1000.0;
    let prepared = PreparedEnvironment::new_independent(
        metadata,
        diffuse,
        levels,
        asset::roughness_knots(),
        asset::FieldGenerator::F64HostF32GpuV1,
        limits.prepared,
    )?;
    Ok((prepared, gpu.report))
}
