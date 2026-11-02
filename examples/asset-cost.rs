//! Headless prepared-asset cost observations; presentation and scanout are excluded.
#[path = "../viewer/src/prepared.rs"]
mod prepared;

use renderlab::{Events, Limits, Renderer, RendererOptions, SampleCount, Vec3, Visibility, wgpu};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn collect(events: Events, samples: &mut [Option<Duration>]) {
    for sample in events.gpu {
        samples[(sample.frame.0 - 1) as usize] = Some(sample.mesh_pass);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("expected: asset-cost <prepared-payload> <1|4> <static|motion>".into());
    }
    let samples = match args[1].as_str() {
        "1" => SampleCount::One,
        "4" => SampleCount::Four,
        _ => return Err("samples must be 1 or 4".into()),
    };
    let motion = match args[2].as_str() {
        "static" => false,
        "motion" => true,
        _ => return Err("mode must be static or motion".into()),
    };
    if Path::new("D:/AraBuild/render-v3/quiet-window.lock").exists() {
        return Err("another measurement owns the GPU quiet window".into());
    }
    let mut asset = prepared::Prepared::load(Path::new(&args[0]))?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!(
        "asset_cost adapter={:?} debug_build={} validation=false samples={} extent=1920x1080 motion={} warmup=30 frames=180 presentation=none",
        adapter.get_info(),
        cfg!(debug_assertions),
        samples as u32,
        motion
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("asset cost output"),
        size: wgpu::Extent3d {
            width: 1920,
            height: 1080,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let mut renderer = Renderer::new_with_options(
        device,
        queue,
        color.format(),
        Limits {
            max_instances: asset.max_instances(),
            ..Default::default()
        },
        RendererOptions { samples },
        include_str!("../shaders/scene.wgsl"),
    )?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while !asset.ready() {
        renderer.poll()?;
        asset.advance(&mut renderer)?;
        if Instant::now() > deadline {
            return Err("asset upload timed out".into());
        }
    }
    let mut gpu = vec![None; 180];
    let mut rows = Vec::with_capacity(180);
    let mut accepted = 0;
    let mut prepared_frame = None;
    let mut camera = asset.camera(0.4, 0.25, Vec3::ZERO, false, 1920.0 / 1080.0)?;
    let mut interval = Instant::now();
    let mut caller_cpu = Duration::ZERO;
    let mut busy_cpu = Duration::ZERO;
    let mut poll_cpu = Duration::ZERO;
    let mut pacing = Duration::ZERO;
    while accepted < 180 {
        let events = renderer.poll()?;
        poll_cpu += events.poll_cpu;
        collect(events, &mut gpu);
        let caller_start = Instant::now();
        if motion && prepared_frame != Some(accepted) {
            let phase = accepted as f32 * 0.008;
            asset.animate(phase)?;
            camera = asset.camera(0.4 + phase * 0.5, 0.25, Vec3::ZERO, false, 1920.0 / 1080.0)?;
            prepared_frame = Some(accepted);
        }
        let mut draws = asset.draws(Visibility::Frustum);
        caller_cpu += caller_start.elapsed();
        let start = Instant::now();
        let report = renderer.try_render(&color, &camera, &mut draws)?;
        let cpu = start.elapsed();
        if let Some(report) = report {
            if accepted >= 30 && !motion && report.uploads != renderlab::UploadBytes::default() {
                return Err("static frame uploaded data".into());
            }
            rows.push((
                report,
                cpu,
                caller_cpu,
                busy_cpu,
                poll_cpu,
                pacing,
                interval.elapsed(),
            ));
            accepted += 1;
            caller_cpu = Duration::ZERO;
            busy_cpu = Duration::ZERO;
            poll_cpu = Duration::ZERO;
            pacing = Duration::ZERO;
            interval = Instant::now();
        } else {
            busy_cpu += cpu;
            let start = Instant::now();
            std::thread::sleep(Duration::from_micros(100));
            pacing += start.elapsed();
        }
        if Instant::now() > deadline {
            return Err("frame deadline exceeded".into());
        }
    }
    while renderer.stats().flights != 0 {
        collect(renderer.poll()?, &mut gpu);
        if Instant::now() > deadline {
            return Err("final completion deadline exceeded".into());
        }
        std::thread::yield_now();
    }
    let stats = renderer.stats();
    if stats.events_dropped != 0 || stats.gpu_samples != 180 || stats.timing_failures != 0 {
        return Err("incomplete GPU observations".into());
    }
    println!(
        "frame,measured,render_cpu_ms,caller_cpu_ms,busy_cpu_ms,poll_ms,pacing_ms,interval_ms,gpu_ms,model_bytes,normal_bytes,texture_bytes,draws,triangles"
    );
    for (report, cpu, caller, busy, poll, pacing, interval) in rows {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let gpu = gpu[(report.frame.0 - 1) as usize].ok_or("missing GPU timestamp")?;
        println!(
            "{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{}",
            report.frame.0,
            report.frame.0 > 30,
            ms(cpu),
            ms(caller),
            ms(busy),
            ms(poll),
            ms(pacing),
            ms(interval),
            ms(gpu),
            report.uploads.models,
            report.uploads.normals,
            report.uploads.textures,
            report.scene.draws,
            report.scene.triangles
        );
    }
    eprintln!("final_stats={stats:?}");
    Ok(())
}
