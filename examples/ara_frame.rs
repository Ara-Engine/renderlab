//! One real frame on either GPU layer, and the comparison between the two.
//!
//! Renders the authored M5-A lighting fixture (six spheres across roughness and
//! metalness, a floor, a mirrored panel and a double-sided panel under a directional,
//! a point and a spot light) through the forward lit path and writes the exposed HDR
//! target. `--backend wgpu` is the existing renderer; `--backend ara` is the optional
//! ara-gpu backend (`--features backend-ara`), which is the only one that runs on
//! Metal. Both turn the same features off: local shadows always, and the atmosphere
//! unless `--atmosphere` asks for the physical sky (sky and sun disk, sun
//! transmittance, procedural environment, aerial perspective and height fog; no
//! volumetric fog, which the Ara frame does not assemble yet). `--sun-shadows` adds
//! ray-traced soft shadows for the sun (a 0.02 rad disk, one 40 m cascade); it needs
//! ray queries on either layer, makes every material double-sided as hardware shadows
//! require, and on wgpu takes the experimental ray-query acknowledgment
//! `RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK`.
//! `--traced` implies `--sun-shadows` and draws the frame with camera rays and
//! deferred shading instead of the mesh pass: the renderer's traced primary
//! visibility with `SoftShadows::Exact` and GI, temporal reconstruction and
//! reflections off, which is `HardwareRt` at `Quality::Reference`. Post is at its
//! defaults, motion blur included: on a traced frame its motion surface comes from the
//! hit records.
//!
//! `--frames N` renders a sequence: pose 0 to N-1 of the fixture's camera path (the
//! eye moves 0.04 m sideways per pose) through one renderer, so every frame after the
//! first has history: the previous camera for motion blur, and whatever later
//! families keep between frames. The same line on either layer writes one summary
//! per frame; `--compare` then reports every frame. `--still` holds pose 0 for the
//! whole sequence instead.
//!
//! `--gi` implies `--traced` and turns the world radiance cache on for sky visibility
//! (bounce and the cell filter off) with a table of 2^17 slots on both layers, small
//! enough that no device limit shrinks it. Each frame's whole table is written as
//! `table-NNN.u32` (little-endian words, large: keep them local) and summarised in
//! `frame.json` with digests of 256 equal chunks. `--compare` reports the slot count
//! and, frame by frame, how the tables differ: the first differing word and what it
//! holds, and the cells by key (present on one side only, direction bits, SH). It ends
//! with one verdict line, PASS or FAIL against the acceptance rule documented at
//! `HEADER_ULPS` below, and exits nonzero on FAIL. Without the raw tables it falls back
//! to the digests: equal digests pass, anything else is NO VERDICT.
//! Run both a moving and a `--still` sequence: a still view goes idle after eight
//! frames, and from then on its tables say nothing new. `--bounce` implies `--gi` and
//! turns the cache's bounce light on as well (gathers, light visibility and cell
//! shading). `--filter` implies `--bounce` and adds the cell filter at its default
//! bits, which makes the cache Renderlab's default configuration but for the table
//! size. `--reconstruction` implies `--traced` and turns temporal reconstruction on at its
//! default settings and native scale: each frame is rendered through a jittered camera
//! and resolved against its history, so only a sequence shows it. `--interactive` implies `--traced` and swaps the exact
//! soft shadows for `SoftShadows::Interactive`, the fused eight-direction trace with its
//! repair, which is `HardwareRt` at its realtime quality; with `--reconstruction` its
//! directions rotate and its ray count follows the reconstruction's budget.
//! `--reflections` implies `--traced` and turns traced reflections on at their
//! default roughness band: the shading program's own reflection rays.
//! `--scale quality|performance` implies `--reconstruction` and renders the frame at two
//! thirds or half the output size in each axis, resolved to the output's.
//! `--depth-of-field` adds depth of field at its default lens to the post chain, on
//! the mesh pass or the traced frame.
//! `--normal-map` implies `--traced` and gives the floor a tangent stream and a
//! tangent-space normal map of round studs, with its full mip chain.
//! `--local-shadows` implies `--traced` and shadows every point and spot light with the
//! shading program's own rays, at `LocalShadows`' defaults (off otherwise).
//! `--textures` gives the floor a base-colour
//! texture (a 64-texel checker with its full mip chain, tiled eight times, trilinear with
//! the renderer's anisotropy) so that a comparison covers texture upload, mip selection
//! and filtering; the fixture itself is untextured.
//! `--crevice` adds a slot to the scene, two 1.2 m walls 7 cm apart on the floor
//! with their open end toward the camera: the filter's contrast refine fires on the
//! floor cells inside it, which nothing in M5-A itself makes it do.
//!
//! ```text
//! cargo run --example ara_frame -- --backend wgpu --output out/wgpu
//! cargo run --features backend-ara --example ara_frame -- --backend ara --output out/ara
//! cargo run --example ara_frame -- --compare out/wgpu out/ara
//! ```
//!
//! Below native scale the HDR a run writes is the frame's shaded samples at the internal
//! size (all the wgpu renderer's capture returns there); the displayed image is the
//! output's size, so the display lines are the comparison of the resolved image.
//! Each run writes the exposed HDR target as `frame.rgba16f` (tightly packed rows) with
//! `frame.png` (Reinhard, sRGB; for looking at only), the displayed image after the
//! default post chain as `display.rgba8` and `display.png`, and `frame.json` (extent
//! and 24x16 grids of mean linear RGB and mean display bytes, small enough to paste
//! where the raw files cannot travel). `--compare` prefers the raw files and falls
//! back to the grids.
//!
//! `--scene game` renders the game scene (`examples/game_scene.rs` at its defaults)
//! instead of the fixture, through the engine-level API on either layer: the wgpu
//! `Engine` on its `HardwareRt` path, or the backend's engine. `--frames` follows the
//! example's animation and `--still` holds its first frame. The backend leaves out what
//! it refuses (it prints the list, and writes it in `frame.json`), so a comparison shows
//! those pieces as differences; `--supported` leaves out on wgpu what the backend does
//! not draw (volumetric fog, particles, decals and the glass pane's traced reflection,
//! by giving the pane the standard program without its `reflection_lobe`), for a
//! comparison of what both draw. `--without NAME[,NAME]` leaves a feature out on both
//! layers, to find which one a difference belongs to: `gi`, `reconstruction`,
//! `reflections`, `local-shadows`, `glass`, `lanterns` (the three local lights) or `glow`
//! (the bulbs' emission). `--timing` renders the frames without capturing or reading
//! any back and prints one line: the median, least and greatest time of a frame after
//! the first eight, each from the call to `render` until the GPU has finished the
//! frame, on either layer. It writes no images. `--probe X,Y` prints, every frame, the
//! HDR colour of that pixel and its eight neighbours, to see at which frame two runs
//! part. The other level flags do not apply.
// The fixture also carries names and a scene record for the Unreal match export.
#[allow(dead_code)]
#[path = "../support/lighting_fixture.rs"]
mod lighting_fixture;
// TEMPORARY: a copy of the example's scene, until both include the example's own.
#[path = "../support/ara_game_scene.rs"]
#[allow(dead_code)]
mod game;

use renderlab::decode_f16;
use std::{fs, path::Path};

type Failure = Box<dyn std::error::Error>;
const GRID: [usize; 2] = [24, 16];
/// The fixture's first light is its sun: one cascade over the scene, a 0.02 rad disk.
const SUN_SPLITS: [f32; 1] = [40.];
const SUN_DISK: f32 = 0.02;

fn main() -> Result<(), Failure> {
    let mut args = std::env::args().skip(1);
    let (mut backend, mut output, mut size) = (None, None, [960u32, 640]);
    let mut atmosphere = renderlab::Atmosphere::off();
    let mut sun_shadows = false;
    let mut traced = false;
    let (mut gi, mut bounce, mut filter, mut still) = (false, false, false, false);
    let (mut crevice, mut reconstruction, mut interactive) = (false, false, false);
    let (mut reflections, mut textures, mut local_shadows) = (false, false, false);
    let mut normal_map = false;
    let mut scale = renderlab::RenderScale::Native;
    let mut depth_of_field = false;
    let mut frames = 1usize;
    let (mut game_scene, mut supported) = (false, false);
    let mut without = [false; WITHOUT.len()];
    let mut timing = false;
    let mut probe = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scene" => match args.next().as_deref() {
                Some("game") => game_scene = true,
                _ => return Err("--scene game".into()),
            },
            "--supported" => supported = true,
            "--timing" => timing = true,
            "--probe" => {
                let value = args.next().ok_or("--probe X,Y")?;
                let (x, y) = value.split_once(',').ok_or("--probe X,Y")?;
                probe = Some([x.parse()?, y.parse()?]);
            }
            "--without" => {
                for name in args.next().ok_or("--without NAME[,NAME]")?.split(',') {
                    let at = WITHOUT
                        .iter()
                        .position(|n| *n == name)
                        .ok_or_else(|| format!("--without takes {}", WITHOUT.join(", ")))?;
                    without[at] = true;
                }
            }
            "--backend" => backend = args.next(),
            "--output" => output = args.next(),
            "--sun-shadows" => sun_shadows = true,
            "--traced" => (sun_shadows, traced) = (true, true),
            "--gi" => (sun_shadows, traced, gi) = (true, true, true),
            "--bounce" => (sun_shadows, traced, gi, bounce) = (true, true, true, true),
            "--crevice" => crevice = true,
            "--depth-of-field" => depth_of_field = true,
            "--textures" => textures = true,
            "--reflections" => (sun_shadows, traced, reflections) = (true, true, true),
            "--local-shadows" => (sun_shadows, traced, local_shadows) = (true, true, true),
            "--normal-map" => (sun_shadows, traced, normal_map) = (true, true, true),
            "--scale" => {
                (sun_shadows, traced, reconstruction) = (true, true, true);
                scale = match args.next().as_deref() {
                    Some("quality") => renderlab::RenderScale::Quality,
                    Some("performance") => renderlab::RenderScale::Performance,
                    _ => return Err("--scale quality|performance".into()),
                };
            }
            "--interactive" => (sun_shadows, traced, interactive) = (true, true, true),
            "--reconstruction" => (sun_shadows, traced, reconstruction) = (true, true, true),
            "--filter" => {
                (sun_shadows, traced, gi, bounce, filter) = (true, true, true, true, true)
            }
            "--still" => still = true,
            "--frames" => {
                frames = args.next().ok_or("--frames N")?.parse()?;
                if !(1..=240).contains(&frames) {
                    return Err("--frames 1..=240".into());
                }
            }
            "--atmosphere" => {
                atmosphere = renderlab::Atmosphere {
                    volumetric_fog: None,
                    ..Default::default()
                }
            }
            "--size" => {
                let value = args.next().ok_or("--size WIDTHxHEIGHT")?;
                let (w, h) = value.split_once('x').ok_or("--size WIDTHxHEIGHT")?;
                size = [w.parse()?, h.parse()?];
            }
            "--compare" => {
                let a = args.next().ok_or("--compare A B")?;
                let b = args.next().ok_or("--compare A B")?;
                return compare(Path::new(&a), Path::new(&b));
            }
            _ => return Err(format!("unknown {arg}").into()),
        }
    }
    let output = output.ok_or("--output DIR")?;
    if game_scene {
        // The engine's defaults, for the record `write` keeps and `--compare` matches.
        let level = Level {
            atmosphere: renderlab::Atmosphere::default(),
            sun_shadows: true,
            traced: true,
            gi: true,
            bounce: true,
            filter: true,
            crevice: false,
            reconstruction: true,
            interactive: true,
            reflections: true,
            textures: false,
            local_shadows: true,
            normal_map: false,
            scale: renderlab::RenderScale::Native,
            depth_of_field: false,
            still,
            frames,
            game: Some((supported, without)),
            timing,
            probe,
        };
        let sequence = match backend.as_deref() {
            Some("wgpu") => game_wgpu(size, level)?,
            Some("ara") => game_ara(size, level, Path::new(&output))?,
            _ => return Err("--backend wgpu|ara".into()),
        };
        if timing {
            // Nothing was read back: the run's result is the line it printed.
            return Ok(());
        }
        return write(
            Path::new(&output),
            size,
            &sequence,
            backend.as_deref().unwrap_or_default(),
            level,
        );
    }
    let mut fixture = lighting_fixture::Fixture::new();
    if crevice {
        // In front of the spheres, running away from the camera along -z.
        for x in [0.06f32, 0.13] {
            let vertices = [[x, 0., 0.9], [x, 0., 2.3], [x, 1.2, 2.3], [x, 1.2, 0.9]]
                .map(|position| renderlab::Vertex {
                    position,
                    normal: [1., 0., 0.],
                    uv: [0.; 2],
                    color: [1.; 4],
                })
                .to_vec();
            fixture.objects.push(lighting_fixture::Object {
                name: format!("slot-wall-{x}"),
                vertices,
                indices: vec![0, 1, 2, 0, 2, 3],
                transform: renderlab::Mat4::IDENTITY,
                base: [0.5, 0.5, 0.5, 1.],
                metal: 0.,
                rough: 0.5,
                double_sided: true,
                recipe: serde_json::json!({"shape": "world_quad"}),
            });
        }
    }
    if textures || normal_map {
        // The floor panel is a unit quad with no uv of its own: eight tiles across it.
        let floor = floor_index(&fixture)?;
        for vertex in &mut fixture.objects[floor].vertices {
            vertex.uv = [
                (vertex.position[0] + 0.5) * 8.,
                (vertex.position[1] + 0.5) * 8.,
            ];
        }
    }
    if sun_shadows {
        for object in &mut fixture.objects {
            object.double_sided = true;
        }
    }
    let level = Level {
        atmosphere,
        sun_shadows,
        traced,
        gi,
        bounce,
        filter,
        crevice,
        reconstruction,
        interactive,
        reflections,
        textures,
        local_shadows,
        normal_map,
        scale,
        depth_of_field,
        still,
        frames,
        game: None,
        timing: false,
        probe: None,
    };
    let sequence = match backend.as_deref() {
        Some("wgpu") => wgpu_frame(&fixture, size, level)?,
        Some("ara") => ara_frame(&fixture, size, level, Path::new(&output))?,
        _ => return Err("--backend wgpu|ara".into()),
    };
    write(
        Path::new(&output),
        size,
        &sequence,
        backend.as_deref().unwrap_or_default(),
        level,
    )
}

/// What a run renders: which features are on, and how many poses.
#[derive(Clone, Copy)]
struct Level {
    atmosphere: renderlab::Atmosphere,
    sun_shadows: bool,
    traced: bool,
    /// The radiance cache's sky visibility, on a traced frame.
    gi: bool,
    /// The cache's bounce light as well.
    bounce: bool,
    /// The cell filter as well, at its default bits.
    filter: bool,
    /// The scene has the slot that makes contrast refine fire.
    crevice: bool,
    /// Temporal reconstruction at its defaults, on a traced frame.
    reconstruction: bool,
    /// Interactive soft shadows instead of exact ones, on a traced frame.
    interactive: bool,
    /// Traced reflections at their defaults, on a traced frame.
    reflections: bool,
    /// The floor carries the checker base-colour texture.
    textures: bool,
    /// Point and spot lights are shadowed, at `LocalShadows`' defaults, on a traced frame.
    local_shadows: bool,
    /// The floor carries the stud normal map and a tangent stream, on a traced frame.
    normal_map: bool,
    /// The reconstruction's render scale: below native the frame is drawn at an
    /// internal size and resolved to the output's.
    scale: renderlab::RenderScale,
    /// Depth of field at `DepthOfField`'s defaults (focused at 10 m, f/2.8).
    depth_of_field: bool,
    /// Every frame from pose 0.
    still: bool,
    frames: usize,
    /// `--scene game`: the game scene through the engine API, whether `--supported`
    /// left out on wgpu what the backend refuses, and what `--without` left out on
    /// both, by index of `WITHOUT`.
    game: Option<(bool, [bool; WITHOUT.len()])>,
    /// `--timing`: frames are timed and not captured.
    timing: bool,
    /// `--probe X,Y`: a pixel whose HDR neighbourhood every frame prints.
    probe: Option<[u32; 2]>,
}
/// What `--without` can leave out of the game scene on both layers, to find which
/// feature a difference between them belongs to.
const WITHOUT: [&str; 7] = [
    "gi",
    "reconstruction",
    "reflections",
    "local-shadows",
    "glass",
    "lanterns",
    "glow",
];
impl Level {
    /// The size of the HDR image a run writes. Below native scale the wgpu renderer's
    /// capture is its internal-size shaded samples, not the resolved image, so both
    /// layers write those; the displayed image is the output's size either way.
    fn hdr_size(&self, size: [u32; 2]) -> [u32; 2] {
        if self.reconstruction {
            self.scale.internal_size(size[0], size[1])
        } else {
            size
        }
    }
    fn camera(&self, fixture: &lighting_fixture::Fixture, frame: usize) -> renderlab::Camera {
        fixture.camera(if self.still { 0 } else { frame })
    }
}

/// The fixture's floor object.
fn floor_index(fixture: &lighting_fixture::Fixture) -> Result<usize, Failure> {
    fixture
        .objects
        .iter()
        .position(|o| o.name == "floor")
        .ok_or_else(|| "the fixture has no floor".into())
}

/// The `--textures` floor: a 64-texel sRGB checker of 8-texel squares with every mip
/// level box-filtered from the one above, as an importer would cook it.
fn checker() -> renderlab::asset::Texture {
    use renderlab::asset::{Mip, Texture, TextureRole};
    let mut level: Vec<[f32; 3]> = (0..64 * 64)
        .map(|i| {
            if (i % 64 / 8 + i / 64 / 8) % 2 == 0 {
                [0.85, 0.80, 0.70]
            } else {
                [0.25, 0.30, 0.45]
            }
        })
        .collect();
    let mut mips = Vec::new();
    let mut side = 64usize;
    loop {
        mips.push(Mip {
            width: side as u32,
            height: side as u32,
            rgba8: level
                .iter()
                .flat_map(|c| {
                    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
                    [byte(c[0]), byte(c[1]), byte(c[2]), 255]
                })
                .collect(),
        });
        if side == 1 {
            break;
        }
        let half = side / 2;
        level = (0..half * half)
            .map(|i| {
                let (x, y) = (i % half * 2, i / half * 2);
                std::array::from_fn(|c| {
                    [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)]
                        .iter()
                        .map(|(x, y)| level[y * side + x][c])
                        .sum::<f32>()
                        / 4.
                })
            })
            .collect();
        side = half;
    }
    Texture {
        role: TextureRole::BaseColorSrgb,
        mips,
    }
}
/// The `--normal-map` floor: a 64-texel tangent-space map of round studs, four to a
/// tile, tilting up to about 19 degrees, with every mip level box-filtered from the one
/// above (not renormalised: both layers get the same bytes).
fn studs() -> renderlab::asset::Texture {
    use renderlab::asset::{Mip, Texture, TextureRole};
    let wave = |i: usize| (std::f32::consts::TAU * (i as f32 + 0.5) / 16.).sin() * 0.35;
    let mut level: Vec<[f32; 3]> = (0..64 * 64)
        .map(|i| {
            let n = renderlab::Vec3::new(wave(i % 64), wave(i / 64), 1.).normalize();
            [n.x, n.y, n.z].map(|v| v * 0.5 + 0.5)
        })
        .collect();
    let mut mips = Vec::new();
    let mut side = 64usize;
    loop {
        mips.push(Mip {
            width: side as u32,
            height: side as u32,
            rgba8: level
                .iter()
                .flat_map(|c| {
                    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
                    [byte(c[0]), byte(c[1]), byte(c[2]), 255]
                })
                .collect(),
        });
        if side == 1 {
            break;
        }
        let half = side / 2;
        level = (0..half * half)
            .map(|i| {
                let (x, y) = (i % half * 2, i / half * 2);
                std::array::from_fn(|c| {
                    [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)]
                        .iter()
                        .map(|(x, y)| level[y * side + x][c])
                        .sum::<f32>()
                        / 4.
                })
            })
            .collect();
        side = half;
    }
    Texture {
        role: TextureRole::NormalLinear,
        mips,
    }
}
/// The floor's tangent frames for `--normal-map`: along its own x, right-handed.
fn floor_tangents(
    fixture: &lighting_fixture::Fixture,
) -> Result<renderlab::asset::TangentStream, Failure> {
    let floor = &fixture.objects[floor_index(fixture)?];
    Ok(renderlab::asset::TangentStream {
        basis: renderlab::asset::TangentBasis::Authored,
        values: vec![[1., 0., 0., 1.]; floor.vertices.len()],
    })
}
/// The anisotropy clamp both layers sample material textures with.
const ANISOTROPY: u8 = 16;

/// The post chain both layers run: its defaults, with depth of field when asked.
fn post_settings(level: Level) -> renderlab::PostSettings {
    renderlab::PostSettings {
        depth_of_field: level.depth_of_field.then(renderlab::DepthOfField::default),
        ..Default::default()
    }
}

/// The cache at the `--gi`, `--bounce` or `--filter` level, on both layers.
fn cache_settings(level: Level) -> renderlab::IndirectSettings {
    let default = renderlab::IndirectSettings::default();
    renderlab::IndirectSettings {
        bounce: level.bounce,
        filter: if level.filter { default.filter } else { 0 },
        slots_log2: 17,
        ..default
    }
}

/// The exposed HDR target and the displayed image of one frame, the cache's table after
/// it at the `--gi` level, and what the layer reports about how it made the frame (the
/// reconstruction's restart and ray budget; on wgpu also the frame's uploads, which feed
/// the reconstruction's restart key).
type Frame = (Vec<u8>, Vec<u8>, Option<Vec<u32>>, serde_json::Value);

fn wgpu_frame(
    fixture: &lighting_fixture::Fixture,
    size: [u32; 2],
    level: Level,
) -> Result<Vec<Frame>, Failure> {
    use renderlab::*;
    let Level {
        atmosphere,
        sun_shadows,
        traced,
        gi,
        frames,
        ..
    } = level;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!("wgpu adapter: {:?}", adapter.get_info());
    let mut requested = wgpu::DeviceDescriptor {
        required_features: RECEIVER_FEATURES,
        ..Default::default()
    };
    if sun_shadows {
        if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
            != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        {
            return Err("--sun-shadows on wgpu needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK".into());
        }
        requested.required_features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY;
        requested.required_limits.max_blas_primitive_count = 2_000_000;
        requested.required_limits.max_blas_geometry_count = 1;
        requested.required_limits.max_tlas_instance_count = 4096;
        requested
            .required_limits
            .max_acceleration_structures_per_shader_stage = 1;
        // SAFETY: the acknowledgment above accepts wgpu's experimental ray-query
        // contract for this device, which only this example's frame uses.
        requested.experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    }
    if traced {
        requested.required_features |= TRACED_FEATURES;
        requested.required_limits = traced_limits(requested.required_limits);
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&requested))?;
    let options = RendererOptions {
        samples: SampleCount::One,
        anisotropy_clamp: ANISOTROPY as u16,
    };
    let (format, prelude) = (
        wgpu::TextureFormat::Rgba8UnormSrgb,
        include_str!("../shaders/scene.wgsl"),
    );
    let mut r = if sun_shadows {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            format,
            Limits::default(),
            options,
            HardwareShadowOptions {
                max_estimated_bytes: Some(1 << 30),
            },
            prelude,
        )?
    } else {
        Renderer::new_with_options(device, queue, format, Limits::default(), options, prelude)?
    };
    if traced {
        r.set_primary_visibility(PrimaryVisibility::Traced)?;
        r.set_soft_shadows(if level.interactive {
            SoftShadows::Interactive
        } else {
            SoftShadows::Exact
        });
        r.set_indirect(if gi {
            cache_settings(level)
        } else {
            IndirectSettings::off()
        })?;
        r.set_reconstruction(if level.reconstruction {
            Reconstruction {
                scale: level.scale,
                ..Default::default()
            }
        } else {
            Reconstruction::off()
        })?;
        r.set_reflections(if level.reflections {
            ReflectionSettings::default()
        } else {
            ReflectionSettings::off()
        })?;
    }
    // The post chain is the default on either family, with depth of field when asked.
    r.set_post_settings(post_settings(level))?;
    // The frame the ara backend assembles today: everything else is a later level.
    r.set_atmosphere(atmosphere)?;
    r.set_local_shadows(LocalShadows {
        count: if level.local_shadows {
            LocalShadows::default().count
        } else {
            0
        },
        ..Default::default()
    })?;
    if sun_shadows {
        let mut requests = vec![ShadowRequest::None; fixture.lighting.lights.len()];
        requests[0] = ShadowRequest::Directional {
            split_ends: SUN_SPLITS.to_vec(),
        };
        r.set_lighting_and_shadows(
            &fixture.lighting,
            &Shadows {
                requests,
                directional_disk_angle: SUN_DISK,
                ..Default::default()
            },
        )?;
    } else {
        r.set_lighting(&fixture.lighting)?;
    }
    let color = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("ara_frame output"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // Admission is asynchronous: retry each step while the renderer drains.
    fn admit<T>(
        r: &mut Renderer,
        mut step: impl FnMut(&mut Renderer) -> Result<Option<T>, renderlab::Error>,
    ) -> Result<T, Failure> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            if let Some(value) = step(r)? {
                return Ok(value);
            }
            r.poll()?;
            if std::time::Instant::now() > deadline {
                return Err("renderer admission deadline".into());
            }
            std::thread::yield_now();
        }
    }
    let (mut meshes, mut instances, mut materials) = (vec![], vec![], vec![]);
    let checker = checker();
    let floor = if level.textures {
        let texture = admit(&mut r, |r| r.try_upload_texture(&checker))?;
        Some((floor_index(fixture)?, texture))
    } else {
        None
    };
    let (studs, tangents) = (studs(), floor_tangents(fixture)?);
    let bumps = if level.normal_map {
        let texture = admit(&mut r, |r| r.try_upload_texture(&studs))?;
        Some((floor_index(fixture)?, texture))
    } else {
        None
    };
    for (index, o) in fixture.objects.iter().enumerate() {
        let bumped = bumps.as_ref().filter(|b| b.0 == index);
        meshes.push(admit(&mut r, |r| {
            r.try_upload_mesh_with_tangents(
                &MeshData {
                    vertices: &o.vertices,
                    indices: &o.indices,
                },
                bumped.map(|_| &tangents),
            )
        })?);
        instances.push(admit(&mut r, |r| r.try_upload_instances(&[o.transform]))?);
        materials.push(admit(&mut r, |r| {
            r.try_create_material(&MaterialDescriptor {
                base_color: o.base,
                metallic: o.metal,
                roughness: o.rough,
                double_sided: o.double_sided,
                base_color_texture: floor.as_ref().filter(|f| f.0 == index).map(|f| {
                    TextureBinding {
                        texture: &f.1,
                        sampler: asset::Sampler::default(),
                    }
                }),
                normal_texture: bumped.map(|b| TextureBinding {
                    texture: &b.1,
                    sampler: asset::Sampler::default(),
                }),
                ..Default::default()
            })
        })?);
    }
    let mut sequence = Vec::new();
    for pose in 0..frames {
        let camera = level.camera(fixture, pose);
        let report;
        {
            let mut draws: Vec<_> = instances
                .iter_mut()
                .enumerate()
                .map(|(i, inst)| Draw::new(&meshes[i], inst).with_material(&materials[i]))
                .collect();
            report = admit(&mut r, |r| {
                r.try_render_view(&color, &camera, &mut draws, ViewMode::Lit)
            })?;
        }
        let (u, stats) = (report.uploads, r.stats());
        let trace = serde_json::json!({
            "frame": report.frame.0,
            "revision": report.revision.0,
            "shadow_rays": r.reconstruction_shadow_rays(),
            "busy_frames": stats.busy_frames,
            "submissions": stats.submissions,
            "pipelines_created": stats.pipelines.created,
            // The categories `reconstruction_key` hashes, then the ones it does not.
            "uploads": {
                "environment": u.environment, "geometry": u.geometry, "lut": u.lut,
                "textures": u.textures, "materials": u.materials, "instances": u.instances,
                "models": u.models, "normals": u.normals, "lights": u.lights,
                "lighting": u.lighting, "shadows": u.shadows,
            },
        });
        eprintln!("wgpu pose {pose}: {trace}");
        let mut capture = admit(&mut r, |r| r.try_capture_hdr())?;
        let info = capture.info();
        if info.format != wgpu::TextureFormat::Rgba16Float
            || [info.width, info.height] != level.hdr_size(size)
        {
            return Err("unexpected HDR capture".into());
        }
        let hdr = admit(&mut r, |_| capture.try_read())?;
        let mut shown = admit(&mut r, |r| r.try_capture(&color))?;
        let display = admit(&mut r, |_| shown.try_read())?;
        let table = if gi { r.indirect_table()? } else { None };
        sequence.push((hdr, display, table, trace));
    }
    Ok(sequence)
}

#[cfg(feature = "backend-ara")]
fn ara_frame(
    fixture: &lighting_fixture::Fixture,
    size: [u32; 2],
    level: Level,
    output: &Path,
) -> Result<Vec<Frame>, Failure> {
    use renderlab::backend_ara::{
        cache::ShaderCache, forward, gi_cache::Indirect, temporal::Reconstruct,
    };
    let Level {
        atmosphere,
        sun_shadows,
        traced,
        gi,
        frames,
        ..
    } = level;
    let mut gpu = if sun_shadows {
        ara_gpu::GpuDevice::new(&[], output.join("device"))?
    } else {
        ara_gpu::GpuDevice::new_raster(&[], output.join("device"))?
    };
    let shadows = forward::SunShadows {
        split_ends: SUN_SPLITS.to_vec(),
        disk_angle: SUN_DISK,
    };
    let cache = ShaderCache::new(output.join("shaders"))?;
    let objects: Vec<_> = fixture
        .objects
        .iter()
        .map(|o| forward::ForwardObject {
            vertices: &o.vertices,
            indices: &o.indices,
            transform: o.transform,
            material: renderlab::MaterialDescriptor {
                base_color: o.base,
                metallic: o.metal,
                roughness: o.rough,
                double_sided: o.double_sided,
                ..Default::default()
            },
        })
        .collect();
    let checker = checker();
    let mut slots = vec![forward::ObjectTextures::default(); objects.len()];
    if level.textures {
        slots[floor_index(fixture)?].base_color = Some(forward::ObjectTexture {
            data: &checker,
            sampler: renderlab::asset::Sampler::default(),
        });
    }
    let (studs, tangents) = (studs(), floor_tangents(fixture)?);
    if level.normal_map {
        slots[floor_index(fixture)?].normal = Some(forward::NormalMap {
            texture: forward::ObjectTexture {
                data: &studs,
                sampler: renderlab::asset::Sampler::default(),
            },
            tangents: &tangents,
        });
    }
    let mut history = forward::Frames::default();
    let mut sequence = Vec::new();
    for pose in 0..frames {
        let pixels = history.render(
            &mut gpu,
            &cache,
            &forward::ForwardScene {
                meshes: None,
                materials: None,
                instances: None,
                objects: &objects,
                lighting: &fixture.lighting,
                camera: level.camera(fixture, pose),
                size,
                atmosphere,
                sun_shadows: sun_shadows.then_some(&shadows),
                primary: match (traced, level.interactive) {
                    (true, true) => forward::Primary::TracedInteractive,
                    (true, false) => forward::Primary::Traced,
                    _ => forward::Primary::Raster,
                },
                post: Some(post_settings(level)),
                indirect: gi.then(|| Indirect {
                    settings: cache_settings(level),
                    read_table: true,
                }),
                reconstruction: level.reconstruction.then(|| Reconstruct {
                    settings: renderlab::Reconstruction {
                        scale: level.scale,
                        ..Default::default()
                    },
                    read_history: false,
                }),
                reflections: level
                    .reflections
                    .then(renderlab::ReflectionSettings::default),
                textures: (level.textures || level.normal_map).then_some(forward::SceneTextures {
                    objects: &slots,
                    anisotropy_clamp: ANISOTROPY,
                }),
                local_shadows: level.local_shadows.then(renderlab::LocalShadows::default),
                transparent: &[],
                programs: None,
            },
        )?;
        // The planner budgets shadow rays on every reconstructed frame; only the
        // Interactive trace reads the budget, and only then does the renderer report it.
        let trace = history
            .reconstruction_plan()
            .map_or(serde_json::json!({}), |(reset, rays)| {
                serde_json::json!({
                    "reset": reset,
                    "shadow_rays": level.interactive.then_some(rays),
                })
            });
        eprintln!(
            "ara pose {pose}: {trace}; record {:.2} ms, gpu {:.2} ms, finish {:.2} ms",
            pixels.times.record.as_secs_f64() * 1e3,
            pixels.times.gpu.as_secs_f64() * 1e3,
            pixels.times.finish.as_secs_f64() * 1e3
        );
        sequence.push((
            pixels.samples.unwrap_or(pixels.hdr),
            pixels.display.ok_or("post did not run")?,
            pixels.indirect_table,
            trace,
        ));
    }
    history.retire(&mut gpu)?;
    cache.retire(&mut gpu)?;
    ara_gpu::Device::collect(&mut gpu)?;
    gpu.assert_clean()?;
    Ok(sequence)
}
#[cfg(not(feature = "backend-ara"))]
fn ara_frame(
    _: &lighting_fixture::Fixture,
    _: [u32; 2],
    _: Level,
    _: &Path,
) -> Result<Vec<Frame>, Failure> {
    Err("--backend ara needs --features backend-ara".into())
}

/// The game scene's frames on any layer's engine: `engine` already holds the scene's
/// settings, and `hdr` reads the last frame's exposed HDR from it. `trace` goes into
/// every frame's record.
///
/// With `timing`, no frame is captured or read: each frame's time is taken from the
/// call to `render` until `settle` has waited for the GPU to finish it, and the run
/// prints one line (the median, least and greatest of the frames after the first
/// eight) and returns no frames.
fn game_frames<S: renderlab::engine::Scene>(
    engine: &mut S,
    size: [u32; 2],
    level: Level,
    options: game::Options,
    mut settle: impl FnMut(&mut S) -> Result<(), Failure>,
    mut hdr: impl FnMut(&mut S) -> Result<Vec<u8>, Failure>,
    mut trace: impl FnMut(&mut S) -> serde_json::Value,
) -> Result<Vec<Frame>, Failure> {
    engine.resize(size[0], size[1])?;
    let built = game::build(engine, options)?;
    let mut sequence = Vec::new();
    let mut times = Vec::new();
    let mut records = Vec::new();
    let timing = level.timing;
    for frame in 0..level.frames as u32 {
        let camera = game::animate(engine, &built, if level.still { 0 } else { frame })?;
        let rendering = std::time::Instant::now();
        let report = engine.render(&camera)?;
        settle(engine)?;
        let render_ms = (rendering.elapsed().as_secs_f64() * 1e5).round() / 1e2;
        let mut record = trace(engine);
        record["draws"] = report.scene.draws.into();
        record["triangles"] = report.scene.triangles.into();
        record["render_ms"] = render_ms.into();
        eprintln!("pose {frame}: {record}");
        if timing {
            times.push(render_ms);
            records.push(record);
        } else {
            let display = engine.capture()?.rgba8;
            let hdr = hdr(engine)?;
            if let Some([x, y]) = level.probe {
                // The pixel's row and the rows above and below, three pixels each.
                let rows: Vec<String> = (y.saturating_sub(1)..=(y + 1).min(size[1] - 1))
                    .map(|row| {
                        (x.saturating_sub(1)..=(x + 1).min(size[0] - 1))
                            .map(|column| {
                                let at = ((row * size[0] + column) * 8) as usize;
                                let [r, g, b] = [0, 2, 4].map(|c| {
                                    decode_f16(u16::from_le_bytes([hdr[at + c], hdr[at + c + 1]]))
                                });
                                format!("({r:.4} {g:.4} {b:.4})")
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                eprintln!("probe pose {frame} at ({x}, {y}): {}", rows.join(" / "));
            }
            sequence.push((hdr, display, None, record));
        }
    }
    if timing {
        const WARM: usize = 8;
        let mut steady = times.get(WARM..).unwrap_or_default().to_vec();
        if steady.is_empty() {
            return Err(format!("--timing needs more than {WARM} frames").into());
        }
        steady.sort_by(f64::total_cmp);
        eprintln!(
            "timing: {} frames after the first {WARM} at {}x{}: median {:.2} ms, least {:.2}, greatest {:.2}; first frame {:.2} ms",
            steady.len(),
            size[0],
            size[1],
            steady[steady.len() / 2],
            steady[0],
            steady[steady.len() - 1],
            times[0]
        );
        // Where the time went: the median over the same frames of every number a
        // frame's record carries, one level down (a layer's own passes and phases).
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let mut split = Vec::new();
        let mut fields = |of: &dyn Fn(&serde_json::Value) -> Option<&serde_json::Value>| {
            let Some(serde_json::Value::Object(first)) = records.get(WARM).and_then(of) else {
                return;
            };
            for key in first.keys() {
                let mut values: Vec<f64> = records[WARM..]
                    .iter()
                    .filter_map(|r| of(r)?.get(key)?.as_f64())
                    .collect();
                if values.len() == records.len() - WARM {
                    split.push(format!("{key} {:.2}", median(&mut values)));
                }
            }
        };
        fields(&|r| Some(r));
        fields(&|r| r.get("passes_ms"));
        eprintln!("timing split, medians in ms: {}", split.join(", "));
    }
    Ok(sequence)
}
/// The scene's settings and parts for a run: all of them, or without what the backend
/// refuses.
fn game_ask(
    (supported, without): (bool, [bool; WITHOUT.len()]),
) -> (renderlab::engine::Settings, game::Options) {
    let [
        gi,
        reconstruction,
        reflections,
        local_shadows,
        glass,
        lanterns,
        glow,
    ] = without;
    let mut settings = game::settings();
    if supported {
        settings.volumetric_fog = None;
    }
    settings.global_illumination &= !gi;
    settings.temporal_reconstruction &= !reconstruction;
    if reflections {
        settings.reflections = renderlab::ReflectionSettings::off();
    }
    if local_shadows {
        settings.local_shadows.count = 0;
    }
    (
        settings,
        game::Options {
            toon_hero: true,
            particles: !supported,
            decals: !supported,
            pane_reflection: !supported,
            glass: !glass,
            glow: !glow,
            lanterns: !lanterns,
        },
    )
}

fn game_wgpu(size: [u32; 2], level: Level) -> Result<Vec<Frame>, Failure> {
    use renderlab::engine::*;
    if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
        != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    {
        return Err("--scene game on wgpu needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK".into());
    }
    let adapter = headless_adapter()?;
    eprintln!("wgpu adapter: {:?}", adapter.get_info());
    let (settings, options) = game_ask(level.game.unwrap_or_default());
    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path: RenderPath::HardwareRt,
            // SAFETY: the acknowledgment above accepts wgpu's experimental ray-query
            // contract for this engine's device.
            experimental_ray_query: Some(unsafe { ExperimentalRayQuery::acknowledge() }),
            settings,
            ..Default::default()
        },
    )?;
    game_frames(
        &mut engine,
        size,
        level,
        options,
        // The engine returns once the frame is submitted: wait for the GPU to finish it.
        |engine| {
            engine
                .device()
                .poll(renderlab::wgpu::PollType::wait_indefinitely())?;
            Ok(())
        },
        |engine| {
            let mut capture = engine
                .renderer_mut()
                .try_capture_hdr()?
                .ok_or("HDR capture busy")?;
            engine
                .device()
                .poll(renderlab::wgpu::PollType::wait_indefinitely())?;
            Ok(capture.try_read()?.ok_or("HDR readback pending")?)
        },
        // The renderer's own timestamps for the frames that have completed.
        |engine| {
            let ms = |d: std::time::Duration| (d.as_secs_f64() * 1e5).round() / 1e2;
            let mut record = serde_json::json!({});
            // The Interactive trace's shadow rays a pixel this frame, as the planner
            // budgeted them.
            if let Some(rays) = engine.renderer_mut().reconstruction_shadow_rays() {
                record["shadow_rays"] = rays.into();
            }
            if let Ok(events) = engine.poll()
                && let Some(sample) = events.gpu.last()
            {
                record["gpu_render_span_ms"] = ms(sample.render_span).into();
                record["gpu_shading_ms"] = ms(sample.mesh_pass).into();
                for (name, pass) in [
                    ("gpu_camera_rays_ms", sample.receiver_pass),
                    ("gpu_visibility_ms", sample.visibility_compute),
                    ("gpu_reconstruction_ms", sample.reconstruction_pass),
                    ("gpu_present_ms", sample.present_pass),
                    ("gpu_shadow_generation_ms", sample.shadow_generation),
                ] {
                    if let Some(pass) = pass {
                        record[name] = ms(pass).into();
                    }
                }
            }
            record
        },
    )
}

#[cfg(feature = "backend-ara")]
fn game_ara(size: [u32; 2], level: Level, output: &Path) -> Result<Vec<Frame>, Failure> {
    use ara_gpu::Device as _;
    use renderlab::backend_ara::{cache::ShaderCache, engine::Engine};
    let (settings, options) = game_ask(level.game.unwrap_or_default());
    let mut engine = Engine::new(
        ara_gpu::GpuDevice::new(&[], output.join("device"))?,
        ShaderCache::new(output.join("shaders"))?,
        settings,
    )?;
    engine.set_readback(!level.timing)?;
    let sequence = game_frames(
        &mut engine,
        size,
        level,
        options,
        // The backend's `render` returns when the GPU has finished the frame.
        |_| Ok(()),
        |engine| Ok(engine.images().ok_or("no frame")?.hdr.clone()),
        |engine| {
            let refusals: Vec<_> = engine
                .refusals()
                .iter()
                .map(|r| format!("{}: {}", r.call, r.what))
                .collect();
            let ms = |d: std::time::Duration| (d.as_secs_f64() * 1e5).round() / 1e2;
            let times = engine.images().map(|i| i.times.clone()).unwrap_or_default();
            let passes: serde_json::Map<_, _> = times
                .passes
                .iter()
                .map(|(label, t)| (label.clone(), ((t * 1e2).round() / 1e2).into()))
                .collect();
            // The same planner's budget, and whether it restarted every pixel.
            let (restarted, shadow_rays) = engine.reconstruction_plan().unzip();
            serde_json::json!({
                "refusals": refusals,
                "restarted": restarted.map(u32::from),
                "shadow_rays": shadow_rays,
                "record_ms": ms(times.record),
                "geometry_ms": ms(times.geometry),
                "gpu_ms": ms(times.gpu),
                "finish_ms": ms(times.finish),
                "passes_ms": passes,
            })
        },
    )?;
    for refusal in engine.refusals() {
        eprintln!("ara refused {}: {}", refusal.call, refusal.what);
    }
    let mut gpu = engine.retire()?;
    gpu.collect()?;
    Ok(sequence)
}
#[cfg(not(feature = "backend-ara"))]
fn game_ara(_: [u32; 2], _: Level, _: &Path) -> Result<Vec<Frame>, Failure> {
    Err("--backend ara needs --features backend-ara".into())
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| decode_f16(u16::from_le_bytes(*b)))
        .collect()
}

/// Mean linear RGB of each grid cell, row-major.
fn grid(values: &[f32], size: [u32; 2]) -> Vec<[f64; 3]> {
    let (w, h) = (size[0] as usize, size[1] as usize);
    let mut cells = vec![([0f64; 3], 0u32); GRID[0] * GRID[1]];
    for (i, p) in values.as_chunks::<4>().0.iter().enumerate() {
        let cell = &mut cells[(i / w) * GRID[1] / h * GRID[0] + (i % w) * GRID[0] / w];
        for (sum, v) in cell.0.iter_mut().zip(p) {
            *sum += f64::from(*v);
        }
        cell.1 += 1;
    }
    cells
        .into_iter()
        .map(|(sum, n)| sum.map(|v| v / f64::from(n.max(1))))
        .collect()
}

fn write(
    dir: &Path,
    size: [u32; 2],
    sequence: &[Frame],
    backend: &str,
    level: Level,
) -> Result<(), Failure> {
    let pixels = size[0] as usize * size[1] as usize;
    let hdr_size = level.hdr_size(size);
    let hdr_pixels = hdr_size[0] as usize * hdr_size[1] as usize;
    let mut summaries = Vec::new();
    let mut nonfinite = 0;
    fs::create_dir_all(dir)?;
    for (pose, (bytes, display, table, trace)) in sequence.iter().enumerate() {
        if bytes.len() != hdr_pixels * 8 || display.len() != pixels * 4 {
            return Err("frame is not tightly packed RGBA16F with an RGBA8 display".into());
        }
        let values = floats(bytes);
        let display_values: Vec<f32> = display.iter().map(|v| f32::from(*v)).collect();
        let bad = values.iter().filter(|v| !v.is_finite()).count();
        nonfinite += bad;
        let mut summary = serde_json::json!({
            "pose": pose,
            "nonfinite_channels": bad,
            "grid_mean_linear_rgb": grid(&values, hdr_size),
            "grid_mean_display_bytes": grid(&display_values, size),
            "trace": trace,
        });
        if let Some(table) = table {
            summary["table"] = Table::new(table)?.summary();
            let raw: Vec<u8> = table.iter().flat_map(|w| w.to_le_bytes()).collect();
            fs::write(dir.join(format!("table-{pose:03}.u32")), raw)?;
        }
        summaries.push(summary);
    }
    // The raw files and the pictures are the last frame's: the one with most history.
    let (bytes, display, ..) = sequence.last().ok_or("no frame was rendered")?;
    fs::write(dir.join("frame.rgba16f"), bytes)?;
    fs::write(dir.join("display.rgba8"), display)?;
    let mut shown = png::Encoder::new(fs::File::create(dir.join("display.png"))?, size[0], size[1]);
    shown.set_color(png::ColorType::Rgba);
    shown.set_depth(png::BitDepth::Eight);
    shown.write_header()?.write_image_data(display)?;
    let rgb: Vec<u8> = floats(bytes)
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            [p[0], p[1], p[2]].map(|v| {
                let linear = v.max(0.) / (1. + v.max(0.));
                let srgb = if linear <= 0.0031308 {
                    linear * 12.92
                } else {
                    1.055 * linear.powf(1. / 2.4) - 0.055
                };
                (srgb.clamp(0., 1.) * 255.).round() as u8
            })
        })
        .collect();
    let mut png = png::Encoder::new(
        fs::File::create(dir.join("frame.png"))?,
        hdr_size[0],
        hdr_size[1],
    );
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(&rgb)?;
    let last = summaries.last().cloned().unwrap_or_default();
    fs::write(
        dir.join("frame.json"),
        // Compact: a sequence's grids are large, and this file travels as text.
        serde_json::to_vec(&serde_json::json!({
            "scene": if let Some((supported, without)) = level.game {
                let left_out: Vec<&str> = WITHOUT
                    .iter()
                    .zip(without)
                    .filter_map(|(name, off)| off.then_some(*name))
                    .collect();
                format!(
                    "the game scene at its defaults{}{}, {}",
                    if supported { " without volumetric fog, particles, decals or the pane's traced reflection" } else { "" },
                    if left_out.is_empty() { String::new() } else { format!(", and without {}", left_out.join(", ")) },
                    if level.still { "every frame its first" } else { "its animation from frame 0" }
                )
            } else { format!(
                "M5-A authored v1{}{}{}, {}, local shadows {}, default post{}{}{}{}{}",
                if level.crevice { " with a slot between two walls" } else { "" },
                if level.textures { " with a checker-textured floor" } else { "" },
                if level.normal_map { " with a stud normal map on the floor" } else { "" },
                if level.still { "every frame from pose 0" } else { "camera path from pose 0" },
                if level.local_shadows { "traced for every local light" } else { "off" },
                if level.depth_of_field { " with depth of field" } else { "" },
                if level.reconstruction { ", temporal reconstruction at its defaults" } else { "" },
                match level.scale {
                    renderlab::RenderScale::Native => "",
                    renderlab::RenderScale::Quality => " at two thirds scale",
                    renderlab::RenderScale::Performance => " at half scale",
                },
                if level.interactive { ", Interactive soft shadows" } else { "" },
                if level.reflections { ", traced reflections" } else { "" }
            ) },
                                    "primary": match (level.traced, level.gi, level.bounce) {
                _ if level.game.is_some() => "the engine's HardwareRt path at the scene's settings",
                (true, true, true) if level.filter => "traced, exact soft shadows, radiance cache with bounce and the cell filter (2^17 slots), post at its defaults",
                (true, true, true) => "traced, exact soft shadows, radiance cache with bounce (2^17 slots, no filter), post at its defaults",
                (true, true, false) => "traced, exact soft shadows, radiance cache sky visibility (2^17 slots, no bounce or filter), post at its defaults",
                (true, false, _) => "traced, exact soft shadows, no GI, post at its defaults",
                _ => "mesh pass",
            },
            "atmosphere": if level.atmosphere.active() { "physical sky without volumetric fog" } else { "off" },
            "sun_shadows": if level.sun_shadows { "ray-traced, 0.02 rad disk, all materials double-sided" } else { "off" },
            "backend": backend,
                        "width": size[0],
            "height": size[1],
            // The HDR image's own size: the internal one below native scale.
            "hdr_width": hdr_size[0],
            "hdr_height": hdr_size[1],
            "nonfinite_channels": nonfinite,
            "grid": GRID,
            // The last frame's grids, where a single-frame run has always had them.
            "grid_mean_linear_rgb": last["grid_mean_linear_rgb"],
            "grid_mean_display_bytes": last["grid_mean_display_bytes"],
            "frames": sequence.len(),
            "sequence": summaries,
        }))?,
    )?;
    eprintln!(
        "{backend}: {} frame(s) {}x{} written to {}; nonfinite channels {nonfinite}",
        sequence.len(),
        size[0],
        size[1],
        dir.display()
    );
    Ok(())
}

fn compare(a: &Path, b: &Path) -> Result<(), Failure> {
    let record = |dir: &Path| -> Result<serde_json::Value, Failure> {
        Ok(serde_json::from_slice(&fs::read(dir.join("frame.json"))?)?)
    };
    let (ja, jb) = (record(a)?, record(b)?);
    if ja["width"] != jb["width"] || ja["height"] != jb["height"] {
        return Err("frames differ in extent".into());
    }
    // Frames written before the traced level existed have no "primary" key.
    let primary = |j: &serde_json::Value| j["primary"].as_str().unwrap_or("mesh pass").to_owned();
    if ja["atmosphere"] != jb["atmosphere"]
        || ja["sun_shadows"] != jb["sun_shadows"]
        || primary(&ja) != primary(&jb)
        || ja["scene"] != jb["scene"]
    {
        return Err("frames differ in scene, atmosphere, sun shadow or primary setting".into());
    }
    println!(
        "{} ({}) against {} ({}), {}x{}",
        a.display(),
        ja["backend"],
        b.display(),
        jb["backend"],
        ja["width"],
        ja["height"]
    );
    let cells = |j: &serde_json::Value, name: &str| -> Result<Vec<[f64; 3]>, Failure> {
        Ok(serde_json::from_value(j[name].clone())?)
    };
    // Sequences: every frame's grids. A run from before sequences existed is one frame.
    let count = |j: &serde_json::Value| j["frames"].as_u64().unwrap_or(1);
    if count(&ja) != count(&jb) {
        return Err("frames differ in sequence length".into());
    }
    if count(&ja) > 1 {
        for (fa, fb) in ja["sequence"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(jb["sequence"].as_array().into_iter().flatten())
        {
            let hdr = cells(fa, "grid_mean_linear_rgb")?
                .iter()
                .zip(&cells(fb, "grid_mean_linear_rgb")?)
                .flat_map(|(x, y)| {
                    (0..3).map(move |c| (x[c] - y[c]).abs() / x[c].abs().max(y[c].abs()).max(1e-3))
                })
                .fold(0f64, f64::max);
            let shown = cells(fa, "grid_mean_display_bytes")?
                .iter()
                .zip(&cells(fb, "grid_mean_display_bytes")?)
                .flat_map(|(x, y)| (0..3).map(move |c| (x[c] - y[c]).abs()))
                .fold(0f64, f64::max);
            println!(
                "pose {}: worst HDR cell mean differs by {:.4}%, worst display cell mean by {shown:.3} levels",
                fa["pose"],
                hdr * 100.
            );
            if !fa["trace"].is_null() || !fb["trace"].is_null() {
                println!(
                    "pose {}: made as {} / {}",
                    fa["pose"], fa["trace"], fb["trace"]
                );
            }
        }
        println!("last frame:");
    }
    // The verdict on the cache tables decides the exit status, after the images print.
    let verdict = |failure: Option<String>| -> Result<(), Failure> {
        failure.map_or(Ok(()), |f| Err(f.into()))
    };
    let tables = compare_tables(a, b, &ja, &jb)?;
    // Relative for linear radiance (floored so black does not dominate); display bytes
    // are compared in levels.
    let (ga, gb) = (
        cells(&ja, "grid_mean_linear_rgb")?,
        cells(&jb, "grid_mean_linear_rgb")?,
    );
    let (mut worst, mut worst_cell) = (0f64, 0);
    for (i, (x, y)) in ga.iter().zip(&gb).enumerate() {
        for c in 0..3 {
            let relative = (x[c] - y[c]).abs() / x[c].abs().max(y[c].abs()).max(1e-3);
            if relative > worst {
                (worst, worst_cell) = (relative, i);
            }
        }
    }
    println!(
        "HDR grid: worst cell mean differs by {:.4}% at column {}, row {}",
        worst * 100.,
        worst_cell % GRID[0],
        worst_cell / GRID[0]
    );
    let (da, db) = (
        cells(&ja, "grid_mean_display_bytes")?,
        cells(&jb, "grid_mean_display_bytes")?,
    );
    let levels = da
        .iter()
        .zip(&db)
        .flat_map(|(x, y)| (0..3).map(move |c| (x[c] - y[c]).abs()))
        .fold(0f64, f64::max);
    println!("display grid: worst cell mean differs by {levels:.3} of 255 levels");
    if let (Ok(xa), Ok(xb)) = (
        fs::read(a.join("display.rgba8")),
        fs::read(b.join("display.rgba8")),
    ) {
        if xa.len() != xb.len() {
            return Err("raw displays differ in length".into());
        }
        let mut histogram = [0usize; 256];
        for (pa, pb) in xa.as_chunks::<4>().0.iter().zip(xb.as_chunks::<4>().0) {
            let worst = (0..3).map(|c| pa[c].abs_diff(pb[c])).max().unwrap_or(0);
            histogram[usize::from(worst)] += 1;
        }
        let pixels = xa.len() / 4;
        let over = |limit: usize| histogram[limit + 1..].iter().sum::<usize>();
        println!(
            "display pixels: {pixels}; identical {}; worst channel difference {} levels; over 1 / 2 / 8 levels: {} / {} / {}",
            histogram[0],
            histogram.iter().rposition(|n| *n > 0).unwrap_or(0),
            over(1),
            over(2),
            over(8)
        );
    } else {
        println!("raw displays not both present; grid comparison only");
    }
    let (Ok(ra), Ok(rb)) = (
        fs::read(a.join("frame.rgba16f")),
        fs::read(b.join("frame.rgba16f")),
    ) else {
        println!("raw frames not both present; grid comparison only");
        return verdict(tables);
    };
    let (fa, fb) = (floats(&ra), floats(&rb));
    if fa.len() != fb.len() {
        return Err("raw frames differ in length".into());
    }
    // Errors are relative to the brighter value, floored so black does not dominate.
    let (mut max_abs, mut max_rel, mut sum_rel) = (0f32, 0f32, 0f64);
    let mut over = [0usize; 3];
    let mut worst_pixel = 0;
    for (i, (pa, pb)) in fa
        .as_chunks::<4>()
        .0
        .iter()
        .zip(fb.as_chunks::<4>().0)
        .enumerate()
    {
        let mut pixel = 0f32;
        for c in 0..3 {
            let absolute = (pa[c] - pb[c]).abs();
            max_abs = max_abs.max(absolute);
            pixel = pixel.max(absolute / pa[c].abs().max(pb[c].abs()).max(1e-3));
        }
        if pixel > max_rel {
            (max_rel, worst_pixel) = (pixel, i);
        }
        sum_rel += f64::from(pixel);
        for (count, limit) in over.iter_mut().zip([0.01, 0.05, 0.25]) {
            *count += usize::from(pixel > limit);
        }
    }
    let pixels = fa.len() / 4;
    let width = ja["hdr_width"]
        .as_u64()
        .or(ja["width"].as_u64())
        .unwrap_or(1) as usize;
    println!(
        "pixels: {pixels}; max absolute {max_abs:.6}; max relative {:.4}% at ({}, {}); mean relative {:.5}%",
        max_rel * 100.,
        worst_pixel % width,
        worst_pixel / width,
        sum_rel / pixels as f64 * 100.
    );
    println!(
        "pixels over 1% / 5% / 25% relative: {} / {} / {} ({:.4}% / {:.4}% / {:.4}%)",
        over[0],
        over[1],
        over[2],
        over[0] as f64 / pixels as f64 * 100.,
        over[1] as f64 / pixels as f64 * 100.,
        over[2] as f64 / pixels as f64 * 100.
    );
    verdict(tables)
}

/// A cache table as `Renderer::indirect_table` returns it: 64 header words, a 24-word
/// hot record per slot, then a cold record per slot (`shaders/gi_cells.wgsl`).
struct Table<'a> {
    words: &'a [u32],
    slots: usize,
    cold: usize,
}
const TABLE_CHUNKS: usize = 256;
impl<'a> Table<'a> {
    fn new(words: &'a [u32]) -> Result<Self, Failure> {
        let slots = words.get(1).map_or(0, |mask| *mask as usize + 1);
        let cold = words.get(23).map_or(0, |n| (*n as usize).max(24));
        if !slots.is_power_of_two() || words.len() != 64 + slots * (24 + cold) {
            return Err("cache table length does not match its header".into());
        }
        Ok(Self { words, slots, cold })
    }
    fn hot(&self, slot: usize) -> &'a [u32] {
        &self.words[64 + slot * 24..][..24]
    }
    fn cold(&self, slot: usize) -> &'a [u32] {
        &self.words[64 + self.slots * 24 + slot * self.cold..][..self.cold]
    }
    /// What a word holds, for naming a difference.
    fn field(&self, word: usize) -> String {
        if word < 64 {
            return format!("header word {word}");
        }
        let hot_end = 64 + self.slots * 24;
        if word < hot_end {
            let (slot, at) = ((word - 64) / 24, (word - 64) % 24);
            let name = match at {
                0 => "key a and valid bit",
                1 => "SH scale",
                2..=14 => "SH halves",
                15 => "last SH half and split flag",
                16..=22 => "bounce SH",
                _ => "query normal",
            };
            return format!("slot {slot}, hot word {at} ({name})");
        }
        let (slot, at) = ((word - hot_end) / self.cold, (word - hot_end) % self.cold);
        let name = match at {
            0 => "key b",
            1 => "state",
            2 => "seen stamp",
            3 => "definition",
            4..=6 => "cell",
            7 => "query normal",
            8..=10 => "query point",
            11 => "ray item base",
            12..=13 => "direction bits",
            14..=15 => "known directions",
            16..=17 => "pending directions",
            18..=19 => "near-field directions",
            20..=22 => "material",
            _ => "light bits",
        };
        format!("slot {slot}, cold word {at} ({name})")
    }
    /// Occupied slots by key (a without the valid bit, then b).
    fn cells(&self) -> std::collections::BTreeMap<(u32, u32), usize> {
        (0..self.slots)
            .filter(|slot| self.hot(*slot)[0] != 0)
            .map(|slot| ((self.hot(slot)[0] & 0x7fff_ffff, self.cold(slot)[0]), slot))
            .collect()
    }
    fn summary(&self) -> serde_json::Value {
        use sha2::Digest;
        let digest = |words: &[u32]| {
            let mut h = sha2::Sha256::new();
            for word in words {
                h.update(word.to_le_bytes());
            }
            h.finalize()
                .iter()
                .take(8)
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let valid = (0..self.slots)
            .filter(|slot| self.hot(*slot)[0] & 0x8000_0000 != 0)
            .count();
        serde_json::json!({
            "words": self.words.len(),
            "slots": self.slots,
            "cold_words": self.cold,
            "stamp": self.words[3],
            "occupied": self.cells().len(),
            "valid": valid,
            "digest": digest(self.words),
            // Equal word ranges, the last one taking the remainder.
            "chunks": (0..TABLE_CHUNKS)
                .map(|i| digest(&self.words[chunk(self.words.len(), i)]))
                .collect::<Vec<_>>(),
        })
    }
}
fn chunk(words: usize, index: usize) -> std::ops::Range<usize> {
    let size = words / TABLE_CHUNKS;
    let end = if index + 1 == TABLE_CHUNKS {
        words
    } else {
        (index + 1) * size
    };
    index * size..end
}

/// The acceptance rule for two layers' cache tables. Bit-identical tables are not
/// expected: hit positions and ray directions differ by rounding between shader
/// compilers and drivers, which moves a pixel across a cell or level boundary now and
/// then and flips a ray that grazes an edge. A frame passes when all four hold, and a
/// comparison passes when every frame does:
///
/// 1. **Same plan.** The header's integer words are equal (the frame stamp among them)
///    and its float words are within `HEADER_ULPS`: the header is computed on the CPU
///    by code both layers share, so only another machine's `tan` may move it.
/// 2. **Same cells.** Keys present, or valid, on one side only are at most
///    `ONE_SIDED_CELLS` of the larger side's cells: boundary pixels, not a missing stage.
/// 3. **Same rays.** Over cells valid on both sides, differing direction bits are at
///    most `FLIPPED_RAYS` of their rays: edge grazers, not a wrong origin or basis.
/// 4. **Same values.** Every cell valid on both sides with equal direction bits has
///    each SH coefficient within `SH_BOUND` of its scale: f16 storage and a sampled
///    environment, not a wrong weight.
/// 5. **Same bounce.** Of those cells, the ones with a bounce coefficient further than
///    `BOUNCE_BOUND` of its scale apart are at most `BOUNCE_CELLS`: a gather sums the
///    radiance of up to 64 other cells, so one boundary cell or grazing ray elsewhere
///    moves a neighbour by a ray's share; more than that is a wrong gather or shade.
/// 6. **Same splits.** Of those cells, the ones whose split flag (the probe's two-layer
///    mark or the filter's contrast refine) differs are at most `SPLIT_CELLS` of the
///    larger side's split cells, or `SPLIT_FLOOR` cells: a cell at the refine's
///    threshold may fall either way, a refine that never fires may not.
///
/// The bounds were set before any cross-layer run existed; the output prints the
/// measured figure beside each so they can be tightened.
const HEADER_ULPS: u32 = 2;
const HEADER_FLOATS: [usize; 7] = [4, 5, 8, 9, 10, 12, 25];
const ONE_SIDED_CELLS: f64 = 0.005;
const FLIPPED_RAYS: f64 = 0.001;
const SH_BOUND: f32 = 0.01;
const BOUNCE_BOUND: f32 = 0.02;
const BOUNCE_CELLS: f64 = 0.01;
const SPLIT_CELLS: f64 = 0.1;
const SPLIT_FLOOR: usize = 2;

/// The cache tables of two `--gi` runs, frame by frame, against the acceptance rule
/// above. Prints the slot count, each frame's measurements and first differing word,
/// and one verdict line; returns the failure, if any, for the exit status.
fn compare_tables(
    a: &Path,
    b: &Path,
    ja: &serde_json::Value,
    jb: &serde_json::Value,
) -> Result<Option<String>, Failure> {
    let frames = |j: &serde_json::Value| -> Vec<serde_json::Value> {
        j["sequence"].as_array().cloned().unwrap_or_default()
    };
    let (sa, sb) = (frames(ja), frames(jb));
    let has = |s: &[serde_json::Value]| s.iter().any(|f| !f["table"].is_null());
    match (has(&sa), has(&sb)) {
        (false, false) => return Ok(None),
        (true, true) => {}
        _ => return Err("only one run has cache tables".into()),
    }
    let read = |dir: &Path, pose: usize| -> Option<Vec<u32>> {
        let bytes = fs::read(dir.join(format!("table-{pose:03}.u32"))).ok()?;
        Some(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| u32::from_le_bytes(*w))
                .collect(),
        )
    };
    // The first differing word, the first frame that fails the rule, and whether some
    // differing frame could not be judged for want of its raw tables.
    let (mut first, mut failed, mut unjudged) = (None, None, false);
    for (pose, (fa, fb)) in sa.iter().zip(&sb).enumerate() {
        let (ta, tb) = (&fa["table"], &fb["table"]);
        if ta["slots"] != tb["slots"] || ta["words"] != tb["words"] {
            return Err(format!(
                "pose {pose}: tables differ in shape: {} slots in {} words against {} in {}",
                ta["slots"], ta["words"], tb["slots"], tb["words"]
            )
            .into());
        }
        if pose == 0 {
            println!(
                "cache tables: {} slots, {} cold words a slot, {} words",
                ta["slots"], ta["cold_words"], ta["words"]
            );
        }
        let counts = format!(
            "stamp {} / {}, occupied {} / {}, valid {} / {}",
            ta["stamp"], tb["stamp"], ta["occupied"], tb["occupied"], ta["valid"], tb["valid"]
        );
        let (Some(wa), Some(wb)) = (read(a, pose), read(b, pose)) else {
            // Digests only: the first differing chunk bounds the first differing word.
            match (0..TABLE_CHUNKS).find(|i| ta["chunks"][i] != tb["chunks"][i]) {
                Some(i) => {
                    let range = chunk(ta["words"].as_u64().unwrap_or(0) as usize, i);
                    println!(
                        "pose {pose}: {counts}; raw tables not both present; first differing word is in {}..{}",
                        range.start, range.end
                    );
                    first
                        .get_or_insert((pose, format!("a word in {}..{}", range.start, range.end)));
                    unjudged = true;
                }
                None => println!("pose {pose}: {counts}; digests equal: pass"),
            }
            continue;
        };
        let (xa, xb) = (Table::new(&wa)?, Table::new(&wb)?);
        let differing: Vec<usize> = (0..wa.len()).filter(|i| wa[*i] != wb[*i]).collect();
        let Some(&word) = differing.first() else {
            println!("pose {pose}: {counts}; tables identical: pass");
            continue;
        };
        first.get_or_insert((pose, format!("word {word}: {}", xa.field(word))));
        let mut reasons = Vec::new();
        // 1. The plan.
        let header = (0..64).all(|i| {
            wa[i] == wb[i]
                || (HEADER_FLOATS.contains(&i)
                    && (wa[i] ^ wb[i]) >> 31 == 0
                    && wa[i].abs_diff(wb[i]) <= HEADER_ULPS)
        });
        if !header {
            reasons.push("the headers differ".to_owned());
        }
        // 2. The cells, by key: the same cell may sit in another slot of its bucket.
        let (ca, cb) = (xa.cells(), xb.cells());
        let valid = |table: &Table, slot: usize| table.hot(slot)[0] & 0x8000_0000 != 0;
        let common: Vec<_> = ca
            .iter()
            .filter_map(|(key, slot)| Some((*slot, *cb.get(key)?)))
            .collect();
        let both: Vec<_> = common
            .iter()
            .copied()
            .filter(|(sa, sb)| valid(&xa, *sa) && valid(&xb, *sb))
            .collect();
        let validity = common
            .iter()
            .filter(|(sa, sb)| valid(&xa, *sa) != valid(&xb, *sb))
            .count();
        let one_sided = ca.len() + cb.len() - 2 * common.len() + validity;
        let cells = ca.len().max(cb.len()).max(1);
        let one_sided_share = one_sided as f64 / cells as f64;
        if one_sided_share > ONE_SIDED_CELLS {
            reasons.push(format!("{one_sided} cells are on one side only"));
        }
        // 3 and 4. Direction bits, then the SH where the bits agree.
        let (mut flipped, mut worst, mut compared) = (0u64, 0f32, 0usize);
        let (mut bounce_off, mut bounce_worst) = (0usize, 0f32);
        let (mut splits, mut split_off) = ([0usize; 2], 0usize);
        for &(slot_a, slot_b) in &both {
            let (cold_a, cold_b) = (xa.cold(slot_a), xb.cold(slot_b));
            let rays =
                (cold_a[12] ^ cold_b[12]).count_ones() + (cold_a[13] ^ cold_b[13]).count_ones();
            flipped += u64::from(rays);
            if rays != 0 {
                continue;
            }
            let (hot_a, hot_b) = (xa.hot(slot_a), xb.hot(slot_b));
            let (scale_a, scale_b) = (f32::from_bits(hot_a[1]), f32::from_bits(hot_b[1]));
            let halves = |hot: &'_ [u32], scale: f32| -> Vec<f32> {
                hot[2..16]
                    .iter()
                    .flat_map(|w| [*w as u16, (*w >> 16) as u16])
                    .take(27)
                    .map(|h| decode_f16(h) * scale)
                    .collect()
            };
            let difference = halves(hot_a, scale_a)
                .iter()
                .zip(halves(hot_b, scale_b))
                .map(|(x, y)| (x - y).abs())
                .fold(0f32, f32::max);
            worst = worst.max(difference / scale_a.max(scale_b).max(1e-30));
            compared += 1;
            // The bounce L1 SH: a scale of 0 or -0.0 is no value, which reads as zero.
            let bounce = |hot: &'_ [u32]| -> (f32, Vec<f32>) {
                let scale = f32::from_bits(hot[16]).max(0.);
                let values = hot[17..23]
                    .iter()
                    .flat_map(|w| [*w as u16, (*w >> 16) as u16])
                    .map(|h| decode_f16(h) * scale)
                    .collect();
                (scale, values)
            };
            let ((top_a, bounce_a), (top_b, bounce_b)) = (bounce(hot_a), bounce(hot_b));
            let apart = bounce_a
                .iter()
                .zip(bounce_b)
                .map(|(x, y)| (x - y).abs())
                .fold(0f32, f32::max)
                / top_a.max(top_b).max(1e-30);
            bounce_worst = bounce_worst.max(apart);
            bounce_off += usize::from(apart.is_nan() || apart > BOUNCE_BOUND);
            let split = [hot_a, hot_b].map(|hot| hot[15] & 0x1_0000 != 0);
            splits[0] += usize::from(split[0]);
            splits[1] += usize::from(split[1]);
            split_off += usize::from(split[0] != split[1]);
        }
        let flipped_share = flipped as f64 / (both.len().max(1) as f64 * 64.);
        if flipped_share > FLIPPED_RAYS {
            reasons.push(format!("{flipped} direction bits differ"));
        }
        let bounce_share = bounce_off as f64 / compared.max(1) as f64;
        if bounce_share > BOUNCE_CELLS {
            reasons.push(format!("{bounce_off} cells differ in bounce"));
        }
        let split_bound = SPLIT_FLOOR.max((splits[0].max(splits[1]) as f64 * SPLIT_CELLS) as usize);
        if split_off > split_bound {
            reasons.push(format!("{split_off} cells differ in their split flag"));
        }
        if worst.is_nan() || worst > SH_BOUND {
            reasons.push(format!(
                "an SH coefficient differs by {worst:.2e} of its scale"
            ));
        }
        println!(
            "pose {pose}: {counts}; {} words differ, first at {word} ({}); header {}; one-sided cells {one_sided} of {cells} ({:.3}%, bound {:.1}%); differing direction bits {flipped} over {} cells valid in both ({:.4}%, bound {:.2}%); worst SH coefficient {worst:.2e} of its scale over {compared} cells with equal bits (bound {SH_BOUND:.0e}); bounce further than {BOUNCE_BOUND:.0e} of scale apart in {bounce_off} of them ({:.3}%, bound {:.1}%; worst {bounce_worst:.2e}); split cells {} / {}, {split_off} differing (bound {split_bound}): {}",
            differing.len(),
            xa.field(word),
            if header { "same" } else { "DIFFERS" },
            one_sided_share * 100.,
            ONE_SIDED_CELLS * 100.,
            both.len(),
            flipped_share * 100.,
            FLIPPED_RAYS * 100.,
            bounce_share * 100.,
            BOUNCE_CELLS * 100.,
            splits[0],
            splits[1],
            if reasons.is_empty() { "pass" } else { "FAIL" }
        );
        if !reasons.is_empty() {
            failed.get_or_insert(format!("frame {pose}: {}", reasons.join("; ")));
        }
    }
    if let Some((pose, what)) = &first {
        println!("cache tables: first difference in frame {pose}, {what}");
    }
    let rule = format!(
        "same header, one-sided cells within {:.1}%, differing direction bits within {:.2}%, SH within {SH_BOUND:.0e} of scale where bits agree, bounce further than {BOUNCE_BOUND:.0e} of scale apart in at most {:.1}% of those cells, split flags differing on at most {:.0}% of split cells, in every frame",
        ONE_SIDED_CELLS * 100.,
        FLIPPED_RAYS * 100.,
        BOUNCE_CELLS * 100.,
        SPLIT_CELLS * 100.
    );
    Ok(match (failed, unjudged, first) {
        (Some(failure), ..) => {
            println!("cache tables: FAIL ({failure}). Rule: {rule}");
            Some(format!("cache tables fail the acceptance rule: {failure}"))
        }
        (None, true, _) => {
            println!(
                "cache tables: NO VERDICT. A frame differs and its raw tables are not both present. Rule: {rule}"
            );
            Some("cache tables differ and could not be judged without the raw tables".to_owned())
        }
        (None, false, Some(_)) => {
            println!("cache tables: PASS. Rule: {rule}");
            None
        }
        (None, false, None) => {
            println!("cache tables: PASS, identical in every frame");
            None
        }
    })
}
