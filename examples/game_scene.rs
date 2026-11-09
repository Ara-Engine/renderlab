//! A small game-like scene through the engine API (docs/api.md), offscreen, at dusk.
//!
//! Procedural ground, an arch, crates, a red wall (colour bleeding), a glossy gold sphere, a
//! mirror chrome sphere, a spinning crate, a glass pane, cutout foliage, a brazier with fire
//! and smoke particles, lanterns (point and spot lights with soft shadows and glowing bulbs),
//! an emblem decal and a wet puddle decal, and the skinned Fox walking with a toon program
//! (`--fox Fox.glb`, fetched by `scripts/m7-sk1-asset.py`; without it the hero is two toon
//! spheres). A low sun through the physical atmosphere, volumetric and height fog, GI,
//! traced reflections (hardware rays on RT, software rays on non-RT), temporal
//! reconstruction and post (bloom, motion blur, grading) render it from a camera fly path.
//! Every `--every`-th frame is written as PNG.
//!
//! Options: `--hour H` (sun time of day at 40 degrees latitude; default 17.5, dusk) or
//! `--time dusk|noon`, `--sun-speed DEGREES_PER_SECOND` (advances the time of day, 15 degrees
//! an hour) with `--sun-after FRAME`, `--still` (frozen camera and objects), `--timing-frames
//! N` (a drained timing segment), `--night-lanterns` (27 dim local sources),
//! `--local-shadow-count N` (0-128 map lights, positive enables all-light RT), `--sky
//! uniform`, `--no-volumetric-fog`, `--no-gi`, `--no-temporal`, `--no-post`,
//! `--no-reflections` (every program keeps its untraced estimate), `--render-scale`,
//! `--exposure`, and `--quarry MANIFEST`,
//! which replaces the ground with the registered quarry scans and its RLE2 sky (manifest
//! keys `asset`, `limits`, `environment`, `exposure_stops`).
//!
//! ```text
//! cargo run --example game_scene -- --path nonrt --out D:/Coding/renderlab-work/api1/nonrt
//! RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK \
//!     cargo run --example game_scene -- --path rt --out D:/Coding/renderlab-work/api1/rt
//! ```
#[path = "../support/environment_controls.rs"]
#[allow(dead_code)]
mod environment_controls;
#[path = "../support/limits.rs"]
mod policy;

use renderlab::{asset, engine::*};
use sha2::{Digest, Sha256};
use std::{f32::consts::PI, fs, path::PathBuf, time::Instant};

const GAME_PROGRAM: &str = include_str!("game_scene.wgsl");
const TOON_PROGRAM: &str = include_str!("game_toon.wgsl");
const FPS: f32 = 30.0;
/// The scene's latitude for `Sun::at_hour`, radians.
const LATITUDE: f32 = 0.7;

struct Args {
    path: Option<RenderPath>,
    out: PathBuf,
    frames: u32,
    every: u32,
    size: [u32; 2],
    exposure: Option<f32>,
    quarry: Option<PathBuf>,
    gi: bool,
    temporal: bool,
    scale: RenderScale,
    uniform_sky: bool,
    volumetric_fog: Option<bool>,
    post: bool,
    glass: bool,
    particles: bool,
    local_shadows: bool,
    reflections: bool,
    hour: f32,
    fox: Option<PathBuf>,
    sun_speed: f32,
    sun_after: u32,
    still: bool,
    timing_frames: u32,
    pipeline_cache: Option<PathBuf>,
    reload_probe: bool,
    night_lanterns: bool,
    local_shadow_count: Option<u32>,
}
impl Args {
    fn parse() -> Result<Self, Box<dyn std::error::Error>> {
        let mut a = Self {
            path: None,
            out: PathBuf::from("measurements/game-scene"),
            frames: 48,
            every: 8,
            size: [1280, 720],
            exposure: None,
            quarry: None,
            gi: true,
            temporal: true,
            scale: RenderScale::Native,
            uniform_sky: false,
            volumetric_fog: None,
            post: true,
            glass: true,
            particles: true,
            local_shadows: true,
            reflections: true,
            hour: 17.5,
            fox: std::env::var_os("RENDERLAB_FOX_GLB").map(PathBuf::from),
            sun_speed: 0.0,
            sun_after: 0,
            still: false,
            timing_frames: 0,
            pipeline_cache: Some(
                std::env::var_os("RENDERLAB_PIPELINE_CACHE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| "measurements/pipeline-cache".into()),
            ),
            reload_probe: false,
            night_lanterns: false,
            local_shadow_count: None,
        };
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            let mut value = || it.next().ok_or(format!("{arg} needs a value"));
            match arg.as_str() {
                "--path" => {
                    a.path = match value()?.as_str() {
                        "rt" => Some(RenderPath::HardwareRt),
                        "nonrt" => Some(RenderPath::NonRt),
                        "auto" => None,
                        other => return Err(format!("--path rt|nonrt|auto, not {other}").into()),
                    }
                }
                "--out" => a.out = value()?.into(),
                "--frames" => a.frames = value()?.parse()?,
                "--every" => a.every = value()?.parse::<u32>()?.max(1),
                "--size" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').ok_or("--size WIDTHxHEIGHT")?;
                    a.size = [w.parse()?, h.parse()?];
                }
                "--exposure" => a.exposure = Some(value()?.parse()?),
                "--quarry" => a.quarry = Some(value()?.into()),
                "--pipeline-cache" => a.pipeline_cache = Some(value()?.into()),
                "--no-pipeline-cache" => a.pipeline_cache = None,
                "--reload-probe" => a.reload_probe = true,
                "--no-gi" => a.gi = false,
                "--sky" => {
                    a.uniform_sky = match value()?.as_str() {
                        "atmosphere" => false,
                        "uniform" => true,
                        other => {
                            return Err(format!("--sky atmosphere|uniform, not {other}").into());
                        }
                    }
                }
                "--volumetric-fog" => a.volumetric_fog = Some(true),
                "--no-volumetric-fog" => a.volumetric_fog = Some(false),
                "--no-temporal" => a.temporal = false,
                "--no-post" => a.post = false,
                "--no-glass" => a.glass = false,
                "--no-particles" => a.particles = false,
                "--no-local-shadows" => a.local_shadows = false,
                "--no-reflections" => a.reflections = false,
                "--fox" => a.fox = Some(value()?.into()),
                "--hour" => a.hour = value()?.parse()?,
                "--time" => {
                    a.hour = match value()?.as_str() {
                        "dusk" => 17.5,
                        "noon" => 12.0,
                        other => return Err(format!("--time dusk|noon, not {other}").into()),
                    }
                }
                "--sun-speed" => a.sun_speed = value()?.parse()?,
                "--sun-after" => a.sun_after = value()?.parse()?,
                "--still" => a.still = true,
                "--night-lanterns" => a.night_lanterns = true,
                "--local-shadow-count" => a.local_shadow_count = Some(value()?.parse()?),
                "--timing-frames" => a.timing_frames = value()?.parse()?,
                "--render-scale" => {
                    a.scale = match value()?.as_str() {
                        "native" => RenderScale::Native,
                        "quality" => RenderScale::Quality,
                        "performance" => RenderScale::Performance,
                        other => {
                            return Err(format!(
                                "--render-scale native|quality|performance, not {other}"
                            )
                            .into());
                        }
                    }
                }
                _ => return Err(format!("unknown argument {arg}").into()),
            }
        }
        if a.frames == 0 {
            return Err("--frames must be positive".into());
        }
        if !a.sun_speed.is_finite() {
            return Err("--sun-speed must be finite".into());
        }
        Ok(a)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let args = Args::parse()?;
    let adapter = headless_adapter()?;
    let adapter_seconds = start.elapsed().as_secs_f64();
    let capabilities = Capabilities::query(&adapter);
    let path = match args.path {
        Some(path) => path,
        None => capabilities
            .preferred()
            .ok_or("adapter supports neither render path")?,
    };
    let experimental_ray_query = (path == RenderPath::HardwareRt)
        .then(|| {
            let ack = std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK");
            (ack.as_deref() == Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"))
                // SAFETY: the caller accepted wgpu's experimental ray-query risk in the
                // environment, as the GPU test suites require.
                .then(|| unsafe { ExperimentalRayQuery::acknowledge() })
                .ok_or("rt needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        })
        .transpose()?;
    let quarry: Option<serde_json::Value> = args
        .quarry
        .as_ref()
        .map(|p| fs::read(p).map(|b| serde_json::from_slice(&b)))
        .transpose()?
        .transpose()?;
    let manifest_path = |key: &str| -> Result<PathBuf, String> {
        quarry
            .as_ref()
            .and_then(|m| m[key].as_str())
            .map(PathBuf::from)
            .ok_or(format!("quarry manifest needs `{key}`"))
    };
    let exposure = args.exposure.unwrap_or_else(|| {
        if args.night_lanterns {
            return -1.0;
        }
        quarry
            .as_ref()
            .and_then(|m| m["exposure_stops"].as_f64())
            .map_or(if args.hour < 16.0 { -14.5 } else { -11.0 }, |s| s as f32)
    });
    let mut preparation = Vec::new();

    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            pipeline_cache: args.pipeline_cache.clone(),
            settings: Settings {
                global_illumination: args.gi,
                temporal_reconstruction: args.temporal,
                render_scale: args.scale,
                // A light dusk haze (0.0015 per metre over the first 128 m) for shafts.
                volumetric_fog: match args.volumetric_fog {
                    Some(false) => None,
                    _ => Some(VolumetricFog {
                        extra_density: 0.0015,
                        ..Default::default()
                    }),
                },
                local_shadows: LocalShadows {
                    count: if args.local_shadows {
                        args.local_shadow_count
                            .unwrap_or(LocalShadows::default().count)
                    } else {
                        0
                    },
                    ..Default::default()
                },
                post: PostSettings {
                    enabled: args.post,
                    bloom: 0.06,
                    ..Default::default()
                },
                exposure: Exposure::Manual { stops: exposure },
                reflections: if args.reflections {
                    ReflectionSettings::default()
                } else {
                    ReflectionSettings::off()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    )?;
    let engine_seconds = start.elapsed().as_secs_f64();
    engine.resize(args.size[0], args.size[1])?;
    eprintln!(
        "game_scene adapter={:?} path={path:?} capabilities=(rt {}, nonrt {}) size={}x{} debug_build={}",
        engine.adapter_info().name,
        capabilities.hardware_rt,
        capabilities.non_rt,
        args.size[0],
        args.size[1],
        cfg!(debug_assertions)
    );

    // Two programs in one frame: photoreal props and the toon hero (MP1).
    let program = engine.create_program("game", GAME_PROGRAM)?;
    let toon_program = engine.create_program("toon", TOON_PROGRAM)?;
    let programs_seconds = start.elapsed().as_secs_f64();
    let material = |engine: &mut Engine, color: [f32; 3], metallic: f32, roughness: f32| {
        engine.create_material(&MaterialDesc {
            base_color: [color[0], color[1], color[2], 1.0],
            metallic,
            roughness,
            program: Some(program),
            ..Default::default()
        })
    };
    let stone = material(&mut engine, [0.62, 0.6, 0.56], 0.0, 0.9)?;
    let wood = material(&mut engine, [0.56, 0.36, 0.18], 0.0, 0.75)?;
    let red = material(&mut engine, [0.8, 0.09, 0.06], 0.0, 0.6)?;
    let gold = material(&mut engine, [1.0, 0.78, 0.34], 1.0, 0.25)?;
    let paint = material(&mut engine, [0.1, 0.3, 0.85], 0.0, 0.35)?;
    let chrome = material(&mut engine, [0.95, 0.95, 0.97], 1.0, 0.02)?;
    let hero_style = [
        ("bands", Value::F32(3.0)),
        ("shadow_tint", Value::Vec3([0.22, 0.14, 0.38])),
        ("rim_color", Value::Vec3([0.35, 0.5, 0.7])),
    ];
    let toon = engine.create_material(&MaterialDesc {
        base_color: [0.95, 0.55, 0.2, 1.0],
        program: Some(toon_program),
        parameters: &hero_style,
        ..Default::default()
    })?;
    let glass = engine.create_material(&MaterialDesc {
        base_color: [0.86, 0.95, 0.93, 1.0],
        roughness: 0.04,
        // The standard program (None) declares TP1's `transmittance` hook glass needs.
        transparency: Some(Transparency::default()),
        ..Default::default()
    })?;
    let leaves = engine.create_material(&MaterialDesc {
        base_color: [0.22, 0.42, 0.12, 1.0],
        roughness: 0.7,
        thin: true,
        program: Some(program),
        ..Default::default()
    })?;
    let iron = material(&mut engine, [0.08, 0.08, 0.09], 0.9, 0.45)?;
    // Night diagnostics keep the courtyard sources at lantern-scale exposure.
    let lamp_scale = if args.night_lanterns { 0.001 } else { 1.0 };
    let bulb = engine.create_material(&MaterialDesc {
        base_color: [1.0, 0.9, 0.7, 1.0],
        program: Some(program),
        parameters: &[(
            "glow",
            Value::Vec3([
                60_000.0 * lamp_scale,
                40_000.0 * lamp_scale,
                18_000.0 * lamp_scale,
            ]),
        )],
        ..Default::default()
    })?;

    let cube = engine.create_mesh(&mesh(&cube_geometry()))?;
    let sphere = engine.create_mesh(&mesh(&sphere_geometry(48, 24)))?;
    // Foliage: a card cut to a leaf mask once, at load, into ordinary opaque geometry (TP1).
    let mask = AlphaMask::from_rgba8(256, 256, &leaf_rgba8(256));
    let card = cutout(&mesh(&card_geometry(1.8, 1.5)), &mask, 1.0, 0.5, 64)?;
    let bush = engine.create_mesh(&card.data())?;
    let anchor = match &quarry {
        Some(_) => {
            let policy_args = vec![
                "--limits".into(),
                manifest_path("limits")?.display().to_string(),
            ];
            let (policy, _) = policy::Policy::args(policy_args)?;
            let limits = policy.prepared();
            let loaded = Instant::now();
            let asset = asset::decode(&fs::read(manifest_path("asset")?)?, limits)?;
            preparation.push(serde_json::json!({"phase":"asset_read_decode", "seconds":loaded.elapsed().as_secs_f64()}));
            let upload = Instant::now();
            let handles = engine.add_asset(asset)?;
            preparation.push(serde_json::json!({"phase":"asset_prepare_upload", "seconds":upload.elapsed().as_secs_f64()}));
            let environment = Instant::now();
            let map = environment_controls::load_environment(
                &manifest_path("environment")?,
                asset::environment::Limits {
                    max_dimension: 4096,
                    max_texels: 64 * 1024 * 1024,
                    max_payload_bytes: 128 * 1024 * 1024,
                },
            )?;
            let map = engine.create_environment(&map)?;
            engine.set_sky(Sky::Map {
                map,
                rotation_y: 0.0,
                intensity: 1.0,
                background: Background::Sky,
            })?;
            preparation.push(serde_json::json!({"phase":"environment_read_upload", "seconds":environment.elapsed().as_secs_f64()}));
            eprintln!(
                "game_scene quarry objects={} load_seconds={:.1}",
                handles.objects.iter().map(Vec::len).sum::<usize>(),
                loaded.elapsed().as_secs_f64()
            );
            Vec3::new(0.0, 0.6, -0.15)
        }
        None => {
            let ground = material(&mut engine, [0.34, 0.37, 0.28], 0.0, 0.95)?;
            let plane = engine.create_mesh(&mesh(&plane_geometry(60.0, 8)))?;
            engine.spawn(plane, ground, Mat4::IDENTITY)?;
            // The physical atmosphere is the default sky (AT1); `--sky uniform` swaps in a
            // constant blue sky for comparison.
            if args.uniform_sky {
                engine.set_sky(Sky::Uniform([3500.0, 5000.0, 7500.0]))?;
            }
            Vec3::ZERO
        }
    };
    let at = |x: f32, y: f32, z: f32| anchor + Vec3::new(x, y, z);
    let boxed = |center: Vec3, size: Vec3| Mat4::from_translation(center) * Mat4::from_scale(size);
    // The arch: two pillars and a lintel.
    for x in [-2.2, 2.2] {
        engine.spawn(
            cube,
            stone,
            boxed(at(x, 2.0, -3.0), Vec3::new(0.8, 4.0, 0.8)),
        )?;
    }
    engine.spawn(
        cube,
        stone,
        boxed(at(0.0, 4.4, -3.0), Vec3::new(5.2, 0.8, 1.0)),
    )?;
    // A red wall beside it, whose colour should bleed onto the ground and hero.
    engine.spawn(
        cube,
        red,
        boxed(at(-5.5, 1.5, 0.0), Vec3::new(0.3, 3.0, 6.0)),
    )?;
    // A stack of crates.
    for (x, y, z, turn) in [
        (4.0, 0.5, 1.0, 0.1),
        (5.1, 0.5, 1.3, -0.25),
        (4.5, 1.5, 1.1, 0.4),
        (4.2, 0.5, 2.3, 0.0),
    ] {
        engine.spawn(
            cube,
            wood,
            Mat4::from_translation(at(x, y, z)) * Mat4::from_rotation_y(turn),
        )?;
    }
    engine.spawn(sphere, gold, boxed(at(1.8, 1.0, 3.2), Vec3::splat(1.0)))?;
    if args.glass {
        // A glass pane in front of the crates (TP1 forward glass: refracts and tints).
        engine.spawn(
            cube,
            glass,
            Mat4::from_translation(at(5.4, 0.8, 3.6))
                * Mat4::from_rotation_y(0.8)
                * Mat4::from_scale(Vec3::new(2.2, 1.6, 0.04)),
        )?;
    }
    // Bushes: three crossed cutout cards each.
    for (x, z) in [(-4.0, -2.2), (3.6, -3.8), (6.2, -0.6)] {
        for k in 0..3 {
            engine.spawn(
                bush,
                leaves,
                Mat4::from_translation(at(x, 0.0, z)) * Mat4::from_rotation_y(k as f32 * PI / 3.0),
            )?;
        }
    }
    // A brazier by the red wall, and lantern fittings: a bulb under the arch and a lamp
    // post over the crates.
    let brazier = at(-3.8, 0.0, 2.0);
    engine.spawn(
        cube,
        stone,
        boxed(brazier + Vec3::new(0.0, 0.3, 0.0), Vec3::splat(0.6)),
    )?;
    let arch_lamp = at(0.0, 3.55, -3.0);
    engine.spawn(sphere, bulb, boxed(arch_lamp, Vec3::splat(0.12)))?;
    engine.spawn(
        cube,
        iron,
        boxed(
            arch_lamp + Vec3::new(0.0, 0.3, 0.0),
            Vec3::new(0.03, 0.5, 0.03),
        ),
    )?;
    let post_lamp = at(3.0, 3.2, 3.0);
    engine.spawn(
        cube,
        iron,
        boxed(at(3.0, 1.6, 3.0), Vec3::new(0.12, 3.2, 0.12)),
    )?;
    engine.spawn(
        cube,
        iron,
        boxed(
            post_lamp + Vec3::new(0.0, 0.12, 0.0),
            Vec3::new(0.4, 0.08, 0.4),
        ),
    )?;
    engine.spawn(sphere, bulb, boxed(post_lamp, Vec3::splat(0.1)))?;
    // A mirror-polished chrome sphere: traced mirror reflections on RT (M7-R1).
    engine.spawn(sphere, chrome, boxed(at(-1.2, 0.8, 5.6), Vec3::splat(0.8)))?;
    let spinner = engine.spawn(cube, paint, Mat4::from_translation(at(-2.5, 0.6, 3.0)))?;
    let mut hero = match &args.fox {
        Some(source) => Hero::fox(&mut engine, source, &args.out, toon_program, &hero_style)?,
        None => Hero::Spheres([
            engine.spawn(sphere, toon, Mat4::IDENTITY)?,
            engine.spawn(sphere, toon, Mat4::IDENTITY)?,
        ]),
    };

    // The time of day sets the sun (AT2 updates the sky in place, without a GI flush). Under
    // the atmosphere its illuminance is outside it (120 klux), which its transmittance dims
    // and reddens near the horizon. +X is east and -Z north; the scene faces the setting sun.
    let sun_hour = |frame: u32| {
        args.hour + frame.saturating_sub(args.sun_after) as f32 / FPS * args.sun_speed / 15.0
    };
    let scene_sun = |hour| {
        Sun::at_hour(hour, LATITUDE).map(|sun| Sun {
            illuminance: if args.night_lanterns {
                [0.03, 0.04, 0.08]
            } else {
                sun.illuminance
            },
            ..sun
        })
    };
    engine.set_sun(scene_sun(sun_hour(0)))?;
    if args.night_lanterns {
        engine.set_sky(Sky::Uniform([0.001, 0.002, 0.005]))?;
    }
    // Fire and smoke over the brazier (TP1 GPU particles), simulated at the frame rate.
    engine.set_particle_step(1.0 / FPS)?;
    let flame = brazier + Vec3::new(0.0, 0.65, 0.0);
    if args.particles {
        engine.create_emitter(
            &EmitterDescriptor {
                position: flame,
                spawn_radius: 0.18,
                spawn_rate: 140.0,
                lifetime: [0.45, 0.9],
                velocity: Vec3::new(0.0, 0.9, 0.0),
                velocity_spread: 0.35,
                acceleration: Vec3::new(0.0, 1.6, 0.0),
                drag: 0.6,
                size: [0.32, 0.08],
                color: [[1.0, 0.55, 0.15, 0.85], [0.4, 0.1, 0.02, 0.0]],
                emission: [
                    [9000.0 * lamp_scale, 3800.0 * lamp_scale, 900.0 * lamp_scale],
                    [1200.0 * lamp_scale, 250.0 * lamp_scale, 40.0 * lamp_scale],
                ],
                fade: [0.1, 0.4],
                soft_distance: 0.15,
                capacity: 128,
                seed: 7,
                ..Default::default()
            },
            None,
        )?;
        engine.create_emitter(
            &EmitterDescriptor {
                position: flame + Vec3::new(0.0, 0.6, 0.0),
                spawn_radius: 0.15,
                spawn_rate: 24.0,
                lifetime: [2.0, 3.0],
                velocity: Vec3::new(0.15, 0.6, 0.0),
                velocity_spread: 0.15,
                acceleration: Vec3::new(0.25, 0.2, 0.0),
                drag: 0.3,
                size: [0.35, 1.2],
                color: [[0.3, 0.3, 0.32, 0.35], [0.5, 0.5, 0.52, 0.0]],
                fade: [0.2, 0.6],
                soft_distance: 0.3,
                capacity: 80,
                seed: 11,
                ..Default::default()
            },
            None,
        )?;
    }
    // Keep the courtyard's point/spot/fire sources; night mode adds 24 LC3 lanterns.
    // The facade shadows every contributing RT local, or up to 128 cached map lights.
    // Source radii retain soft penumbrae on both paths.
    let lanterns = |t: f32| {
        let flicker = 0.85 + 0.1 * (23.0 * t).sin() + 0.05 * (37.0 * t + 1.0).sin();
        let mut lights = vec![
            Light::Point {
                position: arch_lamp - Vec3::new(0.0, 0.2, 0.0),
                candela: [
                    30_000.0 * lamp_scale,
                    21_000.0 * lamp_scale,
                    11_000.0 * lamp_scale,
                ],
                range: Some(14.0),
                source_radius: 0.12,
            },
            Light::Spot {
                position: post_lamp - Vec3::new(0.0, 0.15, 0.0),
                travel: (at(4.6, 0.0, 1.5) - post_lamp).normalize(),
                candela: [
                    26_000.0 * lamp_scale,
                    25_000.0 * lamp_scale,
                    23_000.0 * lamp_scale,
                ],
                range: Some(12.0),
                inner: 0.3,
                outer: 0.6,
                source_radius: 0.1,
            },
            Light::Point {
                position: flame + Vec3::new(0.0, 0.3, 0.0),
                candela: [
                    24_000.0 * flicker * lamp_scale,
                    12_000.0 * flicker * lamp_scale,
                    4_000.0 * flicker * lamp_scale,
                ],
                range: Some(10.0),
                source_radius: 0.25,
            },
        ];
        if args.night_lanterns {
            lights.extend((0..24).map(|i| Light::Point {
                position: at((i % 6) as f32 * 1.8 - 4.5, 1.8, (i / 6) as f32 * 2.0 - 1.5),
                candela: [22.0, 12.0, 4.0],
                range: Some(4.0),
                source_radius: 0.06,
            }));
        }
        lights
    };

    // Decals on the ground: an emblem in front of the arch, and a wet puddle that darkens
    // the ground and makes it glossy. Ground decals turn local +Z (their receiver normal)
    // to world +Y; the Z scale is the projection depth.
    let ground_decal = |center: Vec3, width: f32, depth: f32, turn: f32| {
        Mat4::from_translation(center)
            * Mat4::from_rotation_y(turn)
            * Mat4::from_rotation_x(-PI / 2.0)
            * Mat4::from_scale(Vec3::new(width, width * depth, 0.6))
    };
    let emblem = DecalTexture::from_rgba8(
        DecalTextureKind::BaseColorSrgb,
        [256; 2],
        &emblem_rgba8(256),
    )?;
    let puddle = DecalTexture::from_rgba8(
        DecalTextureKind::BaseColorSrgb,
        [256; 2],
        &puddle_rgba8(256),
    )?;
    engine.set_decals(&[
        Decal {
            transform: ground_decal(at(0.0, 0.0, -0.6), 2.6, 1.0, 0.3),
            textures: DecalTextures {
                base_color: Some(emblem),
                ..Default::default()
            },
            channels: [1.0, 0.0, 0.0, 0.0],
            ..Default::default()
        },
        Decal {
            transform: ground_decal(at(1.5, 0.0, 4.6), 4.0, 0.7, -0.4),
            textures: DecalTextures {
                base_color: Some(puddle),
                ..Default::default()
            },
            base_color: [0.55, 0.57, 0.62, 1.0],
            roughness: 0.04,
            channels: [1.0, 1.0, 0.0, 0.0],
            blend: DecalBlend::Multiply,
            ..Default::default()
        },
    ])?;

    fs::create_dir_all(&args.out)?;
    let ready = start.elapsed();
    let mut frames = Vec::new();
    let mut last_camera = None;
    let mut first_image_seconds = None;
    let mut reload = None;
    for frame in 0..args.frames {
        let t = if args.still { 0.0 } else { frame as f32 / FPS };
        let sun_update = Instant::now();
        let hour = sun_hour(frame);
        if args.sun_speed != 0.0 && frame >= args.sun_after {
            engine.set_sun(scene_sun(hour))?;
        }
        let sun_update_ms = sun_update.elapsed().as_secs_f64() * 1e3;
        // The hero walks a circle through the arch.
        let walk = 0.35 * t;
        let ground = at(3.0 * walk.sin(), 0.0, -1.0 + 3.0 * walk.cos());
        let heading = walk.cos().atan2(-walk.sin());
        let focus = hero.update(&mut engine, ground, heading, t)?;
        engine.set_lights(&lanterns(t))?;
        engine.set_transform(
            spinner,
            Mat4::from_translation(at(-2.5, 0.6 + 0.3 * (2.0 * t).sin(), 3.0))
                * Mat4::from_rotation_y(1.5 * t)
                * Mat4::from_rotation_x(0.6 * t),
        )?;
        // Fly path: a slow orbit that descends and closes in on the hero.
        let orbit = 0.6 + 0.25 * t;
        let radius = 14.0 - 4.0 * (t / 2.0).min(1.0);
        let eye = anchor
            + Vec3::new(
                radius * orbit.sin(),
                5.5 - 2.0 * (t / 2.0).min(1.0),
                radius * orbit.cos(),
            );
        let camera = Camera::look_at(
            eye,
            focus.lerp(anchor + Vec3::Y, 0.5),
            Vec3::Y,
            55f32.to_radians(),
            0.05,
            500.0,
        )?;
        let begin = Instant::now();
        let report = engine.render(&camera)?;
        let cpu_ms = begin.elapsed().as_secs_f64() * 1e3;
        let written = frame % args.every == 0 || frame + 1 == args.frames;
        let mut image_hash = None;
        if written {
            let image = engine.capture()?;
            first_image_seconds.get_or_insert_with(|| start.elapsed().as_secs_f64());
            image_hash = Some(format!("{:x}", Sha256::digest(&image.rgba8)));
            write_png(&args.out.join(format!("frame-{frame:03}.png")), &image)?;
        }
        frames.push(serde_json::json!({
            "frame": frame, "renderer_frame": report.frame.0, "cpu_render_ms": cpu_ms,
            "sun_hour": hour, "cpu_sun_update_ms": sun_update_ms,
            "primary": format!("{:?}", report.primary), "soft_shadows": format!("{:?}", report.soft_shadows),
            "draws": report.scene.draws, "triangles": report.scene.triangles, "written": written,
            "rgba8_sha256": image_hash,
        }));
        last_camera = Some(camera);
        // PC1 diagnostic only: a real, visible program edit, then a rejected edit
        // and restoration. These captures are separate from the ordinary frame hashes.
        if args.reload_probe && frame + 1 == args.frames {
            // CC1: a fresh live constant excludes a previously compiled edit from
            // the NVIDIA private cache; ordinary PC1 probes retain their tint.
            let tint = std::env::var("RENDERLAB_RELOAD_PROBE_TINT")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|t| (0.5..1.0).contains(t))
                .unwrap_or(0.83);
            let edit = GAME_PROGRAM.replace(
                "return exposed;",
                &format!("return exposed*vec3<f32>({tint},1.0,1.0);"),
            );
            let edited = Instant::now();
            engine.reload_program(program, &edit)?;
            let compiled_seconds = edited.elapsed().as_secs_f64();
            engine.render(&camera)?;
            let image = engine.capture()?;
            let image_seconds = edited.elapsed().as_secs_f64();
            write_png(&args.out.join("edited.png"), &image)?;
            let rejected = engine
                .reload_program(program, "this is invalid WGSL")
                .is_err();
            if !rejected {
                return Err("invalid program reload was accepted".into());
            }
            engine.render(&camera)?;
            let retained = engine.capture()?;
            write_png(&args.out.join("rejected-retained.png"), &retained)?;
            engine.reload_program(program, GAME_PROGRAM)?;
            engine.render(&camera)?;
            let restored = engine.capture()?;
            write_png(&args.out.join("restored.png"), &restored)?;
            reload = Some(serde_json::json!({"compile_seconds": compiled_seconds,
                "edit_to_readback_seconds": image_seconds, "invalid_edit_rejected": rejected}));
        }
    }
    let compilation: Vec<_> = engine.take_compilation_timings().into_iter().map(|t|
        serde_json::json!({"kind":t.kind,"label":t.label,"ms":t.duration.as_secs_f64()*1e3})).collect();
    let driver: Vec<_> = renderlab::shader_compiler::take_timings(engine.device()).into_iter().map(|t|
        serde_json::json!({"kind":t.kind,"ms":t.duration.as_secs_f64()*1e3,"cache_hit":t.application_cache_hit})).collect();
    let events = engine.poll()?;
    let spans: Vec<f64> = events
        .gpu
        .iter()
        .map(|g| g.render_span.as_secs_f64() * 1e3)
        .collect();
    // A drained segment excludes scene startup and PNG encoding. The final capture
    // drains all submitted work; its copy/readback cost is included in throughput.
    let timing = if args.timing_frames > 0 {
        let camera = last_camera.as_ref().unwrap();
        engine.capture()?;
        engine.poll()?;
        let begin = Instant::now();
        let mut cpu = Vec::new();
        for step in 0..args.timing_frames {
            let submit = Instant::now();
            if args.sun_speed != 0.0 {
                engine.set_sun(scene_sun(sun_hour(args.frames + step)))?;
            }
            engine.render(camera)?;
            cpu.push(submit.elapsed().as_secs_f64() * 1e3);
        }
        engine.capture()?;
        serde_json::json!({"frames":args.timing_frames,
            "wall_ms_per_frame":begin.elapsed().as_secs_f64()*1e3/f64::from(args.timing_frames),
            "cpu_submit_ms":cpu,
            "note":"drained throughput, final colour copy/readback included; no PNG encoding"})
    } else {
        serde_json::Value::Null
    };
    let summary = serde_json::json!({
        "path": format!("{path:?}"), "adapter": engine.adapter_info().name,
        "debug_build": cfg!(debug_assertions), "size": args.size, "exposure_stops": exposure,
        "settings": format!("{:?}", engine.settings()), "objects": engine.object_count(),
        "fox": args.fox, "hour": args.hour,
        "night_lanterns": args.night_lanterns, "local_lights": lanterns(0.0).len(),
        "local_shadow_count":engine.settings().local_shadows.count,
        "quarry": args.quarry, "startup_to_scene_ready_seconds": ready.as_secs_f64(),
        "startup_to_first_image_seconds": first_image_seconds,
        "startup_stages_seconds": {"adapter":adapter_seconds,"engine":engine_seconds-adapter_seconds,
            "programs":programs_seconds-engine_seconds,"scene":ready.as_secs_f64()-programs_seconds},
        "pipeline_cache": args.pipeline_cache, "compilation": compilation, "driver_compilation":driver,
        "preparation":preparation,
        "reload":reload,
        "gpu_render_span_ms": spans, "frames": frames, "timing":timing,
        "note": "shared-GPU diagnostic timings; debug_build records the build profile; not a performance claim",
    });
    fs::write(
        args.out.join("run.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    eprintln!(
        "game_scene frames={} out={} elapsed_seconds={:.1}",
        args.frames,
        args.out.display(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// The walking hero: the skinned Fox playing its walk clip, or two bobbing toon spheres.
enum Hero {
    Fox {
        asset: SkinnedAsset,
        player: ClipPlayer,
        meshes: Vec<(usize, MeshId)>,
        objects: Vec<ObjectId>,
    },
    Spheres([ObjectId; 2]),
}
impl Hero {
    /// Cooks the Fox source (skin recipe, cached beside the output) and spawns it with the
    /// toon program and the Fox's own base-colour texture.
    fn fox(
        engine: &mut Engine,
        source: &std::path::Path,
        out: &std::path::Path,
        toon: ProgramId,
        style: &[(&str, Value)],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let cache = out.parent().unwrap_or(out).join("fox-cache");
        let cooked = renderlab_cook::skin::cook_file(
            source,
            &cache,
            renderlab_cook::Settings {
                normal_maps: true,
                ..Default::default()
            },
            renderlab_cook::CookLimits::default(),
        )?;
        let asset = asset::skin::decode(&fs::read(&cooked.path)?, asset::Limits::default())?;
        let handles = engine.add_skinned_asset(&asset)?;
        let mut meshes = Vec::new();
        let mut objects = Vec::new();
        for (binding, parts) in handles.bindings.iter().enumerate() {
            let mesh = &asset.geometry.meshes[asset.bindings[binding].mesh as usize];
            for ((id, _), primitive) in parts.iter().zip(&mesh.primitives) {
                let source = &asset.geometry.materials[primitive.material as usize];
                let toon = engine.create_material(&MaterialDesc {
                    base_color_texture: source.base_color_texture.map(|t| TextureRef {
                        texture: handles.textures[t.texture as usize],
                        sampler: t.sampler,
                    }),
                    program: Some(toon),
                    parameters: style,
                    ..Default::default()
                })?;
                meshes.push((binding, *id));
                objects.push(engine.spawn(*id, toon, Mat4::IDENTITY)?);
            }
        }
        // The walk clip is the second in the pinned source.
        let player = ClipPlayer::new(usize::from(asset.clips.len() > 1));
        Ok(Self::Fox {
            asset,
            player,
            meshes,
            objects,
        })
    }
    /// Advances the walk to time `t` at `ground` facing `heading` (radians about +Y), and
    /// returns the point the camera looks toward.
    fn update(
        &mut self,
        engine: &mut Engine,
        ground: Vec3,
        heading: f32,
        t: f32,
    ) -> Result<Vec3, Box<dyn std::error::Error>> {
        match self {
            Self::Fox {
                asset,
                player,
                meshes,
                objects,
            } => {
                player.seek(t)?;
                let pose = player.sample(asset)?;
                for &(binding, mesh) in meshes.iter() {
                    engine.set_joint_palette(mesh, &pose.joint_palette(asset, binding)?)?;
                }
                // The Fox is authored in centimetres-scale units facing +Z.
                let model = Mat4::from_translation(ground)
                    * Mat4::from_rotation_y(heading)
                    * Mat4::from_scale(Vec3::splat(0.025));
                for &object in objects.iter() {
                    engine.set_transform(object, model)?;
                }
                Ok(ground + Vec3::new(0.0, 0.8, 0.0))
            }
            Self::Spheres([body, head]) => {
                let center = ground + Vec3::new(0.0, 0.55 + 0.12 * (6.0 * t).sin().abs(), 0.0);
                let ball =
                    |c: Vec3, r: f32| Mat4::from_translation(c) * Mat4::from_scale(Vec3::splat(r));
                engine.set_transform(*body, ball(center, 0.55))?;
                engine.set_transform(*head, ball(center + Vec3::new(0.0, 0.8, 0.0), 0.35))?;
                Ok(center)
            }
        }
    }
}

/// Leaves on a transparent card: overlapping ellipses at hashed positions, RGBA8.
fn leaf_rgba8(size: u32) -> Vec<u8> {
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    // Leaves cluster toward the card's lower middle, like a bush's silhouette.
    let leaves: Vec<[f32; 5]> = (0..90)
        .map(|_| {
            let (x, y) = (0.12 + 0.76 * next(), 0.08 + 0.85 * next());
            [
                x,
                y,
                0.035 + 0.035 * next(),
                0.015 + 0.012 * next(),
                PI * next(),
            ]
        })
        .filter(|l| ((l[0] - 0.5) / 0.45).powi(2) + ((l[1] - 0.55) / 0.5).powi(2) < 1.0)
        .collect();
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = [
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            ];
            let inside = leaves.iter().any(|&[cx, cy, a, b, angle]| {
                let (dx, dy) = (p[0] - cx, p[1] - cy);
                let (u, v) = (
                    dx * angle.cos() + dy * angle.sin(),
                    -dx * angle.sin() + dy * angle.cos(),
                );
                (u / a).powi(2) + (v / b).powi(2) < 1.0
            });
            rgba.extend_from_slice(&[255, 255, 255, if inside { 255 } else { 0 }]);
        }
    }
    rgba
}
/// A vertical card `width` by `height` metres standing on y = 0, facing +Z, uv over [0, 1]
/// with v down.
fn card_geometry(width: f32, height: f32) -> Geometry {
    let corners = [(-0.5, 0.0), (0.5, 0.0), (0.5, 1.0), (-0.5, 1.0)];
    Geometry {
        vertices: corners
            .iter()
            .map(|&(x, y)| {
                vertex(
                    Vec3::new(x * width, y * height, 0.0),
                    Vec3::Z,
                    [x + 0.5, 1.0 - y],
                )
            })
            .collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

/// Vertices and triangles before upload.
struct Geometry {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}
fn mesh(g: &Geometry) -> MeshData<'_> {
    MeshData {
        vertices: &g.vertices,
        indices: &g.indices,
    }
}
fn vertex(position: Vec3, normal: Vec3, uv: [f32; 2]) -> Vertex {
    Vertex {
        position: position.to_array(),
        normal: normal.to_array(),
        uv,
        color: [1.0; 4],
    }
}
/// A unit cube centred on the origin, with flat faces.
fn cube_geometry() -> Geometry {
    let mut g = Geometry {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for (normal, u, v) in [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ] {
        let base = g.vertices.len() as u32;
        for (s, t) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = 0.5 * (normal + s * u + t * v);
            g.vertices
                .push(vertex(p, normal, [0.5 + 0.5 * s, 0.5 - 0.5 * t]));
        }
        g.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    g
}
/// A unit-radius UV sphere with smooth normals.
fn sphere_geometry(segments: u32, rings: u32) -> Geometry {
    let mut g = Geometry {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for r in 0..=rings {
        let polar = PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let azimuth = 2.0 * PI * s as f32 / segments as f32;
            let n = Vec3::new(
                polar.sin() * azimuth.cos(),
                polar.cos(),
                polar.sin() * azimuth.sin(),
            );
            g.vertices.push(vertex(
                n,
                n,
                [s as f32 / segments as f32, r as f32 / rings as f32],
            ));
        }
    }
    let row = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let a = r * row + s;
            let b = a + row;
            g.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    g
}
/// A square in the XZ plane facing +Y, `size` metres across, `cells` quads per side.
fn plane_geometry(size: f32, cells: u32) -> Geometry {
    let mut g = Geometry {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for z in 0..=cells {
        for x in 0..=cells {
            let (u, v) = (x as f32 / cells as f32, z as f32 / cells as f32);
            let p = Vec3::new((u - 0.5) * size, 0.0, (v - 0.5) * size);
            g.vertices.push(vertex(p, Vec3::Y, [u * size, v * size]));
        }
    }
    let row = cells + 1;
    for z in 0..cells {
        for x in 0..cells {
            let a = z * row + x;
            g.indices
                .extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    g
}
/// An orange ring around a five-pointed star on a transparent background, sRGB RGBA8.
fn emblem_rgba8(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = [
                (x as f32 + 0.5) / size as f32 * 2.0 - 1.0,
                (y as f32 + 0.5) / size as f32 * 2.0 - 1.0,
            ];
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
            // Star: radius between 0.3 and 0.62 by angle, five points.
            let angle = p[1].atan2(p[0]);
            let star = 0.3 + 0.32 * (0.5 + 0.5 * (5.0 * angle + PI / 2.0).cos()).powf(3.0);
            let ring = (0.78..0.9).contains(&r);
            let inside = r < star;
            let (color, alpha) = match (ring, inside) {
                (true, _) => ([235, 120, 30], 230),
                (false, true) => ([250, 215, 60], 220),
                _ => ([0, 0, 0], 0),
            };
            rgba.extend_from_slice(&[color[0], color[1], color[2], alpha]);
        }
    }
    rgba
}
/// A soft-edged blob, white with alpha falling off toward its wavy rim (a puddle mask).
fn puddle_rgba8(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = [
                (x as f32 + 0.5) / size as f32 * 2.0 - 1.0,
                (y as f32 + 0.5) / size as f32 * 2.0 - 1.0,
            ];
            let angle = p[1].atan2(p[0]);
            let rim = 0.8 + 0.08 * (3.0 * angle).sin() + 0.05 * (7.0 * angle + 1.0).sin();
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt() / rim;
            let alpha = ((1.0 - r) / 0.15).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[255, 255, 255, (alpha * 255.0).round() as u8]);
        }
    }
    rgba
}
fn write_png(path: &std::path::Path, image: &Image) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(fs::File::create(path)?, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.rgba8)?;
    Ok(())
}
