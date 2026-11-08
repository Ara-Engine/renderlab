//! A small game-like scene through the engine API (docs/api.md), offscreen.
//!
//! Procedural ground, an arch, crates, a red wall (colour bleeding), a glossy gold sphere,
//! a mirror chrome sphere, a spinning crate and a bobbing toon-styled hero, with an emblem
//! decal and a wet puddle decal on the ground. A sun with soft shadows, the physical
//! atmosphere with aerial perspective and height fog, GI, traced reflections (RT) and
//! temporal reconstruction light it, seen from a camera fly path. `--sky uniform` changes
//! the sky; `--volumetric-fog` strengthens the default haze for shafts, and
//! `--no-volumetric-fog` disables it. Every `--every`-th frame is written as PNG. `--quarry MANIFEST` replaces the
//! ground with the registered quarry scans and its RLE2 sky (manifest keys `asset`,
//! `limits`, `environment`, `exposure_stops`), shown as a visible sky. `--render-scale`
//! picks a TR1 upscaling preset.
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
use std::{f32::consts::PI, fs, path::PathBuf, time::Instant};

const GAME_PROGRAM: &str = include_str!("game_scene.wgsl");
const FPS: f32 = 30.0;

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
        Ok(a)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let args = Args::parse()?;
    let adapter = headless_adapter()?;
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

    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            settings: Settings {
                global_illumination: args.gi,
                temporal_reconstruction: args.temporal,
                render_scale: args.scale,
                // Sun shafts through a light haze (0.01 per metre over the first 128 m).
                volumetric_fog: match args.volumetric_fog {
                    Some(true) => Some(VolumetricFog {
                        extra_density: 0.01,
                        ..Default::default()
                    }),
                    Some(false) => None,
                    None => Settings::default().volumetric_fog,
                },
                exposure: Exposure::Manual { stops: exposure },
                ..Default::default()
            },
            ..Default::default()
        },
    )?;
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

    // One program for the whole frame; the hero's material selects the toon style.
    let program = engine.create_program("game", GAME_PROGRAM)?;
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
        ("style", Value::U32(1)),
        ("bands", Value::F32(3.0)),
        ("shadow_tint", Value::Vec3([0.22, 0.14, 0.38])),
        ("rim_color", Value::Vec3([0.35, 0.5, 0.7])),
    ];
    let toon = engine.create_material(&MaterialDesc {
        base_color: [0.95, 0.55, 0.2, 1.0],
        program: Some(program),
        parameters: &hero_style,
        ..Default::default()
    })?;

    let cube = engine.create_mesh(&mesh(&cube_geometry()))?;
    let sphere = engine.create_mesh(&mesh(&sphere_geometry(48, 24)))?;
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
            let handles = engine.add_asset(asset)?;
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
    // A mirror-polished chrome sphere: traced mirror reflections on RT (M7-R1).
    engine.spawn(sphere, chrome, boxed(at(-1.2, 0.8, 5.6), Vec3::splat(0.8)))?;
    let spinner = engine.spawn(cube, paint, Mat4::from_translation(at(-2.5, 0.6, 3.0)))?;
    let hero_body = engine.spawn(sphere, toon, Mat4::IDENTITY)?;
    let hero_head = engine.spawn(sphere, toon, Mat4::IDENTITY)?;

    engine.set_sun(Some(Sun {
        travel: Vec3::new(-0.45, -0.75, -0.5),
        // Under the atmosphere this is the sun outside it (`Sun::default()`, 120 klux), which
        // its transmittance dims and warms; under `--sky uniform` it arrives as given.
        ..Default::default()
    }))?;
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
    for frame in 0..args.frames {
        let t = frame as f32 / FPS;
        // The hero walks a circle through the arch and bobs.
        let walk = 0.35 * t;
        let hero = at(
            3.0 * walk.sin(),
            0.55 + 0.12 * (6.0 * t).sin().abs(),
            -1.0 + 3.0 * walk.cos(),
        );
        engine.set_transform(hero_body, boxed(hero, Vec3::splat(0.55)))?;
        engine.set_transform(
            hero_head,
            boxed(hero + Vec3::new(0.0, 0.8, 0.0), Vec3::splat(0.35)),
        )?;
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
            hero.lerp(anchor + Vec3::Y, 0.5),
            Vec3::Y,
            55f32.to_radians(),
            0.05,
            500.0,
        )?;
        let begin = Instant::now();
        let report = engine.render(&camera)?;
        let cpu_ms = begin.elapsed().as_secs_f64() * 1e3;
        let written = frame % args.every == 0 || frame + 1 == args.frames;
        if written {
            let image = engine.capture()?;
            write_png(&args.out.join(format!("frame-{frame:03}.png")), &image)?;
        }
        frames.push(serde_json::json!({
            "frame": frame, "renderer_frame": report.frame.0, "cpu_render_ms": cpu_ms,
            "primary": format!("{:?}", report.primary), "soft_shadows": format!("{:?}", report.soft_shadows),
            "draws": report.scene.draws, "triangles": report.scene.triangles, "written": written,
        }));
    }
    let events = engine.poll()?;
    let spans: Vec<f64> = events
        .gpu
        .iter()
        .map(|g| g.render_span.as_secs_f64() * 1e3)
        .collect();
    let summary = serde_json::json!({
        "path": format!("{path:?}"), "adapter": engine.adapter_info().name,
        "debug_build": cfg!(debug_assertions), "size": args.size, "exposure_stops": exposure,
        "settings": format!("{:?}", engine.settings()), "objects": engine.object_count(),
        "quarry": args.quarry, "startup_to_scene_ready_seconds": ready.as_secs_f64(),
        "gpu_render_span_ms": spans, "frames": frames,
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
