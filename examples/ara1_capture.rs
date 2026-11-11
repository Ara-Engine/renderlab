//! Offscreen, scene-matched measurement adapter. Renderer implementation stays untouched.
#[path = "../tools/ara1/allocation.rs"]
mod allocation;
#[path = "../tools/p-still/count.rs"]
mod p_still;
#[path = "../tools/ara1/scene.rs"]
mod scene;
use renderlab::{asset, engine::*};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Instant,
};

fn write_png(path: &Path, image: &Image) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(fs::File::create(path)?, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.rgba8)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let begin = Instant::now();
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("ara1_capture SCENE.json rt|nonrt OUTPUT".into());
    }
    let spec = scene::load(Path::new(&args[0]))?;
    let path = match args[1].as_str() {
        "rt" => RenderPath::HardwareRt,
        "nonrt" => RenderPath::NonRt,
        _ => return Err("path must be rt or nonrt".into()),
    };
    let out = PathBuf::from(&args[2]);
    fs::create_dir_all(&out)?;
    let ack = if path == RenderPath::HardwareRt {
        if env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
            != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        {
            return Err(
                "RT needs the existing explicit experimental-ray-query acknowledgement".into(),
            );
        }
        // SAFETY: the caller supplied the renderer's explicit experimental-feature acknowledgement.
        Some(unsafe { ExperimentalRayQuery::acknowledge() })
    } else {
        None
    };
    let adapter = headless_adapter()?;
    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query: ack,
            settings: Settings {
                global_illumination: !spec.startup_minimal && spec.effects.gi,
                local_shadows: renderlab::LocalShadows {
                    count: if spec.effects.local_shadows { 128 } else { 0 },
                    ..Default::default()
                },
                // The neutral matched subset has no participating medium in either engine.
                aerial_perspective: spec.effects.atmosphere,
                height_fog: spec.effects.fog.then_some(renderlab::HeightFog {
                    density: 0.01,
                    falloff: 1.0 / 90.0,
                    base_height: 0.0,
                    start_distance: 0.0,
                    albedo: [0.9; 3],
                    anisotropy: 0.3,
                }),
                volumetric_fog: spec.effects.fog.then_some(renderlab::VolumetricFog {
                    extra_density: 0.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
            texture_anisotropy: 16,
            pipeline_cache: Some(
                out.parent()
                    .ok_or("output has no parent")?
                    .join("program-cache")
                    .join(&args[1]),
            ),
            ..Default::default()
        },
    )?;
    let view = engine.create_view(&ViewDesc {
        look: ViewLook {
            reflections: renderlab::ReflectionSettings {
                enabled: spec.effects.reflections,
                ..Default::default()
            },
            exposure: Exposure::Manual {
                stops: spec.exposure_stops,
            },
            ..Default::default()
        },
        ..ViewDesc::new(spec.size[0], spec.size[1])
    })?;
    engine
        .renderer_mut()
        .set_post_settings(renderlab::PostSettings {
            tonemapper: renderlab::Tonemapper::Filmic,
            bloom: if spec.effects.post { 0.035 } else { 0.0 },
            motion_blur: false,
            vignette: 0.0,
            ..Default::default()
        })?;
    let encoded = fs::read(&spec.asset)?;
    let prepared = asset::decode(&encoded, asset::Limits::unlimited())?;
    drop(encoded);
    engine.add_asset(prepared)?;
    engine.set_sky(if spec.effects.atmosphere {
        Sky::default()
    } else {
        Sky::Uniform(spec.environment)
    })?;
    engine.set_sun(Some(Sun {
        travel: -Vec3::from_array(spec.to_light),
        illuminance: spec.sun_lux,
        angular_radius: spec.sun_radius,
    }))?;
    let lights = spec
        .lights
        .iter()
        .map(|light| {
            let candela = light
                .color
                .map(|c| c * light.lumens / (4.0 * std::f32::consts::PI));
            match &light.spot {
                Some(spot) => Light::Spot {
                    position: Vec3::from_array(light.position),
                    travel: Vec3::from_array(spot.travel),
                    candela,
                    range: Some(light.range),
                    inner: spot.inner,
                    outer: spot.outer,
                    // Point sources, matching Ara's punctual lights (LC2 added radii).
                    source_radius: 0.0,
                },
                None => Light::Point {
                    position: Vec3::from_array(light.position),
                    candela,
                    range: Some(light.range),
                    source_radius: 0.0,
                },
            }
        })
        .collect::<Vec<_>>();
    engine.set_lights(&lights)?;
    if spec.effects.alpha_panel {
        let vertices: Vec<_> = scene::PANEL_POSITIONS
            .iter()
            .enumerate()
            .map(|(i, &position)| Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]][i],
                color: [1.0; 4],
            })
            .collect();
        let mesh = engine.create_mesh(&MeshData {
            vertices: &vertices,
            indices: &[0, 1, 2, 0, 2, 3],
        })?;
        let program = engine.create_program("ARA1 coverage", include_str!("game_scene.wgsl"))?;
        let material = engine.create_material(&MaterialDesc {
            base_color: scene::PANEL_COLOR,
            metallic: 0.0,
            roughness: 0.18,
            transparency: Some(renderlab::Transparency {
                opacity: 1.0,
                ior: 1.5,
                thickness: 0.0,
            }),
            program: Some(program),
            ..Default::default()
        })?;
        engine.spawn(mesh, material, Mat4::IDENTITY)?;
    }
    if spec.startup_minimal {
        // Avoid ever allocating the normal 2^19-slot table during the startup smoke.
        engine
            .renderer_mut()
            .set_indirect(renderlab::IndirectSettings {
                slots_log2: 10,
                gather_cells: 64,
                claim_cells: 64,
                gathers: 1,
                ..Default::default()
            })?;
    }
    // Measurement-only A/B: the same binary and asset can use the frozen standard
    // program. This never changes settings or other shading stages.
    if let Some(path) = env::var_os("ARA1_STANDARD_PROGRAM") {
        engine.reload_program(engine.standard_program(), &fs::read_to_string(path)?)?;
    }
    let hash_hdr = env::var_os("ARA1_HDR_HASH").is_some();
    let memory = env::var_os("ARA1_MEMORY").is_some();
    if memory {
        allocation::write(&engine, &out.join("memory-upload.json"))?;
    }
    let ready_ms = begin.elapsed().as_secs_f64() * 1e3;
    let mut first_image_ms = None;
    let mut counts = p_still::Counts::start(&mut engine);
    let mut rows = Vec::new();
    let mut gpu = Vec::new();
    for (index, frame) in spec.frames.iter().enumerate() {
        if frame.cut {
            engine.cut_view(view);
        }
        // Ara's perspective camera uses a 0.1 m near plane and infinite reversed Z.
        // The finite far plane here has the same perspective rays, at 10 km.
        let camera = Camera::look_at(
            Vec3::from_array(frame.eye),
            Vec3::from_array(frame.target),
            Vec3::Y,
            frame.fov_y,
            0.1,
            10_000.0,
        )?;
        let start = Instant::now();
        engine.tick(1.0 / 60.0)?;
        let report = engine.render_view(view, &camera)?;
        let cpu_ms = start.elapsed().as_secs_f64() * 1e3;
        if spec.synchronous {
            engine.device().poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })?;
        }
        let service_ms = start.elapsed().as_secs_f64() * 1e3;
        let mut hdr_sha256 = None;
        if frame.capture || index == 0 {
            let image = engine.capture_view(view)?;
            if first_image_ms.is_none() {
                first_image_ms = Some(begin.elapsed().as_secs_f64() * 1e3);
                fs::write(
                    out.join("first-image.ready"),
                    b"GPU output read back; before PNG encode",
                )?;
            }
            if frame.capture {
                write_png(&out.join(format!("frame-{index:04}.png")), &image)?;
            }
            if hash_hdr {
                let mut hdr = engine
                    .renderer_mut()
                    .try_capture_hdr()?
                    .ok_or("HDR capture admission")?;
                let deadline = Instant::now() + std::time::Duration::from_secs(60);
                loop {
                    gpu.extend(engine.poll()?.gpu.into_iter().map(gpu_row));
                    if let Some(bytes) = hdr.try_read()? {
                        hdr_sha256 = Some(format!("{:x}", Sha256::digest(bytes)));
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err("HDR capture readback timeout".into());
                    }
                    std::thread::yield_now();
                }
            }
        }
        if let Some(c) = counts.as_mut() {
            c.frame(&mut engine, index, &frame.phase)?;
        }
        // P-NRT: render()'s own CPU breakdown, to attribute the CPU share of service time.
        let c = &report.cpu;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
        rows.push(json!({"index":index,"phase":frame.phase,"cut":frame.cut,
            "frame_id":report.frame.0,"cpu_submit_ms":cpu_ms,"cpu_service_ms":service_ms,"triangles":report.scene.triangles,
            "cpu_parts_ms":{"preparation":ms(c.preparation),"shadow_preparation":ms(c.shadow_preparation),
                "shadow_upload":ms(c.shadow_upload),"shadow_encode":ms(c.shadow_encode),"visibility":ms(c.visibility),
                "packing":ms(c.packing),"upload":ms(c.upload),"encode":ms(c.encode),"submit":ms(c.submit),"poll":ms(c.poll)},
            "hdr_sha256":hdr_sha256,
            "dxgi":(memory && (frame.capture || index == 0)).then(allocation::usage)}));
        for sample in engine.poll()?.gpu {
            gpu.push(gpu_row(sample));
        }
    }
    engine.capture_view(view)?;
    for sample in engine.poll()?.gpu {
        gpu.push(gpu_row(sample));
    }
    if memory {
        allocation::write(&engine, &out.join("memory.json"))?;
    }
    let mut reload = None;
    if env::var_os("ARA1_SHADER_RELOAD").is_some() {
        let frame = spec.frames.last().ok_or("missing camera")?;
        let camera = Camera::look_at(
            Vec3::from_array(frame.eye),
            Vec3::from_array(frame.target),
            Vec3::Y,
            frame.fov_y,
            0.1,
            10_000.0,
        )?;
        let before = engine.capture_view(view)?;
        let source = include_str!("../shaders/programs/standard.wgsl");
        let edit = source.replace(
            "return exposed;",
            "return exposed * vec3<f32>(0.83, 1.0, 1.0);",
        );
        if source == edit {
            return Err("shader edit did not match".into());
        }
        let start = Instant::now();
        let report = engine.reload_program(engine.standard_program(), &edit)?;
        let compile_ms = start.elapsed().as_secs_f64() * 1e3;
        engine.tick(1.0 / 60.0)?;
        engine.render_view(view, &camera)?;
        let edited = engine.capture_view(view)?;
        let image_ms = start.elapsed().as_secs_f64() * 1e3;
        write_png(&out.join("shader-edited.png"), &edited)?;
        reload = Some(
            json!({"kind":"live standard material WGSL edit", "visible_change":before.rgba8 != edited.rgba8,
            "compile_ms":compile_ms,"edit_to_readback_ms":image_ms,"report":format!("{report:?}")}),
        );
    }
    fs::write(
        out.join("engine.json"),
        serde_json::to_vec_pretty(&json!({
            "engine":"renderlab","path":args[1],"scene":spec.name,"size":spec.size,
            "adapter":engine.adapter_info().name,"debug_build":cfg!(debug_assertions),
            "scene_ready_ms":ready_ms,"startup_to_readback_ms":first_image_ms,
            "settings":format!("{:?}",engine.settings()),"frames":rows,"gpu":gpu,
            "startup_minimal":spec.startup_minimal,"effective_indirect":format!("{:?}",engine.renderer().indirect()),
            "shader_reload":reload,"synchronous":spec.synchronous,"counts":counts.map(p_still::Counts::finish),
            "timing_scope":"CPU submit is render() only; CPU service includes explicit GPU drain, excluding image readback/encoding; GPU span excludes uploads and readbacks",
            "excluded_effects":spec.notes,
        }))?,
    )?;
    Ok(())
}

fn gpu_row(g: renderlab::GpuSample) -> serde_json::Value {
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
    json!({"frame_id":g.frame.0,"span_ms":ms(g.render_span),"shading_ms":ms(g.mesh_pass),
        "primary_ms":g.receiver_pass.map(ms),"visibility_ms":g.visibility_compute.map(ms),
        "shadow_generation_ms":g.shadow_generation.map(ms),"post_ms":g.present_pass.map(ms),
        "reconstruction_ms":g.reconstruction_pass.map(ms)})
}
