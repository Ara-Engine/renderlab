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
//! reflections off, which is `HardwareRt` at `Quality::Reference`. Motion blur is off
//! at this level on both layers (the Ara frame does not chain its hit-based motion
//! surface yet); bloom, tone and the rest of post are on.
//!
//! `--frames N` renders a sequence: pose 0 to N-1 of the fixture's camera path (the
//! eye moves 0.04 m sideways per pose) through one renderer, so every frame after the
//! first has history: the previous camera for motion blur, and whatever later
//! families keep between frames. The same line on either layer writes one summary
//! per frame; `--compare` then reports every frame.
//!
//! ```text
//! cargo run --example ara_frame -- --backend wgpu --output out/wgpu
//! cargo run --features backend-ara --example ara_frame -- --backend ara --output out/ara
//! cargo run --example ara_frame -- --compare out/wgpu out/ara
//! ```
//!
//! Each run writes the exposed HDR target as `frame.rgba16f` (tightly packed rows) with
//! `frame.png` (Reinhard, sRGB; for looking at only), the displayed image after the
//! default post chain as `display.rgba8` and `display.png`, and `frame.json` (extent
//! and 24x16 grids of mean linear RGB and mean display bytes, small enough to paste
//! where the raw files cannot travel). `--compare` prefers the raw files and falls
//! back to the grids.
// The fixture also carries names and a scene record for the Unreal match export.
#[allow(dead_code)]
#[path = "../support/lighting_fixture.rs"]
mod lighting_fixture;

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
    let mut frames = 1usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => backend = args.next(),
            "--output" => output = args.next(),
            "--sun-shadows" => sun_shadows = true,
            "--traced" => (sun_shadows, traced) = (true, true),
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
    let mut fixture = lighting_fixture::Fixture::new();
    if sun_shadows {
        for object in &mut fixture.objects {
            object.double_sided = true;
        }
    }
    let level = Level {
        atmosphere,
        sun_shadows,
        traced,
        frames,
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
    frames: usize,
}

/// Post at the traced level: the defaults without motion blur, on both layers.
fn traced_post() -> renderlab::PostSettings {
    renderlab::PostSettings {
        motion_blur: false,
        ..Default::default()
    }
}

/// The exposed HDR target and the displayed image of one frame.
type Frame = (Vec<u8>, Vec<u8>);

fn wgpu_frame(
    fixture: &lighting_fixture::Fixture,
    size: [u32; 2],
    Level {
        atmosphere,
        sun_shadows,
        traced,
        frames,
    }: Level,
) -> Result<Vec<Frame>, Failure> {
    use renderlab::*;
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
        anisotropy_clamp: 16,
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
        r.set_soft_shadows(SoftShadows::Exact);
        r.set_indirect(IndirectSettings::off())?;
        r.set_reconstruction(Reconstruction::off())?;
        r.set_reflections(ReflectionSettings::off())?;
        r.set_post_settings(traced_post())?;
    }
    // The frame the ara backend assembles today: everything else is a later level.
    r.set_atmosphere(atmosphere)?;
    r.set_local_shadows(LocalShadows {
        count: 0,
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
    for o in &fixture.objects {
        meshes.push(admit(&mut r, |r| {
            r.try_upload_mesh(&MeshData {
                vertices: &o.vertices,
                indices: &o.indices,
            })
        })?);
        instances.push(admit(&mut r, |r| r.try_upload_instances(&[o.transform]))?);
        materials.push(admit(&mut r, |r| {
            r.try_create_material(&MaterialDescriptor {
                base_color: o.base,
                metallic: o.metal,
                roughness: o.rough,
                double_sided: o.double_sided,
                ..Default::default()
            })
        })?);
    }
    let mut sequence = Vec::new();
    for pose in 0..frames {
        let camera = fixture.camera(pose);
        {
            let mut draws: Vec<_> = instances
                .iter_mut()
                .enumerate()
                .map(|(i, inst)| Draw::new(&meshes[i], inst).with_material(&materials[i]))
                .collect();
            admit(&mut r, |r| {
                r.try_render_view(&color, &camera, &mut draws, ViewMode::Lit)
            })?;
        }
        let mut capture = admit(&mut r, |r| r.try_capture_hdr())?;
        let info = capture.info();
        if info.format != wgpu::TextureFormat::Rgba16Float || [info.width, info.height] != size {
            return Err("unexpected HDR capture".into());
        }
        let hdr = admit(&mut r, |_| capture.try_read())?;
        let mut shown = admit(&mut r, |r| r.try_capture(&color))?;
        sequence.push((hdr, admit(&mut r, |_| shown.try_read())?));
    }
    Ok(sequence)
}

#[cfg(feature = "backend-ara")]
fn ara_frame(
    fixture: &lighting_fixture::Fixture,
    size: [u32; 2],
    Level {
        atmosphere,
        sun_shadows,
        traced,
        frames,
    }: Level,
    output: &Path,
) -> Result<Vec<Frame>, Failure> {
    use renderlab::backend_ara::{cache::ShaderCache, forward};
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
    let mut history = forward::Frames::default();
    let mut sequence = Vec::new();
    for pose in 0..frames {
        let pixels = history.render(
            &mut gpu,
            &cache,
            &forward::ForwardScene {
                objects: &objects,
                lighting: &fixture.lighting,
                camera: fixture.camera(pose),
                size,
                atmosphere,
                sun_shadows: sun_shadows.then_some(&shadows),
                primary: if traced {
                    forward::Primary::Traced
                } else {
                    forward::Primary::Raster
                },
                post: Some(if traced {
                    traced_post()
                } else {
                    renderlab::PostSettings::default()
                }),
                indirect: None,
            },
        )?;
        sequence.push((pixels.hdr, pixels.display.ok_or("post did not run")?));
    }
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
    let mut summaries = Vec::new();
    let mut nonfinite = 0;
    for (pose, (bytes, display)) in sequence.iter().enumerate() {
        if bytes.len() != pixels * 8 || display.len() != pixels * 4 {
            return Err("frame is not tightly packed RGBA16F with an RGBA8 display".into());
        }
        let values = floats(bytes);
        let display_values: Vec<f32> = display.iter().map(|v| f32::from(*v)).collect();
        let bad = values.iter().filter(|v| !v.is_finite()).count();
        nonfinite += bad;
        summaries.push(serde_json::json!({
            "pose": pose,
            "nonfinite_channels": bad,
            "grid_mean_linear_rgb": grid(&values, size),
            "grid_mean_display_bytes": grid(&display_values, size),
        }));
    }
    // The raw files and the pictures are the last frame's: the one with most history.
    let (bytes, display) = sequence.last().ok_or("no frame was rendered")?;
    fs::create_dir_all(dir)?;
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
    let mut png = png::Encoder::new(fs::File::create(dir.join("frame.png"))?, size[0], size[1]);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(&rgb)?;
    let last = summaries.last().cloned().unwrap_or_default();
    fs::write(
        dir.join("frame.json"),
        // Compact: a sequence's grids are large, and this file travels as text.
        serde_json::to_vec(&serde_json::json!({
            "scene": "M5-A authored v1, camera path from pose 0, local shadows off, default post",
            "primary": if level.traced { "traced, exact soft shadows, no GI, reconstruction, reflections or motion blur" } else { "mesh pass" },
            "atmosphere": if level.atmosphere.active() { "physical sky without volumetric fog" } else { "off" },
            "sun_shadows": if level.sun_shadows { "ray-traced, 0.02 rad disk, all materials double-sided" } else { "off" },
            "backend": backend,
            "width": size[0],
            "height": size[1],
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
    {
        return Err("frames differ in atmosphere, sun shadow or primary setting".into());
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
        }
        println!("last frame:");
    }
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
        return Ok(());
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
    let width = ja["width"].as_u64().unwrap_or(1) as usize;
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
    Ok(())
}
