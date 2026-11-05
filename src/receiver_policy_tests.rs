//! Production receiver-policy references and independent analytic fixtures.
use super::*;

#[path = "receiver_diagnostic_tests.rs"]
mod receiver_diagnostic_tests;

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
    let _validation = validation::begin(false);
    let mut r = renderer(false);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let frozen = PathBuf::from(m["frozen_modules"].as_str().unwrap());
    let read = |names: [&str; 2]| {
        names.map(|n| fs::read_to_string(frozen.join(format!("{n}.wgsl"))).unwrap())
    };
    let mut old = r.primary.as_ref().unwrap().compile(
        &r.device,
        &r.ledger,
        &read(["kernels-hard", "kernels-soft"]),
    );
    let mut old_shade = shading_pipelines(&r, &read(["traced-hard", "traced-soft"]));
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
        *r.test_standard().traced.as_mut().unwrap() =
            shading_pipelines(&r, &read(["traced-hard", "traced-soft"]));
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
                    std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut old_shade);
                }
                let report = traced_frame(&mut r, &color, &poses[pair % 2], &mut asset, true);
                if split {
                    std::mem::swap(
                        &mut r.pipeline.primary.as_mut().unwrap().fresh,
                        separate.as_mut().unwrap(),
                    );
                }
                // Two complete poses before timing; retained candidates must keep
                // hit records, receiver exports, counts and HDR exactly unchanged.
                if m["require_identity"].as_bool().unwrap_or(false) && pair < 2 {
                    let mut actual = vec![hit_record(&r), capture(&mut r)];
                    if mode != 0 {
                        actual.extend(receivers(&r));
                    }
                    if let Some(expected) = identity.take() {
                        assert!(
                            actual == expected,
                            "candidate byte identity pose={pair} mode={mode}"
                        );
                    } else {
                        identity = Some(actual);
                    }
                }
                if baseline {
                    std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
                    std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut old_shade);
                }
                if pair >= warmup {
                    pending.insert(report.frame.0, (pair - warmup, mode, baseline));
                }
                collect(&mut r, &mut pending);
            }
        }
    }
    r.test_drain("receiver policy paired frames");
    collect(&mut r, &mut pending);
    assert!(pending.is_empty());
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("paired-timing.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"warmup_pairs":warmup,"pairs":pairs,"samples":samples,
                "identity_poses":if m["require_identity"].as_bool().unwrap_or(false) {2} else {0}}),
        )
        .unwrap(),
    )
    .unwrap();
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
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut old_shade);
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
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), v);
                let s = &mut sun[usize::from(!baseline)];
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), s);
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                fs::write(out.join(format!("{prefix}.sun.rgba16f")), capture(&mut r)).unwrap();
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), s);
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                fs::write(
                    out.join(format!("{prefix}-visibility.rgba16f")),
                    capture(&mut r),
                )
                .unwrap();
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), v);
            }
            if baseline {
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut old);
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut old_shade);
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
    let eye = if workload == "sawtooth" { 0.15 } else { 4. };
    let camera = Camera::look_at(
        translation + Vec3::Z * eye,
        translation,
        Vec3::Y,
        55f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let geometry = serde_json::json!({"profile":profile,"translation":translation.to_array(),"incidence":incidence,"disk_angle":if workload=="sawtooth" {0.04} else {0.001},"workload":workload});
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
