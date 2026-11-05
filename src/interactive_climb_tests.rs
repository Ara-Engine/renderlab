//! Interactive hillclimb screen: paired, rotating dispatch sequences over the traced
//! pose-0 frame. Each variant is one or more passes, timed from its first pass begin to
//! its last pass end. `split` is the pre-fusion trace + 8-direction visibility pair.
use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Split,
    Fused,
    Shade,
}
struct Step {
    kind: Kind,
    /// Workgroup tile of the fused or shading pass.
    tile: [u32; 2],
    pipelines: Vec<Owned<wgpu::ComputePipeline>>,
}
struct Variant {
    name: String,
    steps: Vec<Step>,
}

fn encode(
    r: &Renderer,
    encoder: &mut wgpu::CommandEncoder,
    v: &Variant,
    stamps: Option<(&wgpu::QuerySet, u32)>,
) {
    let last = v.steps.len() - 1;
    for (index, step) in v.steps.iter().enumerate() {
        // Only the sequence's first pass stamps a begin and its last pass an end.
        let writes = |first: bool, end: bool| {
            let (first, end) = (first && index == 0, end && index == last);
            stamps
                .filter(|_| first || end)
                .map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: first.then_some(2 * i),
                    end_of_pass_write_index: end.then_some(2 * i + 1),
                })
        };
        let p = &step.pipelines;
        match step.kind {
            Kind::Split => {
                let tile = primary_ray_gpu::TRACE_TILE;
                r.encode_primary_trace(encoder, &p[0], tile, true, writes(true, false));
                r.encode_primary_fresh_tile(encoder, &p[1], [8, 4], writes(false, true));
            }
            Kind::Fused => {
                r.encode_primary_interactive(encoder, &p[0], step.tile, writes(true, true))
            }
            Kind::Shade => {
                let shading = primary_ray_gpu::Shading::Soft;
                r.encode_primary_shade(encoder, &p[0], step.tile, shading, writes(true, true))
            }
        }
    }
}

/// Complete hit records and counts after a visibility pass; the HDR image after shading.
fn outputs(r: &Renderer, kind: Kind) -> Vec<Vec<u8>> {
    if kind == Kind::Shade {
        vec![read_texture(
            r,
            &r.lighting.hdr.as_ref().unwrap().resolved.texture,
        )]
    } else {
        interactive_outputs(r).to_vec()
    }
}

/// One pass from `{kind: split|fused|shade, edits?: [[module, find, replace]], tile?: [x, y],
/// unchecked?: [ray_query|bounds|loops|int_div]}`; module 0 is hard and 1 soft. `tile`
/// resizes the fused workgroup (shading decodes its region and clear rows from
/// `SHADE_TILE` groups); `unchecked` drops those naga runtime checks from a fused module.
fn step(r: &Renderer, base: &[String; 2], spec: &serde_json::Value) -> Step {
    let kind = match spec["kind"].as_str().unwrap() {
        "split" => Kind::Split,
        "fused" => Kind::Fused,
        "shade" => Kind::Shade,
        other => panic!("unknown kind {other}"),
    };
    let mut sources = base.clone();
    for e in spec["edits"].as_array().into_iter().flatten() {
        let m = e[0].as_u64().unwrap() as usize;
        sources[m] = edit(
            &sources[m],
            (e[1].as_str().unwrap(), e[2].as_str().unwrap()),
        );
    }
    let (default, entry) = match kind {
        Kind::Shade => (primary_ray_gpu::SHADE_TILE, "fn primary_shade_tiles("),
        _ => (primary_ray_gpu::TRACE_TILE, "fn primary_trace_interactive("),
    };
    let tile = spec["tile"]
        .as_array()
        .map_or(default, |t| [0, 1].map(|i| t[i].as_u64().unwrap() as u32));
    assert!(
        kind != Kind::Shade || tile == default,
        "shading groups are SHADE_TILE"
    );
    if tile != default {
        let find = format!("@workgroup_size({},{}) {entry}", default[0], default[1]);
        let replace = format!("@workgroup_size({},{}) {entry}", tile[0], tile[1]);
        sources[1] = edit(&sources[1], (&find, &replace));
    }
    let unchecked = spec["unchecked"].as_array().cloned().unwrap_or_default();
    let mut checks = wgpu::ShaderRuntimeChecks::checked();
    for off in &unchecked {
        match off.as_str().unwrap() {
            "ray_query" => checks.ray_query_initialization_tracking = false,
            "bounds" => checks.bounds_checks = false,
            "loops" => checks.force_loop_bounding = false,
            "int_div" => checks.int_div_checks = false,
            other => panic!("unknown check {other}"),
        }
    }
    let primary = r.primary.as_ref().unwrap();
    let descriptor = wgpu::ShaderModuleDescriptor {
        label: Some("climb soft module"),
        source: wgpu::ShaderSource::Wgsl(sources[1].as_str().into()),
    };
    let soft = if unchecked.is_empty() {
        r.device.create_shader_module(descriptor)
    } else {
        assert!(
            kind == Kind::Fused,
            "unchecked modules are screened on fused only"
        );
        // SAFETY: test-only screen of the library's own traced module, whose ray
        // descriptors, loops, divisors and indices are in range on this workload.
        unsafe { r.device.create_shader_module_trusted(descriptor, checks) }
    };
    let eight = [("FRESH_DIRECTIONS", 8.0)];
    let pipelines = match kind {
        Kind::Split => {
            let [_, receivers] = primary.compile(&r.device, &r.ledger, &sources).trace;
            let fresh =
                primary.fresh_pipeline(&r.device, &r.ledger, &soft, "fresh_compute", &eight);
            vec![receivers, fresh]
        }
        Kind::Fused => vec![primary.interactive_pipeline(&r.device, &r.ledger, &soft, &eight)],
        Kind::Shade => {
            let [_, soft] = primary.compile(&r.device, &r.ledger, &sources).shade;
            vec![soft]
        }
    };
    Step {
        kind,
        tile,
        pipelines,
    }
}

/// Manifest `variants`: `{name, steps: [pass...]}` or a single pass inline (see `step`).
/// Production split, fused and shading run first. A shading-only variant follows the
/// production fused pass untimed during the equivalence check, and every non-`diag`
/// variant must reproduce the public Interactive frame's outputs of its last pass.
#[test]
#[ignore = "GPU lock, 8 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_interactive_climb_screen() {
    let manifest = manifest();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let warmup = manifest["warmup_reps"].as_u64().unwrap_or(24) as usize;
    let reps = manifest["reps"].as_u64().unwrap_or(24) as usize;
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[0].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    r.test_drain("climb screen inputs");
    let public = interactive_outputs(&r);
    let public_hdr = read_texture(&r, &r.lighting.hdr.as_ref().unwrap().resolved.texture);
    let base = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let production = [
        serde_json::json!({"name": "production-split", "kind": "split"}),
        serde_json::json!({"name": "production-fused", "kind": "fused"}),
        serde_json::json!({"name": "production-shade", "kind": "shade"}),
    ];
    let mut variants = vec![];
    for spec in production
        .iter()
        .chain(manifest["variants"].as_array().into_iter().flatten())
    {
        let scopes = OperationScopes::new(&r.device);
        let steps = match spec["steps"].as_array() {
            Some(steps) => steps.iter().map(|s| step(&r, &base, s)).collect(),
            None => vec![step(&r, &base, spec)],
        };
        assert!(
            scopes.finish().is_none(),
            "{} failed to compile",
            spec["name"]
        );
        variants.push(Variant {
            name: spec["name"].as_str().unwrap().to_owned(),
            steps,
        });
    }
    let mut differing = vec![];
    for v in &variants {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let kind = v.steps.last().unwrap().kind;
        if v.steps.len() == 1 && kind == Kind::Shade {
            encode(&r, &mut encoder, &variants[1], None);
        }
        encode(&r, &mut encoder, v, None);
        r.queue.submit([encoder.finish()]);
        let o = outputs(&r, kind);
        let (expected, texel): (Vec<&[u8]>, &[usize]) = if kind == Kind::Shade {
            (vec![&public_hdr], &[8])
        } else {
            (public.iter().map(Vec::as_slice).collect(), &[16, 4])
        };
        let d: Vec<usize> = o
            .iter()
            .zip(&expected)
            .zip(texel)
            .map(|((a, b), &n)| {
                let pairs = a.chunks_exact(n).zip(b.chunks_exact(n));
                pairs.filter(|(x, y)| x != y).count()
            })
            .collect();
        assert!(
            v.name.starts_with("diag") || d.iter().all(|&x| x == 0),
            "{} differs from the public Interactive frame: {d:?}",
            v.name
        );
        differing.push(d);
    }
    let n = variants.len();
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("climb screen"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * n as u32,
    });
    let buffer = |usage| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("climb stamps"),
            size: 16 * n as u64,
            usage,
            mapped_at_creation: false,
        })
    };
    let resolve = buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
    let read = buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
    let period = f64::from(r.queue.get_timestamp_period()) * 1e-6;
    let mut samples = vec![vec![]; n];
    for rep in 0..warmup + reps {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        for k in 0..n {
            let i = (k + rep) % n;
            encode(&r, &mut encoder, &variants[i], Some((&queries, i as u32)));
        }
        encoder.resolve_query_set(&queries, 0..2 * n as u32, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16 * n as u64);
        r.queue.submit([encoder.finish()]);
        let s: Vec<u64> = wait_map(&r, &read)
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        if rep >= warmup {
            for (i, v) in samples.iter_mut().enumerate() {
                v.push((s[2 * i + 1] - s[2 * i]) as f64 * period);
            }
        }
    }
    let median = |v: &[f64]| {
        let mut v = v.to_vec();
        v.sort_by(f64::total_cmp);
        (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
    };
    let summary: Vec<_> = variants
        .iter()
        .zip(&samples)
        .zip(&differing)
        .map(|((v, s), d)| {
            // Shading-only variants pair against production shading, others the split pair.
            let shading = v.steps.len() == 1 && v.steps[0].kind == Kind::Shade;
            let base = &samples[if shading { 2 } else { 0 }];
            let ratios: Vec<f64> = s.iter().zip(base).map(|(a, b)| a / b).collect();
            let tiles: Vec<_> = v.steps.iter().map(|s| s.tile).collect();
            let line = serde_json::json!({"variant": v.name, "tiles": tiles, "differing": d,
                "median_ms": median(s), "min_ms": s.iter().copied().fold(f64::MAX, f64::min),
                "max_ms": s.iter().copied().fold(0.0, f64::max),
                "paired_ratio": median(&ratios), "samples_ms": s});
            eprintln!("climb_screen {line}");
            line
        })
        .collect();
    fs::write(
        out.join("screen.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"validation": validate, "reps": reps,
            "warmup_reps": warmup, "public_matches_split": true, "variants": summary}))
        .unwrap(),
    )
    .unwrap();
}
