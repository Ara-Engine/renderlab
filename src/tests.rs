use super::*;
#[path = "../support/validation.rs"]
mod validation;
use std::{
    fs,
    io::Write,
    time::{Duration, Instant},
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SOURCE: &str = include_str!("../shaders/scene.wgsl");
const INDICES: [u32; 3] = [0, 1, 2];
fn triangle(color: [f32; 3]) -> [Vertex; 3] {
    [[-1.0, -0.8, 0.0], [1.0, -0.8, 0.0], [0.0, 1.0, 0.0]]
        .map(|position| Vertex { position, color })
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
fn device(timing: bool) -> (wgpu::Device, wgpu::Queue) {
    assert!(
        !std::path::Path::new("D:/AraBuild/render-v3/quiet-window.lock").exists(),
        "quiet-window lock present"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!(
        "adapter={:?} build_profile=dev tests_sequential=true",
        adapter.get_info()
    );
    let features = if timing {
        adapter.features() & wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        ..Default::default()
    }))
    .unwrap()
}
fn renderer(timing: bool) -> Renderer {
    let (device, queue) = device(timing);
    Renderer::new(device, queue, FORMAT, Limits::default(), SOURCE).unwrap()
}
fn target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
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
    draws: &[Draw<'_>],
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
fn save(name: &str, width: u32, height: u32, rgb: &[u8]) {
    fs::create_dir_all("captures/m1").unwrap();
    let mut file = fs::File::create(format!("captures/m1/{name}.ppm")).unwrap();
    write!(file, "P6\n{width} {height}\n255\n").unwrap();
    file.write_all(rgb).unwrap();
}
fn capture(
    renderer: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &[Draw<'_>],
) -> Vec<u8> {
    render(renderer, color, camera, draws);
    renderer.test_capture(color)
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
    assert!(matrix.project_point3(Vec3::new(0.0, 0.0, -0.1)).z.abs() < 1e-6);
    assert!((matrix.project_point3(Vec3::new(0.0, 0.0, -10.0)).z - 1.0).abs() < 1e-6);
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
        (72, 12)
    );
    assert!(
        Limits {
            max_draws: 0,
            ..Limits::default()
        }
        .validate(&wgpu::Limits::default())
        .is_err()
    );
}

#[test]
#[ignore = "Vulkan GPU; sequential full-image and upload/timing acceptance"]
fn gpu_images_uploads_reload_and_timing() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let red = triangle([1.0, 0.0, 0.0]);
    let blue = triangle([0.0, 0.0, 1.0]);
    let near = upload(&mut r, &red, &INDICES);
    let far = upload(&mut r, &blue, &INDICES);
    let color = target(r.device(), 641, 359);
    let camera = camera(0.0);
    let draws = [
        Draw {
            mesh: &near,
            model: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)),
        },
        Draw {
            mesh: &far,
            model: Mat4::IDENTITY,
        },
    ];
    let baseline = capture(&mut r, &color, &camera, &draws);
    assert_eq!(baseline.len(), 641 * 359 * 3);
    let center = (179 * 641 + 320) * 3;
    assert_eq!(&baseline[center..center + 3], &[255, 0, 0]);
    save("depth-near-red-641", 641, 359, &baseline);
    let reversed = [
        Draw {
            mesh: &far,
            model: Mat4::IDENTITY,
        },
        Draw {
            mesh: &near,
            model: draws[0].model,
        },
    ];
    assert_eq!(baseline, capture(&mut r, &color, &camera, &reversed));
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
        assert_eq!(baseline, capture(&mut r, &color, &camera, &draws));
    }
    assert_eq!(r.stats().pipelines.attempts, pipeline_before.attempts + 3);
    assert_eq!(r.stats().pipelines.successes, pipeline_before.successes);
    assert_eq!(
        r.stats().pipelines.installations,
        pipeline_before.installations
    );
    r.reload(&SOURCE.replace("vec3<f32>(1.0, 1.0, 1.0)", "vec3<f32>(0.5, 1.0, 1.0)"))
        .unwrap();
    let tinted = capture(&mut r, &color, &camera, &draws);
    assert_ne!(baseline, tinted);
    assert!((127..=128).contains(&tinted[center]));
    assert_eq!(&tinted[center + 1..center + 3], &[0, 0]);
    save("reload-corrected", 641, 359, &tinted);
    r.reload(SOURCE).unwrap();
    assert_eq!(baseline, capture(&mut r, &color, &camera, &draws));
    let moved = [
        Draw {
            mesh: &near,
            model: Mat4::from_translation(Vec3::new(0.8, 0.0, 0.5)),
        },
        Draw {
            mesh: &far,
            model: Mat4::IDENTITY,
        },
    ];
    let translated = capture(&mut r, &color, &camera, &moved);
    assert_ne!(baseline, translated);
    save("model-translated", 641, 359, &translated);
    let camera_moved = capture(&mut r, &color, &self::camera(1.3), &draws);
    assert_ne!(baseline, camera_moved);
    save("camera-orbit", 641, 359, &camera_moved);
    let portrait = target(r.device(), 359, 641);
    let resized = capture(&mut r, &portrait, &camera, &draws);
    assert_eq!(
        &resized[(320 * 359 + 179) * 3..(320 * 359 + 179) * 3 + 3],
        &[255, 0, 0]
    );
    save("portrait-resize", 359, 641, &resized);
    render(&mut r, &color, &camera, &draws);
    r.test_drain("static baseline");
    let before = r.stats();
    let mut samples = 0;
    for _ in 0..300 {
        let report = render(&mut r, &color, &camera, &draws);
        assert_eq!(report.uploads, UploadBytes::default());
        samples += r.poll().unwrap().gpu.len();
    }
    r.test_drain("static final");
    let after = r.stats();
    assert_eq!(before.ownership.created(), after.ownership.created());
    assert_eq!(before.uploads, after.uploads);
    assert_eq!(before.pipelines, after.pipelines);
    assert_eq!(before.hal, after.hal);
    for i in 1..=200 {
        let draws = [
            Draw {
                mesh: &near,
                model: Mat4::from_translation(Vec3::new(i as f32 * 0.001, 0.0, 0.5)),
            },
            Draw {
                mesh: &far,
                model: Mat4::IDENTITY,
            },
        ];
        let report = render(&mut r, &color, &camera, &draws);
        assert_eq!(
            report.uploads,
            UploadBytes {
                models: 64,
                write_calls: 1,
                ..UploadBytes::default()
            }
        );
        samples += r.poll().unwrap().gpu.len();
    }
    let draws = [
        Draw {
            mesh: &near,
            model: Mat4::from_translation(Vec3::new(0.2, 0.0, 0.5)),
        },
        Draw {
            mesh: &far,
            model: Mat4::IDENTITY,
        },
    ];
    for i in 1..=200 {
        let report = render(&mut r, &color, &self::camera(i as f32 * 0.002), &draws);
        assert_eq!(
            report.uploads,
            UploadBytes {
                camera: 64,
                write_calls: 1,
                ..UploadBytes::default()
            }
        );
        samples += r.poll().unwrap().gpu.len();
    }
    assert_eq!(before.ownership.created(), r.stats().ownership.created());
    assert_eq!(before.pipelines, r.stats().pipelines);
    let adjacent = [
        Draw {
            mesh: &near,
            model: Mat4::IDENTITY,
        },
        Draw {
            mesh: &far,
            model: Mat4::from_scale(Vec3::splat(0.8)),
        },
    ];
    let report = render(&mut r, &color, &self::camera(0.4), &adjacent);
    assert_eq!(
        report.uploads,
        UploadBytes {
            models: 128,
            write_calls: 1,
            ..UploadBytes::default()
        }
    );
    r.test_drain("timing reuse final");
    samples += r.poll().unwrap().gpu.len();
    if r.stats().timing == TimingStatus::Enabled {
        assert!(samples >= 600, "too few asynchronous samples: {samples}");
    }
    assert_eq!(r.stats().timing_failures, 0);
    assert_eq!(r.stats().gpu_samples, r.stats().timed_frames);
    let cube_vertices = [
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
        color: [position[0] + 0.5, position[1] + 0.5, position[2] + 0.5],
    });
    let cube_indices = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    let cube = upload(&mut r, &cube_vertices, &cube_indices);
    let cube_draws = [Draw {
        mesh: &cube,
        model: Mat4::from_rotation_y(0.6) * Mat4::from_rotation_x(-0.3),
    }];
    let cube_image = capture(&mut r, &color, &camera, &cube_draws);
    assert_ne!(baseline, cube_image);
    assert!(
        cube_image
            .chunks_exact(3)
            .filter(|p| p[0] > 100 && p[1] > 100)
            .count()
            > 1000
    );
    save("indexed-cube", 641, 359, &cube_image);
    r.test_busy(
        &color,
        &camera,
        &near,
        &MeshData {
            vertices: &red,
            indices: &INDICES,
        },
    );
    if r.stats().timing == TimingStatus::Enabled {
        r.test_failed_map();
    }
    r.poll().unwrap();
    eprintln!(
        "static_frames=300 model_frames=200 camera_frames=200 samples={samples} stats={:?}",
        r.stats()
    );
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
    for _ in 0..20 {
        assert!(
            !render(
                &mut r,
                &color,
                &camera,
                &[Draw {
                    mesh: &mesh,
                    model: Mat4::IDENTITY
                }]
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
            &[Draw {
                mesh: &mesh,
                model: Mat4::IDENTITY
            }]
        ),
        Err(Error::Fault(_))
    ));
    let before = r.stats();
    assert!(matches!(
        r.try_render(&color, &camera, &[]),
        Err(Error::Fault(_))
    ));
    assert!(matches!(
        r.try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &INDICES
        }),
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
    fs::create_dir_all("measurements/m1-implementation").unwrap();
    let mut log = fs::File::create("measurements/m1-implementation/stress.csv").unwrap();
    writeln!(log, "frame,wave,incomplete_after_poll,hal_buffers,app_buffers,hal_textures,app_depth_textures,hal_views,hal_pipelines,buffer_bytes,texture_bytes,allocated_bytes,reserved_bytes,flights,peak_flights,retained_overlap").unwrap();
    let vertices = triangle([0.8, 0.3, 0.1]);
    let indices: Vec<u32> = INDICES.into_iter().cycle().take(2048 * 3).collect();
    let mut mesh = upload(&mut r, &vertices, &indices);
    let camera = camera(0.0);
    let mut color = target(r.device(), 513, 384);
    render(
        &mut r,
        &color,
        &camera,
        &[Draw {
            mesh: &mesh,
            model: Mat4::IDENTITY,
        }],
    );
    r.test_drain("replacement setup");
    let setup = r.stats();
    let backend_buffers =
        setup.hal.buffers - setup.ownership.get(ResourceKind::Buffer).live as isize;
    let mut overlap = [0usize; 3];
    let mut incomplete_observations = [0usize; 3];
    let mut waves = [HighWater::default(); 3];
    let mut retained_peak = 0;
    let started = Instant::now();
    for frame in 0..(64 + 3 * 704) {
        // Pipeline replacement is deliberately less frequent so compiler stalls cannot serialize every iteration.
        if frame % 32 == 0 {
            r.reload(SOURCE).unwrap();
        }
        mesh = upload(&mut r, &vertices, &indices);
        color = target(r.device(), 512 + (frame % 2) as u32, 384);
        let (incomplete, observation) = r.test_overlap();
        let retained = observation.hal.buffers
            > observation.ownership.get(ResourceKind::Buffer).live as isize + backend_buffers
            || observation.hal.texture_bytes > setup.hal.texture_bytes;
        let draw_list: Vec<_> = (0..48)
            .map(|i| Draw {
                mesh: &mesh,
                model: Mat4::from_translation(Vec3::new(0.0, 0.0, i as f32 * 0.0001)),
            })
            .collect();
        render(&mut r, &color, &camera, &draw_list);
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
    let final_image = r.test_capture(&color);
    save("stress-final", color.width(), color.height(), &final_image);
    assert!(final_image.chunks_exact(3).any(|rgb| rgb[0] > 100));
    let mut summary =
        fs::File::create("measurements/m1-implementation/stress-summary.txt").unwrap();
    writeln!(summary, "admitted_frames=2176 warmup=64 waves=3 frames_per_wave=704 draws=48 triangles_per_draw=2048 extent=512_or_513x384 samples=1\nno_per_frame_waits=true configure_calls=0 per_frame_captures=0\nincomplete_prior_draws_after_poll={incomplete_observations:?}\nretained_overlap={overlap:?}\nretained_buffer_peak_above_backend_baseline={retained_peak}\npeak_flights={peak_flights}\nwaves={waves:#?}\nsetup={setup:#?}\nfinal_drain={drained:#?}\nexternal_driver_memory=not_measured\nhal_texture_object_count=unreliable_upstream_wgpu30_vulkan_missing_creation_increment\ntiming_is_stress_not_production_perf=true").unwrap();
    eprintln!(
        "stress_waves={waves:?} overlap={overlap:?} incomplete={incomplete_observations:?} peak_flights={peak_flights}"
    );
    // A changing high-water mark within three in-flight replacement footprints is expected.
    for wave in &waves[1..] {
        assert!(
            (wave.buffers - waves[0].buffers).abs() <= 12,
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
    assert_eq!(drained.hal.render_pipelines, 1);
    assert_eq!(drained.timing_failures, 0);
    assert!(
        drained.allocator.is_some(),
        "INCONCLUSIVE: allocator report unavailable"
    );
    assert!(
        peak_flights >= 2 && overlap.iter().all(|&count| count >= 7),
        "INCONCLUSIVE: less than 1% observed incomplete/retained overlap in a wave"
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
