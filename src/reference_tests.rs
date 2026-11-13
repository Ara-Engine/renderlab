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
        ..Default::default()
    }
}
fn instance(mesh: usize, material: usize) -> Instance {
    Instance {
        casts_shadows: true,
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
fn narrow_sun_encoding_preserves_illuminance() {
    for radius in [0.00465, 0.0001, 0.05] {
        let scene = Scene {
            sun: Some(Sun {
                to_light: Vec3::Y,
                illuminance: [1000.0; 3],
                angular_radius: radius,
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
        let f = |word: usize| {
            f32::from_le_bytes(bytes[4 * word..4 * word + 4].try_into().unwrap()) as f64
        };
        // The shader samples this encoded cone height, including when nominal cos(r)
        // rounds to one. Its radiance must integrate to the requested illuminance.
        // Historical packets encoded only cos(r); the revised packet also carries
        // its precise cone height. Decode either format to test the actual cone.
        let height = if f(61) > 0.0 { f(61) } else { 1.0 - f(35) };
        assert!(
            height > 0.0,
            "a finite disk must not become a delta at radius {radius}"
        );
        let integral = f(36) * std::f64::consts::PI * height * (2.0 - height);
        assert!(
            (integral / 1000.0 - 1.0).abs() < 2e-6,
            "radius {radius}: {integral}"
        );
        let exact = 2.0 * (0.5 * f64::from(radius)).sin().powi(2);
        assert!((height / exact - 1.0).abs() < 2e-7);
    }
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
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
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
            base_color_texture: m["base_texture"].as_u64().map(|texture| {
                renderlab_asset::BaseColorTexture {
                    texture: texture as u32,
                    sampler: Default::default(),
                }
            }),
            ..Default::default()
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
            uvs: m["uvs"].as_array().map_or_else(Vec::new, |list| {
                list.iter().map(|v| [f(&v[0]), f(&v[1])]).collect()
            }),
            ..Default::default()
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
            textures: v["textures"].as_array().map_or_else(Vec::new, |list| {
                list.iter()
                    .map(|t| renderlab_asset::Texture {
                        role: renderlab_asset::TextureRole::BaseColorSrgb,
                        mips: t["mips"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| renderlab_asset::Mip {
                                width: m["width"].as_u64().unwrap() as u32,
                                height: m["height"].as_u64().unwrap() as u32,
                                rgba8: std::fs::read(
                                    path.parent().unwrap().join(m["file"].as_str().unwrap()),
                                )
                                .unwrap(),
                            })
                            .collect(),
                    })
                    .collect()
            }),
            camera_background: v["camera_background"]
                .as_array()
                .map(|_| v3(&v["camera_background"])),
            ..Default::default()
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

pub(super) fn gpu() -> (wgpu::Device, wgpu::Queue) {
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
        max_storage_buffers_per_shader_stage: 9,
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

pub(super) fn output() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var("RENDERLAB_M7_REFERENCE_OUT").expect("RENDERLAB_M7_REFERENCE_OUT"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn save(dir: &Path, name: &str, image: &Image, meta: serde_json::Value) {
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
    for (half, values) in image.halves.iter().enumerate() {
        std::fs::write(dir.join(format!("{name}-half{half}.f32")), raw(values)).unwrap();
    }
    if !image.geometry.is_empty() {
        std::fs::write(dir.join(format!("{name}-geometry.bin")), &image.geometry).unwrap();
    }
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
        // RGB shares a path and is correlated. Pool per-pixel channel means first; the
        // triangle inequality bounds their uncertainty without inventing three samples.
        sum += (0..3)
            .map(|c| f64::from(image.mean[p][c] / expected[c]))
            .sum::<f64>()
            / 3.0;
        var += ((0..3)
            .map(|c| f64::from(image.standard_error[p][c] / expected[c]))
            .sum::<f64>()
            / 3.0)
            .powi(2);
        n += 1.0;
    }
    (sum / n, var.sqrt() / n)
}

#[test]
fn region_does_not_count_correlated_rgb_as_three_paths() {
    let image = Image {
        width: 1,
        height: 1,
        samples: 4,
        mean: vec![[8.0; 3]],
        standard_error: vec![[0.8; 3]],
        halves: [vec![[8.8; 3]], vec![[7.2; 3]]],
        geometry: Vec::new(),
        invalid_pixels: 0,
    };
    let (mean, se) = region(&image, [8.0; 3], 0..1);
    assert_eq!(mean, 1.0);
    assert!((se - 0.1).abs() < 1e-7);
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
                    n[axis] = side;
                    m.positions.push(p);
                    m.normals.push(n);
                }
                if side > 0.0 {
                    m.indices
                        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                } else {
                    m.indices
                        .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
                }
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
        ..Default::default()
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
    let mut seeds = vec![];
    for k in 0..8 {
        let settings = Settings {
            seed: settings.seed.wrapping_add(k * 7919),
            ..settings
        };
        let image = render(&device, &queue, &furnace, &v, &settings).unwrap();
        let (mean, se) = region(&image, [8.0; 3], 0..image.mean.len());
        save(
            &dir,
            &format!("furnace-{k}"),
            &image,
            serde_json::json!({"expected":8.0,"relative_mean":mean,"relative_se":se,"seed":settings.seed}),
        );
        seeds.push(serde_json::json!({"seed":settings.seed,"mean":mean,"se":se}));
        rows.push((format!("furnace-{k}"), mean, se, image.invalid_pixels));
    }
    std::fs::write(
        dir.join("furnace-seeds.json"),
        serde_json::to_vec_pretty(&seeds).unwrap(),
    )
    .unwrap();
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
        ..Default::default()
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
    rows.push(("delta-floor".into(), mean, se, image.invalid_pixels));
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
    rows.push(("sky-floor".into(), mean, se, image.invalid_pixels));
    let prepared_sky = Scene {
        sky: [0.0; 3],
        environment: Some(Environment {
            radiance: renderlab_asset::environment::CubeLevel::new(
                8,
                vec![[100.0; 3]; 6 * 8 * 8],
                renderlab_asset::environment::Limits::unbounded(),
            )
            .unwrap(),
            yaw: 0.7,
            intensity: 1.0,
        }),
        ..sky.clone()
    };
    let image = render(
        &device,
        &queue,
        &prepared_sky,
        &v,
        &Settings {
            samples: 8192,
            ..Default::default()
        },
    )
    .unwrap();
    let (mean, se) = region(&image, [50.0; 3], 0..image.mean.len());
    save(
        &dir,
        "cube-sky-floor",
        &image,
        serde_json::json!({"expected":50.0,"relative_mean":mean,"relative_se":se}),
    );
    rows.push(("cube-sky-floor".into(), mean, se, image.invalid_pixels));
    let mut texels = Vec::new();
    for face in renderlab_asset::environment::CubeFace::ALL {
        for y in 0..128 {
            for x in 0..128 {
                let d = face.direction([
                    2.0 * (f64::from(x) + 0.5) / 128.0 - 1.0,
                    2.0 * (f64::from(y) + 0.5) / 128.0 - 1.0,
                ]);
                texels.push([
                    (100.0 + 30.0 * d[1]) as f32,
                    (120.0 - 40.0 * d[1]) as f32,
                    (50.0 + 15.0 * d[1]) as f32,
                ]);
            }
        }
    }
    let gradient_sky = Scene {
        environment: Some(Environment {
            radiance: renderlab_asset::environment::CubeLevel::new(
                128,
                texels,
                renderlab_asset::environment::Limits::unbounded(),
            )
            .unwrap(),
            yaw: 0.7,
            intensity: 1.0,
        }),
        ..prepared_sky
    };
    // Integral of (a+b*y) cos(theta) over the upper hemisphere is pi*(a+2*b/3).
    let expected = [60.0, 46.6666667, 30.0];
    let image = render(
        &device,
        &queue,
        &gradient_sky,
        &v,
        &Settings {
            samples: 16384,
            ..Default::default()
        },
    )
    .unwrap();
    let (mean, se) = region(&image, expected, 0..image.mean.len());
    save(
        &dir,
        "gradient-cube-floor",
        &image,
        serde_json::json!({"expected":expected,"relative_mean":mean,"relative_se":se}),
    );
    rows.push(("gradient-cube-floor".into(), mean, se, image.invalid_pixels));
    // A constant tangent-space normal tests the complete prepared normal-map decode,
    // tangent frame and linear (not sRGB) texture sampling against analytic direct light.
    let texel = [128u8, 180, 230, 255];
    let decoded = Vec3::new(
        128.0 / 255.0 * 2.0 - 1.0,
        180.0 / 255.0 * 2.0 - 1.0,
        230.0 / 255.0 * 2.0 - 1.0,
    )
    .normalize();
    let mut normal_floor = sky;
    normal_floor.sky = [0.0; 3];
    normal_floor.sun = Some(Sun {
        to_light: Vec3::Y,
        illuminance: [1000.0; 3],
        angular_radius: 0.0,
    });
    normal_floor.materials[0].normal_texture = Some(renderlab_asset::NormalTexture {
        texture: 0,
        sampler: Default::default(),
        scale: 1.0,
    });
    normal_floor.textures.push(renderlab_asset::Texture {
        role: renderlab_asset::TextureRole::NormalLinear,
        mips: vec![renderlab_asset::Mip {
            width: 2,
            height: 2,
            rgba8: texel.repeat(4),
        }],
    });
    let image = render(
        &device,
        &queue,
        &normal_floor,
        &view(Vec3::new(0.0, 3.0, 0.1), Vec3::ZERO, 16),
        &Settings {
            samples: 64,
            max_depth: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let expected = 0.5 / std::f32::consts::PI * 1000.0 * decoded.z;
    let (mean, se) = region(&image, [expected; 3], 0..image.mean.len());
    save(
        &dir,
        "normal-floor",
        &image,
        serde_json::json!({"expected":expected,"relative_mean":mean,"relative_se":se}),
    );
    rows.push(("normal-floor".into(), mean, se, image.invalid_pixels));
    let mut disk_floor = normal_floor;
    disk_floor.materials[0].normal_texture = None;
    disk_floor.materials[0].specular = 0.0;
    disk_floor.textures.clear();
    for radius in [0.00465, 0.0001] {
        disk_floor.sun.as_mut().unwrap().angular_radius = radius;
        let image = render(
            &device,
            &queue,
            &disk_floor,
            &view(Vec3::new(0.0, 3.0, 0.1), Vec3::ZERO, 16),
            &Settings {
                samples: 4096,
                max_depth: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let expected = 0.5 / std::f32::consts::PI * 1000.0;
        let (mean, se) = region(&image, [expected; 3], 0..image.mean.len());
        assert!(
            (mean - 1.0).abs() < (4.0 * se).max(5e-5),
            "small disk {radius}: {mean} +/- {se}"
        );
        let name = format!("disk-floor-{radius}");
        save(
            &dir,
            &name,
            &image,
            serde_json::json!({"radius":radius,"expected":expected,"relative_mean":mean,"relative_se":se}),
        );
        rows.push((name, mean, se, image.invalid_pixels));
    }
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
    let seed =
        std::env::var("RENDERLAB_M7_SEED").map_or(Settings::default().seed, |s| s.parse().unwrap());
    let samples_per_dispatch = std::env::var("RENDERLAB_M7_SAMPLES_PER_DISPATCH")
        .map_or(Settings::default().samples_per_dispatch, |s| {
            s.parse().unwrap()
        });
    let max_depth = std::env::var("RENDERLAB_M7_MAX_DEPTH")
        .map_or(Settings::default().max_depth, |s| s.parse().unwrap());
    let list = std::env::var("RENDERLAB_M7_BENCHES").expect("RENDERLAB_M7_BENCHES");
    let mut first = true;
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
                seed,
                samples_per_dispatch,
                max_depth,
                smith: bench.smith,
                capture_geometry: std::env::var("RENDERLAB_M7_CAPTURE_GEOMETRY").as_deref()
                    == Ok("1"),
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
                "seed": settings.seed, "max_depth": settings.max_depth, "smith": format!("{:?}", settings.smith),
                "samples_per_dispatch":settings.samples_per_dispatch,
                "camera_background":bench.scene.camera_background,
                "visibility_samples_per_primary":settings.visibility_samples}),
            );
            if first {
                first = false;
                let label = format!("{name}/{pose}");
                let count = std::env::var("RENDERLAB_M7_REPLAYS")
                    .map_or(1, |v| v.parse::<usize>().unwrap());
                assert!(count > 0);
                let mut reports = Vec::with_capacity(count);
                for _ in 0..count {
                    let replay = render(&device, &queue, &bench.scene, v, &settings).unwrap();
                    reports.push(replay_report(&image, &replay));
                }
                let identical = reports.iter().all(|r| r["bit_identical"] == true);
                std::fs::write(
                    dir.join("replay.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({"case":label,"comparisons":reports,
                        "bit_identical":identical,
                        "scope":"same seed and dispatch layout, independent acceleration-structure and pipeline rebuild"}))
                    .unwrap(),
                )
                .unwrap();
                assert!(identical, "{label}: same-seed replay differs");
            }
        }
    }
}

pub(super) fn replay_report(a: &Image, b: &Image) -> serde_json::Value {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let width = a.width;
    let rgb = |a: &[[f32; 3]], b: &[[f32; 3]]| {
        let reference_rms = (a
            .iter()
            .flatten()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            / (3 * a.len()) as f64)
            .sqrt();
        let mut changed_pixels = 0;
        let mut changed_channels = 0;
        let mut sum_squared = 0.0f64;
        let mut max_abs = 0.0f32;
        let mut max_ulp = 0;
        let mut examples = Vec::new();
        for (pixel, (x, y)) in a.iter().zip(b).enumerate() {
            if x.map(f32::to_bits) == y.map(f32::to_bits) {
                continue;
            }
            changed_pixels += 1;
            for (x, y) in x.iter().zip(y) {
                changed_channels += u32::from(x.to_bits() != y.to_bits());
                sum_squared += f64::from(x - y).powi(2);
                max_abs = max_abs.max((x - y).abs());
                max_ulp = max_ulp.max(x.to_bits().abs_diff(y.to_bits()));
            }
            if examples.len() < 32 {
                examples.push(serde_json::json!({"pixel":[pixel as u32 % width,pixel as u32 / width],"first":x,"replay":y}));
            }
        }
        let rms = (sum_squared / (3 * a.len()) as f64).sqrt();
        let relative = if reference_rms > 0.0 {
            rms / reference_rms
        } else if rms == 0.0 {
            0.0
        } else {
            f64::MAX
        };
        serde_json::json!({"changed_pixels":changed_pixels,"changed_channels":changed_channels,
            "rms_absolute":rms,"rms_relative":relative,"reference_rms":reference_rms,"max_absolute":max_abs,
            "max_ulp":max_ulp,"first_changed_examples":examples})
    };
    let mut centre = true;
    let mut counts = true;
    let mut visibility = true;
    for (x, y) in a.geometry.chunks_exact(80).zip(b.geometry.chunks_exact(80)) {
        centre &= x[..12] == y[..12] && x[16..44] == y[16..44];
        counts &= x[12..16] == y[12..16] && x[44..48] == y[44..48];
        visibility &= x[60..64] == y[60..64] && x[76..80] == y[76..80];
    }
    let mean = rgb(&a.mean, &b.mean);
    let half0 = rgb(&a.halves[0], &b.halves[0]);
    let half1 = rgb(&a.halves[1], &b.halves[1]);
    let identical = mean["changed_channels"] == 0
        && half0["changed_channels"] == 0
        && half1["changed_channels"] == 0
        && a.geometry == b.geometry
        && a.invalid_pixels == 0
        && b.invalid_pixels == 0;
    serde_json::json!({"bit_identical":identical,"mean":mean,"half0":half0,"half1":half1,
        "geometry_captured":!a.geometry.is_empty(),"centre_geometry_identical":centre,
        "accepted_counts_identical":counts,"visibility_sums_identical":visibility,
        "all_geometry_bytes_identical":a.geometry==b.geometry,
        "invalid_pixels":[a.invalid_pixels,b.invalid_pixels]})
}

/// Only the independent-AS HDR comparison is bounded. Fixed resources, geometry,
/// direct/visibility sums and invalid flags retain the exact gate.
pub(super) fn qualify_as_replay(
    report: &serde_json::Value,
    noise_rms: f64,
    fixed_exact: bool,
) -> bool {
    let invariants = report["geometry_captured"] == true
        && report["all_geometry_bytes_identical"] == true
        && report["centre_geometry_identical"] == true
        && report["accepted_counts_identical"] == true
        && report["visibility_sums_identical"] == true
        && report["invalid_pixels"] == serde_json::json!([0, 0]);
    if !invariants || !noise_rms.is_finite() || noise_rms < 0.0 {
        return false;
    }
    if report["bit_identical"] == true {
        return true;
    }
    fixed_exact
        && ["mean", "half0", "half1"].iter().all(|key| {
            let value = &report[*key];
            value["max_ulp"].as_u64().is_some_and(|v| v <= 4)
                && value["rms_relative"]
                    .as_f64()
                    .is_some_and(|v| v.is_finite() && v >= 0.0 && v <= 1e-5)
                && value["rms_absolute"]
                    .as_f64()
                    .is_some_and(|v| v.is_finite() && v >= 0.0 && v <= 1e-3 * noise_rms)
        })
}

#[test]
fn bounded_as_replay_keeps_strict_invariants_and_hard_numeric_limits() {
    let image = || Image {
        width: 1,
        height: 1,
        samples: 2,
        mean: vec![[1.0; 3]],
        halves: [vec![[1.1; 3]], vec![[0.9; 3]]],
        standard_error: vec![[0.1; 3]],
        geometry: vec![0; 80],
        invalid_pixels: 0,
    };
    let a = image();
    let mut b = image();
    let exact = replay_report(&a, &b);
    assert!(qualify_as_replay(&exact, 0.0, false));
    b.mean[0][0] = f32::from_bits(1.0f32.to_bits() + 4);
    let bounded = replay_report(&a, &b);
    assert!(qualify_as_replay(&bounded, 0.1, true));
    assert!(!qualify_as_replay(&bounded, 0.1, false));
    assert!(!qualify_as_replay(&bounded, 0.0, true));
    for key in ["mean", "half0", "half1"] {
        for (field, value) in [
            ("max_ulp", serde_json::json!(5)),
            ("rms_relative", serde_json::json!(1.01e-5)),
            ("rms_absolute", serde_json::json!(1.01e-4)),
            ("rms_absolute", serde_json::Value::Null),
        ] {
            let mut bad = bounded.clone();
            bad[key][field] = value;
            assert!(!qualify_as_replay(&bad, 0.1, true), "{key}/{field}");
        }
    }
    b.geometry[0] = 1;
    assert!(!qualify_as_replay(&replay_report(&a, &b), 0.1, true));
    b = image();
    b.invalid_pixels = 1;
    assert!(!qualify_as_replay(&replay_report(&a, &b), 0.1, true));
}

/// The ordinary bench suite replays only its first pose. Keep the S2G camera-0.25px
/// frame-zero regression explicit, including secondary transport and every D/V byte.
#[test]
#[ignore = "GPU; RENDERLAB_M7_BENCHES must name the registered S2G frame-zero fixture"]
fn gpu_reference_s2g_replay_regression() {
    let list = std::env::var("RENDERLAB_M7_BENCHES").unwrap();
    let (path, spp) = list.split(';').next().unwrap().rsplit_once('@').unwrap();
    let bench = load_bench(&Path::new(path).join("scene.json"));
    let (name, view) = &bench.views[0];
    assert_eq!(name, "camera-0.25px-000", "use the failing registered pose");
    assert_eq!(view.open.eye, Vec3::new(0.0, 0.45, 0.75));
    assert_eq!(bench.scene.camera_background, Some([0.0; 3]));
    let settings = Settings {
        samples: spp.parse().unwrap(),
        smith: bench.smith,
        capture_geometry: true,
        ..Default::default()
    };
    let (device, queue) = gpu();
    let dir = output();
    let first = render(&device, &queue, &bench.scene, view, &settings).unwrap();
    let mut reports = Vec::new();
    for _ in 0..8 {
        let replay = render(&device, &queue, &bench.scene, view, &settings).unwrap();
        reports.push(replay_report(&first, &replay));
    }
    let identical = reports.iter().all(|r| r["bit_identical"] == true);
    save(
        &dir,
        "s2g-frame-zero",
        &first,
        serde_json::json!({"scene_sha256":bench.sha256,"pose":name,
            "camera_background":bench.scene.camera_background,"accepted_for_scoring":identical,
            "seed":settings.seed,"samples_per_dispatch":settings.samples_per_dispatch,
            "max_depth":settings.max_depth,"visibility_samples_per_primary":settings.visibility_samples,
            "frame":{"open":{"eye":view.open.eye.to_array(),"target":view.open.target.to_array()},
                "close":{"eye":view.close.eye.to_array(),"target":view.close.target.to_array()},
                "fov_y":view.fov_y,"width":view.width,"height":view.height}}),
    );
    std::fs::write(
        dir.join("determinism.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"scene_sha256":bench.sha256,"pose":name,
            "comparisons":reports,"bit_identical":identical,
            "scope":"eight independent AS and pipeline rebuilds; full transport, actual halves and all geometry bytes"})).unwrap(),
    ).unwrap();
    assert!(
        identical,
        "S2G frame-zero independent rebuild replay differs"
    );
}

#[test]
#[ignore = "GPU; diagnostic replay with fixed AS/pipeline; RENDERLAB_M7_BENCHES"]
fn gpu_reference_replay_loaded() {
    let list = std::env::var("RENDERLAB_M7_BENCHES").unwrap();
    let entry = list.split(';').next().unwrap();
    let (path, spp) = entry.rsplit_once('@').unwrap();
    let bench = load_bench(&Path::new(path).join("scene.json"));
    let view = &bench.views[0].1;
    let settings = Settings {
        samples: spp.parse().unwrap(),
        smith: bench.smith,
        capture_geometry: std::env::var("RENDERLAB_M7_CAPTURE_GEOMETRY").as_deref() == Ok("1"),
        ..Default::default()
    };
    let (device, queue) = gpu();
    let images = render_repeats(
        &device,
        &queue,
        &bench.scene,
        view,
        &settings,
        3,
        std::env::var("RENDERLAB_M7_REBUILD_PIPELINE").as_deref() == Ok("1"),
    )
    .unwrap();
    let reports: Vec<_> = images[1..]
        .iter()
        .map(|b| replay_report(&images[0], b))
        .collect();
    std::fs::write(
        output().join("loaded-replay.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    assert!(
        reports.iter().all(|r| r["bit_identical"] == true),
        "fixed-resource replay differs"
    );
}
