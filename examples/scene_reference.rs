//! Path-traced reference of a frozen engine frame (`RENDERLAB_SCENE_DUMP`, src/engine/scene_dump.rs):
//! `scene_reference DIR [SAMPLES [MAX_DEPTH]]` reads DIR/scene.rla and DIR/scene.json and writes
//! DIR/reference.png (the engine's exposure and the AcesFit + sRGB display curve), the linear
//! mean as DIR/reference.pfm, split halves (PNG and linear PFM) and DIR/reference.json. Plain unidirectional path
//! tracing (renderlab::reference): sun, sky (uniform, a prepared map, or the procedural
//! atmosphere's cube as the engine shaded with it) and emissive surfaces. Local lights are
//! traced as points. Not traced, and listed in reference.json: fog, aerial perspective and
//! see-through materials (skipped by the dump).
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
/// Irradiance (lux) on a surface facing `n`, integrated from a radiance cube (each texel's solid
/// angle: 4 / (size² (1 + u² + v²)^1.5)).
fn irradiance(level: &asset::environment::CubeLevel, n: [f64; 3]) -> [f64; 3] {
    use asset::environment::CubeFace;
    let size = level.size() as usize;
    let mut e = [0.0; 3];
    for face in CubeFace::ALL {
        for (i, texel) in level.face(face).iter().enumerate() {
            let uv = [2.0 * ((i % size) as f64 + 0.5) / size as f64 - 1.0, 2.0 * ((i / size) as f64 + 0.5) / size as f64 - 1.0];
            let d = face.direction(uv);
            let cos = n[0] * d[0] + n[1] * d[1] + n[2] * d[2];
            if cos > 0.0 {
                let w = cos * 4.0 / ((size * size) as f64 * (1.0 + uv[0] * uv[0] + uv[1] * uv[1]).powf(1.5));
                for c in 0..3 {
                    e[c] += w * f64::from(texel[c]);
                }
            }
        }
    }
    e
}
/// The irradiance cube's texel at the centre of the face facing `face`.
fn stored(level: &asset::environment::CubeLevel, face: asset::environment::CubeFace) -> [f64; 3] {
    let size = level.size() as usize;
    level.face(face)[(size / 2) * size + size / 2].map(f64::from)
}
fn v3(v: &Value) -> [f32; 3] {
    std::array::from_fn(|i| v[i].as_f64().unwrap_or(0.0) as f32)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = Path::new(args.first().ok_or("scene_reference DIR [SAMPLES]")?);
    // `check` in place of SAMPLES prints the sky check and stops, without a GPU.
    let check = args.get(1).is_some_and(|s| s == "check");
    let samples: u32 = if check { 0 } else { args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(16384) };
    if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref() != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK") {
        return Err("set RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK".into());
    }
    let spec: Value = serde_json::from_slice(&fs::read(dir.join("scene.json"))?)?;
    let a = asset::decode(&fs::read(dir.join("scene.rla"))?, asset::Limits::unlimited())?;
    let mut scene = reference::Scene::from_prepared(a);
    let mut gaps = Vec::new();
    let mut sky_check: Vec<Value> = Vec::new();
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
            // The tracer samples the radiance; engine shading's diffuse reads the irradiance cube.
            // They must agree (reference.json `sky_check`), or the two light the scene differently.
            use asset::environment::CubeFace;
            sky_check = CubeFace::ALL
                .iter()
                .map(|&f| {
                    let n = f.direction([0.0, 0.0]);
                    json!({"face": format!("{f:?}"), "from_radiance": irradiance(&map.specular_radiance()[0], n), "irradiance_cube": stored(map.diffuse_irradiance(), f)})
                })
                .collect();
            scene.environment = Some(reference::Environment {
                radiance: map.specular_radiance()[0].clone(),
                yaw: map.metadata().rotation_y_radians + spec["sky"]["rotation_y"].as_f64().unwrap_or(0.0) as f32,
                intensity: spec["sky"]["intensity"].as_f64().unwrap_or(1.0) as f32,
            });
        }
        // The procedural atmosphere as shading read it: its specular level 0 in world axes
        // (raw f32 RGB, WebGPU face order), and the sun already dimmed by its transmittance.
        Some("cube") => {
            let file = spec["sky"]["file"].as_str().ok_or("cube sky without its file")?;
            let size = spec["sky"]["size"].as_u64().ok_or("cube sky without its size")? as u32;
            let texels = fs::read(dir.join(file))?
                .chunks_exact(12)
                .map(|t| std::array::from_fn(|c| f32::from_le_bytes(t[4 * c..4 * c + 4].try_into().unwrap())))
                .collect();
            scene.environment = Some(reference::Environment {
                radiance: asset::environment::CubeLevel::new(size, texels, asset::environment::Limits::unbounded())?,
                yaw: 0.0,
                intensity: 1.0,
            });
            gaps.push("atmosphere: sky from the engine's 128-texel cube (no aerial perspective or fog between surfaces)".into());
        }
        other => return Err(format!("sky {other:?} is not traceable (a dump from before the procedural sky readback)").into()),
    }
    for l in spec["lights"].as_array().into_iter().flatten() {
        let f = |k: &str| l[k].as_f64().unwrap_or(0.0) as f32;
        match l["kind"].as_str() {
            Some("point") | Some("spot") => scene.lights.push(reference::LocalLight {
                position: Vec3::from_array(v3(&l["position"])),
                candela: v3(&l["candela"]),
                range: f("range"),
                spot: (l["kind"] == "spot").then(|| (Vec3::from_array(v3(&l["travel"])), f("inner").cos(), f("outer").cos())),
            }),
            _ => gaps.push("a directional light other than the sun".into()),
        }
        if f("radius") > 0.0 {
            gaps.push(format!("light source radius {} m traced as a point", f("radius")));
        }
    }
    if spec["skipped_instances"].as_u64().unwrap_or(0) > 0 {
        gaps.push(format!("{} see-through or unrecorded instances skipped", spec["skipped_instances"]));
    }
    let size = [spec["size"][0].as_u64().unwrap_or(1280) as u32, spec["size"][1].as_u64().unwrap_or(720) as u32];
    let pose = reference::Pose { eye: Vec3::from_array(v3(&spec["camera"]["eye"])), target: Vec3::from_array(v3(&spec["camera"]["target"])) };
    let view = reference::View { open: pose, close: pose, up: Vec3::Y, fov_y: spec["camera"]["fov_y"].as_f64().unwrap_or(1.0) as f32, width: size[0], height: size[1] };
    let stops = spec["exposure_stops"].as_f64().unwrap_or(0.0) as f32;

    if check {
        println!("{}", serde_json::to_string_pretty(&sky_check)?);
        return Ok(());
    }
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
    // MAX_DEPTH (path segments, as `reference::Settings::max_depth`: 2 is direct light, 3 one
    // bounce) writes `reference-depth<N>.*` beside the full reference, to see how much light the
    // later bounces carry.
    let depth: Option<u32> = args.get(2).map(|s| s.parse()).transpose()?;
    let stem = depth.map_or("reference".to_owned(), |d| format!("reference-depth{d}"));
    // The centre hit's material per pixel (the geometry capture, with the one sun visibility ray it requires)
    // gives the mask of materials the tracer does not shade as the engine does (scene.json
    // `custom_materials`: programs that replace the standard `surface`).
    let custom: Vec<u32> = spec["custom_materials"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|i| i as u32).collect();
    let settings = reference::Settings { samples, capture_geometry: depth.is_none(), visibility_samples: 1, ..Default::default() };
    let settings = reference::Settings { max_depth: depth.unwrap_or(settings.max_depth), ..settings };
    let image = reference::render(&device, &queue, &scene, &view, &settings)?;
    let seconds = start.elapsed().as_secs_f64();
    // reference-mask.png: 255 where the reference is judged, 0 where the centre hit's material is
    // custom (its 80-byte geometry record's ids.z is the material + 1).
    let mut masked = 0usize;
    if !image.geometry.is_empty() {
        let mask: Vec<u8> = image
            .geometry
            .chunks_exact(80)
            .flat_map(|r| {
                let material = u32::from_le_bytes(r[40..44].try_into().unwrap());
                let out = material > 0 && custom.contains(&(material - 1));
                masked += usize::from(out);
                let v = if out { 0 } else { 255 };
                [v, v, v, 255]
            })
            .collect();
        png(&dir.join("reference-mask.png"), size, &mask)?;
    }
    let masked_fraction = masked as f64 / f64::from(size[0] * size[1]);
    png(&dir.join(format!("{stem}.png")), size, &codes(&image.mean, stops))?;
    pfm(&dir.join(format!("{stem}.pfm")), size, &image.mean)?;
    for (k, half) in image.halves.iter().enumerate().filter(|_| depth.is_none()) {
        png(&dir.join(format!("reference-half{k}.png")), size, &codes(half, stops))?;
        // Linear too: a frame at another exposure judges noise against halves shown at its stops.
        pfm(&dir.join(format!("reference-half{k}.pfm")), size, half)?;
    }
    fs::write(
        dir.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&json!({
            "samples": samples, "max_depth": settings.max_depth, "seconds": seconds, "invalid_pixels": image.invalid_pixels, "size": size,
            "exposure_stops": stops, "display_transform": "AcesFit+sRGB at the engine view's exposure",
            "method": "renderlab::reference: unidirectional path tracing, NEE + MIS, Russian roulette; no reuse or denoising",
            "not_traced": gaps, "debug_build": cfg!(debug_assertions), "sky_check": sky_check, "custom_materials": custom, "masked_fraction": masked_fraction,
        }))?,
    )?;
    eprintln!("reference {} samples in {seconds:.1} s; not traced: {gaps:?}", samples);
    Ok(())
}
