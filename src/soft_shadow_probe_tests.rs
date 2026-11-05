//! Test-only Level-B cost screen, independent of the selected production mode.
use super::*;

#[path = "determinism_tests.rs"]
mod determinism_tests;
#[path = "interactive_climb_tests.rs"]
mod interactive_climb_tests;
#[path = "soft_shadow_acceptance_tests.rs"]
mod soft_shadow_acceptance_tests;
#[path = "soft_shadow_replay_tests.rs"]
mod soft_shadow_replay_tests;

const LAYOUTS: [(&str, u32, [u32; 2]); 3] =
    [("8x1", 8, [2, 2]), ("4x2", 4, [2, 4]), ("1x8", 1, [8, 4])];

/// Only the loop bound, integer output scale and receiver scheduling change.
fn probe_source(lanes: u32, tile: [u32; 2], traversal: bool) -> String {
    let mut source = primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone();
    for (find, replace) in [
        (
            "const FRESH_LANES:u32=8u;",
            format!("const FRESH_LANES:u32={lanes}u;"),
        ),
        (
            "override FRESH_DIRECTIONS:u32=256u;",
            "const FRESH_DIRECTIONS:u32=8u;".into(),
        ),
        (
            "j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)",
            "j+=FRESH_LANES".into(),
        ),
        (
            "let lanes=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u);",
            "let lanes=FRESH_LANES;".into(),
        ),
        (
            "select(FRESH_TILE,vec2<u32>(8u,4u),FRESH_DIRECTIONS==8u)",
            "FRESH_TILE".into(),
        ),
        (
            "const FRESH_TILE=vec2<u32>(2u,2u);",
            format!("const FRESH_TILE=vec2<u32>({}u,{}u);", tile[0], tile[1]),
        ),
        (
            "fresh_counts:array<atomic<u32>,32>",
            format!("fresh_counts:array<atomic<u32>,{}>", 32 / lanes),
        ),
    ] {
        source = edit(&source, (find, &replace));
    }
    if !traversal {
        source = edit(&source, ("while rayQueryProceed(&query) {}", ""));
    }
    source
}

#[test]
fn soft_probe_layouts_compile_and_cover_each_receiver() {
    for (_, lanes, tile) in LAYOUTS {
        for traversal in [true, false] {
            let source = probe_source(lanes, tile, traversal);
            let (module, info) = validate_wgsl(&source);
            assert_eq!(tile[0] * tile[1] * lanes, 32);
            let entry = module
                .entry_points
                .iter()
                .find(|e| e.name == "fresh_compute")
                .unwrap();
            assert_eq!(entry.workgroup_size, [32, 1, 1]);
            naga::back::spv::write_vec(
                &module,
                &info,
                &Default::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: entry.stage,
                    entry_point: entry.name.clone(),
                }),
            )
            .unwrap();
        }
        for [w, h] in [[1u32, 1], [3, 5], [256, 192], [1920, 1080]] {
            let mut seen = vec![0u8; (w * h) as usize];
            for gy in 0..h.div_ceil(tile[1]) {
                for gx in 0..w.div_ceil(tile[0]) {
                    for slot in 0..32 / lanes {
                        let x = gx * tile[0] + slot % tile[0];
                        let y = gy * tile[1] + slot / tile[0];
                        if x < w && y < h {
                            seen[(x + y * w) as usize] += 1;
                        }
                    }
                }
            }
            assert!(seen.iter().all(|&v| v == 1));
        }
    }
}

fn pipeline(r: &Renderer, source: String) -> Owned<wgpu::ComputePipeline> {
    let scopes = OperationScopes::new(&r.device);
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test-only 8-direction prefix"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let p = r.primary.as_ref().unwrap().fresh_pipeline(
        &r.device,
        &r.ledger,
        &module,
        "fresh_compute",
        &[],
    );
    assert!(scopes.finish().is_none());
    p
}

fn dispatch(
    r: &Renderer,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    tile: [u32; 2],
    stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) {
    let primary = r.primary.as_ref().unwrap();
    let texture = &primary.targets.as_ref().unwrap().hit.texture;
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("test-only 8-ray cost probe"),
        timestamp_writes: stamps,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, primary.test_fresh_bindings().1, &[]);
    pass.set_bind_group(3, r.lighting.bind(), &[]);
    pass.dispatch_workgroups(
        texture.width().div_ceil(tile[0]),
        texture.height().div_ceil(tile[1]),
        1,
    );
}

/// Rotating, paired dispatches share immutable traced receivers. Frame samples then
/// pair the current hard frame with the unchanged exact soft frame in this process.
#[test]
#[ignore = "GPU lock, 8 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_soft_shadow_cost_probe() {
    let manifest = manifest();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let warmup = manifest["warmup_pairs"].as_u64().unwrap_or(24) as usize;
    let pairs = manifest["pairs"].as_u64().unwrap_or(24) as usize;
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[0].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    r.test_drain("probe immutable traced receivers");
    let mut variants: Vec<_> = LAYOUTS
        .iter()
        .map(|&(name, lanes, tile)| (name, tile, pipeline(&r, probe_source(lanes, tile, true))))
        .collect();
    variants.push((
        "exact256",
        [2, 2],
        pipeline(
            &r,
            primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone(),
        ),
    ));
    variants.push((
        "diag-no-proceed",
        [2, 2],
        pipeline(&r, probe_source(8, [2, 2], false)),
    ));
    variants.push((
        "diag-1x8-no-proceed",
        [8, 4],
        pipeline(&r, probe_source(1, [8, 4], false)),
    ));
    let n = variants.len();
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("soft probe timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * n as u32,
    });
    let buffer = |usage| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("soft probe timestamps"),
            size: 16 * n as u64,
            usage,
            mapped_at_creation: false,
        })
    };
    let resolve = buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
    let read = buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
    let mut samples = vec![vec![]; n];
    for rep in 0..warmup + pairs {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        for k in 0..n {
            let i = (rep + k) % n;
            dispatch(
                &r,
                &mut encoder,
                &variants[i].2,
                variants[i].1,
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
        let stamps: Vec<_> = wait_map(&r, &read)
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        if rep >= warmup {
            for i in 0..n {
                samples[i].push(
                    (stamps[2 * i + 1] - stamps[2 * i]) as f64
                        * f64::from(r.queue.get_timestamp_period())
                        * 1e-6,
                );
            }
        }
    }
    // Every layout must produce the identical scaled eight-direction counts.
    let mut expected = None;
    for (name, tile, p) in variants.iter().take(3) {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        dispatch(&r, &mut encoder, p, *tile, None);
        r.queue.submit([encoder.finish()]);
        let counts = receivers(&r)[2].clone();
        if let Some(ref previous) = expected {
            assert_eq!(&counts, previous, "layout {name}");
        }
        expected = Some(counts);
    }
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.0;
    let mut pending = std::collections::HashMap::new();
    let mut frames = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<u64, (usize, bool)>| {
            for g in r.poll().unwrap().gpu {
                if let Some((pair, exact)) = pending.remove(&g.frame.0) {
                    frames.push(serde_json::json!({"pair":pair,"exact":exact,"receiver_ms":g.receiver_pass.map(ms),"compute_ms":g.visibility_compute.map(ms),"shading_ms":ms(g.mesh_pass),"tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
                }
            }
        };
    for pair in 0..warmup + pairs {
        for exact in if pair % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            r.set_shadows(if exact { &soft } else { &hard }).unwrap();
            let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
            if pair >= warmup {
                pending.insert(report.frame.0, (pair - warmup, exact));
            }
            collect(&mut r, &mut pending);
        }
    }
    r.test_drain("probe paired exact and hard frames");
    collect(&mut r, &mut pending);
    assert!(pending.is_empty());
    let summary: Vec<_> = variants.iter().zip(samples).map(|((name,tile,_),s)| {
        let mut sorted=s.clone(); sorted.sort_by(f64::total_cmp);
        serde_json::json!({"name":name,"tile":tile,"median_ms":(sorted[(sorted.len()-1)/2]+sorted[sorted.len()/2])/2.,"min_ms":sorted[0],"max_ms":sorted[sorted.len()-1],"samples_ms":s})
    }).collect();
    fs::write(out.join("cost.json"),serde_json::to_vec_pretty(&serde_json::json!({"validation":validate,"warmup_pairs":warmup,"pairs":pairs,"layouts_equal":true,"variants":summary,"frames":frames})).unwrap()).unwrap();
    eprintln!(
        "soft_probe_complete {}",
        serde_json::to_string(&summary).unwrap()
    );
}
