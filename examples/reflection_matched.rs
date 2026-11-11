//! The reflection measurement scene (M7-RQ1, docs/m7-rq1.md): `examples/game_scene.rs` as it
//! was at main `4c8ef7d`, frozen so that glossy-reflection measurements against the path
//! tracer stay comparable while the game example evolves (dusk, lanterns, glass, foliage).
//!
//! Procedural ground, an arch, crates, a red wall, a glossy gold sphere (roughness 0.25), a
//! mirror chrome sphere (0.02), a spinning glossy crate (0.35) and a bobbing hero, under the
//! physical atmosphere with fog, decals, GI, traced reflections (both paths), TR1 and post,
//! seen from a 48-frame camera fly path. Every `--every`-th frame is written as PNG.
//!
//! Measurement modes: `--matched` renders the lighting the path tracer reproduces (uniform
//! sky; no aerial perspective, fog or decals; the hero on the photoreal program; post off),
//! and `--reference SPP` path-traces the same placements and camera instead
//! (`renderlab::reference`), writing the mean and both half-sample images. `--freeze FRAME`
//! holds every frame at FRAME's time (a still camera at that pose), `--write-frames 8,47`
//! writes exactly those frames, and `--no-post` presents without post processing. The other
//! options are the game example's at that revision: `--sky uniform`, `--no-volumetric-fog`,
//! `--no-gi`, `--no-temporal`, `--render-scale`, `--exposure`, `--sun-speed`, `--sun-after`,
//! `--still`, `--timing-frames N` and `--quarry MANIFEST`. `--hour H` replaces the fixed sun
//! with the game example's sun at that time of day (`Sun::at_hour`, latitude 0.7), and
//! `--diag KEY=VALUE` sets the game example's estimator bisect switches (P-FLAT,
//! docs/p-flat.md). `--lamp SCALE` hangs the game example's arch lamp under the lintel (a white
//! bulb with its point light 0.25 m below, at SCALE times 30,000 cd); in the reference, which
//! has no point lights, a sphere of the light's source radius and intensity stands in for it
//! (U-SPECK, docs/speck.md).
//!
//! P-DEFECT's Showcase-like additions (docs/p-defect.md): `--stage` lays a glossy black
//! dielectric turntable (Ara Showcase's: base 0.06, roughness 0.35) under the chrome and gold
//! spheres, in the reference too; `--glass-balls` adds Showcase's clear and tinted glass balls
//! (opacity 0, IOR 1.5, 0.6 m slab), which the path tracer cannot render.
//! P-SHOW2 (docs/p-show2.md): `--metal-balls` adds Showcase's brushed aluminium (base 0.91,
//! roughness 0.42, metallic 1) and copper (roughness 0.3) balls on the turntable, in the
//! reference too.
//! Sky-map cost (docs/design-sky-map-cost.md): `--sky map` lights the scene with Showcase's
//! gradient sky as a prepared `Sky::Map` (support/gradient_sky.rs; `--sky-face N` sets the
//! field's face size, default 64 as Ara's bridge prepares it), and `--sky map-flat` with the
//! same layout holding its mean radiance.
//!
//! ```text
//! RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK //!     cargo run --example reflection_matched -- --path nonrt --reference 16384 --write-frames 47 --out REF
//! cargo run --example reflection_matched -- --path rt --matched --write-frames 16,32,47 --out RUN
//! ```
#[path = "../support/estimator_diag.rs"]
#[allow(dead_code)]
mod diag;
#[path = "../support/environment_controls.rs"]
#[allow(dead_code)]
mod environment_controls;
#[path = "../support/gradient_sky.rs"]
mod gradient_sky;
#[path = "../support/limits.rs"]
mod policy;

use renderlab::{asset, engine::*, reference};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, f32::consts::PI, fs, path::PathBuf, time::Instant};

const GAME_PROGRAM: &str = include_str!("game_scene.wgsl");
const TOON_PROGRAM: &str = include_str!("game_toon.wgsl");
const FPS: f32 = 30.0;
/// Vertical field of view, degrees.
const FOV_Y: f32 = 55.0;
/// `--sky uniform` and `--matched`: a constant blue sky, cd/m².
const UNIFORM_SKY: [f32; 3] = [3500.0, 5000.0, 7500.0];

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
    /// `--sky map` (Some(false)) or `map-flat` (Some(true)): support/gradient_sky.rs.
    sky_map: Option<bool>,
    sky_face: u32,
    volumetric_fog: Option<bool>,
    sun_speed: f32,
    sun_after: u32,
    still: bool,
    timing_frames: u32,
    pipeline_cache: Option<PathBuf>,
    reload_probe: bool,
    matched: bool,
    post: bool,
    reference: Option<u32>,
    freeze: Option<u32>,
    write_frames: Option<Vec<u32>>,
    hour: Option<f32>,
    diag: Vec<(String, String)>,
    /// `--lamp SCALE`: the game example's arch lamp, a white bulb with its point light 0.25 m
    /// below, at SCALE times its candela.
    lamp: Option<f32>,
    stage: bool,
    glass_balls: bool,
    metal_balls: bool,
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
            sky_map: None,
            sky_face: 64,
            volumetric_fog: None,
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
            matched: false,
            post: true,
            reference: None,
            freeze: None,
            write_frames: None,
            hour: None,
            diag: Vec::new(),
            lamp: None,
            stage: false,
            glass_balls: false,
            metal_balls: false,
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
                    (a.uniform_sky, a.sky_map) = match value()?.as_str() {
                        "atmosphere" => (false, None),
                        "uniform" => (true, None),
                        "map" => (false, Some(false)),
                        "map-flat" => (false, Some(true)),
                        other => {
                            return Err(format!(
                                "--sky atmosphere|uniform|map|map-flat, not {other}"
                            )
                            .into());
                        }
                    }
                }
                "--sky-face" => a.sky_face = value()?.parse()?,
                "--volumetric-fog" => a.volumetric_fog = Some(true),
                "--no-volumetric-fog" => a.volumetric_fog = Some(false),
                "--no-temporal" => a.temporal = false,
                "--sun-speed" => a.sun_speed = value()?.parse()?,
                "--sun-after" => a.sun_after = value()?.parse()?,
                "--still" => a.still = true,
                "--timing-frames" => a.timing_frames = value()?.parse()?,
                "--matched" => a.matched = true,
                "--no-post" => a.post = false,
                "--reference" => a.reference = Some(value()?.parse::<u32>()?.max(2)),
                "--freeze" => a.freeze = Some(value()?.parse()?),
                "--hour" => a.hour = Some(value()?.parse()?),
                "--lamp" => a.lamp = Some(value()?.parse()?),
                "--stage" => a.stage = true,
                "--glass-balls" => a.glass_balls = true,
                "--metal-balls" => a.metal_balls = true,
                "--diag" => {
                    let v = value()?;
                    let (key, setting) = v.split_once('=').ok_or("--diag KEY=VALUE")?;
                    a.diag.push((key.into(), setting.into()));
                }
                "--write-frames" => {
                    a.write_frames = Some(
                        value()?
                            .split(',')
                            .map(str::parse)
                            .collect::<Result<_, _>>()?,
                    )
                }
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
        if a.hour.is_some() && a.sun_speed != 0.0 {
            return Err("--hour sets a fixed sun; --sun-speed turns the default sun only".into());
        }
        if a.reference.is_some() && (a.quarry.is_some() || a.sun_speed != 0.0) {
            return Err("--reference traces the procedural scene under a fixed sun only".into());
        }
        if a.reference.is_some() && a.glass_balls {
            return Err("the path tracer has no transparent materials (--glass-balls)".into());
        }
        // The path tracer reproduces only the matched lighting.
        a.matched |= a.reference.is_some();
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
        quarry
            .as_ref()
            .and_then(|m| m["exposure_stops"].as_f64())
            .map_or(-14.5, |s| s as f32)
    });
    let mut preparation = Vec::new();
    let diag = diag::Diag::parse(&args.diag)?;

    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            pipeline_cache: args.pipeline_cache.clone(),
            settings: diag.settings(Settings {
                global_illumination: args.gi,
                temporal_reconstruction: args.temporal,
                // Sun shafts through a light haze (0.01 per metre over the first 128 m).
                volumetric_fog: match args.volumetric_fog {
                    _ if args.matched => None,
                    Some(true) => Some(VolumetricFog {
                        extra_density: 0.01,
                        ..Default::default()
                    }),
                    Some(false) => None,
                    None => Settings::default().volumetric_fog,
                },
                aerial_perspective: !args.matched,
                height_fog: Settings::default().height_fog.filter(|_| !args.matched),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let view_desc = ViewDesc {
        render_scale: args.scale,
        look: diag.look(ViewLook {
            exposure: Exposure::Manual { stops: exposure },
            ..Default::default()
        }),
        ..ViewDesc::new(args.size[0], args.size[1])
    };
    let view = engine.create_view(&view_desc)?;
    diag.apply(&mut engine, path, args.scale, view_desc.look.quality)?;
    let engine_seconds = start.elapsed().as_secs_f64();
    if args.matched || !args.post {
        engine
            .renderer_mut()
            .set_post_settings(renderlab::PostSettings::disabled())?;
    }
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
    // Photoreal materials, recorded with their standard-model parameters for `--reference`.
    let mut traced_materials = HashMap::new();
    let mut material = |engine: &mut Engine, color: [f32; 3], metallic: f32, roughness: f32| {
        let id = engine.create_material(&MaterialDesc {
            base_color: [color[0], color[1], color[2], 1.0],
            metallic,
            roughness,
            program: Some(program),
            ..Default::default()
        })?;
        traced_materials.insert(
            id,
            reference::Material {
                base_color: color,
                metallic,
                roughness,
                ..Default::default()
            },
        );
        Ok::<_, Error>(id)
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
    // Matched runs shade the hero with the photoreal program: the path tracer has no toon.
    let toon = if args.matched {
        material(&mut engine, [0.95, 0.55, 0.2], 0.0, 0.5)?
    } else {
        engine.create_material(&MaterialDesc {
            base_color: [0.95, 0.55, 0.2, 1.0],
            program: Some(toon_program),
            parameters: &hero_style,
            ..Default::default()
        })?
    };

    // Meshes and placements are also recorded for `--reference`.
    let mut traced_meshes = HashMap::new();
    let mut placed: Vec<Placed> = Vec::new();
    let cube_mesh = cube_geometry();
    let cube = engine.create_mesh(&mesh(&cube_mesh))?;
    traced_meshes.insert(cube, cube_mesh);
    let sphere_mesh = sphere_geometry(48, 24);
    let sphere = engine.create_mesh(&mesh(&sphere_mesh))?;
    traced_meshes.insert(sphere, sphere_mesh);
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
            let plane_mesh = plane_geometry(60.0, 8);
            let plane = engine.create_mesh(&mesh(&plane_mesh))?;
            traced_meshes.insert(plane, plane_mesh);
            spawn(&mut engine, &mut placed, plane, ground, Mat4::IDENTITY)?;
            // The physical atmosphere is the default sky (AT1); `--sky uniform` swaps in a
            // constant blue sky for comparison.
            if args.uniform_sky || args.matched {
                engine.set_sky(Sky::Uniform(UNIFORM_SKY))?;
            } else if let Some(flat) = args.sky_map {
                let map =
                    engine.create_environment(&gradient_sky::prepare(args.sky_face, flat)?)?;
                engine.set_sky(Sky::Map {
                    map,
                    rotation_y: 0.0,
                    intensity: 1.0,
                    background: Background::Sky,
                })?;
            }
            Vec3::ZERO
        }
    };
    let at = |x: f32, y: f32, z: f32| anchor + Vec3::new(x, y, z);
    let boxed = |center: Vec3, size: Vec3| Mat4::from_translation(center) * Mat4::from_scale(size);
    // The arch: two pillars and a lintel.
    for x in [-2.2, 2.2] {
        spawn(
            &mut engine,
            &mut placed,
            cube,
            stone,
            boxed(at(x, 2.0, -3.0), Vec3::new(0.8, 4.0, 0.8)),
        )?;
    }
    spawn(
        &mut engine,
        &mut placed,
        cube,
        stone,
        boxed(at(0.0, 4.4, -3.0), Vec3::new(5.2, 0.8, 1.0)),
    )?;
    // U-SPECK (docs/speck.md): the game example's arch lamp, a white bulb hanging under the
    // lintel with its point light 0.25 m below it, so the bulb's underside is 0.13 m from the
    // light: the small bright surface the GI gather must resolve. The reference has no point
    // lights: there a sphere of the light's source radius and the same intensity stands in for
    // it (its radiance is the intensity over pi r^2).
    let arch_lamp = at(0.0, 3.55, -3.0);
    const LAMP_SOURCE_RADIUS: f32 = 0.12;
    let lamp = args.lamp.map(|scale| {
        let candela = [30_000.0 * scale, 21_000.0 * scale, 11_000.0 * scale];
        (arch_lamp - Vec3::new(0.0, 0.25, 0.0), candela)
    });
    if let Some((position, candela)) = lamp {
        let bulb = material(&mut engine, [1.0, 0.9, 0.7], 0.0, 0.9)?;
        spawn(
            &mut engine,
            &mut placed,
            sphere,
            bulb,
            boxed(arch_lamp, Vec3::splat(0.12)),
        )?;
        engine.set_lights(&diag.lights(vec![Light::Point {
            position,
            candela,
            range: None,
            source_radius: LAMP_SOURCE_RADIUS,
        }]))?;
    }
    // A red wall beside it, whose colour should bleed onto the ground and hero.
    spawn(
        &mut engine,
        &mut placed,
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
        spawn(
            &mut engine,
            &mut placed,
            cube,
            wood,
            Mat4::from_translation(at(x, y, z)) * Mat4::from_rotation_y(turn),
        )?;
    }
    spawn(
        &mut engine,
        &mut placed,
        sphere,
        gold,
        boxed(at(1.8, 1.0, 3.2), Vec3::splat(1.0)),
    )?;
    // A mirror-polished chrome sphere: traced mirror reflections on RT (M7-R1).
    spawn(
        &mut engine,
        &mut placed,
        sphere,
        chrome,
        boxed(at(-1.2, 0.8, 5.6), Vec3::splat(0.8)),
    )?;
    let spinner = spawn(
        &mut engine,
        &mut placed,
        cube,
        paint,
        Mat4::from_translation(at(-2.5, 0.6, 3.0)),
    )?;
    if args.stage {
        // Showcase's turntable: glossy black dielectric, traced glossy (16 rays) on both paths.
        let turntable = material(&mut engine, [0.06, 0.06, 0.065], 0.0, 0.35)?;
        spawn(
            &mut engine,
            &mut placed,
            cube,
            turntable,
            boxed(at(0.3, 0.01, 6.0), Vec3::new(7.0, 0.02, 4.0)),
        )?;
    }
    if args.metal_balls {
        // Showcase's flat metal balls (Ara samples, showcase.rs), resting on the turntable.
        for (center, size, base, roughness) in [
            (at(0.8, 0.52, 6.6), 1.0, [0.91, 0.92, 0.92], 0.42),
            (at(-2.3, 0.42, 7.2), 0.8, [0.95, 0.64, 0.54], 0.3),
        ] {
            let metal = material(&mut engine, base, 1.0, roughness)?;
            spawn(
                &mut engine,
                &mut placed,
                sphere,
                metal,
                boxed(center, Vec3::splat(size)),
            )?;
        }
    }
    if args.glass_balls {
        for (center, base) in [
            (at(3.0, 0.5, 4.8), [1.0, 1.0, 1.0]),
            (at(-3.2, 0.5, 6.4), [0.55, 0.8, 0.65]),
        ] {
            let glass = engine.create_material(&MaterialDesc {
                base_color: [base[0], base[1], base[2], 1.0],
                roughness: 0.0,
                transparency: Some(Transparency {
                    opacity: 0.0,
                    ior: 1.5,
                    thickness: 0.6,
                }),
                ..Default::default()
            })?;
            engine.spawn(sphere, glass, boxed(center, Vec3::splat(0.5)))?;
        }
    }
    let hero_body = spawn(&mut engine, &mut placed, sphere, toon, Mat4::IDENTITY)?;
    let hero_head = spawn(&mut engine, &mut placed, sphere, toon, Mat4::IDENTITY)?;

    let sun = match args.hour {
        Some(hour) => Sun::at_hour(hour, 0.7).ok_or("--hour puts the sun below the horizon")?,
        None => Sun {
            travel: Vec3::new(-0.45, -0.75, -0.5),
            // Under the atmosphere this is the sun outside it (`Sun::default()`, 120 klux), which
            // its transmittance dims and warms; under `--sky uniform` it arrives as given.
            ..Default::default()
        },
    };
    engine.set_sun(Some(sun))?;
    // A warm lantern under the arch arrives with many-light shading (LC1): at this
    // revision a soft sun must be the only light (docs/api.md, "Current limits").

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
    // The path tracer has no decals, so matched runs leave them out.
    let decals = [
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
    ];
    engine.set_decals(if args.matched { &[] } else { &decals })?;

    fs::create_dir_all(&args.out)?;
    let ready = start.elapsed();
    let mut frames = Vec::new();
    let mut last_camera = None;
    let mut first_image_seconds = None;
    let mut reload = None;
    let traced = match args.reference {
        Some(samples) => Some(Traced::new(
            samples,
            &traced_meshes,
            &traced_materials,
            sun,
            lamp.map(|(position, candela)| Emitter {
                mesh: sphere,
                transform: boxed(position, Vec3::splat(LAMP_SOURCE_RADIUS)),
                radiance: candela
                    .map(|c| c / (std::f32::consts::PI * LAMP_SOURCE_RADIUS * LAMP_SOURCE_RADIUS)),
            }),
        )?),
        None => None,
    };
    for frame in 0..args.frames {
        let written = match &args.write_frames {
            Some(list) => list.contains(&frame),
            None => frame % args.every == 0 || frame + 1 == args.frames,
        };
        if traced.is_some() && !written {
            continue;
        }
        let t = match args.freeze {
            _ if args.still => 0.0,
            Some(held) => held as f32 / FPS,
            None => frame as f32 / FPS,
        };
        let sun_update = Instant::now();
        let sun_angle =
            (frame.saturating_sub(args.sun_after) as f32 / FPS * args.sun_speed).to_radians();
        if args.sun_speed != 0.0 && frame >= args.sun_after {
            engine.set_sun(Some(Sun {
                travel: Mat4::from_rotation_z(sun_angle)
                    .transform_vector3(Vec3::new(-0.45, -0.75, -0.5)),
                ..Default::default()
            }))?;
        }
        let sun_update_ms = sun_update.elapsed().as_secs_f64() * 1e3;
        // The hero walks a circle through the arch and bobs.
        let walk = 0.35 * t;
        let hero = at(
            3.0 * walk.sin(),
            0.55 + 0.12 * (6.0 * t).sin().abs(),
            -1.0 + 3.0 * walk.cos(),
        );
        let moved = [
            (hero_body, boxed(hero, Vec3::splat(0.55))),
            (
                hero_head,
                boxed(hero + Vec3::new(0.0, 0.8, 0.0), Vec3::splat(0.35)),
            ),
            (
                spinner,
                Mat4::from_translation(at(-2.5, 0.6 + 0.3 * (2.0 * t).sin(), 3.0))
                    * Mat4::from_rotation_y(1.5 * t)
                    * Mat4::from_rotation_x(0.6 * t),
            ),
        ];
        // Fly path: a slow orbit that descends and closes in on the hero.
        let orbit = 0.6 + 0.25 * t;
        let radius = 14.0 - 4.0 * (t / 2.0).min(1.0);
        let eye = anchor
            + Vec3::new(
                radius * orbit.sin(),
                5.5 - 2.0 * (t / 2.0).min(1.0),
                radius * orbit.cos(),
            );
        let target = hero.lerp(anchor + Vec3::Y, 0.5);
        if let Some(traced) = &traced {
            let begin = Instant::now();
            let (noise, invalid) = traced.render(
                &placed,
                &moved,
                reference::Pose { eye, target },
                args.size,
                exposure,
                &args.out.join(format!("frame-{frame:03}")),
            )?;
            frames.push(serde_json::json!({
                "frame": frame, "trace_seconds": begin.elapsed().as_secs_f64(), "samples": traced.samples,
                "eye": eye.to_array(), "target": target.to_array(),
                "split_half_noise_codes_rms": noise, "invalid_pixels": invalid,
            }));
            continue;
        }
        for (object, transform) in moved {
            engine.set_transform(object, transform)?;
        }
        let camera = Camera::look_at(eye, target, Vec3::Y, FOV_Y.to_radians(), 0.05, 500.0)?;
        let begin = Instant::now();
        engine.tick(1.0 / 60.0)?;
        let report = engine.render_view(view, &camera)?;
        let cpu_ms = begin.elapsed().as_secs_f64() * 1e3;
        let mut image_hash = None;
        if written {
            let image = engine.capture_view(view)?;
            first_image_seconds.get_or_insert_with(|| start.elapsed().as_secs_f64());
            image_hash = Some(format!("{:x}", Sha256::digest(&image.rgba8)));
            write_png(&args.out.join(format!("frame-{frame:03}.png")), &image)?;
        }
        frames.push(serde_json::json!({
            "frame": frame, "renderer_frame": report.frame.0, "cpu_render_ms": cpu_ms,
            "sun_angle_degrees": sun_angle.to_degrees(), "cpu_sun_update_ms": sun_update_ms,
            "primary": format!("{:?}", report.primary), "soft_shadows": format!("{:?}", report.soft_shadows),
            "draws": report.scene.draws, "triangles": report.scene.triangles, "written": written,
            "rgba8_sha256": image_hash, "eye": eye.to_array(), "target": target.to_array(),
        }));
        last_camera = Some(camera);
        // PC1 diagnostic only: a real, visible program edit, then a rejected edit
        // and restoration. These captures are separate from the ordinary frame hashes.
        if args.reload_probe && frame + 1 == args.frames {
            let edit =
                GAME_PROGRAM.replace("return exposed;", "return exposed*vec3<f32>(0.83,1.0,1.0);");
            let edited = Instant::now();
            engine.reload_program(program, &edit)?;
            let compiled_seconds = edited.elapsed().as_secs_f64();
            engine.tick(1.0 / 60.0)?;
            engine.render_view(view, &camera)?;
            let image = engine.capture_view(view)?;
            let image_seconds = edited.elapsed().as_secs_f64();
            write_png(&args.out.join("edited.png"), &image)?;
            let rejected = engine
                .reload_program(program, "this is invalid WGSL")
                .is_err();
            if !rejected {
                return Err("invalid program reload was accepted".into());
            }
            engine.tick(1.0 / 60.0)?;
            engine.render_view(view, &camera)?;
            let retained = engine.capture_view(view)?;
            write_png(&args.out.join("rejected-retained.png"), &retained)?;
            engine.reload_program(program, GAME_PROGRAM)?;
            engine.tick(1.0 / 60.0)?;
            engine.render_view(view, &camera)?;
            let restored = engine.capture_view(view)?;
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
        engine.capture_view(view)?;
        engine.poll()?;
        let begin = Instant::now();
        let mut cpu = Vec::new();
        for step in 0..args.timing_frames {
            let submit = Instant::now();
            if args.sun_speed != 0.0 {
                let angle = ((args.frames + step).saturating_sub(args.sun_after) as f32 / FPS
                    * args.sun_speed)
                    .to_radians();
                engine.set_sun(Some(Sun {
                    travel: Mat4::from_rotation_z(angle)
                        .transform_vector3(Vec3::new(-0.45, -0.75, -0.5)),
                    ..Default::default()
                }))?;
            }
            engine.tick(1.0 / 60.0)?;
            engine.render_view(view, camera)?;
            cpu.push(submit.elapsed().as_secs_f64() * 1e3);
        }
        engine.capture_view(view)?;
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
        "hour": args.hour, "diag": args.diag,
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

/// One spawned object, recorded for the path-traced reference.
struct Placed {
    object: ObjectId,
    mesh: MeshId,
    material: MaterialId,
    transform: Mat4,
}
/// Spawns an object and records its placement for `--reference`.
fn spawn(
    engine: &mut Engine,
    placed: &mut Vec<Placed>,
    mesh: MeshId,
    material: MaterialId,
    transform: Mat4,
) -> Result<ObjectId, Error> {
    let object = engine.spawn(mesh, material, transform)?;
    placed.push(Placed {
        object,
        mesh,
        material,
        transform,
    });
    Ok(object)
}

/// A point light's stand-in in the path-traced reference: a black sphere emitting `radiance`.
struct Emitter {
    mesh: MeshId,
    transform: Mat4,
    radiance: [f32; 3],
}
/// The path-traced reference of the matched scene (`--reference SPP`): the same meshes,
/// standard-model materials, sun disk and uniform sky, on its own ray-query device.
struct Traced {
    /// The lamp's stand-in: its mesh, its material's index and its transform.
    emitter: Option<(usize, usize, Mat4)>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    samples: u32,
    meshes: Vec<reference::Mesh>,
    mesh_index: HashMap<MeshId, usize>,
    materials: Vec<reference::Material>,
    material_index: HashMap<MaterialId, usize>,
    sun: reference::Sun,
}
impl Traced {
    fn new(
        samples: u32,
        meshes: &HashMap<MeshId, Geometry>,
        materials: &HashMap<MaterialId, reference::Material>,
        sun: Sun,
        emitter: Option<Emitter>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
            != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        {
            return Err("--reference needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK".into());
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        let a = adapter.limits();
        let limits = renderlab::traced_limits(wgpu::Limits {
            max_blas_primitive_count: a.max_blas_primitive_count,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
            max_acceleration_structures_per_shader_stage: 1,
            max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
            max_buffer_size: a.max_buffer_size,
            ..Default::default()
        });
        // SAFETY: the caller accepted wgpu's experimental ray-query risk in the environment
        // (checked above), as the GPU test suites require.
        let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_features: reference::FEATURES | renderlab::TRACED_FEATURES,
                required_limits: limits,
                experimental_features,
                memory_hints: renderlab::MEMORY_HINTS,
                ..Default::default()
            }))?;
        let mut mesh_index = HashMap::new();
        let meshes = meshes
            .iter()
            .enumerate()
            .map(|(k, (id, g))| {
                mesh_index.insert(*id, k);
                reference::Mesh {
                    positions: g.vertices.iter().map(|v| v.position).collect(),
                    normals: g.vertices.iter().map(|v| v.normal).collect(),
                    uvs: g.vertices.iter().map(|v| v.uv).collect(),
                    indices: g.indices.clone(),
                    ..Default::default()
                }
            })
            .collect();
        let mut material_index = HashMap::new();
        let mut materials: Vec<reference::Material> = materials
            .iter()
            .enumerate()
            .map(|(k, (id, m))| {
                material_index.insert(*id, k);
                *m
            })
            .collect();
        let emitter = emitter.map(|e| {
            materials.push(reference::Material {
                base_color: [0.0; 3],
                roughness: 1.0,
                emission: e.radiance,
                ..Default::default()
            });
            (mesh_index[&e.mesh], materials.len() - 1, e.transform)
        });
        Ok(Self {
            emitter,
            device,
            queue,
            samples,
            meshes,
            mesh_index,
            materials,
            material_index,
            sun: reference::Sun {
                to_light: -sun.travel.normalize(),
                illuminance: sun.illuminance,
                angular_radius: sun.angular_radius,
            },
        })
    }

    /// Traces one frame and writes `{stem}.png` plus its two independent half-sample
    /// images (`-a`, `-b`) in the legacy display transform (exposure, x/(1+x), sRGB).
    /// Returns the split-half noise of the mean in display codes (RMS) and the invalid
    /// pixel count.
    fn render(
        &self,
        placed: &[Placed],
        moved: &[(ObjectId, Mat4)],
        pose: reference::Pose,
        size: [u32; 2],
        exposure_stops: f32,
        stem: &std::path::Path,
    ) -> Result<(f64, u32), Box<dyn std::error::Error>> {
        let instances = placed
            .iter()
            .map(|p| reference::Instance {
                mesh: self.mesh_index[&p.mesh],
                material: self.material_index[&p.material],
                transform: moved
                    .iter()
                    .find(|(o, _)| *o == p.object)
                    .map_or(p.transform, |(_, t)| *t),
            })
            .chain(
                self.emitter
                    .map(|(mesh, material, transform)| reference::Instance {
                        mesh,
                        material,
                        transform,
                    }),
            )
            .collect();
        let scene = reference::Scene {
            meshes: self.meshes.clone(),
            materials: self.materials.clone(),
            instances,
            sun: Some(self.sun),
            sky: UNIFORM_SKY,
            ..Default::default()
        };
        let view = reference::View {
            open: pose,
            close: pose,
            up: Vec3::Y,
            fov_y: FOV_Y.to_radians(),
            width: size[0],
            height: size[1],
        };
        let settings = reference::Settings {
            samples: self.samples,
            ..Default::default()
        };
        let image = reference::render(&self.device, &self.queue, &scene, &view, &settings)?;
        let scale = exposure_stops.exp2();
        let display = |pixels: &[[f32; 3]]| -> Vec<u8> {
            pixels
                .iter()
                .flat_map(|p| {
                    let [r, g, b] = p.map(|c| {
                        let x = (c * scale).max(0.0);
                        (srgb(x / (1.0 + x)) * 255.0).round() as u8
                    });
                    [r, g, b, 255]
                })
                .collect()
        };
        let [a, b] = [display(&image.halves[0]), display(&image.halves[1])];
        // The mean's standard error is half the halves' difference.
        let noise = (a
            .iter()
            .zip(&b)
            .enumerate()
            .filter(|(k, _)| k % 4 != 3)
            .map(|(_, (x, y))| (f64::from(*x) - f64::from(*y)).powi(2) / 4.0)
            .sum::<f64>()
            / (image.mean.len() * 3) as f64)
            .sqrt();
        let name = stem.file_name().unwrap_or_default().to_string_lossy();
        for (suffix, rgba8) in [("", display(&image.mean)), ("-a", a), ("-b", b)] {
            write_png(
                &stem.with_file_name(format!("{name}{suffix}.png")),
                &Image {
                    width: size[0],
                    height: size[1],
                    rgba8,
                },
            )?;
        }
        Ok((noise, image.invalid_pixels))
    }
}
/// The sRGB transfer function of the engine's sRGB output format.
fn srgb(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
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
