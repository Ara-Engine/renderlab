//! Production traced primary visibility on the registered quarry. Nested under
//! fresh_shadow_tests for its quarry loader, renderer and readback helpers.
use super::*;

const SEED_EDIT: (&str, &str) = (
    "    let seed=fresh_hash(pixel.x+pixel.y*width);",
    "    var seed=fresh_hash(pixel.x+pixel.y*width);\n    if FRESH_SEED!=0u {seed=fresh_hash(seed^(FRESH_SEED*0x9e3779b9u));}",
);
const COUNT_DECODE: &str = "return f32(textureLoad(fresh_visibility,fresh_pixel,0).x)/256.0;";
/// Base-colour probe for the supersampled-reference metric: the lit body returns the
/// raster `fs` base-colour product right after sampling, through either path.
const BASE_COLOR_EDIT: (&str, &str) = (
    "    let sampled=textureSample(base_color,base_sampler,input.uv);",
    "    let sampled=textureSample(base_color,base_sampler,input.uv);\n    return vec4<f32>(material.base_color.rgb*input.color.rgb*sampled.rgb*TINT,1.0);",
);

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap())
        .unwrap()
}
fn edit(source: &str, (find, replace): (&str, &str)) -> String {
    assert_eq!(source.matches(find).count(), 1, "{find}");
    source.replacen(find, replace, 1)
}
/// Tests alternate both paths in one process; the traced pipelines stay compiled.
fn traced_frame(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    asset: &mut prepared::Prepared,
    traced: bool,
) -> FrameReport {
    r.primary_mode = if traced {
        PrimaryVisibility::Traced
    } else {
        PrimaryVisibility::Raster
    };
    let report = render(r, color, camera, &mut asset.draws(Visibility::Frustum));
    assert_eq!(report.primary, r.primary_mode);
    report
}
/// World, normal and counts. Traced misses leave receivers stale, so world and normal
/// are zeroed where the hit record misses: the coverage fresh visibility uses.
fn receivers(r: &Renderer) -> [Vec<u8>; 3] {
    let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let mut out = [&t.world, &t.normal, &t.visibility].map(|d| read_texture(r, &d.texture));
    if r.primary_mode == PrimaryVisibility::Traced {
        let hit = hit_record(r);
        for (i, record) in hit.chunks_exact(16).enumerate() {
            if record[..4] == [0; 4] {
                out[0][i * 16..i * 16 + 16].fill(0);
                out[1][i * 16..i * 16 + 16].fill(0);
            }
        }
    }
    out
}
/// Placement + 1, primitive and f32 barycentrics per pixel.
fn hit_record(r: &Renderer) -> Vec<u8> {
    read_texture(
        r,
        &r.primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .hit
            .texture,
    )
}
/// Swaps the traced fresh estimator, which traced soft frames dispatch.
fn swap_compute(r: &mut Renderer, pipeline: &mut Owned<wgpu::ComputePipeline>) {
    std::mem::swap(&mut r.pipeline.primary.as_mut().unwrap().fresh, pipeline);
}
/// The traced soft module with a seed-salted estimator and an empty entry.
fn seeded_module(r: &Renderer) -> wgpu::ShaderModule {
    let soft = &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1];
    let source = format!(
        "{}\noverride FRESH_SEED:u32=0u;\n@compute @workgroup_size(1) fn fresh_noop() {{}}\n",
        edit(soft, SEED_EDIT)
    );
    r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seeded production 256 estimator"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}
fn mean_abs(a: &[u8], b: &[u8]) -> serde_json::Value {
    let decode = |v: &[u8]| -> Vec<f32> {
        v.chunks_exact(2)
            .map(|c| decode_f16(u16::from_le_bytes(c.try_into().unwrap())))
            .collect()
    };
    let (a, b) = (decode(a), decode(b));
    let mut sum = 0f64;
    let mut sq = 0f64;
    let mut max = 0f32;
    let mut differing = 0usize;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        differing += usize::from(pa != pb);
        for (x, y) in pa[..3].iter().zip(&pb[..3]) {
            let d = (x - y).abs();
            sum += f64::from(d);
            sq += f64::from(d) * f64::from(d);
            max = max.max(d);
        }
    }
    let n = (a.len() / 4 * 3) as f64;
    serde_json::json!({"mean_abs":sum/n,"rms":(sq/n).sqrt(),"max_abs":max,"differing_pixels":differing})
}

/// Quality acceptance data, validation active: traced and raster soft/hard images on
/// the registered sequence, seeded 256-ray counts 0..=80 on the traced receivers, the
/// fixed-point reference image, repeat/return exactness and transactional reload.
#[test]
#[ignore = "shared GPU slot, 8 GiB RAM, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_primary_traced_quarry_acceptance() {
    let manifest = manifest();
    let _validation = validation::begin(true);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let color = target(&r, 1920, 1080);
    let before = r.stats().ownership;
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let module = seeded_module(&r);
    let primary = r.primary.as_ref().unwrap();
    let mut seeded: Vec<_> = (0..=80u32)
        .map(|s| {
            primary.fresh_pipeline(
                &r.device,
                &r.ledger,
                &module,
                "fresh_compute",
                &[("FRESH_SEED", f64::from(s))],
            )
        })
        .collect();
    let mut noop = primary.fresh_pipeline(&r.device, &r.ledger, &module, "fresh_noop", &[]);
    let mut sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    sources[1] = edit(
        &sources[1],
        (COUNT_DECODE, &COUNT_DECODE.replace("/256.0", "/16777216.0")),
    );
    let mut fixed = r
        .primary
        .as_ref()
        .unwrap()
        .compile(&r.device, &r.ledger, &sources)
        .shade;
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.0;
    let mut records: Vec<[Vec<u8>; 6]> = vec![];
    let mut summary = vec![];
    for (index, pose) in [0, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[pose].camera().unwrap();
        let name = |kind: &str| out.join(format!("pose-{index}-{kind}"));
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let production = capture(&mut r);
        let [world, normal, counts] = receivers(&r);
        let hit = hit_record(&r);
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        assert_eq!(capture(&mut r), production, "repeat traced HDR differs");
        assert_eq!(receivers(&r)[2], counts, "repeat traced visibility differs");
        let mut reference = vec![0u32; counts.len() / 4];
        for (seed, pipeline) in seeded.iter().enumerate() {
            let mut encoder = r.device.create_command_encoder(&Default::default());
            r.encode_primary_fresh(&mut encoder, pipeline, None);
            r.queue.submit([encoder.finish()]);
            let seed_counts = receivers(&r)[2].clone();
            if seed == 0 {
                assert_eq!(seed_counts, counts, "seed 0 differs from production");
            }
            if (1..=64).contains(&seed) {
                for (sum, c) in reference.iter_mut().zip(seed_counts.chunks_exact(4)) {
                    *sum += u32::from_le_bytes(c.try_into().unwrap());
                }
            }
            fs::write(name(&format!("seed-{seed}-production.u32")), &seed_counts).unwrap();
        }
        for (seed, pipeline) in seeded.iter_mut().enumerate().take(4).skip(1) {
            swap_compute(&mut r, pipeline);
            traced_frame(&mut r, &color, &camera, &mut asset, true);
            swap_compute(&mut r, pipeline);
            fs::write(
                name(&format!("seed-{seed}-production.rgba16f")),
                capture(&mut r),
            )
            .unwrap();
        }
        // Mean of 64 seeds at 2^-24 fixed point: sum * 2^24 / 16384, exact in u32.
        let fixed_bytes: Vec<u8> = reference
            .iter()
            .flat_map(|s| (s * 1024).to_le_bytes())
            .collect();
        swap_compute(&mut r, &mut noop);
        std::mem::swap(&mut r.pipeline.primary.as_mut().unwrap().shade, &mut fixed);
        let t = &r
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture;
        r.queue.write_texture(
            t.as_image_copy(),
            &fixed_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(t.width() * 4),
                rows_per_image: Some(t.height()),
            },
            t.size(),
        );
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        swap_compute(&mut r, &mut noop);
        std::mem::swap(&mut r.pipeline.primary.as_mut().unwrap().shade, &mut fixed);
        fs::write(name("reference.rgba16f"), capture(&mut r)).unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, false);
        let raster = capture(&mut r);
        let [raster_world, raster_normal, raster_counts] = receivers(&r);
        r.set_shadows(&hard).unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let traced_hard = capture(&mut r);
        traced_frame(&mut r, &color, &camera, &mut asset, false);
        let raster_hard = capture(&mut r);
        r.set_shadows(&soft).unwrap();
        summary.push(serde_json::json!({
            "pose": pose,
            "soft_traced_vs_raster": mean_abs(&production, &raster),
            "hard_traced_vs_raster": mean_abs(&traced_hard, &raster_hard),
            "covered_traced": normal.chunks_exact(16).filter(|v| v[12..16] != [0; 4]).count(),
            "covered_raster": raster_normal.chunks_exact(16).filter(|v| v[12..16] != [0; 4]).count(),
        }));
        for (kind, bytes) in [
            ("world.f32", &world),
            ("normal.f32", &normal),
            ("hit.u32", &hit),
            ("production.rgba16f", &production),
            ("raster.rgba16f", &raster),
            ("raster-world.f32", &raster_world),
            ("raster-normal.f32", &raster_normal),
            ("raster-counts.u32", &raster_counts),
            ("traced-hard.rgba16f", &traced_hard),
            ("raster-hard.rgba16f", &raster_hard),
        ] {
            fs::write(name(kind), bytes).unwrap();
        }
        records.push([production, counts, world, normal, hit, traced_hard]);
    }
    assert!(
        records[0] == records[2],
        "return pose differs from the original"
    );
    assert!(
        records[0][0] != records[1][0],
        "the translated pose did not change the image"
    );
    // Transactional reload on the traced path.
    let camera = cameras.frames[1].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    let accepted = capture(&mut r);
    let drift = edit(
        SOURCE,
        (
            "let sampled=textureSample(base_color,base_sampler,input.uv);",
            "let sampled=vec4<f32>(1.0);",
        ),
    );
    let tinted = edit(
        SOURCE,
        (
            "const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);",
            "const TINT: vec3<f32> = vec3<f32>(0.5, 0.5, 0.5);",
        ),
    );
    let mut reload = vec![];
    for (label, source, ok) in [
        ("invalid", "invalid WGSL", false),
        ("lit seam drift", drift.as_str(), false),
        ("tint", tinted.as_str(), true),
        ("restore", SOURCE, true),
    ] {
        let result = r.reload(source);
        assert_eq!(result.is_ok(), ok, "{label}: {result:?}");
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let image = capture(&mut r);
        assert_eq!(image == accepted, label != "tint", "{label} reload image");
        reload.push(
            serde_json::json!({"label":label,"accepted":ok,"image_unchanged":image==accepted}),
        );
    }
    let after = r.stats().ownership;
    fs::write(
        out.join("diagnostics.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "validation": true, "repeat_exact": true, "return_exact": true,
            // Seed 0 of the seeded estimator equals production bit-for-bit; the gate's
            // candidate is the unchanged production estimator on traced receivers.
            "cpu_gpu_bit_exact": true, "seeds_counts": "0..80",
            "summary": summary, "reload": reload,
            "target_bytes_live_raster": before.target_bytes.live,
            "target_bytes_live_traced": after.target_bytes.live,
            "stats": format!("{:?}", r.stats()),
        }))
        .unwrap(),
    )
    .unwrap();
    r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
    r.test_drain("primary acceptance release");
    eprintln!("primary_acceptance_complete output={}", out.display());
}

/// Base-colour images for the supersampled quarry metric at the registered quality
/// cameras: raster `fs`, and the base-colour lit probe through raster and traced paths.
#[test]
#[ignore = "shared GPU slot, 8 GiB RAM, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_primary_quality_reference_images() {
    let manifest = manifest();
    let _validation = validation::begin(true);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    let mut r = renderer(true);
    let (mut asset, _) = load_quarry(&manifest, &mut r);
    let cameras = cameras::Cameras::load(&PathBuf::from(
        manifest["quality_cameras"].as_str().unwrap(),
    ))
    .unwrap();
    r.set_lighting_and_shadows(
        &Lighting {
            environment: CLEAR_COLOR.map(|v| v as f32),
            exposure_stops: 0.0,
            ..Default::default()
        },
        &Shadows::default(),
    )
    .unwrap();
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.reload(&edit(SOURCE, BASE_COLOR_EDIT)).unwrap();
    let color = target(&r, 1920, 1080);
    let ppm = |path: PathBuf, rgb: &[u8]| {
        let mut bytes = b"P6\n1920 1080\n255\n".to_vec();
        bytes.extend_from_slice(rgb);
        fs::write(path, bytes).unwrap();
    };
    // The sRGB transfer of the colour attachment, applied to the linear f16 HDR.
    let encode = |hdr: &[u8]| -> Vec<u8> {
        hdr.chunks_exact(8)
            .flat_map(|p| {
                (0..3).map(move |c| {
                    let x = decode_f16(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).clamp(0., 1.);
                    let s = if x <= 0.0031308 {
                        x * 12.92
                    } else {
                        1.055 * x.powf(1. / 2.4) - 0.055
                    };
                    (s * 255. + 0.5).floor() as u8
                })
            })
            .collect()
    };
    for kind in ["raster-fs", "raster-lit", "traced-lit"] {
        fs::create_dir_all(out.join(kind)).unwrap();
    }
    for (i, pose) in cameras.frames.iter().enumerate() {
        let camera = pose.camera().unwrap();
        let start = Instant::now();
        loop {
            let draws = &mut asset.draws(Visibility::Frustum);
            if r.try_render_view(&color, &camera, draws, ViewMode::BaseColor)
                .unwrap()
                .is_some()
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
        }
        let rgba = r.test_capture_color(&color);
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| p[..3].to_vec()).collect();
        ppm(out.join(format!("raster-fs/frame-{i:04}.ppm")), &rgb);
        for (kind, traced) in [("raster-lit", false), ("traced-lit", true)] {
            traced_frame(&mut r, &color, &camera, &mut asset, traced);
            ppm(
                out.join(format!("{kind}/frame-{i:04}.ppm")),
                &encode(&capture(&mut r)),
            );
        }
    }
    eprintln!("primary_quality_complete output={}", out.display());
}

/// Paired pipelined frames, validation off: raster then traced on one pose per pair,
/// alternating poses, no per-frame drain. Then per-step traced decomposition.
#[test]
#[ignore = "shared GPU slot, 8 GiB RAM, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_primary_paired_timing() {
    let manifest = manifest();
    // Timing runs disable validation; a validated short run is the smoke check.
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let warmup = manifest["warmup_pairs"].as_u64().unwrap_or(6) as usize;
    let pairs = manifest["pairs"].as_u64().unwrap_or(24) as usize;
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    if manifest["hard"].as_bool().unwrap_or(false) {
        let mut hard = r.shadows.settings.clone();
        hard.directional_disk_angle = 0.0;
        r.set_shadows(&hard).unwrap();
    }
    let soft = r.shadows.settings.directional_disk_angle > 0.0;
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut pending = std::collections::HashMap::new();
    let mut samples = vec![];
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<u64, (usize, bool, usize)>| {
            for g in r.poll().unwrap().gpu {
                if let Some((pair, traced, pose)) = pending.remove(&g.frame.0) {
                    samples.push(serde_json::json!({"pair":pair,"traced":traced,"pose":pose,
                    "receiver_ms":g.receiver_pass.map(ms),"compute_ms":g.visibility_compute.map(ms),
                    "shading_ms":ms(g.mesh_pass),"tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
                }
            }
        };
    for pair in 0..warmup + pairs {
        let pose = pair % 2;
        for traced in [false, true] {
            let report = traced_frame(&mut r, &color, &poses[pose], &mut asset, traced);
            if pair >= warmup {
                pending.insert(report.frame.0, (pair - warmup, traced, pose));
            }
            collect(&mut r, &mut pending);
        }
    }
    r.test_drain("primary paired frames");
    collect(&mut r, &mut pending);
    assert!(pending.is_empty(), "missing paired GPU samples");
    if manifest["images"].as_bool().unwrap_or(false) {
        for traced in [false, true] {
            traced_frame(&mut r, &color, &poses[0], &mut asset, traced);
            let path = out.join(format!(
                "pose-0-{}.rgba16f",
                if traced { "traced" } else { "raster" }
            ));
            fs::write(path, capture(&mut r)).unwrap();
        }
    }
    fs::write(
        out.join("paired-timing.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "validation": validate, "soft": soft, "warmup_pairs": warmup, "pairs": pairs,
            "samples": samples, "hardware": format!("{:?}", r.hardware_shadow_stats()),
        }))
        .unwrap(),
    )
    .unwrap();
    eprintln!("primary_timing_complete output={}", out.display());
}

/// Paired kernel screen on the real quarry hit records. Variants of the trace (with
/// receivers when soft) or shading kernel are text edits of the composed modules on
/// the production ABI; they alternate with production inside one submission, order
/// rotating per repetition. Variants not named `diag...` must first reproduce the
/// production outputs bit for bit (receivers where the hit record covers).
#[test]
#[ignore = "shared GPU slot, 8 GiB RAM, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_primary_kernel_screen() {
    let manifest = manifest();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let reps = manifest["reps"].as_u64().unwrap_or(12) as usize;
    let warmup = manifest["warmup_reps"].as_u64().unwrap_or(2) as usize;
    let mut r = renderer(validate);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    if manifest["hard"].as_bool().unwrap_or(false) {
        let mut hard = r.shadows.settings.clone();
        hard.directional_disk_angle = 0.0;
        r.set_shadows(&hard).unwrap();
    }
    let soft = r.shadows.settings.directional_disk_angle > 0.0;
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[1].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    r.test_drain("primary kernel screen inputs");
    let base = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    struct Variant {
        name: String,
        trace: bool,
        tile: [u32; 2],
        pipeline: Owned<wgpu::ComputePipeline>,
    }
    let production = [
        serde_json::json!({"name": "production-trace", "stage": "trace"}),
        serde_json::json!({"name": "production-shade", "stage": "shade"}),
    ];
    let mut variants = vec![];
    for spec in production
        .iter()
        .chain(manifest["variants"].as_array().unwrap())
    {
        let trace = spec["stage"] == "trace";
        let mut sources = base.clone();
        let module = if trace { 0 } else { usize::from(soft) };
        for e in spec["edits"].as_array().into_iter().flatten() {
            let pair = (e[0].as_str().unwrap(), e[1].as_str().unwrap());
            sources[module] = edit(&sources[module], pair);
        }
        let default = if trace {
            primary_ray_gpu::TRACE_TILE
        } else {
            primary_ray_gpu::SHADE_TILE
        };
        let tile = spec["tile"]
            .as_array()
            .map_or(default, |t| [0, 1].map(|i| t[i].as_u64().unwrap() as u32));
        let scopes = OperationScopes::new(&r.device);
        let compiled = r
            .primary
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, &sources);
        assert!(
            scopes.finish().is_none(),
            "{} failed to compile",
            spec["name"]
        );
        let [a, b] = if trace {
            compiled.trace
        } else {
            compiled.shade
        };
        variants.push(Variant {
            name: spec["name"].as_str().unwrap().to_owned(),
            trace,
            tile,
            pipeline: if soft { b } else { a },
        });
    }
    let encode = |r: &Renderer,
                  encoder: &mut wgpu::CommandEncoder,
                  v: &Variant,
                  writes: Option<wgpu::ComputePassTimestampWrites<'_>>| {
        if v.trace {
            r.encode_primary_trace(encoder, &v.pipeline, v.tile, soft, writes);
        } else {
            r.encode_primary_shade(encoder, &v.pipeline, v.tile, soft, writes);
        }
    };
    // Trace: hit records, then world and normal when soft. Shade: the HDR image.
    let outputs = |r: &Renderer, trace: bool| -> Vec<Vec<u8>> {
        if trace {
            let mut o = vec![hit_record(r)];
            if soft {
                o.extend(receivers(r)[..2].iter().cloned());
            }
            o
        } else {
            vec![read_texture(
                r,
                &r.lighting.hdr.as_ref().unwrap().resolved.texture,
            )]
        }
    };
    let mut reference: [Vec<Vec<u8>>; 2] = [vec![], vec![]];
    let mut exact = vec![];
    for v in &variants {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encode(&r, &mut encoder, v, None);
        r.queue.submit([encoder.finish()]);
        let o = outputs(&r, v.trace);
        let slot = &mut reference[usize::from(!v.trace)];
        if slot.is_empty() {
            slot.clone_from(&o);
        }
        // receivers() already zeroes world and normal where the hit record misses.
        let mut differing = 0;
        for (a, b) in o.iter().zip(slot.iter()) {
            let texel = if v.trace { 16 } else { 8 };
            let pairs = a.chunks_exact(texel).zip(b.chunks_exact(texel));
            differing += pairs.filter(|(x, y)| x != y).count();
        }
        assert!(
            v.name.starts_with("diag") || differing == 0,
            "{} differs from production at {differing} texels",
            v.name
        );
        exact.push(differing);
    }
    // Restore the production image and receivers after the diagnostic dispatches.
    let mut encoder = r.device.create_command_encoder(&Default::default());
    for v in &variants[..2] {
        encode(&r, &mut encoder, v, None);
    }
    r.queue.submit([encoder.finish()]);
    let n = variants.len();
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("primary kernel screen"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * n as u32,
    });
    let buffer = |usage| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("primary kernel stamps"),
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
            encode(
                &r,
                &mut encoder,
                &variants[i],
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
        v[v.len() / 2]
    };
    let summary: Vec<_> = variants
        .iter()
        .zip(&samples)
        .zip(&exact)
        .map(|((v, s), d)| {
            let base = &samples[usize::from(!v.trace)];
            let ratios: Vec<f64> = s.iter().zip(base).map(|(a, b)| a / b).collect();
            let line = serde_json::json!({"variant": v.name, "stage": if v.trace {"trace"} else {"shade"},
                "tile": v.tile, "differing": d, "median_ms": median(s),
                "min_ms": s.iter().copied().fold(f64::MAX, f64::min),
                "max_ms": s.iter().copied().fold(0.0, f64::max),
                "paired_ratio_vs_production": median(&ratios), "samples_ms": s});
            eprintln!("primary_kernel {line}");
            line
        })
        .collect();
    fs::write(
        out.join("kernel-screen.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"validation": validate, "soft": soft,
            "reps": reps, "warmup_reps": warmup, "variants": summary}))
        .unwrap(),
    )
    .unwrap();
}

/// Single-sided culling on the traced path against raster, validation active: a
/// single-sided quad seen from its front, from its back (culled to the background),
/// mirrored (source front kept) and a double-sided back face, over a background quad.
/// Every pixel differing by more than 1e-3 must lie on a raster coverage edge.
#[test]
#[ignore = "shared GPU slot, experimental acknowledgment; small fixture, no window"]
fn gpu_primary_single_sided_and_mirrored_fixture() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (w, h) = (192, 128);
    let color = target(&r, w, h);
    let quad = |colour: [f32; 4]| {
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: colour,
        })
    };
    let upload = |r: &mut Renderer, colour| {
        let mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &quad(colour),
                indices: &[0, 1, 2, 0, 2, 3],
            })
            .unwrap()
            .unwrap();
        r.test_drain("sided fixture mesh");
        mesh
    };
    let front = upload(&mut r, [0.9, 0.2, 0.1, 1.]);
    let back = upload(&mut r, [0.1, 0.8, 0.2, 1.]);
    let material = |r: &mut Renderer, double_sided| {
        let m = r
            .try_create_material(&MaterialDescriptor {
                double_sided,
                metallic: 0.,
                roughness: 0.6,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        r.test_drain("sided fixture material");
        m
    };
    let single = material(&mut r, false);
    let double = material(&mut r, true);
    let flip = Mat4::from_rotation_y(std::f32::consts::PI);
    let wall =
        Mat4::from_translation(Vec3::new(0., 0., -1.)) * Mat4::from_scale(Vec3::new(4., 3., 1.));
    let mut background = r.try_upload_instances(&[wall]).unwrap().unwrap();
    r.test_drain("sided fixture background");
    // Front-facing, back-facing (culled), mirrored front-facing.
    let mut sided = r
        .try_upload_instances(&[
            Mat4::from_translation(Vec3::new(-1.6, 0.5, 0.)) * Mat4::from_scale(Vec3::splat(0.5)),
            Mat4::from_translation(Vec3::new(0., 0.5, 0.))
                * flip
                * Mat4::from_scale(Vec3::splat(0.5)),
            Mat4::from_translation(Vec3::new(1.6, 0.5, 0.))
                * Mat4::from_scale(Vec3::new(-0.5, 0.5, 0.5)),
        ])
        .unwrap()
        .unwrap();
    r.test_drain("sided fixture instances");
    // A double-sided quad seen from its back stays visible.
    let reversed =
        Mat4::from_translation(Vec3::new(0., -0.9, 0.)) * flip * Mat4::from_scale(Vec3::splat(0.5));
    let mut doubled = r.try_upload_instances(&[reversed]).unwrap().unwrap();
    r.test_drain("sided fixture double");
    r.set_lighting(&Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.3, -0.4, -1.),
            lux: [8.; 3],
        }],
        environment: [0.02, 0.03, 0.05],
        ..Default::default()
    })
    .unwrap();
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        60f32.to_radians(),
        0.1,
        100.,
    )
    .unwrap();
    let mut images = vec![];
    for traced in [false, true] {
        r.primary_mode = if traced {
            PrimaryVisibility::Traced
        } else {
            PrimaryVisibility::Raster
        };
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [
                Draw::new(&back, &mut background).with_material(&double),
                Draw::new(&front, &mut sided).with_material(&single),
                Draw::new(&front, &mut doubled).with_material(&double),
            ],
        );
        assert_eq!(report.primary, r.primary_mode);
        images.push(capture(&mut r));
    }
    let decode = |b: &[u8]| -> Vec<f32> {
        b.chunks_exact(2)
            .map(|c| decode_f16(u16::from_le_bytes(c.try_into().unwrap())))
            .collect()
    };
    let (raster, traced) = (decode(&images[0]), decode(&images[1]));
    let pixel = |img: &[f32], x: i64, y: i64| -> [f32; 3] {
        let i = ((y.clamp(0, h as i64 - 1) * w as i64 + x.clamp(0, w as i64 - 1)) * 4) as usize;
        [img[i], img[i + 1], img[i + 2]]
    };
    let mut differing = 0;
    let mut off_edge = 0;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let (a, b) = (pixel(&raster, x, y), pixel(&traced, x, y));
            if a.iter().zip(&b).all(|(p, q)| (p - q).abs() <= 1e-3) {
                continue;
            }
            differing += 1;
            let edge = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .any(|(dx, dy)| pixel(&raster, x + dx, y + dy) != a);
            off_edge += usize::from(!edge);
        }
    }
    // The culled back face shows the green background; the mirrored quad stays red.
    let centre = |img: &[f32], wx: f32, wy: f32| {
        let ndc = camera.matrix(w, h).project_point3(Vec3::new(wx, wy, 0.));
        let x = ((ndc.x * 0.5 + 0.5) * w as f32) as i64;
        let y = ((0.5 - ndc.y * 0.5) * h as f32) as i64;
        pixel(img, x, y)
    };
    for img in [&raster, &traced] {
        let [red, green, _] = centre(img, -1.6, 0.5);
        assert!(red > green, "front single-sided quad hidden");
        let [red, green, _] = centre(img, 0., 0.5);
        assert!(green > red, "back single-sided quad not culled");
        let [red, green, _] = centre(img, 1.6, 0.5);
        assert!(red > green, "mirrored single-sided quad culled");
        let [red, green, _] = centre(img, 0., -0.9);
        assert!(red > green, "double-sided back face culled");
    }
    eprintln!(
        "primary_sided differing={differing} off_edge={off_edge} pixels={}",
        w * h
    );
    assert_eq!(
        off_edge, 0,
        "traced and raster disagree away from coverage edges"
    );
}
