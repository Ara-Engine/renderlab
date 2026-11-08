//! Private full-detail quarry reference. The manifest names prepared inputs and registered
//! cameras; every output records content identities and its exact shutter endpoints.
use super::*;
use std::path::Path;

#[test]
#[ignore = "GPU; RENDERLAB_M7_QUARRY_MANIFEST, explicit experimental RT acknowledgement"]
fn gpu_reference_quarry() {
    use sha2::Digest;
    let manifest_path = std::env::var("RENDERLAB_M7_QUARRY_MANIFEST").unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let read = |key: &str| std::fs::read(manifest[key].as_str().unwrap()).unwrap();
    let asset_bytes = read("asset");
    let asset_sha = format!("{:x}", sha2::Sha256::digest(&asset_bytes));
    let asset =
        renderlab_asset::decode(&asset_bytes, renderlab_asset::Limits::unlimited()).unwrap();
    drop(asset_bytes);
    let mut scene = Scene::from_prepared(asset);
    // An explicit `"camera_background": null` leaves primary misses on the environment (the
    // visible-sky walk, docs/m7-tr1b.md); a missing key or an array keeps a constant backdrop
    // (black when missing), exactly as before.
    scene.camera_background = match manifest.get("camera_background") {
        Some(serde_json::Value::Null) => None,
        value => Some(
            value
                .and_then(serde_json::Value::as_array)
                .map_or([0.0; 3], |v| {
                    std::array::from_fn(|i| v[i].as_f64().unwrap() as f32)
                }),
        ),
    };
    let env_bytes = read("environment");
    let environment_sha = format!("{:x}", sha2::Sha256::digest(&env_bytes));
    let env = renderlab_asset::environment::decode(
        &env_bytes,
        renderlab_asset::environment::Limits::unbounded(),
    )
    .unwrap();
    scene.environment = Some(Environment {
        radiance: env.specular_radiance()[0].clone(),
        yaw: env.metadata().rotation_y_radians,
        intensity: 1.0,
    });
    let lighting: serde_json::Value = serde_json::from_slice(&read("lighting")).unwrap();
    let v3 = |v: &serde_json::Value| {
        Vec3::new(
            v[0].as_f64().unwrap() as f32,
            v[1].as_f64().unwrap() as f32,
            v[2].as_f64().unwrap() as f32,
        )
    };
    scene.sun = Some(Sun {
        to_light: -v3(&lighting["lights"][0]["travel"]),
        illuminance: v3(&lighting["lights"][0]["lux"]).to_array(),
        angular_radius: 0.04,
    });
    let (device, queue) = super::tests::gpu();
    let dir = super::tests::output();
    let info = serde_json::json!({"asset_sha256":asset_sha,"environment_sha256":environment_sha,
        "triangles":scene.meshes.iter().map(|m| m.indices.len()/3).sum::<usize>(),
        "textures":scene.textures.iter().map(|t| serde_json::json!({"role":format!("{:?}",t.role),"size":[t.mips[0].width,t.mips[0].height],"mips":t.mips.len()})).collect::<Vec<_>>(),
        "normal_mapped_materials":scene.materials.iter().filter(|m| m.normal_texture.is_some()).count(),
        "environment_metadata":format!("{:?}",env.metadata())});
    std::fs::write(
        dir.join("content.json"),
        serde_json::to_vec_pretty(&info).unwrap(),
    )
    .unwrap();
    let batch_limit =
        std::env::var("RENDERLAB_M7_REFERENCE_BATCH").map_or(u32::MAX, |v| v.parse().unwrap());
    let mut rendered = 0;
    let frame_limit = std::env::var("RENDERLAB_M7_REFERENCE_MAX_FRAME_SECONDS")
        .map_or(f64::INFINITY, |v| v.parse().unwrap());
    for frame in manifest["frames"].as_array().unwrap() {
        if rendered >= batch_limit {
            break;
        }
        let name = frame["name"].as_str().unwrap();
        if let Some(alias) = frame["alias_of"].as_str() {
            assert!(
                dir.join(format!("{alias}.npz")).exists(),
                "alias source not rendered: {alias}"
            );
            std::fs::write(
                dir.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({"frame":frame,"alias_of":alias}))
                    .unwrap(),
            )
            .unwrap();
            continue;
        }
        if Path::new(&dir.join(format!("{name}.npz"))).exists() {
            continue;
        }
        let pose = |key: &str| Pose {
            eye: v3(&frame[key]["eye"]),
            target: v3(&frame[key]["target"]),
        };
        let view = View {
            open: pose("open"),
            close: pose("close"),
            up: Vec3::Y,
            fov_y: frame["fov_y"].as_f64().unwrap() as f32,
            width: frame["width"].as_u64().unwrap() as u32,
            height: frame["height"].as_u64().unwrap() as u32,
        };
        let settings = Settings {
            samples: frame["samples"].as_u64().unwrap() as u32,
            samples_per_dispatch: 1,
            capture_geometry: true,
            ..Default::default()
        };
        let sun = scene.sun;
        if frame["sky_only"].as_bool() == Some(true) {
            scene.sun = None;
        }
        let mode = std::env::var("RENDERLAB_M7_QUARRY_REPLAY_MODE")
            .unwrap_or_else(|_| "independent".into());
        let repeats = std::env::var("RENDERLAB_M7_QUARRY_REPLAY_COUNT")
            .map_or(2, |v| v.parse::<usize>().unwrap());
        assert!((2..=8).contains(&repeats));
        let start = std::time::Instant::now();
        let images = match mode.as_str() {
            "independent" => (0..repeats)
                .map(|_| render(&device, &queue, &scene, &view, &settings).unwrap())
                .collect::<Vec<_>>(),
            "fixed" | "rebuild-pipeline" => render_repeats(
                &device,
                &queue,
                &scene,
                &view,
                &settings,
                repeats,
                mode == "rebuild-pipeline",
            )
            .unwrap(),
            _ => panic!("unknown replay diagnostic mode"),
        };
        let image = &images[0];
        let render_seconds = start.elapsed().as_secs_f64() / repeats as f64;
        let mut replay_reports = images[1..]
            .iter()
            .map(|replay| super::tests::replay_report(image, replay))
            .collect::<Vec<_>>();
        let identical = replay_reports.iter().all(|r| r["bit_identical"] == true);
        let policy =
            std::env::var("RENDERLAB_M7_QUARRY_REPLAY_POLICY").unwrap_or_else(|_| "strict".into());
        assert!(matches!(policy.as_str(), "strict" | "bounded-as-v1"));
        assert!(
            mode == "independent" || policy == "strict",
            "diagnostic modes keep the strict gate"
        );
        // Persist comparisons before optional GPU controls or large image writes.
        // A later device/guard failure must not erase an already available report;
        // unique directories also preserve rejected attempts across retries.
        let replay_dir = dir.join("replay-reports").join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&replay_dir).unwrap();
        let retain = |stage: &str, report: serde_json::Value| {
            std::fs::write(
                replay_dir.join(format!("{name}-{stage}.json")),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
        };
        retain(
            "base",
            serde_json::json!({"policy":policy,"mode":mode,"reports":replay_reports}),
        );
        let noise_rms = (image
            .standard_error
            .iter()
            .flatten()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            / (3 * image.mean.len()) as f64)
            .sqrt();
        let mut fixed_report = None;
        if policy == "bounded-as-v1"
            && !identical
            && replay_reports
                .iter()
                .all(|r| super::tests::qualify_as_replay(r, noise_rms, true))
        {
            let control =
                render_repeats(&device, &queue, &scene, &view, &settings, 2, false).unwrap();
            fixed_report = Some(super::tests::replay_report(&control[0], &control[1]));
            replay_reports.push(super::tests::replay_report(image, &control[0]));
            retain(
                "fixed",
                serde_json::json!({"fixed_resource_replay":fixed_report,"reports":replay_reports}),
            );
        }
        let fixed_exact = fixed_report
            .as_ref()
            .is_some_and(|r| r["bit_identical"] == true);
        let accepted = if policy == "strict" {
            identical
        } else {
            replay_reports
                .iter()
                .all(|r| super::tests::qualify_as_replay(r, noise_rms, fixed_exact))
        };
        let representative = replay_reports
            .iter()
            .max_by_key(|r| {
                (
                    r["bit_identical"] != true,
                    ["mean", "half0", "half1"]
                        .iter()
                        .map(|k| r[*k]["changed_pixels"].as_u64().unwrap())
                        .sum::<u64>(),
                )
            })
            .unwrap();
        let maximum = |field: &str| {
            replay_reports
                .iter()
                .flat_map(|r| {
                    // Nonfinite rejected statistics serialize as null. Keep the raw
                    // report instead of panicking before save can retain it.
                    ["mean", "half0", "half1"]
                        .map(move |k| r[k][field].as_f64().unwrap_or(f64::MAX))
                })
                .fold(0.0f64, f64::max)
        };
        let qualification = serde_json::json!({"policy":policy,"accepted":accepted,
            "label":if identical {"strict"} else if accepted {"bounded-as"} else {"rejected"},
            "same_resource_bit_identical":fixed_exact,"half_noise_rms":noise_rms,
            "max_ulp":maximum("max_ulp") as u64,"max_changed_pixels":maximum("changed_pixels") as u64,
            "max_rms_relative":maximum("rms_relative"),"max_rms_absolute":maximum("rms_absolute"),
            "limits":{"max_ulp":4,"rms_relative":1e-5,"fraction_of_half_noise":1e-3},
            "statistics_scope":"maxima across every comparison and RGB image/half; raw reports retained"});
        rendered += 1;
        super::tests::save(
            &dir,
            name,
            image,
            serde_json::json!({"frame":frame,"content":info,"seconds":render_seconds,"qualification_seconds":start.elapsed().as_secs_f64(),"seed":settings.seed,"max_depth":settings.max_depth,"visibility_samples_per_primary":settings.visibility_samples,"camera_background":scene.camera_background,
                "accepted_for_scoring":accepted,"strict_replay":representative,"replay_reports":replay_reports,
                "fixed_resource_replay":fixed_report,"replay_qualification":qualification,
                "raw_replay_reports_directory":replay_dir,
                "replay_mode":mode,"replay_count":repeats,
                "seconds_scope":"mean base-render wall time; fixed resource setup amortized; not GPU time",
                "replay_scope":"same seed and dispatch layout; mode records independent AS/pipeline, fixed resources, or fixed AS with rebuilt pipelines; every HDR-half and geometry byte"}),
        );
        assert!(accepted, "{name}: quarry replay policy {policy} rejected");
        let status = std::process::Command::new("py")
            .args([
                "-3",
                "-B",
                "scripts/m7-u0b-score.py",
                "compact",
                dir.to_str().unwrap(),
                name,
            ])
            .status()
            .unwrap();
        assert!(status.success(), "reference compaction failed: {name}");
        scene.sun = sun;
        assert_eq!(image.invalid_pixels, 0, "{name}");
        eprintln!(
            "{name}: {} spp, {:.2}s",
            settings.samples,
            start.elapsed().as_secs_f64()
        );
        if start.elapsed().as_secs_f64() > frame_limit {
            eprintln!(
                "Reference frame exceeded the game-contention limit; stopping this batch cleanly"
            );
            break;
        }
    }
}
