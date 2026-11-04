//! Bounded, window-independent M6 hard-shadow image evidence and paired feature-cost sample.
#[allow(dead_code)]
#[path = "m4/cameras.rs"]
mod cameras;
#[allow(dead_code)]
#[path = "../support/environment_controls.rs"]
mod environment_controls;
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
#[path = "../support/shadow_fixture.rs"]
mod shadow_fixture;
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
    let mut shader_path = None::<PathBuf>;
    let mut environment_path = None::<PathBuf>;
    let mut hardware = false;
    let mut experimental_ack = false;
    let mut hardware_budget = HardwareShadowOptions::default().max_estimated_bytes;
    let mut out = PathBuf::from("measurements/m6-gpu/fixture");
    let mut samples = SampleCount::One;
    let mut validate = false;
    let mut frames = 12usize;
    let mut cost = false;
    let mut shadow_resolution = 1024u32;
    let mut shadow_distance = 40.0f32;
    let mut shadow_near = 0.05f32;
    let mut motion = String::from("camera");
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--asset" => asset_path = Some(args.next().ok_or("missing asset")?.into()),
            "--cameras" => camera_path = Some(args.next().ok_or("missing cameras")?.into()),
            "--lighting" => light_path = Some(args.next().ok_or("missing lighting")?.into()),
            "--shader" => shader_path = Some(args.next().ok_or("missing shader")?.into()),
            "--environment" => {
                environment_path = Some(args.next().ok_or("missing environment")?.into())
            }
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
            "--hardware-shadows" => hardware = true,
            "--ack-experimental-wgpu-risk" => experimental_ack = true,
            "--hardware-max-estimated-bytes" => {
                hardware_budget = match args.next().ok_or("missing hardware budget")?.as_str() {
                    "none" => None,
                    value => Some(value.parse()?),
                };
            }
            "--shadow-resolution" => {
                shadow_resolution = args.next().ok_or("missing shadow resolution")?.parse()?
            }
            "--shadow-distance" => {
                shadow_distance = args.next().ok_or("missing shadow distance")?.parse()?
            }
            "--shadow-near" => shadow_near = args.next().ok_or("missing shadow near")?.parse()?,
            "--motion" => motion = args.next().ok_or("missing motion")?,
            _ => return Err(format!("unknown {arg}").into()),
        }
    }
    if hardware && !experimental_ack {
        return Err("--hardware-shadows requires --ack-experimental-wgpu-risk: wgpu experimental features may contain undefined behavior".into());
    }
    if hardware && ["point", "spot"].contains(&motion.as_str()) {
        return Err("hardware evidence supports directional shadows only".into());
    }
    if !["camera", "sun", "point", "spot", "blocker", "static"].contains(&motion.as_str()) {
        return Err("motion camera|sun|point|spot|blocker|static".into());
    }
    if frames == 0 || frames > 120 {
        return Err("frames must be 1..120".into());
    }
    if asset_path.is_some() && motion == "blocker" {
        return Err("blocker motion requires the owned fixture".into());
    }
    let shader = shader_path
        .as_deref()
        .map(fs::read_to_string)
        .transpose()?
        .unwrap_or_else(|| include_str!("../shaders/scene.wgsl").to_owned());
    let _validation = validation::begin(validate);
    fs::create_dir_all(&out)?;
    let mut fixture = shadow_fixture::Fixture::new();
    if hardware {
        // Adapt the owned fixture; prepared scans pass unchanged to library validation.
        fixture
            .lighting
            .lights
            .retain(|l| matches!(l, Light::Directional { .. }));
        for object in &mut fixture.objects {
            object.double_sided = true;
        }
    }
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
        "m6 adapter={:?} validation={validate} debug_build={} extent={width}x{height} samples={} aniso=16 cost={cost} presentation=none",
        adapter.get_info(),
        cfg!(debug_assertions),
        samples as u32
    );
    let mut requested = wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    };
    if hardware {
        if samples == SampleCount::Four
            && !adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::MULTISAMPLED_SHADING)
        {
            return Err("4x hardware shadows require sample-rate shading support".into());
        }
        if !adapter
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        {
            return Err(
                "adapter does not support experimental ray query; no fallback was selected".into(),
            );
        }
        requested.required_features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY;
        requested.required_limits.max_blas_primitive_count = 2_000_000;
        requested.required_limits.max_blas_geometry_count = 1;
        requested.required_limits.max_tlas_instance_count = 4096;
        requested
            .required_limits
            .max_acceleration_structures_per_shader_stage = 1;
        if !requested.required_limits.check_limits(&adapter.limits()) {
            return Err("hardware evidence AS device limits unsupported".into());
        }
        // SAFETY: the explicit CLI acknowledgment above accepts wgpu's experimental
        // implementation risk. This boundary belongs to the tool requesting Device.
        requested.experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&requested))?;
    let limits = runtime_policy::renderer(&policy, Limits::default());
    let options = RendererOptions {
        samples,
        anisotropy_clamp: 16,
    };
    let mut r = if hardware {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            limits,
            options,
            HardwareShadowOptions {
                max_estimated_bytes: hardware_budget,
            },
            &shader,
        )
    } else {
        Renderer::new_with_options(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            limits,
            options,
            &shader,
        )
    }?;
    let make_shadows = |lighting: &Lighting| Shadows {
        requests: lighting
            .lights
            .iter()
            .map(|light| match light {
                Light::Directional { .. } => ShadowRequest::Directional {
                    split_ends: vec![
                        shadow_distance * 0.1,
                        shadow_distance * 0.3,
                        shadow_distance,
                    ],
                },
                Light::Point { .. } => ShadowRequest::Point {
                    near: shadow_near,
                    far: shadow_distance,
                },
                Light::Spot { .. } => ShadowRequest::Spot {
                    near: shadow_near,
                    far: shadow_distance,
                },
            })
            .collect(),
        resolution: shadow_resolution,
        ..Default::default()
    };
    r.set_lighting_and_shadows(&lighting, &make_shadows(&lighting))?;
    let color = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("M6 evidence output"),
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
    if let Some(path) = &environment_path {
        let data = environment_controls::load_environment(
            path,
            asset::environment::Limits {
                max_dimension: policy.dimension("environment", "max_dimension", 4096),
                max_texels: policy.size("environment", "max_texels", 64 * 1024 * 1024) as u64,
                max_payload_bytes: policy.size(
                    "environment",
                    "max_payload_bytes",
                    128 * 1024 * 1024,
                ) as u64,
            },
        )?;
        let map = loop {
            if let Some(map) = r.try_upload_environment(&data)? {
                break map;
            }
            progress(&mut r, deadline)?;
        };
        r.set_environment(Some(&map), EnvironmentParameters::default())?;
        fs::write(
            out.join("environment.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "path": path, "generator_hash": format!("{:?}", data.metadata().generator_hash),
                "layout": format!("{:?}", data.specular_layout()),
                "parameters": "default intensity=1 yaw=0; metadata yaw retained"
            }))?,
        )?;
    }
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
    let prepared_elapsed = start.elapsed();
    let mut rows = vec![];
    for pose in 0..frames {
        let camera = if let Some(c) = &cameras {
            c.frames[if motion == "camera" {
                pose.min(c.frames.len() - 1)
            } else {
                0
            }]
            .camera()?
        } else {
            fixture.camera(if motion == "camera" { pose } else { 0 })
        };
        let lights = shadow_fixture::moved_lighting(&lighting, pose, &motion)?;
        if motion == "blocker" && asset.is_none() {
            instances[1].set(
                0,
                Mat4::from_translation(Vec3::new(pose as f32 * 0.03, 0., 0.))
                    * fixture.objects[1].transform,
            )?;
        }
        let enabled: &[bool] = if cost {
            &[false, true, true, false]
        } else {
            &[true]
        };
        for (pair, shadowed) in enabled.iter().enumerate() {
            r.set_lighting_and_shadows(
                &lights,
                &if *shadowed {
                    make_shadows(&lights)
                } else {
                    Shadows::default()
                },
            )?;
            let mode = ViewMode::Lit;
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
                if let Some(f) = r.try_render_view(&color, &camera, &mut draws, mode)? {
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
            rows.push(serde_json::json!({"pose":pose,"pair":pair,"mode":format!("{mode:?}"),"shadowed":shadowed,"motion":motion,"shadow_stats":format!("{:?}",r.shadow_stats()),"hardware_shadow_stats":format!("{:?}",r.hardware_shadow_stats()),"shadow_preparation_ms":report.cpu.shadow_preparation.as_secs_f64()*1e3,"shadow_upload_ms":report.cpu.shadow_upload.as_secs_f64()*1e3,"shadow_encode_ms":report.cpu.shadow_encode.as_secs_f64()*1e3,"shadow_generation_ms":gpu.shadow_generation.map(|d|d.as_secs_f64()*1e3),"render_span_ms":gpu.render_span.as_secs_f64()*1e3,"frame":report.frame.0,"cpu_render_admission_ms":cpu.as_secs_f64()*1e3,"mesh_pass_ms":gpu.mesh_pass.as_secs_f64()*1e3,"present_pass_ms":gpu.present_pass.map(|d|d.as_secs_f64()*1e3),"draws":report.scene.draws,"triangles":report.scene.triangles,"camera_bytes":report.uploads.camera,"light_bytes":report.uploads.lights,"lighting_uniform_bytes":report.uploads.lighting}));
            if !cost || pair <= 1 {
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
                let prefix = if *shadowed {
                    format!("pose-{pose:03}")
                } else {
                    format!("pose-{pose:03}-disabled")
                };
                hdr_files::write(
                    &out.join(prefix),
                    info,
                    &bytes,
                    serde_json::json!({"asset_path":asset_path,"camera_path":camera_path,"lighting_path":light_path,"pose":pose,"scene":"scene.json","motion":motion,"hardware_shadows":hardware,"hardware_shadow_stats":format!("{:?}",r.hardware_shadow_stats()),"shadowed":shadowed,"shadow_settings":format!("{:?}",if *shadowed {make_shadows(&lights)} else {Shadows::default()}),"shadows":format!("{:?}",r.shadow_stats()),"camera_view":camera.view().to_cols_array(),"camera_projection":camera.matrix(width,height).to_cols_array()}),
                )?;
            }
        }
    }
    fs::write(
        out.join("cost.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"adapter":format!("{:?}",adapter.get_info()),"validation":validate,"debug_build":cfg!(debug_assertions),"samples":samples as u32,"anisotropy":16,"width":width,"height":height,"presentation":"none; pass timings exclude uploads/acquire/present; CPU includes any admission retries","rows":rows,"startup_through_evidence_seconds":start.elapsed().as_secs_f64(),"startup_to_ready_seconds":prepared_elapsed.as_secs_f64(),"stats":format!("{:?}",r.stats()),"lighting":format!("{:?}",r.lighting_stats()),"shadow_limits":format!("{:?}",r.shadow_limits()),"shadow_settings":format!("{:?}",make_shadows(&lighting)),"hardware_shadows":hardware,"hardware_shadow_stats":format!("{:?}",r.hardware_shadow_stats()),"filtering":"hard visibility inseparable from mesh pass; no softness or cascade blend; optional AS builds are outside render-pass GPU span"}),
        )?,
    )?;
    eprintln!(
        "m6 evidence={} elapsed_seconds={:.3}",
        out.display(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
fn progress(r: &mut Renderer, deadline: Instant) -> Result<(), Box<dyn std::error::Error>> {
    r.poll()?;
    if Instant::now() > deadline {
        return Err("bounded M6 run deadline".into());
    }
    std::thread::yield_now();
    Ok(())
}
