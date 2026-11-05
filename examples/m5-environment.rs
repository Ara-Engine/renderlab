//! Prepared RLE1 environment images and bounded on/off feature-cost observations.
#[allow(dead_code)]
#[path = "m4/cameras.rs"]
mod cameras;
#[allow(dead_code)]
#[path = "../support/environment_controls.rs"]
mod environment_controls;
#[path = "../support/hdr_files.rs"]
mod hdr_files;
#[allow(dead_code)]
#[path = "../support/lighting_fixture.rs"]
mod lighting_fixture;
#[allow(dead_code)]
#[path = "../support/lighting_controls.rs"]
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
        return Err("environment evidence deadline".into());
    }
    std::thread::yield_now();
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (policy, args) = policy::Policy::args(std::env::args().skip(1).collect())?;
    let (mut asset, mut environment, mut cameras, mut lighting) = (None, None, None, None);
    let mut parameters = EnvironmentParameters::default();
    let mut compare_constant = false;
    let mut output = PathBuf::from("measurements/m5-environment-runtime/images");
    let mut samples = SampleCount::One;
    let mut frames = 3usize;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--asset" => asset = Some(PathBuf::from(args.next().ok_or("missing asset")?)),
            "--environment" => {
                environment = Some(PathBuf::from(args.next().ok_or("missing RLE1")?))
            }
            "--yaw" => parameters.rotation_y_radians = args.next().ok_or("missing yaw")?.parse()?,
            "--intensity" => {
                parameters.intensity = args.next().ok_or("missing intensity")?.parse()?
            }
            "--compare-constant" => compare_constant = true,
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
    fs::create_dir_all(&output)?;
    let fixture = lighting_fixture::Fixture::new();
    let (fixture_asset, fixture_cameras) = if asset.is_none() || cameras.is_none() {
        let (a, c) = write_fixture(&output, &fixture, frames)?;
        (Some(a), Some(c))
    } else {
        (None, None)
    };
    let asset = asset.or(fixture_asset).unwrap();
    let environment = environment.ok_or("--environment required")?;
    let cameras_path = cameras.or(fixture_cameras).unwrap();
    let cameras = cameras::Cameras::load(&cameras_path)?;
    let light = lighting.as_ref().map(fs::read_to_string).transpose()?;
    let lighting = light
        .as_deref()
        .map(lighting_settings::parse_with_shadows)
        .transpose()?
        .unwrap_or((
            Lighting {
                exposure_stops: -14.,
                ..fixture.lighting.clone()
            },
            Shadows::default(),
        ));
    fs::create_dir_all(&output)?;
    let _validation = validation::begin(true);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: renderlab::RECEIVER_FEATURES
            | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
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
    r.set_lighting_and_shadows(&lighting.0, &lighting.1)?;
    let environment_limits = asset::environment::Limits {
        max_dimension: policy.dimension("environment", "max_dimension", 4096),
        max_texels: policy.size("environment", "max_texels", 64 * 1024 * 1024) as u64,
        max_payload_bytes: policy.size("environment", "max_payload_bytes", 768 * 1024 * 1024)
            as u64,
    };
    r.set_environment_limits(EnvironmentLimits {
        max_dimension: policy.dimension("environment", "max_dimension", 2048),
        max_map_bytes: policy.size("environment", "max_map_bytes", 128 * 1024 * 1024) as u64,
        max_owned_bytes: policy.size("environment", "max_owned_bytes", 384 * 1024 * 1024) as u64,
    })?;
    let decode_start = Instant::now();
    let data = environment_controls::load_environment(&environment, environment_limits)?;
    let decode_ms = decode_start.elapsed().as_secs_f64() * 1e3;
    let upload_start = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(30);
    let map = loop {
        if let Some(map) = r.try_upload_environment(&data)? {
            break map;
        }
        progress(&mut r, deadline)?;
    };
    let upload_ms = upload_start.elapsed().as_secs_f64() * 1e3;
    let environment_upload_cpu = format!("{:?}", r.stats().upload_cpu);
    let prepared_bytes = data.pixel_bytes();
    let packed_bytes = data.texel_count() * 16;
    drop(data);
    r.set_environment(Some(&map), parameters)?;
    let pose = &cameras.frames[0];
    let color = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("environment evidence color"),
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
    let paths = [asset];
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
    for selected in [false, true] {
        r.set_environment(if selected { Some(&map) } else { None }, parameters)?;
        let mut draws = prepared[0].draws(Visibility::Unculled);
        loop {
            if r.try_render_view(
                &color,
                &cameras.frames[0].camera()?,
                &mut draws,
                ViewMode::Lit,
            )?
            .is_some()
            {
                break;
            }
            progress(&mut r, deadline)?;
        }
        while r.stats().flights > 0 {
            progress(&mut r, deadline)?;
        }
    }
    let mut rows = vec![];
    for pose in 0..frames {
        let camera = cameras.frames[pose.min(cameras.frames.len() - 1)].camera()?;
        let order: &[usize] = if compare_constant {
            &[1, 0, 0, 1]
        } else {
            &[0]
        };
        for (pair, &index) in order.iter().enumerate() {
            r.set_environment(if index == 0 { Some(&map) } else { None }, parameters)?;
            let mut draws = prepared[0].draws(Visibility::Unculled);
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
                    return Err("environment completion deadline".into());
                }
                std::thread::yield_now();
            }
            let timing = timing.ok_or("missing environment timing")?;
            rows.push(
                serde_json::json!({"pose":pose,"pair":pair,"environment_map":index==0,
                "cpu_render_admission_ms":cpu.as_secs_f64()*1e3,
                "mesh_pass_ms":timing.mesh_pass.as_secs_f64()*1e3,
                "render_span_ms":timing.render_span.as_secs_f64()*1e3,
                "present_pass_ms":timing.present_pass.map(|d|d.as_secs_f64()*1e3),
                "draws":report.scene.draws,"triangles":report.scene.triangles,
                "geometry_upload_bytes":report.uploads.geometry,"environment_upload_bytes":report.uploads.environment,"lighting_upload_bytes":report.uploads.lighting,"light_upload_bytes":report.uploads.lights,"shadow_upload_bytes":report.uploads.shadows}),
            );
            if !compare_constant || pair < 2 {
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
                        return Err("environment capture deadline".into());
                    }
                    std::thread::yield_now();
                };
                hdr_files::write(
                    &output.join(format!(
                        "pose-{pose:03}-{}",
                        if index == 0 {
                            "environment"
                        } else {
                            "constant"
                        }
                    )),
                    info,
                    &bytes,
                    serde_json::json!({"asset":paths[0],"environment":environment,"parameters":format!("{parameters:?}"),"camera_file":cameras_path,"pose":pose,"pair":pair}),
                )?;
            }
        }
    }
    let identities = paths
        .iter()
        .map(|p| Ok(serde_json::json!({"path":p,"sha256":file_hash(p)?})))
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
        "environment":environment,"environment_sha256":file_hash(&environment)?,"environment_parameters":format!("{parameters:?}"),
        "environment_decode_ms":decode_ms,"environment_upload_ms":upload_ms,"environment_upload_cpu":environment_upload_cpu,"prepared_rgb_bytes":prepared_bytes,"packed_rgba_bytes":packed_bytes,
        "assets":identities,"cameras":fs::read_to_string(cameras_path)?,"lighting":format!("{lighting:?}"),
        "adapter":format!("{:?}",adapter.get_info()),"samples":samples as u32,"anisotropy":16,
        "width":color.width(),"height":color.height(),"debug_build":cfg!(debug_assertions),
        "timing_scope":"paced mesh/present GPU passes; CPU admission; excludes uploads and native presentation; not pipelined throughput",
        "rows":rows,"stats":format!("{:?}",r.stats())}))?,
    )?;
    Ok(())
}

fn file_hash(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut bytes = [0; 65536];
    let mut hash = Sha256::new();
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn write_fixture(
    output: &std::path::Path,
    fixture: &lighting_fixture::Fixture,
    frames: usize,
) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let mut source = asset::Asset::default();
    for (i, object) in fixture.objects.iter().enumerate() {
        source.meshes.push(asset::Mesh {
            primitives: vec![asset::Primitive {
                vertices: object.vertices.clone(),
                tangents: None,
                indices: object.indices.clone(),
                material: i as u32,
            }],
        });
        source.placements.push(asset::Placement {
            mesh: i as u32,
            transform: object.transform.to_cols_array_2d(),
        });
        source.materials.push(asset::Material {
            base_color: object.base,
            metallic: object.metal,
            roughness: object.rough,
            double_sided: object.double_sided,
            ..Default::default()
        });
    }
    let asset_path = output.join("material-fixture.rla");
    fs::write(
        &asset_path,
        asset::encode(&source, asset::Limits::default())?,
    )?;
    let cameras_path = output.join("material-fixture-cameras.json");
    let poses=(0..frames).map(|i|serde_json::json!({"eye":(fixture.eye+Vec3::new(i as f32*0.04,0.,0.)).to_array(),"target":fixture.target.to_array(),"up":[0,1,0],"fov_y_radians":0.78,"near":0.05,"far":50.,"width":960,"height":640,"frame":i,"phase":"short_environment_material_motion"})).collect::<Vec<_>>();
    fs::write(
        &cameras_path,
        serde_json::to_vec_pretty(&serde_json::json!({"frames":poses}))?,
    )?;
    fs::write(
        output.join("material-fixture.json"),
        serde_json::to_vec_pretty(&fixture.record(960, 640))?,
    )?;
    Ok((asset_path, cameras_path))
}
