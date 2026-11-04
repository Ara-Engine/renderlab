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
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        for entry in &module.entry_points {
            let binary = naga::back::spv::write_vec(
                &module,
                &info,
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
        required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: wgpu::Limits {
            max_blas_primitive_count: 2_000_000,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096,
            max_acceleration_structures_per_shader_stage: 1,
            ..Default::default()
        },
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
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
        assert!(started.elapsed() < Duration::from_secs(120));
    }
    r.test_drain("fresh quarry upload");
    let prepared_ready_ms = startup.elapsed().as_secs_f64() * 1e3;
    let cameras = cameras::Cameras::load(&path("cameras")).unwrap();
    assert!(cameras.frames.len() >= 2);
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
