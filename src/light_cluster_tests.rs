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
                source_radius: 0.0,
                cast_shadows: None,
            }
        } else {
            Light::Point {
                position,
                candela: rgb.map(|c| c * candela),
                range,
                source_radius: 0.0,
                cast_shadows: None,
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
                source_radius: 0.0,
                cast_shadows: None,
            }
        } else {
            Light::Point {
                position,
                candela: rgb.map(|c| c * 60.0),
                range: Some(7.0),
                source_radius: 0.0,
                cast_shadows: None,
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
fn shadowed_lights_rank_by_screen_influence_and_keep_their_slots() {
    let camera = test_camera();
    let vp = camera.matrix(1920, 1080);
    let eye = Vec3::new(5.2, 3.4, 6.2);
    let point = |position: Vec3, candela: f32| Light::Point {
        position,
        candela: [candela; 3],
        range: Some(5.0),
        source_radius: 0.0,
        cast_shadows: None,
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

/// The RT ray budget (`LocalShadows::rays`): under it every light traces at full strength,
/// exactly the words of no budget; over it the ranking chooses, a first frame at full
/// strength, and a light that loses its place fades out over eight frames before the one
/// that took it fades in. `cast_shadows` overrides the ranking both ways.
#[test]
fn the_ray_budget_ranks_fades_and_honours_each_lights_choice() {
    let camera = test_camera();
    let vp = camera.matrix(1920, 1080);
    let eye = Vec3::new(5.2, 3.4, 6.2);
    let point = |x: f32, candela: f32, cast_shadows: Option<bool>| Light::Point {
        position: Vec3::new(x, 1.0, 0.0),
        candela: [candela; 3],
        range: Some(5.0),
        source_radius: 0.0,
        cast_shadows,
    };
    let sun = Light::Directional {
        travel: -Vec3::Y,
        lux: [1.0; 3],
    };
    let settings = |rays: u32| LocalShadows {
        rays,
        ..LocalShadows::default()
    };
    // Under the budget: every point light's word is its index plus one, the sun none.
    let few = vec![sun, point(0.0, 100.0, None), point(1.0, 50.0, None)];
    let mut selection = lc::RaySelection::default();
    assert_eq!(selection.words(&few, vp, eye, &settings(8)), [0, 2, 3]);
    // Over it: the two brightest at full strength on the first frame.
    let mut lights = vec![
        sun,
        point(0.0, 100.0, None),
        point(1.0, 50.0, None),
        point(-1.0, 80.0, None),
    ];
    let mut selection = lc::RaySelection::default();
    assert_eq!(
        selection.words(&lights, vp, eye, &settings(2)),
        [0, 2, 0, 4]
    );
    // Light 2 outshines light 3: 3 fades out over eight frames, its slot is then 2's,
    // and 2 fades in over the next eight; nothing pops and 1 never changes.
    lights[2] = point(1.0, 400.0, None);
    let mut faded = vec![];
    for _ in 0..17 {
        let words = selection.words(&lights, vp, eye, &settings(2));
        assert_eq!(words[1], 2);
        faded.push((
            words[2] >> 24,
            words[2] & 0xffff,
            words[3] >> 24,
            words[3] & 0xffff,
        ));
    }
    assert_eq!(faded[0], (0, 0, 223, 4), "3 starts fading at 7/8");
    assert_eq!(faded[6], (0, 0, 32, 4));
    assert_eq!(faded[7].2, 0, "3 is out after eight frames");
    assert_eq!(faded[7].3, 0);
    assert_eq!(faded[8], (32, 3, 0, 0), "2 fades in once 3's slot is free");
    assert_eq!(faded[15], (0, 3, 0, 0), "2 at full strength");
    // `cast_shadows`: Some(false) never traces, Some(true) always does, past the budget.
    let chosen = vec![
        sun,
        point(0.0, 1000.0, Some(false)),
        point(1.0, 1.0, Some(true)),
        point(-1.0, 80.0, None),
        point(2.0, 70.0, None),
    ];
    let mut selection = lc::RaySelection::default();
    let words = selection.words(&chosen, vp, eye, &settings(1));
    assert_eq!(
        words,
        [0, 0, 3, 0, 0],
        "the forced light takes the one place"
    );
    let words = selection.words(&chosen, vp, eye, &settings(2));
    assert_eq!(words[1], 0);
    assert_eq!(words[2], 3);
}

#[test]
fn local_shadow_tiles_stay_disjoint_and_fades_release_before_reuse() {
    for maximum in [1, 2, 7, 768, 1001, 1024] {
        let mut tiles = vec![None; lc::MAX_SLOTS as usize];
        for i in 0..lc::MAX_SLOTS as usize {
            let tile = lc::allocate_tile(&tiles, maximum, maximum).expect("every slot fits");
            assert!(tile.size <= maximum);
            tiles[i] = Some(tile);
        }
        assert_eq!(tiles.len(), 128);
        if maximum >= 8 {
            assert!(tiles.iter().flatten().any(|t| t.size < maximum));
        }
    }
    let mut slots = vec![Some(1)];
    let mut strengths = vec![1.0];
    for frame in 1..=8 {
        (slots, strengths) = lc::transition(&slots, &strengths, &[Some(2)], 1);
        assert_eq!(strengths[0], 1.0 - frame as f32 / 8.0);
        assert_eq!(slots[0], Some(if frame < 8 { 1 } else { 2 }));
    }
    for frame in 1..=8 {
        (slots, strengths) = lc::transition(&slots, &strengths, &[Some(2)], 1);
        assert_eq!(strengths[0], frame as f32 / 8.0);
    }
    assert!(lc::FRAME_BYTES <= 65536);
}

#[test]
fn screen_influence_prioritizes_large_pools_at_equal_camera_illuminance() {
    let camera = test_camera();
    let point = |range| Light::Point {
        position: Vec3::ZERO,
        candela: [20.0; 3],
        range: Some(range),
        source_radius: 0.0,
        cast_shadows: None,
    };
    let lights = [point(0.1), point(3.0)];
    assert_eq!(
        lc::select(
            &lights,
            camera.matrix(1920, 1080),
            camera.view().inverse().w_axis.truncate(),
            &[],
            1
        ),
        [Some(1)]
    );
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
    let map = |module: &str| {
        module.contains(
            "return mix(1.0,rl_local_shadow_map(light,receiver,l),rl_light_frame.tiles[slot].w)",
        )
    };
    // The restart ray budget (U-SPECK2) asks the frame's restart zones, which only traced ray
    // stages see; every other module composes the empty stub.
    let budget = |module: &str| module.contains("if rl_restarts(world) { rl_local_rays=restart; }");
    // Every traced module can ask whether the resolve restarts a receiver.
    let zones = |module: &str| {
        module.contains("fn rl_restarts(world:vec3<f32>)->bool")
            && module.contains("fn rl_zone_relit(x:vec3<f32>)->bool")
            && module.contains("primary_frame.zones.relit[k]")
    };
    let stub = |module: &str| module.contains("fn rl_local_budget(world:vec3<f32>) {}");
    let raster = compose(SOURCE, standard, Stage::Raster, false).source;
    assert!(map(&raster) && !ray(&raster, "hardware_scene"));
    assert!(stub(&raster) && !budget(&raster));
    validate("raster", &raster);
    let hardware = crate::hardware_shadows::shader_source(SOURCE, crate::SampleCount::One).unwrap();
    let hardware = compose(&hardware, standard, Stage::Raster, false).source;
    assert!(ray(&hardware, "hardware_scene") && !map(&hardware));
    assert!(stub(&hardware) && !budget(&hardware));
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
        assert!(
            budget(&module) && !stub(&module) && zones(&module),
            "traced {k}"
        );
        assert!(module.contains("let samples=select(1u,max(rl_local_rays,4u),radius>0.0);"));
        // Reflection hits from rough enough lobes skip local shadows
        // (`LocalShadows::unshadowed_hits_from`).
        assert!(module.contains("if rl_reflection_hit && lights[s.index].kind.w!=0u && rl_light_frame.far.z!=0.0 && rl_hist_rough+1.0>=rl_light_frame.far.z { return 1.0; }"));
        validate(&format!("traced {k}"), &module);
    }
    let non_rt = primary_ray_gpu::non_rt_sources(SOURCE).unwrap().1;
    for (k, head) in non_rt.iter().enumerate() {
        let module = compose(head, standard, Stage::Traced, false).source;
        assert!(map(&module), "non-RT {k}");
        assert!(
            stub(&module) && !budget(&module) && zones(&module),
            "non-RT {k}"
        );
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
        let texture = &self.view.clusters.test_lists();
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
/// the timed frames per configuration (default 24). LC2=1 adds 8 cm source radii,
/// hard-shadow captures/timing and the GI visibility counters beyond lights 32 and 256.
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
    } else if path == "raster" {
        r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
        r.set_soft_shadows(SoftShadows::Exact);
    }
    let lc2 = env("LC2").as_deref() == Some("1");
    let lc3 = env("LC3").as_deref() == Some("1");
    let mut all = night_lights(count);
    if lc2 {
        for light in &mut all {
            if let Light::Point { source_radius, .. } | Light::Spot { source_radius, .. } = light {
                *source_radius = 0.08;
            }
        }
    }
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
    // LC2: the low-level non-RT renderer needs explicit software GI geometry. LC1's
    // original night harness omitted it, so its non-RT GI-on capture was a fallback.
    if lc2 && path == "nonrt" {
        let field = crate::asset::near_field::decode(
            &std::fs::read(env("LC2_NEAR").expect("LC2_NEAR names G1c's cooked near field"))
                .unwrap(),
            crate::asset::near_field::Limits::default(),
        )
        .unwrap();
        assert_eq!(field.meshes.len(), asset.meshes_mut().len());
        for (i, mesh) in asset.meshes_mut().iter().enumerate() {
            r.set_indirect_near_field(mesh, &field, i).unwrap();
        }
    }
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
    let capture_device = r.device.clone();
    let capture_queue = r.queue.clone();
    let save = |name: &str, image: &[f32]| {
        write_png(
            &out.join(format!("{path}-{name}.png")),
            image,
            frame.width,
            frame.height,
        );
        if lc2 {
            write_presented_png(
                &capture_device,
                &capture_queue,
                &target,
                &out.join(format!("{path}-{name}-present.png")),
            );
        }
        let bytes: Vec<u8> = image.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(out.join(format!("{path}-{name}.f32")), bytes).unwrap();
    };
    // LC2 also exercises in-flight bitset growth from one light to 261 lights. This
    // follows the same allocation/retirement path a game adding its first many lights uses.
    if lc2 {
        r.set_lighting_and_shadows(&lighting(&all[..1]), &shadows(1))
            .unwrap();
        render(&mut r, &mut asset, &camera);
        r.set_lighting_and_shadows(&lighting(&all), &shadows(all.len()))
            .unwrap();
    }
    // 1. The shipped defaults (GI, TR1 on traced and visibility-buffer frames, atmosphere),
    // clustered, four shadowed local lights, after the cache and history settle.
    for _ in 0..12 {
        render(&mut r, &mut asset, &camera);
    }
    save("night", &capture(&mut r));
    if lc2 {
        log["gi_visible_cells_per_light"] =
            serde_json::json!(gi_light_counts(&mut r, all.len() as u32));
        log["gi_table_bytes"] = serde_json::json!(r.indirect_stats().table_bytes);
        let mut hard = all.clone();
        for light in &mut hard {
            if let Light::Point { source_radius, .. } | Light::Spot { source_radius, .. } = light {
                *source_radius = 0.0;
            }
        }
        r.set_lighting(&lighting(&hard)).unwrap();
        for _ in 0..12 {
            render(&mut r, &mut asset, &camera);
        }
        save("night-hard", &capture(&mut r));
        log["time_hard"] = time(&mut r, &mut asset, &mut |_, _| {});
        r.set_lighting(&lighting(&all)).unwrap();
        for _ in 0..12 {
            render(&mut r, &mut asset, &camera);
        }
    }
    let stats = r.light_cluster_stats();
    log["selection"] = serde_json::json!({"shadowed": stats.shadowed, "maps": stats.maps,
        "faces_drawn_total": stats.total_faces_drawn, "clustered": stats.clustered,
        "resolutions": stats.resolutions, "strengths": stats.strengths, "atlas_bytes": stats.atlas_bytes,
        "local_lights_without_direct_shadow": all.len()-1-stats.shadowed.iter().flatten().count(),
        "cached_faces_redrawn": stats.faces_drawn});
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
    if lc3 {
        r.set_local_shadows(LocalShadows {
            count: 4,
            ..Default::default()
        })
        .unwrap();
        r.view.clusters.legacy_limit = true;
        r.local_maps.test_reset_selection();
        for _ in 0..16 {
            render(&mut r, &mut asset, &camera);
        }
        save("night-lc2-limit", &capture(&mut r));
        let baseline = r.light_cluster_stats();
        log["lc2_limit"] = serde_json::json!({"shadowed":baseline.shadowed,
            "local_lights_without_direct_shadow":all.len()-1-baseline.shadowed.iter().flatten().count(),
            "time":time(&mut r,&mut asset,&mut|_,_|{})});
        r.set_local_shadows(LocalShadows::default()).unwrap();
        r.view.clusters.legacy_limit = false;
        for _ in 0..16 {
            render(&mut r, &mut asset, &camera);
        }
        // One short camera walk, preserving this process's loaded asset and caches.
        let mut samples = vec![];
        let mut walk = vec![];
        for k in 0..16 {
            let shift = Vec3::new(0.03 * k as f32, 0.0, 0.0);
            let moving_camera = Camera::look_at(
                Vec3::from_array(frame.eye) + shift,
                Vec3::from_array(frame.target),
                Vec3::Y,
                frame.fov_y_radians,
                frame.near,
                frame.far,
            )
            .unwrap();
            let begin = Instant::now();
            render(&mut r, &mut asset, &moving_camera);
            r.test_drain("LC3 camera motion");
            if k >= 8 {
                samples.push(begin.elapsed().as_secs_f64() * 1e3);
            }
            let stats = r.light_cluster_stats();
            walk.push(serde_json::json!({"frame": k, "shadowed":stats.shadowed,
                "strengths":stats.strengths,"faces_drawn":stats.faces_drawn}));
            if [0, 3, 7, 11, 15].contains(&k) {
                save(&format!("walk-{k:02}"), &capture(&mut r));
            }
        }
        samples.sort_by(f64::total_cmp);
        log["time_camera_moving"] =
            serde_json::json!({"median_ms":samples[4],"min_ms":samples[0],"max_ms":samples[7]});
        log["walk"] = serde_json::json!(walk);
        for _ in 0..16 {
            render(&mut r, &mut asset, &camera);
        }
    }
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
    if lc3 {
        save("lights-moving", &capture(&mut r));
    }
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
    if lc3 {
        for _ in 0..16 {
            render(&mut r, &mut asset, &camera);
        }
    }
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
    let file = out.join(format!("{}-{path}.json", if lc2 { "lc2" } else { "lc1" }));
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

// LC2: reduce the cache's per-light visibility to small counters on the GPU. No table
// readback or raw dump: the evidence checks lights beyond 32 and beyond 256 explicitly.
fn gi_light_counts(r: &mut Renderer, count: u32) -> Vec<u32> {
    r.test_drain("LC2 GI light list");
    let Some(table) = r.test_lc2_gi_table() else {
        return vec![];
    };
    let output = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LC2 light visibility counters"),
        size: u64::from(count) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LC2 light counter readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // P-MEM: use the production cold layout rather than hard-coding 128-direction padding.
    let lights_offset = crate::renderer::gi_cache::light_cold_offset(r.indirect().directions);
    let source = format!(
        "@group(0) @binding(0) var<storage,read> table:array<u32>;
@group(0) @binding(1) var<storage,read_write> counts:array<atomic<u32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {{
 let slots=table[1]+1u; let slot=id.x; if slot>=slots {{ return; }}
 if (table[64u+slot*24u]&0x80000000u)==0u {{ return; }}
 let cold=64u+slots*24u+slot*table[23];
 if (table[cold]&2u)!=0u {{ return; }}
 for(var i=0u;i<{count}u;i+=1u) {{
  if (table[cold+{lights_offset}u+(i>>5u)]&(1u<<(i&31u)))!=0u {{ atomicAdd(&counts[i],1u); }}
 }}
}}"
    );
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("LC2 list reduction"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = r
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("LC2 list reduction"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: table.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups((1u32 << r.indirect().slots_log2).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    r.queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.test_drain("LC2 light counters");
    let bytes = slice.get_mapped_range().unwrap();
    let counts: Vec<_> = bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert!(
        counts.iter().skip(32).any(|n| *n > 0),
        "lights beyond 32 reach no GI cell"
    );
    assert!(
        counts.iter().skip(256).any(|n| *n > 0),
        "lights beyond 256 reach no GI cell"
    );
    counts
}

/// A bounded penumbra fixture: point and spot, hard/soft, a floor and raised blocker.
/// LC1_PATH selects raster, traced (RT) or nonrt. Captures are visibility itself so a
/// wider penumbra cannot be confused with changed exposure, material or bounce lighting.
#[test]
#[ignore = "GPU; LC1_PATH, LC1_OUT; both local light types, radius zero and positive"]
fn gpu_lc2_penumbra() {
    let path = env("LC1_PATH").unwrap_or_else(|| "traced".into());
    let out = PathBuf::from(
        env("LC1_OUT")
            .or_else(|| env("RENDERLAB_M7_G1_OUT"))
            .unwrap(),
    );
    std::fs::create_dir_all(&out).unwrap();
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    let mut r = if path == "nonrt" {
        let (d, q) = no_rt_device();
        Renderer::new_with_options(
            d,
            q,
            wgpu::TextureFormat::Rgba8Unorm,
            Limits::default(),
            options,
            SOURCE,
        )
        .unwrap()
    } else {
        let (d, q) = rt_device();
        Renderer::new_with_hardware_shadows(
            d,
            q,
            wgpu::TextureFormat::Rgba8Unorm,
            Limits::default(),
            options,
            HardwareShadowOptions::default(),
            SOURCE,
        )
        .unwrap()
    };
    if path == "traced" {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    } else if path == "raster" {
        r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
        r.set_soft_shadows(SoftShadows::Exact);
    }
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "LC2 visibility",
            source: "
struct Surface { unused:f32 };
fn surface(input:SurfaceInput)->Surface {return Surface(0.0);}
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {
 for(var i=0u;i<light_count();i+=1u) {
  let sample=light_sample(i,input);
  if sample.index==1u {return vec3<f32>(light_visibility(input,sample));}
 }
 return vec3<f32>(1.0);
}",
        })
        .unwrap();
    let material = loop {
        if let Some(m) = r
            .try_create_material(&MaterialDescriptor {
                program: Some(&program),
                double_sided: true,
                ..Default::default()
            })
            .unwrap()
        {
            break m;
        }
        r.test_drain("LC2 material");
    };
    let indices = [0, 1, 2, 0, 2, 3];
    let mut meshes = vec![];
    let mut placements = vec![];
    for (half, translation) in [(2.5, Vec3::ZERO), (0.35, Vec3::new(0.0, 0.0, 1.0))] {
        let vertices = [
            [-half, -half, 0.0],
            [half, -half, 0.0],
            [half, half, 0.0],
            [-half, half, 0.0],
        ]
        .map(|position| Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            uv: [0.0; 2],
            color: [1.0; 4],
        });
        let mesh = loop {
            if let Some(m) = r
                .try_upload_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })
                .unwrap()
            {
                break m;
            }
            r.test_drain("LC2 mesh");
        };
        let instance = loop {
            if let Some(i) = r
                .try_upload_instances(&[Mat4::from_translation(translation)])
                .unwrap()
            {
                break i;
            }
            r.test_drain("LC2 instances");
        };
        meshes.push(mesh);
        placements.push(instance);
    }
    let size = [512, 512];
    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("LC2 penumbra"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let camera = Camera::look_at(
        Vec3::new(0.0, 0.0, 5.0),
        Vec3::ZERO,
        Vec3::Y,
        0.8,
        0.05,
        20.0,
    )
    .unwrap();
    let render = |r: &mut Renderer, placements: &mut Vec<Instances>| {
        let mut draws: Vec<_> = meshes
            .iter()
            .zip(placements)
            .map(|(m, i)| Draw::new(m, i).with_material(&material))
            .collect();
        loop {
            if r.try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                .unwrap()
                .is_some()
            {
                break;
            }
            r.test_drain("LC2 penumbra admission");
        }
    };
    let mut log = serde_json::json!({"path":path});
    for spot in [false, true] {
        let name = if spot { "spot" } else { "point" };
        let mut images = vec![];
        for radius in [0.0, 0.25] {
            let position = Vec3::new(1.0, 0.6, 3.0);
            let local = if spot {
                Light::Spot {
                    position,
                    travel: -position,
                    candela: [10.0; 3],
                    range: Some(10.0),
                    inner: 0.5,
                    outer: 0.8,
                    source_radius: radius,
                    cast_shadows: None,
                }
            } else {
                Light::Point {
                    position,
                    candela: [10.0; 3],
                    range: Some(10.0),
                    source_radius: radius,
                    cast_shadows: None,
                }
            };
            r.set_lighting_and_shadows(
                &Lighting {
                    lights: vec![
                        Light::Directional {
                            travel: -Vec3::Z,
                            lux: [0.1; 3],
                        },
                        local,
                    ],
                    ..Default::default()
                },
                &Shadows {
                    directional_disk_angle: 0.04,
                    requests: vec![
                        ShadowRequest::Directional {
                            split_ends: vec![4.0, 12.0, 40.0],
                        },
                        ShadowRequest::None,
                    ],
                    ..Default::default()
                },
            )
            .unwrap();
            r.set_reconstruction(Reconstruction::default()).unwrap();
            for _ in 0..24 {
                render(&mut r, &mut placements);
            }
            let image = capture(&mut r);
            let mode = if radius == 0.0 { "hard" } else { "soft" };
            write_png(
                &out.join(format!("{path}-{name}-{mode}.png")),
                &image,
                size[0],
                size[1],
            );
            write_presented_png(
                &r.device,
                &r.queue,
                &target,
                &out.join(format!("{path}-{name}-{mode}-present.png")),
            );
            images.push(image);
        }
        let changed = images[0]
            .iter()
            .zip(&images[1])
            .filter(|(a, b)| (*a - *b).abs() > 0.03)
            .count()
            / 3;
        log[name] = serde_json::json!({"changed_pixels":changed});
        assert!(changed > 100, "{name}: source radius changed no penumbra");
    }
    if env("LC3").as_deref() == Some("1") {
        // All 64 sources share a position, so every light is demonstrably blocked in
        // the umbra. The diagnostic's minimum visibility counts leaking lights there.
        let program = r
            .try_create_program(&ProgramDescriptor {
                label: "LC3 leaking-light counter",
                source: "
struct Surface { unused:f32 };
fn surface(input:SurfaceInput)->Surface { return Surface(0.0); }
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {
 var sum=0.0;
 for(var i=0u;i<light_count();i+=1u) {
  let sample=light_sample(i,input);
  if sample.index>0u { sum+=light_visibility(input,sample); }
 }
 return vec3<f32>(sum/64.0);
}",
            })
            .unwrap();
        let diagnostic = loop {
            if let Some(m) = r
                .try_create_material(&MaterialDescriptor {
                    program: Some(&program),
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap()
            {
                break m;
            }
            r.test_drain("LC3 diagnostic material");
        };
        let mut lights = vec![Light::Directional {
            travel: -Vec3::Z,
            lux: [0.1; 3],
        }];
        lights.extend(std::iter::repeat_n(
            Light::Point {
                position: Vec3::new(1.0, 0.6, 3.0),
                candela: [10.0; 3],
                range: Some(10.0),
                source_radius: 0.08,
                cast_shadows: None,
            },
            64,
        ));
        r.set_lighting_and_shadows(
            &Lighting {
                lights,
                ..Default::default()
            },
            &Shadows {
                directional_disk_angle: 0.04,
                requests: std::iter::once(ShadowRequest::Directional {
                    split_ends: vec![4., 12., 40.],
                })
                .chain(std::iter::repeat_n(ShadowRequest::None, 64))
                .collect(),
                ..Default::default()
            },
        )
        .unwrap();
        r.set_reconstruction(Reconstruction::off()).unwrap();
        for (name, budget, legacy) in [("lc2-limit", 4, true), ("lc3", 128, false)] {
            r.set_local_shadows(LocalShadows {
                count: budget,
                ..Default::default()
            })
            .unwrap();
            r.view.clusters.legacy_limit = legacy;
            for _ in 0..24 {
                let mut draws: Vec<_> = meshes
                    .iter()
                    .zip(&mut placements)
                    .map(|(m, i)| Draw::new(m, i).with_material(&diagnostic))
                    .collect();
                loop {
                    if r.try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                        .unwrap()
                        .is_some()
                    {
                        break;
                    }
                    r.test_drain("LC3 diagnostic admission");
                }
            }
            let image = capture(&mut r);
            write_png(
                &out.join(format!("{path}-64-lights-{name}-visibility.png")),
                &image,
                size[0],
                size[1],
            );
            let minimum = image
                .iter()
                .copied()
                .filter(|v| *v > 0.0)
                .fold(f32::INFINITY, f32::min);
            let umbra = ((292usize * size[0] as usize + 195) * 3).min(image.len() - 3);
            let visibility = image[umbra];
            let shadowed = r.light_cluster_stats().shadowed.iter().flatten().count();
            log["leak_fixture"][name] = serde_json::json!({"lights":64,"shadowed":shadowed,
            "minimum_positive_visibility":minimum,
            "umbra_visibility":visibility,"leaking_lights":(visibility*64.0).round()});
            assert_eq!(shadowed, budget.min(64) as usize);
            let expected = if legacy { 60.0 / 64.0 } else { 0.0 };
            assert!(
                (visibility - expected).abs() < 0.01,
                "{name}: visibility {visibility}, expected {expected}"
            );
        }
        if path == "nonrt" {
            // Force a real map-slot replacement; the quarry walk can keep its same
            // 128 lights, so motion images alone do not exercise the fade boundary.
            let lights = |swapped: bool| {
                vec![
                    Light::Directional {
                        travel: -Vec3::Z,
                        lux: [0.1; 3],
                    },
                    Light::Point {
                        position: Vec3::new(1.0, 0.6, 3.0),
                        candela: [if swapped { 1.0 } else { 100.0 }; 3],
                        range: Some(10.0),
                        source_radius: 0.0,
                        cast_shadows: None,
                    },
                    Light::Point {
                        position: Vec3::new(-1.0, 0.6, 3.0),
                        candela: [if swapped { 100.0 } else { 1.0 }; 3],
                        range: Some(10.0),
                        source_radius: 0.0,
                        cast_shadows: None,
                    },
                ]
            };
            let lighting = |swapped| Lighting {
                lights: lights(swapped),
                ..Default::default()
            };
            r.set_local_shadows(LocalShadows {
                count: 1,
                ..Default::default()
            })
            .unwrap();
            r.local_maps.test_reset_selection();
            r.set_lighting_and_shadows(
                &lighting(false),
                &Shadows {
                    directional_disk_angle: 0.04,
                    requests: vec![
                        ShadowRequest::Directional {
                            split_ends: vec![4., 12., 40.],
                        },
                        ShadowRequest::None,
                        ShadowRequest::None,
                    ],
                    ..Default::default()
                },
            )
            .unwrap();
            for _ in 0..16 {
                render(&mut r, &mut placements);
            }
            let sample =
                ((292usize * size[0] as usize + 195) * 3).min((size[0] * size[1] * 3 - 3) as usize);
            let mut curve = vec![capture(&mut r)[sample]];
            r.set_lighting(&lighting(true)).unwrap();
            for frame in 0..16 {
                render(&mut r, &mut placements);
                let image = capture(&mut r);
                curve.push(image[sample]);
                if [0, 3, 7, 15].contains(&frame) {
                    write_png(
                        &out.join(format!("nonrt-fade-{frame:02}.png")),
                        &image,
                        size[0],
                        size[1],
                    );
                }
            }
            let maximum_step = curve
                .windows(2)
                .map(|p| (p[1] - p[0]).abs())
                .fold(0.0f32, f32::max);
            log["map_replacement"] =
                serde_json::json!({"receiver_visibility":curve,"maximum_step":maximum_step});
            assert!(
                curve[0] < 0.01 && curve[16] > 0.99,
                "shadow never replaced: {curve:?}"
            );
            assert!(maximum_step <= 0.126, "shadow popped: {curve:?}");
        }
    }
    std::fs::write(
        out.join(format!("penumbra-{path}.json")),
        serde_json::to_string_pretty(&log).unwrap(),
    )
    .unwrap();
}

// LC2 final-image inspection after PP1: capture the presented target as well as HDR.
// These tests use a linear Unorm target, so encode sRGB for display in the PNG viewer.
fn write_presented_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    path: &std::path::Path,
) {
    let (width, height) = (target.width(), target.height());
    let row = (width * 4).next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LC2 presented image"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for line in mapped.chunks_exact(row as usize) {
        for pixel in line[..width as usize * 4].chunks_exact(4) {
            for &channel in &pixel[..3] {
                let v = f32::from(channel) / 255.0;
                let srgb = if target.format().is_srgb() {
                    v
                } else if v <= 0.0031308 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                };
                bytes.push((srgb * 255.0 + 0.5) as u8);
            }
        }
    }
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path).unwrap()),
        width,
        height,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}
