//! TP1 checks (docs/m7-tp1.md): CPU validation of every transparent-pass module, and the
//! ignored GPU test scene: the registered quarry plus cutout foliage, a rippled glass pane
//! and fire and smoke emitters, one path per process.
use super::*;
use std::path::PathBuf;

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../examples/m4/cameras.rs"]
mod quarry_cameras;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../support/environment_controls.rs"]
mod quarry_environment;
#[allow(clippy::duplicate_mod)]
#[path = "../support/limits.rs"]
mod quarry_policy;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../viewer/src/prepared.rs"]
mod quarry_prepared;

const SOURCE: &str = include_str!("../shaders/scene.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn validate(label: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
}

#[test]
fn tp1_modules_validate() {
    let toon = include_str!("../shaders/programs/toon.wgsl");
    let hardware = crate::hardware_shadows::shader_source(SOURCE, SampleCount::One).unwrap();
    for (name, program) in [("standard", crate::STANDARD_PROGRAM), ("toon", toon)] {
        let parameters = crate::program::declares_parameters(program);
        validate(
            &format!("{name} map"),
            &glass_source(SOURCE, false, program, parameters),
        );
        validate(
            &format!("{name} rt"),
            &glass_source(&hardware, true, program, parameters),
        );
    }
    for source in particle_sources(SOURCE) {
        validate("particles", &source);
    }
    validate("hit depth", &hit_depth_source());
    validate(
        "raster depth",
        &format!("{}{}", section("TP_FRAME"), section("TP_DEPTH_RASTER")),
    );
    validate(
        "shadow",
        &format!("{}{}", section("TP_FRAME"), section("TP_SHADOW")),
    );
    // The standard program declares the hook; toon blends by coverage.
    assert!(crate::program::declares_transmittance(
        crate::STANDARD_PROGRAM
    ));
    assert!(!crate::program::declares_transmittance(toon));
}

#[test]
fn transparency_word_packs_opacity_and_ior() {
    let t = crate::Transparency {
        opacity: 0.25,
        ior: 1.5,
        thickness: 0.01,
    };
    let word = crate::Transparency::word(Some(&t));
    assert_eq!(word & 0xffff, 0x3400);
    assert_eq!(word >> 16, 0x3e00);
    assert_eq!(crate::Transparency::word(None), 0);
    // Clear glass (opacity 0) still marks the material transparent through its IOR.
    let clear = crate::Transparency { opacity: 0.0, ..t };
    assert_ne!(crate::Transparency::word(Some(&clear)), 0);
}

#[test]
fn shadow_fit_maps_receivers_into_the_map() {
    let receivers = [(Vec3::new(-1.0, 2.0, 0.0), Vec3::new(1.0, 3.0, 2.0))];
    let travel = Vec3::new(0.3, 0.2, -1.0).normalize();
    let fit = shadow_fit(travel, &receivers, &[]).unwrap();
    for i in 0..8 {
        let (min, max) = receivers[0];
        let p = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let q = fit.matrix * p.extend(1.0);
        assert!(
            q.x.abs() < 1.0 && q.y.abs() < 1.0 && (0.0..1.0).contains(&q.z),
            "{q}"
        );
    }
    // Farther along the sun's travel is deeper in the map.
    let near = fit.matrix * Vec3::new(0.0, 2.5, 1.0).extend(1.0);
    let far = fit.matrix * (Vec3::new(0.0, 2.5, 1.0) + travel * 0.5).extend(1.0);
    assert!(far.z > near.z);
}

/// The standard program as it was before TP1's marked edits, rebuilt from the current one.
fn pre_tp1_standard() -> String {
    let mut s = crate::STANDARD_PROGRAM.to_owned();
    for (from, to) in [
        ("// alpha: TP1 coverage of transparent materials (docs/m7-tp1.md), exactly 1 for opaque ones.
struct Surface { n:vec3<f32>, base:vec3<f32>, metal:f32, rough:f32, alpha:f32 };",
         "struct Surface { n:vec3<f32>, base:vec3<f32>, metal:f32, rough:f32 };"),
        ("    let alpha=transparent_coverage(material,material.base_color.a*sampled.a*input.color.a);
    return Surface(n,decal_base.rgb,decal_mr.y,decal_mr.x,alpha);",
         "    return Surface(n,decal_base.rgb,decal_mr.y,decal_mr.x);"),
        ("    // TP1: a transparent surface reflects diffusely over its coverage only.
    let base=s.base*s.alpha;",
         "    let base=s.base;"),
    ] {
        assert_eq!(s.matches(from).count(), 1, "{from}");
        s = s.replace(from, to);
    }
    let start = s
        .find(
            "
// TP1: the share of the light behind",
        )
        .unwrap();
    let end = start
        + s[start..]
            .find(
                "
}
",
            )
            .unwrap()
        + 3;
    s.replace_range(start..end, "");
    s
}

#[test]
fn pre_tp1_standard_is_exact() {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(pre_tp1_standard().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    // shaders/programs/standard.wgsl at main f159fcd (DC1, R1, G2b, AT1; before TP1).
    assert_eq!(
        hex,
        "c30842af78c4fa216e494182e63673f4aedf083c00ce2405c552f0836c5f43c7"
    );
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn device(rt: bool) -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let timing = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
    if rt {
        assert_eq!(
            std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
            Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"),
            "GPU tests require explicit caller acknowledgment"
        );
        let limits = crate::traced_limits(wgpu::Limits {
            max_blas_primitive_count: a.max_blas_primitive_count,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
            max_acceleration_structures_per_shader_stage: 1,
            max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
            max_buffer_size: a.max_buffer_size,
            ..Default::default()
        });
        let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | wgpu::Features::EXPERIMENTAL_RAY_QUERY
                | crate::TRACED_FEATURES
                | timing,
            required_limits: limits,
            experimental_features,
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        }))
        .unwrap()
    } else {
        let mut limits = crate::visibility_limits(wgpu::Limits::default());
        limits.max_buffer_size = a.max_buffer_size;
        limits.max_storage_buffer_binding_size = a.max_storage_buffer_binding_size;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | crate::VISIBILITY_FEATURES
                | crate::VISIBILITY_RASTER_FEATURES
                | crate::VIRTUAL_FEATURES
                | timing,
            required_limits: limits,
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        }))
        .unwrap()
    }
}

fn read_texture(r: &Renderer, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
    let size = texture.size();
    let row = (size.width * texel).next_multiple_of(256);
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("TP1 readback"),
        size: u64::from(row * size.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    r.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    let data = slice.get_mapped_range().unwrap();
    let width = (size.width * texel) as usize;
    data.chunks_exact(row as usize)
        .flat_map(|line| line[..width].to_vec())
        .collect()
}

fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut opaque = rgba.to_vec();
    for p in opaque.chunks_exact_mut(4) {
        p[3] = 255;
    }
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&opaque)
        .unwrap();
}

/// A full mip chain by 2x2 box filtering (odd edges clamp).
fn mips(width: u32, height: u32, rgba: Vec<u8>) -> Vec<crate::asset::Mip> {
    let mut levels = vec![crate::asset::Mip {
        width,
        height,
        rgba8: rgba,
    }];
    while let Some(last) = levels.last().filter(|m| m.width > 1 || m.height > 1) {
        let (w, h) = ((last.width / 2).max(1), (last.height / 2).max(1));
        let mut next = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (2 * x + dx).min(last.width - 1);
                        let sy = (2 * y + dy).min(last.height - 1);
                        sum += u32::from(last.rgba8[((sy * last.width + sx) * 4 + c) as usize]);
                    }
                    next[((y * w + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        levels.push(crate::asset::Mip {
            width: w,
            height: h,
            rgba8: next,
        });
    }
    levels
}

/// A procedural fern-like leaf cluster: a stem with paired elliptical leaflets that
/// shrink toward the tip. Green albedo with small variation; alpha 255 inside.
fn leaf_texture(size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let v = 1.0 - (y as f32 + 0.5) / size as f32;
            let mut inside = (u - 0.5).abs() < 0.012 && v < 0.97;
            for k in 0..9 {
                let t = 0.08 + k as f32 * 0.1;
                let span = 0.42 * (1.0 - t * 0.85);
                for side in [-1.0f32, 1.0] {
                    let cx = 0.5 + side * span * 0.55;
                    let cy = t + span * 0.25;
                    let (dx, dy) = (u - cx, v - cy);
                    // Leaflet tilted 30 degrees up from horizontal.
                    let (c, s) = (0.866f32, 0.5 * side);
                    let (a, b) = (dx * c + dy * s, -dx * s + dy * c);
                    if (a / (span * 0.55)).powi(2) + (b / (span * 0.16)).powi(2) < 1.0 {
                        inside = true;
                    }
                }
            }
            let shade = 0.75 + 0.25 * ((u * 37.0).sin() * (v * 23.0).cos()).abs();
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[
                (60.0 * shade) as u8,
                (120.0 * shade) as u8,
                (35.0 * shade) as u8,
                if inside { 255 } else { 0 },
            ]);
        }
    }
    rgba
}

/// A soft round smoke puff sprite: white rgb, radial alpha with a little noise.
fn puff_texture(size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let p = [x, y].map(|c| (c as f32 + 0.5) / size as f32 * 2.0 - 1.0);
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
            let noise = 0.85 + 0.15 * ((p[0] * 9.0).sin() * (p[1] * 11.0).cos());
            let a = ((1.0 - r).clamp(0.0, 1.0).powf(1.5) * noise * 255.0) as u8;
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
        }
    }
    rgba
}

fn vertex(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> crate::Vertex {
    crate::Vertex {
        position,
        normal,
        uv,
        color: [1.0; 4],
    }
}

/// A unit card in local x (width) and z (height), facing -y.
fn card() -> (Vec<crate::Vertex>, Vec<u32>) {
    let n = [0.0, -1.0, 0.0];
    (
        vec![
            vertex([-0.5, 0.0, 0.0], n, [0.0, 1.0]),
            vertex([0.5, 0.0, 0.0], n, [1.0, 1.0]),
            vertex([0.5, 0.0, 1.0], n, [1.0, 0.0]),
            vertex([-0.5, 0.0, 1.0], n, [0.0, 0.0]),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// A flat pane in local x and z, centred on x, rising from z = 0, facing -y, with rippled
/// vertex normals (the glass's shading normal; the surface stays planar).
fn pane(width: f32, height: f32, cells: u32) -> (Vec<crate::Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    for j in 0..=cells {
        for i in 0..=cells {
            let (u, v) = (i as f32 / cells as f32, j as f32 / cells as f32);
            let (x, z) = ((u - 0.5) * width, v * height);
            // Window glass: a gentle ripple of about 1.5 degrees.
            let nx = 0.025 * (x * 9.0 + z * 2.0).sin();
            let nz = 0.025 * (z * 7.0).cos();
            let length = (nx * nx + 1.0 + nz * nz).sqrt();
            vertices.push(vertex(
                [x, 0.0, z],
                [nx / length, -1.0 / length, nz / length],
                [u, 1.0 - v],
            ));
        }
    }
    let mut indices = vec![];
    let row = cells + 1;
    for j in 0..cells {
        for i in 0..cells {
            let a = j * row + i;
            indices.extend([a, a + 1, a + row + 1, a, a + row + 1, a + row]);
        }
    }
    (vertices, indices)
}

/// World placement: local z along `up`, local -y toward `facing` (projected flat), at `at`.
fn placement(at: Vec3, up: Vec3, facing: Vec3, yaw: f32, scale: f32) -> Mat4 {
    let flat = (facing - up * facing.dot(up)).normalize();
    let toward = Mat4::from_axis_angle(up, yaw).transform_vector3(flat);
    let y = -toward;
    let x = y.cross(up).normalize();
    Mat4::from_cols(
        (x * scale).extend(0.0),
        (y * scale).extend(0.0),
        (up * scale).extend(0.0),
        at.extend(1.0),
    )
}

fn upload_texture(
    r: &mut Renderer,
    role: crate::asset::TextureRole,
    size: u32,
    rgba: Vec<u8>,
) -> crate::Texture {
    let data = crate::asset::Texture {
        role,
        mips: mips(size, size, rgba),
    };
    loop {
        if let Some(t) = r.try_upload_texture(&data).unwrap() {
            return t;
        }
        r.test_drain("TP1 texture");
    }
}
fn upload_mesh(r: &mut Renderer, vertices: &[crate::Vertex], indices: &[u32]) -> Mesh {
    loop {
        if let Some(m) = r.try_upload_mesh(&MeshData { vertices, indices }).unwrap() {
            return m;
        }
        r.test_drain("TP1 mesh");
    }
}
fn upload_instances(r: &mut Renderer, models: &[Mat4]) -> Instances {
    loop {
        if let Some(i) = r.try_upload_instances(models).unwrap() {
            return i;
        }
        r.test_drain("TP1 instances");
    }
}
fn create_material(r: &mut Renderer, d: &MaterialDescriptor<'_>) -> Material {
    loop {
        if let Some(m) = r.try_create_material(d).unwrap() {
            return m;
        }
        r.test_drain("TP1 material");
    }
}

#[test]
#[ignore = "GPU; optimized build; RENDERLAB_TP1_QUARRY (G2's manifest), RENDERLAB_TP1_OUT, RENDERLAB_TP1_PATH rt|nonrt|rtraster"]
fn gpu_tp1_scene() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("RENDERLAB_TP1_QUARRY").unwrap()).unwrap())
            .unwrap();
    let out = PathBuf::from(env("RENDERLAB_TP1_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let path = env("RENDERLAB_TP1_PATH").unwrap_or_else(|| "rt".into());
    let sections = env("RENDERLAB_TP1_SECTIONS").unwrap_or_else(|| "images,timing".into());
    let path_of = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let (policy, _) = quarry_policy::Policy::args(vec![
        "--limits".into(),
        path_of("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let limits = policy.prepared();
    let source = crate::asset::decode(&std::fs::read(path_of("asset")).unwrap(), limits).unwrap();
    let cameras = quarry_cameras::Cameras::load(&path_of("cameras")).unwrap();
    let renderer_limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let rt = path != "nonrt";
    let (device, queue) = device(rt);
    let mut r = if rt {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            renderer_limits,
            Default::default(),
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap()
    } else {
        Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            renderer_limits,
            Default::default(),
            SOURCE,
        )
        .unwrap()
    };
    if path == "rt" {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    let lighting: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path_of("lighting")).unwrap()).unwrap();
    let v3 = |v: &serde_json::Value| -> [f32; 3] {
        std::array::from_fn(|k| v[k].as_f64().unwrap() as f32)
    };
    let exposure: f32 = env("RENDERLAB_TP1_EXPOSURE").map_or(-14.0, |s| s.parse().unwrap());
    let travel = Vec3::from_array(v3(&lighting["lights"][0]["travel"]));
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel,
                lux: v3(&lighting["lights"][0]["lux"]),
            }],
            environment: [0.0; 3],
            exposure_stops: exposure,
        },
        &Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![4., 12., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    if path == "rt" {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    if env("RENDERLAB_TP1_TR1").as_deref() == Some("1") && path != "rtraster" {
        r.set_reconstruction(Reconstruction::identity()).unwrap();
    }
    let map = quarry_environment::load_environment(
        &path_of("environment"),
        crate::asset::environment::Limits {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 128 * 1024 * 1024,
        },
    )
    .unwrap();
    let sky = loop {
        if let Some(sky) = r.try_upload_environment(&map).unwrap() {
            break sky;
        }
        r.test_drain("TP1 environment");
    };
    r.set_environment(Some(&sky), Default::default()).unwrap();
    drop(map);
    let mut asset = quarry_prepared::Prepared::new_with_limits(source, limits, 1_000_000).unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("TP1 upload");
    let pose = &cameras.frames[env("RENDERLAB_TP1_POSE").map_or(0, |s| s.parse().unwrap())];
    let camera = pose.camera().unwrap();
    let up = Vec3::from_array(pose.up).normalize();
    let (width, height) = (pose.width, pose.height);
    let target = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("TP1 color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut log = serde_json::json!({"path": path, "primary": format!("{:?}", r.primary_mode),
        "indirect": r.indirect().enabled,
        "soft": format!("{:?}", r.soft_mode), "exposure_stops": exposure, "size": [width, height],
        "tr1": r.reconstruction.enabled});

    // 1. Pick ground points from the opaque depth the pass writes (a probe emitter runs it).
    let probe = r
        .try_create_emitter(&EmitterDescriptor {
            capacity: 1,
            spawn_rate: 0.0,
            ..Default::default()
        })
        .unwrap();
    {
        let mut draws = asset.draws(Visibility::Frustum);
        while r
            .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            r.test_drain("TP1 probe");
        }
    }
    r.test_drain("TP1 probe frame");
    drop(probe);
    let depth: Vec<f32> = {
        let texture = &r.transparency.targets.as_ref().unwrap().depth.texture;
        read_texture(&r, texture, 4)
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    };
    // The opaque depth itself, as an image (near white, far black) and statistics.
    let finite: Vec<f32> = depth
        .iter()
        .copied()
        .filter(|d| d.is_finite() && *d < 1e20)
        .collect();
    let far = finite.iter().copied().fold(0.0f32, f32::max).max(1e-3);
    let image: Vec<u8> = depth
        .iter()
        .flat_map(|&d| {
            let v = if d.is_finite() && d < 1e20 {
                (255.0 * (1.0 - d / far).clamp(0.0, 1.0)) as u8
            } else if d.is_nan() {
                0
            } else {
                32
            };
            [v, v, if d.is_nan() { 255 } else { v }, 255]
        })
        .collect();
    write_png(
        &out.join(format!("{path}-depth.png")),
        width,
        height,
        &image,
    );
    log["depth"] = serde_json::json!({"covered": finite.len(), "pixels": depth.len(),
        "nan": depth.iter().filter(|d| d.is_nan()).count(), "max": far,
        "min": finite.iter().copied().fold(f32::INFINITY, f32::min)});
    eprintln!("tp1 depth {}", log["depth"]);
    let inverse_view = camera.view().inverse();
    let eye = inverse_view.w_axis.truncate();
    let forward = -inverse_view.z_axis.truncate().normalize();
    let unproject = camera.matrix(width, height).inverse();
    let pick = |fx: f32, fy: f32| -> Option<Vec3> {
        let (x, y) = ((fx * width as f32) as u32, (fy * height as f32) as u32);
        let d = depth[(y * width + x) as usize];
        let ndc = [
            (x as f32 + 0.5) / width as f32 * 2.0 - 1.0,
            1.0 - (y as f32 + 0.5) / height as f32 * 2.0,
        ];
        let far = unproject * glam::Vec4::new(ndc[0], ndc[1], 0.0, 1.0);
        let dir = (far.truncate() / far.w - eye).normalize();
        // Only island hits: the backdrop is empty.
        (d < 1e20).then(|| eye + dir * (d / dir.dot(forward)))
    };
    // Pose 0 sees the quarry island over a black backdrop: bushes on its gravel and sand,
    // the fire on the sand in front of the cliff.
    let picks: Vec<(f32, f32)> = vec![
        (0.33, 0.56),
        (0.36, 0.58),
        (0.45, 0.66),
        (0.52, 0.69),
        (0.62, 0.66),
        (0.80, 0.60),
    ];
    let bushes: Vec<Vec3> = picks.iter().filter_map(|&(x, y)| pick(x, y)).collect();
    let fire_at = pick(0.56, 0.67).unwrap();
    let centre = pick(0.5, 0.55).map_or(8.0, |p| (p - eye).dot(forward));
    // The pane stands halfway to the floor in front of the gravel, rising 1.6 m.
    let floor = pick(0.42, 0.66).unwrap();
    let glass_at = eye + (floor - eye) * 0.5 - up * 0.5;
    log["picks"] = serde_json::json!({"bushes": bushes.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
        "fire": fire_at.to_array(), "glass": glass_at.to_array(), "centre_depth": centre});

    // 2. Foliage: one fern card cut to its alpha contour, three crossed cards per bush.
    let leaf_rgba = leaf_texture(256);
    let mask = crate::cutout::AlphaMask::from_rgba8(256, 256, &leaf_rgba);
    let (card_vertices, card_indices) = card();
    let cells: u32 = env("RENDERLAB_TP1_CELLS").map_or(96, |s| s.parse().unwrap());
    let cut_start = Instant::now();
    let cut = crate::cutout::cutout(
        &MeshData {
            vertices: &card_vertices,
            indices: &card_indices,
        },
        &mask,
        1.0,
        0.5,
        cells,
    )
    .unwrap();
    let cut_ms = cut_start.elapsed().as_secs_f64() * 1e3;
    let leaf_texture = upload_texture(
        &mut r,
        crate::asset::TextureRole::BaseColorSrgb,
        256,
        leaf_rgba,
    );
    let leaf_mesh = upload_mesh(&mut r, &cut.vertices, &cut.indices);
    let leaf = create_material(
        &mut r,
        &MaterialDescriptor {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.65,
            base_color_texture: Some(TextureBinding {
                texture: &leaf_texture,
                sampler: Default::default(),
            }),
            double_sided: true,
            thin: true,
            ..Default::default()
        },
    );
    let mut models = vec![];
    for (k, &at) in bushes.iter().enumerate() {
        let scale = 0.9 + 0.25 * (k % 3) as f32;
        for c in 0..3 {
            models.push(placement(
                at - up * 0.05,
                up,
                eye - at,
                k as f32 * 0.7 + c as f32 * 2.094,
                scale,
            ));
        }
    }
    assert!(
        models.iter().all(|m| m.is_finite()),
        "TP1 foliage placements {:?}",
        log["picks"]
    );
    let mut foliage = upload_instances(&mut r, &models);
    log["foliage"] = serde_json::json!({"cards": models.len(), "cut_triangles": cut.indices.len() / 3,
        "cut_vertices": cut.vertices.len(), "cut_ms": cut_ms, "cells": cells});

    // 3. Glass: a rippled clear pane facing the camera.
    let (pane_vertices, pane_indices) = pane(2.2, 1.6, 48);
    let pane_mesh = upload_mesh(&mut r, &pane_vertices, &pane_indices);
    let glass = create_material(
        &mut r,
        &MaterialDescriptor {
            base_color: [0.86, 0.95, 0.92, 1.0],
            metallic: 0.0,
            roughness: 0.04,
            double_sided: true,
            thin: true,
            transparency: Some(crate::Transparency {
                opacity: 0.0,
                ior: 1.5,
                thickness: 0.02,
            }),
            ..Default::default()
        },
    );
    let mut glass_instances = upload_instances(
        &mut r,
        &[placement(glass_at, up, eye - glass_at, 0.25, 1.0)],
    );

    // 4. Fire and smoke.
    let puff = upload_texture(
        &mut r,
        crate::asset::TextureRole::BaseColorSrgb,
        64,
        puff_texture(64),
    );
    let lux = v3(&lighting["lights"][0]["lux"]);
    let sun = lux[0].max(lux[1]).max(lux[2]);
    let fire = EmitterDescriptor {
        position: fire_at + up * 0.1,
        spawn_radius: 0.12,
        spawn_rate: 220.0,
        lifetime: [0.3, 0.65],
        velocity: up * 1.4,
        velocity_spread: 0.3,
        acceleration: up * 2.0,
        drag: 0.8,
        size: [0.28, 0.06],
        color: [[1.0, 0.5, 0.15, 0.25], [0.6, 0.2, 0.05, 0.0]],
        // A flame several times brighter than the sunlit ground.
        emission: [
            [sun * 0.9, sun * 0.42, sun * 0.1],
            [sun * 0.25, sun * 0.06, 0.0],
        ],
        fade: [0.15, 0.5],
        soft_distance: 0.15,
        texture: Some(&puff),
        capacity: 192,
        seed: 7,
    };
    let smoke = EmitterDescriptor {
        position: fire_at + up * 0.6,
        spawn_radius: 0.15,
        spawn_rate: 40.0,
        lifetime: [3.0, 4.5],
        velocity: up * 0.6,
        velocity_spread: 0.18,
        acceleration: up * 0.05 + inverse_view.x_axis.truncate() * 0.12,
        drag: 0.25,
        size: [0.35, 1.6],
        color: [[0.42, 0.42, 0.44, 0.55], [0.55, 0.55, 0.58, 0.0]],
        emission: [[0.0; 3]; 2],
        fade: [0.12, 0.55],
        soft_distance: 0.4,
        texture: Some(&puff),
        capacity: 256,
        seed: 11,
    };

    let render = |r: &mut Renderer,
                  camera: &Camera,
                  asset: &mut quarry_prepared::Prepared,
                  foliage: Option<&mut Instances>,
                  glass_draw: Option<&mut Instances>| {
        let mut draws = asset.draws(Visibility::Frustum);
        if let Some(instances) = foliage {
            draws.push(Draw::new(&leaf_mesh, instances).with_material(&leaf));
        }
        if let Some(instances) = glass_draw {
            draws.push(Draw::new(&pane_mesh, instances).with_material(&glass));
        }
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(report) = r
                .try_render_view(&target, camera, &mut draws, ViewMode::Lit)
                .unwrap()
            {
                return report;
            }
            assert!(Instant::now() < until, "TP1 admission");
            r.test_drain("TP1 admission");
        }
    };
    let save = |r: &mut Renderer, name: &str| {
        r.test_drain("TP1 capture");
        let rgba = read_texture(r, &target, 4);
        write_png(
            &out.join(format!("{path}-{name}.png")),
            width,
            height,
            &rgba,
        );
    };
    let samples = |r: &mut Renderer, rows: &mut Vec<f64>| {
        let events = r.poll().unwrap();
        rows.extend(events.gpu.iter().map(|g| g.render_span.as_secs_f64() * 1e3));
    };
    let median = |rows: &mut Vec<f64>| {
        rows.sort_by(f64::total_cmp);
        rows.get(rows.len() / 2).copied()
    };
    if sections.contains("images") {
        for _ in 0..3 {
            render(&mut r, &camera, &mut asset, None, None);
        }
        save(&mut r, "opaque");
        // The opaque image with the pre-TP1 standard program: TP1 leaves it unchanged.
        // TR1 and the radiance cache off for this pair, so both images are single frames of
        // the same shading (a program reload relights the cache over several frames).
        let reconstruction = r.reconstruction;
        let indirect = r.indirect();
        r.set_reconstruction(Reconstruction::off()).unwrap();
        r.set_indirect(IndirectSettings::off()).unwrap();
        for _ in 0..3 {
            render(&mut r, &camera, &mut asset, None, None);
        }
        r.test_drain("TP1 identity current");
        let current = read_texture(&r, &target, 4);
        let standard = r.standard_program().clone();
        r.reload_program(&standard, &pre_tp1_standard()).unwrap();
        for _ in 0..3 {
            render(&mut r, &camera, &mut asset, None, None);
        }
        r.test_drain("TP1 identity");
        let before = read_texture(&r, &target, 4);
        r.reload_program(&standard, crate::STANDARD_PROGRAM)
            .unwrap();
        r.set_reconstruction(reconstruction).unwrap();
        r.set_indirect(indirect).unwrap();
        let differing = current
            .chunks_exact(4)
            .zip(before.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        log["identity"] = serde_json::json!({"differing_pixels": differing,
            "note": "opaque quarry, TP1 standard program vs the pre-TP1 one, presented RGBA8"});
        eprintln!("tp1 identity {}", log["identity"]);
        for _ in 0..3 {
            render(&mut r, &camera, &mut asset, Some(&mut foliage), None);
        }
        save(&mut r, "foliage");
    }
    let fire_emitter = r.try_create_emitter(&fire).unwrap();
    let smoke_emitter = r.try_create_emitter(&smoke).unwrap();
    if sections.contains("images") {
        for frame in 1..=240 {
            render(
                &mut r,
                &camera,
                &mut asset,
                Some(&mut foliage),
                Some(&mut glass_instances),
            );
            if frame == 60 || frame == 240 {
                save(&mut r, &format!("full-f{frame}"));
            }
        }
        // Motion: the camera slides 1.5 cm a frame sideways for 40 frames.
        let right = inverse_view.x_axis.truncate();
        for k in 1..=40 {
            let shift = right * (0.015 * k as f32);
            let moved = Camera::look_at(
                eye + shift,
                Vec3::from_array(pose.target) + shift,
                up,
                pose.fov_y_radians,
                pose.near,
                pose.far,
            )
            .unwrap();
            render(
                &mut r,
                &moved,
                &mut asset,
                Some(&mut foliage),
                Some(&mut glass_instances),
            );
        }
        save(&mut r, "full-pan40");
    }
    log["stats"] = serde_json::json!(format!("{:?}", r.transparency_stats()));
    if sections.contains("timing") {
        // Two phases of 8 interleaved 48-frame blocks, keeping each block's last 16 frames:
        // a changed draw set relights the radiance cache, which goes idle after 16 still
        // frames, so kept frames time TP1, not relighting. With the warmed emitters:
        // foliage plus particles, then everything (glass cost is the difference). Without
        // emitters: opaque, then foliage.
        if !sections.contains("images") {
            for _ in 0..300 {
                render(
                    &mut r,
                    &camera,
                    &mut asset,
                    Some(&mut foliage),
                    Some(&mut glass_instances),
                );
            }
        }
        let mut phase = |r: &mut Renderer,
                         asset: &mut quarry_prepared::Prepared,
                         configs: [u8; 2]| {
            let mut rows: [Vec<f64>; 2] = Default::default();
            for _ in 0..8 {
                for (k, &config) in configs.iter().enumerate() {
                    for _ in 0..48 {
                        match config {
                            0 => render(r, &camera, asset, None, None),
                            1 => render(r, &camera, asset, Some(&mut foliage), None),
                            _ => render(
                                r,
                                &camera,
                                asset,
                                Some(&mut foliage),
                                Some(&mut glass_instances),
                            ),
                        };
                    }
                    r.test_drain("TP1 timing block");
                    let mut collected = vec![];
                    samples(r, &mut collected);
                    let skip = collected.len().saturating_sub(16);
                    rows[k].extend(collected.into_iter().skip(skip));
                }
            }
            rows.map(|mut v| serde_json::json!({"median_ms": median(&mut v), "frames": v.len()}))
        };
        let [particles, full] = phase(&mut r, &mut asset, [1, 2]);
        drop((fire_emitter, smoke_emitter));
        let [opaque, foliage_only] = phase(&mut r, &mut asset, [0, 1]);
        log["timing"] = serde_json::json!({"opaque": opaque, "foliage": foliage_only,
            "foliage_particles": particles, "full": full,
            "note": "GPU render span (first receiver pass to present end) medians over the last 16 frames of 8 interleaved 48-frame blocks"});
    } else {
        drop((fire_emitter, smoke_emitter));
    }
    std::fs::write(
        out.join(format!("{path}-tp1.json")),
        serde_json::to_string_pretty(&log).unwrap(),
    )
    .unwrap();
}
