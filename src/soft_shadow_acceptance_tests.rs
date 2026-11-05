//! Complete frames exercise the public selector, including the reload transaction.
use super::*;

#[test]
fn soft_shadow_default_is_exact() {
    assert_eq!(crate::SoftShadows::default(), crate::SoftShadows::Exact);
}

#[test]
#[ignore = "GPU lock, 8 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
pub(super) fn gpu_soft_shadow_traced_acceptance() {
    let manifest = manifest();
    let _validation = validation::begin(true);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    let replay = PathBuf::from(manifest["replay"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(true);
    assert_eq!(r.soft_shadows(), crate::SoftShadows::Exact);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let module = seeded_module(&r);
    let mut records = vec![];
    for (index, pose) in [0, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[pose].camera().unwrap();
        let name = |kind: &str| out.join(format!("pose-{index}-{kind}"));
        r.set_soft_shadows(crate::SoftShadows::Exact);
        let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
        assert_eq!(report.soft_shadows, crate::SoftShadows::Exact);
        let exact = capture(&mut r);
        let [world, normal, counts] = receivers(&r);
        // Frozen primary acceptance is reused; this is not a new reference.
        let baseline = PathBuf::from(manifest["reference"].as_str().unwrap());
        assert!(
            counts
                == fs::read(baseline.join(format!("pose-{index}-seed-0-production.u32"))).unwrap(),
            "Exact default count regression"
        );
        fs::write(name("world.f32"), world).unwrap();
        fs::write(name("normal.f32"), normal).unwrap();
        fs::write(name("exact.rgba16f"), &exact).unwrap();
        let mut row = vec![];
        for seed in 0..4 {
            let mut p = r.primary.as_ref().unwrap().interactive_pipeline(
                &r.device,
                &r.ledger,
                &module,
                &[("FRESH_SEED", f64::from(seed)), ("FRESH_DIRECTIONS", 8.0)],
            );
            r.set_soft_shadows(crate::SoftShadows::Interactive);
            std::mem::swap(
                &mut r.pipeline.primary.as_mut().unwrap().interactive,
                &mut p,
            );
            let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
            assert_eq!(report.soft_shadows, crate::SoftShadows::Interactive);
            let image = capture(&mut r);
            let [hits, counts] = interactive_outputs(&r);
            assert!(
                counts
                    == fs::read(replay.join(format!("pose-{index}-seed-{seed}-1x8.u32"))).unwrap(),
                "public Interactive frame differs from replay"
            );
            for repeat in 0..manifest["static_repeats"].as_u64().unwrap_or(1) {
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                let repeated = capture(&mut r);
                if repeated != image {
                    super::determinism_tests::investigate(
                        &mut r, &image, &repeated, &hits, &counts, seed, &out,
                    );
                }
                assert!(
                    repeated == image,
                    "static HDR seed={seed} repeat={repeat}; see determinism.json"
                );
                assert!(
                    interactive_outputs(&r)[1] == counts,
                    "static counts seed={seed} repeat={repeat}"
                );
            }
            std::mem::swap(
                &mut r.pipeline.primary.as_mut().unwrap().interactive,
                &mut p,
            );
            fs::write(name(&format!("seed-{seed}-interactive.rgba16f")), &image).unwrap();
            fs::write(name(&format!("seed-{seed}-interactive.u32")), &counts).unwrap();
            row.push(image);
            row.push(counts);
        }
        records.push(row);
        let soft = r.shadows.settings.clone();
        let mut hard = soft.clone();
        hard.directional_disk_angle = 0.;
        r.set_shadows(&hard).unwrap();
        let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
        assert_eq!(report.soft_shadows, crate::SoftShadows::Exact);
        fs::write(name("hard.rgba16f"), capture(&mut r)).unwrap();
        r.set_shadows(&soft).unwrap();
    }
    assert!(
        records[0] == records[2],
        "return frame differs; compare pose files"
    );
    assert!(records[0] != records[1], "motion did not change output");
    let camera = cameras.frames[0].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    let accepted = capture(&mut r);
    let [_, accepted_counts] = interactive_outputs(&r);
    let tinted = edit(
        SOURCE,
        (
            "const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);",
            "const TINT: vec3<f32> = vec3<f32>(0.5, 0.5, 0.5);",
        ),
    );
    let standard = crate::STANDARD_PROGRAM;
    let unmarked = SOURCE.replace(crate::hardware_shadows::VISIBILITY_END, "");
    let syntax = edit(
        standard,
        ("let mr=sample_metallic_roughness(input);", "let mr=;"),
    );
    let implicit = edit(
        standard,
        (
            "let sampled=sample_base_color(input);",
            "let sampled=textureSample(base_color,base_sampler,input.uv);",
        ),
    );
    let program_tint = edit(standard, ("*sampled.rgb*TINT", "*sampled.rgb*TINT*0.5"));
    // Prelude reloads, then program reloads: rejections keep the image, edits change only
    // shading (counts are engine visibility), and restores are byte-identical.
    for (program, source, ok, tint) in [
        (false, "invalid WGSL", false, false),
        (false, unmarked.as_str(), false, false),
        (false, tinted.as_str(), true, true),
        (false, SOURCE, true, false),
        (true, syntax.as_str(), false, false),
        (true, implicit.as_str(), false, false),
        (true, program_tint.as_str(), true, true),
        (true, standard, true, false),
    ] {
        let result = if program {
            r.test_reload_standard(source)
        } else {
            r.reload(source)
        };
        assert_eq!(result.is_ok(), ok, "{result:?}");
        assert_eq!(r.soft_shadows(), crate::SoftShadows::Interactive);
        let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
        assert_eq!(report.soft_shadows, crate::SoftShadows::Interactive);
        assert_eq!(capture(&mut r) == accepted, !tint, "reload output");
        assert!(
            interactive_outputs(&r)[1] == accepted_counts,
            "reload changed counts"
        );
    }
    // A raster frame always uses Exact, retaining the preference for the next traced frame.
    let report = traced_frame(&mut r, &color, &camera, &mut asset, false);
    assert_eq!(report.soft_shadows, crate::SoftShadows::Exact);
    assert_eq!(r.soft_shadows(), crate::SoftShadows::Interactive);
    fs::write(out.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"validation":true,"public_api_matches_replay":true,"exact_matches_frozen":true,"repeat_exact":true,"return_exact":true,"reload":true,"raster_fallback_exact":true})).unwrap()).unwrap();
}

#[test]
#[ignore = "GPU lock, 8 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_soft_shadow_complete_paired_timing() {
    let manifest = manifest();
    let _validation = validation::begin(false);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let warmup = manifest["warmup_pairs"].as_u64().unwrap_or(24) as usize;
    let pairs = manifest["pairs"].as_u64().unwrap_or(24) as usize;
    let mut r = renderer(false);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.;
    let mut pending = std::collections::HashMap::new();
    let mut samples = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<u64, (usize, usize, usize)>| {
            for g in r.poll().unwrap().gpu {
                if let Some((pair, mode, pose)) = pending.remove(&g.frame.0) {
                    samples.push(serde_json::json!({"pair":pair,"mode":(["hard","exact","interactive"][mode]),"pose":pose,"receiver_ms":g.receiver_pass.map(ms),"compute_ms":g.visibility_compute.map(ms),"shading_ms":ms(g.mesh_pass),"tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
                }
            }
        };
    for pair in 0..warmup + pairs {
        let pose = pair % 2;
        for k in 0..3 {
            let mode = (pair + k) % 3;
            r.set_shadows(if mode == 0 { &hard } else { &soft })
                .unwrap();
            r.set_soft_shadows(if mode == 2 {
                crate::SoftShadows::Interactive
            } else {
                crate::SoftShadows::Exact
            });
            let report = traced_frame(&mut r, &color, &poses[pose], &mut asset, true);
            assert_eq!(report.soft_shadows, r.soft_shadows());
            if pair >= warmup {
                pending.insert(report.frame.0, (pair - warmup, mode, pose));
            }
            collect(&mut r, &mut pending);
        }
    }
    r.test_drain("complete paired shadow frames");
    collect(&mut r, &mut pending);
    assert!(pending.is_empty());
    fs::write(out.join("timing.json"),serde_json::to_vec_pretty(&serde_json::json!({"validation":false,"warmup_pairs":warmup,"pairs":pairs,"samples":samples})).unwrap()).unwrap();
}
