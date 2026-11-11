//! Test-only F0/R1 film references: 256 primary samples, each with 256 shadow rays.
//! Camera.view_mode.y is test scratch (sample + 1). The trace copies it into
//! normal.w so the unchanged fresh ABI can salt each subpixel independently.
use super::*;
#[path = "../support/fresh_fixture.rs"]
mod fixture;

#[path = "receiver_policy_tests.rs"]
mod receiver_policy_tests;

#[path = "film_accumulation_tests.rs"]
pub(super) mod film_accumulation_tests;

/// Trace/visibility and program shading have separate production modules and ABIs.
struct FilmSources {
    kernels: String,
    shading: String,
}

/// Randomized point within each of the 16x16 footprint strata. Odd/even indices
/// interleave across the footprint, rather than splitting it in two.
const R1_JITTER: &str = "\nfn film_jitter(i:u32)->vec2<f32> {\n let h=fresh_hash(i^0x83d2e19bu);\n let q=vec2<f32>(f32(h>>8u),f32(fresh_hash(h)>>8u))*(1.0/16777216.0);\n return (vec2<f32>(f32(i%16u),f32(i/16u))+q)/16.0;\n}\n";

/// The camera ray through `film_jitter(sample)` (`jitter` defines it) instead of the
/// pixel centre; the sample index is camera.view_mode.y - 1.
fn jittered_with(source: &str, jitter: &str) -> String {
    let mut source = edit(
        source,
        (
            "(vec2<f32>(pixel)+vec2<f32>(0.5))",
            "(vec2<f32>(pixel)+film_jitter(camera.view_mode.y-1u))",
        ),
    );
    source.push_str(jitter);
    source
}

fn film_shading(program: &str) -> String {
    film_shading_with(program, R1_JITTER)
}

fn film_shading_with(program: &str, jitter: &str) -> String {
    let soft = if let Some(frozen) = frozen_reference_modules() {
        let source = fs::read_to_string(frozen.join("traced-soft.wgsl"))
            .unwrap()
            .replace("\r\n", "\n");
        edit(
            &source,
            (
                &STANDARD.replace("\r\n", "\n"),
                &program.replace("\r\n", "\n"),
            ),
        )
    } else {
        primary_ray_gpu::shading_sources(SOURCE, program).unwrap()[1].clone()
    };
    // The hit-plane gradients must use the same jittered ray as primary tracing.
    jittered_with(&soft, jitter)
}

fn frozen_reference_modules() -> Option<PathBuf> {
    std::env::var_os("RENDERLAB_PRIMARY_MANIFEST")?;
    manifest()["film_frozen_modules"]
        .as_str()
        .map(PathBuf::from)
}

fn film_sources() -> FilmSources {
    film_sources_with(R1_JITTER)
}

fn film_sources_with(jitter: &str) -> FilmSources {
    let frozen = frozen_reference_modules();
    let soft = if let Some(frozen) = &frozen {
        fs::read_to_string(frozen.join("kernels-soft.wgsl")).unwrap()
    } else {
        primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone()
    };
    let mut kernels = edit(
        &jittered_with(&soft, jitter),
        (
            if !soft.contains("textureStore(primary_normal_output,p,vec4<f32>(normal,select(1.0,-1.0,receiver.thin)));") {
                "textureStore(primary_normal_output,p,vec4<f32>(normal,1.0));"
            } else {
                "textureStore(primary_normal_output,p,vec4<f32>(normal,select(1.0,-1.0,receiver.thin)));"
            },
            if !soft.contains("textureStore(primary_normal_output,p,vec4<f32>(normal,select(1.0,-1.0,receiver.thin)));") {
                "textureStore(primary_normal_output,p,vec4<f32>(normal,f32(camera.view_mode.y)));"
            } else {
                "textureStore(primary_normal_output,p,vec4<f32>(normal,select(f32(camera.view_mode.y),-f32(camera.view_mode.y),receiver.thin)));"
            },
        ),
    );
    kernels = edit(&kernels, SEED_EDIT);
    kernels = kernels.replace(
        "FRESH_SEED",
        "(u32(abs(textureLoad(fresh_normal,vec2<i32>(pixel),0).w)))",
    );
    let mut sources = FilmSources {
        kernels,
        shading: film_shading_with(STANDARD, jitter),
    };
    // A bounded arithmetic reducer for receiver-equivalence investigations.
    // Registered production references omit this field entirely.
    if std::env::var_os("RENDERLAB_PRIMARY_MANIFEST").is_some()
        && let Some(edits) = manifest()["film_shader_edits"].as_array()
    {
        for pair in edits {
            let find = pair[0].as_str().unwrap();
            let replace = pair[1].as_str().unwrap();
            for source in [&mut sources.kernels, &mut sources.shading] {
                *source = edit(source, (find, replace));
            }
        }
    }
    sources
}

#[test]
fn film_reference_sources_compile() {
    {
        let sources = film_sources();
        for source in [sources.kernels, sources.shading] {
            validate_wgsl(&source);
        }
    }
    validate_wgsl(&receiver_policy_tests::accumulator_source(false));
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
    // R1 comparisons keep the same registered surface-acceptance anchor: moving
    // the centre from the old rounded ray point onto its triangle can otherwise
    // change a few 2%-depth boundary memberships, independently of shadow rays.
    // R_L/R_D never use this acceptance mask. Production receivers above are
    // still captured unchanged, and all shadow origins use production code.
    let anchor = std::env::var_os("RENDERLAB_PRIMARY_MANIFEST")
        .and_then(|_| manifest()["r1_reference"].as_str().map(PathBuf::from))
        .filter(|path| path.join(format!("pose-{pose}.world.f32")).is_file());
    let anchor_world = anchor.as_ref().map_or_else(
        || world.clone(),
        |p| fs::read(p.join(format!("pose-{pose}.world.f32"))).unwrap(),
    );
    let centre: Vec<u8> = anchor_world
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
    } else {
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: wgpu::BindingResource::TextureView(&tile_view),
        });
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
    let sources = policy.map_or_else(film_sources, receiver_policy_tests::sources);
    let pipelines = r.primary.as_ref().unwrap().test_film_pipelines(
        &r.device,
        &sources.kernels,
        &sources.shading,
    );
    let direct_pipelines = policy.map(|_| {
        r.primary.as_ref().unwrap().test_film_pipelines(
            &r.device,
            &sources.kernels,
            &receiver_policy_tests::direct_source(),
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
    fs::write(prefix.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"pose":pose,"size":[w,h],"samples":256,"rays_per_sample":256,"seconds":start.elapsed().as_secs_f64(),"camera_inverse":camera.matrix(w,h).inverse().to_cols_array(),"eye":eye.to_array(),"forward":forward.to_array(),"receiver_policy":if frozen_reference_modules().is_some() {"frozen-767c28c"} else {"Production"},"normal_bias":if frozen_reference_modules().is_some() {"0.001 m"} else {"max(10um,16ulp(max abs world triangle vertex))"},"tmin":if frozen_reference_modules().is_some() {"frozen projected-depth policy"} else {"epsilon (ray distance)"}})).unwrap()).unwrap();
    if let Some(policy) = policy {
        let path = prefix.with_extension("json");
        let mut info: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if frozen_reference_modules().is_none() {
            info["receiver_policy"] = serde_json::json!(format!("{policy:?}"));
        }
        info["acceptance_anchor"] = serde_json::json!(anchor.map_or_else(
            || "production centre".to_owned(),
            |p| p.display().to_string()
        ));
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
    // The immutable old modules supply a same-process old-policy control, including
    // direct film radiance which F0 did not save. No production pipeline uses it.
    if let Some(frozen) = frozen_reference_modules() {
        let read = |names: [&str; 2]| {
            names.map(|n| fs::read_to_string(frozen.join(format!("{n}.wgsl"))).unwrap())
        };
        r.pipeline.primary = Some(r.primary.as_ref().unwrap().compile(
            &r.device,
            &r.ledger,
            &read(["kernels-hard", "kernels-soft"]),
        ));
        r.test_standard().traced =
            shading_pipelines(&r, &read(["traced-hard", "traced-soft"])).map(Some);
    }
    let out = PathBuf::from(m["output"].as_str().unwrap());
    let minimal = m["minimal"].as_bool().unwrap();
    let receiver_policy = match m["receiver_policy"].as_str() {
        Some("production") => Some(receiver_policy_tests::Policy::Production),
        None => None,
        _ => panic!("unknown receiver policy"),
    };
    if matches!(
        m["workload"].as_str(),
        Some("flat" | "convex" | "sawtooth" | "contact")
    ) {
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
        rectangle_fixture(&mut r, &m, receiver_policy);
    }
}

/// The F0/R1 rectangle fixture (thin, or stress when `stress`): a ground quad and blocker
/// rectangles with their lighting set, and the frame they are integrated at.
fn rect_scene(
    r: &mut Renderer,
    stress: bool,
) -> (Mesh, Instances, Instances, Camera, Vec<fixture::Rectangle>) {
    let rects = fixture::rectangles(stress);
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
    let ground = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("film ground");
    let blockers = r
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
    (mesh, ground, blockers, camera, rects)
}

fn rectangle_fixture(
    r: &mut Renderer,
    m: &serde_json::Value,
    receiver_policy: Option<receiver_policy_tests::Policy>,
) {
    let out = PathBuf::from(m["output"].as_str().unwrap());
    let minimal = m["minimal"].as_bool().unwrap_or(false);
    let (mesh, mut ground, mut blockers, camera, rects) = rect_scene(r, m["workload"] == "stress");
    let color = target(r, 256, 192);
    render(
        r,
        &color,
        &camera,
        &mut [
            Draw::new(&mesh, &mut ground),
            Draw::new(&mesh, &mut blockers),
        ],
    );
    integrate_with(r, &camera, &out, 0, minimal, receiver_policy);
    fs::write(
        out.join("rectangles.json"),
        serde_json::to_vec_pretty(&rects).unwrap(),
    )
    .unwrap();
}
