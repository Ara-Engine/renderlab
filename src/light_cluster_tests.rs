//! LC1 (docs/m7-lc1.md): clustered culling, shadowed-light selection, composition on every
//! shading head, and the quarry-at-night scene on both paths (GPU, ignored).
use super::*;
use crate::light_cluster as lc;
use std::path::PathBuf;

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../examples/m4/cameras.rs"]
mod quarry_cameras;
#[allow(clippy::duplicate_mod)]
#[path = "../support/limits.rs"]
mod quarry_policy;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../viewer/src/prepared.rs"]
mod quarry_prepared;

const SOURCE: &str = include_str!("../shaders/scene.wgsl");

/// A small deterministic generator (xorshift), so scenes are identical across runs.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// The night scene's lights: a dim shadowed moon, `count` small coloured point and spot
/// lights over the quarry's floor, and four bright ones placed to rank first.
fn night_lights(count: usize) -> Vec<Light> {
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let mut lights = vec![Light::Directional {
        travel: Vec3::new(0.3, -1.0, 0.45),
        lux: [0.05, 0.07, 0.12],
    }];
    for i in 0..count {
        let hue = rng.next() * 6.0;
        let rgb = [0.0f32, 2.0, 4.0].map(|o| (((hue + o) % 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0));
        let candela = rng.range(3.0, 12.0);
        let position = Vec3::new(
            rng.range(-3.2, 3.2),
            rng.range(0.3, 1.8),
            rng.range(-3.2, 3.2),
        );
        let range = Some(rng.range(0.8, 1.8));
        lights.push(if i % 3 == 0 {
            Light::Spot {
                position,
                travel: Vec3::new(rng.range(-0.4, 0.4), -1.0, rng.range(-0.4, 0.4)),
                candela: rgb.map(|c| c * candela * 3.0),
                range,
                inner: 0.25,
                outer: 0.55,
            }
        } else {
            Light::Point {
                position,
                candela: rgb.map(|c| c * candela),
                range,
            }
        });
    }
    // Two warm spots and two cool points, the frame's most important lights.
    for (k, position) in [
        Vec3::new(1.2, 2.6, 2.0),
        Vec3::new(-2.0, 2.4, 0.5),
        Vec3::new(0.3, 1.6, -1.6),
        Vec3::new(2.8, 1.4, -0.4),
    ]
    .into_iter()
    .enumerate()
    {
        let rgb = if k < 2 {
            [1.0, 0.72, 0.45]
        } else {
            [0.55, 0.75, 1.0]
        };
        lights.push(if k < 2 {
            Light::Spot {
                position,
                travel: Vec3::new(-0.2 * position.x, -1.0, -0.2 * position.z),
                candela: rgb.map(|c| c * 150.0),
                range: Some(9.0),
                inner: 0.35,
                outer: 0.7,
            }
        } else {
            Light::Point {
                position,
                candela: rgb.map(|c| c * 60.0),
                range: Some(7.0),
            }
        });
    }
    lights
}

fn test_camera() -> Camera {
    Camera::look_at(
        Vec3::new(5.2, 3.4, 6.2),
        Vec3::new(0.0, 0.6, -0.15),
        Vec3::Y,
        0.85,
        0.005,
        100.0,
    )
    .unwrap()
}

/// Whether `light` reaches `p` with nonzero attenuation (the light loop's own test).
fn reaches(light: &Light, p: Vec3) -> bool {
    match *light {
        Light::Directional { .. } => true,
        Light::Point {
            position, range, ..
        } => range.is_none_or(|r| position.distance(p) < r),
        Light::Spot {
            position,
            travel,
            range,
            outer,
            ..
        } => {
            let d = p - position;
            range.is_none_or(|r| d.length() < r)
                && d.normalize().dot(travel.normalize()) > outer.cos()
        }
    }
}

#[test]
fn cluster_lists_hold_every_light_that_reaches_a_receiver() {
    let lights = night_lights(256);
    let camera = test_camera();
    let viewport = [1920, 1080];
    let inverse = camera.matrix(1920, 1080).inverse();
    let mut rng = Rng(7);
    let mut cache = std::collections::HashMap::new();
    let (mut tested, mut reached, mut longest) = (0, 0, 0);
    while tested < 4000 {
        // A random point on a random camera ray, at a random depth up to 15 m.
        let ndc = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
        let far = inverse.project_point3(Vec3::new(ndc[0], ndc[1], 0.0001));
        let eye = camera.view().inverse().w_axis.truncate();
        let p = eye + (far - eye).normalize() * rng.range(0.05, 15.0);
        let Some(cluster) = lc::cluster_of(&camera, viewport, p) else {
            continue;
        };
        tested += 1;
        let list = cache
            .entry(cluster)
            .or_insert_with(|| lc::reference_cluster(&lights, &camera, viewport, cluster));
        longest = longest.max(list.len());
        for (i, light) in lights.iter().enumerate() {
            if reaches(light, p) {
                reached += 1;
                assert!(
                    list.contains(&(i as u32)),
                    "light {i} reaches {p} outside cluster {cluster}"
                );
            }
        }
    }
    assert!(reached > 4000, "{reached}");
    assert!(longest <= lc::MAX_PER_CLUSTER as usize, "{longest}");
}

#[test]
fn shadowed_lights_rank_by_illuminance_and_keep_their_slots() {
    let camera = test_camera();
    let vp = camera.matrix(1920, 1080);
    let eye = Vec3::new(5.2, 3.4, 6.2);
    let point = |position: Vec3, candela: f32| Light::Point {
        position,
        candela: [candela; 3],
        range: Some(5.0),
    };
    let lights = vec![
        Light::Directional {
            travel: -Vec3::Y,
            lux: [1.0; 3],
        },
        point(Vec3::new(0.0, 1.0, 0.0), 100.0),
        point(Vec3::new(1.0, 1.0, 0.0), 50.0),
        // Behind the camera, beyond its range: never chosen.
        point(eye + Vec3::new(10.0, 0.0, 10.0), 1e6),
        point(Vec3::new(-1.0, 1.0, 0.0), 80.0),
    ];
    let slots = lc::select(&lights, vp, eye, &[], 2);
    assert_eq!(slots, [Some(1), Some(4)]);
    // Light 2 brightens past 4 by less than the 25 % hysteresis: 4 keeps its slot.
    let mut brighter = lights.clone();
    brighter[2] = point(Vec3::new(1.0, 1.0, 0.0), 70.0);
    assert_eq!(
        lc::select(&brighter, vp, eye, &slots, 2),
        [Some(1), Some(4)]
    );
    // Well past it: 2 takes 4's slot, and 1 keeps slot 0.
    brighter[2] = point(Vec3::new(1.0, 1.0, 0.0), 300.0);
    assert_eq!(
        lc::select(&brighter, vp, eye, &slots, 2),
        [Some(1), Some(2)]
    );
    assert!(lc::select(&lights, vp, eye, &[], 0).is_empty());
    assert_eq!(lc::matrices(&lights[1], 0.02).len(), 6);
}

#[test]
fn light_cluster_kernel_and_every_shading_head_validate() {
    let validate = |label: &str, source: &str| {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
    };
    validate("kernel", &format!("{}{}", lc::constants(), lc::KERNEL));
    use crate::program::{Stage, compose};
    let standard = crate::STANDARD_PROGRAM;
    // LC1's own visibility variant in each module: one inline ray, or the pooled maps.
    let ray = |module: &str, scene: &str| {
        module.contains(&format!(
            "rayQueryInitialize(&lc_query,{scene},RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,receiver.epsilon,tmax,origin,delta/distance))"
        ))
    };
    let map = |module: &str| module.contains("{ return rl_local_shadow_map(light,receiver,l); }");
    let raster = compose(SOURCE, standard, Stage::Raster, false).source;
    assert!(map(&raster) && !ray(&raster, "hardware_scene"));
    validate("raster", &raster);
    let hardware = crate::hardware_shadows::shader_source(SOURCE, crate::SampleCount::One).unwrap();
    let hardware = compose(&hardware, standard, Stage::Raster, false).source;
    assert!(ray(&hardware, "hardware_scene") && !map(&hardware));
    validate("hardware raster", &hardware);
    // The fresh raster layout binds no acceleration structure for fragments: maps.
    let fresh = fresh_shadow_gpu::shader_source(SOURCE).unwrap();
    let fresh = compose(&fresh, standard, Stage::Raster, false).source;
    assert!(map(&fresh) && !ray(&fresh, "fresh_scene"));
    validate("fresh raster", &fresh);
    let traced = primary_ray_gpu::shading_heads(SOURCE).unwrap();
    for (k, (head, scene)) in traced
        .iter()
        .zip(["hardware_scene", "fresh_scene"])
        .enumerate()
    {
        let module = compose(head, standard, Stage::Traced, false).source;
        assert!(ray(&module, scene) && !map(&module), "traced {k}");
        validate(&format!("traced {k}"), &module);
    }
    let non_rt = primary_ray_gpu::non_rt_sources(SOURCE).unwrap().1;
    for (k, head) in non_rt.iter().enumerate() {
        let module = compose(head, standard, Stage::Traced, false).source;
        assert!(map(&module), "non-RT {k}");
        validate(&format!("non-RT {k}"), &module);
    }
}

// ---- the quarry at night (GPU) ----

// IQ1's two quarry devices (src/iq1_tests.rs), unchanged.
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
            | crate::TRACED_FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

/// A device without ray queries that selects the visibility buffer and Virtual shadows.
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
            | crate::VIRTUAL_FEATURES,
        required_limits: limits,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Tone-mapped (fs_present's Reinhard) 8-bit sRGB-ish PNG of an exposed HDR capture.
fn write_png(path: &std::path::Path, rgb: &[f32], width: u32, height: u32) {
    let bytes: Vec<u8> = rgb
        .iter()
        .map(|v| {
            let t = (v / (1.0 + v)).clamp(0.0, 1.0);
            (t.powf(1.0 / 2.2) * 255.0 + 0.5) as u8
        })
        .collect();
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

fn capture(r: &mut Renderer) -> Vec<f32> {
    r.test_drain("LC1 HDR admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("LC1 HDR copy");
    capture
        .try_read()
        .unwrap()
        .unwrap()
        .chunks_exact(8)
        .flat_map(|p| {
            (0..3).map(move |k| crate::decode_f16(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])))
        })
        .collect()
}

impl Renderer {
    /// The cluster list texture as words in cluster order (`STRIDE` per cluster).
    fn test_cluster_words(&mut self) -> Vec<u32> {
        self.test_drain("LC1 list readback");
        let texture = &self.lighting.clusters.test_lists();
        let [w, h] = lc::LIST_SIZE;
        let row = (w * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LC1 list readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        let data = slice.get_mapped_range().unwrap();
        data.chunks_exact(row as usize)
            .flat_map(|line| {
                line[..(w * 4) as usize]
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// LC1's quarry at night, one path per process (LC1_PATH: `raster` = the RT device's code
/// default, raster primaries and Exact; `traced` = the RT device with traced primaries and
/// Interactive; `nonrt` = the visibility buffer and Virtual). LC1_QUARRY names G1c's
/// manifest; LC1_OUT receives PNGs, raw f32 captures and `lc1-<path>.json` (cluster checks,
/// selection, timings). LC1_LIGHTS sets the small-light count (default 256), LC1_FRAMES
/// the timed frames per configuration (default 24).
#[test]
#[ignore = "GPU; optimized build; LC1_QUARRY, LC1_OUT, LC1_PATH"]
fn gpu_lc1_quarry_night() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("LC1_QUARRY").unwrap()).unwrap()).unwrap();
    let out = PathBuf::from(
        env("LC1_OUT")
            .or_else(|| env("RENDERLAB_M7_G1_OUT"))
            .unwrap(),
    );
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
    let path = env("LC1_PATH").unwrap_or_else(|| "raster".into());
    let count: usize = env("LC1_LIGHTS").map_or(256, |s| s.parse().unwrap());
    let frames: usize = env("LC1_FRAMES").map_or(24, |s| s.parse().unwrap());
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let renderer_limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let options = RendererOptions {
        samples: SampleCount::One,
        anisotropy_clamp: 1,
    };
    let mut r = if path == "nonrt" {
        let (device, queue) = no_rt_device();
        Renderer::new_with_options(device, queue, format, renderer_limits, options, SOURCE).unwrap()
    } else {
        let (device, queue) = rt_device();
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            format,
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
    let all = night_lights(count);
    let shadows = |n: usize| Shadows {
        directional_disk_angle: 0.04,
        requests: std::iter::once(ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        })
        .chain(std::iter::repeat_n(ShadowRequest::None, n - 1))
        .collect(),
        ..Default::default()
    };
    let lighting = |lights: &[Light]| Lighting {
        lights: lights.to_vec(),
        environment: [0.0015, 0.002, 0.004],
        exposure_stops: -1.0,
    };
    r.set_lighting_and_shadows(&lighting(&all), &shadows(all.len()))
        .unwrap();
    if path == "traced" {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    let mut asset = quarry_prepared::Prepared::new_with_limits(source, limits, 1_000_000).unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("LC1 upload");
    let frame = &cameras.frames[0];
    let camera = frame.camera().unwrap();
    let viewport = [frame.width, frame.height];
    let target = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("LC1 color"),
        size: wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
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
            assert!(Instant::now() < until, "LC1 admission");
            r.test_drain("LC1 admission");
        }
    };
    // Median drained frame time (submission to GPU idle) over `frames` after 4 warm-ups.
    let time = |r: &mut Renderer,
                asset: &mut quarry_prepared::Prepared,
                step: &mut dyn FnMut(&mut Renderer, usize)| {
        let mut samples = vec![];
        for k in 0..frames + 4 {
            step(r, k);
            let start = Instant::now();
            render(r, asset, &camera);
            r.test_drain("LC1 timing");
            if k >= 4 {
                samples.push(start.elapsed().as_secs_f64() * 1e3);
            }
        }
        samples.sort_by(f64::total_cmp);
        serde_json::json!({"median_ms": samples[samples.len() / 2], "min_ms": samples[0],
            "max_ms": samples[samples.len() - 1]})
    };
    let mut log = serde_json::json!({"path": path, "lights": all.len(), "primary": format!("{:?}", r.primary_mode),
        "viewport": viewport, "frames": frames});
    let save = |name: &str, image: &[f32]| {
        write_png(
            &out.join(format!("{path}-{name}.png")),
            image,
            frame.width,
            frame.height,
        );
        let bytes: Vec<u8> = image.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(out.join(format!("{path}-{name}.f32")), bytes).unwrap();
    };
    // 1. The shipped defaults (GI, TR1 on traced and visibility-buffer frames, atmosphere),
    // clustered, four shadowed local lights, after the cache and history settle.
    for _ in 0..12 {
        render(&mut r, &mut asset, &camera);
    }
    save("night", &capture(&mut r));
    let stats = r.light_cluster_stats();
    log["selection"] = serde_json::json!({"shadowed": stats.shadowed, "maps": stats.maps,
        "faces_drawn_total": stats.total_faces_drawn, "clustered": stats.clustered});
    // GPU lists against the CPU reference over every cluster.
    let words = r.test_cluster_words();
    let (mut mismatched, mut overflowed, mut longest, mut total) = (0u32, 0u32, 0u32, 0u64);
    for c in 0..lc::CLUSTERS {
        let base = (c * lc::STRIDE) as usize;
        let n = words[base] & 0xffff;
        let wanted = words[base] >> 16;
        let gpu: Vec<u32> = (0..n)
            .map(|i| (words[base + 1 + (i / 2) as usize] >> ((i & 1) * 16)) & 0xffff)
            .collect();
        let cpu = lc::reference_cluster(&all, &camera, viewport, c);
        let stored: Vec<u32> = cpu
            .iter()
            .copied()
            .take(lc::MAX_PER_CLUSTER as usize)
            .collect();
        if gpu != stored || wanted as usize != cpu.len() {
            mismatched += 1;
        }
        overflowed += u32::from(wanted > n);
        longest = longest.max(wanted);
        total += u64::from(n);
    }
    log["clusters"] = serde_json::json!({"mismatched_vs_cpu": mismatched, "overflowed": overflowed,
        "longest": longest, "mean": total as f64 / f64::from(lc::CLUSTERS)});
    // 2. Timings in the shipped configuration.
    log["time_default"] = time(&mut r, &mut asset, &mut |_, _| {});
    r.set_light_clusters(false);
    log["time_unclustered"] = time(&mut r, &mut asset, &mut |_, _| {});
    r.set_light_clusters(true);
    r.set_local_shadows(LocalShadows {
        count: 0,
        ..Default::default()
    })
    .unwrap();
    render(&mut r, &mut asset, &camera);
    log["time_no_local_shadows"] = time(&mut r, &mut asset, &mut |_, _| {});
    save("night-noshadow", &capture(&mut r));
    r.set_local_shadows(LocalShadows::default()).unwrap();
    // Moving shadowed lights: every frame redraws their maps (where maps are used).
    let moving = |r: &mut Renderer, k: usize| {
        let mut lights = all.clone();
        let n = lights.len();
        for light in &mut lights[n - 4..] {
            if let Light::Point { position, .. } | Light::Spot { position, .. } = light {
                *position += Vec3::new(0.01 * (k % 7) as f32, 0.0, 0.0);
            }
        }
        r.set_lighting(&lighting(&lights)).unwrap();
    };
    log["time_moving_shadowed_lights"] = time(&mut r, &mut asset, &mut |r, k| moving(r, k));
    r.set_lighting(&lighting(&all)).unwrap();
    r.set_lighting_and_shadows(&lighting(&all[..1]), &shadows(1))
        .unwrap();
    log["time_moon_only"] = time(&mut r, &mut asset, &mut |_, _| {});
    save("moon-only", &capture(&mut r));
    // 3. Clustered against walking every light, without the temporal state (GI cache, TR1
    // history) that changes between frames: the sums must be bit-identical.
    r.set_lighting_and_shadows(&lighting(&all), &shadows(all.len()))
        .unwrap();
    r.set_indirect(IndirectSettings::off()).unwrap();
    r.set_reconstruction(Reconstruction::off()).unwrap();
    render(&mut r, &mut asset, &camera);
    render(&mut r, &mut asset, &camera);
    let clustered = capture(&mut r);
    save("night-gi-off", &clustered);
    r.set_light_clusters(false);
    render(&mut r, &mut asset, &camera);
    let unclustered = capture(&mut r);
    r.set_light_clusters(true);
    let differing = clustered
        .iter()
        .zip(&unclustered)
        .filter(|(a, b)| a.to_bits() != b.to_bits())
        .count();
    log["unclustered_differing_channels"] = serde_json::json!(differing);
    r.set_indirect(IndirectSettings::default()).unwrap();
    r.set_reconstruction(Reconstruction::default()).unwrap();
    // 4. The quarry's soft sun (G1c's 1000 lux lighting, 0.04 rad disk) with every local
    // light: by day (-3 stops), and at dusk (sun at 1/50, -1 stop) where both read.
    let sun: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path_of("lighting")).unwrap()).unwrap();
    let v3 = |v: &serde_json::Value| -> [f32; 3] {
        std::array::from_fn(|k| v[k].as_f64().unwrap() as f32)
    };
    let travel = Vec3::from_array(v3(&sun["lights"][0]["travel"]));
    let lux = v3(&sun["lights"][0]["lux"]);
    for (name, scale, exposure) in [("sun-day", 1.0, -3.0), ("sun-dusk", 0.02, -1.0)] {
        let mut lights = all.clone();
        lights[0] = Light::Directional {
            travel,
            lux: lux.map(|l| l * scale),
        };
        r.set_lighting_and_shadows(
            &Lighting {
                lights,
                environment: [0.0; 3],
                exposure_stops: exposure,
            },
            &shadows(all.len()),
        )
        .unwrap();
        for _ in 0..12 {
            render(&mut r, &mut asset, &camera);
        }
        save(name, &capture(&mut r));
        log[name] = serde_json::json!({"shadowed": r.light_cluster_stats().shadowed,
            "maps": r.light_cluster_stats().maps});
    }
    let file = out.join(format!("lc1-{path}.json"));
    std::fs::write(&file, serde_json::to_string_pretty(&log).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&log).unwrap());
    // LC1_RECORD_ONLY=1: a batch records every path before any gate stops it.
    if env("LC1_RECORD_ONLY").as_deref() == Some("1") {
        return;
    }
    assert_eq!(
        mismatched, 0,
        "GPU cluster lists differ from the CPU reference"
    );
    assert_eq!(
        differing, 0,
        "clustered shading differs from walking every light"
    );
}
