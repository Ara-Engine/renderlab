//! Bounded, window-independent M5-A image evidence and paired feature-cost sample.
#[allow(dead_code)]
#[path = "m4/cameras.rs"]
mod cameras;
#[path = "../support/hdr_files.rs"]
mod hdr_files;
#[path = "../support/lighting_fixture.rs"]
mod lighting_fixture;
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
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let (policy, args) = policy::Policy::args(std::env::args().skip(1).collect())?;
    let (mut asset_path, mut camera_path, mut light_path) =
        (None::<PathBuf>, None::<PathBuf>, None::<PathBuf>);
    let mut out = PathBuf::from("measurements/m5-a/fixture");
    let mut samples = SampleCount::One;
    let mut validate = false;
    let mut frames = 12usize;
    let mut cost = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--asset" => asset_path = Some(args.next().ok_or("missing asset")?.into()),
            "--cameras" => camera_path = Some(args.next().ok_or("missing cameras")?.into()),
            "--lighting" => light_path = Some(args.next().ok_or("missing lighting")?.into()),
            "--output" => out = args.next().ok_or("missing output")?.into(),
            "--frames" => frames = args.next().ok_or("missing frames")?.parse()?,
            "--samples" => {
                samples = match args.next().as_deref() {
                    Some("1") => SampleCount::One,
                    Some("4") => SampleCount::Four,
                    _ => return Err("samples 1|4".into()),
                }
            }
            "--validate" => validate = true,
            "--cost" => cost = true,
            _ => return Err(format!("unknown {arg}").into()),
        }
    }
    if frames == 0 || frames > 120 {
        return Err("frames must be 1..120".into());
    }
    let _validation = validation::begin(validate);
    fs::create_dir_all(&out)?;
    let fixture = lighting_fixture::Fixture::new();
    let cameras = camera_path
        .as_deref()
        .map(cameras::Cameras::load)
        .transpose()?;
    let (width, height) = cameras
        .as_ref()
        .map_or((960, 640), |c| (c.frames[0].width, c.frames[0].height));
    let lighting = if let Some(path) = &light_path {
        lighting_settings::parse(&fs::read_to_string(path)?)?
    } else {
        fixture.lighting.clone()
    };
    // Export the exact authored data, or the private asset/camera/preset identities.
    let scene_record = if let Some(path) = &asset_path {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = fs::File::open(path)?;
        let mut hash = Sha256::new();
        let mut block = vec![0; 1024 * 1024];
        loop {
            let read = file.read(&mut block)?;
            if read == 0 {
                break;
            }
            hash.update(&block[..read]);
        }
        serde_json::json!({"geometry":"full_resident_prepared_asset","asset_path":path,"asset_sha256":format!("{:x}",hash.finalize()),"lighting":light_path.as_ref().map(|p|fs::read_to_string(p).unwrap()),"cameras":cameras.as_ref().map(|c|c.frames.iter().take(frames).map(|p|serde_json::json!({"eye":p.eye,"target":p.target,"up":p.up,"fov_y_radians":p.fov_y_radians,"near":p.near,"far":p.far,"width":p.width,"height":p.height})).collect::<Vec<_>>())})
    } else {
        fixture.record(width, height)
    };
    fs::write(
        out.join("scene.json"),
        serde_json::to_vec_pretty(&scene_record)?,
    )?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: if validate {
            wgpu::InstanceFlags::debugging()
        } else {
            wgpu::InstanceFlags::empty()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!(
        "m5 adapter={:?} validation={validate} debug_build={} extent={width}x{height} samples={} aniso=16 cost={cost} presentation=none",
        adapter.get_info(),
        cfg!(debug_assertions),
        samples as u32
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
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
    let color = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("M5 evidence output"),
        size: wgpu::Extent3d {
            width,
            height,
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
    let mut asset = asset_path
        .as_deref()
        .map(|p| {
            prepared::Prepared::load_with_limits(
                p,
                policy.prepared(),
                policy.size("viewer", "max_placement_slots", 1_000_000),
            )
        })
        .transpose()?;
    let mut meshes = vec![];
    let mut instances = vec![];
    let mut materials = vec![];
    if let Some(asset) = &mut asset {
        while !asset.ready() {
            asset.advance(&mut r)?;
            progress(&mut r, deadline)?;
        }
    } else {
        for o in &fixture.objects {
            loop {
                if let Some(m) = r.try_upload_mesh(&MeshData {
                    vertices: &o.vertices,
                    indices: &o.indices,
                })? {
                    meshes.push(m);
                    break;
                }
                progress(&mut r, deadline)?;
            }
            loop {
                if let Some(i) = r.try_upload_instances(&[o.transform])? {
                    instances.push(i);
                    break;
                }
                progress(&mut r, deadline)?;
            }
            loop {
                if let Some(m) = r.try_create_material(&MaterialDescriptor {
                    base_color: o.base,
                    metallic: o.metal,
                    roughness: o.rough,
                    double_sided: o.double_sided,
                    ..Default::default()
                })? {
                    materials.push(m);
                    break;
                }
                progress(&mut r, deadline)?;
            }
        }
    }
    let mut rows = vec![];
    for pose in 0..frames {
        let camera = if let Some(c) = &cameras {
            c.frames[pose.min(c.frames.len() - 1)].camera()?
        } else {
            fixture.camera(pose)
        };
        let modes: &[ViewMode] = if cost {
            &[
                ViewMode::BaseColor,
                ViewMode::Lit,
                ViewMode::Lit,
                ViewMode::BaseColor,
            ]
        } else {
            &[ViewMode::Lit]
        };
        for (pair, mode) in modes.iter().enumerate() {
            let mut draws = if let Some(asset) = &mut asset {
                asset.draws(Visibility::Frustum)
            } else {
                instances
                    .iter_mut()
                    .enumerate()
                    .map(|(i, inst)| Draw::new(&meshes[i], inst).with_material(&materials[i]))
                    .collect()
            };
            let begin = Instant::now();
            let report = loop {
                if let Some(f) = r.try_render_view(&color, &camera, &mut draws, *mode)? {
                    break f;
                }
                progress(&mut r, deadline)?;
            };
            let cpu = begin.elapsed();
            drop(draws);
            // One paired sample: each mode's asynchronous pass timings collected separately.
            let mut gpu = None;
            while r.stats().flights > 0 {
                for sample in r.poll()?.gpu {
                    if sample.frame == report.frame {
                        gpu = Some(sample);
                    }
                }
                if Instant::now() > deadline {
                    return Err("completion deadline".into());
                }
                std::thread::yield_now();
            }
            let gpu = gpu.ok_or("missing pass timing")?;
            rows.push(serde_json::json!({"pose":pose,"pair":pair,"mode":format!("{mode:?}"),"frame":report.frame.0,"cpu_render_admission_ms":cpu.as_secs_f64()*1e3,"mesh_pass_ms":gpu.mesh_pass.as_secs_f64()*1e3,"present_pass_ms":gpu.present_pass.map(|d|d.as_secs_f64()*1e3),"draws":report.scene.draws,"triangles":report.scene.triangles,"camera_bytes":report.uploads.camera,"light_bytes":report.uploads.lights,"lighting_uniform_bytes":report.uploads.lighting}));
            if *mode == ViewMode::Lit && (!cost || pair == 1) {
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
                        return Err("capture deadline".into());
                    }
                    std::thread::yield_now();
                };
                hdr_files::write(
                    &out.join(format!("pose-{pose:03}")),
                    info,
                    &bytes,
                    serde_json::json!({"asset_path":asset_path,"camera_path":camera_path,"lighting_path":light_path,"pose":pose,"scene":"scene.json"}),
                )?;
            }
        }
    }
    fs::write(
        out.join("cost.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"adapter":format!("{:?}",adapter.get_info()),"validation":validate,"debug_build":cfg!(debug_assertions),"samples":samples as u32,"anisotropy":16,"width":width,"height":height,"presentation":"none; pass timings exclude uploads/acquire/present; CPU includes any admission retries","rows":rows,"startup_through_evidence_seconds":start.elapsed().as_secs_f64(),"stats":format!("{:?}",r.stats()),"lighting":format!("{:?}",r.lighting_stats())}),
        )?,
    )?;
    eprintln!(
        "m5 evidence={} elapsed_seconds={:.3}",
        out.display(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
fn progress(r: &mut Renderer, deadline: Instant) -> Result<(), Box<dyn std::error::Error>> {
    r.poll()?;
    if Instant::now() > deadline {
        return Err("bounded M5 run deadline".into());
    }
    std::thread::yield_now();
    Ok(())
}
