//! P-PIPE cache-persistence check (docs/p-pipe.md): several `Engine`s started at once in
//! one process, sharing one `EngineDescriptor::pipeline_cache` directory, each drawing a
//! small sun-lit scene to its first captured image.
//!
//!     p_pipe_engines rt|nonrt CACHE_DIR ENGINES [--sequential] [--forget] [--abort-after SECONDS]
//!
//! ENGINES is 1 or 2: each engine holds a full device and its compiled set. With
//! `--sequential` the second engine starts only after the first has shown its image;
//! otherwise both start at once. Pipelines compiling at once are bounded process-wide by
//! the engine (`RENDERLAB_PIPELINE_COMPILES`). `--forget` leaks the engines instead of dropping them, so nothing is saved on drop.
//! `--abort-after` aborts the process (no destructors) that many seconds after start, as a
//! killed player would. Prints one JSON line: seconds to first image and the image hash
//! per engine, then the cache files.
use renderlab::engine::*;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

/// A box of half-size `h` at `c`, with per-face normals.
fn cube(c: [f32; 3], h: [f32; 3], color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let (mut vertices, mut indices) = (vec![], vec![]);
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut normal = [0.0; 3];
            normal[axis] = sign;
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut position = c;
                position[axis] += sign * h[axis];
                position[u] += a * h[u];
                position[v] += b * h[v];
                vertices.push(Vertex {
                    position,
                    normal,
                    uv: [0.0; 2],
                    color,
                });
            }
            let quad = if sign > 0.0 {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            indices.extend(quad.map(|i| base + i));
        }
    }
    (vertices, indices)
}

/// Starts one engine, draws two frames and returns (seconds to first image, RGBA8 hash).
fn first_image(
    path: RenderPath,
    cache: &Path,
    start: Instant,
) -> Result<(f64, String, Engine), Box<dyn std::error::Error + Send + Sync>> {
    let adapter = headless_adapter()?;
    // SAFETY: the caller supplied the renderer's explicit experimental-feature acknowledgement.
    let experimental_ray_query =
        (path == RenderPath::HardwareRt).then(|| unsafe { ExperimentalRayQuery::acknowledge() });
    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: Some(cache.to_path_buf()),
            ..Default::default()
        },
    )?;
    engine.resize(640, 360)?;
    let material = engine.create_material(&MaterialDesc {
        base_color: [0.7, 0.6, 0.5, 1.0],
        ..Default::default()
    })?;
    for (c, h) in [
        ([0.0, -0.05, 0.0], [8.0, 0.05, 8.0]),
        ([0.0, 0.5, 0.0], [0.5; 3]),
    ] {
        let (vertices, indices) = cube(c, h, [1.0; 4]);
        let mesh = engine.create_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })?;
        engine.spawn(mesh, material, Mat4::IDENTITY)?;
    }
    engine.set_sun(Some(Sun {
        travel: Vec3::new(-0.4, -1.0, -0.3).normalize(),
        ..Sun::default()
    }))?;
    let camera = Camera::look_at(
        Vec3::new(3.0, 2.0, 4.0),
        Vec3::new(0.0, 0.4, 0.0),
        Vec3::Y,
        0.9,
        0.05,
        200.0,
    )?;
    engine.render(&camera)?;
    engine.render(&camera)?;
    let image = engine.capture()?;
    let seconds = start.elapsed().as_secs_f64();
    Ok((
        seconds,
        format!("{:x}", Sha256::digest(&image.rgba8)),
        engine,
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let usage =
        "p_pipe_engines rt|nonrt CACHE_DIR 1|2 [--sequential] [--forget] [--abort-after SECONDS]";
    let [path, cache, count, rest @ ..] = args.as_slice() else {
        return Err(usage.into());
    };
    let path = match path.as_str() {
        "rt" => RenderPath::HardwareRt,
        "nonrt" => RenderPath::NonRt,
        _ => return Err(usage.into()),
    };
    if path == RenderPath::HardwareRt
        && env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
            != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    {
        return Err("RT needs the explicit experimental-ray-query acknowledgement".into());
    }
    let cache = PathBuf::from(cache);
    let count: usize = count.parse()?;
    if !(1..=2).contains(&count) {
        return Err("at most two engines in one process".into());
    }
    let sequential = rest.iter().any(|a| a == "--sequential");
    let forget = rest.iter().any(|a| a == "--forget");
    if let Some(i) = rest.iter().position(|a| a == "--abort-after") {
        let seconds: f64 = rest.get(i + 1).ok_or(usage)?.parse()?;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(seconds));
            eprintln!("p_pipe_engines aborting after {seconds} s");
            std::process::abort();
        });
    }
    let start = Instant::now();
    let mut engines = vec![];
    let mut results = vec![];
    let mut record = |(seconds, hash, engine): (f64, String, Engine)| {
        results.push(serde_json::json!({"first_image_seconds": seconds, "rgba8_sha256": hash}));
        engines.push(engine);
    };
    if sequential {
        for _ in 0..count {
            record(first_image(path, &cache, start).map_err(|e| e.to_string())?);
        }
    } else {
        let barrier = Arc::new(Barrier::new(count));
        let threads: Vec<_> = (0..count)
            .map(|_| {
                let (barrier, cache) = (barrier.clone(), cache.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    first_image(path, &cache, start)
                })
            })
            .collect();
        for thread in threads {
            record(thread.join().unwrap().map_err(|e| e.to_string())?);
        }
    }
    if forget {
        std::mem::forget(engines);
    } else {
        drop(engines);
    }
    let mut files: Vec<_> = fs::read_dir(&cache)?
        .filter_map(Result::ok)
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            serde_json::json!({"name": name, "bytes": e.metadata().map(|m| m.len()).unwrap_or(0)})
        })
        .collect();
    files.sort_by_key(|f| f["name"].as_str().unwrap_or("").to_owned());
    println!(
        "{}",
        serde_json::json!({"path": format!("{path:?}"), "engines": results, "forget": forget,
            "sequential": sequential,
            "total_seconds": start.elapsed().as_secs_f64(), "cache_files": files})
    );
    Ok(())
}
