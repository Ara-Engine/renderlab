use renderlab::{
    Camera, Draw, Events, Instances, Limits, Mat4, Mesh, MeshData, Renderer, Vec3, Vertex,
    Visibility, wgpu,
};
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
    if count == 0 || frames <= WARMUP || !matches!(case, "wide" | "narrow") {
        return Err("expected: [positive count] [frames > 20] [wide|narrow] [--validate]".into());
    }
    let mut validate = false;
    let mut visibility = Visibility::Frustum;
    let mut motion = "static";
    let mut geometry = "single";
    for arg in args.iter().skip(3) {
        match arg.as_str() {
            "--validate" => validate = true,
            "--unculled" => visibility = Visibility::Unculled,
            "--frustum" => visibility = Visibility::Frustum,
            value if value.starts_with("--motion=") => motion = &value[9..],
            value if value.starts_with("--geometry=") => geometry = &value[11..],
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }
    if !matches!(motion, "static" | "camera" | "sparse" | "contiguous")
        || !matches!(geometry, "single" | "grouped" | "alternating")
    {
        return Err("expected --motion=static|camera|sparse|contiguous --geometry=single|grouped|alternating".into());
    }
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
        "scene_scale version=2 motion={motion} geometry={geometry} visibility={visibility:?} seed=0 adapter={:?} debug_build={} validation={validate} count={count} case={case} frames={frames} warmup={WARMUP} size={WIDTH}x{HEIGHT} samples=1 presentation=none",
        adapter.get_info(),
        cfg!(debug_assertions)
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: renderlab::RECEIVER_FEATURES | wgpu::Features::TIMESTAMP_QUERY,
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
            max_instances: count,
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
        color: [
            0.5 + position[0] * 0.7,
            0.5 + position[1] * 0.7,
            0.5 + position[2] * 0.7,
            1.0,
        ],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0; 2],
    });
    let indices = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    let setup_start = Instant::now();
    let mut meshes = vec![upload_mesh(&mut renderer, &vertices, &indices)?];
    if geometry != "single" {
        meshes.push(upload_mesh(&mut renderer, &vertices[..3], &[0, 1, 2])?);
    }
    let geometry_setup = setup_start.elapsed();
    let scene_start = Instant::now();
    let side = (count as f64).sqrt().ceil() as u32;
    let center = (side - 1) as f32 * 0.75;
    let base_model = |i: u32| {
        Mat4::from_translation(Vec3::new(
            (i % side) as f32 * 1.5 - center,
            (i / side) as f32 * 1.5 - center,
            0.0,
        ))
    };
    let mut collections: Vec<(usize, Instances)> = Vec::new();
    let mut instance_setup = Duration::ZERO;
    let mut add = |mesh: usize, models: Vec<Mat4>| -> Result<(), Box<dyn std::error::Error>> {
        let start = Instant::now();
        let instances = upload_instances(&mut renderer, &models)?;
        instance_setup += start.elapsed();
        collections.push((mesh, instances));
        Ok(())
    };
    match geometry {
        "single" => add(0, (0..count).map(base_model).collect())?,
        "grouped" => {
            add(0, (0..count).step_by(2).map(base_model).collect())?;
            if count > 1 {
                add(1, (1..count).step_by(2).map(base_model).collect())?;
            }
        }
        _ => {
            for i in 0..count {
                add((i % 2) as usize, vec![base_model(i)])?;
            }
        }
    }
    let distance = if case == "wide" {
        side as f32 * 1.8
    } else {
        12.0
    };
    let mut camera = Camera::look_at(
        Vec3::new(0.0, 0.0, distance),
        Vec3::ZERO,
        Vec3::Y,
        50.0_f32.to_radians(),
        0.1,
        distance + 100.0,
    )?;
    eprintln!(
        "scene_setup_ms={:.6} geometry_setup_ms={:.6} instance_upload_admission_ms={:.6} collections={} matrix_bytes={} id_capacity_bytes={}",
        millis(scene_start.elapsed()),
        millis(geometry_setup),
        millis(instance_setup),
        collections.len(),
        u64::from(count) * 64,
        u64::from(count) * 4
    );

    let mut rows = Vec::with_capacity(frames as usize);
    let mut gpu = vec![None; frames as usize];
    let mut accepted = 0;
    let mut busy_cpu = Duration::ZERO;
    let mut poll_cpu = Duration::ZERO;
    let mut backpressure = Duration::ZERO;
    let mut caller_cpu = Duration::ZERO;
    let mut prepared_frame = None;
    let mut interval_start = Instant::now();
    let deadline = interval_start + Duration::from_secs(180);
    while accepted < frames {
        let events = renderer.poll()?;
        poll_cpu += events.poll_cpu;
        collect_gpu(events, &mut gpu);
        let caller_start = Instant::now();
        if prepared_frame != Some(accepted) {
            let phase = accepted as f32 * 0.03;
            if motion == "camera" {
                let x = phase.sin() * side as f32 * 0.4;
                camera = Camera::look_at(
                    Vec3::new(x, 0.0, distance),
                    Vec3::new(x, 0.0, 0.0),
                    Vec3::Y,
                    50.0_f32.to_radians(),
                    0.1,
                    distance + 100.0,
                )?;
            } else if motion != "static" {
                for edit in 0..count.div_ceil(100) {
                    let global = if motion == "sparse" {
                        (edit * 100 + accepted as u32 % 100) % count
                    } else {
                        edit
                    };
                    let (collection, slot) = match geometry {
                        "single" => (0, global),
                        "grouped" => ((global % 2) as usize, global / 2),
                        _ => (global as usize, 0),
                    };
                    collections[collection].1.set(
                        slot as usize,
                        base_model(global)
                            * Mat4::from_translation(Vec3::new(0.0, 0.0, phase.sin() * 0.2)),
                    )?;
                }
            }
            prepared_frame = Some(accepted);
        }
        let mut draws: Vec<_> = collections
            .iter_mut()
            .map(|(mesh, instances)| {
                Draw::new(&meshes[*mesh], instances).with_visibility(visibility)
            })
            .collect();
        caller_cpu += caller_start.elapsed();
        let start = Instant::now();
        if let Some(report) = renderer.try_render(&color, &camera, &mut draws)? {
            let render_cpu = start.elapsed();
            let accounting_start = Instant::now();
            let stats = renderer.stats();
            let accounting_cpu = accounting_start.elapsed();
            rows.push((
                report,
                render_cpu,
                busy_cpu,
                poll_cpu,
                backpressure,
                interval_start.elapsed(),
                caller_cpu,
                stats,
                accounting_cpu,
            ));
            accepted += 1;
            caller_cpu = Duration::ZERO;
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
        "frame,measured,render_cpu_ms,prepare_ms,upload_ms,encode_ms,submit_ms,render_poll_ms,external_poll_ms,busy_cpu_ms,backpressure_sleep_ms,admission_interval_ms,gpu_mesh_ms,geometry_bytes,model_bytes,camera_bytes,writes,caller_cpu_ms,visibility_ms,packing_ms,collections,total_instances,visible_instances,submitted_instances,draws,triangles,cull_full_tests,cull_partial_tests,instance_creation_bytes,visible_id_bytes,model_writes,id_writes,persistent_created,pipeline_attempts,hal_buffer_bytes,allocator_allocated_bytes,accounting_cpu_ms,visibility_cache_bytes"
    );
    for (report, render_cpu, busy, poll, sleep, interval, caller, stats, accounting) in rows {
        let measured = report.frame.0 > WARMUP;
        let sample = gpu[(report.frame.0 - 1) as usize].ok_or("missing GPU sample")?;
        if measured && motion == "static" && report.uploads != renderlab::UploadBytes::default() {
            return Err("static measured frame uploaded data".into());
        }
        println!(
            "{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{:.6},{:.6},{:.6},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.6},{}",
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
            report.uploads.write_calls,
            millis(caller),
            millis(report.cpu.visibility),
            millis(report.cpu.packing),
            report.scene.collections,
            report.scene.total_instances,
            report.scene.visible_instances,
            report.scene.submitted_instances,
            report.scene.draws,
            report.scene.triangles,
            report.scene.cull_full_tests,
            report.scene.cull_partial_tests,
            report.uploads.instances,
            report.uploads.visible_ids,
            report.uploads.model_writes,
            report.uploads.visible_id_writes,
            stats.ownership.created(),
            stats.pipelines.attempts,
            stats.hal.buffer_bytes,
            stats.allocator.map_or(0, |a| a.allocated_bytes),
            millis(accounting),
            report.scene.visibility_cache_bytes
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

fn upload_mesh(
    renderer: &mut Renderer,
    vertices: &[Vertex],
    indices: &[u32],
) -> Result<Mesh, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        renderer.poll()?;
        if let Some(mesh) = renderer.try_upload_mesh(&MeshData { vertices, indices })? {
            return Ok(mesh);
        }
        if Instant::now() >= deadline {
            return Err("mesh upload admission timeout".into());
        }
        std::thread::yield_now();
    }
}
fn upload_instances(
    renderer: &mut Renderer,
    models: &[Mat4],
) -> Result<Instances, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        renderer.poll()?;
        if let Some(instances) = renderer.try_upload_instances(models)? {
            return Ok(instances);
        }
        if Instant::now() >= deadline {
            return Err("instance upload admission timeout".into());
        }
        std::thread::yield_now();
    }
}
