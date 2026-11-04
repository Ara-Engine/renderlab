//! Bounded Phase-A experiment. This module is nested under fresh_shadow_tests so
//! it shares the registered quarry loader and readback/validation helpers.
use super::*;

const TRACE: &str = include_str!("../shaders/primary_ray_probe.wgsl");
const RECEIVER: &str = include_str!("../shaders/primary_ray_receiver_probe.wgsl");
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

#[test]
fn primary_probe_wgsl_compiles_to_spirv() {
    for source in [TRACE, RECEIVER] {
        let (module, info) = validate_wgsl(source);
        for entry in &module.entry_points {
            for diagnostic in [false, true] {
                let mut constants = naga::back::PipelineConstants::default();
                if source == TRACE {
                    constants.insert("WRITE_WORLD".into(), f64::from(diagnostic));
                }
                let (resolved, info) = naga::back::pipeline_constants::process_overrides(
                    &module,
                    &info,
                    Some((entry.stage, &entry.name)),
                    &constants,
                )
                .unwrap();
                naga::back::spv::write_vec(
                    &resolved,
                    &info,
                    &Default::default(),
                    Some(&naga::back::spv::PipelineOptions {
                        shader_stage: entry.stage,
                        entry_point: entry.name.clone(),
                    }),
                )
                .unwrap();
            }
        }
    }
}

/// Both traced modules compile to SPIR-V on the CPU, every entry the renderer
/// dispatches matches its Rust grid, and lit-body drift is rejected before compile.
#[test]
fn primary_modules_compile_and_reject_lit_seam_drift() {
    let sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    assert!(sources[0].contains("var hardware_scene") && !sources[0].contains("fresh_visibility:"));
    assert!(sources[1].contains("fresh_visibility:") && !sources[1].contains("fn primary_trace"));
    let [[x, y], [u, v]] = [primary_ray_gpu::TRACE_TILE, primary_ray_gpu::SHADE_TILE];
    let grids = [
        ("primary_trace", [x, y, 1]),
        ("primary_trace_receivers", [x, y, 1]),
        ("primary_shade", [u, v, 1]),
    ];
    for (index, source) in sources.iter().enumerate() {
        let (module, info) = validate_wgsl(source);
        for (name, grid) in &grids[if index == 0 { 0.. } else { 2.. }] {
            let entry = module
                .entry_points
                .iter()
                .find(|e| e.name == *name)
                .unwrap();
            assert_eq!(entry.workgroup_size, *grid, "{name} dispatch grid drift");
            naga::back::spv::write_vec(
                &module,
                &info,
                &naga::back::spv::Options {
                    lang_version: (1, 3),
                    ..Default::default()
                },
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: entry.stage,
                    entry_point: name.to_string(),
                }),
            )
            .unwrap();
        }
    }
    for drift in [
        SOURCE.replace(
            "textureSample(base_color,base_sampler,input.uv)",
            "vec4<f32>(1.0)",
        ),
        SOURCE.replace(
            "let mr=textureSample(metallic_roughness,mr_sampler,input.uv);",
            "let mr=textureSample(metallic_roughness,mr_sampler,input.uv)+textureSample(base_color,base_sampler,input.uv*2.0);",
        ),
        SOURCE.replace("{ discard; }", "{ return vec4<f32>(0.0); }"),
    ] {
        assert!(primary_ray_gpu::shader_sources(&drift).is_err());
    }
}

struct Image {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}
impl Image {
    fn new(r: &Renderer, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Self {
        let texture = r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("primary probe diagnostic"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: usage | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}

struct Probe {
    camera: wgpu::Buffer,
    trace: [wgpu::ComputePipeline; 2],
    trace_bind: wgpu::BindGroup,
    receiver: wgpu::RenderPipeline,
    instance_bases: Vec<wgpu::BindGroup>,
    hits: Image,
    depths: Image,
    worlds: Image,
    raster_world: Image,
    raster_ids: Image,
}
#[derive(Clone, Copy)]
enum RasterMode {
    Production,
    Ids,
    Triangle { draw: usize, primitive: u32 },
}
impl Probe {
    fn new(r: &Renderer, draws: &[Draw<'_>]) -> Self {
        let storage = wgpu::TextureUsages::STORAGE_BINDING;
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let hits = Image::new(r, wgpu::TextureFormat::Rgba32Uint, storage);
        let depths = Image::new(r, wgpu::TextureFormat::R32Float, storage);
        let worlds = Image::new(r, wgpu::TextureFormat::Rgba32Float, storage);
        let raster_world = Image::new(r, wgpu::TextureFormat::Rgba32Float, attachment);
        let raster_ids = Image::new(r, wgpu::TextureFormat::Rgba32Uint, attachment);
        let camera = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("primary probe camera"),
            size: 144,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let trace_module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("primary closest hit probe"),
            source: wgpu::ShaderSource::Wgsl(TRACE.into()),
        });
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(144),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
                count: None,
            },
        ];
        for (binding, format) in [
            (2, wgpu::TextureFormat::Rgba32Uint),
            (3, wgpu::TextureFormat::R32Float),
            (4, wgpu::TextureFormat::Rgba32Float),
        ] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            });
        }
        let trace_layout = r
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("primary probe trace inputs"),
                entries: &entries,
            });
        let layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("primary probe shared trace layout"),
                bind_group_layouts: &[Some(&trace_layout)],
                immediate_size: 0,
            });
        let trace = [false, true].map(|diagnostic| {
            r.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("primary closest hit probe"),
                    layout: Some(&layout),
                    module: &trace_module,
                    entry_point: Some("primary_probe"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("WRITE_WORLD", if diagnostic { 1.0 } else { 0.0 })],
                        ..Default::default()
                    },
                    cache: None,
                })
        });
        let trace_bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("primary probe scene"),
            layout: &trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: r.hardware_shadows.as_ref().unwrap().scene().as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&hits.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&depths.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&worlds.view),
                },
            ],
        });
        let receiver_module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("primary probe raster IDs"),
            source: wgpu::ShaderSource::Wgsl(RECEIVER.into()),
        });
        let receiver = r
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("primary probe raster IDs"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &receiver_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 48,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &VERTEX_ATTRIBUTES[..1],
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 4,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &MODEL_ATTRIBUTES,
                        }),
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &receiver_module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::TextureFormat::Rgba32Float.into()),
                        Some(wgpu::TextureFormat::Rgba32Uint.into()),
                    ],
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
            });
        let mut base = 0u32;
        let instance_bases = draws
            .iter()
            .map(|d| {
                let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("primary probe global instance base"),
                    size: 32,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                r.queue.write_buffer(&buffer, 0, &base.to_le_bytes());
                base += d.instances.len() as u32;
                r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("primary probe global instance base"),
                    layout: &receiver.get_bind_group_layout(2),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            })
            .collect();
        Self {
            camera,
            trace,
            trace_bind,
            receiver,
            instance_bases,
            hits,
            depths,
            worlds,
            raster_world,
            raster_ids,
        }
    }

    fn update_camera(&self, r: &Renderer, camera: Camera) {
        let matrix = camera.matrix(WIDTH, HEIGHT);
        let mut bytes = matrix_bytes(matrix).to_vec();
        r.queue.write_buffer(&r.uniform, 0, &bytes);
        bytes.extend(matrix_bytes(matrix.inverse()));
        bytes.extend(WIDTH.to_le_bytes());
        bytes.extend(HEIGHT.to_le_bytes());
        bytes.extend([0; 8]);
        r.queue.write_buffer(&self.camera, 0, &bytes);
    }

    fn trace(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        diagnostic: bool,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("primary closest hit"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.trace[usize::from(diagnostic)]);
        pass.set_bind_group(0, &self.trace_bind, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(4), 1);
    }

    /// Diagnostic and original receiver use the same draws, ID stream and depth target.
    fn raster(
        &self,
        r: &Renderer,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[Draw<'_>],
        mode: RasterMode,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let diagnostic = !matches!(mode, RasterMode::Production);
        let targets = if diagnostic {
            [&self.raster_world.view, &self.raster_ids.view]
        } else {
            let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
            [&*t.world.view, &*t.normal.view]
        };
        let attachments = targets.map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("primary probe receiver control"),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &r.depth.as_ref().unwrap().view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: timestamps,
            ..Default::default()
        });
        if diagnostic {
            // Auto layouts cannot reuse the renderer's explicit bind groups.
            let camera = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("primary probe raster camera"),
                layout: &self.receiver.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: r.uniform.as_entire_binding(),
                }],
            });
            pass.set_bind_group(0, &camera, &[]);
        } else {
            pass.set_bind_group(0, &*r.bind, &[]);
        }
        for (i, draw) in draws.iter().enumerate() {
            if matches!(mode, RasterMode::Triangle {draw, ..} if draw != i) {
                continue;
            }
            // The quarry's twelve placements are all in view at both registered poses.
            assert_eq!(draw.instances.state.ids.len(), draw.instances.len());
            pass.set_pipeline(if diagnostic {
                &self.receiver
            } else {
                &r.pipeline.fresh.as_ref().unwrap().receiver
            });
            if diagnostic {
                let models = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("primary probe raster models"),
                    layout: &self.receiver.get_bind_group_layout(1),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: draw.instances.transforms.as_entire_binding(),
                    }],
                });
                pass.set_bind_group(1, &models, &[]);
                pass.set_bind_group(2, &self.instance_bases[i], &[]);
            } else {
                pass.set_bind_group(1, &*draw.instances.bind, &[]);
            }
            pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
            pass.set_vertex_buffer(1, draw.instances.id_buffer.slice(..));
            pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            let indices = match mode {
                RasterMode::Triangle { primitive, .. } => primitive * 3..primitive * 3 + 3,
                _ => 0..draw.mesh.index_count,
            };
            pass.draw_indexed(indices, 0, 0..draw.instances.len() as u32);
        }
    }
}

#[test]
#[ignore = "offscreen GPU slot, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_primary_ray_quarry_probe() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let validate = manifest["validate"].as_bool().unwrap_or(true);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer_with_features(validate, wgpu::Features::PRIMITIVE_INDEX);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let color = target(&r, WIDTH, HEIGHT);
    render(
        &mut r,
        &color,
        &cameras.frames[0].camera().unwrap(),
        &mut asset.draws(Visibility::Frustum),
    );
    r.test_drain("primary probe existing AS and targets");
    let draws = asset.draws(Visibility::Frustum);
    assert_eq!(draws.len(), 12, "probe requires the registered full quarry");
    assert_eq!(
        draws
            .iter()
            .map(|d| u64::from(d.mesh.index_count / 3) * d.instances.len() as u64)
            .sum::<u64>(),
        13_034_903,
        "probe must trace the full source geometry"
    );
    let scopes = OperationScopes::new(&r.device);
    let probe = Probe::new(&r, &draws);
    assert!(scopes.finish().is_none(), "probe pipelines rejected");
    if manifest["compare"].as_bool().unwrap_or(true) {
        for (index, pose) in [0, 1, 0].into_iter().enumerate() {
            let camera = cameras.frames[pose].camera().unwrap();
            probe.update_camera(&r, camera);
            let mut encoder = r.device.create_command_encoder(&Default::default());
            probe.raster(&r, &mut encoder, &draws, RasterMode::Production, None);
            probe.raster(&r, &mut encoder, &draws, RasterMode::Ids, None);
            probe.trace(&mut encoder, true, None);
            r.queue.submit([encoder.finish()]);
            let world = read_texture(&r, &probe.raster_world.texture);
            let production = read_texture(
                &r,
                &r.fresh
                    .as_ref()
                    .unwrap()
                    .targets
                    .as_ref()
                    .unwrap()
                    .world
                    .texture,
            );
            assert_eq!(
                world
                    .chunks_exact(16)
                    .zip(production.chunks_exact(16))
                    .filter(|(a, b)| a != b)
                    .count(),
                0,
                "diagnostic receiver changed production world/depth"
            );
            let prefix = out.join(format!("pose-{index}"));
            fs::write(prefix.with_extension("raster-world.bin"), world).unwrap();
            fs::write(
                prefix.with_extension("raster-ids.bin"),
                read_texture(&r, &probe.raster_ids.texture),
            )
            .unwrap();
            fs::write(
                prefix.with_extension("trace-world.bin"),
                read_texture(&r, &probe.worlds.texture),
            )
            .unwrap();
            fs::write(
                prefix.with_extension("trace-hits.bin"),
                read_texture(&r, &probe.hits.texture),
            )
            .unwrap();
            fs::write(
                prefix.with_extension("trace-depth.bin"),
                read_texture(&r, &probe.depths.texture),
            )
            .unwrap();
            fs::write(prefix.with_extension("camera.json"),serde_json::to_vec_pretty(&serde_json::json!({"matrix":camera.matrix(WIDTH,HEIGHT).to_cols_array(),"pose":pose})).unwrap()).unwrap();
        }
        // Four non-edge winner disagreements from the first exhaustive screen.
        // Render each competing triangle alone to measure actual hardware depth
        // quantization, rather than inferring ties from the ray's different depth.
        assert!(draws.iter().all(|d| d.instances.len() == 1));
        let mut isolated = vec![];
        for (pose, x, y) in [(0, 591, 617), (0, 844, 622), (1, 1065, 514), (1, 852, 707)] {
            probe.update_camera(&r, cameras.frames[pose].camera().unwrap());
            let offset = (y * WIDTH + x) as usize * 16;
            for (side, suffix) in [("raster", "raster-ids"), ("trace", "trace-hits")] {
                let ids = fs::read(out.join(format!("pose-{pose}.{suffix}.bin"))).unwrap();
                let instance = u32::from_le_bytes(ids[offset..offset + 4].try_into().unwrap());
                let primitive = u32::from_le_bytes(ids[offset + 4..offset + 8].try_into().unwrap());
                let mut encoder = r.device.create_command_encoder(&Default::default());
                probe.raster(
                    &r,
                    &mut encoder,
                    &draws,
                    RasterMode::Triangle {
                        draw: instance as usize - 1,
                        primitive,
                    },
                    None,
                );
                r.queue.submit([encoder.finish()]);
                let world = read_texture(&r, &probe.raster_world.texture);
                let values: Vec<_> = world[offset..offset + 16]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                isolated.push(serde_json::json!({"pose":pose,"x":x,"y":y,"side":side,"instance":instance,"primitive":primitive,"world_depth":values}));
            }
        }
        fs::write(
            out.join("isolated-depths.json"),
            serde_json::to_vec_pretty(&isolated).unwrap(),
        )
        .unwrap();
    }
    // Fresh-screen method: 12 warmup frames, then 24 receiver/trace pairs, with
    // camera alternating after each pair. Unique queries; no per-frame drain.
    const WARMUP: usize = 12;
    const FRAMES: usize = 48;
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("primary probe pairs"),
        ty: wgpu::QueryType::Timestamp,
        count: (FRAMES * 2) as u32,
    });
    let size = (FRAMES * 16) as u64;
    let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("primary probe resolve"),
        size,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("primary probe read"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut labels = vec![];
    for frame in 0..WARMUP + FRAMES {
        let k = if frame < WARMUP {
            frame
        } else {
            frame - WARMUP
        };
        let pair = k / 2;
        let candidate = k % 2 == 1;
        let pose = pair % 2;
        probe.update_camera(&r, cameras.frames[pose].camera().unwrap());
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let stamps = (frame >= WARMUP).then_some((2 * k as u32, 2 * k as u32 + 1));
        if candidate {
            probe.trace(
                &mut encoder,
                false,
                stamps.map(|(a, b)| wgpu::ComputePassTimestampWrites {
                    query_set: &queries,
                    beginning_of_pass_write_index: Some(a),
                    end_of_pass_write_index: Some(b),
                }),
            );
        } else {
            probe.raster(
                &r,
                &mut encoder,
                &draws,
                RasterMode::Production,
                stamps.map(|(a, b)| wgpu::RenderPassTimestampWrites {
                    query_set: &queries,
                    beginning_of_pass_write_index: Some(a),
                    end_of_pass_write_index: Some(b),
                }),
            );
        }
        r.queue.submit([encoder.finish()]);
        if frame >= WARMUP {
            labels.push((candidate, pose));
        }
    }
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.resolve_query_set(&queries, 0..(FRAMES * 2) as u32, &resolve, 0);
    encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, size);
    r.queue.submit([encoder.finish()]);
    let stamps: Vec<u64> = wait_map(&r, &read)
        .chunks_exact(8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let samples:Vec<_>=labels.iter().enumerate().map(|(k,&(candidate,pose))|serde_json::json!({"candidate":candidate,"pose":pose,"ms":(stamps[2*k+1]-stamps[2*k]) as f64*f64::from(r.queue.get_timestamp_period())*1e-6})).collect();
    fs::write(out.join("timing.json"),serde_json::to_vec_pretty(&serde_json::json!({"warmup_frames":WARMUP,"pairs":FRAMES/2,"validation":validate,"samples":samples,"stats":format!("{:?}",r.stats())})).unwrap()).unwrap();
    eprintln!("primary_probe_complete output={}", out.display());
}
