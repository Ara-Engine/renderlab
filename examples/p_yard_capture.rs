//! P-YARD2 measurement adapter: renders one scene of the shared Unreal yardstick manifest
//! (`tools/p-yard/manifest.json`) through the public `Engine` with its shipping defaults,
//! except what the matched Unreal content lacks (fog, aerial perspective). Renderer source
//! is untouched.
//!
//!     p_yard_capture MANIFEST SCENE rt|nonrt OUTPUT capture|timing
//!
//! `capture` writes each `capture` frame's post-reconstruction HDR (the Tonemap input, as
//! Unreal's native decode) as raw RGBA f16 `hdr-NNNN.bin`; `timing` reads nothing back.
//! Both write `engine.json` with every frame's GPU render span.
#[path = "../support/estimator_diag.rs"]
#[allow(dead_code)]
mod diag;
#[path = "../tools/p-still/count.rs"]
mod p_still;
use renderlab::{asset, engine::*};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, env, fs, path::PathBuf, time::Instant};

#[derive(Deserialize)]
struct Manifest {
    scenes: HashMap<String, Scene>,
}

#[derive(Deserialize)]
struct Scene {
    kind: String,
    /// Quarry: prepared RLA and RLE2 environment. Bench: U0b schema-2 scene.json.
    asset: Option<PathBuf>,
    environment: Option<PathBuf>,
    bench: Option<PathBuf>,
    sun: Option<SunSpec>,
    exposure_stops: f32,
    size: [u32; 2],
    near: f32,
    far: f32,
    frames: Vec<Frame>,
}

#[derive(Deserialize)]
struct SunSpec {
    to_light: [f32; 3],
    lux: [f32; 3],
    angular_radius: f32,
}

#[derive(Deserialize)]
struct Frame {
    eye: [f32; 3],
    target: [f32; 3],
    fov_y: f32,
    state: String,
    cut: bool,
    capture: bool,
    #[serde(default)]
    movers: HashMap<String, [f32; 3]>,
}

/// A constant sky of `sky` cd/m² as a prepared map (irradiance pi times it), so the bench
/// background can stay black like Unreal's while the sky still lights the scene.
fn constant_sky(
    sky: [f32; 3],
) -> Result<asset::environment::PreparedEnvironment, Box<dyn std::error::Error>> {
    use asset::environment::{self as e, CubeLevel, Metadata, PreparedEnvironment, RadianceUnits};
    let level = |size: u32, value: [f32; 3]| {
        CubeLevel::new(
            size,
            vec![value; (6 * size * size) as usize],
            e::Limits::default(),
        )
    };
    Ok(PreparedEnvironment::new(
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            source_radiance_scale: 1.0,
            rotation_y_radians: 0.0,
            generator_hash: [0x5c; 32],
        },
        level(8, sky.map(|v| v * std::f32::consts::PI))?,
        [16, 8, 4, 2, 1]
            .into_iter()
            .map(|size| level(size, sky))
            .collect::<Result<_, _>>()?,
        e::Limits::default(),
    )?)
}

/// P-FLICKER A/B knobs (docs/p-flicker.md), measurement only; unset changes nothing.
/// `RENDERLAB_P_FLICKER_SUN_ACTIVE`: TR1's moving-frame sun rays (default 32).
/// `RENDERLAB_P_FLICKER_TR1`: TR1 overrides, e.g. `sharp=32,blur=2,sharpblur=1,cr=0,window=256`.
/// `RENDERLAB_P_FLICKER_GI_EVICT`: frames a GI cell may go unseen before its slot is reusable.
fn flicker_knobs(engine: &mut Engine) -> Result<(), Box<dyn std::error::Error>> {
    let r = engine.renderer_mut();
    if let Ok(v) = env::var("RENDERLAB_P_FLICKER_SUN_ACTIVE") {
        let budget = renderlab::ReconstructionShadowBudget {
            active_rays: v.parse()?,
            ..r.reconstruction_shadow_budget()
        };
        r.set_reconstruction_shadow_budget(budget)?;
    }
    if let Ok(v) = env::var("RENDERLAB_P_FLICKER_TR1") {
        let mut t = r.reconstruction();
        for item in v.split(',').filter(|s| !s.is_empty()) {
            let (key, value) = item.split_once('=').ok_or("TR1 knob: key=value")?;
            match key {
                "sharp" => t.sharp_window = value.parse()?,
                "window" => t.window = value.parse()?,
                "blur" => t.blur_budget = value.parse()?,
                "sharpblur" => t.sharp_blur_budget = value.parse()?,
                "cr" => t.catmull_rom = value != "0",
                _ => return Err(format!("unknown TR1 knob {key}").into()),
            }
        }
        r.set_reconstruction(t)?;
    }
    if let Ok(v) = env::var("RENDERLAB_P_FLICKER_GI_EVICT") {
        let s = r.indirect();
        if s.enabled {
            r.set_indirect(renderlab::IndirectSettings {
                evict_frames: v.parse()?,
                ..s
            })?;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let begin = Instant::now();
    let args: Vec<_> = env::args().skip(1).collect();
    let [manifest, name, path, out, mode] = args.as_slice() else {
        return Err("p_yard_capture MANIFEST SCENE rt|nonrt OUTPUT capture|timing".into());
    };
    let capture = match mode.as_str() {
        "capture" => true,
        "timing" => false,
        _ => return Err("mode must be capture or timing".into()),
    };
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(manifest)?)?;
    let spec = manifest
        .scenes
        .remove(name)
        .ok_or("scene not in manifest")?;
    let render_path = match path.as_str() {
        "rt" => RenderPath::HardwareRt,
        "nonrt" => RenderPath::NonRt,
        _ => return Err("path must be rt or nonrt".into()),
    };
    let out = PathBuf::from(out);
    fs::create_dir_all(&out)?;
    let ack = if render_path == RenderPath::HardwareRt {
        if env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
            != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        {
            return Err("RT needs the explicit experimental-ray-query acknowledgement".into());
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
            path: render_path,
            experimental_ray_query: ack,
            settings: Settings {
                exposure: Exposure::Manual {
                    stops: spec.exposure_stops,
                },
                // The matched Unreal levels and the path-traced references have no medium.
                aerial_perspective: false,
                height_fog: None,
                volumetric_fog: None,
                ..Default::default()
            },
            // Unreal runs r.MaxAnisotropy 16.
            texture_anisotropy: 16,
            pipeline_cache: Some(
                out.parent()
                    .ok_or("output has no parent")?
                    .join("program-cache")
                    .join(path),
            ),
            ..Default::default()
        },
    )?;
    engine.resize(spec.size[0], spec.size[1])?;
    engine
        .renderer_mut()
        .set_environment_limits(renderlab::EnvironmentLimits {
            max_dimension: 4096,
            max_map_bytes: 512 << 20,
            max_owned_bytes: 1 << 30,
        })?;
    let black = Background::Color([0.0; 3]);
    let mut movers = HashMap::new();
    let sun = match spec.kind.as_str() {
        "quarry" => {
            let prepared = asset::decode(
                &fs::read(spec.asset.as_ref().ok_or("quarry needs asset")?)?,
                asset::Limits::unlimited(),
            )?;
            engine.add_asset(prepared)?;
            let limits = asset::environment::Limits {
                max_dimension: 4096,
                max_texels: 64 * 1024 * 1024,
                max_payload_bytes: 512 * 1024 * 1024,
            };
            let map = asset::environment::decode(
                &fs::read(
                    spec.environment
                        .as_ref()
                        .ok_or("quarry needs environment")?,
                )?,
                limits,
            )?;
            let map = engine.create_environment(&map)?;
            engine.set_sky(Sky::Map {
                map,
                rotation_y: 0.0,
                intensity: 1.0,
                background: black,
            })?;
            spec.sun.ok_or("quarry needs a sun")?
        }
        "bench" => {
            let v: serde_json::Value = serde_json::from_slice(&fs::read(
                spec.bench.as_ref().ok_or("bench needs scene")?,
            )?)?;
            let f = |x: &serde_json::Value| x.as_f64().unwrap_or(f64::NAN) as f32;
            let v3 = |x: &serde_json::Value| [f(&x[0]), f(&x[1]), f(&x[2])];
            let mut materials = Vec::new();
            for m in v["materials"].as_array().ok_or("materials")? {
                let c = v3(&m["base_color"]);
                materials.push(engine.create_material(&MaterialDesc {
                    base_color: [c[0], c[1], c[2], 1.0],
                    metallic: f(&m["metallic"]),
                    roughness: f(&m["roughness"]),
                    double_sided: m["double_sided"].as_bool().unwrap_or(true),
                    ..Default::default()
                })?);
            }
            for m in v["meshes"].as_array().ok_or("meshes")? {
                let positions = m["positions"].as_array().ok_or("positions")?;
                let normals = m["normals"].as_array().ok_or("normals")?;
                let vertices: Vec<_> = positions
                    .iter()
                    .zip(normals)
                    .map(|(p, n)| Vertex {
                        position: v3(p),
                        normal: v3(n),
                        uv: [0.0; 2],
                        color: [1.0; 4],
                    })
                    .collect();
                let indices: Vec<u32> = m["indices"]
                    .as_array()
                    .ok_or("indices")?
                    .iter()
                    .map(|i| i.as_u64().unwrap_or(u64::MAX) as u32)
                    .collect();
                let mesh = engine.create_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })?;
                let material = materials[m["material"].as_u64().ok_or("material")? as usize];
                let object = engine.spawn(mesh, material, Mat4::IDENTITY)?;
                if m["moving"].as_bool().unwrap_or(false) {
                    movers.insert(m["name"].as_str().ok_or("name")?.to_owned(), object);
                }
            }
            let map = engine.create_environment(&constant_sky(v3(&v["sky"]["constant"]))?)?;
            engine.set_sky(Sky::Map {
                map,
                rotation_y: 0.0,
                intensity: 1.0,
                background: black,
            })?;
            let s = &v["sun"];
            SunSpec {
                to_light: v3(&s["to_light"]),
                lux: v3(&s["lux"]),
                angular_radius: f(&s["angular_radius"]),
            }
        }
        other => return Err(format!("unknown scene kind {other}").into()),
    };
    engine.set_sun(Some(Sun {
        travel: -Vec3::from_array(sun.to_light).normalize(),
        illuminance: sun.lux,
        angular_radius: sun.angular_radius,
    }))?;
    flicker_knobs(&mut engine)?;
    // N-VSM1: the example scenes' estimator switches (`RENDERLAB_DIAG=vsm-lookup=map;...`),
    // measurement only; unset changes nothing.
    if let Ok(keys) = env::var("RENDERLAB_DIAG") {
        let keys: Vec<(String, String)> = keys
            .split(';')
            .filter_map(|k| k.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        diag::Diag::parse(&keys)?.apply(&mut engine, render_path, RenderScale::Native)?;
    }
    let ready_ms = begin.elapsed().as_secs_f64() * 1e3;
    let mut counts = p_still::Counts::start(&mut engine);
    let mut rows = Vec::new();
    let mut gpu = Vec::new();
    let mut hdr_info = None;
    for (index, frame) in spec.frames.iter().enumerate() {
        for (mover, offset) in &frame.movers {
            let object = *movers.get(mover).ok_or("unknown mover")?;
            engine.set_transform(object, Mat4::from_translation(Vec3::from_array(*offset)))?;
        }
        if frame.cut {
            engine.cut();
        }
        let camera = Camera::look_at(
            Vec3::from_array(frame.eye),
            Vec3::from_array(frame.target),
            Vec3::Y,
            frame.fov_y,
            spec.near,
            spec.far,
        )?;
        let start = Instant::now();
        let report = engine.render(&camera)?;
        let cpu_ms = start.elapsed().as_secs_f64() * 1e3;
        // One frame in flight, as Unreal's -csvGpuStats runs; the GPU span is unaffected.
        engine.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(60)),
        })?;
        let service_ms = start.elapsed().as_secs_f64() * 1e3;
        if capture && frame.capture {
            let mut hdr = engine
                .renderer_mut()
                .try_capture_hdr()?
                .ok_or("HDR capture admission")?;
            let info = hdr.info();
            hdr_info.get_or_insert_with(|| {
                json!({"format":format!("{:?}",info.format),"width":info.width,"height":info.height,
                       "pre_exposure":info.pre_exposure})
            });
            let deadline = Instant::now() + std::time::Duration::from_secs(60);
            loop {
                gpu.extend(engine.poll()?.gpu.into_iter().map(gpu_row));
                if let Some(bytes) = hdr.try_read()? {
                    fs::write(out.join(format!("hdr-{index:04}.bin")), &bytes)?;
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("HDR capture readback timeout".into());
                }
                std::thread::yield_now();
            }
        }
        if let Some(c) = counts.as_mut() {
            c.frame(&mut engine, index, &frame.state)?;
        }
        rows.push(
            json!({"index":index,"state":frame.state,"cut":frame.cut,"frame_id":report.frame.0,
            "triangles":report.scene.triangles,"cpu_submit_ms":cpu_ms,"cpu_service_ms":service_ms}),
        );
        gpu.extend(engine.poll()?.gpu.into_iter().map(gpu_row));
    }
    engine.capture()?;
    gpu.extend(engine.poll()?.gpu.into_iter().map(gpu_row));
    fs::write(
        out.join("engine.json"),
        serde_json::to_vec_pretty(&json!({
            "engine":"renderlab","scene":name,"path":path,"mode":mode,"size":spec.size,
            "adapter":engine.adapter_info().name,"debug_build":cfg!(debug_assertions),
            "scene_ready_ms":ready_ms,"settings":format!("{:?}",engine.settings()),
            "sky":format!("{:?}",engine.sky()),
            "virtual_shadows":format!("{:?}",engine.renderer().virtual_shadow_stats()),"hdr":hdr_info,"frames":rows,"gpu":gpu,"counts":counts.map(p_still::Counts::finish),
            "timing_scope":"GPU render span: first pass to present end, GPU timestamps; one frame in flight",
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
