//! Single-receiver evidence and immutable shader snapshots for policy diagnosis.
use super::*;

fn probe_source() -> String {
    let source = primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone();
    // Naga 30 emits an invalid opaque null for loop-local ray queries. The
    // independent vertex-return witness must stay in the estimator's scope.
    let source = edit(
        &source,
        (
            "    var visible=0u;",
            "    var visible=0u;\n    var vertices:ray_query<vertex_return>;",
        ),
    );
    let source = edit(
        &source,
        (
            "    let seed=fresh_hash(pixel.x+pixel.y*width);",
            "    var seed=fresh_hash(pixel.x+pixel.y*width);\n    if probe_seed!=0u {seed=fresh_hash(seed^(probe_seed*0x9e3779b9u));}",
        ),
    );
    let source = edit(
        &source,
        (
            "        visible+=select(1u,0u,blocked);",
            include_str!("../support/receiver_probe_record.wgsl"),
        ),
    );
    format!(
        "{source}\n{}",
        include_str!("../support/receiver_probe.wgsl")
    )
}

#[test]
fn receiver_probe_validates_and_freezes_modules() {
    validate_wgsl(&probe_source());
    if let Ok(path) = std::env::var("RENDERLAB_FREEZE_MODULES") {
        let path = PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        let kernels = primary_ray_gpu::shader_sources(SOURCE).unwrap();
        let shading = primary_ray_gpu::shading_sources(SOURCE, STANDARD).unwrap();
        for (name, source) in [
            ("kernels-hard", &kernels[0]),
            ("kernels-soft", &kernels[1]),
            ("traced-hard", &shading[0]),
            ("traced-soft", &shading[1]),
        ] {
            fs::write(path.join(format!("{name}.wgsl")), source).unwrap();
        }
    }
}

#[test]
#[ignore = "owned GPU lease; exact blocking-ray witness and high-sample single receiver"]
fn gpu_receiver_blocking_ray() {
    let m = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Exact);
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[0].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    // Raw attachments retain epsilon (the generic evidence helper exports depth).
    let t = r.view.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    fs::write(out.join("world.f32"), read_texture(&r, &t.world.texture)).unwrap();
    fs::write(out.join("normal.f32"), read_texture(&r, &t.normal.texture)).unwrap();
    fs::write(
        out.join("visibility.u32"),
        read_texture(&r, &t.visibility.texture),
    )
    .unwrap();
    fs::write(out.join("hit.u32"), hit_record(&r)).unwrap();
    let seeds = m["probe_seeds"].as_u64().unwrap_or(4097) as u32;
    let buffer = |size| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("single receiver witness"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    };
    let masks = buffer(u64::from(seeds) * 32);
    let rays = buffer(256 * 12 * 16);
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("single receiver probe"),
            entries: &[
                storage(0),
                storage(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure {
                        vertex_return: true,
                    },
                    count: None,
                },
            ],
        });
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("single receiver probe"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: masks.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: rays.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: r.hardware_shadows.as_ref().unwrap().scene().as_binding(),
            },
        ],
    });
    let (fresh_layout, fresh_bind) = r.view.primary.as_ref().unwrap().test_fresh_bindings();
    let pipeline_layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("receiver probe ABI"),
            bind_group_layouts: &[
                Some(fresh_layout),
                None,
                Some(&layout),
                Some(&r.lighting.bindings),
            ],
            immediate_size: 0,
        });
    let shader = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production ray witness"),
        source: wgpu::ShaderSource::Wgsl(probe_source().into()),
    });
    let pipeline = r
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("single receiver high-sample reference"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("receiver_probe"),
            compilation_options: Default::default(),
            cache: None,
        });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, fresh_bind, &[]);
        pass.set_bind_group(2, &bind, &[]);
        pass.set_bind_group(3, r.lighting.bind(), &[]);
        pass.dispatch_workgroups(seeds, 1, 1);
    }
    r.queue.submit([encoder.finish()]);
    let masks_bytes = read_buffer(&mut r, &masks, masks.size());
    // First prove the probe preserves the production 256-bit diagnostic mask.
    let expected = [0u32, 0, 0, 0, 1 << 26, 0, 0, 0];
    let actual: Vec<u32> = masks_bytes[..32]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(actual, expected, "production ray 154 must reproduce");
    fs::write(out.join("masks.u32"), masks_bytes).unwrap();
    fs::write(
        out.join("rays.f32"),
        read_buffer(&mut r, &rays, rays.size()),
    )
    .unwrap();
    r.test_drain("single receiver evidence");
}

#[test]
#[ignore = "owned GPU lease; 4096 independent replicas at all solid terminator receivers"]
fn gpu_receiver_solid_verification() {
    let m = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Exact);
    let color = target(&r, 1920, 1080);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let layout = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("solid receiver verification"),
            entries: &(0..5)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: if binding == 2 {
                        wgpu::BindingType::AccelerationStructure {
                            vertex_return: true,
                        }
                    } else {
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: binding == 3,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        }
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
    let shader = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("solid verification production rays"),
        source: wgpu::ShaderSource::Wgsl(probe_source().into()),
    });
    // Seed zero is the candidate. Verification uses the same independent salts
    // 1..4096 as the one-receiver reference and the original 1..64 reference.
    for (pose, frame) in [0, 1, 0].into_iter().enumerate() {
        traced_frame(
            &mut r,
            &color,
            &cameras.frames[frame].camera().unwrap(),
            &mut asset,
            true,
        );
        let selected = fs::read(out.join(format!("pose-{pose}-pixels.u32"))).unwrap();
        let count = selected.len() / 4;
        assert!(count > 0 && count <= 65535);
        let buffer = |label, size| {
            r.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let masks = buffer("unused witness masks", 32);
        let rays = buffer("unused witness rays", 192);
        let pixels = buffer("solid terminator pixels", selected.len() as u64);
        let stats = buffer("independent solid verification", count as u64 * 16);
        r.queue.write_buffer(&pixels, 0, &selected);
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("solid verification buffers"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: masks.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: rays.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: r.hardware_shadows.as_ref().unwrap().scene().as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: pixels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: stats.as_entire_binding(),
                },
            ],
        });
        let (fresh_layout, fresh_bind) = r.view.primary.as_ref().unwrap().test_fresh_bindings();
        let pipeline_layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("solid verification ABI"),
                bind_group_layouts: &[
                    Some(fresh_layout),
                    None,
                    Some(&layout),
                    Some(&r.lighting.bindings),
                ],
                immediate_size: 0,
            });
        let pipeline = r
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("solid terminator verification"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("receiver_verify"),
                compilation_options: Default::default(),
                cache: None,
            });
        let mut encoder = r.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, fresh_bind, &[]);
            pass.set_bind_group(2, &bind, &[]);
            pass.set_bind_group(3, r.lighting.bind(), &[]);
            pass.dispatch_workgroups(4096, count as u32, 1);
        }
        r.queue.submit([encoder.finish()]);
        fs::write(
            out.join(format!("pose-{pose}-stats.u32")),
            read_buffer(&mut r, &stats, stats.size()),
        )
        .unwrap();
    }
    r.test_drain("solid terminator verification");
}

#[test]
#[ignore = "owned GPU lease; paired immutable-receiver scheduling screen"]
fn gpu_receiver_fresh_screen() {
    let m = manifest();
    let _validation = validation::begin(false);
    let mut r = renderer(false);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Exact);
    let color = target(&r, 1920, 1080);
    traced_frame(
        &mut r,
        &color,
        &cameras.frames[0].camera().unwrap(),
        &mut asset,
        true,
    );
    let expected = receivers(&r)[2].clone();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    traced_frame(
        &mut r,
        &color,
        &cameras.frames[0].camera().unwrap(),
        &mut asset,
        true,
    );
    let expected_eight = receivers(&r)[2].clone();
    let mut variants = vec![];
    for spec in m["variants"].as_array().unwrap() {
        let source = fs::read_to_string(spec["source"].as_str().unwrap()).unwrap();
        let shader = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fresh scheduling candidate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let p = r.view.primary.as_ref().unwrap().fresh_pipeline(
            &r.device,
            &r.ledger,
            &shader,
            "fresh_compute",
            &[],
        );
        let tile = [0, 1].map(|i| spec["tile"][i].as_u64().unwrap() as u32);
        let mut encoder = r.device.create_command_encoder(&Default::default());
        r.encode_primary_fresh_tile(&mut encoder, &p, tile, None);
        r.queue.submit([encoder.finish()]);
        let reference = if spec["directions"].as_u64() == Some(8) {
            &expected_eight
        } else {
            &expected
        };
        let identical = *reference == receivers(&r)[2];
        assert!(identical, "fresh scheduling byte identity {}", spec["name"]);
        variants.push((spec["name"].as_str().unwrap().to_owned(), tile, p));
    }
    let n = variants.len();
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("paired fresh screen"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * n as u32,
    });
    let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fresh screen timestamps"),
        size: 16 * n as u64,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fresh screen timestamp readback"),
        size: 16 * n as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let warmup = m["warmup_pairs"].as_u64().unwrap_or(12);
    let pairs = m["pairs"].as_u64().unwrap_or(24);
    let mut samples = vec![vec![]; n];
    let mut resources = vec![];
    for rep in 0..warmup + pairs {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        for k in 0..n {
            let i = (rep as usize + k) % n;
            r.encode_primary_fresh_tile(
                &mut encoder,
                &variants[i].2,
                variants[i].1,
                Some(wgpu::ComputePassTimestampWrites {
                    query_set: &queries,
                    beginning_of_pass_write_index: Some(2 * i as u32),
                    end_of_pass_write_index: Some(2 * i as u32 + 1),
                }),
            );
        }
        encoder.resolve_query_set(&queries, 0..2 * n as u32, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16 * n as u64);
        r.queue.submit([encoder.finish()]);
        let stamps: Vec<_> = wait_map(&r, &read)
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        if rep >= warmup {
            resources.push(serde_json::json!({"host_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()}));
            for i in 0..n {
                samples[i].push(
                    (stamps[2 * i + 1] - stamps[2 * i]) as f64
                        * f64::from(r.queue.get_timestamp_period())
                        * 1e-6,
                );
            }
        }
    }
    let rows: Vec<_> = variants.iter().zip(samples).map(|((name,tile,_),ms)|
        serde_json::json!({"name":name,"tile":tile,"byte_identical":true,"samples_ms":ms,"sample_context":resources})).collect();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("fresh-screen.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
