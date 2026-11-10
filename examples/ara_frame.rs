//! One real frame on either GPU layer, and the comparison between the two.
//!
//! Renders the authored M5-A lighting fixture (six spheres across roughness and
//! metalness, a floor, a mirrored panel and a double-sided panel under a directional,
//! a point and a spot light) through the forward lit path and writes the exposed HDR
//! target. `--backend wgpu` is the existing renderer; `--backend ara` is the optional
//! ara-gpu backend (`--features backend-ara`), which is the only one that runs on
//! Metal. Both turn the same features off: atmosphere, local shadows.
//!
//! ```text
//! cargo run --example ara_frame -- --backend wgpu --output out/wgpu
//! cargo run --features backend-ara --example ara_frame -- --backend ara --output out/ara
//! cargo run --example ara_frame -- --compare out/wgpu out/ara
//! ```
//!
//! Each run writes `frame.rgba16f` (tightly packed rows), `frame.png` (Reinhard, sRGB;
//! for looking at only) and `frame.json` (extent and a 24x16 grid of mean linear RGB,
//! small enough to paste where the raw file cannot travel). `--compare` prefers the
//! raw files and falls back to the grids.
#[path = "../support/lighting_fixture.rs"]
mod lighting_fixture;

use renderlab::decode_f16;
use std::{fs, path::Path};

type Failure = Box<dyn std::error::Error>;
const GRID: [usize; 2] = [24, 16];

fn main() -> Result<(), Failure> {
    let mut args = std::env::args().skip(1);
    let (mut backend, mut output, mut size) = (None, None, [960u32, 640]);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => backend = args.next(),
            "--output" => output = args.next(),
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
    let fixture = lighting_fixture::Fixture::new();
    let pixels = match backend.as_deref() {
        Some("wgpu") => wgpu_frame(&fixture, size)?,
        Some("ara") => ara_frame(&fixture, size, Path::new(&output))?,
        _ => return Err("--backend wgpu|ara".into()),
    };
    write(
        Path::new(&output),
        size,
        &pixels,
        backend.as_deref().unwrap(),
    )
}

fn wgpu_frame(fixture: &lighting_fixture::Fixture, size: [u32; 2]) -> Result<Vec<u8>, Failure> {
    use renderlab::*;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!("wgpu adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: RECEIVER_FEATURES,
        ..Default::default()
    }))?;
    let mut r = Renderer::new_with_options(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Limits::default(),
        RendererOptions {
            samples: SampleCount::One,
            anisotropy_clamp: 16,
        },
        include_str!("../shaders/scene.wgsl"),
    )?;
    // The frame the ara backend assembles today: everything else is a later level.
    r.set_atmosphere(Atmosphere::off())?;
    r.set_local_shadows(LocalShadows {
        count: 0,
        ..Default::default()
    })?;
    r.set_lighting(&fixture.lighting)?;
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
    let camera = fixture.camera(0);
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
    admit(&mut r, |_| capture.try_read())
}

#[cfg(feature = "backend-ara")]
fn ara_frame(
    fixture: &lighting_fixture::Fixture,
    size: [u32; 2],
    output: &Path,
) -> Result<Vec<u8>, Failure> {
    use renderlab::backend_ara::{cache::ShaderCache, forward};
    let mut gpu = ara_gpu::GpuDevice::new_raster(&[], output.join("device"))?;
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
    let pixels = forward::render(
        &mut gpu,
        &cache,
        &forward::ForwardScene {
            objects: &objects,
            lighting: &fixture.lighting,
            camera: fixture.camera(0),
            size,
        },
    )?;
    gpu.assert_clean()?;
    Ok(pixels)
}
#[cfg(not(feature = "backend-ara"))]
fn ara_frame(_: &lighting_fixture::Fixture, _: [u32; 2], _: &Path) -> Result<Vec<u8>, Failure> {
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

fn write(dir: &Path, size: [u32; 2], bytes: &[u8], backend: &str) -> Result<(), Failure> {
    if bytes.len() != size[0] as usize * size[1] as usize * 8 {
        return Err("frame is not tightly packed RGBA16F".into());
    }
    fs::create_dir_all(dir)?;
    fs::write(dir.join("frame.rgba16f"), bytes)?;
    let values = floats(bytes);
    let rgb: Vec<u8> = values
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
    let nonfinite = values.iter().filter(|v| !v.is_finite()).count();
    fs::write(
        dir.join("frame.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scene": "M5-A authored v1, pose 0, forward lit, atmosphere and local shadows off",
            "backend": backend,
            "width": size[0],
            "height": size[1],
            "nonfinite_channels": nonfinite,
            "grid": GRID,
            "grid_mean_linear_rgb": grid(&values, size),
        }))?,
    )?;
    eprintln!(
        "{backend} frame {}x{} written to {}; nonfinite channels {nonfinite}",
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
    println!(
        "{} ({}) against {} ({}), {}x{}",
        a.display(),
        ja["backend"],
        b.display(),
        jb["backend"],
        ja["width"],
        ja["height"]
    );
    let cells = |j: &serde_json::Value| -> Result<Vec<[f64; 3]>, Failure> {
        Ok(serde_json::from_value(j["grid_mean_linear_rgb"].clone())?)
    };
    let (ga, gb) = (cells(&ja)?, cells(&jb)?);
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
        "grid: worst cell mean differs by {:.4}% at column {}, row {}",
        worst * 100.,
        worst_cell % GRID[0],
        worst_cell / GRID[0]
    );
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
