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
//! `--local-shadow-count N` (0-128 map lights, positive enables all-light RT),
//! `--atmosphere-light K` (`Settings::sky_light`: the atmosphere lights at K), `--sky
//! uniform`, `--sky map` or `map-flat` (Showcase's gradient sky as a prepared map, or the
//! same layout at its mean radiance; support/gradient_sky.rs, face size `--sky-face N`;
//! `--sky-light K` lights with the map at K while drawing it at 1, `Background::ScaledSky`),
//! `--no-volumetric-fog`, `--no-gi`, `--no-temporal`, `--no-post`,
//! `--no-reflections` (every program keeps its untraced estimate), `--render-scale`,
//! `--exposure`, and `--quarry MANIFEST`,
//! which replaces the ground with the registered quarry scans and its RLE2 sky (manifest
//! keys `asset`, `limits`, `environment`, `exposure_stops`). `--diag KEY=VALUE` (repeatable)
//! sets estimator switches the public settings do not expose, for bisecting image defects
//! (P-FLAT, docs/p-flat.md; keys in `Diag::parse`). P-RELIT's scene events (docs/p-relit.md)
//! happen at frame 24: `fire=off` or `fire=on` switches the fire light, `fire=move` circles
//! it from the first frame, `glow=step` dims the lamp bulbs' emission to a tenth, and `cut=1`
//! jumps the camera a third of the way round its orbit with `Engine::cut_view`; `cut=turn` cuts
//! the field or forest walk there by turning it 150 degrees in place. `hold=1` keeps
//! the camera at its first pose while the lights, the hero and the objects animate;
//! `hold=pose` (U-FIXED) also keeps the objects and the hero's placement still while the
//! Fox plays its walk in place, so the skinned mesh is the only mover. `hold=cut` (U-RTVIS)
//! holds the first pose, jumps to the `cut=1` pose at frame 24 with `cut_view` and holds that,
//! while the lights, the hero and the objects animate: a respawn under a still camera, as Ara's
//! Skyward bridge cuts on every respawn. `fox=2` (U-FIXED,
//! with `--fox`) adds a second Fox walking the same circle half a turn behind: two skinned
//! characters, as on Ara's Showcase.
//! `--views a|b|ab|ai` (docs/design-views.md) picks which of two views render each tick: `a`
//! is the fly path, `b` a second camera across the scene, still unless `--b-orbit`. With `ab`
//! both render from one engine and each frame records both hashes, so a run of `a` or `b`
//! alone checks that each view matches a single-view engine; `ai` is `a` with a second view
//! created and never rendered. `--cut-b FRAME` jumps view b's camera and cuts it there.
//! `--diag casts=toggle` (U-FIXED, `Engine::set_casts_shadows`): the crates stop casting shadows
//! at frame 12 and cast again from frame 24, on the flight's moving camera.
//! `--diag forest=1` (U-RTVIS, docs/rtvis.md) swaps the flat ground for a 200 m rolling
//! terrain (16 tiles) under trees and bushes of cutout cards and walks the field walk's camera
//! through it: Outpost's layered ground and foliage, for the RT visibility pass's cost.
//! `forest=bare` is the same terrain and walk without the foliage. `--diag floor=R` gives the
//! flat ground perceptual roughness R (0.95 otherwise): a glossy floor for the reflection strata.
//! `--diag cut=outpost --size 1920x1080` selects the separate Outpost-like forest fixture:
//! hold400, then the original120-frame clip's slow motion, with the(-20,0,15)m/140-degree
//! cut at frame460 and25 constant local lights around the two arrivals. Existing clips stay
//! unchanged. RENDERLAB_SCENE_EVENT_FRAME may move this cut (the hold ends60 frames earlier).
//! `--diag program=standard` puts the scene's `game` materials on the standard program, which
//! reports its reflectance to the deferred reflection composite (RENDERLAB_RL_DEFERRED=1).
//! `--diag far=1` (U-FIXED) holds a camera 40 m up over a 2.4 km ground with columns at 80,
//! 200, 300 and 800 m whose sun shadows cross the view (the sun's coverage, Ara #185 item 33);
//! `far=move` slides that camera sideways 0.25 m a frame (far shadows in motion).
//! `far=low` puts the camera 1.7 m up with a body-sized box 0.3 m beside it and columns at 50,
//! 200 and 1000 m (the clipmap's reach from a caster at the lens); `far=box` puts the same eye
//! inside a diagonal wall's bounds instead. `lowmove` and `boxmove` slide them with the caster.
//!
//! ```text
//! cargo run --example game_scene -- --path nonrt --out D:/Coding/renderlab-work/api1/nonrt
//! RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK \
//!     cargo run --example game_scene -- --path rt --out D:/Coding/renderlab-work/api1/rt
//! ```
#[path = "../support/estimator_diag.rs"]
mod diag;
#[path = "../support/environment_controls.rs"]
#[allow(dead_code)]
mod environment_controls;
#[path = "../support/gradient_sky.rs"]
mod gradient_sky;
#[path = "../support/limits.rs"]
mod policy;
#[path = "game_scene_outpost_cut.rs"]
mod outpost_cut;

use diag::Diag;
use renderlab::{asset, engine::*};
use sha2::{Digest, Sha256};
use std::{f32::consts::PI, fs, path::PathBuf, time::Instant};

const GAME_PROGRAM: &str = include_str!("game_scene.wgsl");
const TOON_PROGRAM: &str = include_str!("game_toon.wgsl");
// PC1/P-SHC: include the current programmable emission term in a real visible edit.
const GAME_RETURN: &str =
    "return min(exposed+expose(max(params.glow,vec3<f32>(0.0))),vec3<f32>(65504.0));";
fn tinted_program(tint: f32) -> String {
    assert_eq!(
        GAME_PROGRAM.matches(GAME_RETURN).count(),
        1,
        "reload probe return seam moved"
    );
    let expression = GAME_RETURN
        .strip_prefix("return ")
        .unwrap()
        .strip_suffix(';')
        .unwrap();
    GAME_PROGRAM.replace(
        GAME_RETURN,
        &format!("return ({expression})*vec3<f32>({tint},1.0,1.0);"),
    )
}
const FPS: f32 = 30.0;
/// The frame of P-RELIT's scene events (`--diag fire=off|on`, `glow=step`, `cut=1`).
const EVENT_FRAME: u32 = 24;
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
    /// `--sky map` (Some(false)) or `map-flat` (Some(true)): support/gradient_sky.rs.
    sky_map: Option<bool>,
    sky_face: u32,
    /// `--sky-light K`: the map lights at K and is drawn at 1 (`Background::ScaledSky`).
    sky_light: Option<f32>,
    volumetric_fog: Option<bool>,
    post: bool,
    glass: bool,
    /// `--glass-crowd N`: N more panes of the glass draw, overlapping on screen (one draw of
    /// many overlapping instances, as Ara Outpost's foliage and fences). A frame-time gate case.
    glass_crowd: u32,
    /// `--crowd-films`: the crowd's panes are films (IOR 1, no thickness: plain blended
    /// coverage, as Outpost's fences, foliage and windows), one draw of their own.
    crowd_films: bool,
    particles: bool,
    local_shadows: bool,
    reflections: bool,
    hour: f32,
    fox: Option<PathBuf>,
    sun_speed: f32,
    /// `--atmosphere-light K`: `Settings::sky_light`, the atmosphere's light over its look.
    atmosphere_light: f32,
    sun_after: u32,
    still: bool,
    /// `--views`: which of the two cameras render (`a`, `b` or `ab`).
    views: String,
    b_orbit: bool,
    /// `--cut-b FRAME`: view b's camera jumps across the scene there, with `cut_view`.
    cut_b: Option<u32>,
    timing_frames: u32,
    pipeline_cache: Option<PathBuf>,
    reload_probe: bool,
    night_lanterns: bool,
    local_shadow_count: Option<u32>,
    diag: Vec<(String, String)>,
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
            sky_light: None,
            volumetric_fog: None,
            post: true,
            glass: true,
            glass_crowd: 0,
            crowd_films: false,
            particles: true,
            local_shadows: true,
            reflections: true,
            hour: 17.5,
            fox: std::env::var_os("RENDERLAB_FOX_GLB").map(PathBuf::from),
            sun_speed: 0.0,
            atmosphere_light: 1.0,
            sun_after: 0,
            still: false,
            views: "a".into(),
            b_orbit: false,
            cut_b: None,
            timing_frames: 0,
            pipeline_cache: Some(
                std::env::var_os("RENDERLAB_PIPELINE_CACHE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| "measurements/pipeline-cache".into()),
            ),
            reload_probe: false,
            night_lanterns: false,
            local_shadow_count: None,
            diag: Vec::new(),
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
                "--sky-light" => a.sky_light = Some(value()?.parse()?),
                "--volumetric-fog" => a.volumetric_fog = Some(true),
                "--no-volumetric-fog" => a.volumetric_fog = Some(false),
                "--no-temporal" => a.temporal = false,
                "--no-post" => a.post = false,
                "--no-glass" => a.glass = false,
                "--glass-crowd" => a.glass_crowd = value()?.parse()?,
                "--crowd-films" => a.crowd_films = true,
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
                "--atmosphere-light" => a.atmosphere_light = value()?.parse()?,
                "--sun-after" => a.sun_after = value()?.parse()?,
                "--still" => a.still = true,
                "--views" => {
                    a.views = value()?;
                    if !["a", "b", "ab", "ai"].contains(&a.views.as_str()) {
                        return Err("--views a|b|ab|ai".into());
                    }
                }
                "--b-orbit" => a.b_orbit = true,
                "--cut-b" => a.cut_b = Some(value()?.parse()?),
                "--night-lanterns" => a.night_lanterns = true,
                "--local-shadow-count" => a.local_shadow_count = Some(value()?.parse()?),
                "--timing-frames" => a.timing_frames = value()?.parse()?,
                "--diag" => {
                    let v = value()?;
                    let (key, setting) = v.split_once('=').ok_or("--diag KEY=VALUE")?;
                    a.diag.push((key.into(), setting.into()));
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
    // Scene switches (`flicker=0`: a steady fire light; `lamp=post`: v0.1.0's lamp placement,
    // with each soft source inside its own fixture; `sun=0`: no sun) stay here; the rest are
    // estimator switches.
    let (scene_keys, estimator_keys): (Vec<_>, Vec<_>) =
        args.diag.iter().cloned().partition(|(k, _)| {
            matches!(
                k.as_str(),
                "flicker"
                    | "lamp"
                    | "sun"
                    | "fire"
                    | "glow"
                    | "cut"
                    | "hold"
                    | "bulb"
                    | "field"
                    | "forest"
                    | "fox"
                    | "floor"
                    | "program"
                    | "far"
                    | "casts"
            )
        });
    // U-SPECK (docs/speck.md): `bulb=off` leaves out the two lamp bulbs' spheres, `bulb=dark`
    // keeps them glowing but reflecting nothing, to tell which lit surface a blob comes from.
    let (bulb_off, bulb_dark) = (
        scene_keys.iter().any(|(k, v)| k == "bulb" && v == "off"),
        scene_keys.iter().any(|(k, v)| k == "bulb" && v == "dark"),
    );
    let diag = Diag::parse(&estimator_keys)?;
    let scene_key = |key: &str, value: &str| scene_keys.iter().any(|(k, v)| k == key && v == value);
    let steady_fire = scene_key("flicker", "0");
    let (fire_off, fire_on, fire_moves) = (
        scene_key("fire", "off"),
        scene_key("fire", "on"),
        scene_key("fire", "move"),
    );
    let (glow_steps, cuts) = (scene_key("glow", "step"), scene_key("cut", "1"));
    let pose_only = scene_key("hold", "pose");
    let two_foxes = scene_key("fox", "2");
    let hold_cut = scene_key("hold", "cut");
    let hold = scene_key("hold", "1") || pose_only || hold_cut;
    // `casts=toggle`: the crates' shadow casting off at frame 12, on again at frame 24.
    let casts_toggle = scene_key("casts", "toggle");
    // U-SPECK (docs/speck.md): `field=1` scatters 300 small bright boxes and a pale slab over
    // an empty quarter of the ground and walks the camera 4 m across it while turning 12
    // degrees, whatever `--frames` is: the cell mosaic Ara's Outpost showed on flat ground.
    let field = scene_key("field", "1");
    // `cut=every` with the field walk: every frame is a camera cut to another point of the
    // walk (a fixed shuffle of its frames), as Ara's every-frame-cut clip at Outpost's hangar.
    let cut_every = scene_key("cut", "every");
    // `cut=turn` (U-RTVIS, the GI cut refill): the field or forest walk turns 150 degrees in
    // place at the event frame, with `cut_view`, onto ground and trees it has not seen.
    let cut_turn = scene_key("cut", "turn");
    let outpost = scene_key("cut", "outpost");
    // `forest=bare`: the same terrain and walk without the trees and bushes.
    let bare = scene_key("forest", "bare");
    let forest = scene_key("forest", "1") || bare || outpost;
    let floor = scene_keys
        .iter()
        .find(|(k, _)| k == "floor")
        .map_or(0.95, |(_, v)| {
            v.parse::<f32>().expect("--diag floor=<roughness>")
        });
    // `far=1` (U-FIXED, the sun's coverage, Ara #185 item 33): a 2.4 km ground and columns
    // at 80, 200, 300 and 800 m north of a camera 40 m up with a 2 km far plane, whose shadows
    // cross the view: sun shadows at game-scale distances, on both paths. `far=move` slides the
    // camera east 0.25 m a frame. `far=low`/`lowmove`: the camera 1.7 m up, 8 m south of the
    // arch, with a body-sized box 0.3 m east of the eye (out of view) moving with it, and
    // columns at 50, 200 and 1000 m. `far=box`/`boxmove`: the same eye 1.4 m from a 6 m wall
    // turned 45 degrees behind its left, whose bounds hold the eye (out of view too).
    let far_low = scene_key("far", "low") || scene_key("far", "lowmove");
    let far_box = scene_key("far", "box") || scene_key("far", "boxmove");
    let far_move = ["move", "lowmove", "boxmove"]
        .iter()
        .any(|v| scene_key("far", v));
    let far = scene_key("far", "1") || far_move || far_low || far_box;
    let mut held_pose = None;
    let no_sun = scene_keys.iter().any(|(k, v)| k == "sun" && v == "0");
    let legacy_lamps = scene_keys.iter().any(|(k, v)| k == "lamp" && v == "post");
    // A soft source must clear its own fixture, or every receiver sees it partly blocked: a
    // permanent penumbra that shadow rays sample as grain (P-FLAT). The post lamp's head hangs
    // beside its post under a wider cap, and the arch light sits one source radius plus one
    // bulb radius below its bulb (v0.1.0 had the spot inside the post, 75-80 % blocked).
    let (lamp_offset, arch_drop) = if legacy_lamps {
        (Vec3::ZERO, 0.2)
    } else {
        (Vec3::new(0.22, 0.0, -0.22), 0.25)
    };

    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            pipeline_cache: args.pipeline_cache.clone(),
            settings: diag.settings(Settings {
                global_illumination: args.gi,
                temporal_reconstruction: args.temporal,
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
                sky_light: args.atmosphere_light,
                ..Default::default()
            }),
            // `forest=1` puts 9000 leaf cards on one mesh-material pair.
            limits: {
                let limits = EngineDescriptor::default().limits;
                Limits {
                    max_instances: if forest { 16384 } else { limits.max_instances },
                    ..limits
                }
            },
            ..Default::default()
        },
    )?;
    let look = diag.look(ViewLook {
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
    });
    let view_desc = ViewDesc {
        render_scale: args.scale,
        look,
        ..ViewDesc::new(args.size[0], args.size[1])
    };
    // `--views ab`: the second view is created first, so the first view's renderer tuning
    // (diag) is what both views render with. `ai`: it is created and never rendered (an
    // idle view's cost).
    let idle = (["ab", "ai"].contains(&args.views.as_str()))
        .then(|| engine.create_view(&view_desc))
        .transpose()?;
    let second = idle.filter(|_| args.views == "ab");
    let view = engine.create_view(&view_desc)?;
    diag.apply(&mut engine, path, args.scale, view_desc.look.quality)?;
    let engine_seconds = start.elapsed().as_secs_f64();
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
    // P-SHC diagnostic: a live, previously unseen initial shader, not a cache flush.
    let game_program = std::env::var("RENDERLAB_SHC_PROGRAM_TINT")
        .ok()
        .map(|tint| {
            let tint: f32 = tint.parse().expect("P-SHC tint must be f32");
            assert!((0.5..=1.0).contains(&tint));
            tinted_program(tint)
        });
    let game_program = game_program.as_deref().unwrap_or(GAME_PROGRAM);
    let program = engine.create_program("game", game_program)?;
    let standard = scene_keys.iter().any(|(k, v)| k == "program" && v == "standard");
    let toon_program = engine.create_program("toon", TOON_PROGRAM)?;
    let programs_seconds = start.elapsed().as_secs_f64();
    let material = |engine: &mut Engine, color: [f32; 3], metallic: f32, roughness: f32| {
        engine.create_material(&MaterialDesc {
            base_color: [color[0], color[1], color[2], 1.0],
            metallic,
            roughness,
            program: (!standard).then_some(program),
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
        base_color: if bulb_dark {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [1.0, 0.9, 0.7, 1.0]
        },
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
            let ground = material(&mut engine, [0.34, 0.37, 0.28], 0.0, floor)?;
            if forest {
                for tile in terrain_tiles(FOREST_SIZE, 400, 4, outpost) {
                    let tile = engine.create_mesh(&mesh(&tile))?;
                    engine.spawn(tile, ground, Mat4::IDENTITY)?;
                }
            } else {
                let size = if far { 2400.0 } else { 60.0 };
                let plane = engine.create_mesh(&mesh(&plane_geometry(size, 8)))?;
                engine.spawn(plane, ground, Mat4::IDENTITY)?;
            }
            // The physical atmosphere is the default sky (AT1); `--sky uniform` swaps in a
            // constant blue sky for comparison.
            if args.uniform_sky {
                engine.set_sky(Sky::Uniform([3500.0, 5000.0, 7500.0]))?;
            } else if let Some(flat) = args.sky_map {
                let map =
                    engine.create_environment(&gradient_sky::prepare(args.sky_face, flat)?)?;
                engine.set_sky(Sky::Map {
                    map,
                    rotation_y: 0.0,
                    intensity: args.sky_light.unwrap_or(1.0),
                    background: match args.sky_light {
                        Some(_) => Background::ScaledSky(1.0),
                        None => Background::Sky,
                    },
                })?;
            }
            Vec3::ZERO
        }
    };
    let at = |x: f32, y: f32, z: f32| anchor + Vec3::new(x, y, z);
    let boxed = |center: Vec3, size: Vec3| Mat4::from_translation(center) * Mat4::from_scale(size);
    // The field walk: from FIELD_EYE, 4 m along FIELD_WALK.
    const FIELD_EYE: Vec3 = Vec3::new(-6.0, 1.6, -6.0);
    const FIELD_WALK: Vec3 = Vec3::new(-0.707_106_77, 0.0, -0.707_106_77);
    if field {
        let magenta = material(&mut engine, [0.75, 0.08, 0.75], 0.0, 0.6)?;
        let slab = material(&mut engine, [0.35, 0.65, 0.85], 0.0, 0.7)?;
        engine.spawn(
            cube,
            slab,
            boxed(at(-10.5, 0.03, -8.0), Vec3::new(4.0, 0.06, 4.0)),
        )?;
        // A fixed scatter (xorshift), clear of the camera's walk.
        let mut seed = 0x9e37_79b9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        for _ in 0..300 {
            let (x, z, size) = (-29.0 + 24.0 * next(), -29.0 + 24.0 * next(), 0.3 + 0.9 * next());
            let along = (Vec3::new(x, 0.0, z) - FIELD_EYE).dot(FIELD_WALK).clamp(0.0, 4.0);
            let aside = Vec3::new(x, 0.0, z) - (FIELD_EYE + along * FIELD_WALK);
            if aside.length() - FIELD_EYE.y > 2.5 || aside.length() > 6.0 {
                engine.spawn(
                    cube,
                    magenta,
                    boxed(at(x, 0.5 * size, z), Vec3::splat(size)),
                )?;
            }
        }
    }
    if forest && !bare && quarry.is_none() {
        let bark = material(&mut engine, [0.3, 0.22, 0.15], 0.0, 0.85)?;
        let mut seed = 0x6a09_e667_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        let mut spot = |clearing: f32| loop {
            let (x, z) = (FOREST_SIZE * (next() - 0.5), FOREST_SIZE * (next() - 0.5));
            let clear = forest_clear(x, z).min(if outpost { outpost_cut::clear(x, z) } else { 1.0 });
            if clear >= clearing {
                let height = forest_height(x, z) * if outpost { outpost_cut::clear(x, z) } else { 1.0 };
                return (Vec3::new(x, height, z), next());
            }
        };
        // Trees: a trunk and four crossed canopy cards from 3 m up.
        for _ in 0..900 {
            let (p, r) = spot(1.0);
            let scale = 1.8 + 0.8 * r;
            engine.spawn(
                cube,
                bark,
                boxed(p + Vec3::new(0.0, 2.5, 0.0), Vec3::new(0.35, 5.0, 0.35)),
            )?;
            for k in 0..4 {
                engine.spawn(
                    bush,
                    leaves,
                    Mat4::from_translation(p + Vec3::new(0.0, 3.0, 0.0))
                        * Mat4::from_rotation_y(r * PI + k as f32 * PI / 4.0)
                        * Mat4::from_scale(Vec3::splat(scale)),
                )?;
            }
        }
        // Bushes: three crossed cards each, closer in.
        for _ in 0..1800 {
            let (p, r) = spot(0.5);
            for k in 0..3 {
                engine.spawn(
                    bush,
                    leaves,
                    Mat4::from_translation(p) * Mat4::from_rotation_y(r * PI + k as f32 * PI / 3.0),
                )?;
            }
        }
    }
    // `far=low`/`box`: the caster beside the eye at a slide of `x` metres east.
    let far_caster = |x: f32| {
        if far_box {
            Mat4::from_translation(at(x - 1.0, 1.5, 9.0))
                * Mat4::from_rotation_y(-PI / 4.0)
                * Mat4::from_scale(Vec3::new(6.0, 3.0, 0.1))
        } else {
            boxed(at(x + 0.5, 0.9, 8.0), Vec3::new(0.4, 1.8, 0.3))
        }
    };
    let mut far_body = None;
    if far {
        let column = material(&mut engine, [0.7, 0.7, 0.72], 0.0, 0.8)?;
        let distances: &[f32] = if far_low || far_box {
            &[50.0, 200.0, 1000.0]
        } else {
            &[80.0, 200.0, 300.0, 800.0]
        };
        for &d in distances {
            let (width, height) = (d / 25.0, d / 8.0);
            engine.spawn(
                cube,
                column,
                boxed(at(-0.15 * d, 0.5 * height, -d), Vec3::new(width, height, width)),
            )?;
        }
        if far_low || far_box {
            far_body = Some(engine.spawn(cube, column, far_caster(0.0))?);
        }
    }
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
    let mut crates = vec![];
    for (x, y, z, turn) in [
        (4.0, 0.5, 1.0, 0.1),
        (5.1, 0.5, 1.3, -0.25),
        (4.5, 1.5, 1.1, 0.4),
        (4.2, 0.5, 2.3, 0.0),
    ] {
        crates.push(engine.spawn(
            cube,
            wood,
            Mat4::from_translation(at(x, y, z)) * Mat4::from_rotation_y(turn),
        )?);
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
        let crowd = if args.crowd_films {
            engine.create_material(&MaterialDesc {
                base_color: [0.35, 0.45, 0.3, 1.0],
                roughness: 0.6,
                transparency: Some(Transparency {
                    opacity: 0.3,
                    ior: 1.0,
                    thickness: 0.0,
                }),
                ..Default::default()
            })?
        } else {
            glass
        };
        // Panes stepping back and sideways from the first, so each overlaps the ones before.
        for k in 1..=args.glass_crowd {
            let t = k as f32 * 0.07;
            engine.spawn(
                cube,
                crowd,
                Mat4::from_translation(at(5.4 - t, 0.8 + 0.3 * (t * 3.0).sin(), 3.6 - t))
                    * Mat4::from_rotation_y(0.8)
                    * Mat4::from_scale(Vec3::new(2.2, 1.6, 0.04)),
            )?;
        }
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
    if !bulb_off {
        engine.spawn(sphere, bulb, boxed(arch_lamp, Vec3::splat(0.12)))?;
    }
    engine.spawn(
        cube,
        iron,
        boxed(
            arch_lamp + Vec3::new(0.0, 0.3, 0.0),
            Vec3::new(0.03, 0.5, 0.03),
        ),
    )?;
    let post_top = at(3.0, 3.2, 3.0);
    let post_lamp = post_top + lamp_offset;
    engine.spawn(
        cube,
        iron,
        boxed(at(3.0, 1.6, 3.0), Vec3::new(0.12, 3.2, 0.12)),
    )?;
    engine.spawn(
        cube,
        iron,
        boxed(
            post_top + 0.5 * lamp_offset + Vec3::new(0.0, 0.12, 0.0),
            Vec3::new(0.4 + lamp_offset.x.abs(), 0.08, 0.4 + lamp_offset.z.abs()),
        ),
    )?;
    if !bulb_off {
        engine.spawn(sphere, bulb, boxed(post_lamp, Vec3::splat(0.1)))?;
    }
    // A mirror-polished chrome sphere: traced mirror reflections on RT (M7-R1).
    engine.spawn(sphere, chrome, boxed(at(-1.2, 0.8, 5.6), Vec3::splat(0.8)))?;
    let spinner = engine.spawn(cube, paint, Mat4::from_translation(at(-2.5, 0.6, 3.0)))?;
    let mut second_hero = match &args.fox {
        Some(source) if two_foxes => Some(Hero::fox(
            &mut engine,
            source,
            &args.out,
            toon_program,
            &hero_style,
        )?),
        _ => None,
    };
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
        Sun::at_hour(hour, LATITUDE)
            .filter(|_| !no_sun)
            .map(|sun| Sun {
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
    // Fire and smoke over the brazier (TP1 GPU particles) step at the frame rate: every
    // render below is its own engine tick (`Engine::tick`), as one view's game loop is.
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
    let outpost_lights = outpost.then(|| outpost_cut::lights(anchor, |x, z| {
        forest_height(x, z) * outpost_cut::clear(x, z)
    }));
    let lanterns = |t: f32, frame: u32| {
        if let Some(lights) = &outpost_lights { return lights.clone(); }
        let ft = if steady_fire { 0.0 } else { t };
        // P-RELIT: the fire switched off or on at the event frame.
        let lit = if frame < EVENT_FRAME {
            !fire_on
        } else {
            !fire_off
        };
        let flicker = if lit {
            0.85 + 0.1 * (23.0 * ft).sin() + 0.05 * (37.0 * ft + 1.0).sin()
        } else {
            0.0
        };
        // P-RELIT: the fire light circling its brazier, 0.6 m out.
        let fire_offset = if fire_moves {
            0.6 * Vec3::new((3.0 * t).cos(), 0.0, (3.0 * t).sin())
        } else {
            Vec3::ZERO
        };
        let mut lights = vec![
            Light::Point {
                position: arch_lamp - Vec3::new(0.0, arch_drop, 0.0),
                candela: [
                    30_000.0 * lamp_scale,
                    21_000.0 * lamp_scale,
                    11_000.0 * lamp_scale,
                ],
                range: Some(14.0),
                source_radius: 0.12,
                cast_shadows: None,
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
                cast_shadows: None,
            },
            Light::Point {
                position: flame + Vec3::new(0.0, 0.3, 0.0) + fire_offset,
                candela: [
                    24_000.0 * flicker * lamp_scale,
                    12_000.0 * flicker * lamp_scale,
                    4_000.0 * flicker * lamp_scale,
                ],
                range: Some(10.0),
                source_radius: 0.25,
                cast_shadows: None,
            },
        ];
        if args.night_lanterns {
            lights.extend((0..24).map(|i| Light::Point {
                position: at((i % 6) as f32 * 1.8 - 4.5, 1.8, (i / 6) as f32 * 2.0 - 1.5),
                candela: [22.0, 12.0, 4.0],
                range: Some(4.0),
                source_radius: 0.06,
                cast_shadows: None,
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
    let pipeline_preparation = if std::env::var("RENDERLAB_PREWARM_PIPELINES").is_ok_and(|v| v == "1") {
        let p = engine.prepare_pipelines()?;
        Some(serde_json::json!({"ms": p.cpu.as_secs_f64() * 1e3,
            "render": p.render_pipelines, "compute": p.compute_pipelines,
            "added_buffer_bytes": p.added_buffer_bytes, "added_texture_bytes": p.added_texture_bytes}))
    } else { None };
    let ready = start.elapsed();
    let mut frames = Vec::new();
    let mut last_camera = None;
    let mut last_camera_b = None;
    let mut first_image_seconds = None;
    let mut reload = None;
    // Measurement-only controls: hold the unchanged scene before a late cut, and read back
    // only post-cut/reference frames without changing the frames the engine renders.
    let event_frame = std::env::var("RENDERLAB_SCENE_EVENT_FRAME")
        .map_or(Ok(if outpost { outpost_cut::CUT_FRAME } else { EVENT_FRAME }), |v| v.parse::<u32>())?;
    let keep_frames = std::env::var("RENDERLAB_SCENE_KEEP_FRAMES").ok()
        .map(|list| list.split(',').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>())
        .transpose()?;
    let drain = std::env::var("RENDERLAB_SCENE_DRAIN").is_ok_and(|v| v == "1");
    // U-FIXED measurement only: RENDERLAB_SCENE_EVENT_FRAMES=a,b,c cuts at each listed frame
    // (`cut=turn` alternates between the turned view and the walk's own), and
    // RENDERLAB_SCENE_COUNTERS=<file> writes one JSON line per frame of cumulative pipeline,
    // resource and acceleration-structure counters.
    let events: Vec<u32> = match std::env::var("RENDERLAB_SCENE_EVENT_FRAMES") {
        Ok(list) => list.split(',').map(str::parse::<u32>).collect::<Result<_, _>>()?,
        Err(_) => vec![event_frame],
    };
    if outpost && events != [event_frame] {
        return Err("cut=outpost needs one event; use RENDERLAB_SCENE_EVENT_FRAME to move it".into());
    }
    // Clear only GI, far before the measured cut, without dropping view pipelines.
    let gi_resets: Vec<u32> = std::env::var("RENDERLAB_SCENE_GI_RESET_FRAMES").ok()
        .map(|list| list.split(',').map(str::parse::<u32>).collect::<Result<_, _>>())
        .transpose()?.unwrap_or_default();
    let mut queue_diagnostic=std::env::var_os("RENDERLAB_GI_QUEUE_DIAG").map(std::fs::File::create).transpose()?;
    let queue_frames:Vec<u32>=std::env::var("RENDERLAB_SCENE_QUEUE_FRAMES").unwrap_or_else(|_|"443,462,512,641".into())
        .split(',').map(str::parse::<u32>).collect::<Result<_,_>>()?;
    let mut counters = std::env::var_os("RENDERLAB_SCENE_COUNTERS")
        .map(std::fs::File::create)
        .transpose()?;
    for frame in 0..args.frames {
        if gi_resets.contains(&frame) { engine.renderer_mut().reset_indirect(); }
        let t = if args.still || outpost { 0.0 } else { frame as f32 / FPS };
        let sun_update = Instant::now();
        let hour = sun_hour(frame);
        if args.sun_speed != 0.0 && frame >= args.sun_after {
            engine.set_sun(scene_sun(hour))?;
        }
        let sun_update_ms = sun_update.elapsed().as_secs_f64() * 1e3;
        // The hero walks a circle through the arch.
        let walk = if pose_only { 0.0 } else { 0.35 * t };
        let ground = at(3.0 * walk.sin(), 0.0, -1.0 + 3.0 * walk.cos());
        let heading = walk.cos().atan2(-walk.sin());
        let focus = hero.update(&mut engine, ground, heading, t)?;
        if let Some(second) = &mut second_hero {
            let behind = walk + std::f32::consts::PI;
            let ground = at(3.0 * behind.sin(), 0.0, -1.0 + 3.0 * behind.cos());
            second.update(&mut engine, ground, behind.cos().atan2(-behind.sin()), t + 0.4)?;
        }
        engine.set_lights(&diag.lights(lanterns(t, frame)))?;
        if glow_steps && frame == event_frame {
            let dim = [6_000.0, 4_000.0, 1_800.0].map(|v| v * lamp_scale);
            engine.set_material_parameters(bulb, &[("glow", Value::Vec3(dim))])?;
        }
        if casts_toggle && (frame == 12 || frame == 24) {
            for &object in &crates {
                engine.set_casts_shadows(object, frame == 24)?;
            }
        }
        let spin = if pose_only { 0.0 } else { t };
        engine.set_transform(
            spinner,
            Mat4::from_translation(at(-2.5, 0.6 + 0.3 * (2.0 * spin).sin(), 3.0))
                * Mat4::from_rotation_y(1.5 * spin)
                * Mat4::from_rotation_x(0.6 * spin),
        )?;
        // Fly path: a slow orbit that descends and closes in on the hero.
        let cut = (cuts || hold_cut) && frame >= event_frame;
        if (cuts || cut_turn || hold_cut || outpost) && events.contains(&frame) {
            engine.cut_view(view);
            // `hold=cut`: the held pose moves to the cut's pose and holds there.
            if hold_cut {
                held_pose = None;
            }
        }
        let orbit = 0.6 + 0.25 * t + if cut { 2.1 } else { 0.0 };
        let radius = 14.0 - 4.0 * (t / 2.0).min(1.0);
        let eye = anchor
            + Vec3::new(
                radius * orbit.sin(),
                5.5 - 2.0 * (t / 2.0).min(1.0),
                radius * orbit.cos(),
            );
        let target = focus.lerp(anchor + Vec3::Y, 0.5);
        let (eye, target) = if outpost {
            let pose = outpost_cut::pose(frame, event_frame);
            (anchor + pose.eye, anchor + pose.eye + pose.forward)
        } else if hold {
            *held_pose.get_or_insert((eye, target))
        } else if field || forest {
            // `--still` stays at the walk's start; `cut=every` jumps about the walk.
            let steps = args.frames.max(2) - 1;
            let along = if args.still {
                0.0
            } else if cut_every {
                (frame * 37 % (steps + 1)) as f32 / steps as f32
            } else {
                frame as f32 / steps as f32
            };
            if cut_every && frame > 0 {
                engine.cut_view(view);
            }
            let eye = anchor + FIELD_EYE + 4.0 * along * FIELD_WALK;
            let passed = events.iter().filter(|&&e| frame >= e).count();
            let turn = if cut_turn && passed % 2 == 1 { 150.0 } else { 0.0 };
            let look = Mat4::from_rotation_y((12.0 * along + turn).to_radians())
                .transform_vector3(FIELD_WALK);
            (eye, eye + 10.0 * look - Vec3::new(0.0, 2.2, 0.0))
        } else {
            (eye, target)
        };
        let (eye, target, far_plane) = if far {
            let slide = Vec3::new(if far_move { 0.25 * frame as f32 } else { 0.0 }, 0.0, 0.0);
            if let Some(body) = far_body {
                engine.set_transform(body, far_caster(slide.x))?;
            }
            let eye = if far_body.is_some() {
                Vec3::new(0.0, 1.7, 8.0)
            } else {
                Vec3::new(0.0, 40.0, 0.0)
            };
            (
                anchor + slide + eye,
                anchor + slide + Vec3::new(0.0, 0.0, -400.0),
                2000.0,
            )
        } else {
            (eye, target, if outpost { 4000.0 } else { 500.0 })
        };
        let camera = Camera::look_at(eye, target, Vec3::Y, 55f32.to_radians(), 0.05, far_plane)?;
        // View b: across the scene, still at its first pose unless `--b-orbit`, a third of
        // the way round from `--cut-b` on.
        let b_cut = args.cut_b.is_some_and(|f| frame >= f);
        let b_orbit =
            0.6 + PI + if args.b_orbit { 0.25 * t } else { 0.0 } + if b_cut { 2.1 } else { 0.0 };
        if args.cut_b == Some(frame) {
            match (args.views.as_str(), second) {
                ("ab", Some(second)) => engine.cut_view(second),
                ("b", _) => engine.cut_view(view),
                _ => {}
            }
        }
        let camera_b = Camera::look_at(
            anchor + Vec3::new(12.0 * b_orbit.sin(), 4.5, 12.0 * b_orbit.cos()),
            anchor + Vec3::Y,
            Vec3::Y,
            50f32.to_radians(),
            0.05,
            500.0,
        )?;
        let camera = if args.views == "b" { camera_b } else { camera };
        let begin = Instant::now();
        engine.tick(1.0 / FPS)?;
        let queue_sample=queue_diagnostic.is_some() && queue_frames.contains(&frame);
        if queue_sample {engine.renderer_mut().begin_indirect_queue_probe()?;}
        let report = engine.render_view(view, &camera)?;
        if queue_sample {
            use std::io::Write;
            let q=engine.renderer_mut().indirect_queue_probe()?;
            writeln!(queue_diagnostic.as_mut().unwrap(),"{}",serde_json::json!({"frame":frame,"phase":"incoming_after_probe","visible_never_gathered":q[0],"visible_with_raw_data":q[1],"off_screen":q[2],"ineligible":q[3],"duplicates":q[4],"entries":q[5],"visible_filtered_only":q[6]}))?;
        }
        // `--views ab`: the second view renders in the same tick, after the first.
        let second_hash = match second {
            Some(second) => {
                engine.render_view(second, &camera_b)?;
                let written = keep_frames.as_ref().map_or(frame % args.every == 0 || frame + 1 == args.frames, |keep| keep.contains(&frame));
                written
                    .then(|| -> Result<_, Box<dyn std::error::Error>> {
                        let image = engine.capture_view(second)?;
                        write_png(&args.out.join(format!("frame-{frame:03}-b.png")), &image)?;
                        Ok(format!("{:x}", Sha256::digest(&image.rgba8)))
                    })
                    .transpose()?
            }
            None => None,
        };
        if drain {
            engine.device().poll(wgpu::PollType::wait_indefinitely())?;
            engine.poll()?;
        }
        let cpu_ms = begin.elapsed().as_secs_f64() * 1e3;
        if let Some(file) = counters.as_mut() {
            use std::io::Write;
            let r = engine.renderer();
            let st = r.stats();
            let kinds: Vec<u64> = st.ownership.persistent.iter().map(|c| c.created).collect();
            let hw = r.hardware_shadow_stats();
            writeln!(file, "{}", serde_json::json!({
                "frame": frame, "pipelines_created": st.pipelines.created, "created_by_kind": kinds,
                "shadow_creations": st.ownership.shadow_creations, "target_creations": st.ownership.target_creations,
                "lut_creations": st.ownership.lut_creations, "material_creations": st.ownership.material_creations,
                "texture_creations": st.ownership.texture_creations, "light_creations": st.ownership.light_creations,
                "blas_builds": hw.map(|h| h.blas_builds), "blas_updates": hw.map(|h| h.blas_updates),
                "tlas_builds": hw.map(|h| h.tlas_builds), "pending_builds": hw.map(|h| h.pending_builds),
                "as_bytes": hw.map(|h| h.estimated_as_bytes),
                "visible_instances": report.scene.visible_instances,
                "submitted_instances": report.scene.submitted_instances,
            }))?;
        }
        let written = keep_frames.as_ref().map_or(frame % args.every == 0 || frame + 1 == args.frames, |keep| keep.contains(&frame));
        let mut image_hash = None;
        let mut hdr_hash = None;
        if written {
            let image = engine.capture_view(view)?;
            first_image_seconds.get_or_insert_with(|| start.elapsed().as_secs_f64());
            image_hash = Some(format!("{:x}", Sha256::digest(&image.rgba8)));
            if std::env::var_os("RENDERLAB_SHC_HDR_HASHES").is_some() {
                let mut capture = engine
                    .renderer_mut()
                    .try_capture_hdr()?
                    .ok_or("HDR capture busy")?;
                engine.device().poll(wgpu::PollType::wait_indefinitely())?;
                let bytes = capture.try_read()?.ok_or("HDR readback pending")?;
                hdr_hash = Some(format!("{:x}", Sha256::digest(&bytes)));
            }
            write_png(&args.out.join(format!("frame-{frame:03}.png")), &image)?;
        }
        // The non-RT sun clipmap's ladder and its reach (receiver counts lag a frame or two).
        let v = engine.renderer().virtual_shadow_stats();
        let clipmap = serde_json::json!({
            "levels": v.levels, "finest_level": v.finest_level, "skipped_levels": v.skipped_levels,
            "nearest_receiver": v.nearest_receiver, "receivers": v.receivers,
            "unplaced_receivers": v.unplaced_receivers, "nearest_unplaced": v.nearest_unplaced,
        });
        frames.push(serde_json::json!({
            "frame": frame, "renderer_frame": report.frame.0, "cpu_render_ms": cpu_ms,
            "sun_hour": hour, "cpu_sun_update_ms": sun_update_ms,
            "primary": format!("{:?}", report.primary), "soft_shadows": format!("{:?}", report.soft_shadows),
            "draws": report.scene.draws, "triangles": report.scene.triangles, "written": written,
            "rgba8_sha256": image_hash, "t": t, "eye": eye.to_array(), "target": target.to_array(),
            "hdr_sha256": hdr_hash, "view": &args.views[..1], "b_rgba8_sha256": second_hash,
            "clipmap": clipmap,
        }));
        last_camera = Some(camera);
        last_camera_b = Some(camera_b);
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
            let edit = tinted_program(tint);
            assert_ne!(edit, game_program, "probe must change the shader");
            let edited = Instant::now();
            engine.reload_program(program, &edit)?;
            let compiled_seconds = edited.elapsed().as_secs_f64();
            engine.tick(1.0 / FPS)?;
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
            engine.tick(1.0 / FPS)?;
            engine.render_view(view, &camera)?;
            let retained = engine.capture_view(view)?;
            write_png(&args.out.join("rejected-retained.png"), &retained)?;
            engine.reload_program(program, game_program)?;
            engine.tick(1.0 / FPS)?;
            engine.render_view(view, &camera)?;
            let restored = engine.capture_view(view)?;
            write_png(&args.out.join("restored.png"), &restored)?;
            let repeated = Instant::now();
            engine.reload_program(program, &edit)?;
            engine.tick(1.0 / FPS)?;
            engine.render_view(view, &camera)?;
            engine.capture_view(view)?;
            let repeat_seconds = repeated.elapsed().as_secs_f64();
            engine.reload_program(program, game_program)?;
            engine.tick(1.0 / FPS)?;
            engine.render_view(view, &camera)?;
            engine.capture_view(view)?;
            reload = Some(serde_json::json!({"compile_seconds": compiled_seconds,
                "edit_to_readback_seconds": image_seconds, "repeat_to_readback_seconds": repeat_seconds,
                "tint":tint,"source_changed":edit != game_program,
                "edited_rgba8_sha256":format!("{:x}",Sha256::digest(&image.rgba8)),
                "invalid_edit_rejected": rejected}));
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
                engine.set_sun(scene_sun(sun_hour(args.frames + step)))?;
            }
            engine.tick(1.0 / FPS)?;
            engine.render_view(view, camera)?;
            if let (Some(second), Some(camera_b)) = (second, &last_camera_b) {
                engine.render_view(second, camera_b)?;
            }
            cpu.push(submit.elapsed().as_secs_f64() * 1e3);
        }
        engine.capture_view(view)?;
        serde_json::json!({"frames":args.timing_frames,
            "wall_ms_per_frame":begin.elapsed().as_secs_f64()*1e3/f64::from(args.timing_frames),
            "cpu_submit_ms":cpu,
            "views": args.views,
            "note":"drained throughput per tick (every view of --views), final colour copy/readback included; no PNG encoding"})
    } else {
        serde_json::Value::Null
    };
    // Device memory after the run, for a view's cost (`--views ab` against `a`).
    let stats = engine.renderer().stats();
    let memory = serde_json::json!({
        "allocator_allocated_bytes": stats.allocator.as_ref().map(|a| a.allocated_bytes),
        "allocator_reserved_bytes": stats.allocator.as_ref().map(|a| a.reserved_bytes),
        "hal_buffer_bytes": stats.hal.buffer_bytes, "hal_texture_bytes": stats.hal.texture_bytes,
        "target_bytes": stats.ownership.target_bytes.live, "shadow_bytes": stats.ownership.shadow_bytes.live,
    });
    let summary = serde_json::json!({
        "memory": memory,
        "path": format!("{path:?}"), "adapter": engine.adapter_info().name,
        "debug_build": cfg!(debug_assertions), "size": args.size, "exposure_stops": exposure,
        "settings": format!("{:?}", engine.settings()), "objects": engine.object_count(),
        "fox": args.fox, "hour": args.hour,
        "night_lanterns": args.night_lanterns, "local_lights": diag.lights(lanterns(0.0, 0)).len(),
        "outpost_cut_fixture": outpost.then(|| serde_json::json!({
            "cut_frame":event_frame,"hold_frames":event_frame.saturating_sub(outpost_cut::LEAD_FRAMES),
            "lead_frames":outpost_cut::LEAD_FRAMES,"jump":[-20,0,15],"yaw_degrees":140,
            "clip_move":[4,0,-4],"clip_yaw_degrees":12,"clip_steps":119,
            "vertical_fov_degrees":55,"near":0.05,"far":4000,
            "default_gi_cell_pixels":renderlab::IndirectSettings::default().cell_pixels,
            "lights":25,"scene_frozen":true,"forest_size":FOREST_SIZE,
        })),
        "diag": args.diag, "lamp_head": post_lamp.to_array(),
        "local_shadow_count":engine.settings().local_shadows.count,
        "quarry": args.quarry, "pipeline_preparation": pipeline_preparation,
        "startup_to_scene_ready_seconds": ready.as_secs_f64(),
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
/// `forest=1`: the terrain's side in metres.
const FOREST_SIZE: f32 = 200.0;
/// 0 within 14 m of the courtyard or 3 m of the field walk, rising to 1 over the next 10 m:
/// where `forest=1` keeps the ground flat and the foliage away.
fn forest_clear(x: f32, z: f32) -> f32 {
    // The field walk's start and direction on the ground (FIELD_EYE, FIELD_WALK).
    let (start, walk) = (
        Vec3::new(-6.0, 0.0, -6.0),
        Vec3::new(-0.707_106_77, 0.0, -0.707_106_77),
    );
    let p = Vec3::new(x, 0.0, z);
    let along = (p - start).dot(walk).clamp(0.0, 4.0);
    let smooth = |a: f32, b: f32, d: f32| {
        let t = ((d - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    smooth(14.0, 24.0, p.length()).min(smooth(3.0, 13.0, (p - start - along * walk).length()))
}
/// Rolling hills 0.8-7 m high outside the clearings.
fn forest_height(x: f32, z: f32) -> f32 {
    let hills = 2.5 * (0.11 * x).sin() * (0.13 * z).cos()
        + 1.2 * (0.31 * x + 0.17 * z).sin()
        + 0.4 * (0.9 * x - 0.7 * z).sin();
    (hills + 4.0) * forest_clear(x, z)
}
/// `forest_height` over a `size` metre square of `cells` by `cells` quads.
fn terrain_geometry(size: f32, cells: u32, outpost: bool) -> Geometry {
    let mut g = plane_geometry(size, cells);
    let step = size / cells as f32;
    for v in &mut g.vertices {
        let [x, _, z] = v.position;
        let h = |dx: f32, dz: f32| {
            let height = forest_height(x + dx, z + dz);
            if outpost { height * outpost_cut::clear(x + dx, z + dz) } else { height }
        };
        v.position[1] = h(0.0, 0.0);
        let normal = Vec3::new(
            h(-step, 0.0) - h(step, 0.0),
            2.0 * step,
            h(0.0, -step) - h(0.0, step),
        );
        v.normal = normal.normalize().to_array();
    }
    g
}
/// `terrain_geometry` cut into `tiles` by `tiles` meshes of the same vertices, as an engine
/// streams terrain in chunks (and so the non-RT Virtual sun map's 32-bit placement and
/// primitive packing fits the forest).
fn terrain_tiles(size: f32, cells: u32, tiles: u32, outpost: bool) -> Vec<Geometry> {
    let full = terrain_geometry(size, cells, outpost);
    let (row, n) = (cells + 1, cells / tiles);
    let mut out = Vec::new();
    for tz in 0..tiles {
        for tx in 0..tiles {
            let mut g = plane_geometry(1.0, n);
            for z in 0..=n {
                for x in 0..=n {
                    let at = (tz * n + z) * row + tx * n + x;
                    g.vertices[(z * (n + 1) + x) as usize] = full.vertices[at as usize];
                }
            }
            out.push(g);
        }
    }
    out
}
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
