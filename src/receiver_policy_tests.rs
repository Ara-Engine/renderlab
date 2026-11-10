//! Production receiver-policy references and independent analytic fixtures.
use super::*;

#[path = "adjacency_parallel_gpu_tests.rs"]
mod adjacency_parallel_gpu_tests;
#[path = "receiver_diagnostic_tests.rs"]
mod receiver_diagnostic_tests;
#[path = "receiver_fold_tests.rs"]
mod receiver_fold_tests;

#[derive(Clone, Copy, Debug)]
pub(super) enum Policy {
    Production,
}

pub(super) fn sources(_: Policy) -> FilmSources {
    // Only footprint jitter and independent shadow salts are test instrumentation.
    // Receiver reconstruction, side, scale and t_min are all production code.
    film_sources()
}

pub(super) fn direct_source() -> String {
    // Preserve F0/R1's environment reset in the standard program itself. A separate
    // sun-only shade body changes rounding on the quarry, even with the same BRDF.
    let program = edit(
        STANDARD,
        (
            "    for(var i=0u;i<light_count();i+=1u) {",
            "    exposed=vec3<f32>(0.0);\n    for(var i=0u;i<light_count();i+=1u) {",
        ),
    );
    let source = film_shading(&program);
    assert_eq!(source.matches("primary_frame.clear").count(), 3);
    source.replace("primary_frame.clear", "vec4<f32>(0.0,0.0,0.0,1.0)")
}

pub(super) fn accumulator_source(r1: bool) -> String {
    let source = include_str!("../support/film_accumulate.wgsl");
    if !r1 {
        // Uncovered trace tiles leave stale hit records. F0 predates that engine
        // contract: reject those samples as R1 already does, preserving its estimand.
        let source = edit(
            source,
            (
                "    if h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
                "    if textureLoad(tiles,id.xy/vec2<u32>(8u,4u),0).x!=0u && h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
            ),
        );
        return format!("{source}\n@group(0) @binding(8) var tiles:texture_2d<u32>;\n");
    }
    let source = edit(
        source,
        (
            "    if h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
            "    saved[i]=0xffffu;\n    if textureLoad(tiles,id.xy/vec2<u32>(8u,4u),0).x!=0u && h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
        ),
    );
    let source = edit(
        &source,
        (
            "        let v=textureLoad(visibility,p,0).x;",
            "        let v=textureLoad(visibility,p,0).x;\n        saved[i]=v;",
        ),
    );
    format!(
        "{source}\n@group(0) @binding(7) var<storage,read_write> saved:array<u32>;\n@group(0) @binding(8) var tiles:texture_2d<u32>;\n"
    )
}

#[test]
fn receiver_policy_sources_compile() {
    for policy in [Policy::Production] {
        let sources = sources(policy);
        for source in [sources.kernels, sources.shading] {
            validate_wgsl(&source);
        }
    }
    validate_wgsl(&direct_source());
    validate_wgsl(&accumulator_source(true));
}

#[test]
fn receiver_precision_scale() {
    let epsilon = |m: f32| 1e-5f32.max(16.0 * (f32::from_bits(m.to_bits() + 1) - m));
    assert_eq!(epsilon(4.0), 1e-5);
    assert_eq!(epsilon(1000.0), 0.0009765625);
    assert_eq!(epsilon(10000.0), 0.015625);
}

#[test]
#[ignore = "owned GPU lease; production film against immutable R1 viewed references"]
fn gpu_receiver_policy_r1_equivalence() {
    let m = manifest();
    assert!(
        m["film_shader_edits"].is_null() && m["film_frozen_modules"].is_null(),
        "R1 acceptance must exercise production receiver code"
    );
    super::gpu_film_reference();
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    let reference = PathBuf::from(m["r1_reference"].as_str().unwrap());
    let pose = m["pose"].as_u64().unwrap_or(0);
    let mut comparisons = vec![];
    for kind in ["film", "direct"] {
        let file = format!("pose-{pose}.{kind}.bin");
        let actual = fs::read(out.join(&file)).unwrap();
        let expected = fs::read(reference.join(&file)).unwrap();
        assert_eq!(actual.len(), expected.len());
        let mut visibility_identical = true;
        let mut max_radiance_error = 0.0f32;
        for (a, b) in actual.chunks_exact(48).zip(expected.chunks_exact(48)) {
            visibility_identical &= a[..16] == b[..16];
            for (a, b) in a[32..].chunks_exact(4).zip(b[32..].chunks_exact(4)) {
                let a = f32::from_le_bytes(a.try_into().unwrap()) / 256.0;
                let b = f32::from_le_bytes(b.try_into().unwrap()) / 256.0;
                max_radiance_error = max_radiance_error.max((a - b).abs());
            }
        }
        comparisons.push(serde_json::json!({"kind":kind,"byte_identical":actual==expected,
            "visibility_counts_identical":visibility_identical,"max_linear_radiance_error":max_radiance_error}));
        // R1's documented radiance replay roundoff, with exact visibility sums
        // and accepted counts. This is not a relaxed image-quality gate.
        assert!(
            visibility_identical,
            "R1 {kind} visibility/acceptance changed"
        );
        assert!(
            max_radiance_error <= 1.788_139_3e-7,
            "R1 {kind} radiance error {max_radiance_error}"
        );
    }
    fs::write(
        out.join("r1-equivalence.json"),
        serde_json::to_vec_pretty(&comparisons).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "owned GPU lease; opaque/thin backlighting on raster and traced shadow paths"]
fn gpu_receiver_material_thin() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let program = r
        .try_create_program(&crate::ProgramDescriptor {
            label: "receiver visibility",
            source: &crate::program::standard_returning(VISIBILITY),
        })
        .unwrap();
    let materials = [false, true].map(|thin| {
        r.try_create_material(&MaterialDescriptor {
            double_sided: true,
            thin,
            program: Some(&program),
            ..Default::default()
        })
        .unwrap()
        .unwrap()
    });
    r.test_drain("thin materials");
    let vertices =
        [[-2., -2., 0.], [2., -2., 0.], [2., 2., 0.], [-2., 2., 0.]].map(|position| Vertex {
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
    r.test_drain("thin mesh");
    let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("thin instances");
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )
    .unwrap();
    let color = target(&r, 64, 48);
    let mut results = vec![];
    for primary in [PrimaryVisibility::Raster, PrimaryVisibility::Traced] {
        r.set_primary_visibility(primary).unwrap();
        for (mode, disk, soft) in [
            ("hard", 0., crate::SoftShadows::Exact),
            ("exact", 0.04, crate::SoftShadows::Exact),
            ("interactive", 0.04, crate::SoftShadows::Interactive),
        ] {
            r.set_soft_shadows(soft);
            r.set_lighting_and_shadows(
                &Lighting {
                    lights: vec![Light::Directional {
                        travel: Vec3::new(-1., 0., 1.),
                        lux: [1000.; 3],
                    }],
                    ..Default::default()
                },
                &Shadows {
                    directional_disk_angle: disk,
                    requests: vec![ShadowRequest::Directional {
                        split_ends: vec![10., 40.],
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
            for (thin, material) in materials.iter().enumerate() {
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut [Draw::new(&mesh, &mut instances).with_material(material)],
                );
                let hdr = capture(&mut r);
                let middle = (24 * 64 + 32) * 8;
                let visibility = decode_f16(u16::from_le_bytes(
                    hdr[middle..middle + 2].try_into().unwrap(),
                ));
                assert_eq!(visibility, thin as f32, "{primary:?} {mode} thin={thin}");
                results.push(serde_json::json!({"primary":format!("{primary:?}"),"mode":mode,"thin":thin!=0,"visibility":visibility}));
            }
        }
    }
    let out = PathBuf::from(manifest()["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("thin-material.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "owned GPU lease; in-process paired old/new complete traced frames"]
fn gpu_receiver_policy_paired_timing() {
    let m = manifest();
    let validate = m["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    receiver_paired_with(&m, &mut r, &mut asset, &cameras, &color);
}

pub(super) fn receiver_paired_with(
    m: &serde_json::Value,
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    cameras: &cameras::Cameras,
    color: &wgpu::Texture,
) {
    let validate = m["validate"].as_bool().unwrap_or(false);
    let frozen = PathBuf::from(m["frozen_modules"].as_str().unwrap());
    let read = |names: [&str; 2]| {
        names.map(|n| fs::read_to_string(frozen.join(format!("{n}.wgsl"))).unwrap())
    };
    let mut old = r.primary.as_ref().unwrap().compile(
        &r.device,
        &r.ledger,
        &read(["kernels-hard", "kernels-soft"]),
    );
    let mut old_shade = shading_pipelines(r, &read(["traced-hard", "traced-soft"]));
    if let Some(directory) = m["candidate_modules"].as_str() {
        let directory = PathBuf::from(directory);
        let read = |names: [&str; 2]| {
            names.map(|n| fs::read_to_string(directory.join(format!("{n}.wgsl"))).unwrap())
        };
        r.pipeline.primary = Some(r.primary.as_ref().unwrap().compile(
            &r.device,
            &r.ledger,
            &read(["kernels-hard", "kernels-soft"]),
        ));
        r.test_standard().traced =
            shading_pipelines(r, &read(["traced-hard", "traced-soft"])).map(Some);
    }
    let mut separate = m["separate_interactive"].as_str().map(|path| {
        let source = fs::read_to_string(path).unwrap();
        let shader = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("separate eight-ray screen"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        r.primary.as_ref().unwrap().fresh_pipeline(
            &r.device,
            &r.ledger,
            &shader,
            "fresh_compute",
            &[],
        )
    });
    let warmup = m["warmup_pairs"].as_u64().unwrap_or(24) as usize;
    let pairs = m["pairs"].as_u64().unwrap_or(48) as usize;
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.;
    let mut pending = std::collections::HashMap::new();
    let mut samples = vec![];
    let mut edge_counts = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<u64, (usize, usize, bool)>| {
            for g in r.poll().unwrap().gpu {
                if let Some((pair, mode, baseline)) = pending.remove(&g.frame.0) {
                    samples.push(serde_json::json!({"pair":pair,"mode":(["hard","exact","interactive"][mode]),"baseline":baseline,
                    "host_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64(),
                    "receiver_ms":g.receiver_pass.map(ms),"compute_ms":g.visibility_compute.map(ms),"shading_ms":ms(g.mesh_pass),
                    "tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
                }
            }
        };
    for pair in 0..warmup + pairs {
        // Each mode gets an old/new pair, with mode and order rotating together.
        for k in 0..3 {
            let mode = (pair + k) % 3;
            let mut identity = None;
            for j in 0..2 {
                let baseline = (pair + j) % 2 == 0;
                r.set_shadows(if mode == 0 { &hard } else { &soft })
                    .unwrap();
                r.set_soft_shadows(if mode == 2 {
                    crate::SoftShadows::Interactive
                } else {
                    crate::SoftShadows::Exact
                });
                let split = mode == 2 && !baseline && separate.is_some();
                r.primary.as_mut().unwrap().test_fresh_tile = if split {
                    m["separate_tile"]
                        .as_array()
                        .map(|t| [0, 1].map(|i| t[i].as_u64().unwrap() as u32))
                } else {
                    None
                };
                if split {
                    r.set_soft_shadows(crate::SoftShadows::Exact);
                    std::mem::swap(
                        &mut r.pipeline.primary.as_mut().unwrap().fresh,
                        separate.as_mut().unwrap(),
                    );
                }
                if baseline {
                    std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
                    r.test_standard().swap_traced(&mut old_shade);
                }
                let report = traced_frame(r, color, &poses[pair % 2], asset, true);
                if pair < 2
                    && mode == 2
                    && !baseline
                    && r.pipeline
                        .primary
                        .as_ref()
                        .unwrap()
                        .interactive_repair
                        .is_some()
                {
                    let buffer = (*r
                        .primary
                        .as_ref()
                        .unwrap()
                        .targets
                        .as_ref()
                        .unwrap()
                        .repairs
                        .value)
                        .clone();
                    let bytes = read_buffer(r, &buffer, 16);
                    let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
                    edge_counts.push(serde_json::json!({"pose":pair%2,"near_edge_pixels":count,"image_pixels":1920*1080}));
                }
                if split {
                    std::mem::swap(
                        &mut r.pipeline.primary.as_mut().unwrap().fresh,
                        separate.as_mut().unwrap(),
                    );
                }
                // Two complete poses before timing; retained candidates must keep
                // hit records, receiver exports, counts and HDR exactly unchanged.
                if m["require_identity"].as_bool().unwrap_or(false) && pair < 2 {
                    let mut actual = vec![hit_record(r), capture(r)];
                    if mode != 0 {
                        actual.extend(receivers(r));
                    }
                    if let Some(expected) = identity.take() {
                        assert!(
                            actual == expected,
                            "candidate byte identity pose={pair} mode={mode}; differing buffers {:?}",
                            actual
                                .iter()
                                .zip(&expected)
                                .enumerate()
                                .filter_map(|(i, (a, b))| (a != b).then_some((
                                    i,
                                    a.iter().zip(b).filter(|(a, b)| a != b).count()
                                )))
                                .collect::<Vec<_>>()
                        );
                    } else {
                        identity = Some(actual);
                    }
                }
                if baseline {
                    std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
                    r.test_standard().swap_traced(&mut old_shade);
                }
                if pair >= warmup {
                    pending.insert(report.frame.0, (pair - warmup, mode, baseline));
                }
                collect(r, &mut pending);
            }
        }
    }
    r.test_drain("receiver policy paired frames");
    collect(r, &mut pending);
    assert!(pending.is_empty());
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("paired-timing.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"warmup_pairs":warmup,"pairs":pairs,"samples":samples,"edge_counts":edge_counts,"validation":validate,
                "identity_poses":if m["require_identity"].as_bool().unwrap_or(false) {2} else {0}}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "owned GPU lease; complete deferred Interactive repeats per pose"]
fn gpu_receiver_deferred_repeats() {
    let m = manifest();
    let repeats = m["repeats"].as_u64().unwrap_or(8);
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    let mut rows = vec![];
    let mut return_reference = None;
    for (pose, frame) in [0, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[frame].camera().unwrap();
        let mut first = None;
        for repeat in 0..repeats {
            traced_frame(&mut r, &color, &camera, &mut asset, true);
            let hdr = capture(&mut r);
            let [hit, counts] = interactive_outputs(&r);
            let actual = [hdr, hit, counts];
            if let Some(expected) = &first {
                assert_eq!(&actual, expected, "deferred repeat {pose}/{repeat}");
            } else {
                first = Some(actual);
            }
        }
        let first = first.unwrap();
        if pose == 0 {
            return_reference = Some(first.clone());
        }
        if pose == 2 {
            assert_eq!(
                Some(&first),
                return_reference.as_ref(),
                "deferred return pose"
            );
        }
        let buffer = (*r
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .repairs
            .value)
            .clone();
        let counter = read_buffer(&mut r, &buffer, 16);
        let near = u32::from_le_bytes(counter[12..16].try_into().unwrap());
        let covered = first[1]
            .chunks_exact(16)
            .filter(|h| h[..4] != [0; 4])
            .count();
        let hashes: Vec<String> = first
            .iter()
            .map(|b| {
                Sha256::digest(b)
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect()
            })
            .collect();
        rows.push(serde_json::json!({"pose":pose,"repeats":repeats,"byte_identical":true,"near_edge_pixels":near,"covered_pixels":covered,"sha256_hdr_hit_counts":hashes}));
    }
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("deferred-repeats.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "owned GPU lease; same-frame hot, barrier and cold timestamps"]
fn gpu_receiver_pass_timing() {
    let m = manifest();
    let validate = m["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    let pose = m["pose"].as_u64().unwrap_or(0) as usize;
    let camera = cameras.frames[pose].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    r.test_drain("pass-timing setup");
    receiver_pass_with(&m, &mut r, &mut asset, &cameras, &color);
}

pub(super) fn receiver_pass_with(
    m: &serde_json::Value,
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    cameras: &cameras::Cameras,
    color: &wgpu::Texture,
) {
    let validate = m["validate"].as_bool().unwrap_or(false);
    let pose = m["pose"].as_u64().unwrap_or(0) as usize;
    let camera = cameras.frames[pose].camera().unwrap();
    traced_frame(r, color, &camera, asset, true);
    r.test_drain("batched stage timestamps");
    let compile = |path: &str| {
        let path = PathBuf::from(path);
        let sources = ["kernels-hard", "kernels-soft"]
            .map(|name| fs::read_to_string(path.join(format!("{name}.wgsl"))).unwrap());
        r.primary
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, &sources)
    };
    let mut old = compile(m["frozen_modules"].as_str().unwrap());
    if let Some(path) = m["candidate_modules"].as_str() {
        r.pipeline.primary = Some(compile(path));
    }
    r.primary
        .as_mut()
        .unwrap()
        .test_split_interactive_timestamps = true;
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("Interactive hot and cold"),
        ty: wgpu::QueryType::Timestamp,
        count: 6,
    });
    let resolved = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Interactive pass stamps"),
        size: 48,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let warmup = m["warmup_pairs"].as_u64().unwrap_or(8) as usize;
    let pairs = m["pairs"].as_u64().unwrap_or(12) as usize;
    let period_ms = f64::from(r.queue.get_timestamp_period()) * 1e-6;
    let mut samples = vec![];
    let mut raw_hits = None;
    let mut changed_hits = 0;
    let mut near_edges = 0;
    let mut retry_pixels = 0;
    let mut retry_fallback_pixels = 0;
    let mut fallback_pixels = 0;
    let mut fallback_queries = 0u64;
    for pair in 0..warmup + pairs {
        for j in 0..2 {
            let baseline = (pair + j) % 2 == 0;
            if baseline {
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
            }
            let deferred = r
                .pipeline
                .primary
                .as_ref()
                .unwrap()
                .interactive_repair
                .is_some();
            let mut encoder = r.device.create_command_encoder(&Default::default());
            r.encode_primary_interactive(
                &mut encoder,
                &r.pipeline.primary.as_ref().unwrap().interactive,
                primary_ray_gpu::TRACE_TILE,
                Some(wgpu::ComputePassTimestampWrites {
                    query_set: &queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            );
            encoder.resolve_query_set(&queries, 0..if deferred { 6 } else { 2 }, &resolved, 0);
            r.queue.submit([encoder.finish()]);
            let bytes = read_buffer(r, &resolved, 48);
            let t: Vec<_> = bytes
                .chunks_exact(8)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
                .collect();
            assert!(t[1] >= t[0]);
            if deferred {
                assert!(t[5] >= t[4] && t[4] >= t[3] && t[3] >= t[2] && t[2] >= t[1]);
            }
            if pair == 0 {
                let hits = hit_record(r);
                if baseline {
                    raw_hits = Some(hits);
                } else {
                    changed_hits = hits
                        .chunks_exact(16)
                        .zip(raw_hits.as_ref().unwrap().chunks_exact(16))
                        .filter(|(a, b)| a != b)
                        .count();
                    if deferred {
                        let queue = (*r
                            .primary
                            .as_ref()
                            .unwrap()
                            .targets
                            .as_ref()
                            .unwrap()
                            .repairs
                            .value)
                            .clone();
                        let header = read_buffer(r, &queue, 16);
                        near_edges = u32::from_le_bytes(header[12..16].try_into().unwrap());
                        let retries = (*r
                            .primary
                            .as_ref()
                            .unwrap()
                            .targets
                            .as_ref()
                            .unwrap()
                            .retries
                            .value)
                            .clone();
                        let retry_header = read_buffer(r, &retries, 16);
                        retry_pixels = u32::from_le_bytes(retry_header[12..16].try_into().unwrap());
                        if retry_pixels != 0 {
                            let list = read_buffer(r, &retries, 16 + u64::from(retry_pixels) * 4);
                            let records = read_buffer(r, &queue, 16 + u64::from(near_edges) * 36);
                            for bytes in list[16..].chunks_exact(4) {
                                let i = u32::from_le_bytes(bytes.try_into().unwrap()) as usize;
                                let base = 16 + i * 36 + 32;
                                retry_fallback_pixels += usize::from(
                                    u32::from_le_bytes(records[base..base + 4].try_into().unwrap())
                                        != 0,
                                );
                            }
                        }

                        if m["probe_stats"].as_bool().unwrap_or(false) {
                            let counts = interactive_outputs(r)[1].clone();
                            for bytes in counts.chunks_exact(4) {
                                let value = u32::from_le_bytes(bytes.try_into().unwrap());
                                if value >= 65536 {
                                    let n = value - 65536;
                                    fallback_pixels += usize::from(n != 0);
                                    fallback_queries += u64::from(n);
                                }
                            }
                        }
                    }
                }
            }
            if pair >= warmup {
                samples.push(serde_json::json!({"pair":pair-warmup,"baseline":baseline,
                    "hot_ms":(t[1]-t[0]) as f64*period_ms,
                    "gap_ms":if deferred {(t[2]-t[1]) as f64*period_ms} else {0.},
                    "check_ms":if deferred {(t[3]-t[2]) as f64*period_ms} else {0.},
                    "retry_ms":if deferred {(t[5]-t[4]) as f64*period_ms} else {0.},
                    "gap2_ms":if deferred {(t[4]-t[3]) as f64*period_ms} else {0.},
                    "cold_ms":if deferred {(t[3]-t[2]+t[5]-t[4]) as f64*period_ms} else {0.},
                    "total_ms":(t[if deferred {5} else {1}]-t[0]) as f64*period_ms,
                    "host_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()}));
            }
            if baseline {
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
            }
        }
    }
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("kernel-screen.json"),serde_json::to_vec_pretty(&serde_json::json!({"samples":samples,"validation":validate,"warmup_pairs":warmup,"pairs":pairs,"pose":pose,"near_edge_pixels":near_edges,"changed_hit_records":changed_hits,"retry_pixels":retry_pixels,"retry_fallback_pixels":retry_fallback_pixels,"fallback_pixels":fallback_pixels,"fallback_queries":fallback_queries,"probe_stats":m["probe_stats"].as_bool().unwrap_or(false)})).unwrap()).unwrap();
    r.primary
        .as_mut()
        .unwrap()
        .test_split_interactive_timestamps = false;
}

#[test]
#[ignore = "owned GPU lease; old/new hard, Exact and Interactive HDR and visibility"]
fn gpu_receiver_policy_centres() {
    let m = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let frozen = PathBuf::from(m["frozen_modules"].as_str().unwrap());
    let read = |names: [&str; 2]| {
        names.map(|n| fs::read_to_string(frozen.join(format!("{n}.wgsl"))).unwrap())
    };
    let mut old = r.primary.as_ref().unwrap().compile(
        &r.device,
        &r.ledger,
        &read(["kernels-hard", "kernels-soft"]),
    );
    let old_sources = read(["traced-hard", "traced-soft"]);
    let mut old_shade = shading_pipelines(&r, &old_sources);
    let mut visibility = [
        old_sources,
        primary_ray_gpu::shading_sources(SOURCE, STANDARD).unwrap(),
    ]
    .map(|sources| {
        shading_pipelines(
            &r,
            &sources.map(|s| edit(&s, ("return exposed;", &format!("return {VISIBILITY};")))),
        )
    });
    let mut sun = [
        read(["traced-hard", "traced-soft"]),
        primary_ray_gpu::shading_sources(SOURCE, STANDARD).unwrap(),
    ]
    .map(|sources| {
        shading_pipelines(
            &r,
            &sources.map(|s| {
                let source = edit(
                    &s,
                    (
                        "    for(var i=0u;i<light_count();i+=1u) {",
                        "    exposed=vec3<f32>(0.0);\n    for(var i=0u;i<light_count();i+=1u) {",
                    ),
                );
                assert_eq!(source.matches("primary_frame.clear").count(), 3);
                source.replace("primary_frame.clear", "vec4<f32>(0.0,0.0,0.0,1.0)")
            }),
        )
    });
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.;
    for (pose, index) in [0, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[index].camera().unwrap();
        for baseline in [true, false] {
            if baseline {
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
                r.test_standard().swap_traced(&mut old_shade);
            }
            for (mode, name) in ["hard", "exact", "interactive"].into_iter().enumerate() {
                r.set_shadows(if mode == 0 { &hard } else { &soft })
                    .unwrap();
                r.set_soft_shadows(if mode == 2 {
                    crate::SoftShadows::Interactive
                } else {
                    crate::SoftShadows::Exact
                });
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                let prefix = format!(
                    "pose-{pose}-{}-{name}",
                    if baseline { "old" } else { "new" }
                );
                fs::write(out.join(format!("{prefix}.rgba16f")), capture(&mut r)).unwrap();
                let v = &mut visibility[usize::from(!baseline)];
                r.test_standard().swap_traced(v);
                let s = &mut sun[usize::from(!baseline)];
                r.test_standard().swap_traced(s);
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                fs::write(out.join(format!("{prefix}.sun.rgba16f")), capture(&mut r)).unwrap();
                r.test_standard().swap_traced(s);
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                fs::write(
                    out.join(format!("{prefix}-visibility.rgba16f")),
                    capture(&mut r),
                )
                .unwrap();
                r.test_standard().swap_traced(v);
            }
            if baseline {
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
                r.test_standard().swap_traced(&mut old_shade);
            }
        }
    }
}

/// An R1 analytic fixture (flat, convex or sawtooth) uploaded with its lighting set, and
/// the frame it is integrated at: mesh, instances, camera and geometry record.
pub(super) fn analytic_scene(
    r: &mut Renderer,
    m: &serde_json::Value,
) -> (Mesh, Instances, Camera, serde_json::Value) {
    let workload = m["workload"].as_str().unwrap();
    let distance = m["distance"].as_f64().unwrap_or(0.0) as f32;
    let translation = Vec3::splat(distance);
    let mut profile = vec![];
    if workload == "sawtooth" {
        let mut x = -0.1008f32;
        profile.push([x, 0.]);
        for dx in [0.0018, 0.0036, 0.0072] {
            for _ in 0..8 {
                x += dx;
                profile.push([x, dx * 0.45]);
                x += dx;
                profile.push([x, 0.]);
            }
        }
    } else if workload == "contact" {
        profile = vec![
            [-0.08, 0.],
            [-0.006, 0.],
            [-0.006, 0.004],
            [0.006, 0.004],
            [0.006, 0.],
            [0.08, 0.],
        ];
    } else {
        profile = vec![
            [-2., if workload == "convex" { -0.004 } else { 0. }],
            [0., 0.],
            [2., if workload == "convex" { -0.004 } else { 0. }],
        ];
    }
    let mut vertices = vec![];
    let mut indices = vec![];
    for pair in profile.windows(2) {
        let [[x0, z0], [x1, z1]] = [pair[0], pair[1]];
        let normal = Vec3::new(z0 - z1, 0., x1 - x0).normalize().to_array();
        let base = vertices.len() as u32;
        for position in [[x0, -8., z0], [x1, -8., z1], [x1, 8., z1], [x0, 8., z0]] {
            vertices.push(Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })
        .unwrap()
        .unwrap();
    r.test_drain("R1 fixture mesh");
    let instances = r
        .try_upload_instances(&[Mat4::from_translation(translation)])
        .unwrap()
        .unwrap();
    r.test_drain("R1 fixture instances");
    let incidence = m["incidence"].as_f64().unwrap_or(85.) as f32;
    let theta = incidence.to_radians();
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-theta.sin(), 0., -theta.cos()),
                lux: [1000.; 3],
            }],
            exposure_stops: -8.,
            ..Default::default()
        },
        &Shadows {
            directional_disk_angle: if workload == "sawtooth" { 0.04 } else { 0.001 },
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let normal_distance = if workload == "sawtooth" {
        0.15
    } else if workload == "contact" {
        0.1
    } else {
        4.
    };
    let view_angle = m["view_incidence"].as_f64().unwrap_or(0.) as f32;
    let view = view_angle.to_radians();
    let underside = m["underside"].as_bool().unwrap_or(false);
    let eye = if view_angle > 0. && !underside {
        (if workload == "sawtooth" { 0.06 } else { 0.02 }) / view.cos()
    } else {
        normal_distance
    };
    // Preserve the world-space pixel footprint while keeping front-side grazing
    // views above the groove planes. The below-peak view remains a control.
    let fov = 2. * ((55f32.to_radians() * 0.5).tan() * normal_distance / eye).atan();
    let view_target = if workload == "contact" {
        Vec3::new(-0.009, 0., 0.)
    } else {
        Vec3::ZERO
    };
    let offset = Vec3::new(0., view.sin(), view.cos());
    let camera = Camera::look_at(
        translation + view_target + offset * eye,
        translation + view_target,
        Vec3::Y,
        fov,
        0.01,
        100.,
    )
    .unwrap();
    let geometry = serde_json::json!({"profile":profile,"translation":translation.to_array(),"incidence":incidence,"view_incidence":view_angle,"underside":underside,"disk_angle":if workload=="sawtooth" {0.04} else {0.001},"workload":workload});
    (mesh, instances, camera, geometry)
}

pub(super) fn analytic_fixture(r: &mut Renderer, m: &serde_json::Value, policy: Policy) {
    let (mesh, mut instances, camera, geometry) = analytic_scene(r, m);
    let color = target(r, 256, 192);
    render(r, &color, &camera, &mut [Draw::new(&mesh, &mut instances)]);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    integrate_with(r, &camera, &out, 0, true, Some(policy));
    fs::write(
        out.join("geometry.json"),
        serde_json::to_vec_pretty(&geometry).unwrap(),
    )
    .unwrap();
}

/// Rectangle controls share a renderer and never load the quarry asset.
#[test]
#[ignore = "owned GPU lease; receiver thin and stress controls"]
fn gpu_receiver_rectangle_fixtures() {
    let m = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    for kind in ["thin", "stress"] {
        let mut case = m.clone();
        case["workload"] = serde_json::json!(kind);
        case["minimal"] = serde_json::json!(false);
        case["output"] = serde_json::json!(
            PathBuf::from(m["output"].as_str().unwrap()).join(format!("{kind}-viewed"))
        );
        super::rectangle_fixture(&mut r, &case, Some(Policy::Production));
        r.test_drain("receiver rectangle fixture");
        r.collect().unwrap();
    }
}
