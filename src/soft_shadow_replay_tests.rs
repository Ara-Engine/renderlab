//! Dump the exact estimator's hit masks, then replay their 8-bit prefix on the CPU.
use super::*;

#[path = "../support/fresh_fixture.rs"]
mod fixture;

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

/// Same diagnostic atomicOr as e1c7822, in a test-only extra binding. No change to
/// production layouts, ray directions, origins, intersection flags or count logic.
fn diagnostic_source() -> String {
    let s = edit(
        &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1],
        SEED_EDIT,
    );
    let s = edit(
        &s,
        (
            "        visible+=select(1u,0u,blocked);",
            "        visible+=select(1u,0u,blocked);\n        if blocked {atomicOr(&probe_masks[(pixel.x+pixel.y*width)*8u+j/32u],1u<<(j%32u));}",
        ),
    );
    format!(
        "{s}\noverride FRESH_SEED:u32=0u;\n@group(2) @binding(0) var<storage,read_write> probe_masks:array<atomic<u32>>;\n"
    )
}

fn replay_pose(r: &Renderer, out: &std::path::Path, pose: usize, seeds: u32) -> Vec<u8> {
    fs::create_dir_all(out).unwrap();
    let [world, normal, _] = receivers(r);
    fs::write(out.join(format!("pose-{pose}-world.f32")), &world).unwrap();
    fs::write(out.join(format!("pose-{pose}-normal.f32")), &normal).unwrap();
    let t = r.view.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let (w, h) = (t.world.texture.width(), t.world.texture.height());
    let size = u64::from(w) * u64::from(h) * 32;
    let masks = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("exact hit masks"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let staging = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("exact mask readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test-only hit masks"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test-only hit masks"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: masks.as_entire_binding(),
        }],
    });
    let primary = r.view.primary.as_ref().unwrap();
    let (fresh_layout, fresh_bind) = primary.test_fresh_bindings();
    let pipeline_layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("test-only diagnostic ABI"),
            bind_group_layouts: &[
                Some(fresh_layout),
                None,
                Some(&layout),
                Some(&r.lighting.bindings),
            ],
            immediate_size: 0,
        });
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("exact diagnostic masks from e1c7822"),
        source: wgpu::ShaderSource::Wgsl(diagnostic_source().into()),
    });
    let mut seed_zero = vec![];
    // Fixture contour bounds need production spread on these traced receivers too.
    // Only counts are needed for those seeds, beyond the required mask replay set.
    for seed in (0..seeds).chain(if seeds == 16 { 65..81 } else { 0..0 }) {
        let constants = [("FRESH_SEED", f64::from(seed))];
        let p = r
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exact diagnostic masks"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("fresh_compute"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                cache: None,
            });
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&masks, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&p);
            pass.set_bind_group(0, fresh_bind, &[]);
            pass.set_bind_group(2, &bind, &[]);
            pass.set_bind_group(3, r.lighting.bind(), &[]);
            pass.dispatch_workgroups(w.div_ceil(2), h.div_ceil(2), 1);
        }
        encoder.copy_buffer_to_buffer(&masks, 0, &staging, 0, size);
        r.queue.submit([encoder.finish()]);
        let dumped = wait_map(r, &staging);
        let exact = read_texture(r, &t.visibility.texture);
        let masks = words(&dumped);
        let expected: Vec<u32> = masks
            .chunks_exact(8)
            .map(|m| 32 * (8 - (m[0] & 255).count_ones()))
            .collect();
        if seed == 0 {
            seed_zero = expected.iter().flat_map(|v| v.to_le_bytes()).collect();
        }
        let exact_replay: Vec<u32> = masks
            .chunks_exact(8)
            .map(|m| 256 - m.iter().map(|v| v.count_ones()).sum::<u32>())
            .collect();
        assert_eq!(
            words(&exact),
            exact_replay,
            "256-bit diagnostic identity pose={pose} seed={seed}"
        );
        fs::write(
            out.join(format!("pose-{pose}-seed-{seed}-hits.u32")),
            &dumped,
        )
        .unwrap();
        fs::write(
            out.join(format!("pose-{pose}-seed-{seed}-production.u32")),
            &exact,
        )
        .unwrap();
        if seed >= 65 {
            continue;
        }
        for (name, lanes, tile) in LAYOUTS {
            let s = format!(
                "{}\noverride FRESH_SEED:u32=0u;\n",
                edit(&probe_source(lanes, tile, true), SEED_EDIT)
            );
            let shader = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("seeded 8-ray probe"),
                source: wgpu::ShaderSource::Wgsl(s.into()),
            });
            let p =
                primary.fresh_pipeline(&r.device, &r.ledger, &shader, "fresh_compute", &constants);
            let mut encoder = r.device.create_command_encoder(&Default::default());
            dispatch(r, &mut encoder, &p, tile, None);
            r.queue.submit([encoder.finish()]);
            let actual = read_texture(r, &t.visibility.texture);
            assert_eq!(
                words(&actual),
                expected,
                "8-bit CPU/GPU replay layout={name} pose={pose} seed={seed}"
            );
            fs::write(
                out.join(format!("pose-{pose}-seed-{seed}-{name}.u32")),
                actual,
            )
            .unwrap();
        }
        eprintln!(
            "soft_replay_exact pose={pose} seed={seed} layouts=3 pixels={}",
            w * h
        );
    }
    seed_zero
}

#[test]
fn soft_probe_diagnostic_shader_and_count_scaling() {
    validate_wgsl(&diagnostic_source());
    for mask in 0u32..256 {
        let visible = (0..8).filter(|i| mask & (1 << i) == 0).count() as u32;
        assert_eq!(32 * visible, 32 * (8 - mask.count_ones()));
    }
}

#[test]
#[ignore = "GPU lock, 8 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_soft_shadow_mask_replay() {
    let manifest = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    if manifest["workload"] == "quarry" {
        let (mut asset, cameras) = load_quarry(&manifest, &mut r);
        let color = target(&r, 1920, 1080);
        for (index, pose) in [0, 1, 0].into_iter().enumerate() {
            traced_frame(
                &mut r,
                &color,
                &cameras.frames[pose].camera().unwrap(),
                &mut asset,
                true,
            );
            r.test_drain("replay traced quarry receivers");
            replay_pose(&r, &out, index, 4);
        }
    } else {
        let rects = fixture::rectangles(manifest["workload"] == "stress");
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
        r.test_drain("replay mesh");
        let mut receiver = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("replay receiver");
        let mut blockers = r
            .try_upload_instances(&rects.iter().map(|r| r.transform()).collect::<Vec<_>>())
            .unwrap()
            .unwrap();
        r.test_drain("replay blockers");
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(-1., 0., -1.),
                    lux: [1000.; 3],
                }],
                exposure_stops: -8.,
                ..Default::default()
            },
            &Shadows {
                directional_disk_angle: 0.04,
                requests: vec![ShadowRequest::Directional {
                    split_ends: vec![10., 40.],
                }],
                ..Default::default()
            },
        )
        .unwrap();
        r.set_soft_shadows(crate::SoftShadows::Interactive);
        let mut original = None;
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
            let report = render(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&mesh, &mut receiver),
                    Draw::new(&mesh, &mut blockers),
                ],
            );
            assert_eq!(report.soft_shadows, crate::SoftShadows::Interactive);
            r.test_drain("replay traced fixture receivers");
            let [_, rendered] = interactive_outputs(&r);
            let image = capture(&mut r);
            if index == 0 {
                original = Some((rendered.clone(), image.clone()));
            }
            if index == 3 {
                assert_eq!(
                    original.as_ref().unwrap(),
                    &(rendered.clone(), image.clone()),
                    "fixture return"
                );
            }
            fs::write(out.join(format!("pose-{index}-interactive.rgba16f")), image).unwrap();
            // Interactive frames keep receivers in registers; an Exact frame of the same
            // pose publishes the identical traced receivers the replay reads.
            r.set_soft_shadows(crate::SoftShadows::Exact);
            render(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&mesh, &mut receiver),
                    Draw::new(&mesh, &mut blockers),
                ],
            );
            r.set_soft_shadows(crate::SoftShadows::Interactive);
            r.test_drain("replay traced fixture receivers");
            let expected = replay_pose(&r, &out, index, 16);
            assert_eq!(rendered, expected, "fixture public Interactive path");
        }
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
        r.test_drain("Interactive removed blocker");
        assert!(
            words(&interactive_outputs(&r)[1]).iter().all(|&v| v == 256),
            "removed blocker retained shadow"
        );
        r.set_shadows(&Shadows::default()).unwrap();
        r.collect().unwrap();
        assert!(
            r.view.fresh.as_ref().unwrap().targets.is_none(),
            "disabled shadow targets retained"
        );
    }
    fs::write(out.join("replay.json"),serde_json::to_vec_pretty(&serde_json::json!({"validation":true,"workload":manifest["workload"],"all_layouts_bit_exact":true,"seeds":if manifest["workload"]=="quarry" {4} else {16}})).unwrap()).unwrap();
}
