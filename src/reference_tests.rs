//! Reference path tracer: CPU checks of its inputs and shader, and ignored GPU runs that
//! validate it analytically and render the M7 benches for the Mitsuba cross-check.
use super::*;
use std::path::{Path, PathBuf};

fn quad(half: f32, y: f32) -> Mesh {
    Mesh {
        positions: vec![
            [-half, y, -half],
            [half, y, -half],
            [half, y, half],
            [-half, y, half],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        indices: vec![0, 2, 1, 0, 3, 2],
    }
}
fn instance(mesh: usize, material: usize) -> Instance {
    Instance {
        mesh,
        material,
        transform: Mat4::IDENTITY,
    }
}
fn view(eye: Vec3, target: Vec3, size: u32) -> View {
    let pose = Pose { eye, target };
    View {
        open: pose,
        close: pose,
        up: Vec3::Y,
        fov_y: 0.5,
        width: size,
        height: size,
    }
}

#[test]
fn rejects_scenes_the_shader_would_misread() {
    let base = Scene {
        meshes: vec![quad(1.0, 0.0)],
        materials: vec![Material::default()],
        instances: vec![instance(0, 0)],
        ..Default::default()
    };
    let v = view(Vec3::new(0.0, 2.0, 2.0), Vec3::ZERO, 8);
    let s = Settings::default();
    assert!(validate(&base, &v, &s).is_ok());
    let mut bad = base.clone();
    bad.meshes[0].indices[0] = 9;
    assert!(validate(&bad, &v, &s).is_err(), "index outside the mesh");
    let mut bad = base.clone();
    bad.instances[0].material = 1;
    assert!(validate(&bad, &v, &s).is_err(), "missing material");
    let mut bad = base.clone();
    bad.instances[0].transform = Mat4::ZERO;
    assert!(validate(&bad, &v, &s).is_err(), "singular transform");
    let mut bad = base;
    bad.materials[0].roughness = 1.5;
    assert!(validate(&bad, &v, &s).is_err(), "roughness above one");
}

#[test]
fn sun_disk_radiance_reproduces_its_illuminance() {
    let r = 0.05f32;
    let scene = Scene {
        sun: Some(Sun {
            to_light: Vec3::Y,
            illuminance: [1000.0; 3],
            angular_radius: r,
        }),
        ..Default::default()
    };
    let bytes = frame_bytes(
        &scene,
        &view(Vec3::Z, Vec3::ZERO, 4),
        &Settings::default(),
        0,
        1,
    );
    assert_eq!(bytes.len(), FRAME_BYTES);
    let f = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    // sun is the 9th vec4, its radiance the 10th: integrate L cos over the cone numerically.
    assert!((f(35) - r.cos()).abs() < 1e-7);
    let radiance = f(36) as f64;
    let n = 20000;
    let rr = r as f64;
    let e: f64 = (0..n)
        .map(|k| {
            let t = (k as f64 + 0.5) / n as f64 * rr;
            radiance * t.cos() * t.sin() * 2.0 * std::f64::consts::PI * rr / n as f64
        })
        .sum();
    assert!(
        (e / 1000.0 - 1.0).abs() < 1e-4,
        "integrated illuminance {e}"
    );
}

#[test]
fn split_half_error_matches_its_definition() {
    let v = view(Vec3::Z, Vec3::ZERO, 1);
    // even sum 6 over 2 samples (mean 3), odd sum 2 over 2 samples (mean 1).
    let bytes: Vec<u8> = [6.0f32, 6.0, 6.0, 0.0, 2.0, 2.0, 2.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let image = finish(&bytes, &v, 4);
    assert_eq!(image.mean[0], [2.0; 3]);
    assert_eq!(image.standard_error[0], [1.0; 3]);
    assert_eq!(image.invalid_pixels, 0);
}

#[test]
fn shader_validates_with_ray_queries() {
    let module = naga::front::wgsl::parse_str(SHADER).expect("WGSL parses");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("WGSL validates");
}

/// A schema-2 bench (scripts/m7-u0-scenes.py) as a reference scene: one instance per mesh.
pub(super) struct Bench {
    pub scene: Scene,
    pub views: Vec<(String, View)>,
    pub smith: Smith,
    pub sha256: String,
}
pub(super) fn load_bench(path: &Path) -> Bench {
    use sha2::Digest;
    let bytes = std::fs::read(path).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let f = |x: &serde_json::Value| x.as_f64().unwrap() as f32;
    let v3 = |x: &serde_json::Value| [f(&x[0]), f(&x[1]), f(&x[2])];
    let materials = v["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| Material {
            base_color: v3(&m["base_color"]),
            metallic: f(&m["metallic"]),
            roughness: f(&m["roughness"]),
            specular: f(&m["specular"]),
            emission: v3(&m["emission"]),
        })
        .collect();
    let mut meshes = vec![];
    let mut instances = vec![];
    for (i, m) in v["meshes"].as_array().unwrap().iter().enumerate() {
        let list = |k: &str| m[k].as_array().unwrap().iter().map(v3).collect::<Vec<_>>();
        meshes.push(Mesh {
            positions: list("positions"),
            normals: list("normals"),
            indices: m["indices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u32)
                .collect(),
        });
        instances.push(instance(i, m["material"].as_u64().unwrap() as usize));
    }
    let sun = (!v["sun"].is_null()).then(|| Sun {
        to_light: Vec3::from(v3(&v["sun"]["to_light"])),
        illuminance: v3(&v["sun"]["lux"]),
        angular_radius: f(&v["sun"]["angular_radius"]),
    });
    let [width, height] = [0, 1].map(|k| v["size"][k].as_u64().unwrap() as u32);
    let fov_y = f(&v["fov_y_degrees"]).to_radians();
    let views = v["poses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let pose = Pose {
                eye: Vec3::from(v3(&p["eye"])),
                target: Vec3::from(v3(&p["target"])),
            };
            let name = p["name"].as_str().unwrap().to_string();
            (
                name,
                View {
                    open: pose,
                    close: pose,
                    up: Vec3::Y,
                    fov_y,
                    width,
                    height,
                },
            )
        })
        .collect();
    Bench {
        scene: Scene {
            meshes,
            materials,
            instances,
            sun,
            sky: v3(&v["sky"]["constant"]),
        },
        views,
        smith: if v["smith"] == "separable" {
            Smith::Separable
        } else {
            Smith::HeightCorrelated
        },
        sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
    }
}

#[test]
fn bench_loader_reads_the_generated_schema() {
    let dir = std::env::temp_dir().join(format!("renderlab-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.json");
    std::fs::write(&path, r#"{"schema":2,"size":[4,3],"fov_y_degrees":40,"smith":"separable",
        "sun":{"to_light":[0,1,0],"lux":[1,2,3],"angular_radius":0},"sky":{"constant":[4,5,6]},
        "materials":[{"base_color":[1,1,1],"metallic":1,"roughness":0.3,"specular":1,"emission":[0,0,0]}],
        "meshes":[{"name":"q","positions":[[0,0,0],[1,0,0],[0,0,1]],"normals":[[0,1,0],[0,1,0],[0,1,0]],"indices":[0,1,2],"material":0}],
        "poses":[{"name":"p","eye":[0,1,2],"target":[0,0,0]}]}"#).unwrap();
    let bench = load_bench(&path);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(bench.smith, Smith::Separable);
    assert_eq!(bench.scene.sky, [4.0, 5.0, 6.0]);
    assert_eq!(bench.scene.instances.len(), 1);
    assert_eq!((bench.views[0].1.width, bench.views[0].1.height), (4, 3));
    assert!(validate(&bench.scene, &bench.views[0].1, &Settings::default()).is_ok());
}

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"),
        "GPU tests require explicit caller acknowledgment"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let limits = wgpu::Limits {
        max_blas_primitive_count: a.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
        max_acceleration_structures_per_shader_stage: 1,
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        ..Default::default()
    };
    // SAFETY: the test caller explicitly accepts wgpu's experimental implementation risk.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

fn output() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var("RENDERLAB_M7_REFERENCE_OUT").expect("RENDERLAB_M7_REFERENCE_OUT"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save(dir: &Path, name: &str, image: &Image, meta: serde_json::Value) {
    let raw = |v: &[[f32; 3]]| {
        v.iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<u8>>()
    };
    std::fs::write(dir.join(format!("{name}-mean.f32")), raw(&image.mean)).unwrap();
    std::fs::write(
        dir.join(format!("{name}-se.f32")),
        raw(&image.standard_error),
    )
    .unwrap();
    let mut meta = meta;
    meta["width"] = image.width.into();
    meta["height"] = image.height.into();
    meta["samples"] = image.samples.into();
    meta["invalid_pixels"] = image.invalid_pixels.into();
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&meta).unwrap(),
    )
    .unwrap();
}

/// Mean and pooled standard error of the region, relative to `expected`.
fn region(image: &Image, expected: [f32; 3], pixels: impl Iterator<Item = usize>) -> (f64, f64) {
    let (mut sum, mut var, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for p in pixels {
        for (c, e) in expected.iter().enumerate() {
            sum += f64::from(image.mean[p][c] / e);
            var += f64::from(image.standard_error[p][c] / e).powi(2);
            n += 1.0;
        }
    }
    (sum / n, var.sqrt() / n)
}

/// Analytic gates: a white Lambert sphere under a constant sky has radiance equal to the sky
/// at every bounce count (furnace), and a Lambert floor under a delta sun has rho/pi E cos.
#[test]
#[ignore = "GPU; RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK and RENDERLAB_M7_REFERENCE_OUT"]
fn gpu_reference_analytic() {
    let (device, queue) = gpu();
    let dir = output();
    let mut rows = vec![];
    // Furnace: a closed emissive Lambert box (flat normals) seen from inside, no sky or sun.
    // Any closed uniform enclosure has L = Le / (1 - rho) at every point, so the converged
    // image is flat only if the integrator neither loses nor gains energy over all bounces.
    let cube = {
        let mut m = Mesh::default();
        for axis in 0..3 {
            for side in [-1.0f32, 1.0] {
                let base = m.positions.len() as u32;
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let mut p = [0.0f32; 3];
                    p[axis] = side;
                    p[u] = a;
                    p[v] = b;
                    let mut n = [0.0f32; 3];
                    n[axis] = -side;
                    m.positions.push(p);
                    m.normals.push(n);
                }
                m.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        m
    };
    let white = Material {
        base_color: [1.0; 3],
        specular: 0.0,
        ..Default::default()
    };
    let furnace = Scene {
        meshes: vec![cube],
        materials: vec![Material {
            emission: [2.0; 3],
            base_color: [0.75; 3],
            specular: 0.0,
            ..Default::default()
        }],
        instances: vec![instance(0, 0)],
        sky: [0.0; 3],
        sun: None,
    };
    // Le = 2, rho = 0.75: L = 8 everywhere.
    let v = View {
        fov_y: 1.0,
        ..view(Vec3::new(0.1, 0.05, 0.0), Vec3::new(0.0, 0.0, -1.0), 32)
    };
    let settings = Settings {
        samples: 4096,
        ..Default::default()
    };
    let image = render(&device, &queue, &furnace, &v, &settings).unwrap();
    let (mean, se) = region(&image, [8.0; 3], 0..image.mean.len());
    save(
        &dir,
        "furnace",
        &image,
        serde_json::json!({"expected": 8.0, "relative_mean": mean, "relative_se": se}),
    );
    rows.push(("furnace", mean, se, image.invalid_pixels));
    // Delta sun on a Lambert floor at 30 degrees elevation, sky black: L = rho/pi * E * sin(30).
    let floor = Scene {
        meshes: vec![quad(50.0, 0.0)],
        materials: vec![Material {
            base_color: [0.5; 3],
            ..white
        }],
        instances: vec![instance(0, 0)],
        sun: Some(Sun {
            to_light: Vec3::new(0.0, 0.5, 3f32.sqrt() / 2.0),
            illuminance: [1000.0; 3],
            angular_radius: 0.0,
        }),
        sky: [0.0; 3],
    };
    let v = view(Vec3::new(0.0, 1.0, 1.0), Vec3::new(0.0, 0.0, -0.5), 16);
    let image = render(
        &device,
        &queue,
        &floor,
        &v,
        &Settings {
            samples: 256,
            ..Default::default()
        },
    )
    .unwrap();
    let expected = 0.5 / std::f32::consts::PI * 1000.0 * 0.5;
    let (mean, se) = region(&image, [expected; 3], 0..image.mean.len());
    save(
        &dir,
        "delta-floor",
        &image,
        serde_json::json!({"expected": expected, "relative_mean": mean, "relative_se": se}),
    );
    rows.push(("delta-floor", mean, se, image.invalid_pixels));
    // Sky only on the same floor: L = rho * sky (a plane sees the whole upper hemisphere).
    let sky = Scene {
        sun: None,
        sky: [100.0; 3],
        ..floor
    };
    let image = render(
        &device,
        &queue,
        &sky,
        &v,
        &Settings {
            samples: 1024,
            ..Default::default()
        },
    )
    .unwrap();
    let (mean, se) = region(&image, [50.0; 3], 0..image.mean.len());
    save(
        &dir,
        "sky-floor",
        &image,
        serde_json::json!({"expected": 50.0, "relative_mean": mean, "relative_se": se}),
    );
    rows.push(("sky-floor", mean, se, image.invalid_pixels));
    std::fs::write(dir.join("analytic.json"), serde_json::to_vec_pretty(&serde_json::json!(rows
        .iter()
        .map(|(n, m, s, i)| serde_json::json!({"case": n, "relative_mean": m, "relative_se": s, "invalid_pixels": i}))
        .collect::<Vec<_>>())).unwrap()).unwrap();
    for (name, mean, se, invalid) in rows {
        eprintln!("{name}: relative mean {mean:.6} +- {se:.6}");
        assert_eq!(invalid, 0, "{name}: non-finite samples");
        assert!(
            (mean - 1.0).abs() < (4.0 * se).max(2e-3),
            "{name}: {mean} +- {se}"
        );
    }
}

/// Renders every bench listed in RENDERLAB_M7_BENCHES (`;`-separated directories holding scene.json,
/// each optionally `dir@spp`; default RENDERLAB_M7_SPP or 4096), plus a same-seed replay of the
/// first for bit identity.
#[test]
#[ignore = "GPU; RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK, RENDERLAB_M7_REFERENCE_OUT, RENDERLAB_M7_BENCHES"]
fn gpu_reference_benches() {
    let (device, queue) = gpu();
    let dir = output();
    let spp: u32 = std::env::var("RENDERLAB_M7_SPP").map_or(4096, |s| s.parse().unwrap());
    let list = std::env::var("RENDERLAB_M7_BENCHES").expect("RENDERLAB_M7_BENCHES");
    let mut first: Option<(String, Vec<[f32; 3]>)> = None;
    for entry in list.split(';').filter(|s| !s.is_empty()) {
        let (bench_dir, spp) = entry
            .rsplit_once('@')
            .map_or((entry, spp), |(d, s)| (d, s.parse().unwrap()));
        let bench = load_bench(&Path::new(bench_dir).join("scene.json"));
        let name = Path::new(bench_dir)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        for (pose, v) in &bench.views {
            let settings = Settings {
                samples: spp,
                smith: bench.smith,
                ..Default::default()
            };
            let start = std::time::Instant::now();
            let image = render(&device, &queue, &bench.scene, v, &settings).unwrap();
            let seconds = start.elapsed().as_secs_f64();
            eprintln!(
                "{name}/{pose}: {spp} spp in {seconds:.2} s, invalid {}",
                image.invalid_pixels
            );
            assert_eq!(image.invalid_pixels, 0, "{name}/{pose}: non-finite samples");
            save(
                &dir,
                &format!("{name}-{pose}"),
                &image,
                serde_json::json!({
                "bench": name, "pose": pose, "scene_sha256": bench.sha256, "seconds": seconds,
                "seed": settings.seed, "max_depth": settings.max_depth, "smith": format!("{:?}", settings.smith)}),
            );
            if first.is_none() {
                first = Some((format!("{name}/{pose}"), image.mean));
                let replay = render(&device, &queue, &bench.scene, v, &settings).unwrap();
                let (label, mean) = first.as_ref().unwrap();
                let identical = replay
                    .mean
                    .iter()
                    .flatten()
                    .zip(mean.iter().flatten())
                    .all(|(a, b)| a.to_bits() == b.to_bits());
                std::fs::write(
                    dir.join("replay.json"),
                    serde_json::to_vec(
                        &serde_json::json!({"case": label, "bit_identical": identical}),
                    )
                    .unwrap(),
                )
                .unwrap();
                assert!(identical, "{label}: same-seed replay differs");
            }
        }
    }
}
