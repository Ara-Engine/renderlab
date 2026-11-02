//! Offscreen scale baseline: `cargo run --release --example scene-scale -- 10000 120 wide`.
//! Use `narrow` for a mostly offscreen scene; append `--validate` for Vulkan validation.
use renderlab::{Camera, Draw, Events, Limits, Mat4, MeshData, Renderer, Vec3, Vertex, wgpu};
use std::time::{Duration, Instant};

#[path = "../support/validation.rs"]
mod validation;

const WARMUP: u64 = 20;
const WIDTH: u32 = 960;
const HEIGHT: u32 = 640;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let count = args.first().map_or(Ok(1000), |s| s.parse::<u32>())?;
    let frames = args.get(1).map_or(Ok(120), |s| s.parse::<u64>())?;
    let case = args.get(2).map_or("wide", String::as_str);
    if count == 0
        || frames <= WARMUP
        || !matches!(case, "wide" | "narrow")
        || args.len() > 4
        || args.get(3).is_some_and(|s| s != "--validate")
    {
        return Err("expected: [positive count] [frames > 20] [wide|narrow] [--validate]".into());
    }
    let validate = args.get(3).is_some_and(|s| s == "--validate");
    let _validation = validation::begin(validate);
    if std::path::Path::new("D:/AraBuild/render-v3/quiet-window.lock").exists() {
        return Err("another GPU measurement owns the quiet window".into());
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: if validate {
            wgpu::InstanceFlags::DEBUG | wgpu::InstanceFlags::VALIDATION
        } else {
            wgpu::InstanceFlags::empty()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!(
        "scene_scale version=1 adapter={:?} debug_build={} validation={validate} count={count} case={case} frames={frames} warmup={WARMUP} size={WIDTH}x{HEIGHT} samples=1 presentation=none",
        adapter.get_info(),
        cfg!(debug_assertions)
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scale target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let mut renderer = Renderer::new(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        Limits {
            max_draws: count,
            ..Limits::default()
        },
        include_str!("../shaders/scene.wgsl"),
    )?;
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
        color: position.map(|p| 0.5 + p * 0.7),
    });
    let indices = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    let mesh = renderer
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })?
        .ok_or("initial upload unexpectedly busy")?;
    let scene_start = Instant::now();
    let side = (count as f64).sqrt().ceil() as u32;
    let center = (side - 1) as f32 * 0.75;
    let draws: Vec<_> = (0..count)
        .map(|i| Draw {
            mesh: &mesh,
            model: Mat4::from_translation(Vec3::new(
                (i % side) as f32 * 1.5 - center,
                (i / side) as f32 * 1.5 - center,
                0.0,
            )),
        })
        .collect();
    let distance = if case == "wide" {
        side as f32 * 1.8
    } else {
        12.0
    };
    let camera = Camera::look_at(
        Vec3::new(0.0, 0.0, distance),
        Vec3::ZERO,
        Vec3::Y,
        50.0_f32.to_radians(),
        0.1,
        distance + 100.0,
    )?;
    eprintln!("scene_setup_ms={:.6}", millis(scene_start.elapsed()));

    let mut rows = Vec::with_capacity(frames as usize);
    let mut gpu = vec![None; frames as usize];
    let mut accepted = 0;
    let mut busy_cpu = Duration::ZERO;
    let mut poll_cpu = Duration::ZERO;
    let mut backpressure = Duration::ZERO;
    let mut interval_start = Instant::now();
    let deadline = interval_start + Duration::from_secs(180);
    while accepted < frames {
        let events = renderer.poll()?;
        poll_cpu += events.poll_cpu;
        collect_gpu(events, &mut gpu);
        let start = Instant::now();
        if let Some(report) = renderer.try_render(&color, &camera, &draws)? {
            let render_cpu = start.elapsed();
            rows.push((
                report,
                render_cpu,
                busy_cpu,
                poll_cpu,
                backpressure,
                interval_start.elapsed(),
            ));
            accepted += 1;
            busy_cpu = Duration::ZERO;
            poll_cpu = Duration::ZERO;
            backpressure = Duration::ZERO;
            interval_start = Instant::now();
        } else {
            busy_cpu += start.elapsed();
            // Harness-only pacing while the bounded renderer is full; no GPU completion wait.
            let start = Instant::now();
            std::thread::sleep(Duration::from_micros(100));
            backpressure += start.elapsed();
        }
        if Instant::now() >= deadline {
            return Err("scale run exceeded 180 seconds".into());
        }
    }
    loop {
        collect_gpu(renderer.poll()?, &mut gpu);
        if renderer.stats().flights == 0 {
            break;
        }
        if Instant::now() >= deadline {
            return Err("final collection timeout".into());
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    println!(
        "frame,measured,render_cpu_ms,prepare_ms,upload_ms,encode_ms,submit_ms,render_poll_ms,external_poll_ms,busy_cpu_ms,backpressure_sleep_ms,admission_interval_ms,gpu_mesh_ms,geometry_bytes,model_bytes,camera_bytes,writes"
    );
    for (report, render_cpu, busy, poll, sleep, interval) in rows {
        let measured = report.frame.0 > WARMUP;
        let sample = gpu[(report.frame.0 - 1) as usize].ok_or("missing GPU sample")?;
        if measured && report.uploads != renderlab::UploadBytes::default() {
            return Err("static measured frame uploaded data".into());
        }
        println!(
            "{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{}",
            report.frame.0,
            measured,
            millis(render_cpu),
            millis(report.cpu.preparation),
            millis(report.cpu.upload),
            millis(report.cpu.encode),
            millis(report.cpu.submit),
            millis(report.cpu.poll),
            millis(poll),
            millis(busy),
            millis(sleep),
            millis(interval),
            millis(sample),
            report.uploads.geometry,
            report.uploads.models,
            report.uploads.camera,
            report.uploads.write_calls
        );
    }
    let stats = renderer.stats();
    if stats.events_dropped != 0 || stats.gpu_samples != frames || stats.timing_failures != 0 {
        return Err("incomplete timing observations".into());
    }
    eprintln!("final_stats={stats:?}");
    Ok(())
}

fn collect_gpu(events: Events, samples: &mut [Option<Duration>]) {
    for sample in events.gpu {
        samples[(sample.frame.0 - 1) as usize] = Some(sample.mesh_pass);
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
