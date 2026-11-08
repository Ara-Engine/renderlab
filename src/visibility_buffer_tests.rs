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
            anisotropy_clamp: test_anisotropy(),
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
/// Opt-in caller-thread wall timing, including motion updates and busy admission.
/// GPU spans remain separate; serialization happens after the measured interval.
fn latency_cpu(
    rows: &mut Vec<serde_json::Value>,
    enabled: bool,
    label: &str,
    start: Instant,
    report: &FrameReport,
) {
    if !enabled {
        return;
    }
    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
    let ms = |v: Duration| v.as_secs_f64() * 1000.0;
    let c = &report.cpu;
    rows.push(
        serde_json::json!({"label":label,"frame":report.frame.0,"caller_wall_ms":wall_ms,
        "preparation_ms":ms(c.preparation+c.visibility+c.packing),
        "shadow_preparation_ms":ms(c.shadow_preparation),"upload_ms":ms(c.upload+c.shadow_upload),
        "encode_ms":ms(c.encode+c.shadow_encode),"submit_ms":ms(c.submit),"poll_ms":ms(c.poll)}),
    );
}

fn timing(r: &mut Renderer, label: &str, rows: &mut Vec<serde_json::Value>) {
    for t in r.poll().unwrap().gpu {
        let ms = |v: Duration| v.as_secs_f64() * 1000.;
        // shade_ms: traced frames time their deferred shading pass with the mesh-pass stamps.
        rows.push(serde_json::json!({"label":label,"frame":t.frame.0,"span_ms":ms(t.render_span),"primary_ms":t.receiver_pass.map(ms),"shadow_ms":t.visibility_compute.map(ms),"shade_ms":ms(t.mesh_pass),"tone_ms":t.present_pass.map(ms)}));
    }
}
/// N8n: with `VSC_STATS` among a variant's constants, the surface cache's counters after each
/// frame (drains the GPU, so that variant's timings are not used).
fn surface_stats(
    r: &mut Renderer,
    on: bool,
    label: &str,
    frame: usize,
    stats: &mut Vec<serde_json::Value>,
) {
    if !on {
        return;
    }
    if let Some([probes, hits, reached, sun, unshown, misses, overflow, fills]) =
        r.test_surface_stats()
    {
        stats.push(
            serde_json::json!({"label": label, "frame": frame, "probes": probes,
            "hits": hits, "caster_reached": reached, "sun_refreshes": sun, "unshown": unshown,
            "misses": misses, "margin_overflow": overflow, "fills": fills}),
        );
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
        "hardware": r.hardware_shadow_stats().map(|h| serde_json::json!({
            "blas_builds": h.blas_builds, "blas_compactions": h.blas_compactions,
            "pending_builds": h.pending_builds, "retained_generations": h.retained_generations,
        })),
        "residency": residency_report(r),
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
/// M3: streamed-texture residency counters, and per texture its resident top and the
/// finest surveyed level (when `survey` probes anisotropic samples).
fn residency_report(r: &Renderer) -> serde_json::Value {
    let s = r.residency_stats();
    serde_json::json!({
        "mode": format!("{:?}", r.texture_residency()), "survey": r.residency.survey,
        "resident_bytes": s.resident_bytes, "full_bytes": s.full_bytes, "misses": s.misses,
        "missed_frames": s.missed_frames, "verified_frames": s.verified_frames,
        "reshaded_frames": s.reshaded_frames, "serviced_misses": s.serviced_misses,
        "transitions": s.transitions, "uploaded_bytes": s.uploaded_bytes,
        "textures": r.test_residency_entries(),
    })
}
/// N8h: the triangles with walk records under this configuration.
fn record_counters(r: &mut Renderer, label: &str) -> serde_json::Value {
    serde_json::json!({"label": label, "recorded": r.test_walk_records(), "residency": residency_report(r)})
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
    // Preserve N1b's recorded Exact reference output.
    r.set_virtual_shadow_settings(crate::VirtualShadowSettings {
        lookup: crate::VirtualLookup::Exact,
        ..Default::default()
    })
    .unwrap();
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
    // Preserve N1b's recorded Exact reference output.
    r.set_virtual_shadow_settings(crate::VirtualShadowSettings {
        lookup: crate::VirtualLookup::Exact,
        ..Default::default()
    })
    .unwrap();
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

/// Pixels whose 4-byte counts differ.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| x != y)
        .count()
}
/// N8o caster gates (docs/non-rt-n8o.md) at one pose, for the configuration's lookup. Every
/// sequence starts from a reset and ends with `frames` frames at the pose in which placement 0 of
/// `batch` steps 1 mm a frame along x (`caster`) or holds still. `at(x)` is the pose's camera
/// moved x metres along x.
/// - Static: `frames` frames at the pose first. The rolled counts must equal a fresh lookup's (a
///   reset, then one frame: every pixel walks) and the counts of the same frames with the
///   placement final throughout.
/// - History: N8n's approach (16 frames of 2 mm), 8 frames at the pose, then `frames` camera
///   frames 1 mm apart, away and back. A pixel may differ from a fresh lookup only where the twin
///   whose caster holds still differs too, with the same count: it shows a cell computed at
///   another camera position, which no moved caster reached.
/// - Determinism: the moving history runs twice, and both counts' hashes are recorded.
fn caster_gates(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    at: &dyn Fn(f32) -> Camera,
    batch: usize,
    frames: u32,
) -> serde_json::Value {
    use sha2::Digest;
    let sha256 = |bytes: &[u8]| format!("{:x}", sha2::Sha256::digest(bytes));
    let base = asset.model(batch, 0);
    let place = |asset: &mut prepared::Prepared, i: u32| {
        let step = Mat4::from_translation(Vec3::X * (0.001 * i as f32));
        asset.set_model(batch, 0, step * base).unwrap();
    };
    let frame = |r: &mut Renderer, asset: &mut prepared::Prepared, camera: &Camera| {
        render(r, color, camera, &mut asset.draws(Visibility::Frustum));
        let _ = r.poll().unwrap();
    };
    let counts = |r: &mut Renderer| {
        r.test_drain("N8o caster gate");
        let [_, counts] = interactive_outputs(r);
        counts
    };
    let pose = at(0.0);
    let fresh = |r: &mut Renderer, asset: &mut prepared::Prepared| {
        r.reset_virtual_shadow_cache();
        frame(r, asset, &pose);
        counts(r)
    };
    // The last `frames` frames, with the caster stepping from its base or holding.
    let tail = |r: &mut Renderer, asset: &mut prepared::Prepared, caster: bool| {
        for i in 1..=frames {
            if caster {
                place(asset, i);
            }
            frame(r, asset, &pose);
        }
        counts(r)
    };
    let still = |r: &mut Renderer, asset: &mut prepared::Prepared, caster: bool| {
        r.reset_virtual_shadow_cache();
        place(asset, if caster { 0 } else { frames });
        for _ in 0..frames {
            frame(r, asset, &pose);
        }
        tail(r, asset, caster)
    };
    let history = |r: &mut Renderer, asset: &mut prepared::Prepared, caster: bool| {
        r.reset_virtual_shadow_cache();
        place(asset, 0);
        for i in (1..=16).rev() {
            frame(r, asset, &at(0.002 * i as f32));
        }
        for _ in 0..8 {
            frame(r, asset, &pose);
        }
        for i in 1..=frames {
            frame(r, asset, &at(0.001 * i.min(frames - i) as f32));
        }
        tail(r, asset, caster)
    };
    let rolled = still(r, asset, true);
    let rolled_fresh = fresh(r, asset);
    let twin = still(r, asset, false);
    let moving = history(r, asset, true);
    let moving_fresh = fresh(r, asset);
    let again = history(r, asset, true);
    let held = history(r, asset, false);
    let held_fresh = fresh(r, asset);
    place(asset, 0);
    let mut unexplained = 0;
    let pixels = moving.chunks_exact(4).zip(moving_fresh.chunks_exact(4));
    let twins = held.chunks_exact(4).zip(held_fresh.chunks_exact(4));
    for ((m, f), (h, g)) in pixels.zip(twins) {
        if m != f && (h == g || m != h) {
            unexplained += 1;
        }
    }
    serde_json::json!({
        "static_differs_from_fresh": differing(&rolled, &rolled_fresh),
        "static_differs_from_twin": differing(&rolled, &twin),
        "history_differs_from_fresh": differing(&moving, &moving_fresh),
        "history_twin_differs_from_fresh": differing(&held, &held_fresh),
        "history_unexplained": unexplained,
        "history_sha256": sha256(&moving),
        "history_again_sha256": sha256(&again),
        "history_repeat_differs": differing(&moving, &again),
        "static_sha256": sha256(&rolled),
    })
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
    // M3: `residency` full/exact/feedback (default full) before the uploads, and
    // `survey` to probe anisotropic samples for the sparse-residency projection.
    let mode = m["residency"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| std::env::var("RENDERLAB_RESIDENCY").ok());
    r.set_texture_residency(match mode.as_deref() {
        Some("full") => TextureResidency::Full,
        Some("feedback") => TextureResidency::Feedback,
        Some("exact") => TextureResidency::Exact,
        _ => TextureResidency::Full,
    });
    r.residency.survey = m["survey"].as_bool() == Some(true)
        || std::env::var_os("RENDERLAB_RESIDENCY_SURVEY").is_some();
    let first_hashes = m["first_hashes"].as_bool() == Some(true);
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
    let mut cpu_rows = vec![];
    let cpu_probe = m["cpu_timing"].as_bool() == Some(true);
    // M3: wall-clock totals of the turn, camera and caster phases, frames through their
    // tail drain: throughput, which per-frame GPU spans cannot show.
    let mut walls = vec![];
    let wall = |walls: &mut Vec<serde_json::Value>,
                label: String,
                frames: usize,
                start: Instant| {
        let ms = start.elapsed().as_secs_f64() * 1e3;
        walls.push(serde_json::json!({"label": label, "frames": frames, "wall_ms": ms, "per_frame_ms": ms / frames.max(1) as f64}));
    };
    let mut cache_rows = vec![];
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
            // N8n: `approach_frames` camera frames stepping `approach_step` metres a frame along
            // x into the pose first, so the pose's frames start from what motion cached.
            let approach = m["approach_frames"].as_u64().unwrap_or(0) as u32;
            let approach_step = m["approach_step"].as_f64().unwrap_or(0.002) as f32;
            for i in (1..=approach).rev() {
                let pose = &cameras.frames[index];
                let step = Vec3::X * (approach_step * i as f32);
                let moved = Camera::look_at(
                    Vec3::from_array(pose.eye) + step,
                    Vec3::from_array(pose.target) + step,
                    Vec3::from_array(pose.up),
                    pose.fov_y_radians,
                    pose.near,
                    pose.far,
                )
                .unwrap();
                r.primary_mode = mode;
                render(
                    &mut r,
                    &color,
                    &moved,
                    &mut asset.draws(Visibility::Frustum),
                );
                let _ = r.poll().unwrap();
            }
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
            let stats = constants
                .iter()
                .any(|&(name, v)| name == "VSC_STATS" && v != 0.0);
            let mut first = None;
            for i in 0..warm + 8 {
                let cpu_start = Instant::now();
                // N8i: the variant's constants, once the shadow state exists (an unchanged
                // list rebuilds nothing).
                if r.virtual_shadows.is_some() {
                    screens::set_screen_constants(&mut r, constants.clone());
                }
                let cpu_report = render(
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
                // M3: the first frame after a pose or configuration change, as returned.
                if i == 0 && first_hashes {
                    r.test_drain("M3 first frame");
                    use sha2::Digest;
                    let hdr = capture(&mut r);
                    let hits = hit_record(&r);
                    changed.push(serde_json::json!({
                        "label": label,
                        "first_hdr_sha256": format!("{:x}", sha2::Sha256::digest(&hdr)),
                        "first_hit_sha256": format!("{:x}", sha2::Sha256::digest(&hits)),
                        "residency": residency_report(&r),
                    }));
                }
                if warm == 4 || i + 1 == warm {
                    r.test_drain("N1c quarry frame");
                }
                if i + 1 == warm {
                    let _ = r.poll().unwrap();
                } else {
                    timing(&mut r, if i < warm { "warm" } else { &label }, &mut rows);
                }
                surface_stats(
                    &mut r,
                    stats,
                    &format!("{label}-static"),
                    i,
                    &mut cache_rows,
                );
                latency_cpu(&mut cpu_rows, cpu_probe, &label, cpu_start, &cpu_report);
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
                let start = Instant::now();
                // N8o: `turn_wrap` > 0 turns the sun back and forth over that many 0.5 mrad steps
                // (traces of long turning runs inside the map's hysteresis).
                let turn_wrap = m["turn_wrap"].as_u64().unwrap_or(0) as usize;
                for i in 0..turn_frames {
                    let cpu_start = Instant::now();
                    let at = if turn_wrap > 0 {
                        let k = (i + 1) % (2 * turn_wrap);
                        k.min(2 * turn_wrap - k)
                    } else {
                        i + 1
                    };
                    let mut lit = lighting.clone();
                    lit.lights[0] = Light::Directional {
                        travel: Mat4::from_rotation_y(0.0005 * at as f32).transform_vector3(travel),
                        lux,
                    };
                    r.set_lighting_and_shadows(&lit, &shadows).unwrap();
                    let cpu_report = render(
                        &mut r,
                        &color,
                        &camera,
                        &mut asset.draws(Visibility::Frustum),
                    );

                    timing(&mut r, &format!("{label}-turn"), &mut rows);
                    surface_stats(&mut r, stats, &format!("{label}-turn"), i, &mut cache_rows);
                    latency_cpu(
                        &mut cpu_rows,
                        cpu_probe,
                        &format!("{label}-turn"),
                        cpu_start,
                        &cpu_report,
                    );
                }
                r.test_drain("N8c quarry turning tail");
                wall(&mut walls, format!("{label}-turn"), turn_frames, start);
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
                let start = Instant::now();
                // N8n: `move_step` metres a frame along x (default 1 mm), and `move_yaw` radians
                // a frame of pan about the vertical through the eye (default none).
                let step_metres = m["move_step"].as_f64().unwrap_or(0.001) as f32;
                let yaw = m["move_yaw"].as_f64().unwrap_or(0.0) as f32;
                for i in 1..=move_frames {
                    let cpu_start = Instant::now();
                    let at = if wrap > 0 {
                        let k = i % (2 * wrap);
                        k.min(2 * wrap - k)
                    } else {
                        i
                    };
                    let step = Vec3::X * (step_metres * at as f32);
                    let eye = Vec3::from_array(pose.eye);
                    let turned = glam::Mat3::from_rotation_y(yaw * at as f32)
                        * (Vec3::from_array(pose.target) - eye);
                    let moved = Camera::look_at(
                        eye + step,
                        eye + step + turned,
                        Vec3::from_array(pose.up),
                        pose.fov_y_radians,
                        pose.near,
                        pose.far,
                    )
                    .unwrap();
                    let cpu_report = render(
                        &mut r,
                        &color,
                        &moved,
                        &mut asset.draws(Visibility::Frustum),
                    );

                    timing(&mut r, &format!("{label}-move"), &mut rows);
                    let i = i as usize;
                    surface_stats(&mut r, stats, &format!("{label}-move"), i, &mut cache_rows);
                    latency_cpu(
                        &mut cpu_rows,
                        cpu_probe,
                        &format!("{label}-move"),
                        cpu_start,
                        &cpu_report,
                    );
                }
                r.test_drain("N8f camera motion tail");
                let [_, moved_counts] = interactive_outputs(&r);
                {
                    use sha2::Digest;
                    let sha = format!("{:x}", sha2::Sha256::digest(&moved_counts));
                    changed.push(serde_json::json!({"label": label, "move_counts_sha256": sha}));
                }
                wall(
                    &mut walls,
                    format!("{label}-move"),
                    move_frames as usize,
                    start,
                );
                timing(&mut r, &format!("{label}-move"), &mut rows);
                let batch = m["move_batch"].as_u64().unwrap_or(0) as usize;
                let base = asset.model(batch, 0);
                let start = Instant::now();
                // N8o: `caster_frames` (default `move_frames`) caster-moving frames; `caster_wrap`
                // > 0 steps the placement back and forth over that many millimetres (traces).
                let caster_frames = m["caster_frames"]
                    .as_u64()
                    .map_or(move_frames, |n| n as u32);
                let caster_wrap = m["caster_wrap"].as_u64().unwrap_or(0) as u32;
                for i in 1..=caster_frames {
                    let cpu_start = Instant::now();
                    let at = if caster_wrap > 0 {
                        let k = i % (2 * caster_wrap);
                        k.min(2 * caster_wrap - k)
                    } else {
                        i
                    };
                    let step = Mat4::from_translation(Vec3::X * (0.001 * at as f32));
                    asset.set_model(batch, 0, step * base).unwrap();
                    let cpu_report = render(
                        &mut r,
                        &color,
                        &camera,
                        &mut asset.draws(Visibility::Frustum),
                    );

                    timing(&mut r, &format!("{label}-caster"), &mut rows);
                    let i = i as usize;
                    surface_stats(
                        &mut r,
                        stats,
                        &format!("{label}-caster"),
                        i,
                        &mut cache_rows,
                    );
                    latency_cpu(
                        &mut cpu_rows,
                        cpu_probe,
                        &format!("{label}-caster"),
                        cpu_start,
                        &cpu_report,
                    );
                }
                r.test_drain("N8f caster motion tail");
                wall(
                    &mut walls,
                    format!("{label}-caster"),
                    caster_frames as usize,
                    start,
                );
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
                {
                    use sha2::Digest;
                    let sha = format!("{:x}", sha2::Sha256::digest(&rolled));
                    changed.push(serde_json::json!({"label": label, "caster_rolled_differs": differ, "caster_counts_sha256": sha}));
                }
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
            // N8o: `caster_gates` runs the caster gates at this pose (`move_frames`, default 48).
            if m["caster_gates"].as_bool() == Some(true) {
                let pose = &cameras.frames[index];
                let at = |x: f32| {
                    let step = Vec3::X * x;
                    Camera::look_at(
                        Vec3::from_array(pose.eye) + step,
                        Vec3::from_array(pose.target) + step,
                        Vec3::from_array(pose.up),
                        pose.fov_y_radians,
                        pose.near,
                        pose.far,
                    )
                    .unwrap()
                };
                let batch = m["move_batch"].as_u64().unwrap_or(0) as usize;
                let frames = m["move_frames"].as_u64().filter(|&n| n > 0).unwrap_or(48) as u32;
                let gates = caster_gates(&mut r, &mut asset, &color, &at, batch, frames);
                eprintln!("N8o caster gates {label}: {gates}");
                changed.push(serde_json::json!({"label": label, "caster_gates": gates}));
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
    fs::write(
        out.join("wall.json"),
        serde_json::to_vec_pretty(&walls).unwrap(),
    )
    .unwrap();
    if cpu_probe {
        fs::write(
            out.join("cpu.json"),
            serde_json::to_vec_pretty(&cpu_rows).unwrap(),
        )
        .unwrap();
    }
    if !cache_rows.is_empty() {
        fs::write(
            out.join("surface-stats.json"),
            serde_json::to_vec_pretty(&cache_rows).unwrap(),
        )
        .unwrap();
    }
}

/// Measurement-only RT1c: the M1 upload/allocation/WDDM harness, without image
/// readbacks. Fresh processes keep each configuration's memory independent.
#[test]
#[ignore = "RT1c quiet GPU lease; optimized binary; no Cargo during the lease"]
fn gpu_rt1c_m1_cost() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer_with_features(
        false,
        crate::VIRTUAL_FEATURES | crate::VISIBILITY_RASTER_FEATURES,
    );
    assert_eq!(r.primary_mode, PrimaryVisibility::Raster);
    assert_eq!(r.soft_shadows(), SoftShadows::Exact);
    let (mut asset, cameras, load) = load_quarry_lod(&m, &mut r);
    r.test_drain("RT1c M1 upload");
    memory_report(&r, &out, "uploaded");
    fs::write(
        out.join("load.json"),
        serde_json::to_vec_pretty(&load).unwrap(),
    )
    .unwrap();
    let raster = m["rt1c_path"].as_str() == Some("rt-raster-exact");
    let (primary, soft) = if raster {
        (PrimaryVisibility::Raster, SoftShadows::Exact)
    } else {
        assert_eq!(m["rt1c_path"].as_str(), Some("rt"));
        (PrimaryVisibility::Traced, SoftShadows::Interactive)
    };
    r.set_primary_visibility(primary).unwrap();
    r.set_soft_shadows(soft);
    let color = target(&r, 1920, 1080);
    let lighting =
        lighting_settings::parse(&fs::read_to_string(m["lighting"].as_str().unwrap()).unwrap())
            .unwrap();
    let shadows = r.shadows.settings.clone();
    let Light::Directional { travel, lux } = lighting.lights[0] else {
        panic!("quarry sun");
    };
    let original = asset.model(0, 0);
    let visits = m["visits"].as_u64().unwrap_or(8) as usize;
    let mut pending = std::collections::HashMap::new();
    let mut rows = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1000.;
    let mut collect = |r: &mut Renderer,
                       pending: &mut std::collections::HashMap<
        u64,
        (usize, usize, &'static str),
    >| {
        for g in r.poll().unwrap().gpu {
            if let Some((pose, visit, phase)) = pending.remove(&g.frame.0) {
                rows.push(serde_json::json!({"pose":pose,"visit":visit,"phase":phase,"frame":g.frame.0,
                    "primary_ms":g.receiver_pass.map(ms),"shadow_ms":g.visibility_compute.map(ms),
                    "shade_ms":ms(g.mesh_pass),"tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
            }
        }
    };
    for pose in [0, 1] {
        let p = &cameras.frames[pose];
        let camera = p.camera().unwrap();
        for visit in 0..visits {
            asset.set_model(0, 0, original).unwrap();
            r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
            for _ in 0..32 {
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                collect(&mut r, &mut pending);
            }
            r.test_drain("RT1c M1 warmup");
            collect(&mut r, &mut pending);
            memory_report(&r, &out, &format!("p{pose}-visit{visit}"));
            for (phase, frames) in [("static", 8), ("sun", 16), ("camera", 48), ("caster", 48)] {
                if phase == "camera" {
                    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
                }
                for frame in 1..=frames {
                    let mut moved = None;
                    match phase {
                        "sun" => {
                            let mut lit = lighting.clone();
                            lit.lights[0] = Light::Directional {
                                travel: Mat4::from_rotation_y(0.0005 * frame as f32)
                                    .transform_vector3(travel),
                                lux,
                            };
                            r.set_lighting_and_shadows(&lit, &shadows).unwrap();
                        }
                        "camera" => {
                            let step = Vec3::X * (0.001 * frame as f32);
                            moved = Some(
                                Camera::look_at(
                                    Vec3::from_array(p.eye) + step,
                                    Vec3::from_array(p.target) + step,
                                    Vec3::from_array(p.up),
                                    p.fov_y_radians,
                                    p.near,
                                    p.far,
                                )
                                .unwrap(),
                            );
                        }
                        "caster" => {
                            asset
                                .set_model(
                                    0,
                                    0,
                                    Mat4::from_translation(Vec3::X * (0.001 * frame as f32))
                                        * original,
                                )
                                .unwrap();
                        }
                        _ => {}
                    }
                    let report = render(
                        &mut r,
                        &color,
                        moved.as_ref().unwrap_or(&camera),
                        &mut asset.draws(Visibility::Frustum),
                    );
                    assert_eq!(report.soft_shadows, soft);
                    pending.insert(report.frame.0, (pose, visit, phase));
                    collect(&mut r, &mut pending);
                }
                r.test_drain("RT1c phase tail");
                collect(&mut r, &mut pending);
                assert!(pending.is_empty());
            }
        }
    }
    fs::write(
        out.join("m1-cost.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "primary":format!("{primary:?}"),"shadows":format!("{soft:?}"),"visits_per_pose":visits,
            "warm_frames":32,"samples":rows,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "M1 owned GPU lease: repair capacity, mode round trips and compacted BLAS"]
fn gpu_primary_memory_modes() {
    let mut r = renderer_with_features(
        true,
        crate::VIRTUAL_FEATURES | crate::VISIBILITY_RASTER_FEATURES,
    );
    // Three triangles: permit their initial build and scratch, but no compacted copy.
    r.hardware_shadows
        .as_mut()
        .unwrap()
        .options
        .max_estimated_bytes = Some(3 * (256 + 128));
    light(&mut r);
    r.set_virtual_shadow_settings(crate::VirtualShadowSettings {
        pages: 4,
        refresh: 1,
        lookup: crate::VirtualLookup::Exact,
        ..Default::default()
    })
    .unwrap();
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
    let modes = [
        (PrimaryVisibility::Traced, SoftShadows::Interactive),
        (PrimaryVisibility::Traced, SoftShadows::Virtual),
        (PrimaryVisibility::VisibilityBuffer, SoftShadows::Virtual),
    ];
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(SoftShadows::Interactive);
    // Admit this one-instance TLAS and its scratch, but no second BLAS generation.
    let mesh_estimate = r.stats().ownership.hardware_as_estimated_bytes.live;
    r.hardware_shadows
        .as_mut()
        .unwrap()
        .options
        .max_estimated_bytes = Some(mesh_estimate + 2 * (4096 + 256));
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("M1 compaction policy fallback");
    assert_eq!(r.hardware_shadow_stats().unwrap().blas_compactions, 0);
    let original = (interactive_outputs(&r), capture(&mut r));
    r.hardware_shadows
        .as_mut()
        .unwrap()
        .options
        .max_estimated_bytes = None;
    let mut expected = vec![];
    for visit in 0..2 {
        for (i, &(primary, shadows)) in modes.iter().enumerate() {
            r.set_primary_visibility(primary).unwrap();
            r.set_soft_shadows(shadows);
            r.reset_virtual_shadow_cache();
            for _ in 0..2 {
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut [Draw::new(&mesh, &mut instances)],
                );
            }
            r.test_drain("M1 memory mode round trip");
            let t = r.primary.as_ref().unwrap().targets.as_ref().unwrap();
            let interactive = shadows == SoftShadows::Interactive;
            assert_eq!(
                t.repairs.value.size(),
                if interactive { 16 + 320 * 192 * 36 } else { 52 }
            );
            assert_eq!(
                t.retries.value.size(),
                if interactive { 16 + 320 * 192 * 4 } else { 20 }
            );
            let image = (interactive_outputs(&r), capture(&mut r));
            if i == 0 {
                assert!(image == original, "compaction changed receivers or HDR");
            }
            if visit == 0 {
                expected.push(image);
            } else {
                assert!(
                    image == expected[i],
                    "mode round trip {primary:?}/{shadows:?}"
                );
            }
        }
    }
    assert_eq!(r.hardware_shadow_stats().unwrap().blas_compactions, 1);
}

#[test]
#[ignore = "M2 owned GPU lease: immutable table sharing and generation release"]
fn gpu_shared_cluster_tables() {
    use crate::renderer::{PreparedDraw, visibility_buffer::ClusterInstances};
    fn draws<'a>(mesh: &'a Mesh, instances: &'a Instances) -> [PreparedDraw<'a>; 1] {
        [PreparedDraw {
            mesh,
            instances,
            material: None,
            indices: 0..mesh.index_count,
            slots: 0..instances.len() as u32,
            reference_ids: None,
        }]
    }
    let mut r = no_rt_renderer();
    let positions = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
    let (mesh, mut instances) = geometry(&mut r, &positions, &[0, 1, 2]);
    let first = ClusterInstances::new(&r, &draws(&mesh, &instances)).unwrap();
    instances.set(0, Mat4::from_translation(Vec3::Y)).unwrap();
    let second = ClusterInstances::new(&r, &draws(&mesh, &instances)).unwrap();
    assert!(Arc::ptr_eq(&first.clusters, &second.clusters));
    assert!(Arc::ptr_eq(&first.groups, &second.groups));
    let doubled = r
        .try_upload_instances(&[Mat4::IDENTITY; 2])
        .unwrap()
        .unwrap();
    r.test_drain("M2 table instances");
    let changed = ClusterInstances::new(&r, &draws(&mesh, &doubled)).unwrap();
    assert_eq!(changed.count, first.count * 2);
    assert!(!Arc::ptr_eq(&first.clusters, &changed.clusters));
    let weak = Arc::downgrade(&changed.clusters);
    let (replacement, _) = geometry(&mut r, &positions, &[0, 1, 2]);
    let replaced = ClusterInstances::new(&r, &draws(&replacement, &doubled)).unwrap();
    assert!(!Arc::ptr_eq(&changed.clusters, &replaced.clusters));
    drop(changed);
    assert!(weak.upgrade().is_none(), "cache must not retain GPU tables");
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
        lookup: crate::VirtualLookup::Exact,
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

#[path = "texture_residency_tests.rs"]
mod texture_residency_tests;

#[path = "temporal_reconstruction_tests.rs"]
mod temporal_reconstruction_tests;

#[path = "virtual_limits_tests.rs"]
mod virtual_limits_tests;
