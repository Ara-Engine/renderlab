//! Path-traced reference of a frozen engine frame (`RENDERLAB_SCENE_DUMP`, src/engine/scene_dump.rs):
//! `scene_reference DIR [SAMPLES]` reads DIR/scene.rla and DIR/scene.json and writes
//! DIR/reference.png (the engine's exposure and the AcesFit + sRGB display curve), the linear
//! mean as DIR/reference.pfm, split halves and DIR/reference.json. Plain unidirectional path
//! tracing (renderlab::reference): sun, sky (uniform or a prepared map) and emissive surfaces.
//! Not yet traced, and listed in reference.json: local lights, the procedural atmosphere sky
//! (a uniform stand-in is refused), fog and see-through materials (skipped by the dump).
use renderlab::{Vec3, asset, engine::headless_adapter, reference};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Instant};

fn codes(pixels: &[[f32; 3]], stops: f32) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|pixel| {
            let rgb = pixel.map(|value| {
                let x = (value * stops.exp2()).max(0.0);
                let mapped = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
                let srgb = if mapped <= 0.0031308 { 12.92 * mapped } else { 1.055 * mapped.powf(1.0 / 2.4) - 0.055 };
                (srgb * 255.0).round() as u8
            });
            [rgb[0], rgb[1], rgb[2], 255]
        })
        .collect()
}
fn png(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = png::Encoder::new(fs::File::create(path)?, size[0], size[1]);
    writer.set_color(png::ColorType::Rgba);
    writer.set_depth(png::BitDepth::Eight);
    writer.write_header()?.write_image_data(pixels)?;
    Ok(())
}
fn pfm(path: &Path, size: [u32; 2], pixels: &[[f32; 3]]) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = format!("PF\n{} {}\n-1.0\n", size[0], size[1]).into_bytes();
    // PFM rows run bottom to top.
    for row in pixels.chunks(size[0] as usize).rev() {
        for p in row {
            for c in p {
                bytes.extend(c.to_le_bytes());
            }
        }
    }
    fs::write(path, bytes)?;
    Ok(())
}
fn v3(v: &Value) -> [f32; 3] {
    std::array::from_fn(|i| v[i].as_f64().unwrap_or(0.0) as f32)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = Path::new(args.first().ok_or("scene_reference DIR [SAMPLES]")?);
    let samples: u32 = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(16384);
    if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref() != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK") {
        return Err("set RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK".into());
    }
    let spec: Value = serde_json::from_slice(&fs::read(dir.join("scene.json"))?)?;
    let a = asset::decode(&fs::read(dir.join("scene.rla"))?, asset::Limits::unlimited())?;
    let mut scene = reference::Scene::from_prepared(a);
    let mut gaps = Vec::new();
    for e in spec["emission"].as_array().into_iter().flatten() {
        let i = e[0].as_u64().unwrap_or(0) as usize;
        if let Some(m) = scene.materials.get_mut(i) {
            m.emission = [e[1].as_f64().unwrap_or(0.0) as f32, e[2].as_f64().unwrap_or(0.0) as f32, e[3].as_f64().unwrap_or(0.0) as f32];
        }
    }
    let sun = &spec["sun"];
    if !sun.is_null() {
        scene.sun = Some(reference::Sun {
            to_light: Vec3::from_array(v3(&sun["to_light"])),
            illuminance: v3(&sun["illuminance"]),
            angular_radius: sun["angular_radius"].as_f64().unwrap_or(0.0) as f32,
        });
    }
    match spec["sky"]["kind"].as_str() {
        Some("uniform") => scene.sky = v3(&spec["sky"]["radiance"]),
        Some("map") => {
            let file = spec["sky"]["file"].as_str().ok_or("map sky without its file")?;
            let map = asset::environment::decode(&fs::read(dir.join(file))?, asset::environment::Limits::unbounded())?;
            scene.environment = Some(reference::Environment {
                radiance: map.specular_radiance()[0].clone(),
                yaw: map.metadata().rotation_y_radians + spec["sky"]["rotation_y"].as_f64().unwrap_or(0.0) as f32,
                intensity: spec["sky"]["intensity"].as_f64().unwrap_or(1.0) as f32,
            });
        }
        other => return Err(format!("sky {other:?} is not traceable yet (the procedural atmosphere needs its cube read back)").into()),
    }
    if spec["lights"].as_array().is_some_and(|l| !l.is_empty()) {
        gaps.push(format!("{} local lights not traced", spec["lights"].as_array().unwrap().len()));
    }
    if spec["skipped_instances"].as_u64().unwrap_or(0) > 0 {
        gaps.push(format!("{} see-through or unrecorded instances skipped", spec["skipped_instances"]));
    }
    let size = [spec["size"][0].as_u64().unwrap_or(1280) as u32, spec["size"][1].as_u64().unwrap_or(720) as u32];
    let pose = reference::Pose { eye: Vec3::from_array(v3(&spec["camera"]["eye"])), target: Vec3::from_array(v3(&spec["camera"]["target"])) };
    let view = reference::View { open: pose, close: pose, up: Vec3::Y, fov_y: spec["camera"]["fov_y"].as_f64().unwrap_or(1.0) as f32, width: size[0], height: size[1] };
    let stops = spec["exposure_stops"].as_f64().unwrap_or(0.0) as f32;

    let adapter = headless_adapter()?;
    let limits = adapter.limits();
    let limits = renderlab::traced_limits(wgpu::Limits {
        max_blas_primitive_count: limits.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: limits.max_tlas_instance_count,
        max_acceleration_structures_per_shader_stage: 1,
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_buffer_size: limits.max_buffer_size,
        ..Default::default()
    });
    // SAFETY: the caller supplied the explicit experimental ray-query acknowledgement above.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: reference::FEATURES | renderlab::TRACED_FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: renderlab::MEMORY_HINTS,
        ..Default::default()
    }))?;
    let start = Instant::now();
    let image = reference::render(&device, &queue, &scene, &view, &reference::Settings { samples, ..Default::default() })?;
    let seconds = start.elapsed().as_secs_f64();
    png(&dir.join("reference.png"), size, &codes(&image.mean, stops))?;
    pfm(&dir.join("reference.pfm"), size, &image.mean)?;
    for (k, half) in image.halves.iter().enumerate() {
        png(&dir.join(format!("reference-half{k}.png")), size, &codes(half, stops))?;
    }
    fs::write(
        dir.join("reference.json"),
        serde_json::to_vec_pretty(&json!({
            "samples": samples, "seconds": seconds, "invalid_pixels": image.invalid_pixels, "size": size,
            "exposure_stops": stops, "display_transform": "AcesFit+sRGB at the engine view's exposure",
            "method": "renderlab::reference: unidirectional path tracing, NEE + MIS, Russian roulette; no reuse or denoising",
            "not_traced": gaps, "debug_build": cfg!(debug_assertions),
        }))?,
    )?;
    eprintln!("reference {} samples in {seconds:.1} s; not traced: {gaps:?}", samples);
    Ok(())
}
