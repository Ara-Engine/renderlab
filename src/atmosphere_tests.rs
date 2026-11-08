//! AT1 CPU checks: the CPU transmittance against the medium's closed-form optical depth,
//! validation, directional scaling, and naga validation of every atmosphere module.
use super::*;

fn validate(source: &str) {
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| {
        panic!("{}", e.emit_to_string(source));
    });
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn zenith_transmittance_matches_closed_form() {
    let a = Atmosphere::default();
    let m = a.medium;
    // Straight up from the ground: each density integrates to its scale height (the top's
    // cut-off is negligible) and the ozone tent to its half width.
    let expected: Vec<f32> = (0..3)
        .map(|k| {
            let depth = m.rayleigh_scattering[k]
                * m.rayleigh_scale_height_km
                * (1.0 - (-100.0f32 / m.rayleigh_scale_height_km).exp())
                + (m.mie_scattering[k] + m.mie_absorption[k]) * m.mie_scale_height_km
                + m.ozone_absorption[k] * m.ozone_half_width_km;
            (-depth).exp()
        })
        .collect();
    let t = a.transmittance(1.0, Vec3::Y);
    for k in 0..3 {
        assert!((t[k] - expected[k]).abs() < 2e-3, "{t:?} {expected:?}");
    }
    // Blue scatters most; a low sun is redder and dimmer; a set sun is dark.
    assert!(t[2] < t[1] && t[1] < t[0]);
    let low = a.transmittance(1.0, Vec3::new(0.99, 0.087, 0.0));
    assert!(low.iter().zip(&t).all(|(l, z)| l < z));
    assert!(low[2] / low[0] < t[2] / t[0]);
    assert_eq!(a.transmittance(1.0, Vec3::new(1.0, -0.2, 0.0)), [0.0; 3]);
}

#[test]
fn settings_validate() {
    assert!(Atmosphere::default().validate().is_ok());
    assert!(Atmosphere::off().validate().is_ok());
    assert!(!Atmosphere::off().active());
    let mut a = Atmosphere::default();
    a.medium.top_radius_km = a.medium.ground_radius_km;
    assert!(a.validate().is_err());
    let mut a = Atmosphere::default();
    a.height_fog = Some(HeightFog {
        anisotropy: 1.0,
        ..Default::default()
    });
    assert!(a.validate().is_err());
    let mut a = Atmosphere::default();
    a.volumetric_fog = Some(VolumetricFog {
        history_weight: 1.0,
        ..Default::default()
    });
    assert!(a.validate().is_err());
}

#[test]
fn only_directional_records_scale() {
    let lighting = crate::Lighting {
        lights: vec![
            crate::Light::Directional {
                travel: -Vec3::Y,
                lux: [2.0, 4.0, 8.0],
            },
            crate::Light::Point {
                position: Vec3::ZERO,
                candela: [1.0; 3],
                range: None,
            },
        ],
        ..Default::default()
    };
    let packed = crate::lighting::PackedLighting::new(&lighting, 8).unwrap();
    let scaled = scale_directional(&packed.records, [0.5, 0.25, 0.125]);
    let rgb = |r: &[u8], i: usize| -> [f32; 3] {
        std::array::from_fn(|k| {
            let at = i * 64 + 32 + 4 * k;
            f32::from_ne_bytes(r[at..at + 4].try_into().unwrap())
        })
    };
    assert_eq!(rgb(&scaled, 0), [1.0; 3]);
    assert_eq!(rgb(&scaled, 1), rgb(&packed.records, 1));
}

#[test]
fn atmosphere_modules_validate() {
    let common = include_str!("../shaders/atmosphere_common.wgsl");
    let kernels = include_str!("../shaders/atmosphere.wgsl");
    validate(&format!(
        "{common}{kernels}fn at_sun_visible(p:vec3<f32>) -> f32 {{ return 1.0; }}\n"
    ));
    validate(&format!(
        "{common}{kernels}{}",
        include_str!("../shaders/atmosphere_shadow_map.wgsl")
    ));
    validate(&format!(
        "{common}{kernels}{}",
        crate::Renderer::atmosphere_shadow_source()
    ));
    validate(&format!(
        "{}{}",
        api_source(),
        include_str!("../shaders/atmosphere_sky.wgsl")
    ));
    // Every camera-pixel stage of the standard program composes with the API.
    let prelude = include_str!("../shaders/scene.wgsl");
    let raster = crate::program::compose(
        prelude,
        crate::STANDARD_PROGRAM,
        crate::program::Stage::Raster,
        false,
    );
    validate(&raster.source);
}

// ---- GPU: the quarry at noon and at a low sun on both paths (docs/m7-at1.md) ----
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

use crate::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn rt_device() -> (wgpu::Device, wgpu::Queue) {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"),
        "GPU tests require explicit caller acknowledgment"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let limits = crate::traced_limits(wgpu::Limits {
        max_blas_primitive_count: a.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
        max_acceleration_structures_per_shader_stage: 1,
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        ..Default::default()
    });
    // SAFETY: the test caller explicitly accepts wgpu's experimental implementation risk.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | wgpu::Features::EXPERIMENTAL_RAY_QUERY
            | crate::TRACED_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

fn no_rt_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let mut limits = crate::visibility_limits(wgpu::Limits::default());
    limits.max_buffer_size = a.max_buffer_size;
    limits.max_storage_buffer_binding_size = a.max_storage_buffer_binding_size;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: limits,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

/// The read-only clipmap contract: lit/blocked heights, coarse fallback, missing
/// epochs and samples outside coverage. This device has no ray-query feature.
#[test]
#[ignore = "GPU; VF1 owned lease"]
fn gpu_vf1_clip_reader() {
    use wgpu::util::DeviceExt;
    let (device, queue) = no_rt_device();
    let mut frame = vec![0u8; 1680]; // the reused VsmFrame ABI
    let mut put = |at: usize, words: [u32; 4]| {
        for (i, word) in words.into_iter().enumerate() {
            frame[at + i * 4..at + i * 4 + 4].copy_from_slice(&word.to_ne_bytes());
        }
    };
    put(0, [1.0f32.to_bits(), 0, 0, 0]);
    put(16, [0, 1.0f32.to_bits(), 0, 0]);
    put(32, [0, 0, 1.0f32.to_bits(), 0]);
    put(
        64,
        [0, 1.0f32.to_bits(), 1.0f32.to_bits(), 1.0f32.to_bits()],
    );
    put(96, [2, 1, 0, 0]);
    for (i, (origin, texel)) in [(-64.0f32, 1.0f32), (-128.0, 2.0)].into_iter().enumerate() {
        put(
            128 + i * 32,
            [origin.to_bits(), origin.to_bits(), texel.to_bits(), 0],
        );
        put(144 + i * 32, [1, 0, 0, 0]);
    }
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("VF1 synthetic clipmap"),
        contents: &frame,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let top: Vec<u8> = [0.75f32, 0.5]
        .into_iter()
        .flat_map(|height| {
            (0..128 * 128).flat_map(move |_| {
                [1u32, height.to_bits()]
                    .into_iter()
                    .flat_map(u32::to_ne_bytes)
            })
        })
        .collect();
    let top = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("VF1 synthetic top"),
        contents: &top,
        usage: wgpu::BufferUsages::STORAGE,
    });
    let pages = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 8,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let result = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 28,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 28,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let source = format!(
        "{}{}",
        Renderer::atmosphere_shadow_source(),
        r#"
@group(0) @binding(24) var<storage,read_write> results:array<f32>;
@compute @workgroup_size(1) fn test_reader() {
    results[0]=at_sun_visible(vec3<f32>(0.0,0.0,0.25));
    results[1]=at_sun_visible(vec3<f32>(0.0,0.0,0.9));
    results[2]=at_sun_visible(vec3<f32>(80.0,0.0,0.6));
    results[3]=at_sun_visible(vec3<f32>(80.0,0.0,0.25));
    results[4]=at_sun_visible(vec3<f32>(256.0,0.0,0.25));
    results[5]=at_sun_visible(vec3<f32>(0.0,0.0,2.0));
    results[6]=at_sun_visible(vec3<f32>(0.0,0.0,0.6));
}"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("VF1 clip reader oracle"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("test_reader"),
        compilation_options: Default::default(),
        cache: None,
    });
    let entries = [&uniform, &top, &pages, &result]
        .into_iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry {
            binding: 21 + i as u32,
            resource: b.as_entire_binding(),
        })
        .collect::<Vec<_>>();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    for (epochs, expected) in [
        ([1u32, 1], [0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0]),
        ([0, 1], [0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0]),
        ([0, 0], [1.0; 7]),
    ] {
        queue.write_buffer(
            &pages,
            0,
            &epochs
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect::<Vec<_>>(),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, 28);
        let done = std::sync::Arc::new(std::sync::OnceLock::new());
        let callback = done.clone();
        encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |r| {
            callback.set(r).unwrap();
        });
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        done.get().unwrap().as_ref().unwrap();
        let view = readback.get_mapped_range(..).unwrap();
        let actual = view
            .chunks_exact(4)
            .map(|v| f32::from_ne_bytes(v.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "page epochs {epochs:?}");
        drop(view);
        readback.unmap();
    }
}

/// The exposed HDR target as RGB.
fn capture(r: &mut Renderer) -> Vec<[f32; 3]> {
    r.test_drain("AT1 HDR admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("AT1 HDR copy");
    capture
        .try_read()
        .unwrap()
        .unwrap()
        .chunks_exact(8)
        .map(|p| {
            std::array::from_fn(|k| crate::decode_f16(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])))
        })
        .collect()
}

/// Reinhard then 1/2.2, as the GI checks' inspection images.
fn png(path: &Path, size: [u32; 2], pixels: &[[f32; 3]]) {
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|c| {
            c.map(|v| ((v.max(0.0) / (1.0 + v.max(0.0))).powf(1.0 / 2.2) * 255.0 + 0.5) as u8)
        })
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

/// A sun at `elevation` (radians) on the azimuth of `azimuth_from`, travelling down.
fn sun_travel(azimuth_from: Vec3, elevation: f32) -> Vec3 {
    let h = Vec3::new(azimuth_from.x, 0.0, azimuth_from.z).normalize();
    -(h * elevation.cos() + Vec3::Y * elevation.sin())
}

/// AT1's quarry inspection, one path per process (AT1_PATH: `raster` = the RT device's
/// code default, raster primaries and Exact; `nonrt` = the visibility buffer and Virtual;
/// `traced` = traced primaries and Interactive). Renders, per configuration, PNG and raw
/// exposed HDR (`<path>-<config>.png/.f32`) of pose 0:
///   pre-at1        Atmosphere::off(), the RLE2 sky and the quarry light (the old default);
///   default        Atmosphere::default() with the same captured sky and light;
///   noon / low     the procedural environment and sky, a 120 klux sun at 65 / 4 degrees;
///   low-volumetric the low sun with 0.01/m volumetric fog.
/// Then times AT1_FRAMES (default 60) static frames per timing configuration, twice
/// (`<path>-at1.json`): wall-clock throughput and the timed passes.
#[test]
#[ignore = "GPU; optimized build; AT1_QUARRY (G1c's manifest), AT1_OUT, AT1_PATH"]
fn gpu_at1_quarry() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("AT1_QUARRY").unwrap()).unwrap()).unwrap();
    let out = PathBuf::from(env("AT1_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let path_of = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let (policy, _) = quarry_policy::Policy::args(vec![
        "--limits".into(),
        path_of("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let limits = policy.prepared();
    let source = crate::asset::decode(&std::fs::read(path_of("asset")).unwrap(), limits).unwrap();
    let cameras = quarry_cameras::Cameras::load(&path_of("cameras")).unwrap();
    let path = env("AT1_PATH").unwrap_or_else(|| "raster".into());
    let renderer_limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    const SOURCE: &str = include_str!("../shaders/scene.wgsl");
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    let options = RendererOptions::default();
    let mut r = if path == "nonrt" {
        let (device, queue) = no_rt_device();
        Renderer::new_with_options(device, queue, FORMAT, renderer_limits, options, SOURCE).unwrap()
    } else {
        let (device, queue) = rt_device();
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            renderer_limits,
            options,
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap()
    };
    if path == "traced" {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    let lighting: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path_of("lighting")).unwrap()).unwrap();
    let v3 = |v: &serde_json::Value| -> [f32; 3] {
        std::array::from_fn(|k| v[k].as_f64().unwrap() as f32)
    };
    let quarry_travel = Vec3::from_array(v3(&lighting["lights"][0]["travel"]));
    let quarry_lux = v3(&lighting["lights"][0]["lux"]);
    let exposure = manifest["exposure_stops"].as_f64().unwrap() as f32;
    let shadows = Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        }],
        ..Default::default()
    };
    let set_sun = |r: &mut Renderer, travel: Vec3, lux: [f32; 3]| {
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional { travel, lux }],
                environment: [0.0; 3],
                exposure_stops: exposure,
            },
            &shadows,
        )
        .unwrap();
    };
    set_sun(&mut r, quarry_travel, quarry_lux);
    if path == "traced" {
        r.set_soft_shadows(SoftShadows::Interactive);
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
        r.test_drain("AT1 environment");
    };
    drop(map);
    let mut asset = quarry_prepared::Prepared::new_with_limits(source, limits, 1_000_000).unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("AT1 upload");
    if (env("AT2").is_some() || env("VF2").is_some()) && path == "nonrt" {
        let bvh = crate::asset::cluster_bvh::decode(
            &std::fs::read(path_of("cluster_bvh")).unwrap(),
            crate::asset::cluster_bvh::Limits::default(),
        )
        .unwrap();
        for (m, mesh) in asset.meshes_mut().iter().enumerate() {
            let level = bvh
                .levels
                .iter()
                .position(|l| l.mesh as usize == m)
                .unwrap();
            r.set_indirect_bvh(mesh, bvh.clone(), level).unwrap();
        }
    }
    let frame = &cameras.frames[0];
    let camera = frame.camera().unwrap();
    let size = [frame.width, frame.height];
    let target = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("AT1 color"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let render = |r: &mut Renderer, asset: &mut quarry_prepared::Prepared, camera: &Camera| {
        let mut draws = asset.draws(Visibility::Frustum);
        let until = Instant::now() + Duration::from_secs(60);
        while r
            .try_render_view(&target, camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < until, "AT1 admission");
            r.test_drain("AT1 admission");
        }
    };
    if env("VF2").is_some() {
        vf2::quarry(
            &mut r,
            &mut asset,
            &camera,
            size,
            &out,
            &path,
            quarry_travel,
            set_sun,
            render,
        );
        return;
    }
    if env("AT2").is_some() {
        at2::quarry(
            &mut r,
            &mut asset,
            &camera,
            size,
            &out,
            &path,
            quarry_travel,
            set_sun,
            render,
        );
        return;
    }
    let noon = sun_travel(quarry_travel, 65f32.to_radians());
    let low = sun_travel(quarry_travel, 4f32.to_radians());
    let space = [120_000.0f32; 3];
    let volumetric = Atmosphere {
        volumetric_fog: Some(VolumetricFog {
            extra_density: 0.01,
            ..Default::default()
        }),
        ..Default::default()
    };
    let legacy = Atmosphere {
        volumetric_fog: None,
        ..Default::default()
    };
    // (name, atmosphere, captured sky, sun travel, lux, frames)
    let configs = [
        (
            "pre-at1",
            Atmosphere::off(),
            true,
            quarry_travel,
            quarry_lux,
            8,
        ),
        (
            "default",
            Atmosphere::default(),
            true,
            quarry_travel,
            quarry_lux,
            8,
        ),
        ("legacy", legacy, true, quarry_travel, quarry_lux, 24),
        ("noon", Atmosphere::default(), false, noon, space, 24),
        ("low", legacy, false, low, space, 24),
        ("low-default", Atmosphere::default(), false, low, space, 32),
        ("low-volumetric", volumetric, false, low, space, 24),
    ];
    let mut log = serde_json::json!({"path": path,
        "size": size, "exposure_stops": exposure, "configs": {}});
    let apply =
        |r: &mut Renderer, atmosphere: Atmosphere, captured: bool, travel: Vec3, lux: [f32; 3]| {
            r.set_atmosphere(atmosphere).unwrap();
            r.set_environment(captured.then_some(&sky), Default::default())
                .unwrap();
            set_sun(r, travel, lux);
        };
    for (name, atmosphere, captured, travel, lux, frames) in configs {
        apply(&mut r, atmosphere, captured, travel, lux);
        for _ in 0..frames {
            render(&mut r, &mut asset, &camera);
        }
        let image = capture(&mut r);
        png(&out.join(format!("{path}-{name}.png")), size, &image);
        let raw: Vec<u8> = image
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        std::fs::write(out.join(format!("{path}-{name}.f32")), raw).unwrap();
        let mean: Vec<f64> = (0..3)
            .map(|k| image.iter().map(|p| f64::from(p[k])).sum::<f64>() / image.len() as f64)
            .collect();
        log["configs"][name] = serde_json::json!({"mean_exposed_rgb": mean,
            "sun_travel": travel.to_array(), "lux": lux, "captured_sky": captured, "frames": frames});
    }
    // A near-horizontal view toward the low sun makes the shafts behind the
    // rocks visible in the surrounding medium, rather than against surfaces only.
    let mut shaft_pose = frame.clone();
    shaft_pose.eye[1] = 1.1;
    shaft_pose.target[1] = 1.1;
    let shaft_camera = shaft_pose.camera().unwrap();
    for (name, atmosphere) in [
        ("off", legacy),
        ("default", Atmosphere::default()),
        ("dense", volumetric),
    ] {
        apply(&mut r, atmosphere, false, low, space);
        for _ in 0..24 {
            render(&mut r, &mut asset, &shaft_camera);
        }
        png(
            &out.join(format!("{path}-shaft-view-{name}.png")),
            size,
            &capture(&mut r),
        );
    }
    // VF1: a short camera-only move followed by a hold, so world reprojection is
    // exercised without caster uploads resetting fog history every frame.
    apply(&mut r, volumetric, false, low, space);
    for step in 0..24 {
        let mut pose = frame.clone();
        let offset = (step.min(15) as f32) * 0.005;
        pose.eye[0] += offset;
        pose.target[0] += offset;
        render(&mut r, &mut asset, &pose.camera().unwrap());
        if [0, 7, 15, 23].contains(&step) {
            png(
                &out.join(format!("{path}-motion-{step:02}.png")),
                size,
                &capture(&mut r),
            );
        }
    }
    // Timing: static frames back to back after a warm-up, every configuration twice.
    let frames: u32 = env("AT1_FRAMES").map_or(60, |s| s.parse().unwrap());
    // Pairs that change one thing: the atmosphere (off/default), the environment source at
    // the quarry sun (procedural), the low sun alone (off-low), then the procedural sky and
    // volumetric fog at the low sun.
    let timing_configs = [
        ("off", legacy, true, quarry_travel, quarry_lux),
        (
            "default",
            Atmosphere::default(),
            true,
            quarry_travel,
            quarry_lux,
        ),
        (
            "procedural",
            Atmosphere::default(),
            false,
            quarry_travel,
            space,
        ),
        ("off-low", Atmosphere::off(), true, low, space),
        ("procedural-low", legacy, false, low, space),
        ("default-fog", Atmosphere::default(), false, low, space),
        ("volumetric", volumetric, false, low, space),
    ];
    let mut timing = serde_json::json!({});
    for round in 0..2 {
        for (name, atmosphere, captured, travel, lux) in timing_configs {
            apply(&mut r, atmosphere, captured, travel, lux);
            for _ in 0..20 {
                render(&mut r, &mut asset, &camera);
            }
            r.test_drain("AT1 warm");
            let _ = r.poll().unwrap();
            let start = Instant::now();
            let mut samples = vec![];
            for _ in 0..frames {
                render(&mut r, &mut asset, &camera);
                samples.extend(r.poll().unwrap().gpu);
            }
            r.test_drain("AT1 timing");
            let wall = start.elapsed().as_secs_f64() * 1e3 / f64::from(frames);
            samples.extend(r.poll().unwrap().gpu);
            let mean = |f: &dyn Fn(&GpuSample) -> Option<Duration>| {
                let v: Vec<f64> = samples
                    .iter()
                    .filter_map(f)
                    .map(|d| d.as_secs_f64() * 1e3)
                    .collect();
                (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
            };
            timing[format!("{name}-r{round}")] = serde_json::json!({
                "wall_ms_per_frame": wall,
                "render_span_ms": mean(&|s| Some(s.render_span)),
                "mesh_pass_ms": mean(&|s| Some(s.mesh_pass)),
                "receiver_pass_ms": mean(&|s| s.receiver_pass),
                "visibility_compute_ms": mean(&|s| s.visibility_compute),
                "gpu_samples": samples.len(),
            });
        }
    }
    log["timing"] = timing;
    log["frames_timed"] = frames.into();
    std::fs::write(
        out.join(format!("{path}-at1.json")),
        serde_json::to_string_pretty(&log).unwrap(),
    )
    .unwrap();
}

#[path = "atmosphere_at2_tests.rs"]
mod at2;

#[path = "atmosphere_vf2_tests.rs"]
mod vf2;
