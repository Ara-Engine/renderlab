//! N1b/N1c batched agreement/quality/timing, including a real device with RT disabled.
use super::*;
use rayon::prelude::*;

fn no_rt_renderer() -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        // WGPU_DEBUG=1 adds shader source line info for profilers (N8f's Nsight traces).
        flags: if manifest()["validate"].as_bool() == Some(false) {
            wgpu::InstanceFlags::empty()
        } else {
            wgpu::InstanceFlags::debugging()
        }
        .with_env(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("N1b adapter {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: crate::visibility_limits(wgpu::Limits::default()),
        memory_hints: super::test_memory_hints(),
        ..Default::default()
    }))
    .unwrap();
    assert!(!device.features().intersects(
        wgpu::Features::EXPERIMENTAL_RAY_QUERY | wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN
    ));
    let r = Renderer::new_with_options(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Limits {
            max_mesh_bytes: 134217728,
            max_texture_bytes: 134217728,
            ..Default::default()
        },
        RendererOptions {
            samples: SampleCount::One,
            anisotropy_clamp: 16,
        },
        SOURCE,
    )
    .unwrap();
    assert_eq!(r.primary_visibility(), PrimaryVisibility::VisibilityBuffer);
    assert_eq!(r.soft_shadows(), SoftShadows::Virtual);
    assert!(r.hardware_shadows.is_none());
    r
}
fn geometry(r: &mut Renderer, positions: &[[f32; 3]], indices: &[u32]) -> (Mesh, Instances) {
    let vertices: Vec<_> = positions
        .iter()
        .map(|&position| Vertex {
            position,
            normal: [0., 1., 0.],
            uv: [0.; 2],
            color: [1.; 4],
        })
        .collect();
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices,
        })
        .unwrap()
        .unwrap();
    r.test_drain("N1b upload");
    let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("N1b placements");
    (mesh, instances)
}
fn light(r: &mut Renderer) {
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: -Vec3::new(0.4, 0.8, 0.3).normalize(),
                lux: [1000.; 3],
            }],
            environment: [0.03; 3],
            exposure_stops: -8.,
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
}
fn timing(r: &mut Renderer, label: &str, rows: &mut Vec<serde_json::Value>) {
    for t in r.poll().unwrap().gpu {
        let ms = |v: Duration| v.as_secs_f64() * 1000.;
        rows.push(serde_json::json!({"label":label,"span_ms":ms(t.render_span),"primary_ms":t.receiver_pass.map(ms),"shadow_ms":t.visibility_compute.map(ms)}));
    }
}
/// N8h: the renderer's GPU memory by owner (ledger), wgpu's HAL counters and the
/// allocator's report grouped by resource label, to `memory-<label>.json`.
fn memory_report(r: &Renderer, out: &std::path::Path, label: &str) {
    let stats = r.stats();
    let o = &stats.ownership;
    let mut groups: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
    let report = r.device.generate_allocator_report();
    for a in report.iter().flat_map(|r| &r.allocations) {
        let e = groups.entry(a.name.clone()).or_default();
        e.0 += a.size;
        e.1 += 1;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));
    let value = serde_json::json!({
        "label": label,
        "allocator_allocated_bytes": report.as_ref().map(|r| r.total_allocated_bytes),
        "allocator_reserved_bytes": report.as_ref().map(|r| r.total_reserved_bytes),
        "allocator_blocks": report.as_ref().map(|r| r.blocks.len()),
        "allocations": report.as_ref().map(|r| r.allocations.iter().map(|a| {
            serde_json::json!({"name": a.name, "bytes": a.size, "offset": a.offset})
        }).collect::<Vec<_>>()),
        "blocks": report.as_ref().map(|r| r.blocks.iter().map(|b| {
            serde_json::json!({"bytes": b.size, "allocations": [b.allocations.start, b.allocations.end]})
        }).collect::<Vec<_>>()),
        "hal_buffer_bytes": stats.hal.buffer_bytes,
        "hal_texture_bytes": stats.hal.texture_bytes,
        "ledger_live_bytes": {
            "environment": o.environment_bytes.live, "texture": o.texture_bytes.live,
            "material": o.material_bytes.live, "target": o.target_bytes.live,
            "light": o.light_bytes.live, "shadow": o.shadow_bytes.live, "lut": o.lut_bytes.live,
            "hardware_as_estimated": o.hardware_as_estimated_bytes.live,
            "hardware_as_observed": o.hardware_as_observed_bytes.live,
            "hardware_scratch_estimated": o.hardware_scratch_estimated_bytes.live,
        },
        "by_label": groups.iter().map(|(name, (bytes, count))| {
            serde_json::json!({"name": name, "bytes": bytes, "count": count})
        }).collect::<Vec<_>>(),
    });
    fs::write(
        out.join(format!("memory-{label}.json")),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}
/// N8h: the triangles with walk records under this configuration.
fn record_counters(r: &mut Renderer, label: &str) -> serde_json::Value {
    serde_json::json!({"label": label, "recorded": r.test_walk_records()})
}
fn snapshot(r: &mut Renderer, out: &std::path::Path, label: &str) {
    r.test_drain("N1b snapshot");
    let [hits, counts] = interactive_outputs(r);
    if manifest()["hash_only"].as_bool() == Some(true) {
        use sha2::Digest;
        let sha256 = |bytes: &[u8]| format!("{:x}", sha2::Sha256::digest(bytes));
        fs::write(
            out.join(format!("{label}.hashes.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "counts_sha256": sha256(&counts), "hits_sha256": sha256(&hits),
            }))
            .unwrap(),
        )
        .unwrap();
        return;
    }
    fs::write(out.join(format!("{label}.hit")), hits).unwrap();
    fs::write(out.join(format!("{label}.counts")), counts).unwrap();
    fs::write(out.join(format!("{label}.hdr")), capture(r)).unwrap();
    let world = &r.fresh.as_ref().unwrap().targets.as_ref().unwrap().world;
    fs::write(
        out.join(format!("{label}.world")),
        read_texture(r, &world.texture),
    )
    .unwrap();
    let matrix: Vec<_> = r
        .camera_shadow
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    fs::write(
        out.join(format!("{label}.camera.json")),
        serde_json::to_vec(&matrix).unwrap(),
    )
    .unwrap();
    if let Some(v) = &r.visibility_buffer {
        let passes: Vec<_> = v
            .test_counts(r)
            .iter()
            .map(|c| {
                serde_json::json!({
                    "software_clusters": c.software_clusters, "hardware_clusters": c.hardware_clusters,
                    "cut_clusters": c.cut_clusters, "cut_triangles": c.cut_triangles,
                    "software_triangles": c.software_triangles, "hardware_triangles": c.hardware_triangles})
            })
            .collect();
        let row = serde_json::json!({"cluster_instances": v.cluster_instances(), "passes": passes});
        fs::write(
            out.join(format!("{label}.cull.json")),
            serde_json::to_vec_pretty(&row).unwrap(),
        )
        .unwrap();
    }
}

/// Median per-stage GPU milliseconds of the prepared visibility buffer, encoded alone:
/// per pass cull, software raster, hardware raster and HZB, then resolve. All repeats
/// go in one submission, so the GPU does not idle (and downclock) between them.
fn stage_profile(r: &mut Renderer, repeats: usize) -> Vec<f64> {
    let n = visibility_buffer::STAGE_QUERIES;
    // Resolve offsets need QUERY_RESOLVE_BUFFER_ALIGNMENT.
    let stride = (u64::from(n) * 8).next_multiple_of(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
    let bytes = stride * repeats as u64;
    let resolved = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("N1c stage timestamps"),
        size: bytes,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("N1c stage readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let period = f64::from(r.queue.get_timestamp_period());
    let mut e = r.device.create_command_encoder(&Default::default());
    for k in 0..repeats as u64 {
        let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("N1c stages"),
            ty: wgpu::QueryType::Timestamp,
            count: n,
        });
        r.encode_visibility_stages(&mut e, true, None, Some(&queries));
        e.resolve_query_set(&queries, 0..n, &resolved, k * stride);
    }
    e.copy_buffer_to_buffer(&resolved, 0, &read, 0, bytes);
    e.map_buffer_on_submit(&read, wgpu::MapMode::Read, .., |result| result.unwrap());
    r.queue.submit([e.finish()]);
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    let mapped = read.get_mapped_range(..).unwrap();
    let t: Vec<u64> = mapped
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let mut samples = vec![vec![]; n as usize / 2];
    for repeat in t.chunks_exact(stride as usize / 8) {
        for (i, s) in samples.iter_mut().enumerate() {
            s.push(repeat[2 * i + 1].saturating_sub(repeat[2 * i]) as f64 * period * 1e-6);
        }
    }
    samples
        .into_iter()
        .map(|mut s| {
            s.sort_by(f64::total_cmp);
            s[s.len() / 2]
        })
        .collect()
}

/// `load_quarry` with each primitive's cooked clusterLOD hierarchy attached when the
/// manifest names `virtual_manifest` and `virtual_cache`. Returns load/attach statistics.
fn load_quarry_lod(
    m: &serde_json::Value,
    r: &mut Renderer,
) -> (prepared::Prepared, cameras::Cameras, serde_json::Value) {
    let path = |key: &str| PathBuf::from(m[key].as_str().unwrap());
    let (policy, _) = policy::Policy::args(vec![
        "--limits".into(),
        path("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let lighting =
        lighting_settings::parse(&fs::read_to_string(path("lighting")).unwrap()).unwrap();
    let shadows = Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        }],
        ..Default::default()
    };
    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    let env = environment_controls::load_environment(
        &path("environment"),
        asset::environment::Limits {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 128 * 1024 * 1024,
        },
    )
    .unwrap();
    let env = r.try_upload_environment(&env).unwrap().unwrap();
    r.set_environment(Some(&env), EnvironmentParameters::default())
        .unwrap();
    r.test_drain("N1c quarry environment");
    let started = Instant::now();
    let bytes = fs::read(path("asset")).unwrap();
    let source = asset::decode(&bytes, policy.prepared()).unwrap();
    drop(bytes);
    let decode_ms = started.elapsed().as_secs_f64() * 1e3;
    let started = Instant::now();
    let lods: Vec<MeshLod> = match m["virtual_manifest"].as_str() {
        Some(manifest) => {
            let package =
                LodPackage::open(&path("virtual_cache"), std::path::Path::new(manifest)).unwrap();
            let primitives: Vec<_> = source
                .meshes
                .iter()
                .enumerate()
                .flat_map(|(i, mesh)| mesh.primitives.iter().map(move |p| (i, p)))
                .collect();
            // Four meshes at a time bound the page and table working set.
            primitives
                .chunks(4)
                .flat_map(|chunk| {
                    chunk
                        .par_iter()
                        .map(|&(i, p)| {
                            let data = MeshData {
                                vertices: &p.vertices,
                                indices: &p.indices,
                            };
                            package.mesh_lod(i, p.material, &data).unwrap()
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        }
        None => vec![],
    };
    let lod_build_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut asset =
        prepared::Prepared::new_with_limits(source, policy.prepared(), 1_000_000).unwrap();
    let started = Instant::now();
    while !asset.ready() {
        asset.advance(r).unwrap();
        r.collect().unwrap();
        assert!(started.elapsed() < Duration::from_secs(120));
    }
    r.test_drain("N1c quarry upload");
    let upload_ms = started.elapsed().as_secs_f64() * 1e3;
    let started = Instant::now();
    for (mesh, lod) in asset.meshes_mut().iter_mut().zip(&lods) {
        while r.try_attach_lod(mesh, lod).unwrap().is_none() {
            r.collect().unwrap();
        }
    }
    r.test_drain("N1c LOD attach");
    let attach_ms = started.elapsed().as_secs_f64() * 1e3;
    let meshes: Vec<_> = lods
        .iter()
        .map(|l| {
            let s = l.stats;
            serde_json::json!({"groups": s.groups, "clusters": s.clusters, "source_clusters": s.source_clusters,
                "lod_triangles": s.lod_triangles, "unmatched_triangles": s.unmatched_triangles,
                "pages": s.pages, "read_ms": s.read_ms, "build_ms": s.build_ms})
        })
        .collect();
    let stats = serde_json::json!({"decode_ms": decode_ms, "lod_build_ms": lod_build_ms,
        "upload_ms": upload_ms, "attach_ms": attach_ms, "meshes": meshes});
    eprintln!("N1c load {stats}");
    let cameras = cameras::Cameras::load(&path("cameras")).unwrap();
    assert!(cameras.frames.len() >= 2);
    (asset, cameras, stats)
}

fn reference(
    r: &mut Renderer,
    pipelines: &[Owned<wgpu::ComputePipeline>],
    out: &std::path::Path,
    label: &str,
) {
    r.test_drain("N1b film reference start");
    let hit = hit_record(r);
    let n = hit.len() / 16;
    let mut sum = vec![0f64; n];
    let mut square = vec![0f64; n];
    for pipeline in pipelines {
        let mut e = r.device.create_command_encoder(&Default::default());
        r.encode_primary_fresh(&mut e, pipeline, None);
        r.queue.submit([e.finish()]);
        let t = &r
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture;
        for (i, c) in read_texture(r, t).chunks_exact(4).enumerate() {
            let value = f64::from(u32::from_le_bytes(c.try_into().unwrap())) / 256.;
            sum[i] += value;
            square[i] += value * value;
        }
    }
    let count = pipelines.len() as f64;
    let mut bytes = Vec::with_capacity(n * 8);
    for (s, q) in sum.into_iter().zip(square) {
        let mean = s / count;
        let sigma = (q / count - mean * mean).max(0.).sqrt();
        bytes.extend_from_slice(&(mean as f32).to_le_bytes());
        bytes.extend_from_slice(&(sigma as f32).to_le_bytes());
    }
    fs::write(out.join(format!("{label}.reference")), bytes).unwrap();
}

#[derive(serde::Deserialize)]
struct Bench {
    size: [u32; 2],
    meshes: Vec<BenchMesh>,
    #[serde(default)]
    poses: Vec<BenchPose>,
    #[serde(default)]
    tracks: Vec<BenchTrack>,
    #[serde(default)]
    to_light: Option<[f32; 3]>,
    #[serde(default)]
    fov_y_degrees: Option<f32>,
    sun_lux: [f32; 3],
    exposure_stops: f32,
    #[serde(default)]
    material: Option<BenchMaterial>,
}
#[derive(serde::Deserialize)]
struct BenchMaterial {
    base_color: [f32; 4],
    metallic: f32,
}
#[derive(serde::Deserialize)]
struct BenchMesh {
    #[serde(default)]
    name: String,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    thin: bool,
    #[serde(default)]
    roughness: Option<f32>,
}
#[derive(serde::Deserialize)]
struct BenchTrack {
    frames: Vec<BenchPose>,
}
#[derive(Clone, serde::Deserialize)]
struct BenchPose {
    eye: [f32; 3],
    target: [f32; 3],
    #[serde(default)]
    to_light: Option<[f32; 3]>,
    #[serde(default)]
    plate_x: f32,
    #[serde(default)]
    blocker_x: f32,
}

#[test]
#[ignore = "N1b owned GPU lease: S1/S2/S3 full-detail primaries and complete no-RT frame"]
fn gpu_visibility_bench() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let no_rt = m["no_rt"].as_bool().unwrap_or(false);
    let mut r = if no_rt {
        no_rt_renderer()
    } else {
        renderer_with_features(
            m["validate"].as_bool() != Some(false),
            crate::VIRTUAL_FEATURES | crate::VISIBILITY_RASTER_FEATURES,
        )
    };
    if !no_rt {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    let mut scene = vec![];
    for g in &b.meshes {
        let vertices: Vec<_> = g
            .positions
            .iter()
            .zip(&g.normals)
            .map(|(&position, &normal)| Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect();
        let mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &g.indices,
            })
            .unwrap()
            .unwrap();
        r.test_drain("N1b bench mesh");
        let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("N1b bench instance");
        let material = r
            .try_create_material(&MaterialDescriptor {
                double_sided: true,
                thin: g.thin,
                base_color: b
                    .material
                    .as_ref()
                    .map_or([0.5, 0.5, 0.5, 1.], |m| m.base_color),
                metallic: b.material.as_ref().map_or(0., |m| m.metallic),
                roughness: g.roughness.unwrap_or(0.6),
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        r.test_drain("N1b bench material");
        scene.push((mesh, instances, material));
    }
    let color = target(&r, b.size[0], b.size[1]);
    let mut rows = vec![];
    let poses: Vec<_> = if b.tracks.is_empty() {
        b.poses.clone()
    } else {
        let f = &b.tracks[0].frames;
        vec![f[0].clone(), f[f.len() / 2].clone(), f[0].clone()]
    };
    for (pose, p) in poses.iter().enumerate() {
        for (g, (_, instances, _)) in b.meshes.iter().zip(&mut scene) {
            let x = match g.name.as_str() {
                "reveal-plate" => p.plate_x,
                "shadow-blocker" => p.blocker_x,
                _ => 0.,
            };
            instances
                .set(0, Mat4::from_translation(Vec3::new(x, 0., 0.)))
                .unwrap();
        }
        let camera = Camera::look_at(
            Vec3::from_array(p.eye),
            Vec3::from_array(p.target),
            Vec3::Y,
            b.fov_y_degrees.unwrap_or(45.).to_radians(),
            0.01,
            100.,
        )
        .unwrap();
        let to = Vec3::from_array(p.to_light.or(b.to_light).unwrap()).normalize();
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: -to,
                    lux: b.sun_lux,
                }],
                environment: [0.; 3],
                exposure_stops: b.exposure_stops,
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
        r.set_soft_shadows(SoftShadows::Virtual);
        for mode in if no_rt {
            vec![PrimaryVisibility::VisibilityBuffer]
        } else {
            vec![
                PrimaryVisibility::Traced,
                PrimaryVisibility::VisibilityBuffer,
            ]
        } {
            r.primary_mode = mode;
            let label = format!("p{pose}-{mode:?}");
            let warm = if m["timing"].as_bool() == Some(true) {
                32
            } else {
                4
            };
            for i in 0..warm + 8 {
                let mut draws: Vec<_> = scene
                    .iter_mut()
                    .map(|(mesh, instances, material)| {
                        Draw::new(mesh, instances).with_material(material)
                    })
                    .collect();
                render(&mut r, &color, &camera, &mut draws);
                if warm == 4 || i + 1 == warm {
                    r.test_drain("N1b bench frame");
                }
                if i + 1 == warm {
                    let _ = r.poll().unwrap();
                } else {
                    timing(&mut r, if i < warm { "warm" } else { &label }, &mut rows);
                }
            }
            r.test_drain("N1b bench timing tail");
            timing(&mut r, &label, &mut rows);
            snapshot(&mut r, &out, &label);
        }
    }
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N1b owned GPU lease: 4 GiB start / 2 GiB abort"]
fn gpu_visibility_fixture() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = no_rt_renderer();
    light(&mut r);
    let (mesh, mut instances) = geometry(
        &mut r,
        &[
            [-2., 0., -2.],
            [2., 0., -2.],
            [2., 0., 2.],
            [-2., 0., 2.],
            [-0.4, 0., 0.],
            [0.4, 0., 0.],
            [0., 0.8, 0.],
        ],
        &[0, 2, 1, 0, 3, 2, 4, 5, 6],
    );
    let color = target(&r, 320, 192);
    let camera = Camera::look_at(
        Vec3::new(0.2, 1.2, 2.5),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let mut rows = vec![];
    for i in 0..12 {
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        assert_eq!(report.primary, PrimaryVisibility::VisibilityBuffer);
        assert_eq!(report.soft_shadows, SoftShadows::Virtual);
        r.test_drain("N1b fixture frame");
        timing(&mut r, if i < 4 { "warm" } else { "no-rt" }, &mut rows);
    }
    snapshot(&mut r, &out, "no-rt");
    let hit = hit_record(&r);
    assert!(hit.chunks_exact(16).any(|h| h[..4] != [0; 4]));
    r.reload(SOURCE).unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("N1b reload");
    assert_eq!(hit, hit_record(&r));
    // Geometry/transform motion and recovery; the return image must be identical.
    instances
        .set(0, Mat4::from_translation(Vec3::new(0.1, 0., 0.)))
        .unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("N1b moved");
    instances.set(0, Mat4::IDENTITY).unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("N1b return");
    assert_eq!(hit, hit_record(&r));
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "N1b toon",
            source: include_str!("../shaders/programs/toon.wgsl"),
        })
        .unwrap();
    let material = r
        .try_create_material(&MaterialDescriptor {
            program: Some(&program),
            double_sided: true,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
    r.test_drain("N1b toon material");
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
    );
    r.test_drain("N1b toon frame");
    assert_eq!(hit, hit_record(&r));
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N1c owned GPU lease: quarry LOD cuts, agreement and paired cost, asset loaded once"]
fn gpu_visibility_quarry() {
    let begun = Instant::now();
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let no_rt = m["no_rt"].as_bool().unwrap_or(false);
    let mut r = if no_rt {
        no_rt_renderer()
    } else {
        renderer_with_features(
            m["validate"].as_bool() != Some(false),
            crate::VIRTUAL_FEATURES | crate::VISIBILITY_RASTER_FEATURES,
        )
    };
    let (mut asset, cameras, load) = load_quarry_lod(&m, &mut r);
    r.test_drain("M1 post-upload memory");
    memory_report(&r, &out, "uploaded");
    fs::write(
        out.join("load.json"),
        serde_json::to_vec_pretty(&load).unwrap(),
    )
    .unwrap();
    if !no_rt {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    // N8h: `soft_shadows: "interactive"` measures the RT shadow path instead (memory runs).
    let interactive = m["soft_shadows"].as_str() == Some("interactive");
    r.set_soft_shadows(if interactive {
        SoftShadows::Interactive
    } else {
        SoftShadows::Virtual
    });
    let color = target(&r, 1920, 1080);
    // Cut error bounds (px; negative = original geometry) and software extents (px).
    let list = |key: &str, default: f32| -> Vec<f32> {
        m[key].as_array().map_or(vec![default], |v| {
            v.iter().map(|x| x.as_f64().unwrap() as f32).collect()
        })
    };
    let lods = list("lod_pixels", visibility_buffer::DEFAULT_LOD_PIXELS);
    let extents = list(
        "software_extent",
        visibility_buffer::DEFAULT_SOFTWARE_EXTENT,
    );
    let lifts = list("lod_lift", visibility_buffer::DEFAULT_LOD_LIFT);
    // N8c: shadow caster cut errors (texels; negative = original geometry) and sun-turning
    // frames timed after each configuration's static frames.
    // The manifest's `virtual` settings (N8c first_block and march_samples, N8e levers).
    let parse = super::film_reference_tests::film_accumulation_tests::virtual_settings;
    let base = parse(&m).unwrap_or_default();
    // N8e: `virtual_variants` {name: settings} runs every configuration once per variant,
    // labelled `-v{name}`, in this one process.
    // N8i: a variant's `constants` {name: number} are lookup override constants.
    use super::film_reference_tests::film_accumulation_tests::virtual_shadow_tests as screens;
    type Variant = (
        String,
        crate::VirtualShadowSettings,
        Vec<(&'static str, f64)>,
    );
    let variants: Vec<Variant> = m["virtual_variants"].as_object().map_or_else(
        || vec![(String::new(), base, vec![])],
        |o| {
            o.iter()
                .map(|(name, v)| {
                    let settings = parse(&serde_json::json!({ "virtual": v }));
                    let constants = screens::screen_constants(v);
                    (format!("-v{name}"), settings.unwrap_or_default(), constants)
                })
                .collect()
        },
    );
    let caster_errors = m["caster_error"]
        .as_array()
        .map(|v| v.iter().map(|x| Some(x.as_f64().unwrap() as f32)).collect())
        .unwrap_or_else(|| vec![None]);
    let turn_frames = m["turn_frames"].as_u64().unwrap_or(0) as usize;
    let lighting =
        lighting_settings::parse(&fs::read_to_string(m["lighting"].as_str().unwrap()).unwrap())
            .unwrap();
    let shadows = r.shadows.settings.clone();
    let poses: Vec<usize> = m["poses"].as_array().map_or(vec![0, 1, 0], |v| {
        v.iter().map(|x| x.as_u64().unwrap() as usize).collect()
    });
    let stage_repeats = m["stage_repeats"].as_u64().unwrap_or(8) as usize;
    let mut records = vec![];
    let mut stages = vec![];
    let mut changed = vec![];
    let mut rows = vec![];
    // The 64-seed Exact reference needs traced fresh visibility (an RT device).
    let seeded = if !no_rt && m["reference"].as_bool() != Some(false) {
        let module = seeded_module(&r);
        (1..=64u32)
            .map(|seed| {
                r.primary.as_ref().unwrap().fresh_pipeline(
                    &r.device,
                    &r.ledger,
                    &module,
                    "fresh_compute",
                    &[("FRESH_SEED", f64::from(seed))],
                )
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    for (pose, index) in poses.into_iter().enumerate() {
        let camera = cameras.frames[index].camera().unwrap();
        if !seeded.is_empty() && pose < 2 {
            r.primary_mode = PrimaryVisibility::Traced;
            r.set_soft_shadows(SoftShadows::Exact);
            render(
                &mut r,
                &color,
                &camera,
                &mut asset.draws(Visibility::Frustum),
            );
            reference(&mut r, &seeded, &out, &format!("p{pose}"));
            r.set_soft_shadows(if interactive {
                SoftShadows::Interactive
            } else {
                SoftShadows::Virtual
            });
        }
        let mut configs = vec![];
        if !no_rt {
            // Traced receivers are full detail, so their casters are too.
            configs.push((PrimaryVisibility::Traced, 0., 0., 0., Some(-1.)));
        }
        for &lod in &lods {
            for &extent in &extents {
                for &lift in &lifts {
                    for &caster in &caster_errors {
                        configs.push((
                            PrimaryVisibility::VisibilityBuffer,
                            lod,
                            extent,
                            lift,
                            caster,
                        ));
                    }
                }
            }
        }
        let configs = configs
            .into_iter()
            .flat_map(|c| variants.iter().map(move |v| (c, v)));
        for ((mode, lod, extent, lift, caster), (variant, settings, constants)) in configs {
            r.primary_mode = mode;
            r.set_lod_error_pixels(lod);
            r.set_software_raster_extent(extent);
            r.set_lod_receiver_lift(lift);
            r.set_virtual_shadow_settings(crate::VirtualShadowSettings {
                caster_error_texels: caster.unwrap_or(settings.caster_error_texels),
                ..*settings
            })
            .unwrap();
            // Each configuration replays an independent sequence, including when the
            // settings equal the preceding configuration's restored settings.
            r.reset_virtual_shadow_cache();
            let suffix = match caster {
                Some(c) if mode != PrimaryVisibility::Traced => format!("-ce{c}{variant}"),
                _ => variant.clone(),
            };
            let label = if mode == PrimaryVisibility::Traced {
                format!("p{pose}-Traced{suffix}")
            } else if lift != 0. {
                format!("p{pose}-lod{lod}-sw{extent}-lift{lift}{suffix}")
            } else {
                format!("p{pose}-lod{lod}-sw{extent}{suffix}")
            };
            let warm = if m["timing"].as_bool() == Some(true) {
                32
            } else {
                4
            };
            let mut first = None;
            for i in 0..warm + 8 {
                // N8i: the variant's constants, once the shadow state exists (an unchanged
                // list rebuilds nothing).
                if r.virtual_shadows.is_some() {
                    screens::set_screen_constants(&mut r, constants.clone());
                }
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                // The first frame after a camera or cut change culls pass 1 with the
                // previous frame's HZB; its final hits must equal the settled frame's.
                if i == 0 && mode == PrimaryVisibility::VisibilityBuffer {
                    r.test_drain("N1c first changed frame");
                    first = Some(hit_record(&r));
                }
                if warm == 4 || i + 1 == warm {
                    r.test_drain("N1c quarry frame");
                }
                if i + 1 == warm {
                    let _ = r.poll().unwrap();
                } else {
                    timing(&mut r, if i < warm { "warm" } else { &label }, &mut rows);
                }
            }
            r.test_drain("N1c quarry timing tail");
            timing(&mut r, &label, &mut rows);
            memory_report(&r, &out, &label);
            records.push(record_counters(&mut r, &label));
            if turn_frames > 0 {
                // N8c: the sun turning every frame re-rasterizes every requested page.
                let Light::Directional { travel, lux } = lighting.lights[0] else {
                    panic!("quarry sun");
                };
                for i in 0..turn_frames {
                    let mut lit = lighting.clone();
                    lit.lights[0] = Light::Directional {
                        travel: Mat4::from_rotation_y(0.0005 * (i + 1) as f32)
                            .transform_vector3(travel),
                        lux,
                    };
                    r.set_lighting_and_shadows(&lit, &shadows).unwrap();
                    render(
                        &mut r,
                        &color,
                        &camera,
                        &mut asset.draws(Visibility::Frustum),
                    );
                    timing(&mut r, &format!("{label}-turn"), &mut rows);
                }
                r.test_drain("N8c quarry turning tail");
                timing(&mut r, &format!("{label}-turn"), &mut rows);
                r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                r.test_drain("N8c quarry sun restored");
                let _ = r.poll().unwrap();
            }
            if let Some(first) = first {
                let settled = first == hit_record(&r);
                changed.push(
                    serde_json::json!({"label": label, "first_frame_hits_equal_settled": settled}),
                );
                assert!(
                    settled,
                    "{label}: first changed frame differs from the settled frame"
                );
            }
            if mode == PrimaryVisibility::VisibilityBuffer && stage_repeats > 0 {
                let ms = stage_profile(&mut r, stage_repeats);
                stages.push(serde_json::json!({"label": label, "lod_pixels": lod, "software_extent": extent, "lod_lift": lift, "stage_ms": ms}));
            }
            snapshot(&mut r, &out, &label);
            // N8f: `move_frames` with the camera stepping 1 mm a frame, then as many with one
            // placement (`move_batch`, slot 0) stepping 1 mm a frame at the pose's camera.
            let move_frames = m["move_frames"].as_u64().unwrap_or(0) as u32;
            if move_frames > 0 {
                let pose = &cameras.frames[index];
                eprintln!(
                    "N8f camera motion {label} from {:.1} s",
                    begun.elapsed().as_secs_f64()
                );
                // N8g: `move_wrap` > 0 walks the camera back and forth over that many steps, so
                // long traced runs stay near the pose (every frame still moves).
                let wrap = m["move_wrap"].as_u64().unwrap_or(0) as u32;
                for i in 1..=move_frames {
                    let at = if wrap > 0 {
                        let k = i % (2 * wrap);
                        k.min(2 * wrap - k)
                    } else {
                        i
                    };
                    let step = Vec3::X * (0.001 * at as f32);
                    let moved = Camera::look_at(
                        Vec3::from_array(pose.eye) + step,
                        Vec3::from_array(pose.target) + step,
                        Vec3::from_array(pose.up),
                        pose.fov_y_radians,
                        pose.near,
                        pose.far,
                    )
                    .unwrap();
                    render(
                        &mut r,
                        &color,
                        &moved,
                        &mut asset.draws(Visibility::Frustum),
                    );
                    timing(&mut r, &format!("{label}-move"), &mut rows);
                }
                r.test_drain("N8f camera motion tail");
                timing(&mut r, &format!("{label}-move"), &mut rows);
                let batch = m["move_batch"].as_u64().unwrap_or(0) as usize;
                let base = asset.model(batch, 0);
                for i in 1..=move_frames {
                    let step = Mat4::from_translation(Vec3::X * (0.001 * i as f32));
                    asset.set_model(batch, 0, step * base).unwrap();
                    render(
                        &mut r,
                        &color,
                        &camera,
                        &mut asset.draws(Visibility::Frustum),
                    );
                    timing(&mut r, &format!("{label}-caster"), &mut rows);
                }
                r.test_drain("N8f caster motion tail");
                timing(&mut r, &format!("{label}-caster"), &mut rows);
                eprintln!("N8f caster motion {label}: {:?}", r.virtual_shadow_stats());
                // Kept counts must equal a full lookup of the final placement (N8f identity).
                let [_, rolled] = interactive_outputs(&r);
                let settings = r.virtual_settings;
                r.set_virtual_shadow_settings(crate::VirtualShadowSettings {
                    refresh: 1,
                    ..settings
                })
                .unwrap();
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                r.test_drain("N8f caster identity");
                let _ = r.poll().unwrap();
                let [_, dense] = interactive_outputs(&r);
                r.set_virtual_shadow_settings(settings).unwrap();
                let differ = rolled
                    .chunks_exact(4)
                    .zip(dense.chunks_exact(4))
                    .filter(|(a, b)| a != b)
                    .count();
                changed.push(serde_json::json!({"label": label, "caster_rolled_differs": differ}));
                eprintln!("N8f caster identity {label}: {differ} pixels differ");
                asset.set_model(batch, 0, base).unwrap();
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                r.test_drain("N8f motion restored");
                let _ = r.poll().unwrap();
            }
        }
    }
    fs::write(
        out.join("stages.json"),
        serde_json::to_vec_pretty(&stages).unwrap(),
    )
    .unwrap();
    fs::write(
        out.join("records.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
    fs::write(
        out.join("changed-frames.json"),
        serde_json::to_vec_pretty(&changed).unwrap(),
    )
    .unwrap();
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N8j owned GPU lease: configuration history must restart at the reset boundary"]
fn gpu_virtual_configuration_reset() {
    let mut r = no_rt_renderer();
    light(&mut r);
    let lighting = Lighting {
        lights: r.lighting.source_lights.clone(),
        environment: [0.03; 3],
        exposure_stops: -8.,
    };
    let shadows = r.shadows.settings.clone();
    let (mesh, mut instances) = geometry(
        &mut r,
        &[
            [-2., 0., -2.],
            [2., 0., -2.],
            [2., 0., 2.],
            [-2., 0., 2.],
            [-0.4, 0., 0.],
            [0.4, 0., 0.],
            [0., 0.8, 0.],
        ],
        &[0, 2, 1, 0, 3, 2, 4, 5, 6],
    );
    let color = target(&r, 320, 192);
    let camera = Camera::look_at(
        Vec3::new(0.2, 1.2, 2.5),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let settings = crate::VirtualShadowSettings {
        pages: 4,
        walk_records: crate::WalkRecords::Off,
        ..Default::default()
    };
    let mut expected = None;
    for (records, reset) in [
        (crate::WalkRecords::Off, false),
        (crate::WalkRecords::Source, false),
        (crate::WalkRecords::Source, true),
        (crate::WalkRecords::Off, false),
        (crate::WalkRecords::Off, true),
    ] {
        let settings = crate::VirtualShadowSettings {
            walk_records: records,
            ..settings
        };
        r.set_virtual_shadow_settings(settings).unwrap();
        assert_eq!(
            r.virtual_settings, settings,
            "saved settings must stay current"
        );
        if reset {
            r.reset_virtual_shadow_cache();
        }
        for _ in 0..40 {
            render(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut instances)],
            );
        }
        let Light::Directional { travel, lux } = lighting.lights[0] else {
            unreachable!()
        };
        for i in 1..=16 {
            let mut lit = lighting.clone();
            lit.lights[0] = Light::Directional {
                travel: Mat4::from_rotation_y(0.0005 * i as f32).transform_vector3(travel),
                lux,
            };
            r.set_lighting_and_shadows(&lit, &shadows).unwrap();
            render(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut instances)],
            );
        }
        r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
        render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("N8j configuration");
        let image = interactive_outputs(&r);
        if let Some(expected) = &expected {
            assert!(
                image == *expected,
                "{records:?}, reset={reset}: fresh counts and receivers"
            );
        } else {
            expected = Some(image);
        }
        let out = PathBuf::from(manifest()["output"].as_str().unwrap());
        fs::create_dir_all(&out).unwrap();
        let mut png = png::Encoder::new(
            fs::File::create(out.join(format!("reset-{records:?}-{reset}.png"))).unwrap(),
            320,
            192,
        );
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&read_texture(&r, &color))
            .unwrap();
        // Leave caster bounds, the light basis and rolling counts from another frame.
        // Restore inputs without rendering them: the next boundary must discard history.
        instances
            .set(0, Mat4::from_translation(Vec3::new(0., 0.1, 0.)))
            .unwrap();
        let mut moved = lighting.clone();
        moved.lights[0] = Light::Directional {
            travel: -Vec3::new(0.6, 0.8, 0.3).normalize(),
            lux,
        };
        r.set_lighting_and_shadows(&moved, &shadows).unwrap();
        render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        instances.set(0, Mat4::IDENTITY).unwrap();
        r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    }
}
