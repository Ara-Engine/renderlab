//! M7-ES1: the dominant-lobe environment specular and its occlusion (docs/m7-es1.md).
//! CPU checks of the lobe fit and share table, then two GPU measurements that one process
//! per path runs: a sky-lit plane against the reference tracer (a roughness, view and
//! metal sweep under the registered RLE2 map), and the registered quarry's captures and
//! per-stage costs for the scorer (scripts/m7-es1-score.py).
use super::*;
use crate::environment_lobes::{self as lobes, Lobe};
use glam::DVec3;
use serde_json::{Value as Json, json};
use std::path::PathBuf;

/// A cosine-power sun of `peak` radiance over a constant sky, as the registered sun128 map.
fn sun_field(size: u32, sky: [f64; 3], peak: [f64; 3], axis: DVec3, power: i32) -> CubeLevel {
    let axis = axis.normalize();
    level(size, |d| {
        let c = DVec3::from_array(d).dot(axis).max(0.0).powi(power);
        std::array::from_fn(|k| (sky[k] + peak[k] * c) as f32)
    })
}

#[test]
fn environment_lobes_fit_suns_and_ignore_smooth_fields() {
    let axis = DVec3::new(0.36, 0.48, 0.8);
    let sun = sun_field(64, [0.02, 0.04, 0.1], [2e6, 1.6e6, 1e6], axis, 128);
    let fitted = lobes::fit(&sun);
    assert_eq!(fitted.len(), 1, "{fitted:?}");
    let lobe = fitted[0];
    let angle = DVec3::from_array(lobe.direction)
        .angle_between(axis.normalize())
        .to_degrees();
    assert!(angle < 0.5, "axis {angle} degrees off");
    // cos^128 is a vMF lobe of sharpness 129 to first order.
    assert!((lobe.sharpness / 129.0 - 1.0).abs() < 0.05, "{lobe:?}");
    // Flux matches the cosine lobe's 2 pi / 129 times its peak.
    let flux = lobe.amplitude[0] * lobe.solid_angle();
    let expected = 2e6 * std::f64::consts::TAU / 129.0;
    assert!((flux / expected - 1.0).abs() < 0.03, "{flux} vs {expected}");
    // Constant and smooth fields keep everything in the prefiltered field.
    assert!(lobes::fit(&level(16, |_| [1.0, 2.0, 3.0])).is_empty());
    assert!(lobes::fit(&level(32, |d| [(1.0 + 0.5 * d[1]) as f32; 3])).is_empty());
    // Two separated suns are both found, brightest first.
    let second = DVec3::new(-0.7, 0.3, -0.2).normalize();
    let pair = level(64, |d| {
        let d = DVec3::from_array(d);
        let g = |s: DVec3, lambda: f64, a: f64| a * (lambda * (d.dot(s) - 1.0)).exp();
        [(0.1 + g(axis.normalize(), 300.0, 1e5) + g(second, 900.0, 1e5)) as f32; 3]
    });
    let fitted = lobes::fit(&pair);
    assert_eq!(fitted.len(), 2, "{fitted:?}");
    assert!(DVec3::from_array(fitted[0].direction).angle_between(axis.normalize()) < 0.01);
    assert!(DVec3::from_array(fitted[1].direction).angle_between(second) < 0.01);
    assert!(
        (fitted[1].sharpness / 900.0 - 1.0).abs() < 0.05,
        "{fitted:?}"
    );
}

/// A dense midpoint quadrature of the cooker's normalized N=V kernel against a lobe, over
/// angles from R spaced quadratically, so it resolves sharp lobes on the axis.
fn dense_share(lobe: &Lobe, r: DVec3, roughness: f64) -> f64 {
    let a = (roughness * roughness).max(1e-4).powi(2);
    let (rows, cols) = (2400, 1200);
    let (t, b) = r.any_orthonormal_pair();
    let (mut num, mut den) = (0.0, 0.0);
    for i in 0..rows {
        let u = (i as f64 + 0.5) / rows as f64;
        let theta = std::f64::consts::FRAC_PI_2 * u * u;
        let (s, c) = theta.sin_cos();
        let weight = s * std::f64::consts::PI * u;
        for k in 0..cols {
            let phi = std::f64::consts::TAU * (k as f64 + 0.5) / cols as f64;
            let l = t * (s * phi.cos()) + b * (s * phi.sin()) + r * c;
            let q = a + (1.0 - a) * (1.0 - c) * 0.5;
            let kernel = a / (q * q) * c * weight;
            let shape = (lobe.sharpness * (l.dot(DVec3::from_array(lobe.direction)) - 1.0)).exp();
            num += kernel * shape;
            den += kernel;
        }
    }
    num / den
}

#[test]
fn environment_lobe_shares_match_dense_quadrature() {
    let axis = DVec3::new(0.36, 0.48, 0.8).normalize();
    let (t, _) = axis.any_orthonormal_pair();
    for sharpness in [129.0, 900.0] {
        let lobe = Lobe {
            direction: axis.to_array(),
            sharpness,
            amplitude: [1.0; 3],
        };
        for roughness in [0.3, 0.6, 0.85, 1.0] {
            for degrees in [0.0, 10.0, 40.0, 80.0] {
                let (s, c) = f64::to_radians(degrees).sin_cos();
                let r = axis * c + t * s;
                let ours = lobes::share(&lobe, r.to_array(), roughness);
                let dense = dense_share(&lobe, r, roughness);
                assert!(
                    (ours - dense).abs() <= 0.01 * dense + 1e-9,
                    "lambda {sharpness} r {roughness} at {degrees}: {ours} vs {dense}"
                );
            }
        }
    }
}

#[test]
fn environment_lobe_table_layout() {
    let lobe = Lobe {
        direction: [0.0, 1.0, 0.0],
        sharpness: 200.0,
        amplitude: [3.0, 2.0, 1.0],
    };
    let texels = lobes::table(&[lobe], &env::roughness_knots());
    assert_eq!(texels.len(), lobes::SHARE_BINS * lobes::TABLE_ROWS);
    assert_eq!(texels[0], [0.0, 1.0, 0.0, 200.0]);
    assert_eq!(texels[4][..3], [3.0, 2.0, 1.0]);
    assert!((f64::from(texels[4][3]) / lobe.solid_angle() - 1.0).abs() < 1e-6);
    assert_eq!(texels[8][0], 1.0);
    // Knot 0 is the delta convention: the share is the lobe's own shape.
    let row = lobes::SHARE_BINS;
    assert_eq!(texels[row][0], 1.0);
    let j = 20;
    let expected = (200.0 * (lobes::bin_angle(j).cos() - 1.0)).exp();
    assert!((f64::from(texels[row + j][0]) - expected).abs() < 1e-6);
    // Unused lobe channels stay zero.
    assert!(
        texels[row..]
            .iter()
            .all(|t| t[1] == 0.0 && t[2] == 0.0 && t[3] == 0.0)
    );
}

/// Upload-time CPU cost of the fit and share table on a registered map
/// (RENDERLAB_ES1_MAP), in this build's profile. CPU only: no device.
#[test]
#[ignore = "CPU timing; RENDERLAB_ES1_MAP"]
fn es1_lobe_fit_cost() {
    let map = crate::asset::environment::decode(
        &std::fs::read(var("RENDERLAB_ES1_MAP").unwrap()).unwrap(),
        crate::asset::environment::Limits::unbounded(),
    )
    .unwrap();
    let start = Instant::now();
    let fitted = lobes::fit(&map.specular_radiance()[0]);
    let fit = start.elapsed();
    let start = Instant::now();
    let texels = lobes::table(&fitted, &env::roughness_knots());
    let table = start.elapsed();
    eprintln!(
        "es1 lobe fit {:.1} ms, table {:.1} ms, lobes {fitted:?}, table bytes {}, debug_assertions={}",
        fit.as_secs_f64() * 1e3,
        table.as_secs_f64() * 1e3,
        texels.len() * 16,
        cfg!(debug_assertions)
    );
}

// ---- GPU measurements ----

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../examples/m4/cameras.rs"]
mod es1_cameras;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../viewer/src/prepared.rs"]
mod es1_prepared;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}
/// RENDERLAB_ES1_PATH: "rt" (hardware rays, traced primaries) or "nonrt" (a device without
/// ray queries, visibility-buffer primaries).
fn nonrt() -> bool {
    var("RENDERLAB_ES1_PATH").as_deref() == Some("nonrt")
}
fn es1_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let descriptor = if nonrt() {
        let mut limits = crate::visibility_limits(wgpu::Limits::default());
        limits.max_buffer_size = a.max_buffer_size;
        limits.max_storage_buffer_binding_size = a.max_storage_buffer_binding_size;
        wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | crate::VISIBILITY_FEATURES
                | crate::VISIBILITY_RASTER_FEATURES
                | crate::VIRTUAL_FEATURES
                | wgpu::Features::TIMESTAMP_QUERY,
            required_limits: limits,
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        }
    } else {
        assert_eq!(
            var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
            Some("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        );
        wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | wgpu::Features::EXPERIMENTAL_RAY_QUERY
                | crate::TRACED_FEATURES
                | wgpu::Features::TIMESTAMP_QUERY,
            required_limits: crate::traced_limits(wgpu::Limits {
                max_blas_primitive_count: a.max_blas_primitive_count,
                max_blas_geometry_count: 1,
                max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
                max_acceleration_structures_per_shader_stage: 1,
                max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
                max_buffer_size: a.max_buffer_size,
                ..Default::default()
            }),
            // SAFETY: the test caller explicitly accepts wgpu's experimental contract.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    assert_eq!(
        device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
        !nonrt()
    );
    (device, queue)
}
/// Each path's shipping renderer: traced primaries with Interactive shadows on RT, the
/// visibility buffer with its creation-time Virtual shadows on non-RT (G1c's protocol).
fn es1_renderer(device: wgpu::Device, queue: wgpu::Queue, limits: Limits) -> Renderer {
    // AA state (docs/iq1-quarry-image.md): 1x, TR1 off, anisotropy 1, the shipping default.
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    let r = if nonrt() {
        Renderer::new_with_options(device, queue, FORMAT, limits, options, SOURCE).unwrap()
    } else {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            limits,
            options,
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r
    };
    assert_eq!(
        r.primary_visibility(),
        if nonrt() {
            PrimaryVisibility::VisibilityBuffer
        } else {
            PrimaryVisibility::Traced
        }
    );
    r
}
fn es1_target(r: &Renderer, size: [u32; 2]) -> wgpu::Texture {
    r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("ES1 color"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn es1_frame(r: &mut Renderer, target: &wgpu::Texture, camera: &Camera, draws: &mut [Draw<'_>]) {
    let until = Instant::now() + Duration::from_secs(60);
    while r
        .try_render_view(target, camera, draws, ViewMode::Lit)
        .unwrap()
        .is_none()
    {
        assert!(Instant::now() < until, "ES1 frame admission");
        r.test_drain("ES1 admission");
    }
}
/// Exposed HDR back to radiance (cd/m^2), RGB per pixel.
fn es1_radiance(r: &mut Renderer, exposure: f32) -> Vec<[f32; 3]> {
    r.test_drain("ES1 HDR admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("ES1 HDR copy");
    let half: Vec<f32> = capture
        .try_read()
        .unwrap()
        .unwrap()
        .chunks_exact(2)
        .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
        .collect();
    let scale = (-exposure).exp2();
    half.chunks_exact(4)
        .map(|p| [p[0] * scale, p[1] * scale, p[2] * scale])
        .collect()
}
fn es1_settings(enabled: bool) -> IndirectSettings {
    IndirectSettings {
        enabled,
        cut_phases: 1,
        ..Default::default()
    }
}
/// The programs a measurement renders: the baseline standard source (RENDERLAB_ES1_BASELINE)
/// when given, then ES1's; with RENDERLAB_ES1_REPEAT=1 the baseline again last, which
/// separates run-order state from the program's effect.
fn es1_programs() -> Vec<(String, String)> {
    let mut programs = vec![("es1".to_string(), crate::STANDARD_PROGRAM.to_string())];
    if let Some(path) = var("RENDERLAB_ES1_BASELINE") {
        let baseline = std::fs::read_to_string(path).unwrap();
        if var("RENDERLAB_ES1_REPEAT").as_deref() == Some("1") {
            programs.push(("baseline-repeat".into(), baseline.clone()));
        }
        programs.insert(0, ("baseline".into(), baseline));
    }
    // RENDERLAB_ES1_VARIANTS: "name=path;..." standard-program variants screened after ES1.
    for item in var("RENDERLAB_ES1_VARIANTS")
        .iter()
        .flat_map(|v| v.split(';'))
    {
        let (name, path) = item.split_once('=').unwrap();
        programs.push((name.into(), std::fs::read_to_string(path).unwrap()));
    }
    programs
}
fn map_reference(map: &crate::asset::environment::PreparedEnvironment) -> reference::Environment {
    reference::Environment {
        radiance: map.specular_radiance()[0].clone(),
        yaw: map.metadata().rotation_y_radians,
        intensity: 1.0,
    }
}
use crate::reference;

/// The sky-lit plane: an open, untextured plane under the registered RLE2 map only, seen
/// from every view in a roughness, N.V, azimuth and metal sweep. Its centre pixel is
/// scored against a 16,384-spp depth-2 reference (camera, surface, sky), which the RT
/// process renders and writes to RENDERLAB_ES1_OUT/plane-reference.json for the non-RT one.
/// The cache runs (the plane is open, so it reads unoccluded) and is off, for both the
/// baseline and the ES1 standard program. RENDERLAB_ES1_MAP: the RLE2 file.
#[test]
#[ignore = "GPU lease; RENDERLAB_ES1_PATH, RENDERLAB_ES1_MAP, RENDERLAB_ES1_OUT, optional RENDERLAB_ES1_BASELINE"]
fn gpu_es1_sky_plane() {
    let out = PathBuf::from(var("RENDERLAB_ES1_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let map = crate::asset::environment::decode(
        &std::fs::read(var("RENDERLAB_ES1_MAP").unwrap()).unwrap(),
        crate::asset::environment::Limits::unbounded(),
    )
    .unwrap();
    let sun = DVec3::new(0.36, 0.48, 0.8).normalize();
    let sun_azimuth = sun.z.atan2(sun.x);
    let mut cases = vec![];
    for metal in [false, true] {
        for rough in [0.3f32, 0.5, 0.85, 1.0] {
            // N.V 0.95 rather than 1: a camera straight above the plane has no look-at frame.
            for nv in [0.15f32, 0.3, 0.6, 0.95] {
                // Views beside the sun's azimuth, looking toward its glint, and with it behind.
                for (name, azimuth) in [
                    ("side", sun_azimuth + std::f64::consts::FRAC_PI_2),
                    ("glint", sun_azimuth + std::f64::consts::PI),
                    ("back", sun_azimuth),
                ] {
                    cases.push((metal, rough, nv, name, azimuth as f32));
                }
            }
        }
    }
    let vertices = [
        [-100., 0., -100.],
        [100., 0., -100.],
        [100., 0., 100.],
        [-100., 0., 100.],
    ];
    let indices = [0u32, 2, 1, 0, 3, 2];
    let eye_of = |nv: f32, azimuth: f32| {
        let s = (1.0 - nv * nv).sqrt();
        Vec3::new(s * azimuth.cos(), nv, s * azimuth.sin()) * 10.0
    };
    let (device, queue) = es1_device();
    let reference_path = out.join("plane-reference.json");
    let references: Vec<Json> = if nonrt() {
        serde_json::from_slice::<Json>(&std::fs::read(&reference_path).unwrap()).unwrap()["cases"]
            .as_array()
            .unwrap()
            .clone()
    } else {
        let mut rows = vec![];
        for &(metal, rough, nv, name, azimuth) in &cases {
            let scene = reference::Scene {
                meshes: vec![reference::Mesh {
                    positions: vertices.to_vec(),
                    normals: vec![[0., 1., 0.]; 4],
                    indices: indices.to_vec(),
                    ..Default::default()
                }],
                instances: vec![reference::Instance {
                    casts_shadows: true,
                    mesh: 0,
                    material: 0,
                    transform: Mat4::IDENTITY,
                }],
                materials: vec![reference::Material {
                    base_color: if metal { [0.9; 3] } else { [0.1; 3] },
                    metallic: if metal { 1.0 } else { 0.0 },
                    roughness: rough,
                    ..Default::default()
                }],
                sky: [0.0; 3],
                environment: Some(map_reference(&map)),
                ..Default::default()
            };
            let pose = reference::Pose {
                eye: eye_of(nv, azimuth),
                target: Vec3::ZERO,
            };
            let image = reference::render(
                &device,
                &queue,
                &scene,
                &reference::View {
                    open: pose,
                    close: pose,
                    up: Vec3::Y,
                    fov_y: 0.01,
                    width: 17,
                    height: 17,
                },
                &reference::Settings {
                    samples: 16384,
                    samples_per_dispatch: 64,
                    max_depth: 2,
                    ..Default::default()
                },
            )
            .unwrap();
            // A case whose reference has non-finite samples is recorded, not scored.
            let c = 8 * 17 + 8;
            rows.push(json!({"metal": metal, "roughness": rough, "nv": nv, "view": name,
                "invalid_pixels": image.invalid_pixels, "mean": image.mean[c], "se": image.standard_error[c]}));
        }
        std::fs::write(
            &reference_path,
            serde_json::to_vec_pretty(&json!({"samples": 16384, "max_depth": 2, "cases": rows}))
                .unwrap(),
        )
        .unwrap();
        rows
    };
    let mut r = es1_renderer(device, queue, Limits::default());
    r.set_lighting(&Lighting {
        lights: vec![],
        environment: [0.0; 3],
        exposure_stops: -14.0,
    })
    .unwrap();
    let sky = loop {
        if let Some(sky) = r.try_upload_environment(&map).unwrap() {
            break sky;
        }
        r.test_drain("ES1 plane environment");
    };
    r.set_environment(Some(&sky), Default::default()).unwrap();
    let mesh_vertices = vertices.map(|position| Vertex {
        position,
        normal: [0., 1., 0.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let mesh = loop {
        if let Some(mesh) = r
            .try_upload_mesh(&MeshData {
                vertices: &mesh_vertices,
                indices: &indices,
            })
            .unwrap()
        {
            break mesh;
        }
        r.test_drain("ES1 plane mesh");
    };
    r.set_indirect_geometry(&mesh, &vertices, &indices).unwrap();
    let mut inst = loop {
        if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
            break i;
        }
        r.test_drain("ES1 plane instances");
    };
    let materials: Vec<((bool, f32), Material)> = [false, true]
        .into_iter()
        .flat_map(|metal| [0.3f32, 0.5, 0.85, 1.0].map(|rough| (metal, rough)))
        .map(|(metal, rough)| {
            let material = loop {
                let descriptor = MaterialDescriptor {
                    base_color: if metal {
                        [0.9, 0.9, 0.9, 1.0]
                    } else {
                        [0.1, 0.1, 0.1, 1.0]
                    },
                    metallic: if metal { 1.0 } else { 0.0 },
                    roughness: rough,
                    double_sided: true,
                    ..Default::default()
                };
                if let Some(m) = r.try_create_material(&descriptor).unwrap() {
                    break m;
                }
                r.test_drain("ES1 plane material");
            };
            ((metal, rough), material)
        })
        .collect();
    let target = es1_target(&r, [17, 17]);
    let mut rows = vec![];
    for (program, source) in es1_programs() {
        r.test_reload_standard(&source).unwrap();
        for enabled in [false, true] {
            r.set_indirect(es1_settings(enabled)).unwrap();
            for (&(metal, rough, nv, name, azimuth), reference) in cases.iter().zip(&references) {
                let material = &materials
                    .iter()
                    .find(|(k, _)| *k == (metal, rough))
                    .unwrap()
                    .1;
                let camera =
                    Camera::look_at(eye_of(nv, azimuth), Vec3::ZERO, Vec3::Y, 0.01, 0.01, 1000.0)
                        .unwrap();
                r.reset_indirect();
                for _ in 0..4 {
                    es1_frame(
                        &mut r,
                        &target,
                        &camera,
                        &mut [Draw::new(&mesh, &mut inst).with_material(material)],
                    );
                }
                let image = es1_radiance(&mut r, -14.0);
                let ours = image[8 * 17 + 8];
                rows.push(json!({"program": program, "cache": enabled, "metal": metal, "roughness": rough,
                    "nv": nv, "view": name, "radiance": ours, "reference": reference["mean"], "reference_se": reference["se"],
                    "reference_invalid_pixels": reference["invalid_pixels"]}));
            }
        }
    }
    let path = if nonrt() { "nonrt" } else { "rt" };
    std::fs::write(
        out.join(format!("plane-{path}.json")),
        serde_json::to_vec_pretty(&json!({"path": path, "rows": rows})).unwrap(),
    )
    .unwrap();
}

/// The registered quarry, one path per process (RENDERLAB_ES1_PATH), U0b's full-radiance
/// protocol (its own materials, sun and shadows, the RLE2 sky, a black backdrop, exposure
/// -14). RENDERLAB_ES1_QUARRY: a manifest with asset, limits, cameras, environment,
/// lighting, near (non-RT: the cooked near field, docs/m7-g1c.md) and out. Every program
/// (the baseline standard source, then ES1's) renders, in RENDERLAB_ES1_SECTIONS order:
/// - static: poses 0 and 1, cache off then on (a reset and eight settle frames);
/// - pan: from pose 0 settled, moving 2 cm a frame as U0b's pan, cache on, at its four
///   witness frames (97, 112, 128, 159);
/// - cost: cache off then on, 24 frames moving 1 cm a frame (two visits) and three cuts
///   of ten frames, each frame's span and shading pass;
/// - abcost: the same rows with every program interleaved in each of four visits.
///
/// Captures are raw little-endian f32 radiance for the CPU scorer; nothing is scored here.
/// Correctness and functional timing only: a speed claim needs a quiet window.
#[test]
#[ignore = "GPU lease; RENDERLAB_ES1_PATH, RENDERLAB_ES1_QUARRY, optional RENDERLAB_ES1_BASELINE"]
fn gpu_es1_quarry() {
    let start = Instant::now();
    let manifest: Json =
        serde_json::from_slice(&std::fs::read(var("RENDERLAB_ES1_QUARRY").unwrap()).unwrap())
            .unwrap();
    let file = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let path = if nonrt() { "nonrt" } else { "rt" };
    // RENDERLAB_ES1_TAG names a shader-variant process (RENDERLAB_GI_SHADERS) beside the path.
    let out = file("out")
        .join(var("RENDERLAB_ES1_TAG").map_or(path.to_string(), |t| format!("{path}-{t}")));
    std::fs::create_dir_all(&out).unwrap();
    let sections: Vec<String> = var("RENDERLAB_ES1_SECTIONS")
        .unwrap_or_else(|| "static,pan,cost".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let source = crate::asset::decode(
        &std::fs::read(file("asset")).unwrap(),
        crate::asset::Limits::unlimited(),
    )
    .unwrap();
    let cameras = es1_cameras::Cameras::load(&file("cameras")).unwrap();
    let map = crate::asset::environment::decode(
        &std::fs::read(file("environment")).unwrap(),
        crate::asset::environment::Limits::unbounded(),
    )
    .unwrap();
    let lighting: Json = serde_json::from_slice(&std::fs::read(file("lighting")).unwrap()).unwrap();
    let v3 = |v: &Json| Vec3::from_array(std::array::from_fn(|k| v[k].as_f64().unwrap() as f32));
    let (device, queue) = es1_device();
    let mut r = es1_renderer(
        device,
        queue,
        Limits {
            max_mesh_bytes: 134217728,
            max_texture_bytes: 134217728,
            ..Default::default()
        },
    );
    // M7-R1n: RENDERLAB_ES1_TR1=0 captures with TR1 off; TR1 (the default since TR1b)
    // jitters each program's frames differently, so program A/B images need it off.
    if var("RENDERLAB_ES1_TR1").as_deref() == Some("0") {
        r.set_reconstruction(crate::Reconstruction::off()).unwrap();
    }
    let exposure = -14.0;
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: v3(&lighting["lights"][0]["travel"]),
                lux: v3(&lighting["lights"][0]["lux"]).to_array(),
            }],
            environment: [0.0; 3],
            exposure_stops: exposure,
        },
        &Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![4., 12., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    if !nonrt() {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    let upload = Instant::now();
    let sky = loop {
        if let Some(sky) = r.try_upload_environment(&map).unwrap() {
            break sky;
        }
        r.test_drain("ES1 quarry environment");
    };
    let environment_upload_seconds = upload.elapsed().as_secs_f64();
    r.set_environment(Some(&sky), Default::default()).unwrap();
    drop(map);
    let mut asset = es1_prepared::Prepared::new_with_limits(
        source,
        crate::asset::Limits::unlimited(),
        1_000_000,
    )
    .unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("ES1 quarry upload");
    if nonrt() {
        let field = crate::asset::near_field::decode(
            &std::fs::read(file("near")).unwrap(),
            crate::asset::near_field::Limits::default(),
        )
        .unwrap();
        for (m, mesh) in asset.meshes_mut().iter().enumerate() {
            r.set_indirect_near_field(mesh, &field, m).unwrap();
        }
    }
    let frame0 = &cameras.frames[0];
    let size = [frame0.width, frame0.height];
    let target = es1_target(&r, size);
    let camera_at = |pose: usize, dx: f32| {
        let mut f = cameras.frames[pose].clone();
        f.eye[0] += dx;
        f.target[0] += dx;
        f.camera().unwrap()
    };
    let write = |name: &str, image: &[[f32; 3]]| {
        let raw: Vec<u8> = image
            .iter()
            .flatten()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        std::fs::write(out.join(format!("{name}.f32")), raw).unwrap();
    };
    let mut results = json!({"path": path, "environment_upload_seconds": environment_upload_seconds,
        "sections": sections, "programs": {}});
    // abcost: the cost rows paired, programs interleaved within each visit (A B A B), so
    // intermittent foreign GPU load falls on every program alike. Per cache state and
    // visit: a cut of ten frames, then 24 frames moving 1 cm a frame.
    if sections.iter().any(|s| s == "abcost") {
        let programs = es1_programs();
        let mut paired = json!({});
        for enabled in [false, true] {
            r.set_indirect(es1_settings(enabled)).unwrap();
            r.set_indirect_profiling(false);
            let cache = if enabled { "on" } else { "off" };
            for _ in 0..4 {
                for (program, text) in &programs {
                    r.test_reload_standard(text).unwrap();
                    let mut draw = |r: &mut Renderer, camera: &Camera| {
                        let mut draws = asset.draws(Visibility::Frustum);
                        es1_frame(r, &target, camera, &mut draws);
                    };
                    r.test_drain("ES1 paired cut");
                    r.poll().unwrap();
                    r.reset_indirect();
                    for _ in 0..10 {
                        draw(&mut r, &camera_at(0, 0.0));
                    }
                    r.test_drain("ES1 paired cut frames");
                    let cut = frame_times(&mut r, 10, 0);
                    for t in 0..24 {
                        draw(&mut r, &camera_at(0, 0.01 * t as f32));
                    }
                    r.test_drain("ES1 paired motion frames");
                    let motion = frame_times(&mut r, 24, 2);
                    let slot = &mut paired[program][cache];
                    if slot.is_null() {
                        *slot = json!({"cuts": [], "motion": []});
                    }
                    slot["cuts"].as_array_mut().unwrap().push(cut);
                    slot["motion"].as_array_mut().unwrap().push(motion);
                }
            }
        }
        results["paired"] = paired;
    }
    // MP1 (docs/m7-mp1.md): the quarry's twelve materials on 1, 2 and 8 programs (material
    // i on program i % K), interleaved in each of three visits per cache state, the `abcost`
    // rows. `mp1`: standard, toon, then distinct toon variants (images at pose 0 for K = 2);
    // `mp1std`: standard then distinct copies of it, so every pixel runs the same code and
    // only the per-program dispatch differs.
    for (section, base) in [
        ("mp1", include_str!("../shaders/programs/toon.wgsl")),
        ("mp1std", crate::STANDARD_PROGRAM),
    ] {
        if !sections.iter().any(|s| s == section) {
            continue;
        }
        let mut programs = vec![r.standard_program().clone()];
        for k in 0..7 {
            let source = if k == 0 && section == "mp1" {
                base.to_string()
            } else {
                base.replacen(
                    "struct Surface",
                    &format!("const VARIANT:f32={k}.0;\nstruct Surface"),
                    1,
                )
            };
            let label = format!("mp1-{k}");
            programs.push(
                r.try_create_program(&crate::ProgramDescriptor {
                    label: &label,
                    source: &source,
                })
                .unwrap(),
            );
        }
        let mut rows = json!({});
        for enabled in [false, true] {
            r.set_indirect(es1_settings(enabled)).unwrap();
            let cache = if enabled { "on" } else { "off" };
            for visit in 0..3 {
                for k in [1usize, 2, 8] {
                    let set: Vec<&crate::Program> = programs.iter().take(k).collect();
                    asset.set_programs(&mut r, &set).unwrap();
                    let mut draw = |r: &mut Renderer, camera: &Camera| {
                        let mut draws = asset.draws(Visibility::Frustum);
                        es1_frame(r, &target, camera, &mut draws);
                    };
                    r.test_drain("MP1 cut");
                    r.poll().unwrap();
                    r.reset_indirect();
                    for _ in 0..10 {
                        draw(&mut r, &camera_at(0, 0.0));
                    }
                    r.test_drain("MP1 cut frames");
                    let cut = frame_times(&mut r, 10, 0);
                    if visit == 0 && k == 2 && section == "mp1" {
                        let image = es1_radiance(&mut r, exposure);
                        write(&format!("mp1-k2-{cache}-p0"), &image);
                    }
                    for t in 0..24 {
                        draw(&mut r, &camera_at(0, 0.01 * t as f32));
                    }
                    r.test_drain("MP1 motion frames");
                    let motion = frame_times(&mut r, 24, 2);
                    let slot = &mut rows[format!("k{k}")][cache];
                    if slot.is_null() {
                        *slot = json!({"cuts": [], "motion": []});
                    }
                    slot["cuts"].as_array_mut().unwrap().push(cut);
                    slot["motion"].as_array_mut().unwrap().push(motion);
                }
            }
        }
        let set = [&programs[0]];
        asset.set_programs(&mut r, &set).unwrap();
        results[section] = rows;
    }
    for (program, text) in es1_programs() {
        r.test_reload_standard(&text).unwrap();
        let mut row = json!({});
        let mut draw = |r: &mut Renderer, camera: &Camera| {
            let mut draws = asset.draws(Visibility::Frustum);
            es1_frame(r, &target, camera, &mut draws);
        };
        for section in &sections {
            match section.as_str() {
                "static" => {
                    for enabled in [false, true] {
                        r.set_indirect(es1_settings(enabled)).unwrap();
                        for pose in 0..2 {
                            let camera = camera_at(pose, 0.0);
                            r.reset_indirect();
                            for _ in 0..if enabled { 9 } else { 1 } {
                                draw(&mut r, &camera);
                            }
                            let image = es1_radiance(&mut r, exposure);
                            let cache = if enabled { "on" } else { "off" };
                            write(&format!("{program}-{cache}-p{pose}"), &image);
                        }
                    }
                }
                "pan" => {
                    // U0b's pan: frame f >= 96 is pose 0 moved 2 cm a frame in x.
                    r.set_indirect(es1_settings(true)).unwrap();
                    r.reset_indirect();
                    for _ in 0..9 {
                        draw(&mut r, &camera_at(0, 0.0));
                    }
                    for f in 97..=159 {
                        draw(&mut r, &camera_at(0, 0.02 * (f - 96) as f32));
                        if [97, 112, 128, 159].contains(&f) {
                            let image = es1_radiance(&mut r, exposure);
                            write(&format!("{program}-pan-{f:03}"), &image);
                        }
                    }
                }
                "cost" => {
                    let mut timing = json!({});
                    for enabled in [false, true] {
                        r.set_indirect(es1_settings(enabled)).unwrap();
                        r.set_indirect_profiling(false);
                        let cache = if enabled { "on" } else { "off" };
                        let mut visits = vec![];
                        for _ in 0..2 {
                            r.reset_indirect();
                            for _ in 0..10 {
                                draw(&mut r, &camera_at(0, 0.0));
                            }
                            r.test_drain("ES1 motion");
                            r.poll().unwrap();
                            for t in 0..24 {
                                draw(&mut r, &camera_at(0, 0.01 * t as f32));
                            }
                            r.test_drain("ES1 motion frames");
                            visits.push(frame_times(&mut r, 24, 2));
                        }
                        let mut cuts = vec![];
                        for _ in 0..3 {
                            r.test_drain("ES1 cut");
                            r.poll().unwrap();
                            r.reset_indirect();
                            for _ in 0..10 {
                                draw(&mut r, &camera_at(0, 0.0));
                            }
                            r.test_drain("ES1 cut frames");
                            cuts.push(frame_times(&mut r, 10, 0));
                        }
                        timing[cache] = json!({"motion": visits, "cuts": cuts});
                    }
                    row["cost"] = timing;
                }
                "abcost" | "mp1" | "mp1std" => {}
                other => panic!("unknown ES1 section {other}"),
            }
        }
        results["programs"][&program] = row;
        std::fs::write(
            out.join("quarry.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }
    results["seconds"] = json!(start.elapsed().as_secs_f64());
    std::fs::write(
        out.join("quarry.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}

/// The last `frames` GPU samples after skipping `skip`: per frame, the render span and the
/// shading pass in milliseconds.
fn frame_times(r: &mut Renderer, frames: usize, skip: usize) -> Json {
    let gpu = r.poll().unwrap().gpu;
    let rows: Vec<&GpuSample> = gpu.iter().rev().take(frames).rev().skip(skip).collect();
    json!({
        "span_ms": rows.iter().map(|g| g.render_span.as_secs_f64() * 1e3).collect::<Vec<_>>(),
        "shading_ms": rows.iter().map(|g| g.mesh_pass.as_secs_f64() * 1e3).collect::<Vec<_>>(),
    })
}
