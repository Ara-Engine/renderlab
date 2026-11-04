// These tooling modules are intentionally shared with other independent GPU suites.
#![allow(clippy::duplicate_mod)]
use super::*;
use std::{fs, path::PathBuf};

const SOURCE: &str = include_str!("../shaders/scene.wgsl");
const PIXEL_SEAM: &str = "    let source_front=front==(input.winding>0.0);";

fn reference_source(width: u32, visibility_only: bool) -> String {
    let (before, rest) = SOURCE
        .split_once(crate::hardware_shadows::VISIBILITY_BEGIN)
        .unwrap();
    let (_, after) = rest
        .split_once(crate::hardware_shadows::VISIBILITY_END)
        .unwrap();
    let source = format!(
        "enable wgpu_ray_query;\nconst REFERENCE_WIDTH:u32={width}u;\n{before}\n{}\n{after}",
        include_str!("../support/fresh_fragment_reference.wgsl")
    )
    .replacen(
        PIXEL_SEAM,
        &format!("    reference_pixel=vec2<u32>(input.position.xy);\n{PIXEL_SEAM}"),
        1,
    );
    if visibility_only {
        source.replace("    return vec4<f32>(exposed,1.0);", "    return vec4<f32>(vec3<f32>(hard_shadow(lights[0],input.world,geometric,-lights[0].travel_outer.xyz)),1.0);")
    } else {
        source
    }
}

fn validate_wgsl(source: &str) -> (naga::Module, naga::valid::ModuleInfo) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    (module, info)
}

#[test]
fn fresh_shader_reload_seams_and_cpu_spirv() {
    let source = fresh_shadow_gpu::shader_source(SOURCE).unwrap();
    assert!(fresh_shadow_gpu::shader_source(&SOURCE.replace(PIXEL_SEAM, "")).is_err());
    assert!(fresh_shadow_gpu::shader_source(&format!("{SOURCE}\n{PIXEL_SEAM}")).is_err());
    assert!(fresh_shadow_gpu::shader_source(&SOURCE.replace("let geometric=unit_normal(cross(unit_normal(dpdx(input.world)),unit_normal(dpdy(input.world))));", "let geometric=unit_normal(input.normal);")).is_err());
    assert!(
        fresh_shadow_gpu::shader_source(
            &SOURCE.replace("vec4<f32>(position, 1.0)", "vec4<f32>(position*2.0, 1.0)")
        )
        .is_err()
    );
    assert_eq!(
        source.replace("\r\n", "\n"),
        fresh_shadow_gpu::shader_source(&SOURCE.replace("\r\n", "\n").replace('\n', "\r\n"))
            .unwrap()
            .replace("\r\n", "\n")
    );
    for (name, source) in [
        ("compute", source),
        ("fragment", reference_source(256, false)),
        ("counts", reference_source(256, true)),
    ] {
        let (module, info) = validate_wgsl(&source);
        if name == "compute" {
            let entry = module
                .entry_points
                .iter()
                .find(|e| e.name == "fresh_compute");
            let [x, y] = fresh_shadow_gpu::TILE;
            assert_eq!(entry.unwrap().workgroup_size, [32, 1, 1]);
            assert!(
                source.contains(&format!("const FRESH_TILE=vec2<u32>({x}u,{y}u);")),
                "dispatch tile drift"
            );
        }
        for entry in &module.entry_points {
            let (resolved, resolved_info) = naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                Some((entry.stage, &entry.name)),
                &Default::default(),
            )
            .unwrap();
            let binary = naga::back::spv::write_vec(
                &resolved,
                &resolved_info,
                &naga::back::spv::Options {
                    lang_version: (1, 3),
                    ..Default::default()
                },
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: entry.stage,
                    entry_point: entry.name.clone(),
                }),
            )
            .unwrap();
            if let Some(dir) = std::env::var_os("RENDERLAB_FRESH_SPIRV_DIR") {
                let dir = PathBuf::from(dir);
                fs::create_dir_all(&dir).unwrap();
                fs::write(
                    dir.join(format!("{name}-{}.spv", entry.name)),
                    binary
                        .iter()
                        .flat_map(|w| w.to_le_bytes())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn fresh_target_budget_counts_all_cascades_and_checked_extent() {
    use fresh_shadow_gpu::{admit_targets, target_bytes};
    let mut settings = Shadows {
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        }],
        ..Default::default()
    };
    assert_eq!(target_bytes(1920, 1080).unwrap(), 74_649_600);
    assert!(target_bytes(u32::MAX, u32::MAX).is_err());
    let bytes = target_bytes(1920, 1080).unwrap() + 4 * crate::shadows::VIEW_BYTES + 4;
    settings.max_bytes = Some(bytes);
    assert!(admit_targets(&settings, 1920, 1080).is_ok());
    settings.max_bytes = Some(bytes - 1);
    assert!(admit_targets(&settings, 1920, 1080).is_err());
    settings.max_bytes = None;
    assert!(admit_targets(&settings, 1920, 1080).is_ok());
}

fn renderer(validate: bool) -> Renderer {
    renderer_with_features(validate, wgpu::Features::empty())
}

fn renderer_with_features(validate: bool, extra: wgpu::Features) -> Renderer {
    let startup = Instant::now();
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: if validate {
            wgpu::InstanceFlags::debugging()
        } else {
            wgpu::InstanceFlags::empty()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("fresh_adapter={:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY
            | wgpu::Features::TIMESTAMP_QUERY
            | crate::TRACED_FEATURES
            | extra,
        required_limits: crate::traced_limits(wgpu::Limits {
            max_blas_primitive_count: 2_000_000,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096,
            max_acceleration_structures_per_shader_stage: 1,
            ..Default::default()
        }),
        // The test caller explicitly accepts the experimental device contract above.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .unwrap();
    let device_ms = startup.elapsed().as_secs_f64() * 1e3;
    let pipelines = Instant::now();
    let renderer = Renderer::new_with_hardware_shadows(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Limits {
            max_mesh_bytes: 134217728,
            max_texture_bytes: 134217728,
            ..Default::default()
        },
        RendererOptions {
            samples: SampleCount::One,
            anisotropy_clamp: 16,
        },
        HardwareShadowOptions {
            max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
        },
        SOURCE,
    )
    .unwrap();
    eprintln!(
        "fresh_startup validation={validate} instance_device_ms={device_ms:.6} all_renderer_pipeline_setup_ms={:.6}",
        pipelines.elapsed().as_secs_f64() * 1e3
    );
    renderer
}

#[path = "primary_ray_gpu_tests.rs"]
mod primary_ray_gpu_tests;
#[path = "primary_ray_tests.rs"]
mod primary_ray_tests;
#[path = "quarry_frame_loop_tests.rs"]
mod quarry_frame_loop_tests;

fn target(r: &Renderer, width: u32, height: u32) -> wgpu::Texture {
    r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fresh headless check"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn render(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
) -> FrameReport {
    let start = Instant::now();
    loop {
        if let Some(frame) = r
            .try_render_view(color, camera, draws, ViewMode::Lit)
            .unwrap()
        {
            return frame;
        }
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
}

fn capture(r: &mut Renderer) -> Vec<u8> {
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("fresh HDR readback");
    let bytes = capture.try_read().unwrap().unwrap();
    assert!(
        bytes
            .chunks_exact(2)
            .all(|v| decode_f16(u16::from_le_bytes(v.try_into().unwrap())).is_finite())
    );
    bytes
}

fn reference_pipelines(
    r: &Renderer,
    width: u32,
    visibility_only: bool,
) -> [Owned<wgpu::RenderPipeline>; 2] {
    let scopes = OperationScopes::new(&r.device);
    let reference = Renderer::compile(
        &r.device,
        &r.ledger,
        &r.layout,
        &r.lighting,
        &r.shadows,
        Some((
            r.hardware_shadows.as_ref().unwrap(),
            &reference_source(width, visibility_only),
        )),
        None,
        None,
        r.format,
        SampleCount::One,
        SOURCE,
    )
    .hardware
    .unwrap();
    assert!(scopes.finish().is_none());
    reference
}

fn read_texture(r: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let tight = texture.width() * texture.format().block_copy_size(None).unwrap();
    let row = tight.div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fresh test receiver readback"),
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
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
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
        callback.set(result).unwrap();
    });
    r.queue.submit([encoder.finish()]);
    let start = Instant::now();
    while done.get().is_none() {
        r.device.poll(wgpu::PollType::Poll).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    done.get().unwrap().as_ref().unwrap();
    let view = buffer.get_mapped_range(..).unwrap();
    let bytes = view
        .chunks_exact(row as usize)
        .flat_map(|r| r[..tight as usize].iter().copied())
        .collect();
    drop(view);
    buffer.unmap();
    bytes
}

fn compare(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
    out: &std::path::Path,
    name: &str,
    reference_counts: &mut [Owned<wgpu::RenderPipeline>; 2],
) -> usize {
    r.fresh_fragment_reference = false;
    render(r, color, camera, draws);
    let computed = capture(r);
    let depth = r.test_capture_full(color).1;
    let targets = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let world = read_texture(r, &targets.world.texture);
    let normal = read_texture(r, &targets.normal.texture);
    let counts = read_texture(r, &targets.visibility.texture);
    r.fresh_fragment_reference = true;
    render(r, color, camera, draws);
    let reference = capture(r);
    let reference_depth = r.test_capture_full(color).1;
    fs::write(out.join(format!("{name}-compute.rgba16f")), &computed).unwrap();
    fs::write(out.join(format!("{name}-reference.rgba16f")), &reference).unwrap();
    fs::write(out.join(format!("{name}-depth.f32")), &depth).unwrap();
    assert_eq!(depth, reference_depth, "depth winners differ at {name}");
    let differences = computed
        .chunks_exact(8)
        .zip(reference.chunks_exact(8))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differences, 0,
        "fresh fragment HDR equivalence failed at {name}"
    );
    std::mem::swap(r.pipeline.hardware.as_mut().unwrap(), reference_counts);
    render(r, color, camera, draws);
    let reference_visibility = capture(r);
    std::mem::swap(r.pipeline.hardware.as_mut().unwrap(), reference_counts);
    fs::write(out.join(format!("{name}-visibility.u32")), &counts).unwrap();
    fs::write(
        out.join(format!("{name}-visibility-reference.rgba16f")),
        &reference_visibility,
    )
    .unwrap();
    let mut covered = 0usize;
    let mut partial = 0usize;
    for (i, count) in counts.chunks_exact(4).enumerate() {
        let count = u32::from_le_bytes(count.try_into().unwrap());
        assert!(count <= 256);
        let present =
            f32::from_le_bytes(normal[i * 16 + 12..i * 16 + 16].try_into().unwrap()) != 0.;
        if present {
            covered += 1;
            partial += usize::from(count > 0 && count < 256);
            assert_eq!(
                &world[i * 16 + 12..i * 16 + 16],
                &depth[i * 4..i * 4 + 4],
                "receiver depth differs at {name} pixel {i}"
            );
            let expected = decode_f16(u16::from_le_bytes(
                reference_visibility[i * 8..i * 8 + 2].try_into().unwrap(),
            ));
            assert_eq!(
                count as f32 / 256.,
                expected,
                "visible count differs at {name} pixel {i}"
            );
        } else {
            assert_eq!(count, 256, "uncovered output retained earlier visibility");
        }
    }
    assert!(covered > 0);
    fs::write(out.join(format!("{name}-equivalence.json")),serde_json::to_vec_pretty(&serde_json::json!({"covered":covered,"partial":partial,"hdr_pixel_differences":differences,"count_differences":0,"depth_differences":0})).unwrap()).unwrap();
    r.fresh_fragment_reference = false;
    partial
}

#[path = "../support/validation.rs"]
mod validation;

#[test]
#[ignore = "root must transfer exclusive GPU slot; explicit experimental acknowledgment; headless"]
fn gpu_fresh_thin_contact_moving_view_equivalence() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let out = std::env::var_os("RENDERLAB_FRESH_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "measurements/m6-fresh/fixture".into());
    fs::create_dir_all(&out).unwrap();
    let color = target(&r, 256, 192);
    let vertices =
        [[-8., -8., 0.], [8., -8., 0.], [8., 8., 0.], [-8., 8., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &[0, 1, 2, 0, 2, 3],
        })
        .unwrap()
        .unwrap();
    r.test_drain("fresh fixture mesh");
    let mut receiver = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("fresh fixture receiver");
    let mut blockers = r
        .try_upload_instances(&[
            Mat4::from_scale_rotation_translation(
                Vec3::new(0.00125, 0.2, 1.),
                glam::Quat::IDENTITY,
                Vec3::new(0.6, 0., 0.6),
            ),
            Mat4::from_scale_rotation_translation(
                Vec3::new(0.03, 0.03, 1.),
                glam::Quat::IDENTITY,
                Vec3::new(1., 0.4, 0.03),
            ),
        ])
        .unwrap()
        .unwrap();
    r.test_drain("fresh fixture blocker");
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-1., 0., -1.),
            lux: [1000.; 3],
        }],
        exposure_stops: -8.,
        ..Default::default()
    };
    let shadows = Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![10., 40.],
        }],
        ..Default::default()
    };
    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    let reference = reference_pipelines(&r, color.width(), false);
    let _hard = std::mem::replace(r.pipeline.hardware.as_mut().unwrap(), reference);
    let mut reference_counts = reference_pipelines(&r, color.width(), true);
    let mut observed_partial = 0;
    for (index, x) in [0., 0.01, 0.02, 0., -0.01].into_iter().enumerate() {
        let camera = Camera::look_at(
            Vec3::new(x, 0., 4.),
            Vec3::new(x, 0., 0.),
            Vec3::Y,
            55f32.to_radians(),
            0.1,
            100.,
        )
        .unwrap();
        observed_partial += compare(
            &mut r,
            &color,
            &camera,
            &mut [
                Draw::new(&mesh, &mut receiver),
                Draw::new(&mesh, &mut blockers),
            ],
            &out,
            &format!("pose-{index}"),
            &mut reference_counts,
        );
    }
    assert!(
        observed_partial > 0,
        "fixture did not exercise partial shadows"
    );
    let masks: Vec<_> = (0..5)
        .map(|i| fs::read(out.join(format!("pose-{i}-visibility.u32"))).unwrap())
        .collect();
    assert!(
        masks.windows(2).any(|pair| pair[0] != pair[1]),
        "moving views must exercise changed visibility"
    );
    // Two differently colored primitives at identical depth must retain the first
    // primitive, including within one indexed draw. Reversing their order must matter.
    let tie_vertices: Vec<_> = [[1., 0., 0., 1.], [0., 0., 1., 1.]]
        .into_iter()
        .flat_map(|color| {
            [[-8., -8., 0.], [8., -8., 0.], [0., 8., 0.]].map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color,
            })
        })
        .collect();
    let tie_camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )
    .unwrap();
    let mut tie_images = vec![];
    for (i, indices) in [[0, 1, 2, 3, 4, 5], [3, 4, 5, 0, 1, 2]].iter().enumerate() {
        let tie_mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &tie_vertices,
                indices,
            })
            .unwrap()
            .unwrap();
        r.test_drain("fresh equal-depth mesh");
        compare(
            &mut r,
            &color,
            &tie_camera,
            &mut [Draw::new(&tie_mesh, &mut receiver)],
            &out,
            &format!("tie-{i}"),
            &mut reference_counts,
        );
        tie_images.push(fs::read(out.join(format!("tie-{i}-compute.rgba16f"))).unwrap());
    }
    assert_ne!(
        tie_images[0], tie_images[1],
        "tie fixture did not distinguish the first winner"
    );
    let baseline = r.stats().ownership;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )
    .unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut receiver)],
    );
    r.test_drain("fresh removed blocker");
    assert_eq!(
        baseline.target_bytes.live,
        r.stats().ownership.target_bytes.live
    );
    assert!(r.reload("invalid WGSL").is_err());
    assert!(r.reload(&SOURCE.replace(PIXEL_SEAM, "")).is_err());
    r.set_shadows(&Shadows::default()).unwrap();
    r.collect().unwrap();
    assert!(r.fresh.as_ref().unwrap().targets.is_none());
}

#[allow(dead_code)]
#[path = "../examples/m4/cameras.rs"]
mod cameras;
#[allow(dead_code)]
#[path = "../support/environment_controls.rs"]
mod environment_controls;
#[allow(dead_code)]
#[path = "../support/lighting_settings.rs"]
mod lighting_settings;
#[path = "../support/limits.rs"]
mod policy;
#[allow(dead_code)]
#[path = "../viewer/src/prepared.rs"]
mod prepared;

/// Loads the registered quarry inputs named by `manifest` and uploads them fully.
fn load_quarry(
    manifest: &serde_json::Value,
    r: &mut Renderer,
) -> (prepared::Prepared, cameras::Cameras) {
    let path = |key: &str| PathBuf::from(manifest[key].as_str().unwrap());
    let (policy, _) = policy::Policy::args(vec![
        "--limits".into(),
        path("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let lighting =
        lighting_settings::parse(&fs::read_to_string(path("lighting")).unwrap()).unwrap();
    let shadows = Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        }],
        ..Default::default()
    };
    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    let env = environment_controls::load_environment(
        &path("environment"),
        asset::environment::Limits {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 128 * 1024 * 1024,
        },
    )
    .unwrap();
    let env = r.try_upload_environment(&env).unwrap().unwrap();
    r.set_environment(Some(&env), EnvironmentParameters::default())
        .unwrap();
    r.test_drain("fresh quarry environment");
    let mut asset =
        prepared::Prepared::load_with_limits(&path("asset"), policy.prepared(), 1_000_000).unwrap();
    let started = Instant::now();
    while !asset.ready() {
        asset.advance(r).unwrap();
        r.collect().unwrap();
        assert!(started.elapsed() < Duration::from_secs(120));
    }
    r.test_drain("fresh quarry upload");
    let cameras = cameras::Cameras::load(&path("cameras")).unwrap();
    assert!(cameras.frames.len() >= 2);
    (asset, cameras)
}

/// A compute-only experiment: text edits applied once each to a composed fresh source.
struct Variant {
    name: String,
    edits: Vec<(String, String)>,
    tile: [u32; 2],
}

const RAY_INIT: &str =
    "rayQueryInitialize(&query,fresh_scene,RayDesc(0u,255u,tmin,tmax,origin,direction));";
const ENTRY_TAIL: &str = "fn fresh_compute(@builtin(global_invocation_id) id:vec3<u32>) {";
const STORE: &str = "    textureStore(fresh_output,p,vec4<u32>(visible));";

/// Parses `atom+atom+...` into edits of the frozen baseline body. `base` has no edits.
/// Atoms prefixed `diag` change results and only locate the limiter.
fn variant(spec: &str) -> Variant {
    let mut edits: Vec<(String, String)> = vec![];
    let mut tile = [8, 8];
    let mut edit = |a: &str, b: String| edits.push((a.to_owned(), b));
    for atom in spec.split('+').filter(|a| *a != "base") {
        let flags = |f: &str| RAY_INIT.replace("RayDesc(0u,", &format!("RayDesc({f},"));
        if let Some(size) = atom.strip_prefix("wg") {
            let (x, y) = size.split_once('x').unwrap();
            tile = [x.parse().unwrap(), y.parse().unwrap()];
            edit("@workgroup_size(8,8)", format!("@workgroup_size({x},{y})"));
        } else if let Some(lanes) = atom.strip_prefix("ppx") {
            // One 32-lane group: `lanes` lanes share each pixel's 256 rays, summed exactly.
            let lanes: u32 = lanes.parse().unwrap();
            let pixels = 32 / lanes;
            let (tw, th) = match pixels {
                1 => (1, 1),
                2 => (2, 1),
                4 => (2, 2),
                8 => (4, 2),
                _ => panic!("ppx lanes must be 4, 8, 16 or 32"),
            };
            tile = [tw, th];
            edit(
                "fn fresh_estimate(world:vec3<f32>, geometric:vec3<f32>, pixel:vec2<u32>, width:u32) -> u32 {",
                "fn fresh_estimate(world:vec3<f32>, geometric:vec3<f32>, pixel:vec2<u32>, width:u32, sub:u32) -> u32 {".into(),
            );
            edit(
                "if !found {return 256u;}",
                "if !found {return 0xffffffffu;}".into(),
            );
            edit(
                "projected.z>1.0 {return 256u;}",
                "projected.z>1.0 {return 0xffffffffu;}".into(),
            );
            edit(
                "for(var j=0u;j<256u;j+=1u) {\n        let direction",
                format!("for(var j=sub;j<256u;j+={lanes}u) {{\n        let direction"),
            );
            edit(
                "@compute @workgroup_size(8,8) fn fresh_compute(@builtin(global_invocation_id) id:vec3<u32>) {\n    let size=textureDimensions(fresh_world);\n    if any(id.xy>=size) {return;}\n    let p=vec2<i32>(id.xy);\n    let normal=textureLoad(fresh_normal,p,0);\n    var visible=256u;\n    if normal.w!=0.0 {\n        visible=fresh_estimate(textureLoad(fresh_world,p,0).xyz,normal.xyz,id.xy,size.x);\n    }\n",
                format!(
                    "var<workgroup> fresh_sum:array<atomic<u32>,{pixels}>;\n@compute @workgroup_size(32) fn fresh_compute(@builtin(workgroup_id) w:vec3<u32>,@builtin(local_invocation_index) li:u32) {{\n    let size=textureDimensions(fresh_world);\n    let slot=li/{lanes}u;\n    let id=vec2<u32>(w.x*{tw}u+slot%{tw}u,w.y*{th}u+slot/{tw}u);\n    let inside=all(id<size);\n    let p=vec2<i32>(id);\n    var partial=0xffffffffu;\n    if inside {{\n        let normal=textureLoad(fresh_normal,p,0);\n        if normal.w!=0.0 {{\n            partial=fresh_estimate(textureLoad(fresh_world,p,0).xyz,normal.xyz,id,size.x,li%{lanes}u);\n        }}\n    }}\n    atomicAdd(&fresh_sum[slot],select(partial,0u,partial==0xffffffffu));\n    workgroupBarrier();\n    if !inside || li%{lanes}u!=0u {{return;}}\n    // Every lane of a pixel takes the same early exits, so lane zero's sentinel is the pixel's.\n    let visible=select(atomicLoad(&fresh_sum[slot]),256u,partial==0xffffffffu);\n"
                ),
            );
        } else if let Some(kib) = atom.strip_prefix("diagpad") {
            // Workgroup memory caps resident groups per SM: an occupancy probe.
            let entries = kib.parse::<u32>().unwrap() * 64;
            edit(
                "fn fresh_hash(input:u32) -> u32 {",
                format!(
                    "var<workgroup> fresh_pad:array<vec4<u32>,{entries}>;\nfn fresh_hash(input:u32) -> u32 {{"
                ),
            );
            edit(
                ENTRY_TAIL,
                format!(
                    "fn fresh_compute(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) li:u32) {{\n    fresh_pad[li%{entries}u]=vec4<u32>(id.xyxy);"
                ),
            );
            edit(
                STORE,
                format!(
                    "    if id.x==0xffffffffu {{visible+=fresh_pad[(li+1u)%{entries}u].x;}}\n{STORE}"
                ),
            );
        } else {
            match atom {
                // First-hit termination is exact only when every hit has t >= tmin > 0;
                // a closest hit at t == 0 counts as visible in the reference.
                "fh" => edit(RAY_INIT, flags("select(1u,5u,tmin>0.0)")),
                "fh0" => edit(RAY_INIT, flags("select(0u,4u,tmin>0.0)")),
                // Each lane starts at the index whose R2 point lies nearest the shared
                // origin, so lanes trace similar directions at the same step. The traced
                // set is unchanged and the integer sum is order independent.
                "rot" => {
                    edit(
                        "    var visible=0u;",
                        "    var offset=0u;\n    var best=4.0;\n    for(var j=0u;j<256u;j+=1u) {\n        let r=fract(vec2<f32>(f32(j))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);\n        let d=min(r,vec2<f32>(1.0)-r);\n        if dot(d,d)<best {best=dot(d,d);offset=j;}\n    }\n    var visible=0u;".into(),
                    );
                    edit(
                        "fresh_direction(j,shift,",
                        "fresh_direction((j+offset)&255u,shift,".into(),
                    );
                }
                "diagshift0" => edit(
                    "let shift=vec2<f32>(f32(seed>>8u),f32(fresh_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);",
                    "let shift=vec2<f32>(0.0)*f32(seed);".into(),
                ),
                "diagalu" => edit("while rayQueryProceed(&query) {}", String::new()),
                "diagrays128" => edit(
                    "j<256u;j+=1u) {\n        let direction",
                    "j<128u;j+=1u) {\n        let direction".into(),
                ),
                "diagshort" => edit(
                    "            if tmax>tmin {",
                    "            tmax=min(tmax,tmin+0.25);\n            if tmax>tmin {".into(),
                ),
                _ => panic!("unknown variant atom {atom}"),
            }
        }
    }
    Variant {
        name: spec.into(),
        edits,
        tile,
    }
}

const SCREEN_BASELINE: &str = include_str!("../support/fresh_screen_baseline.wgsl");

#[test]
fn fresh_screen_atoms_compose_valid_wgsl() {
    let base = fresh_shadow_gpu::compose(SOURCE, SCREEN_BASELINE).unwrap();
    for spec in [
        "base",
        "wg8x4+fh+rot",
        "fh0",
        "ppx32+fh",
        "ppx8",
        "wg8x4+diagpad8",
        "diagshift0",
        "diagalu",
        "diagrays128",
        "diagshort",
    ] {
        let v = variant(spec);
        let (module, _) = validate_wgsl(&variant_source(&base, &v));
        let entry = module
            .entry_points
            .iter()
            .find(|e| e.name == "fresh_compute");
        let size = entry.unwrap().workgroup_size;
        // ppx groups are one warp; `tile` is their receiver pixel extent.
        assert!(
            size == [v.tile[0], v.tile[1], 1] || size == [32, 1, 1],
            "{spec}"
        );
    }
}

fn variant_source(base: &str, v: &Variant) -> String {
    let mut source = base.to_owned();
    for (find, replace) in &v.edits {
        assert_eq!(
            source.matches(find.as_str()).count(),
            1,
            "{}: {find}",
            v.name
        );
        source = source.replacen(find.as_str(), replace, 1);
    }
    source
}

fn wait_map(r: &Renderer, buffer: &wgpu::Buffer) -> Vec<u8> {
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        callback.set(result).unwrap();
    });
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    done.get().unwrap().as_ref().unwrap();
    let bytes = buffer.get_mapped_range(..).unwrap().to_vec();
    buffer.unmap();
    bytes
}

/// Lower-bound screen: unchanged indexed draws/depth/color with a trivial fragment.
/// This diagnostic deliberately changes HDR; it is never a production pipeline.
#[test]
#[ignore = "GPU slot, experimental acknowledgment and external RENDERLAB_FRESH_MANIFEST; headless"]
fn gpu_fresh_geometry_cost_screen() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_FRESH_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let color = target(&r, 1920, 1080);
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    let source = fresh_shadow_gpu::shader_source(SOURCE).unwrap();
    let start = source.find("@fragment fn fs_lit(").unwrap();
    let end = source[start..].find("@vertex fn vs_present(").unwrap() + start;
    // Consume every varying so dead-code elimination cannot turn this into a
    // position-only vertex shader. The optional constant is a separate lower bound.
    let body = if manifest["constant"].as_bool().unwrap_or(false) {
        "return vec4<f32>(0.0,0.0,0.0,1.0);"
    } else {
        "let value=dot(input.color,vec4<f32>(0.1))+dot(input.uv,vec2<f32>(0.1))+dot(input.normal,vec3<f32>(0.1))+input.winding+dot(input.world,vec3<f32>(0.1))+dot(input.tangent,vec3<f32>(0.1))+input.handedness+f32(front); return vec4<f32>(value,value,value,1.0);"
    };
    let trivial = format!(
        "{}@fragment fn fs_lit(input:VertexOutput, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {{ {body} }}\n{}",
        &source[..start],
        &source[end..]
    );
    validate_wgsl(&trivial);
    let scopes = OperationScopes::new(&r.device);
    let mut probe = r
        .fresh
        .as_ref()
        .unwrap()
        .compile(&r.device, &r.ledger, &trivial)
        .shade;
    assert!(scopes.finish().is_none());
    let warmup = manifest["warmup_frames"].as_u64().unwrap_or(12) as usize;
    let frames = manifest["frames"].as_u64().unwrap_or(48) as usize;
    let mut pending = std::collections::HashMap::new();
    let mut samples = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<_, (bool, usize)>| {
            for gpu in r.poll().unwrap().gpu {
                if let Some((trivial, pose)) = pending.remove(&gpu.frame.0) {
                    samples.push(serde_json::json!({"trivial":trivial,"pose":pose,
                    "receiver_ms":ms(gpu.receiver_pass.unwrap()),"compute_ms":ms(gpu.visibility_compute.unwrap()),
                    "shading_ms":ms(gpu.mesh_pass),"tone_ms":ms(gpu.present_pass.unwrap()),"span_ms":ms(gpu.render_span)}));
                }
            }
        };
    for frame in 0..warmup + frames {
        let k = frame.saturating_sub(warmup);
        let trivial = k % 2 == 1;
        let pose = (k / 2) % 2;
        if trivial {
            std::mem::swap(&mut r.pipeline.fresh.as_mut().unwrap().shade, &mut probe);
        }
        let report = render(
            &mut r,
            &color,
            &poses[pose],
            &mut asset.draws(Visibility::Frustum),
        );
        if trivial {
            std::mem::swap(&mut r.pipeline.fresh.as_mut().unwrap().shade, &mut probe);
        }
        if frame >= warmup {
            pending.insert(report.frame.0, (trivial, pose));
        }
        collect(&mut r, &mut pending);
    }
    r.test_drain("paired geometry-cost frames");
    collect(&mut r, &mut pending);
    assert!(pending.is_empty(), "missing geometry-cost samples");
    fs::write(
        out.join("geometry-cost.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "validation":validate,"warmup_frames":warmup,"samples":samples,
            "diagnostic_changes_hdr":true,"submitted_triangles_per_pass":13_034_903,
        }))
        .unwrap(),
    )
    .unwrap();
}

/// Paired compute-only screen on the real quarry receivers, then complete frames.
/// Every variant's visibility is compared bit-for-bit with the frozen baseline first.
#[test]
#[ignore = "GPU slot, experimental acknowledgment and external RENDERLAB_FRESH_MANIFEST; headless"]
fn gpu_fresh_compute_cost_screen() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_FRESH_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let reps = manifest["reps"].as_u64().unwrap_or(6) as usize;
    let warmup = manifest["warmup_reps"].as_u64().unwrap_or(1) as usize;
    let warmup_frames = manifest["warmup_frames"].as_u64().unwrap_or(0) as usize;
    let frames = manifest["frames"].as_u64().unwrap_or(0) as usize;
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let color = target(&r, 1920, 1080);
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    render(
        &mut r,
        &color,
        &poses[1],
        &mut asset.draws(Visibility::Frustum),
    );
    r.test_drain("fresh screen receivers");
    let base = fresh_shadow_gpu::compose(SOURCE, SCREEN_BASELINE).unwrap();
    let mut variants: Vec<_> = std::iter::once("base")
        .chain(
            manifest["variants"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap()),
        )
        .map(variant)
        .collect();
    variants.push(Variant {
        name: "production".into(),
        edits: vec![],
        tile: fresh_shadow_gpu::TILE,
    });
    let production = fresh_shadow_gpu::shader_source(SOURCE).unwrap();
    let mut pipelines: Vec<_> = variants
        .iter()
        .map(|v| {
            let source = if v.name == "production" {
                production.clone()
            } else {
                variant_source(&base, v)
            };
            let scopes = OperationScopes::new(&r.device);
            let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&v.name),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = r.fresh.as_ref().unwrap().compute_pipeline(
                &r.device,
                &r.ledger,
                &module,
                "fresh_compute",
                &[],
            );
            assert!(scopes.finish().is_none(), "{} failed to compile", v.name);
            pipeline
        })
        .collect();
    // Exactness first: every covered and uncovered output count, bit-for-bit.
    let visibility = |r: &Renderer, pipeline: &wgpu::ComputePipeline, tile| {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        r.encode_fresh_compute(&mut encoder, pipeline, tile, None);
        r.queue.submit([encoder.finish()]);
        let texture = &r
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture;
        read_texture(r, texture)
    };
    let reference = visibility(&r, &pipelines[0], variants[0].tile);
    let mut differences = vec![];
    for (v, p) in variants.iter().zip(&pipelines) {
        let counts = visibility(&r, p, v.tile);
        let differing = counts
            .chunks_exact(4)
            .zip(reference.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        let rays: u64 = counts
            .chunks_exact(4)
            .map(|c| u64::from(u32::from_le_bytes(c.try_into().unwrap())))
            .sum();
        eprintln!(
            "fresh_screen_exact variant={} differing_pixels={differing} visible_sum={rays}",
            v.name
        );
        differences.push(differing);
    }
    // Interleaved compute-only timing; the order rotates every repetition.
    let n = variants.len();
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("fresh screen"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * n as u32,
    });
    let buffer = |usage| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fresh screen timestamps"),
            size: 16 * n as u64,
            usage,
            mapped_at_creation: false,
        })
    };
    let resolve = buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
    let read = buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
    let period = f64::from(r.queue.get_timestamp_period());
    let mut samples = vec![vec![]; n];
    for rep in 0..reps + warmup {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        for k in 0..n {
            let i = (k + rep) % n;
            r.encode_fresh_compute(
                &mut encoder,
                &pipelines[i],
                variants[i].tile,
                Some(wgpu::ComputePassTimestampWrites {
                    query_set: &queries,
                    beginning_of_pass_write_index: Some(2 * i as u32),
                    end_of_pass_write_index: Some(2 * i as u32 + 1),
                }),
            );
        }
        encoder.resolve_query_set(&queries, 0..2 * n as u32, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16 * n as u64);
        r.queue.submit([encoder.finish()]);
        let stamps: Vec<u64> = wait_map(&r, &read)
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        // Repetition zero warms clocks and caches and is reported separately.
        for i in 0..n {
            let ms = (stamps[2 * i + 1] - stamps[2 * i]) as f64 * period * 1e-6;
            if rep >= warmup {
                samples[i].push(ms);
            } else {
                eprintln!(
                    "fresh_screen_warmup variant={} ms={ms:.4}",
                    variants[i].name
                );
            }
        }
    }
    let summary: Vec<_> = variants
        .iter()
        .zip(&samples)
        .zip(&differences)
        .map(|((v, s), d)| {
            let mut sorted = s.clone();
            sorted.sort_by(f64::total_cmp);
            let median = sorted[sorted.len() / 2];
            eprintln!(
                "fresh_screen variant={} median_ms={median:.4} min_ms={:.4} max_ms={:.4} differing_pixels={d}",
                v.name,
                sorted[0],
                sorted[sorted.len() - 1]
            );
            serde_json::json!({"variant": v.name, "tile": v.tile, "differing_pixels": d,
                "median_ms": median, "samples_ms": s})
        })
        .collect();
    // Complete frames: named compute variants take turns on each pose, then the pose alternates.
    let frame_variants: Vec<usize> = manifest["frame_variants"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|name| {
                    variants
                        .iter()
                        .position(|v| v.name == name.as_str().unwrap())
                        .unwrap()
                })
                .collect()
        })
        .unwrap_or_else(|| vec![n - 1]);
    // Pipelined like an application: no per-frame drain, samples matched by frame id.
    let mut pending = std::collections::HashMap::new();
    let mut complete = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<_, (usize, usize)>| {
            for gpu in r.poll().unwrap().gpu {
                if let Some((i, pose)) = pending.remove(&gpu.frame.0) {
                    complete.push(serde_json::json!({"variant": variants[i].name, "pose": pose,
                    "receiver_ms": ms(gpu.receiver_pass.unwrap()), "compute_ms": ms(gpu.visibility_compute.unwrap()),
                    "shading_ms": ms(gpu.mesh_pass), "tone_ms": ms(gpu.present_pass.unwrap()), "span_ms": ms(gpu.render_span)}));
                }
            }
        };
    for frame in 0..warmup_frames + frames {
        let k = frame.saturating_sub(warmup_frames);
        let i = frame_variants[k % frame_variants.len()];
        let pose = (k / frame_variants.len()) % 2;
        let swap = |r: &mut Renderer, p: &mut Owned<wgpu::ComputePipeline>, tile: &mut [u32; 2]| {
            let fresh = r.pipeline.fresh.as_mut().unwrap();
            std::mem::swap(&mut fresh.compute, p);
            std::mem::swap(&mut fresh.compute_tile, tile);
        };
        let mut tile = variants[i].tile;
        swap(&mut r, &mut pipelines[i], &mut tile);
        let report = render(
            &mut r,
            &color,
            &poses[pose],
            &mut asset.draws(Visibility::Frustum),
        );
        swap(&mut r, &mut pipelines[i], &mut tile);
        if frame >= warmup_frames {
            pending.insert(report.frame.0, (i, pose));
        }
        collect(&mut r, &mut pending);
    }
    r.test_drain("fresh screen frames");
    collect(&mut r, &mut pending);
    assert!(
        pending.is_empty(),
        "missing GPU samples for {} frames",
        pending.len()
    );
    eprintln!(
        "fresh_screen_complete {}",
        serde_json::to_string(&complete).unwrap()
    );
    fs::write(
        out.join("screen.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"validation": validate, "reps": reps,
            "warmup_reps": warmup, "warmup_frames": warmup_frames, "variants": summary, "complete_frames": complete}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "root GPU slot, experimental acknowledgment and external RENDERLAB_FRESH_MANIFEST; headless"]
fn gpu_fresh_quarry_equivalence_and_complete_frame() {
    let startup = Instant::now();
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_FRESH_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    // Timing is a separate process after the unchanged validation-on quality gates.
    let timing_only = manifest["timing_only"].as_bool().unwrap_or(false);
    let validate = !timing_only;
    let _validation = validation::begin(validate);
    let path = |key: &str| PathBuf::from(manifest[key].as_str().unwrap());
    let out = path("output");
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let prepared_ready_ms = startup.elapsed().as_secs_f64() * 1e3;
    let color = target(&r, 1920, 1080);
    if timing_only {
        let cold_start = Instant::now();
        let cold = render(
            &mut r,
            &color,
            &cameras.frames[0].camera().unwrap(),
            &mut asset.draws(Visibility::Frustum),
        );
        r.test_drain("fresh startup first image completion proxy");
        let cold_completion_ms = cold_start.elapsed().as_secs_f64() * 1e3;
        let startup_completion_ms = startup.elapsed().as_secs_f64() * 1e3;
        let cold_gpu = r.poll().unwrap().gpu;
        fs::write(
            out.join("startup.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "validation": validate, "prepared_ready_ms": prepared_ready_ms,
                "startup_to_first_completed_image_proxy_ms": startup_completion_ms,
                "cold_frame_cpu_to_completion_ms": cold_completion_ms,
                "cold_frame": format!("{cold:?}"), "cold_gpu": format!("{cold_gpu:?}")
            }))
            .unwrap(),
        )
        .unwrap();
    } else {
        let reference = reference_pipelines(&r, color.width(), false);
        let _hard = std::mem::replace(r.pipeline.hardware.as_mut().unwrap(), reference);
        let mut reference_counts = reference_pipelines(&r, color.width(), true);
        // Stop at the first quality failure. Forward/return views expose stale visibility.
        for (index, pose) in [0, 1, 0].into_iter().enumerate() {
            compare(
                &mut r,
                &color,
                &cameras.frames[pose].camera().unwrap(),
                &mut asset.draws(Visibility::Frustum),
                &out,
                &format!("pose-{index}"),
                &mut reference_counts,
            );
        }
    }
    r.poll().unwrap();
    let before = r.stats();
    let started = Instant::now();
    let mut draws = asset.draws(Visibility::Frustum);
    let scene_draws = started.elapsed();
    let report = render(
        &mut r,
        &color,
        &cameras.frames[1].camera().unwrap(),
        &mut draws,
    );
    let admission = started.elapsed();
    r.test_drain("fresh complete frame cost");
    let completion = started.elapsed();
    let gpu = r
        .poll()
        .unwrap()
        .gpu
        .into_iter()
        .find(|v| v.frame == report.frame)
        .unwrap();
    assert!(gpu.receiver_pass.is_some() && gpu.visibility_compute.is_some());
    assert_eq!(
        before.ownership.target_creations,
        r.stats().ownership.target_creations,
        "steady frame recreated targets"
    );
    fs::write(out.join("complete-frame.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "validation":validate,"samples":1,"width":1920,"height":1080,"disk_angle":0.04,"queries_per_covered_receiver":256,
        "cpu_scene_draws_ms":scene_draws.as_secs_f64()*1e3,
        "cpu_admission_ms":admission.as_secs_f64()*1e3,"cpu_submission_to_observed_completion_ms":completion.as_secs_f64()*1e3,
        "receiver_ms":gpu.receiver_pass.unwrap().as_secs_f64()*1e3,"compute_ms":gpu.visibility_compute.unwrap().as_secs_f64()*1e3,
        "shading_ms":gpu.mesh_pass.as_secs_f64()*1e3,"present_shader_ms":gpu.present_pass.unwrap().as_secs_f64()*1e3,"render_span_ms":gpu.render_span.as_secs_f64()*1e3,
        "fresh_target_logical_bytes":fresh_shadow_gpu::target_bytes(1920,1080).unwrap(),"frame":format!("{report:?}"),"before":format!("{before:?}"),"after":format!("{:?}",r.stats())
    })).unwrap()).unwrap();
    if timing_only {
        let expected = fs::read(path("expected_hdr")).unwrap();
        let differences = |actual: &[u8]| {
            assert_eq!(actual.len(), expected.len());
            actual
                .chunks_exact(8)
                .zip(expected.chunks_exact(8))
                .filter(|(a, b)| a != b)
                .count()
        };
        assert_eq!(
            differences(&capture(&mut r)),
            0,
            "timing image differs from validated reference"
        );
        let reload_start = Instant::now();
        let reload = r.reload(SOURCE).unwrap();
        render(
            &mut r,
            &color,
            &cameras.frames[1].camera().unwrap(),
            &mut draws,
        );
        r.test_drain("fresh successful reload completed image proxy");
        let reload_completion_ms = reload_start.elapsed().as_secs_f64() * 1e3;
        assert_eq!(
            differences(&capture(&mut r)),
            0,
            "reloaded image differs from validated reference"
        );
        fs::write(out.join("reload.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "validation": validate, "all_renderer_pipeline_reload_compile_ms": reload.compile.as_secs_f64()*1e3,
            "reload_to_first_completed_image_proxy_ms": reload_completion_ms,
            "hdr_pixel_differences": 0
        })).unwrap()).unwrap();
    }
}
