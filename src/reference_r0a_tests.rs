//! R0a measurement of the unchanged production renderer on the shared RF scenes.
//! No reflection implementation or reference-estimator change lives here.
use crate::{
    Camera, Draw, HardwareShadowOptions, Light, Lighting, Limits, MaterialDescriptor, MeshData,
    PrimaryVisibility, ProgramDescriptor, Renderer, RendererOptions, ShadowRequest, Shadows,
    SoftShadows, Vec3, Vertex, ViewMode,
};
use std::path::Path;

/// Every camera/sample-count state gets its own independent-AS qualification.
/// The shader is the unchanged tracer or the explicitly hashed R0a diagnostic.
#[test]
#[ignore = "GPU; RENDERLAB_M7_R0A_SCENES and RENDERLAB_M7_REFERENCE_OUT"]
fn gpu_r0a_references() {
    use super::{Settings, render};
    let paths = std::env::var("RENDERLAB_M7_R0A_SCENES").unwrap();
    let samples = std::env::var("RENDERLAB_M7_R0A_SAMPLES").unwrap();
    let samples: Vec<u32> = samples.split(',').map(|v| v.parse().unwrap()).collect();
    let output = super::tests::output();
    let (device, queue) = super::tests::gpu();
    for path in paths.split(';').filter(|p| !p.is_empty()) {
        let path = Path::new(path);
        let bench = super::tests::load_bench(&path.join("scene.json"));
        let name = path.file_name().unwrap().to_str().unwrap();
        for &samples in &samples {
            let root = output.join(samples.to_string());
            std::fs::create_dir_all(&root).unwrap();
            for (pose, view) in &bench.views {
                let case = format!("{name}-{pose}");
                if root.join(format!("{case}.npz")).exists() {
                    continue;
                }
                let settings = Settings {
                    samples,
                    samples_per_dispatch: 1,
                    capture_geometry: true,
                    // R0a consumes the HDR and hit identities. The extra sun-only
                    // visibility diagnostic is separate from the radiance estimator.
                    visibility_samples: 1,
                    smith: bench.smith,
                    max_depth: std::env::var("RENDERLAB_M7_MAX_DEPTH")
                        .map_or(Settings::default().max_depth, |v| v.parse().unwrap()),
                    acceleration_build: match std::env::var("RENDERLAB_M7_R0A_AS_BUILD").as_deref()
                    {
                        Ok("build") => super::AccelerationBuild::FastBuild,
                        Ok("trace") | Err(_) => super::AccelerationBuild::FastTrace,
                        Ok(_) => panic!("unknown R0a AS build preference"),
                    },
                    ..Default::default()
                };
                let image = render(&device, &queue, &bench.scene, view, &settings).unwrap();
                let replay = render(&device, &queue, &bench.scene, view, &settings).unwrap();
                let mut reports = vec![super::tests::replay_report(&image, &replay)];
                let strict = reports.iter().all(|r| r["bit_identical"] == true);
                let noise = (image
                    .standard_error
                    .iter()
                    .flatten()
                    .map(|v| f64::from(*v).powi(2))
                    .sum::<f64>()
                    / (3 * image.mean.len()) as f64)
                    .sqrt();
                let mut fixed = None;
                if !strict
                    && reports
                        .iter()
                        .all(|r| super::tests::qualify_as_replay(r, noise, true))
                {
                    let controls = super::render_repeats(
                        &device,
                        &queue,
                        &bench.scene,
                        view,
                        &settings,
                        2,
                        false,
                    )
                    .unwrap();
                    fixed = Some(super::tests::replay_report(&controls[0], &controls[1]));
                    reports.push(super::tests::replay_report(&image, &controls[0]));
                }
                let fixed_exact = fixed.as_ref().is_some_and(|r| r["bit_identical"] == true);
                let accepted = reports
                    .iter()
                    .all(|r| super::tests::qualify_as_replay(r, noise, fixed_exact));
                super::tests::save(
                    &root,
                    &case,
                    &image,
                    serde_json::json!({"bench": name, "pose":pose, "scene_sha256": bench.sha256,
                        "max_depth":settings.max_depth,"smith":format!("{:?}",settings.smith),
                        "seed":settings.seed,"samples_per_dispatch":settings.samples_per_dispatch,
                        "acceleration_build":format!("{:?}", settings.acceleration_build),
                        "strict_replay":reports[0],"replay_reports":reports,"fixed_resource_replay":fixed,
                        "accepted_for_scoring":accepted,
                        "replay_qualification":{"policy":"bounded-as-v1","accepted":accepted,
                            "label":if strict {"strict"} else if accepted {"bounded-as"} else {"rejected"},
                            "same_resource_bit_identical":fixed_exact,"half_noise_rms":noise},
                        "scope":"R0a per-state independent AS, same seed and dispatch layout"}),
                );
                assert_eq!(image.invalid_pixels, 0);
                assert_eq!(replay.invalid_pixels, 0);
                assert!(accepted, "{case}: bounded-as-v1 rejected");
            }
        }
    }
}

fn device(rt: bool) -> (wgpu::Device, wgpu::Queue) {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let mut features = crate::RECEIVER_FEATURES;
    let mut limits = wgpu::Limits {
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        // The current Virtual visibility ABI binds eleven storage buffers.
        max_storage_buffers_per_shader_stage: a.max_storage_buffers_per_shader_stage,
        ..Default::default()
    };
    if rt {
        features |= crate::TRACED_FEATURES | wgpu::Features::EXPERIMENTAL_RAY_QUERY;
        limits.max_blas_primitive_count = a.max_blas_primitive_count;
        limits.max_blas_geometry_count = 1;
        limits.max_tlas_instance_count = 4096.min(a.max_tlas_instance_count);
        limits.max_acceleration_structures_per_shader_stage = 1;
        limits = crate::traced_limits(limits);
    } else {
        features |= crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES;
        limits = crate::visibility_limits(limits);
    }
    // SAFETY: the caller acknowledged the experimental ray-query implementation.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    let pair = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(
        pair.0
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
        rt
    );
    pair
}

#[test]
#[ignore = "GPU; RENDERLAB_M7_R0A_SCENES, RENDERLAB_M7_REFERENCE_OUT, explicit RT acknowledgement"]
fn gpu_r0a_current() {
    let paths = std::env::var("RENDERLAB_M7_R0A_SCENES").unwrap();
    let output = super::tests::output();
    let rt = std::env::var("RENDERLAB_M7_R0A_PATH").as_deref() == Ok("rt");
    let tr1 = std::env::var("RENDERLAB_R1_TR1").as_deref() == Ok("1");
    let (device, queue) = device(rt);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let source = include_str!("../shaders/scene.wgsl");
    // AA state (docs/iq1-quarry-image.md): 1x, TR1 off, anisotropy 1, the shipping default.
    let mut renderer = if rt {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            format,
            Limits::default(),
            RendererOptions::default(),
            HardwareShadowOptions::default(),
            source,
        )
        .unwrap()
    } else {
        Renderer::new_with_options(
            device,
            queue,
            format,
            Limits::default(),
            RendererOptions::default(),
            source,
        )
        .unwrap()
    };
    renderer
        .set_primary_visibility(if rt {
            PrimaryVisibility::Traced
        } else {
            PrimaryVisibility::VisibilityBuffer
        })
        .unwrap();
    for path in paths.split(';').filter(|p| !p.is_empty()) {
        let path = Path::new(path);
        let bench = super::tests::load_bench(&path.join("scene.json"));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("scene.json")).unwrap()).unwrap();
        let exposure = json["exposure_stops"].as_f64().unwrap() as f32;
        let name = json["bench"].as_str().unwrap();
        renderer
            .set_lighting(&Lighting {
                lights: bench
                    .scene
                    .sun
                    .map(|sun| Light::Directional {
                        travel: -sun.to_light,
                        lux: sun.illuminance,
                    })
                    .into_iter()
                    .collect(),
                environment: bench.scene.sky,
                exposure_stops: exposure,
            })
            .unwrap();
        renderer
            .set_shadows(&Shadows {
                requests: vec![ShadowRequest::Directional {
                    split_ends: vec![30.0],
                }],
                resolution: 4096,
                directional_disk_angle: bench.scene.sun.unwrap().angular_radius,
                ..Default::default()
            })
            .unwrap();
        renderer.set_soft_shadows(if rt {
            SoftShadows::Interactive
        } else {
            SoftShadows::Virtual
        });
        // M7-R1 (docs/m7-r1.md): RENDERLAB_R1_TR1=1 captures each pose after 32 TR1 frames
        // from a cut, TR1's measured settings, so its rotated glossy strata integrate; =0
        // keeps R0a's single-frame 1x protocol (TR1 off); unset, the renderer default.
        match std::env::var("RENDERLAB_R1_TR1").as_deref() {
            Ok("1") => renderer
                .set_reconstruction(crate::Reconstruction::identity())
                .unwrap(),
            Ok("0") => renderer
                .set_reconstruction(crate::Reconstruction::off())
                .unwrap(),
            _ => {}
        }
        // The GI cache is on by default since G2b; R0a's protocol scores it off.
        renderer
            .set_indirect(crate::IndirectSettings::off())
            .unwrap();
        assert!(!renderer.indirect().enabled);
        let toon = renderer
            .try_create_program(&ProgramDescriptor {
                label: "R0a toon witness",
                source: include_str!("../shaders/programs/toon.wgsl"),
            })
            .unwrap();
        let materials: Vec<_> = bench
            .scene
            .materials
            .iter()
            .enumerate()
            .map(|(i, m)| {
                loop {
                    let descriptor = MaterialDescriptor {
                        base_color: [m.base_color[0], m.base_color[1], m.base_color[2], 1.0],
                        metallic: m.metallic,
                        roughness: m.roughness,
                        double_sided: true,
                        // MP1 diagnostic: RENDERLAB_R0A_TOON=standard draws the toon
                        // material on the standard program (a single-program frame).
                        program: (json["materials"][i]["program"] == "toon"
                            && std::env::var("RENDERLAB_R0A_TOON").as_deref() != Ok("standard"))
                        .then_some(&toon),
                        ..Default::default()
                    };
                    if let Some(m) = renderer.try_create_material(&descriptor).unwrap() {
                        break m;
                    }
                    renderer.test_drain("R0a material upload");
                }
            })
            .collect();
        let mut meshes = Vec::new();
        let mut instances = Vec::new();
        for mesh in &bench.scene.meshes {
            let vertices: Vec<_> = mesh
                .positions
                .iter()
                .zip(&mesh.normals)
                .map(|(&position, &normal)| Vertex {
                    position,
                    normal,
                    uv: [0.0; 2],
                    color: [1.0; 4],
                })
                .collect();
            let mesh = loop {
                if let Some(m) = renderer
                    .try_upload_mesh(&MeshData {
                        vertices: &vertices,
                        indices: &mesh.indices,
                    })
                    .unwrap()
                {
                    break m;
                }
                renderer.test_drain("R0a mesh upload");
            };
            meshes.push(mesh);
            instances.push(loop {
                if let Some(i) = renderer
                    .try_upload_instances(&[crate::Mat4::IDENTITY])
                    .unwrap()
                {
                    break i;
                }
                renderer.test_drain("R0a instance upload");
            });
        }
        'poses: for (pose, view) in &bench.views {
            let target = crate::tests::target(renderer.device(), view.width, view.height);
            let camera = Camera::look_at(
                view.open.eye,
                view.open.target,
                Vec3::Y,
                view.fov_y,
                0.01,
                100.0,
            )
            .unwrap();
            if tr1 {
                renderer.cut_reconstruction();
            }
            for _ in 0..if tr1 { 32 } else { 8 } {
                let mut draws: Vec<_> = meshes
                    .iter()
                    .zip(instances.iter_mut())
                    .zip(&bench.scene.instances)
                    .map(|((mesh, instance), placement)| {
                        Draw::new(mesh, instance).with_material(&materials[placement.material])
                    })
                    .collect();
                loop {
                    match renderer.try_render_view(&target, &camera, &mut draws, ViewMode::Lit) {
                        Ok(Some(_)) => break,
                        Ok(None) => renderer.test_drain("R0a frame admission"),
                        Err(crate::Error::InvalidInput(reason))
                            if reason
                                == "traced primary visibility supports one shading program per frame; mixed-program frames are not implemented yet" =>
                        {
                            std::fs::write(
                                output.join(format!("{name}-{pose}.unsupported.json")),
                                serde_json::to_vec_pretty(&serde_json::json!({
                                    "bench": name, "pose": pose, "scene_sha256": bench.sha256,
                                    "status":"unsupported", "reason":reason, "rt":rt,
                                    "scope":"current renderer cannot produce this mixed standard/toon frame; no substituted material or fake image"})).unwrap(),
                            ).unwrap();
                            continue 'poses;
                        }
                        Err(error) => panic!("{name}-{pose}: {error}"),
                    }
                }
                renderer.test_drain("R0a frame");
            }
            let mut capture = renderer.try_capture_hdr().unwrap().unwrap();
            renderer.test_drain("R0a capture");
            let bytes = capture.try_read().unwrap().unwrap();
            let radiance: Vec<f32> = bytes
                .chunks_exact(8)
                .flat_map(|p| {
                    [0, 1, 2].map(|c| {
                        crate::decode_f16(u16::from_le_bytes([p[2 * c], p[2 * c + 1]]))
                            * (-exposure).exp2()
                    })
                })
                .collect();
            assert!(radiance.iter().all(|v| v.is_finite()));
            let file = output.join(format!("{name}-{pose}.f32"));
            std::fs::write(
                &file,
                radiance
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            std::fs::write(
                file.with_extension("json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "bench": name, "pose": pose, "scene_sha256": bench.sha256,
                    "width":view.width,"height":view.height,"exposure_stops":exposure,
                    "rt_device":rt,"primary_visibility":format!("{:?}",renderer.primary_visibility()),
                    "reflection_implementation":"current production default: M7-R1 traced reflections on traced frames, environment only on non-RT; indirect cache remains disabled",
                    "tr1":tr1,
                    "shadow_configuration":if rt {"hardware Interactive disk"} else {"Virtual disk; visibility-buffer non-RT default"},
                    "timing_eligible_for_claim":false
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }
}
