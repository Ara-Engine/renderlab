//! Offscreen, scene-matched measurement adapter. Renderer implementation stays untouched.
#[path = "../tools/ara1/scene.rs"]
mod scene;
use renderlab::{asset, engine::*};
use serde_json::json;
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
                global_illumination: !spec.startup_minimal,
                exposure: Exposure::Manual {
                    stops: spec.exposure_stops,
                },
                // The neutral matched subset has no participating medium in either engine.
                aerial_perspective: false,
                height_fog: None,
                volumetric_fog: None,
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
    engine.resize(spec.size[0], spec.size[1])?;
    engine
        .renderer_mut()
        .set_post_settings(renderlab::PostSettings {
            tonemapper: renderlab::Tonemapper::Filmic,
            bloom: 0.0,
            motion_blur: false,
            vignette: 0.0,
            ..Default::default()
        })?;
    let encoded = fs::read(&spec.asset)?;
    let prepared = asset::decode(&encoded, asset::Limits::unlimited())?;
    drop(encoded);
    engine.add_asset(prepared)?;
    engine.set_sky(Sky::Uniform(spec.environment))?;
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
                },
                None => Light::Point {
                    position: Vec3::from_array(light.position),
                    candela,
                    range: Some(light.range),
                },
            }
        })
        .collect::<Vec<_>>();
    engine.set_lights(&lights)?;
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
    let ready_ms = begin.elapsed().as_secs_f64() * 1e3;
    let mut first_image_ms = None;
    let mut rows = Vec::new();
    let mut gpu = Vec::new();
    for (index, frame) in spec.frames.iter().enumerate() {
        if frame.cut {
            engine.cut();
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
        let report = engine.render(&camera)?;
        let cpu_ms = start.elapsed().as_secs_f64() * 1e3;
        if frame.capture || index == 0 {
            let image = engine.capture()?;
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
        }
        rows.push(json!({"index":index,"phase":frame.phase,"cut":frame.cut,
            "frame_id":report.frame.0,"cpu_submit_ms":cpu_ms,"triangles":report.scene.triangles}));
        for sample in engine.poll()?.gpu {
            gpu.push(gpu_row(sample));
        }
    }
    engine.capture()?;
    for sample in engine.poll()?.gpu {
        gpu.push(gpu_row(sample));
    }
    fs::write(
        out.join("engine.json"),
        serde_json::to_vec_pretty(&json!({
            "engine":"renderlab","path":args[1],"scene":spec.name,"size":spec.size,
            "adapter":engine.adapter_info().name,"debug_build":cfg!(debug_assertions),
            "scene_ready_ms":ready_ms,"startup_to_readback_ms":first_image_ms,
            "settings":format!("{:?}",engine.settings()),"frames":rows,"gpu":gpu,
            "startup_minimal":spec.startup_minimal,"effective_indirect":format!("{:?}",engine.renderer().indirect()),
            "timing_scope":"CPU submission excludes image readback/encoding; GPU span excludes uploads and readbacks",
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
