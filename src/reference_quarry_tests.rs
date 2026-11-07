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
    scene.camera_background = Some([0.0; 3]);
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
    for frame in manifest["frames"].as_array().unwrap() {
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
        let start = std::time::Instant::now();
        let image = render(&device, &queue, &scene, &view, &settings).unwrap();
        super::tests::save(
            &dir,
            name,
            &image,
            serde_json::json!({"frame":frame,"content":info,"seconds":start.elapsed().as_secs_f64(),"seed":settings.seed,"max_depth":settings.max_depth}),
        );
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
    }
}
