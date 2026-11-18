//! The live reference's GPU side (src/reference_live.rs, docs/design-reference-live.md): the
//! wavefront path tracer on the last prepared frame's own scene. Its TLAS is the hardware
//! shadows' (with vertex return), its geometry tables and materials are the frame's traced
//! shading groups 1 and 2, its lighting group the cell shading's group 3, and each program the
//! frame's draws run is composed as `Stage::Reference`. Measurement only: it blocks until done.
use super::*;
use crate::reference_live::{LiveInputs, RF_HIT_BYTES, RF_PATH_BYTES, RF_RECORD_BYTES};

/// Bounces between checks of the live-path counter. A sample ends once every path has ended
/// (src/reference.rs's paths run until roulette ends them, up to `LiveInputs::max_depth`).
const CHECK_EVERY: u32 = 8;
const GROUP: u32 = 64;

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}
fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}
fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
    )
}

impl Renderer {
    /// Each draw's program id, in batch order, for the live reference: kept while a live reference
    /// is requested (`reference_live::requested`), so other frames pay nothing.
    pub(super) fn remember_live_programs(&mut self, draws: &[PreparedDraw<'_>]) {
        if !crate::reference_live::requested() {
            return;
        }
        let standard = self.standard.id();
        self.live_programs = draws.iter().map(|d| d.material.map_or(standard, |m| m.program.id())).collect();
        self.live_transparent = draws.iter().map(|d| d.material.is_some_and(|m| m.transparency.is_some())).collect();
    }

    /// While set, frames keep transparent draws in the traced draw list instead of the transparent pass
    /// (`try_render_view`): the live reference renders one such frame before it traces, so the TLAS,
    /// shading groups and per-program materials include see-through surfaces.
    pub(crate) fn set_live_opaque_frame(&mut self, on: bool) {
        self.live_opaque = on;
    }

    /// Path-traces the last prepared frame's scene from `inputs`' camera (module doc). Needs ray
    /// queries and a traced frame prepared before it (hardware shadows on).
    pub(crate) fn trace_live_reference(&mut self, inputs: &LiveInputs) -> Result<crate::reference::Image, Error> {
        let bad = |m| Err(Error::InvalidInput(m));
        let Some(hardware) = self.hardware_shadows.as_ref() else {
            return bad("the live reference needs hardware ray tracing (hardware shadows)");
        };
        let Some(primary) = self.view.primary.as_ref() else {
            return bad("the live reference needs a traced frame prepared before it");
        };
        let Some((arrays, materials)) = primary.shading_groups() else {
            return bad("the live reference needs the frame's traced shading groups");
        };
        if self.live_programs.is_empty() {
            return bad("the live reference has no prepared draws");
        }
        let [width, height] = inputs.size;
        let pixels = u64::from(width) * u64::from(height);
        let device: &wgpu::Device = &self.device;
        let scopes = OperationScopes::new(device);

        // The kernels: begin, trace, shade, end (shaders/reference_live.wgsl).
        let kernel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live reference kernels"),
            entries: &[
                uniform(0),
                entry(1, wgpu::BindingType::AccelerationStructure { vertex_return: true }),
                storage(2, false),
                storage(3, false),
                storage(4, true),
                storage(5, false),
                storage(6, true),
                storage(7, true),
                storage(8, false),
                storage(9, true),
            ],
        });
        let kernel_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live reference kernels"),
            bind_group_layouts: &[Some(&kernel_layout), Some(primary.array_layout()), Some(primary.material_layout())],
            immediate_size: 0,
        });
        let kernels = self.device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live reference kernels"),
            source: wgpu::ShaderSource::Wgsl(crate::reference_live::kernel_source().into()),
        });
        let kernel = |name: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&kernel_pipeline_layout),
                module: &kernels,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let [begin, trace, shade, end] = ["rf_begin", "rf_trace", "rf_shade", "rf_end"].map(kernel);

        // One material pipeline per program the frame's draws run (Stage::Reference).
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live reference materials"),
            entries: &[storage(90, true), storage(91, false), uniform(92), storage(93, true)],
        });
        let material_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live reference materials"),
            bind_group_layouts: &[
                Some(&material_layout),
                Some(primary.array_layout()),
                Some(primary.material_layout()),
                Some(&self.lighting.bindings),
            ],
            immediate_size: 0,
        });
        let heads = primary_ray_gpu::shading_heads(&self.source)?;
        let mut programs: Vec<u64> = self.live_programs.clone();
        programs.sort_unstable();
        programs.dedup();
        let mut material_pipelines = Vec::with_capacity(programs.len());
        for &id in &programs {
            let source = &self.program_slot(id).source;
            let composed = crate::program::compose(
                &heads[1],
                source,
                crate::program::Stage::Reference,
                crate::program::declares_parameters(source),
            );
            let module = self.device.create_program_module(wgpu::ShaderModuleDescriptor {
                label: Some("live reference material"),
                source: wgpu::ShaderSource::Wgsl(composed.source.into()),
            });
            material_pipelines.push(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("rf_materials"),
                layout: Some(&material_pipeline_layout),
                module: &module,
                entry_point: Some("rf_materials"),
                compilation_options: Default::default(),
                cache: None,
            }));
        }

        // Buffers.
        use wgpu::util::DeviceExt;
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false })
        };
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let frame = buffer("live reference frame", crate::reference_live::FRAME_BYTES as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let paths = buffer("live reference paths", pixels * RF_PATH_BYTES, rw);
        let hits = buffer("live reference hits", pixels * RF_HIT_BYTES, rw);
        let records = buffer("live reference records", pixels * RF_RECORD_BYTES, rw);
        let sums = buffer("live reference sums", pixels * 32, rw);
        let counters = buffer("live reference counters", 16, rw);
        let environment = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live reference environment"),
            contents: &crate::reference_live::environment_bytes(inputs),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let lights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live reference lights"),
            contents: &crate::reference_live::light_bytes(inputs),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let program_table: Vec<u8> = self
            .live_programs
            .iter()
            .flat_map(|id| (programs.binary_search(id).unwrap() as u32).to_le_bytes())
            .collect();
        let batch_flags: Vec<u8> = self.live_transparent.iter().flat_map(|&t| u32::from(t).to_le_bytes()).collect();
        let batch_flags = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live reference draw flags"),
            contents: &batch_flags,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let program_table = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live reference programs"),
            contents: &program_table,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let jobs: Vec<wgpu::Buffer> = (0..programs.len() as u32)
            .map(|k| {
                let job: Vec<u8> = [pixels as u32, k, 0, 0].iter().flat_map(|w| w.to_le_bytes()).collect();
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("live reference job"),
                    contents: &job,
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            })
            .collect();
        let readback = buffer("live reference readback", pixels * 32, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
        let counter_readback = buffer("live reference counter readback", 16, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);

        let kernel_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live reference kernels"),
            layout: &kernel_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: frame.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::AccelerationStructure(hardware.scene()) },
                wgpu::BindGroupEntry { binding: 2, resource: paths.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: hits.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: records.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: sums.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: environment.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: counters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: batch_flags.as_entire_binding() },
            ],
        });
        let material_groups: Vec<wgpu::BindGroup> = jobs
            .iter()
            .map(|job| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("live reference materials"),
                    layout: &material_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 90, resource: hits.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 91, resource: records.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 92, resource: job.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 93, resource: program_table.as_entire_binding() },
                    ],
                })
            })
            .collect();
        let lighting = self.lighting.cell_bind();
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(error.to_string()));
        }

        let groups = u32::try_from(pixels.div_ceil(u64::from(GROUP))).map_err(|_| Error::InvalidInput("live reference image too large"))?;
        // The materials' entry strides over the queue: at most 65535 groups a dispatch.
        let material_groups_x = groups.min(65535);
        let dispatch = |encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("live reference"), timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &kernel_group, &[]);
            pass.set_bind_group(1, arrays, &[]);
            pass.set_bind_group(2, materials, &[]);
            pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
        };
        let wait = |device: &wgpu::Device| {
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(600)) }).map(|_| ())
        };
        let mut cut = 0u64;
        let started = Instant::now();
        for sample in 0..inputs.samples {
            self.queue.write_buffer(&frame, 0, &crate::reference_live::frame_bytes(inputs, sample));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("live reference sample") });
            dispatch(&mut encoder, &begin);
            let mut bounce = 0;
            loop {
                for _ in 0..CHECK_EVERY {
                    encoder.clear_buffer(&counters, 0, None);
                    dispatch(&mut encoder, &trace);
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("live reference materials"), timestamp_writes: None });
                        pass.set_bind_group(1, arrays, &[]);
                        pass.set_bind_group(2, materials, &[]);
                        pass.set_bind_group(3, lighting, &[]);
                        for (pipeline, group) in material_pipelines.iter().zip(&material_groups) {
                            pass.set_pipeline(pipeline);
                            pass.set_bind_group(0, group, &[]);
                            pass.dispatch_workgroups(material_groups_x, 1, 1);
                        }
                    }
                    dispatch(&mut encoder, &shade);
                    bounce += 1;
                }
                encoder.copy_buffer_to_buffer(&counters, 0, &counter_readback, 0, 16);
                self.queue.submit([encoder.finish()]);
                counter_readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                wait(device).map_err(|_| Error::InvalidInput("live reference wait failed"))?;
                let alive = {
                    let view = counter_readback.slice(..).get_mapped_range().map_err(|_| Error::InvalidInput("live reference counter mapping failed"))?;
                    u32::from_le_bytes(view[0..4].try_into().unwrap())
                };
                counter_readback.unmap();
                encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("live reference sample") });
                if alive == 0 || bounce >= inputs.max_depth {
                    break;
                }
            }
            encoder.clear_buffer(&counters, 0, None);
            dispatch(&mut encoder, &end);
            encoder.copy_buffer_to_buffer(&counters, 0, &counter_readback, 0, 16);
            self.queue.submit([encoder.finish()]);
            counter_readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            wait(device).map_err(|_| Error::InvalidInput("live reference wait failed"))?;
            {
                let view = counter_readback.slice(..).get_mapped_range().map_err(|_| Error::InvalidInput("live reference counter mapping failed"))?;
                cut += u64::from(u32::from_le_bytes(view[4..8].try_into().unwrap()));
            }
            counter_readback.unmap();
            if inputs.progress && sample.is_power_of_two() {
                eprintln!("live_reference sample={} seconds={:.1}", sample + 1, started.elapsed().as_secs_f64());
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("live reference readback") });
        encoder.copy_buffer_to_buffer(&sums, 0, &readback, 0, pixels * 32);
        self.queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        wait(device).map_err(|_| Error::InvalidInput("live reference wait failed"))?;
        let bytes = readback.slice(..).get_mapped_range().map_err(|_| Error::InvalidInput("live reference readback mapping failed"))?.to_vec();
        readback.unmap();
        if cut > 0 {
            eprintln!("live_reference: {cut} paths reached the bounce limit {} and were cut", inputs.max_depth);
        }
        Ok(crate::reference::finish_sums(&bytes, width, height, inputs.samples))
    }
}
