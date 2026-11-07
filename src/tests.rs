use super::*;
#[path = "../support/validation.rs"]
pub(super) mod validation;
use std::{
    fs,
    io::Write,
    time::{Duration, Instant},
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SOURCE: &str = include_str!("../shaders/scene.wgsl");
const INDICES: [u32; 3] = [0, 1, 2];
fn triangle(color: [f32; 3]) -> [Vertex; 3] {
    [[-1.0, -0.8, 0.0], [1.0, -0.8, 0.0], [0.0, 1.0, 0.0]].map(|position| Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv: [0.0; 2],
        color: [color[0], color[1], color[2], 1.0],
    })
}
fn camera(x: f32) -> Camera {
    Camera::look_at(
        Vec3::new(x, 0.0, 4.0),
        Vec3::ZERO,
        Vec3::Y,
        55.0_f32.to_radians(),
        0.1,
        100.0,
    )
    .unwrap()
}
pub(super) fn device(timing: bool) -> (wgpu::Device, wgpu::Queue) {
    assert!(
        !std::path::Path::new("D:/AraBuild/render-v3/quiet-window.lock").exists(),
        "quiet-window lock present"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!(
        "adapter={:?} debug_assertions={} tests_sequential=true",
        adapter.get_info(),
        cfg!(debug_assertions)
    );
    let features = if timing {
        adapter.features() & wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    };
    request(&adapter, features)
}
/// `RENDERLAB_TEST_PRIMARY=traced` reruns the lit suites on traced primary visibility:
/// 1x renderers become hardware renderers with it selected; 4x renderers stay raster.
pub(super) fn traced_suite() -> bool {
    std::env::var("RENDERLAB_TEST_PRIMARY").as_deref() == Ok("traced")
}
/// The suite device: `features`, plus ray queries and traced binding arrays when
/// `traced_suite`, whose experimental contract the caller accepts explicitly.
fn request(adapter: &wgpu::Adapter, features: wgpu::Features) -> (wgpu::Device, wgpu::Queue) {
    let features = features | crate::RECEIVER_FEATURES;
    let descriptor = if traced_suite() {
        assert_eq!(
            std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
            Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        );
        wgpu::DeviceDescriptor {
            required_features: features
                | wgpu::Features::EXPERIMENTAL_RAY_QUERY
                | crate::TRACED_FEATURES,
            required_limits: crate::traced_limits(wgpu::Limits {
                max_blas_primitive_count: 1 << 20,
                max_blas_geometry_count: 1,
                max_tlas_instance_count: 4096,
                max_acceleration_structures_per_shader_stage: 1,
                ..Default::default()
            }),
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        }
    } else {
        wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }
    };
    pollster::block_on(adapter.request_device(&descriptor)).unwrap()
}
/// Raster renderer, or under `traced_suite` at 1x a hardware renderer with traced
/// primary visibility selected.
pub(super) fn suite_renderer(
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    options: RendererOptions,
    source: &str,
) -> Result<Renderer, Error> {
    if traced_suite() && options.samples == SampleCount::One {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            format,
            Limits::default(),
            options,
            HardwareShadowOptions::default(),
            source,
        )?;
        r.set_primary_visibility(PrimaryVisibility::Traced)?;
        return Ok(r);
    }
    Renderer::new_with_options(device, queue, format, Limits::default(), options, source)
}
fn renderer(timing: bool) -> Renderer {
    let (device, queue) = device(timing);
    Renderer::new(device, queue, FORMAT, Limits::default(), SOURCE).unwrap()
}
pub(super) fn target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen test color"),
        size: wgpu::Extent3d {
            width,
            height,
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
fn upload(renderer: &mut Renderer, vertices: &[Vertex], indices: &[u32]) -> Mesh {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(mesh) = renderer
            .try_upload_mesh(&MeshData { vertices, indices })
            .unwrap()
        {
            return mesh;
        }
        assert!(Instant::now() < deadline, "upload admission timeout");
        std::thread::yield_now();
    }
}
fn render(
    renderer: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
) -> FrameReport {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(report) = renderer.try_render(color, camera, draws).unwrap() {
            return report;
        }
        assert!(Instant::now() < deadline, "render admission timeout");
        std::thread::yield_now();
    }
}
fn evidence_dir() -> std::path::PathBuf {
    std::env::var_os("RENDERLAB_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "measurements/m3-implementation/manual".into())
}
fn compat_captures() -> std::path::PathBuf {
    evidence_dir().join("m2-captures")
}
fn save(name: &str, width: u32, height: u32, rgb: &[u8]) {
    fs::create_dir_all(compat_captures()).unwrap();
    let mut file = fs::File::create(compat_captures().join(format!("{name}.ppm"))).unwrap();
    write!(file, "P6\n{width} {height}\n255\n").unwrap();
    file.write_all(rgb).unwrap();
}
fn instances(renderer: &mut Renderer, models: &[Mat4]) -> Instances {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(instances) = renderer.try_upload_instances(models).unwrap() {
            return instances;
        }
        assert!(Instant::now() < deadline, "instances admission timeout");
        std::thread::yield_now();
    }
}
fn capture_full(
    renderer: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
) -> (Vec<u8>, Vec<u8>) {
    render(renderer, color, camera, draws);
    renderer.test_capture_full(color)
}
fn save_full(name: &str, width: u32, height: u32, image: &(Vec<u8>, Vec<u8>)) {
    let rgb: Vec<_> = image
        .0
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    save(name, width, height, &rgb);
    fs::write(compat_captures().join(format!("{name}.rgba8")), &image.0).unwrap();
    fs::write(
        compat_captures().join(format!("{name}.depth32float")),
        &image.1,
    )
    .unwrap();
}

#[test]
fn cpu_boundaries_and_projection() {
    for (eye, target, up, fov, near, far) in [
        (Vec3::ZERO, Vec3::ZERO, Vec3::Y, 1.0, 0.1, 10.0),
        (Vec3::Z, Vec3::ZERO, Vec3::Z, 1.0, 0.1, 10.0),
        (Vec3::Z, Vec3::ZERO, Vec3::Y, 0.0, 0.1, 10.0),
        (Vec3::Z, Vec3::ZERO, Vec3::Y, 1.0, 0.0, 10.0),
        (Vec3::Z, Vec3::ZERO, Vec3::Y, 1.0, 1.0, 1.0),
        (Vec3::splat(f32::NAN), Vec3::ZERO, Vec3::Y, 1.0, 0.1, 10.0),
    ] {
        assert!(Camera::look_at(eye, target, up, fov, near, far).is_err());
    }
    let camera = Camera::look_at(Vec3::ZERO, -Vec3::Z, Vec3::Y, 1.0, 0.1, 10.0).unwrap();
    let matrix = camera.matrix(640, 320);
    assert!((matrix.project_point3(Vec3::new(0.0, 0.0, -0.1)).z - 1.0).abs() < 1e-6);
    assert!(matrix.project_point3(Vec3::new(0.0, 0.0, -10.0)).z.abs() < 1e-6);
    assert_eq!(mesh::matrix_bytes(Mat4::IDENTITY).len(), 64);
    let vertices = triangle([1.0, 0.0, 0.0]);
    for indices in [&[][..], &[0, 1][..], &[0, 1, 3][..]] {
        assert!(
            MeshData {
                vertices: &vertices,
                indices
            }
            .validate(Limits::default())
            .is_err()
        );
    }
    assert!(
        MeshData {
            vertices: &vertices,
            indices: &INDICES
        }
        .validate(Limits {
            max_mesh_bytes: 83,
            ..Limits::default()
        })
        .is_err()
    );
    assert_eq!(
        MeshData {
            vertices: &vertices,
            indices: &INDICES
        }
        .validate(Limits::default())
        .unwrap(),
        (144, 12)
    );
    assert!(
        Limits {
            max_instances: 0,
            ..Limits::default()
        }
        .validate(&wgpu::Limits::default())
        .is_err()
    );
    for device_limits in [
        wgpu::Limits {
            max_storage_buffer_binding_size: 63,
            ..wgpu::Limits::default()
        },
        wgpu::Limits {
            max_buffer_size: 63,
            ..wgpu::Limits::default()
        },
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: 0,
            ..wgpu::Limits::default()
        },
        wgpu::Limits {
            max_bind_groups: 1,
            ..wgpu::Limits::default()
        },
        wgpu::Limits {
            max_vertex_buffers: 1,
            ..wgpu::Limits::default()
        },
    ] {
        assert!(
            Limits {
                max_instances: 1,
                max_mesh_bytes: 1,
                ..Limits::default()
            }
            .validate(&device_limits)
            .is_err()
        );
    }
}

#[test]
fn cpu_camera_reversed_depth_and_unchanged_xyw() {
    for (near, far) in [(0.005, 100.0), (0.1, 10.0), (1e20, 1e30), (1e-30, 1e-20)] {
        let camera = Camera::look_at(Vec3::ZERO, -Vec3::Z, Vec3::Y, 1.0, near, far).unwrap();
        let matrix = camera.matrix(1920, 1080);
        assert!(matrix.is_finite());
        assert!((matrix.project_point3(Vec3::new(0., 0., -near)).z - 1.).abs() < 2e-7);
        assert!(matrix.project_point3(Vec3::new(0., 0., -far)).z.abs() < 2e-7);
        let mut previous = f32::INFINITY;
        for step in 0..=128 {
            let distance = (f64::from(near)
                * (f64::from(far) / f64::from(near)).powf(f64::from(step) / 128.))
                as f32;
            let depth = matrix.project_point3(Vec3::new(0., 0., -distance)).z;
            assert!(
                depth < previous,
                "{near} {far} {distance} {depth} {previous}"
            );
            previous = depth;
        }
    }
    for eye in [Vec3::ZERO, Vec3::new(2., 3., 4.)] {
        let camera = Camera::look_at(
            eye,
            eye - Vec3::new(0.2, 0.3, 1.),
            Vec3::Y,
            0.78,
            0.005,
            100.,
        )
        .unwrap();
        for (width, height) in [(1920, 1080), (3, 7), (7, 3)] {
            let actual = camera.matrix(width, height).to_cols_array();
            let forward = (Mat4::perspective_rh(0.78, width as f32 / height as f32, 0.005, 100.)
                * camera.view())
            .to_cols_array();
            for i in (0..16).filter(|i| i % 4 != 2) {
                assert_eq!(actual[i].to_bits(), forward[i].to_bits(), "XYW entry {i}");
            }
        }
    }
    let matrix = Camera::look_at(Vec3::ZERO, -Vec3::Z, Vec3::Y, 1., 0.005, 100.)
        .unwrap()
        .matrix(1920, 1080);
    let front = matrix.project_point3(Vec3::new(0., 0., -20.)).z;
    let back = matrix.project_point3(Vec3::new(0., 0., -20.0001)).z;
    assert!(front > back);
    let forward = Mat4::perspective_rh(1., 1920. / 1080., 0.005, 100.);
    assert_eq!(
        forward.project_point3(Vec3::new(0., 0., -20.)).z,
        forward.project_point3(Vec3::new(0., 0., -20.0001)).z
    );
}

#[test]
fn cpu_camera_rejects_unrepresentable_depth_coefficients() {
    for (near, far) in [
        (f32::from_bits(1), f32::MAX),
        (f32::MAX / 2., f32::MAX * 0.75),
    ] {
        assert!(matches!(
            Camera::look_at(Vec3::ZERO, -Vec3::Z, Vec3::Y, 1., near, far),
            Err(Error::InvalidInput(
                "camera depth coefficients unrepresentable"
            ))
        ));
    }
    // No arbitrary minimum near plane: the smallest normal coefficient is legal.
    assert!(Camera::look_at(Vec3::ZERO, -Vec3::Z, Vec3::Y, 1., f32::MIN_POSITIVE, 1.).is_ok());
}

#[test]
#[ignore = "Vulkan GPU; close receiver separation, equal-depth order and reload"]
fn gpu_main_depth_order_and_reload() {
    let _validation = validation::begin(true);
    let mut r = renderer(false);
    let near = upload(&mut r, &triangle([1., 0., 0.]), &INDICES);
    let far = upload(&mut r, &triangle([0., 0., 1.]), &INDICES);
    let mut a = instances(&mut r, &[Mat4::from_translation(Vec3::new(0., 0., -20.))]);
    let mut b = instances(
        &mut r,
        &[Mat4::from_translation(Vec3::new(0., 0., -20.0001))],
    );
    let camera = Camera::look_at(Vec3::ZERO, -Vec3::Z, Vec3::Y, 1., 0.005, 100.).unwrap();
    let color = target(r.device(), 129, 97);
    let first = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&far, &mut b), Draw::new(&near, &mut a)],
    );
    let center = (48 * 129 + 64) * 4;
    assert_eq!(&first.0[center..center + 4], &[255, 0, 0, 255]);
    let depth = f32::from_ne_bytes(first.1[center..center + 4].try_into().unwrap());
    let n = f64::from(0.005_f32);
    let expected = n * (100. / 20. - 1.) / (100. - n);
    assert!((f64::from(depth) - expected).abs() < 1e-10);
    assert_eq!(
        first,
        capture_full(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)]
        )
    );
    assert_eq!(f32::from_ne_bytes(first.1[..4].try_into().unwrap()), 0.);
    assert!(r.reload("invalid WGSL").is_err());
    assert_eq!(
        first,
        capture_full(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&far, &mut b), Draw::new(&near, &mut a)]
        )
    );
    r.reload(&SOURCE.replace("vec3<f32>(1.0, 1.0, 1.0)", "vec3<f32>(0.5, 1.0, 1.0)"))
        .unwrap();
    let tinted = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&far, &mut b), Draw::new(&near, &mut a)],
    );
    assert_ne!(first.0, tinted.0);
    assert_eq!(first.1, tinted.1);
    r.reload(SOURCE).unwrap();
    b.set(0, Mat4::from_translation(Vec3::new(0., 0., -20.)))
        .unwrap();
    let tied = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    let reversed = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&far, &mut b), Draw::new(&near, &mut a)],
    );
    assert_eq!(&tied.0[center..center + 4], &[255, 0, 0, 255]);
    assert_eq!(&reversed.0[center..center + 4], &[0, 0, 255, 255]);
    assert_eq!(tied.1, reversed.1);
}

#[test]
#[ignore = "Vulkan GPU; sequential full RGBA/depth, upload, reload and timing acceptance"]
fn gpu_images_uploads_reload_and_timing() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let red = triangle([1.0, 0.0, 0.0]);
    let near = upload(&mut r, &red, &INDICES);
    let far = upload(&mut r, &triangle([0.0, 0.0, 1.0]), &INDICES);
    let mut a = instances(&mut r, &[Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5))]);
    let mut b = instances(&mut r, &[Mat4::IDENTITY]);
    let color = target(r.device(), 641, 359);
    let camera = camera(0.0);
    let baseline = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    assert_eq!(baseline.0.len(), 641 * 359 * 4);
    assert_eq!(baseline.1.len(), 641 * 359 * 4);
    let center = (179 * 641 + 320) * 4;
    assert_eq!(&baseline.0[center..center + 4], &[255, 0, 0, 255]);
    let depth = f32::from_ne_bytes(baseline.1[center..center + 4].try_into().unwrap());
    let expected_depth = camera
        .matrix(641, 359)
        .project_point3(Vec3::new(0.0, 0.0, 0.5))
        .z;
    assert!((depth - expected_depth).abs() < 2e-6);
    save_full("depth-near-red-641", 641, 359, &baseline);
    assert!(
        baseline
            == capture_full(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&far, &mut b), Draw::new(&near, &mut a)]
            )
    );
    assert_coplanar_order(&mut r, &color, &camera, &near, &far, &mut a, &mut b);
    let pipeline_before = r.stats().pipelines;
    let revision = r.stats().revision;
    for source in [
        "invalid WGSL".to_owned(),
        SOURCE.replace("@binding(0)", "@binding(1)"),
        SOURCE.replace(
            "@location(0) position: vec3<f32>",
            "@location(7) position: vec3<f32>",
        ),
    ] {
        assert!(matches!(
            r.reload(&source),
            Err(ReloadError::Rejected { .. })
        ));
        assert_eq!(r.stats().revision, revision);
        assert!(
            baseline
                == capture_full(
                    &mut r,
                    &color,
                    &camera,
                    &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)]
                )
        );
    }
    assert_eq!(r.stats().pipelines.attempts, pipeline_before.attempts + 3);
    assert_eq!(r.stats().pipelines.successes, pipeline_before.successes);
    assert_eq!(
        r.stats().pipelines.installations,
        pipeline_before.installations
    );
    r.reload(&SOURCE.replace("vec3<f32>(1.0, 1.0, 1.0)", "vec3<f32>(0.5, 1.0, 1.0)"))
        .unwrap();
    let tinted = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    assert_ne!(baseline.0, tinted.0);
    assert!((127..=128).contains(&tinted.0[center]));
    assert_eq!(baseline.1, tinted.1);
    save_full("reload-corrected", 641, 359, &tinted);
    r.reload(SOURCE).unwrap();
    assert!(
        baseline
            == capture_full(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)]
            )
    );
    a.set(0, Mat4::from_translation(Vec3::new(0.8, 0.0, 0.5)))
        .unwrap();
    let moved = capture_full(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    assert_ne!(baseline.0, moved.0);
    save_full("model-translated", 641, 359, &moved);
    a.set(0, Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)))
        .unwrap();
    let camera_moved = capture_full(
        &mut r,
        &color,
        &self::camera(1.3),
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    assert_ne!(baseline.0, camera_moved.0);
    save_full("camera-orbit", 641, 359, &camera_moved);
    let portrait = target(r.device(), 359, 641);
    let resized = capture_full(
        &mut r,
        &portrait,
        &camera,
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    assert_eq!(
        &resized.0[(320 * 359 + 179) * 4..(320 * 359 + 179) * 4 + 4],
        &[255, 0, 0, 255]
    );
    save_full("portrait-resize", 359, 641, &resized);
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
    );
    r.test_drain("static baseline");
    r.poll().unwrap();
    let before = r.stats();
    let mut samples = 0;
    for _ in 0..300 {
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
        );
        assert_eq!(report.uploads, UploadBytes::default());
        assert_eq!(report.cpu.packing, Duration::ZERO);
        assert_eq!(
            report.scene.cull_full_tests + report.scene.cull_partial_tests,
            0
        );
        assert_eq!(report.scene.draws, 2);
        samples += r.poll().unwrap().gpu.len();
    }
    r.test_drain("static final");
    let after = r.stats();
    assert_eq!(before.ownership.created(), after.ownership.created());
    assert_eq!(before.uploads, after.uploads);
    assert_eq!(before.pipelines, after.pipelines);
    assert_eq!(before.hal, after.hal);
    for i in 1..=200 {
        a.set(
            0,
            Mat4::from_translation(Vec3::new(i as f32 * 0.001, 0.0, 0.5)),
        )
        .unwrap();
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
        );
        assert_eq!(
            report.uploads,
            UploadBytes {
                models: 64,
                normals: 64,
                model_writes: 1,
                write_calls: 2,
                ..Default::default()
            }
        );
        assert_eq!(report.scene.cull_partial_tests, 1);
        samples += r.poll().unwrap().gpu.len();
    }
    for i in 1..=200 {
        let report = render(
            &mut r,
            &color,
            &self::camera(i as f32 * 0.002),
            &mut [Draw::new(&near, &mut a), Draw::new(&far, &mut b)],
        );
        assert_eq!(
            report.uploads,
            UploadBytes {
                camera: 64,
                write_calls: 1,
                ..Default::default()
            }
        );
        assert_eq!(report.scene.cull_full_tests, 2);
        samples += r.poll().unwrap().gpu.len();
    }
    assert_eq!(before.ownership.created(), r.stats().ownership.created());
    assert_eq!(before.pipelines, r.stats().pipelines);
    assert_adjacent_slot_write(&mut r, &color, &camera, &near);
    r.test_drain("timing reuse final");
    samples += r.poll().unwrap().gpu.len();
    if r.stats().timing == TimingStatus::Enabled {
        assert!(samples >= 600, "too few async samples: {samples}");
    }
    assert_eq!(r.stats().timing_failures, 0);
    assert_eq!(r.stats().gpu_samples, r.stats().timed_frames);
    let mut other = renderer(false);
    let mut foreign = instances(&mut other, &[Mat4::IDENTITY]);
    r.test_busy(
        &color,
        &camera,
        &near,
        &MeshData {
            vertices: &red,
            indices: &INDICES,
        },
        &mut a,
        &mut foreign,
    );
    if r.stats().timing == TimingStatus::Enabled {
        r.test_failed_map();
    }
    r.test_drain("image acceptance final");
    eprintln!(
        "static_frames=300 model_frames=200 camera_frames=200 samples={samples} stats={:?}",
        r.stats()
    );
}

fn assert_oracle(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    mesh: &Mesh,
    models: &mut Instances,
    label: &str,
) {
    r.test_reference(true);
    let oracle = capture_full(
        r,
        color,
        camera,
        &mut [Draw::new(mesh, models).with_visibility(Visibility::Unculled)],
    );
    r.test_reference(false);
    let batched = capture_full(
        r,
        color,
        camera,
        &mut [Draw::new(mesh, models).with_visibility(Visibility::Unculled)],
    );
    assert!(
        oracle == batched,
        "batched versus per-slot RGBA/depth: {label}"
    );
    let culled = capture_full(r, color, camera, &mut [Draw::new(mesh, models)]);
    assert!(
        oracle == culled,
        "Frustum versus Unculled/per-slot RGBA/depth: {label}"
    );
    save_full(label, color.width(), color.height(), &culled);
}

#[test]
#[ignore = "Vulkan GPU; conservative culling, exact full-image/depth per-slot oracle"]
fn gpu_batched_culling_oracle() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let vertices = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ]
    .map(|position| Vertex {
        position,
        color: [position[0] + 0.5, position[1] + 0.5, position[2] + 0.5, 1.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0; 2],
    });
    let indices = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    let mesh = upload(&mut r, &vertices, &indices);
    let color = target(r.device(), 641, 359);
    let mut single_cube = instances(
        &mut r,
        &[Mat4::from_rotation_y(0.6) * Mat4::from_rotation_x(-0.3)],
    );
    let cube_image = capture_full(
        &mut r,
        &color,
        &camera(0.0),
        &mut [Draw::new(&mesh, &mut single_cube)],
    );
    assert!(
        cube_image
            .0
            .chunks_exact(4)
            .filter(|p| p[0] > 100 && p[1] > 100)
            .count()
            > 1000
    );
    save_full("indexed-cube", 641, 359, &cube_image);
    let isolated_transforms = [
        (
            "oracle-isolated-mirror",
            Mat4::from_scale(Vec3::new(-1.0, 0.5, 2.0)),
        ),
        (
            "oracle-isolated-singular",
            Mat4::from_scale(Vec3::new(1.0, 1.0, 0.0)),
        ),
        (
            "oracle-isolated-shear",
            Mat4::from_cols_array(&[
                1.0, 0.0, 0.0, 0.0, 0.7, 1.0, 0.0, 0.0, 0.3, 0.4, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]),
        ),
        (
            "oracle-isolated-projective",
            Mat4::from_cols_array(&[
                1.0, 0.0, 0.0, 0.4, 0.0, 1.0, 0.0, -0.3, 0.0, 0.0, 1.0, 0.1, 0.0, 0.0, 0.0, 1.0,
            ]),
        ),
    ];
    for (label, model) in isolated_transforms {
        single_cube.set(0, model).unwrap();
        assert_oracle(&mut r, &color, &camera(0.0), &mesh, &mut single_cube, label);
        let image = r.test_capture_full(&color);
        assert!(
            image
                .1
                .chunks_exact(4)
                .any(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()) > 0.0),
            "isolated fixture must cover pixels: {label}"
        );
    }
    let mut models = Vec::new();
    for y in -8_i32..=8 {
        for x in -8..=8 {
            models.push(Mat4::from_translation(Vec3::new(
                x as f32 * 1.5,
                y as f32 * 1.5,
                ((x + y).rem_euclid(5)) as f32 * -0.4,
            )));
        }
    }
    let crosses_near_and_eye = Mat4::from_translation(Vec3::new(0.0, 0.0, 3.9));
    let behind_eye_touching_w_zero = Mat4::from_translation(Vec3::new(0.0, 0.0, 4.5));
    let special_models = [
        Mat4::from_scale(Vec3::new(-1.0, 0.5, 2.0)),
        Mat4::from_scale(Vec3::new(1.0, 0.0, 0.5)),
        Mat4::from_cols_array(&[
            1.0, 0.0, 0.0, 0.0, 0.7, 1.0, 0.0, 0.0, 0.3, 0.4, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]),
        Mat4::from_cols_array(&[
            1.0, 0.0, 0.0, 0.4, 0.0, 1.0, 0.0, -0.3, 0.0, 0.0, 1.0, 0.1, 0.0, 0.0, 0.0, 1.0,
        ]),
        crosses_near_and_eye,
        Mat4::from_translation(Vec3::new(0.0, 0.0, 4.0)),
        behind_eye_touching_w_zero,
        Mat4::from_cols_array(&[
            1e-8, 0.0, 0.0, 0.0, 0.0, 1e-8, 0.0, 0.0, 0.0, 0.0, 1e-8, 0.0, 0.0, 0.0, 0.0, 1e-8,
        ]),
    ];
    let mut special = instances(&mut r, &special_models);
    assert_oracle(
        &mut r,
        &color,
        &camera(0.0),
        &mesh,
        &mut special,
        "oracle-isolated-near-w",
    );
    let mut collection = instances(&mut r, &models);
    for (label, eye, target, fov) in [
        (
            "oracle-full",
            Vec3::new(0.0, 0.0, 40.0),
            Vec3::ZERO,
            60.0_f32,
        ),
        ("oracle-narrow", Vec3::new(0.0, 0.0, 16.0), Vec3::ZERO, 30.0),
        (
            "oracle-offscreen",
            Vec3::new(100.0, 0.0, 4.0),
            Vec3::new(100.0, 0.0, 0.0),
            55.0,
        ),
        (
            "oracle-intersections-near-w",
            Vec3::new(0.0, 0.0, 4.0),
            Vec3::ZERO,
            55.0,
        ),
    ] {
        let camera = Camera::look_at(eye, target, Vec3::Y, fov.to_radians(), 0.1, 200.0).unwrap();
        assert_oracle(&mut r, &color, &camera, &mesh, &mut collection, label);
    }
    let same_frame_edit_and_camera_cuts = [
        Vec3::ZERO,
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 3.95),
        Vec3::new(-1.0, 0.0, 0.0),
    ];
    for (i, translation) in same_frame_edit_and_camera_cuts.into_iter().enumerate() {
        collection
            .set(0, Mat4::from_translation(translation))
            .unwrap();
        assert_oracle(
            &mut r,
            &color,
            &camera(i as f32 * 0.7),
            &mesh,
            &mut collection,
            &format!("oracle-update-cut-{i}"),
        );
    }
    collection.set(0, models[0]).unwrap();
    let portrait = target(r.device(), 359, 641);
    let grid_camera = Camera::look_at(
        Vec3::new(0.0, 0.0, 20.0),
        Vec3::ZERO,
        Vec3::Y,
        55_f32.to_radians(),
        0.1,
        100.0,
    )
    .unwrap();
    assert_oracle(
        &mut r,
        &portrait,
        &grid_camera,
        &mesh,
        &mut collection,
        "oracle-portrait-resize",
    );
    let portrait_image = r.test_capture_full(&portrait);
    let depths: std::collections::HashSet<u32> = portrait_image
        .1
        .chunks_exact(4)
        .map(|p| u32::from_ne_bytes(p.try_into().unwrap()))
        .filter(|p| *p != 1_f32.to_bits())
        .collect();
    assert!(depths.len() > 4, "portrait grid must cover varied depths");
    let mut ties = instances(
        &mut r,
        &[Mat4::IDENTITY, Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))],
    );
    assert_oracle(
        &mut r,
        &color,
        &camera(0.0),
        &mesh,
        &mut ties,
        "oracle-within-collection-tie",
    );
    let first = r.test_capture_full(&color);
    ties.set(0, Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)))
        .unwrap();
    ties.set(1, Mat4::IDENTITY).unwrap();
    assert_oracle(
        &mut r,
        &color,
        &camera(0.0),
        &mesh,
        &mut ties,
        "oracle-within-collection-tie-reversed",
    );
    let reversed = r.test_capture_full(&color);
    assert_ne!(first.0, reversed.0);
    assert_eq!(first.1, reversed.1);
    let triangle = upload(&mut r, &triangle([1.0, 0.2, 0.1]), &INDICES);
    let mesh_bounds_change = render(
        &mut r,
        &color,
        &camera(0.0),
        &mut [Draw::new(&triangle, &mut collection)],
    );
    assert_eq!(
        mesh_bounds_change.scene.cull_full_tests,
        models.len() as u64
    );
    assert_oracle(
        &mut r,
        &color,
        &camera(0.0),
        &triangle,
        &mut collection,
        "oracle-mesh-bounds-change",
    );
    let mut displaced = instances(
        &mut r,
        &[Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0))],
    );
    let source = SOURCE.replace(
        "output.position = camera.view_projection * world;",
        "output.position = camera.view_projection * (world - vec4<f32>(100.0, 0.0, 0.0, 0.0));",
    );
    assert_ne!(source, SOURCE);
    r.reload(&source).unwrap();
    let shifted = capture_full(
        &mut r,
        &color,
        &camera(0.0),
        &mut [Draw::new(&triangle, &mut displaced).with_visibility(Visibility::Unculled)],
    );
    assert!(shifted.0.chunks_exact(4).any(|p| p[0] > 100));
    save_full("unculled-position-reload", 641, 359, &shifted);
    r.reload(SOURCE).unwrap();
    r.test_drain("oracle final");
}

#[test]
#[ignore = "Vulkan GPU; optional timestamps and foreign-device fault injection"]
fn gpu_disabled_timing_and_foreign_device_fault() {
    let _validation = validation::begin(true);
    let mut r = renderer(false);
    let vertices = triangle([0.2, 0.8, 0.3]);
    let mesh = upload(&mut r, &vertices, &INDICES);
    let color = target(r.device(), 128, 128);
    let camera = camera(0.0);
    let mut collection = instances(&mut r, &[Mat4::IDENTITY]);
    for _ in 0..20 {
        assert!(
            !render(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut collection)]
            )
            .timed
        );
    }
    r.test_drain("disabled timing final");
    let events = r.poll().unwrap();
    assert!(events.gpu.is_empty());
    assert!(
        events.dropped > 0,
        "bounded observations must report discarded events"
    );
    assert_eq!(r.stats().timing, TimingStatus::Unavailable);
    let (foreign, _queue) = device(false);
    let foreign_color = target(&foreign, 128, 128);
    assert!(matches!(
        r.try_render(
            &foreign_color,
            &camera,
            &mut [Draw::new(&mesh, &mut collection)]
        ),
        Err(Error::Fault(_))
    ));
    let before = r.stats();
    assert!(matches!(
        r.try_render(&color, &camera, &mut []),
        Err(Error::Fault(_))
    ));
    assert!(matches!(
        r.try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &INDICES
        }),
        Err(Error::Fault(_))
    ));
    assert!(matches!(
        r.try_upload_instances(&[Mat4::IDENTITY]),
        Err(Error::Fault(_))
    ));
    assert!(matches!(r.reload(SOURCE), Err(ReloadError::Fault(_))));
    assert!(matches!(r.poll(), Err(Error::Fault(_))));
    let after = r.stats();
    assert_eq!(before.ownership, after.ownership);
    assert_eq!(before.uploads, after.uploads);
    assert_eq!(before.submissions, after.submissions);
    assert_eq!(before.pipelines, after.pipelines);
    eprintln!(
        "foreign_device_fault=injected_and_observed no_panic=true hardware_device_loss=unverified"
    );
}

#[test]
#[ignore = "Vulkan GPU; cache, exact ID prefix writes and same-frame model/camera publication"]
fn gpu_visibility_cache_publication() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let mesh = upload(&mut r, &triangle([0.8, 0.3, 0.1]), &INDICES);
    let color = target(r.device(), 641, 359);
    let camera = camera(0.0);
    let inside = Mat4::IDENTITY;
    let outside = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
    let mut models = [inside, inside, outside];
    let mut actual = instances(&mut r, &models);
    let first = render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut actual)],
    );
    assert_eq!(first.scene.cull_full_tests, 3);
    assert_eq!(first.scene.visible_instances, 2);
    assert_eq!(first.uploads.visible_ids, 0);
    let steady = render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut actual)],
    );
    assert_eq!(steady.uploads, UploadBytes::default());
    for (i, (slot, model, visible, id_bytes)) in [
        (1, outside, 1, 0),
        (0, outside, 0, 0),
        (2, inside, 1, 4),
        (0, inside, 2, 8),
        (2, outside, 1, 0),
    ]
    .into_iter()
    .enumerate()
    {
        models[slot] = model;
        actual.set(slot, model).unwrap();
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut actual)],
        );
        assert_eq!(report.scene.cull_full_tests, 0);
        assert_eq!(report.scene.cull_partial_tests, 1);
        assert_eq!(report.scene.visible_instances, visible);
        assert_eq!(report.scene.submitted_instances, visible);
        assert_eq!(report.scene.draws, u64::from(visible > 0));
        assert_eq!(report.scene.triangles, visible);
        assert_eq!(
            report.uploads,
            UploadBytes {
                models: 64,
                normals: 64,
                model_writes: 1,
                visible_ids: id_bytes,
                visible_id_writes: u64::from(id_bytes > 0),
                write_calls: 2 + u64::from(id_bytes > 0),
                ..Default::default()
            }
        );
        let image = r.test_capture_full(&color);
        let mut expected = instances(&mut r, &models);
        r.test_reference(true);
        let reference = capture_full(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut expected).with_visibility(Visibility::Unculled)],
        );
        r.test_reference(false);
        assert!(
            image == reference,
            "same-frame dirty visibility publication step={i}"
        );
        save_full(&format!("cache-prefix-{i}"), 641, 359, &image);
    }
    let cut = Camera::look_at(
        Vec3::new(100.0, 0.0, 4.0),
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::Y,
        55.0_f32.to_radians(),
        0.1,
        100.0,
    )
    .unwrap();
    let report = render(&mut r, &color, &cut, &mut [Draw::new(&mesh, &mut actual)]);
    assert_eq!(report.scene.cull_full_tests, 3);
    assert_eq!(report.scene.cull_partial_tests, 0);
    assert_eq!(report.uploads.models + report.uploads.geometry, 0);
    assert_eq!(report.uploads.camera, 64);
    assert_eq!(report.uploads.visible_ids, 8);
    assert_eq!(report.scene.visible_instances, 2);
    let image = r.test_capture_full(&color);
    let mut expected = instances(&mut r, &models);
    r.test_reference(true);
    let reference = capture_full(
        &mut r,
        &color,
        &cut,
        &mut [Draw::new(&mesh, &mut expected).with_visibility(Visibility::Unculled)],
    );
    r.test_reference(false);
    assert!(image == reference, "same-frame camera cut publication");
    save_full("cache-camera-cut", 641, 359, &image);
    r.test_drain("cache publication final");
}

#[derive(Clone, Copy, Debug, Default)]
struct HighWater {
    buffers: isize,
    views: isize,
    pipelines: isize,
    buffer_bytes: isize,
    texture_bytes: isize,
    allocated: u64,
    reserved: u64,
}
impl HighWater {
    fn observe(&mut self, stats: &Stats) {
        self.buffers = self.buffers.max(stats.hal.buffers);
        self.views = self.views.max(stats.hal.texture_views);
        self.pipelines = self.pipelines.max(stats.hal.render_pipelines);
        self.buffer_bytes = self.buffer_bytes.max(stats.hal.buffer_bytes);
        self.texture_bytes = self.texture_bytes.max(stats.hal.texture_bytes);
        if let Some(allocator) = stats.allocator {
            self.allocated = self.allocated.max(allocator.allocated_bytes);
            self.reserved = self.reserved.max(allocator.reserved_bytes);
        }
    }
}

#[test]
#[ignore = "Vulkan GPU; 2176 replacement frames, no per-frame waits or captures"]
fn gpu_replacement_stress() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    fs::create_dir_all(evidence_dir()).unwrap();
    let mut log = fs::File::create(evidence_dir().join("stress.csv")).unwrap();
    writeln!(log, "frame,wave,incomplete_after_poll,hal_buffers,app_buffers,hal_textures,app_depth_textures,hal_views,hal_pipelines,buffer_bytes,texture_bytes,allocated_bytes,reserved_bytes,flights,peak_flights,retained_overlap").unwrap();
    let vertices = triangle([0.8, 0.3, 0.1]);
    let indices: Vec<u32> = INDICES.into_iter().cycle().take(2048 * 3).collect();
    let mut mesh = upload(&mut r, &vertices, &indices);
    let camera = camera(0.0);
    let mut color = target(r.device(), 513, 384);
    let models: Vec<_> = (0..48)
        .map(|i| Mat4::from_translation(Vec3::new(0.0, 0.0, i as f32 * 0.0001)))
        .collect();
    let mut collection = instances(&mut r, &models);
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut collection)],
    );
    r.test_drain("replacement setup");
    let setup = r.stats();
    let backend_buffers =
        setup.hal.buffers - setup.ownership.get(ResourceKind::Buffer).live as isize;
    let mut overlap = [0usize; 3];
    let mut mesh_overlap = [0usize; 3];
    let mut instance_overlap = [0usize; 3];
    let mut incomplete_observations = [0usize; 3];
    let mut waves = [HighWater::default(); 3];
    let mut retained_peak = 0;
    let started = Instant::now();
    for frame in 0..(64 + 3 * 704) {
        // Pipeline replacement is deliberately less frequent so compiler stalls cannot serialize every iteration.
        if frame % 32 == 0 {
            r.reload(SOURCE).unwrap();
        }
        let replace_mesh_this_frame = frame % 2 == 0;
        if replace_mesh_this_frame {
            mesh = upload(&mut r, &vertices, &indices);
        } else {
            collection = instances(&mut r, &models);
        }
        color = target(r.device(), 512 + (frame % 2) as u32, 384);
        let (incomplete, observation) = r.test_overlap();
        let retained = observation.hal.buffers
            > observation.ownership.get(ResourceKind::Buffer).live as isize + backend_buffers
            || observation.hal.texture_bytes > setup.hal.texture_bytes;
        render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut collection)],
        );
        let after = r.stats();
        let wave = if frame < 64 {
            None
        } else {
            Some((frame - 64) / 704)
        };
        if let Some(wave) = wave {
            waves[wave].observe(&observation);
            waves[wave].observe(&after);
            incomplete_observations[wave] += usize::from(incomplete > 0);
            overlap[wave] += usize::from(incomplete > 0 && retained);
            if replace_mesh_this_frame {
                mesh_overlap[wave] += usize::from(incomplete > 0 && retained);
            } else {
                instance_overlap[wave] += usize::from(incomplete > 0 && retained);
            }
        }
        retained_peak = retained_peak.max(
            observation.hal.buffers
                - observation.ownership.get(ResourceKind::Buffer).live as isize
                - backend_buffers,
        );
        writeln!(
            log,
            "{frame},{},{incomplete},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            wave.map_or(-1, |w| w as i32),
            observation.hal.buffers,
            observation.ownership.get(ResourceKind::Buffer).live,
            observation.hal.textures,
            observation.ownership.get(ResourceKind::Texture).live,
            observation.hal.texture_views,
            observation.hal.render_pipelines,
            observation.hal.buffer_bytes,
            observation.hal.texture_bytes,
            observation.allocator.map_or(0, |a| a.allocated_bytes),
            observation.allocator.map_or(0, |a| a.reserved_bytes),
            observation.flights,
            observation.peak_flights,
            incomplete > 0 && retained
        )
        .unwrap();
        if frame % 256 == 0 {
            eprintln!(
                "stress_progress admitted_frames={frame} elapsed_s={:.1}",
                started.elapsed().as_secs_f64()
            );
        }
    }
    let peak_flights = r.stats().peak_flights;
    r.test_drain("replacement final");
    let drained = r.stats();
    let final_image = r.test_capture_full(&color);
    save_full("stress-final", color.width(), color.height(), &final_image);
    assert!(final_image.0.chunks_exact(4).any(|rgba| rgba[0] > 100));
    let mut summary = fs::File::create(evidence_dir().join("stress-summary.txt")).unwrap();
    writeln!(summary, "admitted_frames=2176 warmup=64 waves=3 frames_per_wave=704 collections=1 instances=48 draws=1 triangles_per_instance=2048 extent=512_or_513x384 samples=1\nno_per_frame_waits=true configure_calls=0 per_frame_captures=0\nincomplete_prior_draws_after_poll={incomplete_observations:?}\nretained_overlap={overlap:?}\nretained_buffer_peak_above_backend_baseline={retained_peak}\npeak_flights={peak_flights}\nwaves={waves:#?}\nsetup={setup:#?}\nfinal_drain={drained:#?}\nexternal_driver_memory=not_measured\nhal_texture_object_count=unreliable_upstream_wgpu30_vulkan_missing_creation_increment\ntiming_is_stress_not_production_perf=true").unwrap();
    writeln!(summary, "replacement_policy=alternating_mesh_and_instances\nmesh_replacement_overlap={mesh_overlap:?}\ninstance_replacement_overlap={instance_overlap:?}").unwrap();
    eprintln!(
        "stress_waves={waves:?} overlap={overlap:?} incomplete={incomplete_observations:?} peak_flights={peak_flights}"
    );
    // A changing high-water mark within three in-flight replacement footprints is expected.
    for wave in &waves[1..] {
        assert!(
            (wave.buffers - waves[0].buffers).abs() <= 18,
            "buffer count failed to plateau"
        );
        assert!(
            (wave.views - waves[0].views).abs() <= 6,
            "view count failed to plateau"
        );
        assert!(
            (wave.pipelines - waves[0].pipelines).abs() <= 3,
            "pipeline count failed to plateau"
        );
        assert!(
            (wave.buffer_bytes - waves[0].buffer_bytes).abs() <= 512 * 1024,
            "buffer bytes failed to plateau"
        );
        assert!(
            (wave.texture_bytes - waves[0].texture_bytes).abs() <= 6 * 1024 * 1024,
            "texture bytes failed to plateau"
        );
        assert!(
            wave.allocated.abs_diff(waves[0].allocated) <= 7 * 1024 * 1024,
            "allocator bytes failed to plateau"
        );
        assert!(
            wave.reserved.abs_diff(waves[0].reserved) <= 64 * 1024 * 1024,
            "allocator reservation failed to plateau"
        );
    }
    assert_eq!(drained.hal.buffers, setup.hal.buffers);
    assert_eq!(drained.hal.texture_bytes, setup.hal.texture_bytes);
    assert_eq!(drained.hal.texture_views, setup.hal.texture_views);
    assert_eq!(drained.hal.render_pipelines, setup.hal.render_pipelines);
    assert_eq!(drained.timing_failures, 0);
    assert!(
        drained.allocator.is_some(),
        "INCONCLUSIVE: allocator report unavailable"
    );
    assert!(
        peak_flights >= 2 && overlap.iter().all(|&count| count >= 7),
        "INCONCLUSIVE: less than 1% observed incomplete/retained overlap in a wave"
    );
    assert!(
        mesh_overlap
            .iter()
            .chain(&instance_overlap)
            .all(|&count| count >= 7),
        "INCONCLUSIVE: each replacement kind must retain overlap in every wave"
    );
    // No render call follows these replacements. Each upload must flush its own staging.
    let before = r.stats();
    for _ in 0..256 {
        mesh = upload(&mut r, &vertices, &indices);
    }
    r.test_drain("upload-only replacements final");
    let after = r.stats();
    assert_eq!(after.frames, before.frames);
    assert_eq!(after.submissions, before.submissions + 256);
    assert_eq!(after.hal.buffers, before.hal.buffers);
    assert_eq!(
        after.ownership.get(ResourceKind::Buffer).live,
        before.ownership.get(ResourceKind::Buffer).live
    );
    assert_eq!(
        after.allocator.unwrap().allocated_bytes,
        before.allocator.unwrap().allocated_bytes
    );
    assert_eq!(mesh.index_count, indices.len() as u32);
    writeln!(
        summary,
        "upload_only_replacements=256 final={after:#?}\nresult=passed"
    )
    .unwrap();
}

fn assert_coplanar_order(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    near: &Mesh,
    far: &Mesh,
    a: &mut Instances,
    b: &mut Instances,
) {
    let center = ((color.height() / 2 * color.width() + color.width() / 2) * 4) as usize;
    b.set(0, Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)))
        .unwrap();
    let tied = capture_full(
        r,
        color,
        camera,
        &mut [Draw::new(near, a), Draw::new(far, b)],
    );
    assert_eq!(&tied.0[center..center + 4], &[255, 0, 0, 255]);
    let reversed = capture_full(
        r,
        color,
        camera,
        &mut [Draw::new(far, b), Draw::new(near, a)],
    );
    assert_eq!(&reversed.0[center..center + 4], &[0, 0, 255, 255]);
    assert_eq!(tied.1, reversed.1);
    b.set(0, Mat4::IDENTITY).unwrap();
}

fn assert_adjacent_slot_write(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    mesh: &Mesh,
) {
    let mut adjacent = instances(r, &[Mat4::IDENTITY; 4]);
    render(r, color, camera, &mut [Draw::new(mesh, &mut adjacent)]);
    adjacent.set(1, Mat4::from_scale(Vec3::splat(0.8))).unwrap();
    adjacent.set(2, Mat4::from_scale(Vec3::splat(0.7))).unwrap();
    let report = render(r, color, camera, &mut [Draw::new(mesh, &mut adjacent)]);
    assert_eq!(
        report.uploads,
        UploadBytes {
            models: 128,
            normals: 128,
            model_writes: 1,
            write_calls: 2,
            ..Default::default()
        }
    );
}

#[path = "material_tests.rs"]
mod materials;

#[path = "virtual_gpu_tests.rs"]
mod virtual_pages;

#[path = "lighting_tests.rs"]
mod lighting_tests;

#[path = "shadow_tests.rs"]
mod shadow_tests;

#[path = "hardware_shadow_tests.rs"]
mod hardware_shadow_tests;

#[path = "normal_map_tests.rs"]
mod normal_maps;

#[path = "environment_tests.rs"]
mod environment_tests;
