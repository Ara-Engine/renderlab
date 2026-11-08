//! Headless SK1 example. Run through scripts/n8-run.py on the shared GPU host:
//! `cargo run --example skinned-mesh -- Fox.glb D:/.../output [quarry.rla]`
//! The source is cooked into skin-v1; both devices render the same walk with TR1.
use renderlab::animation::ClipPlayer;
use renderlab::*;
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "../viewer/src/prepared.rs"]
mod prepared;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    run(
        Path::new(args.get(1).ok_or("source GLB required")?),
        Path::new(args.get(2).ok_or("output directory required")?),
        args.get(3).map(Path::new),
    )
}

pub fn drain(r: &mut Renderer) -> Result<Events> {
    r.device().poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(60)),
    })?;
    Ok(r.poll()?)
}
fn upload<T>(
    r: &mut Renderer,
    mut f: impl FnMut(&mut Renderer) -> std::result::Result<Option<T>, renderlab::Error>,
) -> Result<T> {
    loop {
        if let Some(v) = f(r)? {
            return Ok(v);
        }
        drain(r)?;
    }
}
pub fn make_renderer(rt: bool) -> Result<Renderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!("SK1 adapter={:?} rt={rt}", adapter.get_info());
    let a = adapter.limits();
    let mut limits = visibility_limits(wgpu::Limits {
        max_buffer_size: a.max_buffer_size,
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        ..Default::default()
    });
    let mut features = RECEIVER_FEATURES
        | VISIBILITY_FEATURES
        | VISIBILITY_RASTER_FEATURES
        | VIRTUAL_FEATURES
        | wgpu::Features::TIMESTAMP_QUERY;
    if rt {
        if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
            != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        {
            return Err("explicit experimental ray-query acknowledgement required".into());
        }
        features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY | TRACED_FEATURES;
        limits = traced_limits(wgpu::Limits {
            max_blas_primitive_count: a.max_blas_primitive_count,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096,
            ..limits
        });
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: limits,
        // SAFETY: only this example's RT device, after the explicit check above.
        experimental_features: if rt {
            unsafe { wgpu::ExperimentalFeatures::enabled() }
        } else {
            Default::default()
        },
        memory_hints: MEMORY_HINTS,
        ..Default::default()
    }))?;
    assert_eq!(
        device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
        rt
    );
    let limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let mut r = if rt {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            limits,
            RendererOptions::default(),
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            include_str!("../shaders/scene.wgsl"),
        )?
    } else {
        Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            limits,
            RendererOptions::default(),
            include_str!("../shaders/scene.wgsl"),
        )?
    };
    if rt {
        r.set_primary_visibility(PrimaryVisibility::Traced)?;
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    r.set_reconstruction(Reconstruction {
        enabled: true,
        ..Default::default()
    })?;
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-0.5, -1., -0.4),
                lux: [1600., 1450., 1250.],
            }],
            environment: [90., 110., 150.],
            exposure_stops: -9.,
        },
        &Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![6., 18., 60.],
            }],
            directional_disk_angle: 0.025,
            resolution: 2048,
            ..Default::default()
        },
    )?;
    Ok(r)
}
fn capture(r: &mut Renderer, color: &wgpu::Texture, path: &Path) -> Result<()> {
    drain(r)?;
    let mut c = r.try_capture(color)?.ok_or("capture admission")?;
    drain(r)?;
    let bytes = c.try_read()?.ok_or("capture pending after drain")?;
    let mut encoder =
        png::Encoder::new(std::fs::File::create(path)?, color.width(), color.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::RelativeColorimetric);
    encoder.write_header()?.write_image_data(&bytes)?;
    Ok(())
}
fn floor(r: &mut Renderer) -> Result<(Mesh, Instances, Material)> {
    let vertices = [
        [-12., 0., -12.],
        [-12., 0., 12.],
        [12., 0., 12.],
        [12., 0., -12.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0., 1., 0.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let indices = [0, 1, 2, 0, 2, 3];
    let mesh = upload(r, |r| {
        r.try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })
    })?;
    r.set_indirect_geometry(&mesh, &vertices.map(|v| v.position), &indices)?;
    let instances = upload(r, |r| r.try_upload_instances(&[Mat4::IDENTITY]))?;
    let material = upload(r, |r| {
        r.try_create_material(&MaterialDescriptor {
            base_color: [0.32, 0.36, 0.4, 1.],
            metallic: 0.,
            roughness: 0.85,
            double_sided: true,
            ..Default::default()
        })
    })?;
    Ok((mesh, instances, material))
}
pub fn run(source: &Path, out: &Path, quarry: Option<&Path>) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let cooked = renderlab_cook::skin::cook_file(
        source,
        &out.join("cache"),
        renderlab_cook::Settings {
            normal_maps: true,
            ..Default::default()
        },
        renderlab_cook::CookLimits::default(),
    )?;
    let asset = asset::skin::decode(&std::fs::read(&cooked.path)?, asset::Limits::default())?;
    eprintln!(
        "SK1 cooked={} bindings={} clips={}",
        cooked.path.display(),
        asset.bindings.len(),
        asset.clips.len()
    );
    let mut results = vec![];
    for rt in [false, true] {
        let label = if rt { "rt" } else { "nonrt" };
        let mut r = make_renderer(rt)?;
        let mut textures = vec![];
        for t in &asset.geometry.textures {
            textures.push(upload(&mut r, |r| r.try_upload_texture(t))?);
        }
        let mut materials = vec![];
        for m in &asset.geometry.materials {
            let binding = |texture: u32, sampler| TextureBinding {
                texture: &textures[texture as usize],
                sampler,
            };
            materials.push(upload(&mut r, |r| {
                r.try_create_material(&MaterialDescriptor {
                    base_color: m.base_color,
                    metallic: m.metallic,
                    roughness: m.roughness,
                    double_sided: true,
                    base_color_texture: m.base_color_texture.map(|t| binding(t.texture, t.sampler)),
                    metallic_roughness_texture: m
                        .metallic_roughness_texture
                        .map(|t| binding(t.texture, t.sampler)),
                    normal_texture: m.normal_texture.map(|t| binding(t.texture, t.sampler)),
                    normal_scale: m.normal_texture.map_or(1., |t| t.scale),
                    ..Default::default()
                })
            })?);
        }
        let mut skins = vec![];
        let mut instances = vec![];
        let mut skin_materials = vec![];
        for (b, binding) in asset.bindings.iter().enumerate() {
            for (p, primitive) in asset.geometry.meshes[binding.mesh as usize]
                .primitives
                .iter()
                .enumerate()
            {
                let stream = asset.influences[binding.mesh as usize][p]
                    .as_ref()
                    .ok_or("missing skin stream")?;
                let skin = upload(&mut r, |r| {
                    r.try_upload_skinned_mesh(
                        primitive,
                        stream,
                        asset.skins[binding.skin as usize].joints.len() as u32,
                    )
                })?;
                skins.push((b, skin));
                skin_materials.push(primitive.material as usize);
                instances.push(upload(&mut r, |r| {
                    r.try_upload_instances(&[Mat4::from_scale(Vec3::splat(0.025))])
                })?);
            }
        }
        let (ground, mut ground_instances, ground_material) = floor(&mut r)?;
        let color = r.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("SK1 example output"),
            size: wgpu::Extent3d {
                width: 960,
                height: 540,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut player = ClipPlayer::new(usize::from(asset.clips.len() > 1));
        let fixture_camera = Camera::look_at(
            Vec3::new(3.6, 2., 4.),
            Vec3::new(0., 0.75, 0.),
            Vec3::Y,
            0.75,
            0.01,
            80.,
        )?;
        let mut quarry_scene = None;
        for scene in ["fixture", "quarry"] {
            if scene == "quarry" {
                let Some(path) = quarry else {
                    continue;
                };
                let mut q = prepared::Prepared::load_with_limits(
                    path,
                    asset::Limits {
                        max_file_bytes: 2 * 1024 * 1024 * 1024,
                        max_decoded_bytes: 2 * 1024 * 1024 * 1024,
                        max_vertices: 16_000_000,
                        max_indices: 90_000_000,
                        ..Default::default()
                    },
                    1_000_000,
                )?;
                while !q.ready() {
                    q.advance(&mut r)?;
                    drain(&mut r)?;
                }
                quarry_scene = Some(q);
            }
            r.cut_reconstruction();
            // Fixture exercises software GI rebuild + moving-caster invalidation too.
            r.set_indirect(IndirectSettings {
                enabled: scene == "fixture",
                directions: 64,
                bounce: false,
                ..Default::default()
            })?;
            let camera = if scene == "fixture" {
                fixture_camera
            } else {
                Camera::look_at(
                    Vec3::new(5.2, 3.4, 6.2),
                    Vec3::new(0., 0.6, -0.15),
                    Vec3::Y,
                    0.85,
                    0.005,
                    100.,
                )?
            };
            let mut cpu = vec![];
            let mut gpu = vec![];
            let mut wall = vec![];
            let mut palette_bytes = 0u64;
            for frame in 0..32 {
                player.seek(frame as f32 / 30.)?;
                let pose = player.sample(&asset)?;
                for (binding, skin) in &mut skins {
                    let mut palette = pose.joint_palette(&asset, *binding)?;
                    let motion =
                        Mat4::from_translation(Vec3::new(-25. + frame as f32 * 1.5, 0., 0.));
                    for m in &mut palette {
                        *m = motion * *m;
                    }
                    skin.set_joint_palette(&palette)?;
                }
                let start = Instant::now();
                let mut draws = quarry_scene
                    .as_mut()
                    .map_or_else(Vec::new, |q| q.draws(Visibility::Frustum));
                if scene == "fixture" {
                    draws.push(
                        Draw::new(&ground, &mut ground_instances).with_material(&ground_material),
                    );
                }
                for (((_, skin), inst), &material) in
                    skins.iter().zip(&mut instances).zip(&skin_materials)
                {
                    draws.push(Draw::new(skin.mesh(), inst).with_material(&materials[material]));
                }
                let report = loop {
                    if let Some(f) =
                        r.try_render_view(&color, &camera, &mut draws, ViewMode::Lit)?
                    {
                        break f;
                    }
                    r.poll()?;
                };
                drop(draws);
                let cpu_ms = start.elapsed().as_secs_f64() * 1000.;
                palette_bytes = report.uploads.skinning;
                let events = drain(&mut r)?;
                if frame >= 24 {
                    cpu.push(cpu_ms);
                    // Includes pose submission, AS updates and the deliberate drain;
                    // a synchronized headless frame, not throughput or scanout.
                    wall.push(start.elapsed().as_secs_f64() * 1000.);
                    for sample in events.gpu {
                        gpu.push(sample.render_span.as_secs_f64() * 1000.);
                    }
                }
                if [0, 8, 16, 24, 31].contains(&frame) {
                    capture(
                        &mut r,
                        &color,
                        &out.join(format!("{label}-{scene}-{frame:02}.png")),
                    )?;
                }
            }
            #[allow(unused_mut)]
            let mut skinning_gpu_ms = Vec::<f64>::new();
            #[cfg(test)]
            for (_, skin) in &skins {
                skinning_gpu_ms.extend(
                    skin.mesh()
                        .skin
                        .as_ref()
                        .unwrap()
                        .test_profile(&r.device, &r.queue),
                );
            }
            let mean = |a: &[f64]| a.iter().sum::<f64>() / a.len().max(1) as f64;
            results.push(json!({
                "path":label,"scene":scene,"size":[960,540],"frames":32,"warm_samples":cpu.len(),
                "cpu_prepare_encode_submit_mean_ms":mean(&cpu),"gpu_render_span_mean_ms":mean(&gpu),
                "synchronized_frame_wall_mean_ms":mean(&wall),
                "gpu_render_span_excludes_skinning_and_as_build":true,
                "skinning_gpu_component_samples_ms":skinning_gpu_ms,
                "skinning_upload_bytes_per_frame":palette_bytes,"tr1":true,"gi":scene=="fixture",
                "shadow_method":format!("{:?}",r.soft_shadows()),"cook":cooked.path,"build_debug":cfg!(debug_assertions)
            }));
        }
        drain(&mut r)?;
    }
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    Ok(())
}
