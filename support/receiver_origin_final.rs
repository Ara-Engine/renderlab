//! Single-process final gate, included by the isolated ad80825 T1 test harness.
use super::super::film_accumulation_tests as t1;
use super::*;

fn counts(r: &Renderer) -> Vec<u8> {
    let mut bytes = read_texture(
        r,
        &r.fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture,
    );
    for (word, missing) in bytes.chunks_exact_mut(4).zip(uncovered(r)) {
        if missing {
            word.copy_from_slice(&256u32.to_le_bytes());
        }
    }
    bytes
}
fn image_hashes(r: &mut Renderer, soft: bool) -> Vec<String> {
    let mut hashes = vec![t1::digest(&capture(r)), t1::digest(&hit_record(r))];
    if soft {
        hashes.push(t1::digest(&counts(r)));
    }
    hashes
}
fn modules(path: &std::path::Path) -> [String; 2] {
    ["kernels-hard", "kernels-soft"]
        .map(|n| fs::read_to_string(path.join(format!("{n}.wgsl"))).unwrap())
}
fn save_modules(path: &std::path::Path, kernels: &[String; 2]) {
    fs::create_dir_all(path).unwrap();
    let shading = primary_ray_gpu::shading_sources(SOURCE, STANDARD).unwrap();
    for (n, s) in ["kernels-hard", "kernels-soft", "traced-hard", "traced-soft"]
        .into_iter()
        .zip(kernels.iter().chain(shading.iter()))
    {
        fs::write(path.join(format!("{n}.wgsl")), s).unwrap();
    }
}
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "owned GPU lease; final receiver batch, one quarry upload"]
fn gpu_receiver_origin_final() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    // Small controls first; a correctness failure stops before the large asset.
    super::receiver_fold_tests::gpu_receiver_fold();
    super::receiver_fold_tests::gpu_receiver_fold_traced();
    super::gpu_receiver_material_thin();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let mut cases = vec![];
    for kind in ["flat", "convex"] {
        for distance in [1000, 10000] {
            for incidence in [80, 85, 89] {
                cases.push((
                    format!("{kind}-{distance}-{incidence}-viewed"),
                    serde_json::json!({"workload":kind,"distance":distance,"incidence":incidence}),
                ));
            }
        }
    }
    for kind in ["sawtooth", "thin", "stress"] {
        cases.push((
            format!("{kind}-viewed"),
            serde_json::json!({"workload":kind}),
        ));
    }
    for kind in ["sawtooth", "contact"] {
        for angle in [80, 85, 89] {
            cases.push((
                format!("{kind}-view-{angle}"),
                serde_json::json!({"workload":kind,"view_incidence":angle}),
            ));
        }
    }
    for (name, fields) in cases {
        let mut case = m.clone();
        for (k, v) in fields.as_object().unwrap() {
            case[k] = v.clone();
        }
        case["output"] = serde_json::json!(out.join("fixtures").join(&name));
        if matches!(case["workload"].as_str(), Some("thin" | "stress")) {
            super::super::rectangle_fixture(&mut r, &case, Some(Policy::Production));
        } else {
            super::analytic_fixture(&mut r, &case, Policy::Production);
        }
        r.test_drain("batched receiver fixture");
        r.collect().unwrap();
    }
    let (mut asset, cameras) = load_quarry(&m, &mut r); // The only quarry load.
    let color = target(&r, 1920, 1080);
    let views = t1::t1_cameras(&m);
    let modern = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let legacy = modules(std::path::Path::new(m["legacy_modules"].as_str().unwrap()));
    let original = modules(std::path::Path::new(
        m["baseline_modules"].as_str().unwrap(),
    ));
    let quality = modules(std::path::Path::new(m["quality_modules"].as_str().unwrap()));
    save_modules(&out.join("sources/modern"), &modern);
    save_modules(&out.join("sources/legacy"), &legacy);
    save_modules(&out.join("sources/original"), &original);
    let compile = |sources: &[String; 2]| {
        r.primary
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, sources)
    };
    let mut candidates = vec![
        ("modern", compile(&modern), true),
        ("legacy", compile(&legacy), true),
    ];
    r.pipeline.primary = Some(compile(&quality));
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.;
    let repeats = m["repeats"].as_u64().unwrap_or(8) as usize;
    let mut identity = vec![];
    for (pose, camera) in views.iter().enumerate() {
        for mode in 0..3 {
            r.set_shadows(if mode == 0 { &hard } else { &soft })
                .unwrap();
            r.set_soft_shadows(if mode == 2 {
                crate::SoftShadows::Interactive
            } else {
                crate::SoftShadows::Exact
            });
            traced_frame(&mut r, &color, camera, &mut asset, true);
            let expected = image_hashes(&mut r, mode != 0);
            for candidate in &mut candidates {
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut candidate.1);
                let mut hashes = vec![];
                for repeat in 0..repeats {
                    traced_frame(&mut r, &color, camera, &mut asset, true);
                    let actual = image_hashes(&mut r, mode != 0);
                    candidate.2 &= actual == expected;
                    if repeat == 0 {
                        fs::write(
                            out.join(format!(
                                "pose-{pose}-{}-{}.rgba16f",
                                candidate.0,
                                ["hard", "exact", "interactive"][mode]
                            )),
                            capture(&mut r),
                        )
                        .unwrap();
                        if mode != 0 {
                            fs::write(
                                out.join(format!(
                                    "pose-{pose}-{}-{}.u32",
                                    candidate.0,
                                    ["hard", "exact", "interactive"][mode]
                                )),
                                counts(&r),
                            )
                            .unwrap();
                        }
                    }
                    hashes.push(actual);
                }
                identity.push(serde_json::json!({"variant":candidate.0,"pose":pose,"mode":(["hard","exact","interactive"][mode]),"reference":expected,"hashes":hashes,"identity":candidate.2}));
                std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut candidate.1);
            }
        }
    }
    fs::write(
        out.join("identity.json"),
        serde_json::to_vec_pretty(&identity).unwrap(),
    )
    .unwrap();
    let mut timings = vec![];
    for candidate in &candidates {
        if !candidate.2 {
            continue;
        }
        let mut cfg = m.clone();
        cfg["output"] = serde_json::json!(out.join(format!("timing-{}", candidate.0)));
        cfg["candidate_modules"] = serde_json::json!(out.join("sources").join(candidate.0));
        cfg["frozen_modules"] = serde_json::json!(out.join("sources/original"));
        cfg["warmup_pairs"] = serde_json::json!(2);
        cfg["pairs"] = serde_json::json!(8);
        cfg["require_identity"] = serde_json::json!(false);
        super::receiver_paired_with(&cfg, &mut r, &mut asset, &cameras, &color);
        let data: serde_json::Value = serde_json::from_slice(
            &fs::read(out.join(format!("timing-{}/paired-timing.json", candidate.0))).unwrap(),
        )
        .unwrap();
        let values = data["samples"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["mode"] == "interactive" && s["baseline"] == false)
            .map(|s| s["receiver_ms"].as_f64().unwrap())
            .collect();
        timings.push((candidate.0, median(values)));
    }
    let mut stage = m.clone();
    stage["output"] = serde_json::json!(out.join("stage-modern"));
    stage["candidate_modules"] = serde_json::json!(out.join("sources/modern"));
    stage["frozen_modules"] = serde_json::json!(out.join("sources/original"));
    stage["warmup_pairs"] = serde_json::json!(2);
    stage["pairs"] = serde_json::json!(8);
    super::receiver_pass_with(&stage, &mut r, &mut asset, &cameras, &color);
    assert!(!timings.is_empty(), "no variant preserved retained quality");
    timings.sort_by(|a, b| a.1.total_cmp(&b.1));
    let selected = timings[0].0;
    let selected_sources = if selected == "modern" {
        &modern
    } else {
        &legacy
    };
    r.pipeline.primary = Some(r.primary.as_ref().unwrap().compile(
        &r.device,
        &r.ledger,
        selected_sources,
    ));
    let mut micro = m.clone();
    micro["output"] = serde_json::json!(out.join("micro"));
    t1::footprint_with(&micro, &mut r, &mut asset, &color, &views);
    let proof: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("micro/footprint.json")).unwrap()).unwrap();
    for rows in proof["poses"].as_object().unwrap().values() {
        for row in rows.as_array().unwrap() {
            assert_eq!(row["opposite_samples"], 0);
            assert_eq!(row["corner"], 0);
        }
    }
    // New references use the retained geometry policy. Dispatch selection cannot
    // change their per-sample primary/receiver arithmetic.
    let mut sun = t1::centre_shading(&r, &t1::direct_program(), true);
    let visibility_source = crate::program::standard_returning(VISIBILITY);
    let mut visibility = shading_pipelines(
        &r,
        &primary_ray_gpu::shading_sources(SOURCE, &visibility_source).unwrap(),
    );
    for (pose, camera) in views.iter().enumerate() {
        let reference = out.join("reference").join(format!("ref-p{pose}"));
        fs::create_dir_all(&reference).unwrap();
        r.set_shadows(&soft).unwrap();
        t1::integrate_t1(
            &mut r, &mut asset, &color, camera, &reference, pose, 1, false,
        );
        t1::centre_frames(&mut r, &mut asset, &color, camera, &reference, [8, 4]);
        for baseline in [true, false] {
            let sources = if baseline {
                &original
            } else {
                selected_sources
            };
            r.pipeline.primary = Some(
                r.primary
                    .as_ref()
                    .unwrap()
                    .compile(&r.device, &r.ledger, sources),
            );
            for mode in 0..3 {
                r.set_shadows(if mode == 0 { &hard } else { &soft })
                    .unwrap();
                r.set_soft_shadows(if mode == 2 {
                    crate::SoftShadows::Interactive
                } else {
                    crate::SoftShadows::Exact
                });
                let prefix = out.join(format!(
                    "pose-{pose}-{}-{}",
                    if baseline { "old" } else { "new" },
                    ["hard", "exact", "interactive"][mode]
                ));
                traced_frame(&mut r, &color, camera, &mut asset, true);
                fs::write(prefix.with_extension("rgba16f"), capture(&mut r)).unwrap();
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut sun);
                traced_frame(&mut r, &color, camera, &mut asset, true);
                fs::write(prefix.with_extension("sun.rgba16f"), capture(&mut r)).unwrap();
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut sun);
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut visibility);
                traced_frame(&mut r, &color, camera, &mut asset, true);
                fs::write(prefix.with_extension("visibility.rgba16f"), capture(&mut r)).unwrap();
                std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut visibility);
            }
        }
    }
    fs::write(out.join("final.json"),serde_json::to_vec_pretty(&serde_json::json!({"selected":selected,"timings_receiver_ms":timings,"repeats":repeats,"identity":candidates.iter().map(|c|(c.0,c.2)).collect::<Vec<_>>(),"asset_loads":1})).unwrap()).unwrap();
}
