//! Bounded normal-map image and paired material-cost evidence on prepared assets.
#[allow(dead_code)]
#[path = "m4/cameras.rs"]
mod cameras;
#[path = "../support/hdr_files.rs"]
mod hdr_files;
#[allow(dead_code)]
#[path = "../support/lighting_settings.rs"]
mod lighting_settings;
#[path = "../support/limits.rs"]
mod policy;
#[allow(dead_code)]
#[path = "../viewer/src/prepared.rs"]
mod prepared;
#[path = "../support/runtime_policy.rs"]
mod runtime_policy;
#[path = "../support/validation.rs"]
mod validation;
use renderlab::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn progress(r: &mut Renderer, deadline: Instant) -> Result<(), Box<dyn std::error::Error>> {
    r.poll()?;
    if Instant::now() > deadline {
        return Err("normal evidence deadline".into());
    }
    std::thread::yield_now();
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (policy, args) = policy::Policy::args(std::env::args().skip(1).collect())?;
    let (mut asset, mut unmapped, mut cameras, mut lighting) = (None, None, None, None);
    let mut output = PathBuf::from("measurements/m5-normal-integration/images");
    let mut samples = SampleCount::One;
    let mut frames = 3usize;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--asset" => asset = Some(PathBuf::from(args.next().ok_or("missing asset")?)),
            "--unmapped" => unmapped = Some(PathBuf::from(args.next().ok_or("missing unmapped")?)),
            "--cameras" => cameras = Some(PathBuf::from(args.next().ok_or("missing cameras")?)),
            "--lighting" => lighting = Some(PathBuf::from(args.next().ok_or("missing lighting")?)),
            "--output" => output = args.next().ok_or("missing output")?.into(),
            "--frames" => frames = args.next().ok_or("missing frames")?.parse()?,
            "--samples" => {
                samples = match args.next().as_deref() {
                    Some("1") => SampleCount::One,
                    Some("4") => SampleCount::Four,
                    _ => return Err("samples 1|4".into()),
                }
            }
            _ => return Err(format!("unknown {arg}").into()),
        }
    }
    if !(1..=24).contains(&frames) {
        return Err("frames must be 1..24".into());
    }
    let asset = asset.ok_or("--asset required")?;
    let cameras_path = cameras.ok_or("--cameras required")?;
    let cameras = cameras::Cameras::load(&cameras_path)?;
    let light = lighting.as_ref().map(fs::read_to_string).transpose()?;
    let lighting = light
        .as_deref()
        .map(lighting_settings::parse)
        .transpose()?
        .unwrap_or(Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-0.4, -0.7, -1.),
                lux: [3.; 3],
            }],
            environment: [0.02; 3],
            ..Default::default()
        });
    fs::create_dir_all(&output)?;
    let _validation = validation::begin(true);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: renderlab::RECEIVER_FEATURES | wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    let mut r = Renderer::new_with_options(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        runtime_policy::renderer(&policy, Limits::default()),
        RendererOptions {
            samples,
            anisotropy_clamp: 16,
        },
        include_str!("../shaders/scene.wgsl"),
    )?;
    r.set_lighting(&lighting)?;
    let pose = &cameras.frames[0];
    let color = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("normal evidence color"),
        size: wgpu::Extent3d {
            width: pose.width,
            height: pose.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let deadline = Instant::now() + Duration::from_secs(180);
    let paths = std::iter::once(asset).chain(unmapped).collect::<Vec<_>>();
    let mut prepared = paths
        .iter()
        .map(|path| {
            prepared::Prepared::load_with_limits(
                path,
                policy.prepared(),
                policy.size("viewer", "max_placement_slots", 1_000_000),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    for asset in &mut prepared {
        while !asset.ready() {
            asset.advance(&mut r)?;
            progress(&mut r, deadline)?;
        }
    }
    let mut rows = vec![];
    for pose in 0..frames {
        let camera = cameras.frames[pose.min(cameras.frames.len() - 1)].camera()?;
        let order: &[usize] = if prepared.len() == 2 {
            &[1, 0, 0, 1]
        } else {
            &[0]
        };
        for (pair, &index) in order.iter().enumerate() {
            let mut draws = prepared[index].draws(Visibility::Unculled);
            let begin = Instant::now();
            let report = loop {
                if let Some(f) = r.try_render_view(&color, &camera, &mut draws, ViewMode::Lit)? {
                    break f;
                }
                progress(&mut r, deadline)?;
            };
            let cpu = begin.elapsed();
            drop(draws);
            let mut timing = None;
            while r.stats().flights > 0 {
                for g in r.poll()?.gpu {
                    if g.frame == report.frame {
                        timing = Some(g);
                    }
                }
                if Instant::now() > deadline {
                    return Err("normal completion deadline".into());
                }
                std::thread::yield_now();
            }
            let timing = timing.ok_or("missing normal timing")?;
            rows.push(
                serde_json::json!({"pose":pose,"pair":pair,"mapped":index==0,
                "cpu_render_admission_ms":cpu.as_secs_f64()*1e3,
                "mesh_pass_ms":timing.mesh_pass.as_secs_f64()*1e3,
                "render_span_ms":timing.render_span.as_secs_f64()*1e3,
                "present_pass_ms":timing.present_pass.map(|d|d.as_secs_f64()*1e3),
                "draws":report.scene.draws,"triangles":report.scene.triangles,
                "geometry_upload_bytes":report.uploads.geometry}),
            );
            if prepared.len() == 1 || pair < 2 {
                let mut capture = loop {
                    if let Some(c) = r.try_capture_hdr()? {
                        break c;
                    }
                    progress(&mut r, deadline)?;
                };
                let info = capture.info();
                let bytes = loop {
                    r.poll()?;
                    if let Some(b) = capture.try_read()? {
                        break b;
                    }
                    if Instant::now() > deadline {
                        return Err("normal capture deadline".into());
                    }
                    std::thread::yield_now();
                };
                hdr_files::write(
                    &output.join(format!(
                        "pose-{pose:03}-{}",
                        if index == 0 { "mapped" } else { "unmapped" }
                    )),
                    info,
                    &bytes,
                    serde_json::json!({"asset":paths[index],"camera_file":cameras_path,"pose":pose,"pair":pair}),
                )?;
            }
        }
    }
    let identities = paths
        .iter()
        .map(|p| {
            Ok(serde_json::json!({"path":p,"sha256":format!("{:x}",Sha256::digest(fs::read(p)?))}))
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
        "assets":identities,"cameras":fs::read_to_string(cameras_path)?,"lighting":format!("{lighting:?}"),
        "adapter":format!("{:?}",adapter.get_info()),"samples":samples as u32,"anisotropy":16,
        "width":color.width(),"height":color.height(),"debug_build":cfg!(debug_assertions),
        "timing_scope":"paced mesh/present GPU passes; CPU admission; excludes uploads and native presentation; not pipelined throughput",
        "rows":rows,"stats":format!("{:?}",r.stats())}))?,
    )?;
    Ok(())
}
