//! TR1 GPU runs (docs/m7-tr1.md), ignored by default and run under an owned GPU lease with
//! RENDERLAB_PRIMARY_MANIFEST. Nested under visibility_buffer_tests for its RT and no-RT
//! renderers and quarry loaders.
//!
//! `gpu_tr1_sequence` loads the quarry once on one path (`path`: `rt` = traced primaries
//! with Interactive shadows, the RT fast setup; `nonrt` = the no-RT device's visibility
//! buffer with Virtual Surface shadows; `nonrt-on-rt` = that path on an RT device, for the
//! paired byte-identity check), then renders the registered `sequence` from cold once per
//! entry of `variants` ({name: Reconstruction overrides}; `"enabled": false` is the
//! unreconstructed baseline). The first visit of each variant writes `<name>/fNNN.rgba16f`
//! for the frames in `frames` and every frame's SHA-256; `visits` further alternating
//! visits (order reversed on odd visits) record per-frame GPU spans for the paired cost.
//! `replay` renders the first variant again and compares every frame hash (F8).
use super::*;

fn reconstruction(v: &serde_json::Value) -> Reconstruction {
    let mut s = Reconstruction::identity();
    let f = |key: &str, value: &mut f32| {
        if let Some(x) = v[key].as_f64() {
            *value = x as f32;
        }
    };
    f("window", &mut s.window);
    f("blur_budget", &mut s.blur_budget);
    f("sharp_window", &mut s.sharp_window);
    f("silhouette_window", &mut s.silhouette_window);
    f("start_window", &mut s.start_window);
    f("turn_angle", &mut s.turn_angle);
    let b = |key: &str, value: &mut bool| {
        if let Some(x) = v[key].as_bool() {
            *value = x;
        }
    };
    b("enabled", &mut s.enabled);
    b("catmull_rom", &mut s.catmull_rom);
    b("spatial_start", &mut s.spatial_start);
    b("rotate_directions", &mut s.rotate_directions);
    b("silhouette_mix", &mut s.silhouette_mix);
    b("box_clamp", &mut s.box_clamp);
    s
}

/// `no_rt_renderer` with one more storage buffer per stage than `visibility_limits` asks
/// for. Since G1 (714a5a7) bound the radiance-cache table in the lighting group, the
/// non-RT Virtual pass's layout needs 11 and a device at `visibility_limits` (10) rejects it
/// (docs/m7-tr1.md records the defect; primary_ray.rs is outside TR1's files).
fn no_rt_renderer_11() -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: wgpu::Limits {
            max_storage_buffers_per_shader_stage: 11,
            ..crate::visibility_limits(wgpu::Limits::default())
        },
        memory_hints: super::test_memory_hints(),
        ..Default::default()
    }))
    .unwrap();
    assert!(!device.features().intersects(
        wgpu::Features::EXPERIMENTAL_RAY_QUERY | wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN
    ));
    let r = Renderer::new_with_options(
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
        include_str!("../shaders/scene.wgsl"),
    )
    .unwrap();
    assert_eq!(r.primary_visibility(), PrimaryVisibility::VisibilityBuffer);
    r
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
#[ignore = "owned GPU lease; TR1 registered sequences, offscreen (RENDERLAB_PRIMARY_MANIFEST)"]
fn gpu_tr1_sequence() {
    let started = Instant::now();
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let path = m["path"].as_str().unwrap_or("rt").to_owned();
    let validate = m["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let (mut r, mut asset) = match path.as_str() {
        "rt" => {
            let mut r = renderer(validate);
            let (asset, _) = load_quarry(&m, &mut r);
            r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
            r.set_soft_shadows(SoftShadows::Interactive);
            (r, asset)
        }
        "nonrt" | "nonrt-on-rt" => {
            let mut r = if path == "nonrt" {
                no_rt_renderer_11()
            } else {
                renderer_with_features(
                    validate,
                    crate::VIRTUAL_FEATURES | crate::VISIBILITY_RASTER_FEATURES,
                )
            };
            let (asset, _, _) = load_quarry_lod(&m, &mut r);
            if path == "nonrt-on-rt" {
                r.set_primary_visibility(PrimaryVisibility::VisibilityBuffer)
                    .unwrap();
            }
            r.set_soft_shadows(SoftShadows::Virtual);
            (r, asset)
        }
        other => panic!("unknown TR1 path {other}"),
    };
    let load_seconds = started.elapsed().as_secs_f64();
    let path_cameras: Vec<Camera> =
        cameras::Cameras::load(&PathBuf::from(m["sequence"].as_str().unwrap()))
            .unwrap()
            .frames
            .iter()
            .map(|f| f.camera().unwrap())
            .collect();
    let written: Vec<usize> = match m["frames"].as_array() {
        Some(v) => v.iter().map(|x| x.as_u64().unwrap() as usize).collect(),
        None => (0..path_cameras.len()).collect(),
    };
    let variants: Vec<(String, Reconstruction)> = m["variants"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, v)| (name.clone(), reconstruction(v)))
        .collect();
    let visits = m["visits"].as_u64().unwrap_or(0) as usize;
    let color = target(&r, 1920, 1080);
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    // One complete track from cold: frame hashes, the requested HDRs, and per-frame spans.
    let run = |r: &mut Renderer,
               asset: &mut prepared::Prepared,
               name: &str,
               settings: Reconstruction,
               write: bool| {
        r.set_reconstruction(Reconstruction::default()).unwrap();
        r.set_reconstruction(settings).unwrap();
        let dir = out.join(name);
        if write {
            fs::create_dir_all(&dir).unwrap();
        }
        let mut hashes = vec![];
        let mut pending = std::collections::HashMap::new();
        let mut spans = vec![serde_json::Value::Null; path_cameras.len()];
        let collect = |r: &mut Renderer,
                       pending: &mut std::collections::HashMap<u64, usize>,
                       spans: &mut Vec<serde_json::Value>| {
            for g in r.poll().unwrap().gpu {
                if let Some(f) = pending.remove(&g.frame.0) {
                    spans[f] = serde_json::json!({"span_ms":ms(g.render_span),
                        "receiver_ms":g.receiver_pass.map(ms),"shading_ms":ms(g.mesh_pass),
                        "reconstruction_ms":g.reconstruction_pass.map(ms),"present_ms":g.present_pass.map(ms)});
                }
            }
        };
        for (f, camera) in path_cameras.iter().enumerate() {
            let report = render(r, &color, camera, &mut asset.draws(Visibility::Frustum));
            pending.insert(report.frame.0, f);
            if written.contains(&f) {
                // Frames between captures fill every flight; a capture needs a free one.
                r.test_drain("TR1 capture");
                let hdr = capture(r);
                hashes.push(hex(&hdr));
                if write {
                    fs::write(dir.join(format!("f{f:03}.rgba16f")), &hdr).unwrap();
                }
            }
            collect(r, &mut pending, &mut spans);
        }
        r.test_drain("TR1 track");
        collect(r, &mut pending, &mut spans);
        (hashes, spans)
    };
    let mut records = serde_json::Map::new();
    let mut first: Option<Vec<String>> = None;
    for (name, settings) in &variants {
        let (hashes, spans) = run(&mut r, &mut asset, name, *settings, true);
        r.test_drain("TR1 memory");
        memory_report(&r, &out, name);
        if first.is_none() {
            first = Some(hashes.clone());
        }
        records.insert(
            name.clone(),
            serde_json::json!({"settings":format!("{settings:?}"),"hashes":hashes,"visits":[spans]}),
        );
        eprintln!("TR1 {path} {name} {:.1} s", started.elapsed().as_secs_f64());
    }
    // Alternating timing visits: each variant's track again, order reversed on odd visits.
    for visit in 0..visits {
        let mut order: Vec<_> = variants.iter().collect();
        if visit % 2 == 1 {
            order.reverse();
        }
        for (name, settings) in order {
            let (_, spans) = run(&mut r, &mut asset, name, *settings, false);
            records[name.as_str()]["visits"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(spans));
        }
    }
    let mut replay_hashes = vec![];
    let replay = if m["replay"].as_bool().unwrap_or(true) {
        let (name, settings) = &variants[0];
        let (hashes, _) = run(&mut r, &mut asset, name, *settings, false);
        replay_hashes = hashes;
        Some(Some(&replay_hashes) == first.as_ref())
    } else {
        None
    };
    fs::write(
        out.join("tr1.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "path": path, "sequence": m["sequence"], "frames": written, "visits": visits,
            "load_seconds": load_seconds, "seconds": started.elapsed().as_secs_f64(),
            "replay_identical": replay, "replay_hashes": replay_hashes, "variants": records,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_ne!(replay, Some(false), "TR1 replay differs");
}
