//! M7-R1 production reflection gates (docs/m7-r1.md), GPU, inside a shared lease.
//!
//! The mirror-camera identity on the production renderer: a white metal mirror at y = 0
//! under a vertex-coloured target, seen with traced primaries, must equal the target alone
//! seen from the mirrored camera, horizontally flipped (a reflected look-at basis reverses
//! handedness). The reflection hit runs the standard program's own `surface` and `shade`
//! with view vector -d, as the camera hit does, so the two agree to f32 and binary16
//! storage, plus the DFG table's unit-Fresnel error at roughness 0.
use crate::{
    Camera, Draw, HardwareShadowOptions, Light, Lighting, Limits, Mat4, MaterialDescriptor,
    MeshData, PrimaryVisibility, Renderer, RendererOptions, ShadowRequest, Shadows, SoftShadows,
    Vec3, Vertex, ViewMode,
};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 144;

/// Composition contract (CPU): programs naming neither `reflection_lobe` nor
/// `specular_incident` get no R1 text at all; raster, cell and non-RT traced stages get the
/// zero stub; hardware traced stages trace on that head's own TLAS and hook the entry once.
/// naga validation of the real heads is primary_ray_tests' composition test.
#[test]
fn reflection_composition_contract() {
    use crate::program::{Stage, compose};
    let hardware = "@group(0) @binding(2) var hardware_scene:acceleration_structure;\n";
    let fresh = "@group(0) @binding(2) var fresh_scene:acceleration_structure;\n";
    let toon = include_str!("../shaders/programs/toon.wgsl");
    let standard = crate::STANDARD_PROGRAM;
    let call = "rl_reflection_prepare(surface_output,surface_input,hit.batch,id,size);\n    rl_indirect_prepare(surface_input);";
    for stage in [
        Stage::Raster,
        Stage::Traced,
        Stage::TracedStreamed,
        Stage::Cell,
    ] {
        let source = compose(hardware, toon, stage, true).source;
        assert!(!source.contains("SpecularLobe") && !source.contains("rl_reflection"));
    }
    for stage in [Stage::Raster, Stage::Cell] {
        let source = compose(hardware, standard, stage, false).source;
        assert!(source.contains("fn specular_incident(") && !source.contains(call));
        assert!(!source.contains("rayQueryInitialize(&query,hardware_scene"));
    }
    // Non-RT heads declare no TLAS: the untraced estimate.
    let source = compose("", standard, Stage::Traced, false).source;
    assert!(source.contains("return vec4<f32>(0.0);") && !source.contains(call));
    for (head, scene) in [(hardware, "hardware_scene"), (fresh, "fresh_scene")] {
        for stage in [Stage::Traced, Stage::TracedStreamed] {
            let source = compose(head, standard, stage, false).source;
            assert_eq!(source.matches(call).count(), 1);
            assert!(source.contains(&format!("rayQueryInitialize(&query,{scene},")));
            assert!(!source.contains("RL_SCENE"));
            assert_eq!(
                source
                    .matches("if rl_reflection_hit {return rl_hit_shadow(")
                    .count(),
                1
            );
            // U-RTVIS: hits split the local lights' disk rays across the pixel's strata.
            assert!(source.contains("min(rl_hit_strata,samples)"));
            assert!(source.contains("return lit/f32(visited);"));
        }
    }
    // Both halves are needed to trace; either alone composes the stub.
    let lobe_only = standard.replace(
        "let traced=specular_incident(input);",
        "let traced=vec4<f32>(0.0);",
    );
    assert!(
        !compose(hardware, &lobe_only, Stage::Traced, false)
            .source
            .contains(call)
    );
}

fn device() -> (wgpu::Device, wgpu::Queue) {
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
    let limits = crate::traced_limits(wgpu::Limits {
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        max_storage_buffers_per_shader_stage: a.max_storage_buffers_per_shader_stage,
        max_blas_primitive_count: a.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
        max_acceleration_structures_per_shader_stage: 1,
        ..Default::default()
    });
    // SAFETY: the caller acknowledged the experimental ray-query implementation.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::TRACED_FEATURES
            | wgpu::Features::EXPERIMENTAL_RAY_QUERY,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

fn quad(y: f32, colors: [[f32; 4]; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let corners = [
        [-8.0, y, -8.0],
        [8.0, y, -8.0],
        [8.0, y, 8.0],
        [-8.0, y, 8.0],
    ];
    let vertices = corners
        .iter()
        .zip(colors)
        .map(|(&position, color)| Vertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
            color,
        })
        .collect();
    (vertices, vec![0, 2, 1, 0, 3, 2])
}

/// The lit HDR image in unexposed cd/m² (exposure -14 stops), RGB per pixel.
fn render(renderer: &mut Renderer, camera: &Camera, draws: &mut [Draw<'_>]) -> Vec<f32> {
    let target = crate::tests::target(renderer.device(), WIDTH, HEIGHT);
    for _ in 0..3 {
        loop {
            if renderer
                .try_render_view(&target, camera, draws, ViewMode::Lit)
                .unwrap()
                .is_some()
            {
                break;
            }
            renderer.test_drain("R1 identity admission");
        }
        renderer.test_drain("R1 identity frame");
    }
    let mut capture = renderer.try_capture_hdr().unwrap().unwrap();
    renderer.test_drain("R1 identity capture");
    let bytes = capture.try_read().unwrap().unwrap();
    (0..(WIDTH * HEIGHT) as usize * 4)
        .filter(|i| i % 4 != 3)
        .map(|i| crate::decode_f16(u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]])) * 16384.0)
        .collect()
}

#[test]
#[ignore = "GPU; shared GPU lease and explicit RT acknowledgement"]
fn gpu_r1_mirror_identity() {
    let (device, queue) = device();
    let mut renderer = Renderer::new_with_hardware_shadows(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        Limits::default(),
        RendererOptions::default(),
        HardwareShadowOptions::default(),
        include_str!("../shaders/scene.wgsl"),
    )
    .unwrap();
    renderer
        .set_primary_visibility(PrimaryVisibility::Traced)
        .unwrap();
    // Exact single frames: TR1 (the default since TR1b) jitters each camera's own lattice,
    // which the mirrored camera does not share.
    renderer
        .set_reconstruction(crate::Reconstruction::off())
        .unwrap();
    // A sun from above: the target shadows the mirror and its underside faces away, so
    // the reflected and the mirrored-camera images both exercise the light loop's
    // visibility (hit shadow rays against the frame's own) and agree.
    renderer
        .set_lighting(&Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(0.3, -1.0, 0.2).normalize(),
                lux: [1000.0, 980.0, 940.0],
            }],
            environment: [30.0, 40.0, 50.0],
            exposure_stops: -14.0,
        })
        .unwrap();
    renderer
        .set_shadows(&Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![30.0],
            }],
            resolution: 1024,
            directional_disk_angle: 0.00465,
            ..Default::default()
        })
        .unwrap();
    let upload = |vertices: &[Vertex], indices: &[u32], r: &mut Renderer| loop {
        if let Some(mesh) = r.try_upload_mesh(&MeshData { vertices, indices }).unwrap() {
            break mesh;
        }
        r.test_drain("R1 identity mesh");
    };
    let (mirror_vertices, mirror_indices) = quad(0.0, [[1.0; 4]; 4]);
    let (target_vertices, target_indices) = quad(
        3.0,
        [
            [0.3, 0.2, 0.8, 1.0],
            [0.9, 0.2, 0.1, 1.0],
            [0.1, 0.9, 0.2, 1.0],
            [0.8, 0.5, 0.2, 1.0],
        ],
    );
    let mirror_mesh = upload(&mirror_vertices, &mirror_indices, &mut renderer);
    let target_mesh = upload(&target_vertices, &target_indices, &mut renderer);
    let instances = |r: &mut Renderer| loop {
        if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
            break i;
        }
        r.test_drain("R1 identity instances");
    };
    let mut mirror_instances = instances(&mut renderer);
    let mut target_instances = instances(&mut renderer);
    let material = |base: [f32; 4], metallic: f32, roughness: f32, r: &mut Renderer| loop {
        if let Some(m) = r
            .try_create_material(&MaterialDescriptor {
                base_color: base,
                metallic,
                roughness,
                double_sided: true,
                ..Default::default()
            })
            .unwrap()
        {
            break m;
        }
        r.test_drain("R1 identity material");
    };
    let mirror = material([1.0; 4], 1.0, 0.0, &mut renderer);
    // Rough enough to stay untraced, so its own specular is the same untraced estimate
    // in both images (hits never trace further).
    let target = material([0.7, 0.5, 0.3, 1.0], 0.0, 0.6, &mut renderer);
    let fov = 0.7;
    let above = Camera::look_at(
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::ZERO,
        Vec3::Z,
        fov,
        0.01,
        100.0,
    )
    .unwrap();
    let mirrored = Camera::look_at(
        Vec3::new(0.0, -2.0, 0.0),
        Vec3::new(0.0, 3.0, 0.0),
        Vec3::Z,
        fov,
        0.01,
        100.0,
    )
    .unwrap();
    let out = std::env::var_os("RENDERLAB_R1_OUT").map(std::path::PathBuf::from);
    for mode in [SoftShadows::Exact, SoftShadows::Interactive] {
        renderer.set_soft_shadows(mode);
        let reflected = render(
            &mut renderer,
            &above,
            &mut [
                Draw::new(&mirror_mesh, &mut mirror_instances).with_material(&mirror),
                Draw::new(&target_mesh, &mut target_instances).with_material(&target),
            ],
        );
        let camera = render(
            &mut renderer,
            &mirrored,
            &mut [Draw::new(&target_mesh, &mut target_instances).with_material(&target)],
        );
        let (mut failures, mut worst, mut max_absolute) = (0usize, 0.0f32, 0.0f32);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                for c in 0..3 {
                    let a = reflected[((y * WIDTH + x) * 3 + c) as usize];
                    let b = camera[((y * WIDTH + (WIDTH - 1 - x)) * 3 + c) as usize];
                    let error = (a - b).abs();
                    let tolerance = 0.002 + 0.005 * b.abs();
                    max_absolute = max_absolute.max(error);
                    worst = worst.max(error / tolerance);
                    failures += usize::from(error > tolerance);
                }
            }
        }
        let summary = serde_json::json!({"soft_shadows": format!("{mode:?}"), "pixels": WIDTH * HEIGHT,
            "failed_channels": failures, "maximum_absolute_error": max_absolute, "maximum_tolerance_ratio": worst,
            "precision": "abs(reflected - flipped mirrored camera) <= 0.002 cd/m2 + 0.5% (binary16 storage, DFG unit Fresnel); all pixels and channels",
            "reflected_mean": reflected.iter().sum::<f32>() / reflected.len() as f32,
            "camera_mean": camera.iter().sum::<f32>() / camera.len() as f32});
        eprintln!("R1 identity {summary}");
        if let Some(out) = &out {
            std::fs::create_dir_all(out).unwrap();
            std::fs::write(
                out.join(format!("identity-{mode:?}.json")),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .unwrap();
            for (name, image) in [("reflected", &reflected), ("camera", &camera)] {
                std::fs::write(
                    out.join(format!("identity-{mode:?}-{name}.f32")),
                    image
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            }
        }
        assert_eq!(
            failures, 0,
            "{mode:?}: production mirror-camera identity failed, max ratio {worst}"
        );
    }
}
