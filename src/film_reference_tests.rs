//! Test-only F0/R1 film references: 256 primary samples, each with 256 shadow rays.
//! Camera.view_mode.y is test scratch (sample + 1). The trace copies it into
//! normal.w so the unchanged fresh ABI can salt each subpixel independently.
use super::*;
#[path = "../support/fresh_fixture.rs"]
mod fixture;

#[path = "receiver_policy_tests.rs"]
mod receiver_policy_tests;

fn film_sources(minimal: bool) -> [String; 2] {
    let mut sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    for source in &mut sources {
        // The current production entry is tiled; F0's cfg(test) accessor retains
        // its original name. Adapt only the composed test module.
        *source = edit(source, ("fn primary_shade_tiles(", "fn primary_shade("));
        *source = edit(
            source,
            (
                "(vec2<f32>(pixel)+vec2<f32>(0.5))",
                "(vec2<f32>(pixel)+film_jitter(camera.view_mode.y-1u))",
            ),
        );
        *source = edit(
            source,
            (
                "textureStore(primary_normal_output,p,vec4<f32>(normal,1.0));",
                "textureStore(primary_normal_output,p,vec4<f32>(normal,f32(camera.view_mode.y)));",
            ),
        );
        // Randomized point within each of the 16x16 footprint strata. Odd/even
        // indices interleave across the footprint, rather than splitting it in two.
        source.push_str("\nfn film_jitter(i:u32)->vec2<f32> {\n let h=fresh_hash(i^0x83d2e19bu);\n let q=vec2<f32>(f32(h>>8u),f32(fresh_hash(h)>>8u))*(1.0/16777216.0);\n return (vec2<f32>(f32(i%16u),f32(i/16u))+q)/16.0;\n}\n");
        // The hard module does not contain the fresh hash.
        if !source.contains("fn fresh_hash(") {
            source.push_str("fn fresh_hash(input:u32)->u32 {var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);}\n");
        }
    }
    sources[1] = edit(&sources[1], SEED_EDIT);
    sources[1] = sources[1].replace(
        "FRESH_SEED",
        "(u32(textureLoad(fresh_normal,vec2<i32>(pixel),0).w))",
    );
    if minimal {
        sources[1] = edit(
            &sources[1],
            (
                "let origin=world+normal*view.coverage_bias.w;",
                "let origin=world+normal*1e-5;",
            ),
        );
        sources[1] = edit(
            &sources[1],
            ("let tmin=view.coverage_bias.z/rate;", "let tmin=1e-5;"),
        );
    }
    sources
}

#[test]
fn film_reference_sources_compile() {
    for minimal in [false, true] {
        for source in film_sources(minimal) {
            validate_wgsl(&source);
        }
    }
    validate_wgsl(include_str!("../support/film_accumulate.wgsl"));
}

fn read_buffer(r: &mut Renderer, buffer: &wgpu::Buffer, size: u64) -> Vec<u8> {
    let staging = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("film readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut e = r.device.create_command_encoder(&Default::default());
    e.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    e.map_buffer_on_submit(&staging, wgpu::MapMode::Read, .., move |v| {
        callback.set(v).unwrap();
    });
    r.queue.submit([e.finish()]);
    r.test_drain("film reference readback");
    done.get().unwrap().as_ref().unwrap();
    staging.get_mapped_range(..).unwrap().to_vec()
}

/// A normal centre frame must precede this call. Further samples reuse its owned
/// scene/TLAS/material bindings; no resource tables or production files are edited.
fn integrate_with(
    r: &mut Renderer,
    camera: &Camera,
    out: &std::path::Path,
    pose: usize,
    minimal: bool,
    policy: Option<receiver_policy_tests::Policy>,
) {
    fs::create_dir_all(out).unwrap();
    r.test_drain("film centre");
    let start = Instant::now();
    let [world, normal, counts] = receivers(r);
    let hit = hit_record(r);
    let prefix = out.join(format!("pose-{pose}"));
    for (suffix, data) in [
        ("world.f32", &world),
        ("normal.f32", &normal),
        ("exact.u32", &counts),
        ("hit.u32", &hit),
    ] {
        fs::write(prefix.with_extension(suffix), data).unwrap();
    }
    fs::write(prefix.with_extension("exact.rgba16f"), capture(r)).unwrap();
    let centre: Vec<u8> = world
        .chunks_exact(16)
        .zip(hit.chunks_exact(16))
        .flat_map(|(w, h)| w[..12].iter().chain(h[..4].iter()).copied())
        .collect();
    let make = |label, size, usage| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let centre_buffer = make(
        "film centre surface",
        centre.len() as u64,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    r.queue.write_buffer(&centre_buffer, 0, &centre);
    let bytes = (world.len() / 16 * 48) as u64;
    let sum = make(
        "film sums",
        bytes,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let params = make(
        "film parameters",
        48,
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    );
    let accumulator_source = receiver_policy_tests::accumulator_source(policy.is_some());
    let sample_buffer = make(
        "R1 saved visibility",
        (world.len() / 4) as u64,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let direct_sum = make(
        "R1 direct sums",
        bytes,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("F0 accumulator"),
        source: wgpu::ShaderSource::Wgsl(accumulator_source.into()),
    });
    let accumulator = r
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("F0 accumulator"),
            layout: None,
            module: &module,
            entry_point: Some("accumulate"),
            compilation_options: Default::default(),
            cache: None,
        });
    let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let textures = [
        &r.primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .hit
            .texture,
        &t.world.texture,
        &t.visibility.texture,
        &r.lighting.hdr.as_ref().unwrap().resolved.texture,
    ];
    let views = textures.map(|t| t.create_view(&Default::default()));
    let mut entries: Vec<_> = views
        .iter()
        .enumerate()
        .map(|(i, v)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: wgpu::BindingResource::TextureView(v),
        })
        .collect();
    entries.extend([
        wgpu::BindGroupEntry {
            binding: 4,
            resource: centre_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 5,
            resource: sum.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 6,
            resource: params.as_entire_binding(),
        },
    ]);
    let tile_view = r
        .primary
        .as_ref()
        .unwrap()
        .targets
        .as_ref()
        .unwrap()
        .tiles
        .texture
        .create_view(&Default::default());
    if policy.is_some() {
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 7,
                resource: sample_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&tile_view),
            },
        ]);
    }
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("film accumulator"),
        layout: &accumulator.get_bind_group_layout(0),
        entries: &entries,
    });
    entries
        .iter_mut()
        .find(|e| e.binding == 5)
        .unwrap()
        .resource = direct_sum.as_entire_binding();
    let direct_bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("R1 sun accumulator"),
        layout: &accumulator.get_bind_group_layout(0),
        entries: &entries,
    });
    let sources = policy.map_or_else(|| film_sources(minimal), receiver_policy_tests::sources);
    let pipelines = r
        .primary
        .as_ref()
        .unwrap()
        .test_film_pipelines(&r.device, &sources[1]);
    let direct_pipelines = policy.map(|_| {
        r.primary.as_ref().unwrap().test_film_pipelines(
            &r.device,
            &receiver_policy_tests::direct_source(&sources[1]),
        )
    });
    let [w, h] = [textures[0].width(), textures[0].height()];
    if let Some(direct) = &direct_pipelines {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        r.encode_primary_shade(
            &mut encoder,
            &direct[2],
            primary_ray_gpu::SHADE_TILE,
            primary_ray_gpu::Shading::Soft,
            None,
        );
        r.queue.submit([encoder.finish()]);
        fs::write(prefix.with_extension("exact.sun.rgba16f"), capture(r)).unwrap();
    }
    let inverse = camera.view().inverse();
    let eye = inverse.w_axis;
    let forward = -inverse.z_axis;
    let mut values = Vec::new();
    for v in eye.to_array().into_iter().chain(forward.to_array()) {
        values.extend(v.to_le_bytes());
    }
    values.resize(48, 0);
    if policy.is_some() {
        fs::create_dir_all(out.join("samples")).unwrap();
    }
    for ordinal in 0..256u32 {
        let sample = if policy.is_some() {
            (ordinal % 16) * 16 + (ordinal % 16 * 7 + ordinal / 16) % 16
        } else {
            ordinal
        };
        r.queue
            .write_buffer(&r.uniform, 68, &(sample + 1).to_le_bytes());
        values[32..36].copy_from_slice(&sample.to_le_bytes());
        r.queue.write_buffer(&params, 0, &values);
        let mut e = r.device.create_command_encoder(&Default::default());
        r.encode_primary_trace(
            &mut e,
            &pipelines[0],
            primary_ray_gpu::TRACE_TILE,
            true,
            None,
        );
        r.encode_primary_fresh(&mut e, &pipelines[1], None);
        r.encode_primary_shade(
            &mut e,
            &pipelines[2],
            primary_ray_gpu::SHADE_TILE,
            primary_ray_gpu::Shading::Soft,
            None,
        );
        {
            let mut pass = e.begin_compute_pass(&Default::default());
            pass.set_pipeline(&accumulator);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(4), 1);
        }
        r.queue.submit([e.finish()]);
        if policy.is_some() && ordinal < 16 {
            fs::write(
                out.join(format!("samples/{ordinal:02}.rgba16f")),
                capture(r),
            )
            .unwrap();
            let raw = read_buffer(r, &sample_buffer, (world.len() / 4) as u64);
            let packed: Vec<u8> = raw
                .chunks_exact(4)
                .flat_map(|v| (u32::from_le_bytes(v.try_into().unwrap()) as u16).to_le_bytes())
                .collect();
            fs::write(
                out.join(format!("samples/{ordinal:02}.visibility.u16")),
                packed,
            )
            .unwrap();
        }
        if let Some(direct) = &direct_pipelines {
            let mut e = r.device.create_command_encoder(&Default::default());
            r.encode_primary_shade(
                &mut e,
                &direct[2],
                primary_ray_gpu::SHADE_TILE,
                primary_ray_gpu::Shading::Soft,
                None,
            );
            {
                let mut pass = e.begin_compute_pass(&Default::default());
                pass.set_pipeline(&accumulator);
                pass.set_bind_group(0, &direct_bind, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(4), 1);
            }
            r.queue.submit([e.finish()]);
            if ordinal < 16 {
                fs::write(
                    out.join(format!("samples/{ordinal:02}.sun.rgba16f")),
                    capture(r),
                )
                .unwrap();
            }
        }
        // Bound queue residence and the OS watchdog. This is reference throughput,
        // never a production performance measurement.
        if ordinal % 16 == 15 {
            r.test_drain("film batch");
            eprintln!(
                "film pose={pose} minimal={minimal} policy={policy:?} samples={}",
                ordinal + 1
            );
        }
    }
    fs::write(
        prefix.with_extension("film.bin"),
        read_buffer(r, &sum, bytes),
    )
    .unwrap();
    if policy.is_some() {
        fs::write(
            prefix.with_extension("direct.bin"),
            read_buffer(r, &direct_sum, bytes),
        )
        .unwrap();
    }
    r.queue.write_buffer(&r.uniform, 68, &0u32.to_le_bytes());
    fs::write(prefix.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"pose":pose,"minimal":minimal,"size":[w,h],"samples":256,"rays_per_sample":256,"seconds":start.elapsed().as_secs_f64(),"camera_inverse":camera.matrix(w,h).inverse().to_cols_array(),"eye":eye.to_array(),"forward":forward.to_array(),"normal_bias":if minimal {1e-5} else {0.001},"tmin":if minimal {"1e-5 m"} else {"unchanged projected-depth policy"}})).unwrap()).unwrap();
    if let Some(policy) = policy {
        let path = prefix.with_extension("json");
        let mut info: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        info["receiver_policy"] = serde_json::json!(format!("{policy:?}"));
        info["normal_bias"] = if matches!(policy, receiver_policy_tests::Policy::Minimal) {
            serde_json::json!("1e-5 m (F0 replay)")
        } else {
            serde_json::json!("max(10um,16ulp(max abs world triangle vertex))")
        };
        info["tmin"] = serde_json::json!("epsilon (ray distance)");
        info["saved_strata"] = serde_json::json!(
            (0..16u32)
                .map(|i| i * 16 + (i * 7) % 16)
                .collect::<Vec<_>>()
        );
        fs::write(path, serde_json::to_vec_pretty(&info).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "F0: owned GPU lease, preflight, test-only offscreen reference"]
fn gpu_film_reference() {
    let m = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    let minimal = m["minimal"].as_bool().unwrap();
    let receiver_policy = match m["receiver_policy"].as_str() {
        Some("minimal") => Some(receiver_policy_tests::Policy::Minimal),
        Some("magnitude") => Some(receiver_policy_tests::Policy::Magnitude),
        Some("viewed") => Some(receiver_policy_tests::Policy::Viewed),
        None => None,
        _ => panic!("unknown receiver policy"),
    };
    if matches!(m["workload"].as_str(), Some("flat" | "convex" | "sawtooth")) {
        receiver_policy_tests::analytic_fixture(&mut r, &m, receiver_policy.unwrap());
        return;
    }
    if m["workload"] == "quarry" {
        let (mut asset, cameras) = load_quarry(&m, &mut r);
        let color = target(&r, 1920, 1080);
        let pose = m["pose"].as_u64().unwrap() as usize;
        let camera = cameras.frames[usize::from(pose == 1)].camera().unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        integrate_with(&mut r, &camera, &out, pose, minimal, receiver_policy);
    } else {
        let rects = fixture::rectangles(m["workload"] == "stress");
        let vertices =
            [[-8., -8., 0.], [8., -8., 0.], [8., 8., 0.], [-8., 8., 0.]].map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: [1.; 4],
            });
        let mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
            })
            .unwrap()
            .unwrap();
        r.test_drain("film fixture mesh");
        let mut ground = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("film ground");
        let mut blockers = r
            .try_upload_instances(&rects.iter().map(|v| v.transform()).collect::<Vec<_>>())
            .unwrap()
            .unwrap();
        r.test_drain("film blockers");
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(-1., 0., -1.),
                    lux: [1000.; 3],
                }],
                exposure_stops: -8.,
                ..Default::default()
            },
            &Shadows {
                directional_disk_angle: 0.04,
                requests: vec![ShadowRequest::Directional {
                    split_ends: vec![10., 40.],
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let camera = Camera::look_at(
            Vec3::new(0., 0., 4.),
            Vec3::ZERO,
            Vec3::Y,
            55f32.to_radians(),
            0.1,
            100.,
        )
        .unwrap();
        let color = target(&r, 256, 192);
        render(
            &mut r,
            &color,
            &camera,
            &mut [
                Draw::new(&mesh, &mut ground),
                Draw::new(&mesh, &mut blockers),
            ],
        );
        integrate_with(&mut r, &camera, &out, 0, minimal, receiver_policy);
        fs::write(
            out.join("rectangles.json"),
            serde_json::to_vec_pretty(&rects).unwrap(),
        )
        .unwrap();
    }
}
