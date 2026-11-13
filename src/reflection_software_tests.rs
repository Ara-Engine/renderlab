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

/// The standard program's deferred traced module on `head`: `compose` already makes it while
/// RENDERLAB_RL_DEFERRED=1, so `deferred_variant` applies only when the switch is off.
fn deferred_standard(head: &str) -> crate::program::Composed {
    use crate::program::{Stage, compose, deferred_variant};
    let composed = compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false);
    if crate::program::rl_deferred_for(head) {
        composed
    } else {
        deferred_variant(composed)
    }
}

/// Deferred reflections (`program::deferred_variant`): the standard program declares
/// `RL_DEFERRED`, the toon program does not, and the standard program's traced module with
/// the deferred inputs validates on the RT and both non-RT shading heads. It clears both
/// outputs where the entry starts and writes them after `shade`, through the fog.
#[test]
fn deferred_reflection_modules_validate_on_every_traced_head() {
    use crate::program::{Stage, compose};
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
        let source = deferred_standard(head).source;
        assert!(
            source.contains("rl_deferred_clear(pixel);")
                && source.contains("rl_deferred_store(pixel,input.world);")
        );
        // Under the pass (`pass_block`) the composite takes this program's glossy lobes; a
        // program that does not report keeps its inline estimate. Without the switches the
        // module keeps reflections.wgsl's own lobe store and names nothing of the composite, so
        // the default's text is main's.
        let passed = crate::program::pass_block(deferred_standard(head)).source;
        assert!(passed.contains("const RL_DEFERRED_PROGRAM:bool=true;"));
        assert!(passed.contains(
            "rl_lobe_store(pixel,origin,n,v,side,alpha,width,pixel_angle+curvature,count);"
        ));
        if !crate::program::rl_deferred_for(head) && !crate::program::rl_pass_for(head) {
            let plain = crate::program::pass_block(compose(
                head,
                crate::STANDARD_PROGRAM,
                Stage::Traced,
                false,
            ));
            assert!(
                plain
                    .source
                    .contains("const RL_DEFERRED_PROGRAM:bool=false;")
            );
            assert!(composed.contains("    if RL_PASS && !mirror {rl_lobe_store(pixel,origin,n,v,side,alpha,width,pixel_angle+curvature);}\n"));
            assert!(
                !composed.contains("rl_composite") && !composed.contains("RL_DEFERRED_PROGRAM")
            );
        }
        for source in [&source, &passed] {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        }
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
    use crate::renderer::primary_ray_gpu::film_accumulation;
    let soft = &crate::renderer::primary_ray_gpu::shading_heads(SOURCE).unwrap()[1];
    let head = film_accumulation::shading_head(soft).unwrap();
    let composed = deferred_standard(&head);
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

/// Glass hits keep their reflections inline: their module (TP2's `tp_reflect`) drops the camera
/// entry, so it never takes the deferred variant, whatever RENDERLAB_RL_DEFERRED says.
#[test]
fn glass_reflection_module_stays_inline() {
    use crate::renderer::transparency_gpu::reflection;
    let head = crate::renderer::primary_ray_gpu::transparent_reflection_head(SOURCE).unwrap();
    let source = reflection::source(&head, crate::STANDARD_PROGRAM, false);
    assert!(!source.contains("RL_DEFERRED_PROGRAM") && !source.contains("rl_composite"));
    assert!(source.contains("fn reflection_report("));
    assert!(!source.contains("rl_deferred_store"));
}

/// The S4a hit reuse seam in `rl_trace_pixels` (the Mac's composition replaces it under
/// RENDERLAB_RL_HIT_REUSE): once, inside the strata loop, right before the H append.
#[test]
fn trace_hit_reuse_seam_stays_before_the_h_append() {
    let trace = include_str!("../shaders/reflections_trace.wgsl");
    let seam = "        // ---- rl trace hit reuse seam ----\n        let index=atomicAdd(&rl_hits.count,1u);";
    assert_eq!(
        trace
            .matches("// ---- rl trace hit reuse seam ----")
            .count(),
        1
    );
    assert_eq!(trace.matches(seam).count(), 1);
}

/// The non-RT default belongs to the renderer whose device runs the pass: its heads carry
/// `PASS_HEAD_MARK`, and only modules composed on them take the pass and the composite (with
/// inline overflow). Heads composed elsewhere (the Ara port, tests) keep the inline path.
#[test]
fn the_reflection_pass_belongs_to_the_marked_renderer() {
    use crate::program::{PASS_HEAD_MARK, Stage, compose, with_reflection_history};
    if crate::program::rl_pass_env().is_some() || crate::program::rl_deferred_env().is_some() {
        return;
    }
    let (_, heads) = non_rt_sources(SOURCE).unwrap();
    let marked = format!("{}{PASS_HEAD_MARK}", heads[1]);
    let on = with_reflection_history(compose(
        &marked,
        crate::STANDARD_PROGRAM,
        Stage::Traced,
        false,
    ));
    assert!(
        on.source
            .contains("rl_room_reserve(count,traced,lobe.weight)")
    );
    assert!(on.source.contains("const RL_DEFERRED_PROGRAM:bool=true;"));
    assert_eq!(&on.source[on.program.clone()], crate::STANDARD_PROGRAM);
    let off = with_reflection_history(compose(
        &heads[1],
        crate::STANDARD_PROGRAM,
        Stage::Traced,
        false,
    ));
    assert!(!off.source.contains("rl_composite") && !off.source.contains("RL_DEFERRED_PROGRAM"));
    // Mixed-program frames shade with MP1's list modules, whose program list (binding 38) once
    // shared the glossy list's names: both compositions must validate.
    let list = with_reflection_history(crate::program::compose_list(
        &marked,
        crate::STANDARD_PROGRAM,
        Stage::Traced,
        false,
    ));
    assert!(list.source.contains("rl_room_reserve(count,traced,lobe.weight)"));
    for source in [&on.source, &list.source] {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    }
}

/// The regression the per-renderer default guards: a non-RT engine's renderer runs the pass and
/// marks only its own heads, so heads composed after it exists (the Ara port's, a test's) keep the
/// inline path whatever the order of tests or engines in the process.
#[test]
fn an_engine_default_leaves_other_compositions_inline() {
    use crate::program::{Stage, compose, with_reflection_history};
    if crate::program::rl_pass_env().is_some() || crate::program::rl_deferred_env().is_some() {
        return;
    }
    let engine = crate::engine::Engine::new(
        crate::engine::test_adapter(),
        crate::engine::EngineDescriptor::default(),
    )
    .unwrap();
    let (_, heads) = non_rt_sources(SOURCE).unwrap();
    for head in &heads {
        let source =
            with_reflection_history(compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false))
                .source;
        assert!(!source.contains("rl_composite") && !source.contains("RL_DEFERRED_PROGRAM"));
        assert!(!source.contains("rl_deferred_store"));
    }
    drop(engine);
}

/// Binding 63 (the untraced specular, for identity tests) exists only with
/// RENDERLAB_RL_DEFERRED=diag: without it the deferred module neither declares nor writes it,
/// and the reflectance (62) the composite reads stays.
#[test]
fn deferred_untraced_output_only_with_diag() {
    for head in crate::renderer::primary_ray_gpu::shading_heads(SOURCE).unwrap() {
        let source = deferred_standard(&head).source;
        assert!(source.contains("@group(0) @binding(62) var rl_deferred_reflectance"));
        assert_eq!(
            source.contains("rl_deferred_untraced"),
            crate::program::rl_deferred_diag()
        );
    }
}

/// The primary shading layout, with the pass on or off, deferred or not, diagnostic or not, on
/// both visibility inputs and both paths' geometry: no binding number twice. (The pass's room
/// counters once took 60, the resolve's phase, and every pass-on start failed to create the
/// layout; naga cannot see a layout clash.)
#[test]
fn the_primary_shading_layout_has_no_clashing_bindings() {
    use crate::renderer::primary_ray_gpu::{ShadeSwitches, shade_layout_entries};
    let visibilities = [
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
    ];
    for visibility in visibilities {
        for bits in 0..16u32 {
            let switches = ShadeSwitches {
                pass: bits & 1 != 0,
                software: bits & 2 != 0,
                deferred: bits & 4 != 0,
                diag: bits & 8 != 0,
            };
            let mut seen = std::collections::HashSet::new();
            for entry in shade_layout_entries(visibility, switches) {
                assert!(
                    seen.insert(entry.binding),
                    "binding {} twice under {switches:?}",
                    entry.binding
                );
            }
        }
    }
}

/// The deferred composite's point shader (one point a glossy-pixel list entry) validates.
#[test]
fn reflection_composite_validates() {
    let source = include_str!("../shaders/reflections_composite.wgsl");
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
}

/// Mixed-program frames: by default a reflection hit is shaded by its receiver's program, with the
/// hit's parameters only when the hit is that program's draw (inline's MP1 fallback), so the
/// composite draws the inline picture; RENDERLAB_RL_HIT_OWN_PROGRAM=1 shades each hit with its own
/// program instead.
#[test]
fn mixed_frame_hits_follow_inline_fallback_by_default() {
    use crate::program::{Stage, compose_hits};
    let soft = &crate::renderer::primary_ray_gpu::shading_heads(SOURCE).unwrap()[1];
    let source = compose_hits(soft, crate::STANDARD_PROGRAM, Stage::Traced, false, true).source;
    let fallback = source.contains("if rl_receiver==0u || !rl_hits_mine(rl_receiver-1u) {return;}")
        && source.contains("rl_load_parameters(select(primary_placements[rl_receiver-1u]");
    assert_eq!(fallback, !crate::program::hit_own_program());
    let single = compose_hits(soft, crate::STANDARD_PROGRAM, Stage::Traced, false, false).source;
    assert!(!single.contains("rl_receiver"));
}
