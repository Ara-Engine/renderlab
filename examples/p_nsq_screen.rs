//! P-NSQ sun-penumbra screen (docs/p-nsq.md): one process loads an ARA1 scene once per variant
//! and renders its camera path with each sun-visibility variant, writing the captured frames.
//!
//! `p_nsq_screen SCENE.json OUT VARIANT...`, each VARIANT `name:key=value,...` with keys
//! `path` (nonrt|rt), `rot` (0|1: TR1's per-frame direction rotation), `budget`
//! (cold/reset/active/held rays), `cap` (walk cap), `trunc` (0|1), `steps` (walk steps), `march`
//! (march samples), `fb` (first-step block mode), `lookup` (surface|exact) and `tr1` (0|1).
//! `P_NSQ_FRAMES` lists the frames to capture (default: the scene's capture flags).
#[allow(dead_code)]
#[path = "../tools/ara1/scene.rs"]
mod scene;
use renderlab::{asset, engine::*};
use serde_json::json;
use std::{
    collections::BTreeSet,
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

struct Variant {
    name: String,
    path: RenderPath,
    rotate: bool,
    budget: Option<renderlab::ReconstructionShadowBudget>,
    cap: Option<u32>,
    trunc: Option<bool>,
    steps: Option<u32>,
    march: Option<u32>,
    first_block: Option<u32>,
    lookup: Option<renderlab::VirtualLookup>,
    tr1: bool,
}

fn parse(text: &str) -> Result<Variant, Box<dyn std::error::Error>> {
    let (name, keys) = text.split_once(':').unwrap_or((text, ""));
    let mut v = Variant {
        name: name.into(),
        path: RenderPath::NonRt,
        rotate: true,
        budget: None,
        cap: None,
        trunc: None,
        steps: None,
        march: None,
        first_block: None,
        lookup: None,
        tr1: true,
    };
    for pair in keys.split(',').filter(|p| !p.is_empty()) {
        let (k, value) = pair.split_once('=').ok_or("key=value")?;
        match k {
            "path" => {
                v.path = match value {
                    "rt" => RenderPath::HardwareRt,
                    "nonrt" => RenderPath::NonRt,
                    _ => return Err("path rt|nonrt".into()),
                }
            }
            "rot" => v.rotate = value == "1",
            "tr1" => v.tr1 = value == "1",
            "cap" => v.cap = Some(value.parse()?),
            "trunc" => v.trunc = Some(value == "1"),
            "steps" => v.steps = Some(value.parse()?),
            "march" => v.march = Some(value.parse()?),
            "fb" => v.first_block = Some(value.parse()?),
            "lookup" => {
                v.lookup = Some(match value {
                    "surface" => renderlab::VirtualLookup::Surface,
                    "exact" => renderlab::VirtualLookup::Exact,
                    _ => return Err("lookup surface|exact".into()),
                })
            }
            "budget" => {
                let n: Vec<u32> = value.split('/').map(str::parse).collect::<Result<_, _>>()?;
                let [cold_rays, reset_rays, active_rays, held_rays] = n[..] else {
                    return Err("budget c/r/a/h".into());
                };
                v.budget = Some(renderlab::ReconstructionShadowBudget {
                    cold_rays,
                    reset_rays,
                    active_rays,
                    held_rays,
                    ..Default::default()
                });
            }
            _ => return Err(format!("unknown key {k}").into()),
        }
    }
    Ok(v)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("p_nsq_screen SCENE.json OUT VARIANT...".into());
    }
    let spec = scene::load(Path::new(&args[0]))?;
    let out = PathBuf::from(&args[1]);
    let frames: BTreeSet<usize> = match env::var("P_NSQ_FRAMES") {
        Ok(list) => list.split(',').map(str::parse).collect::<Result<_, _>>()?,
        Err(_) => (0..spec.frames.len())
            .filter(|&i| spec.frames[i].capture)
            .collect(),
    };
    let last = frames.iter().max().copied().unwrap_or(0);
    let adapter = headless_adapter()?;
    let encoded = fs::read(&spec.asset)?;
    for text in &args[2..] {
        let v = parse(text)?;
        let dir = out.join(&v.name);
        fs::create_dir_all(&dir)?;
        let begin = Instant::now();
        let ack = (v.path == RenderPath::HardwareRt).then(|| {
            // SAFETY: the lease runner sets the renderer's explicit experimental acknowledgement.
            assert_eq!(
                env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
                Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
            );
            unsafe { ExperimentalRayQuery::acknowledge() }
        });
        let mut engine = Engine::new(
            &adapter,
            EngineDescriptor {
                path: v.path,
                experimental_ray_query: ack,
                settings: Settings {
                    global_illumination: spec.effects.gi,
                    temporal_reconstruction: v.tr1,
                    reflections: renderlab::ReflectionSettings {
                        enabled: spec.effects.reflections,
                        ..Default::default()
                    },
                    exposure: Exposure::Manual {
                        stops: spec.exposure_stops,
                    },
                    aerial_perspective: false,
                    height_fog: None,
                    volumetric_fog: None,
                    ..Default::default()
                },
                texture_anisotropy: 16,
                pipeline_cache: Some(out.join("program-cache").join(match v.path {
                    RenderPath::HardwareRt => "rt",
                    _ => "nonrt",
                })),
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
        engine.add_asset(asset::decode(&encoded, asset::Limits::unlimited())?)?;
        engine.set_sky(Sky::Uniform(spec.environment))?;
        engine.set_sun(Some(Sun {
            travel: -Vec3::from_array(spec.to_light),
            illuminance: spec.sun_lux,
            angular_radius: spec.sun_radius,
        }))?;
        let r = engine.renderer_mut();
        if v.tr1 {
            r.set_reconstruction(renderlab::Reconstruction {
                rotate_directions: v.rotate,
                ..r.reconstruction()
            })?;
        }
        if let Some(budget) = v.budget {
            r.set_reconstruction_shadow_budget(budget)?;
        }
        if v.path == RenderPath::NonRt {
            let mut s = renderlab::VirtualShadowSettings::default();
            if let Some(cap) = v.cap {
                s.walk_cap = cap;
            }
            if let Some(lookup) = v.lookup {
                s.lookup = lookup;
            }
            if let Some(trunc) = v.trunc {
                s.truncated_walks_block = trunc;
            }
            if let Some(steps) = v.steps {
                s.walk_steps = steps;
            }
            if let Some(march) = v.march {
                s.march_samples = march;
            }
            if let Some(first_block) = v.first_block {
                s.first_block = first_block;
            }
            r.set_virtual_shadow_settings(s)?;
        }
        let mut rows = Vec::new();
        let mut gpu = Vec::new();
        for (index, frame) in spec.frames.iter().enumerate().take(last + 1) {
            if frame.cut {
                engine.cut();
            }
            let camera = Camera::look_at(
                Vec3::from_array(frame.eye),
                Vec3::from_array(frame.target),
                Vec3::Y,
                frame.fov_y,
                0.1,
                10_000.0,
            )?;
            let report = engine.render(&camera)?;
            engine.device().poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })?;
            if frames.contains(&index) {
                write_png(
                    &dir.join(format!("frame-{index:04}.png")),
                    &engine.capture()?,
                )?;
            }
            rows.push(
                json!({"index":index,"phase":frame.phase,"frame_id":report.frame.0,
                "shadow_rays":engine.renderer().reconstruction_shadow_rays()}),
            );
            for g in engine.poll()?.gpu {
                gpu.push(json!({"frame_id":g.frame.0,
                    "span_ms":g.render_span.as_secs_f64()*1e3,
                    "visibility_ms":g.visibility_compute.map(|d| d.as_secs_f64()*1e3),
                    "shadow_generation_ms":g.shadow_generation.map(|d| d.as_secs_f64()*1e3)}));
            }
        }
        engine.capture()?;
        for g in engine.poll()?.gpu {
            gpu.push(json!({"frame_id":g.frame.0,
                "span_ms":g.render_span.as_secs_f64()*1e3,
                "visibility_ms":g.visibility_compute.map(|d| d.as_secs_f64()*1e3),
                "shadow_generation_ms":g.shadow_generation.map(|d| d.as_secs_f64()*1e3)}));
        }
        fs::write(
            dir.join("run.json"),
            serde_json::to_vec_pretty(&json!({"variant":text,"scene":spec.name,"size":spec.size,
                "debug_build":cfg!(debug_assertions),"adapter":engine.adapter_info().name,
                "seconds":begin.elapsed().as_secs_f64(),"frames":rows,"gpu":gpu}))?,
        )?;
        println!("{} {:.1}s", v.name, begin.elapsed().as_secs_f64());
    }
    Ok(())
}
