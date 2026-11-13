//! M7-R1n gates (docs/m7-r1n.md): the non-RT shading modules' software reflection
//! composition (CPU), and the production mirror-camera identity on a device without ray
//! queries (GPU, inside a shared lease).
use super::*;
use crate::{
    Draw, Light, Lighting, Mat4, MaterialDescriptor, MeshData, ShadowRequest, Shadows, Vec3,
    Vertex, ViewMode,
};

const SOURCE: &str = include_str!("../shaders/scene.wgsl");

/// Both non-RT shading heads compose the standard program's reflection trace in software:
/// the R1 hook once, no ray query, valid WGSL that writes SPIR-V. A program without the
/// hooks composes no reflection text, and the RT heads keep R1's ray queries unchanged.
#[test]
fn software_reflection_composition() {
    use crate::program::{Stage, compose};
    let (_, heads) = non_rt_sources(SOURCE).unwrap();
    let call = "rl_reflection_prepare(surface_output,surface_input,hit.batch,id,size);";
    for head in &heads {
        assert!(head.contains(crate::program::SOFTWARE_REFLECTION_GEOMETRY));
        let source = compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false).source;
        assert_eq!(source.matches(call).count(), 1);
        assert!(source.contains("fn rs_trace(") && !source.contains("ray_query"));
        assert!(source.contains("if rl_reflection_hit {return rl_hit_shadow("));
        // U-RTVIS: hits split the PCSS filter taps across the pixel's strata.
        assert!(
            source.contains("min(rl_hit_strata,24u)")
                && source.contains("return soft/f32(visited);")
        );
        // Both paths read the global bands (ReflectionSettings).
        assert!(source.contains("let bands=primary_frame.reflection;"));
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::back::spv::write_vec(
            &module,
            &info,
            &naga::back::spv::Options {
                lang_version: (1, 3),
                ..Default::default()
            },
            Some(&naga::back::spv::PipelineOptions {
                shader_stage: naga::ShaderStage::Compute,
                entry_point: "primary_shade_tiles".into(),
            }),
        )
        .unwrap();
        let toon = compose(
            head,
            include_str!("../shaders/programs/toon.wgsl"),
            Stage::Traced,
            true,
        )
        .source;
        assert!(!toon.contains("rs_trace") && !toon.contains("rl_reflection"));
    }
    for head in shading_heads(SOURCE).unwrap() {
        let source = compose(&head, crate::STANDARD_PROGRAM, Stage::Traced, false).source;
        assert!(source.contains("rayQueryInitialize(&query,") && !source.contains("rs_trace"));
        assert!(source.contains("let bands=primary_frame.reflection;"));
    }
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
    let limits = crate::visibility_limits(wgpu::Limits {
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        max_storage_buffers_per_shader_stage: a.max_storage_buffers_per_shader_stage,
        ..Default::default()
    });
    // SAFETY: the caller acknowledged the experimental wgpu features (R0a's device).
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap();
    assert!(
        !device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
    );
    (device, queue)
}

const WIDTH: u32 = 256;
const HEIGHT: u32 = 144;

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
            renderer.test_drain("R1n identity admission");
        }
        renderer.test_drain("R1n identity frame");
    }
    let mut capture = renderer.try_capture_hdr().unwrap().unwrap();
    renderer.test_drain("R1n identity capture");
    let bytes = capture.try_read().unwrap().unwrap();
    (0..(WIDTH * HEIGHT) as usize * 4)
        .filter(|i| i % 4 != 3)
        .map(|i| crate::decode_f16(u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]])) * 16384.0)
        .collect()
}

/// R1's mirror-camera identity (reflection_tests.rs) on the non-RT path: visibility-buffer
/// primaries, Virtual shadows, software reflection rays. A white metal mirror at y = 0 under
/// a vertex-coloured target, seen from above, must equal the target alone seen from the
/// mirrored camera, horizontally flipped, to 0.002 cd/m² + 0.5%. The hit is shaded by the
/// standard program's own `surface` and `shade` with view vector -d, as the camera hit is.
#[test]
#[ignore = "GPU; shared GPU lease and explicit experimental-feature acknowledgement"]
fn gpu_r1n_mirror_identity() {
    let (device, queue) = device();
    let mut renderer = Renderer::new_with_options(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        crate::Limits::default(),
        crate::RendererOptions::default(),
        SOURCE,
    )
    .unwrap();
    renderer
        .set_primary_visibility(PrimaryVisibility::VisibilityBuffer)
        .unwrap();
    // Exact single frames: TR1 jitters each camera's own lattice.
    renderer
        .set_reconstruction(crate::Reconstruction::off())
        .unwrap();
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
    renderer.set_soft_shadows(SoftShadows::Virtual);
    let upload = |vertices: &[Vertex], indices: &[u32], r: &mut Renderer| loop {
        if let Some(mesh) = r.try_upload_mesh(&MeshData { vertices, indices }).unwrap() {
            break mesh;
        }
        r.test_drain("R1n identity mesh");
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
    assert!(mirror_mesh.reflection.is_some() && target_mesh.reflection.is_some());
    let instances = |r: &mut Renderer| loop {
        if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
            break i;
        }
        r.test_drain("R1n identity instances");
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
        r.test_drain("R1n identity material");
    };
    let mirror = material([1.0; 4], 1.0, 0.0, &mut renderer);
    // Rough enough to stay untraced in both images (hits never trace further).
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
    let mut failed = vec![];
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
                if error > tolerance && failed.len() < 16 {
                    failed.push([x, y, c]);
                }
            }
        }
    }
    let summary = serde_json::json!({"path": "nonrt", "soft_shadows": "Virtual", "pixels": WIDTH * HEIGHT,
        "failed_channels": failures, "maximum_absolute_error": max_absolute, "maximum_tolerance_ratio": worst,
        "reflected_mean": reflected.iter().sum::<f32>() / reflected.len() as f32,
        "camera_mean": camera.iter().sum::<f32>() / camera.len() as f32, "first_failures": failed});
    eprintln!("R1n identity {summary}");
    if let Some(out) = std::env::var_os("RENDERLAB_R1_OUT").map(std::path::PathBuf::from) {
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(
            out.join("identity-nonrt.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        failures, 0,
        "non-RT mirror-camera identity failed, max ratio {worst}"
    );
    // ReflectionSettings::off(): no rays, so the mirror keeps the untraced environment
    // estimate and the target's colours leave it.
    renderer
        .set_reflections(crate::ReflectionSettings::off())
        .unwrap();
    let untraced = render(
        &mut renderer,
        &above,
        &mut [
            Draw::new(&mirror_mesh, &mut mirror_instances).with_material(&mirror),
            Draw::new(&target_mesh, &mut target_instances).with_material(&target),
        ],
    );
    renderer
        .set_reflections(crate::ReflectionSettings::default())
        .unwrap();
    let changed = untraced
        .iter()
        .zip(&reflected)
        .filter(|(a, b)| (*a - *b).abs() > 0.002 + 0.005 * b.abs())
        .count();
    eprintln!(
        "R1n reflections off: {changed} of {} channels changed",
        untraced.len()
    );
    assert!(
        changed * 2 > untraced.len(),
        "ReflectionSettings::off() kept the reflection"
    );
}

/// Deferred reflections (`program::deferred_variant`): the standard program declares
/// `RL_DEFERRED`, the toon program does not, and the standard program's traced module with
/// the deferred inputs validates on the RT and both non-RT shading heads. It clears both
/// outputs where the entry starts and writes them after `shade`, through the fog.
#[test]
fn deferred_reflection_modules_validate_on_every_traced_head() {
    use crate::program::{Stage, compose, deferred_variant};
    let toon = include_str!("../shaders/programs/toon.wgsl");
    assert!(!toon.contains("RL_DEFERRED"));
    let rt = crate::renderer::primary_ray_gpu::shading_heads(SOURCE).unwrap();
    let (_, non_rt) = non_rt_sources(SOURCE).unwrap();
    let mut validated = 0;
    for head in rt.iter().chain(&non_rt) {
        let composed = compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false).source;
        if !composed.contains("rl_specular") {
            continue;
        }
        validated += 1;
        let source =
            deferred_variant(compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false)).source;
        assert!(
            source.contains("rl_deferred_clear(pixel);")
                && source.contains("rl_deferred_store(pixel,input.world);")
        );
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    }
    // The soft RT head and both non-RT heads trace reflections; the hard RT head does not.
    assert!(validated >= 3, "{validated} traced heads validated");
}

/// Deferred reflections under temporal shading: `deferred_variant` drops the program API's
/// report stub, before the program, so the program's range moves with it; temporal shading
/// splits the entry off at the program's end (a stale range cut the next block's first line,
/// and every RENDERLAB_RL_DEFERRED=1 run failed to create its shading module).
#[test]
fn deferred_temporal_shading_keeps_the_program_range_and_validates() {
    use crate::program::{Stage, compose, deferred_variant};
    use crate::renderer::primary_ray_gpu::film_accumulation;
    let soft = &crate::renderer::primary_ray_gpu::shading_heads(SOURCE).unwrap()[1];
    let head = film_accumulation::shading_head(soft).unwrap();
    let composed = deferred_variant(compose(
        &head,
        crate::STANDARD_PROGRAM,
        Stage::Traced,
        false,
    ));
    assert_eq!(
        &composed.source[composed.program.clone()],
        crate::STANDARD_PROGRAM
    );
    let source = film_accumulation::film_shading(composed).source;
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}
