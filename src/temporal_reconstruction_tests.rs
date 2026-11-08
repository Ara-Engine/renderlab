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
//!
//! AA state (docs/iq1-quarry-image.md): 1x with TR1 per variant, anisotropy 16 (the test
//! renderers' clamp; the shipping default is 1).
//!
//! A sequence frame's `to_light` (Q-sun, S2G) sets the sun direction before that frame.
//! `scene` replaces the quarry with a U0b bench scene.json (S2G): its meshes, materials,
//! sun and constant sky, black primary misses, identity instances.
//! `visible_environment` shows the bound RLE2 map at primary misses (the visible-sky walk);
//! `integrate: n` writes each listed frame's n x n jittered no-AA mean instead of variants.
use super::*;

/// A loaded bench scene's draw inputs; placement i + 1 is the scene's mesh i, as the
/// reference's hit IDs number them.
struct Bench {
    meshes: Vec<crate::Mesh>,
    instances: Vec<Instances>,
    materials: Vec<Material>,
    material_of: Vec<usize>,
}

enum Content {
    Quarry(Box<prepared::Prepared>),
    Bench(Bench),
}

impl Content {
    fn draws(&mut self) -> Vec<Draw<'_>> {
        match self {
            Content::Quarry(asset) => asset.draws(Visibility::Frustum),
            Content::Bench(b) => b
                .meshes
                .iter()
                .zip(b.instances.iter_mut())
                .zip(&b.material_of)
                .map(|((m, i), &k)| Draw::new(m, i).with_material(&b.materials[k]))
                .collect(),
        }
    }
}

/// A constant sky of `sky` cd/m^2 as a prepared environment: its irradiance is pi times it.
fn constant_sky(sky: [f32; 3]) -> crate::asset::environment::PreparedEnvironment {
    use crate::asset::environment::{
        self as env, CubeLevel, Metadata, PreparedEnvironment, RadianceUnits,
    };
    let level = |size: u32, value: [f32; 3]| {
        CubeLevel::new(
            size,
            vec![value; (6 * size * size) as usize],
            env::Limits::default(),
        )
        .unwrap()
    };
    PreparedEnvironment::new(
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            source_radiance_scale: 1.0,
            rotation_y_radians: 0.0,
            generator_hash: [0x5c; 32],
        },
        level(8, sky.map(|v| v * std::f32::consts::PI)),
        [16, 8, 4, 2, 1]
            .into_iter()
            .map(|size| level(size, sky))
            .collect(),
        env::Limits::default(),
    )
    .unwrap()
}

/// Loads a bench scene.json (U0b schema 2) into `r`: lighting with its sun (disk radius as the
/// shadow disk), a black visible environment, its constant sky as the bound environment.
fn load_bench(path: &std::path::Path, r: &mut Renderer) -> (Bench, Lighting) {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let f = |x: &serde_json::Value| x.as_f64().unwrap() as f32;
    let v3 = |x: &serde_json::Value| [f(&x[0]), f(&x[1]), f(&x[2])];
    assert_eq!(v["camera_background"], serde_json::json!([0.0, 0.0, 0.0]));
    let sun = &v["sun"];
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: -Vec3::from(v3(&sun["to_light"])),
            lux: v3(&sun["lux"]),
        }],
        environment: [0.0; 3],
        exposure_stops: f(&v["exposure_stops"]),
    };
    let shadows = Shadows {
        directional_disk_angle: f(&sun["angular_radius"]),
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![1., 3., 10.],
        }],
        ..Default::default()
    };
    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    let env = r
        .try_upload_environment(&constant_sky(v3(&v["sky"]["constant"])))
        .unwrap()
        .unwrap();
    r.set_environment(Some(&env), EnvironmentParameters::default())
        .unwrap();
    r.test_drain("TR1 bench environment");
    let materials = v["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            assert_eq!(f(&m["specular"]), 1.0, "Renderlab's dielectric F0 is 0.04");
            let b = v3(&m["base_color"]);
            loop {
                let descriptor = MaterialDescriptor {
                    base_color: [b[0], b[1], b[2], 1.0],
                    metallic: f(&m["metallic"]),
                    roughness: f(&m["roughness"]),
                    double_sided: m["double_sided"].as_bool().unwrap_or(false),
                    ..Default::default()
                };
                if let Some(material) = r.try_create_material(&descriptor).unwrap() {
                    break material;
                }
                r.test_drain("TR1 bench material");
            }
        })
        .collect();
    let mut bench = Bench {
        meshes: vec![],
        instances: vec![],
        materials,
        material_of: vec![],
    };
    for m in v["meshes"].as_array().unwrap() {
        let list = |k: &str| m[k].as_array().unwrap().iter().map(v3).collect::<Vec<_>>();
        let vertices: Vec<Vertex> = list("positions")
            .into_iter()
            .zip(list("normals"))
            .map(|(position, normal)| Vertex {
                position,
                normal,
                uv: [0.0; 2],
                color: [1.0; 4],
            })
            .collect();
        let indices: Vec<u32> = m["indices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_u64().unwrap() as u32)
            .collect();
        let mesh = loop {
            let data = MeshData {
                vertices: &vertices,
                indices: &indices,
            };
            if let Some(mesh) = r.try_upload_mesh(&data).unwrap() {
                break mesh;
            }
            r.test_drain("TR1 bench mesh");
        };
        let instances = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.test_drain("TR1 bench instances");
        };
        bench.meshes.push(mesh);
        bench.instances.push(instances);
        bench
            .material_of
            .push(m["material"].as_u64().unwrap() as usize);
    }
    r.test_drain("TR1 bench upload");
    (bench, lighting)
}

/// Each sequence frame's `to_light`, if it has one.
fn sequence_suns(path: &std::path::Path) -> Vec<Option<Vec3>> {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    v["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            f["to_light"].as_array().map(|d| {
                Vec3::new(
                    d[0].as_f64().unwrap() as f32,
                    d[1].as_f64().unwrap() as f32,
                    d[2].as_f64().unwrap() as f32,
                )
            })
        })
        .collect()
}

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
    f("coverage_window", &mut s.coverage_window);
    f("start_window", &mut s.start_window);
    f("turn_angle", &mut s.turn_angle);
    f("start_radius", &mut s.start_radius);
    let b = |key: &str, value: &mut bool| {
        if let Some(x) = v[key].as_bool() {
            *value = x;
        }
    };
    b("enabled", &mut s.enabled);
    b("catmull_rom", &mut s.catmull_rom);
    b("spatial_start", &mut s.spatial_start);
    b("rotate_directions", &mut s.rotate_directions);
    s
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
    let mut r = match path.as_str() {
        "rt" => renderer(validate),
        "nonrt" => no_rt_renderer(),
        "nonrt-on-rt" => renderer_with_features(
            validate,
            crate::VIRTUAL_FEATURES | crate::VISIBILITY_RASTER_FEATURES,
        ),
        other => panic!("unknown TR1 path {other}"),
    };
    let quarry_lighting = || {
        lighting_settings::parse(&fs::read_to_string(m["lighting"].as_str().unwrap()).unwrap())
            .unwrap()
    };
    let (mut content, base_lighting) = match m["scene"].as_str() {
        Some(scene) => {
            let (bench, lighting) = load_bench(std::path::Path::new(scene), &mut r);
            (Content::Bench(bench), lighting)
        }
        None if path == "rt" => (
            Content::Quarry(Box::new(load_quarry(&m, &mut r).0)),
            quarry_lighting(),
        ),
        None => (
            Content::Quarry(Box::new(load_quarry_lod(&m, &mut r).0)),
            quarry_lighting(),
        ),
    };
    if path == "rt" {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r.set_soft_shadows(SoftShadows::Interactive);
    } else {
        if path == "nonrt-on-rt" {
            r.set_primary_visibility(PrimaryVisibility::VisibilityBuffer)
                .unwrap();
        }
        r.set_soft_shadows(SoftShadows::Virtual);
    }
    // `visible_environment`: primary misses show the bound map (the visible-sky walk).
    r.set_visible_environment(m["visible_environment"].as_bool().unwrap_or(false));
    let suns = sequence_suns(&PathBuf::from(m["sequence"].as_str().unwrap()));
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
    // `write_frames: false` hashes the listed frames without writing them (repeats).
    let write_frames = m["write_frames"].as_bool().unwrap_or(true);
    let color = target(&r, 1920, 1080);
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    // `integrate: n`: each listed frame's pixel integral of the renderer's own shading, the
    // mean of n x n stratified sub-pixel offsets with TR1 off, as raw f32 RGB
    // `integrated/fNNN.f32` (exposed). The visible-sky gate's lighting-matched control
    // scores TR1 and no-AA against it; no variants run.
    if let Some(n) = m["integrate"].as_u64().map(|n| n as u32) {
        assert!(
            suns.iter().all(Option::is_none),
            "integrate: fixed-sun tracks only"
        );
        r.set_reconstruction(Reconstruction {
            enabled: false,
            ..Reconstruction::default()
        })
        .unwrap();
        let dir = out.join("integrated");
        fs::create_dir_all(&dir).unwrap();
        for &f in &written {
            let mut sum = vec![0.0f64; 1920 * 1080 * 3];
            for j in 0..n * n {
                let offset = [j % n, j / n].map(|k| (k as f32 + 0.5) / n as f32 - 0.5);
                render(
                    &mut r,
                    &color,
                    &path_cameras[f].jittered(offset),
                    &mut content.draws(),
                );
                r.test_drain("TR1 integrate");
                let hdr = capture(&mut r);
                let rgb = hdr.chunks_exact(8).flat_map(|p| {
                    (0..3).map(move |k| {
                        crate::decode_f16(u16::from_le_bytes([p[2 * k], p[2 * k + 1]]))
                    })
                });
                for (s, v) in sum.iter_mut().zip(rgb) {
                    *s += f64::from(v);
                }
            }
            let mean: Vec<u8> = sum
                .iter()
                .flat_map(|s| ((s / f64::from(n * n)) as f32).to_le_bytes())
                .collect();
            fs::write(dir.join(format!("f{f:03}.f32")), mean).unwrap();
        }
        fs::write(
            out.join("integrated.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"path": path, "n": n, "frames": written,
                "seconds": started.elapsed().as_secs_f64()}))
            .unwrap(),
        )
        .unwrap();
        return;
    }
    // One complete track from cold: frame hashes, the requested HDRs, and per-frame spans.
    let run = |r: &mut Renderer,
               content: &mut Content,
               name: &str,
               settings: Reconstruction,
               write: bool| {
        r.set_reconstruction(Reconstruction::default()).unwrap();
        r.set_reconstruction(settings).unwrap();
        // Every run starts from the same shadow state: the non-RT Surface cache keeps cells
        // across frames, so a replay after other runs would otherwise start warm.
        r.reset_virtual_shadow_cache();
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
        let mut sun = None;
        for (f, camera) in path_cameras.iter().enumerate() {
            if let Some(d) = suns[f]
                && sun != Some(d)
            {
                let mut lighting = base_lighting.clone();
                if let Light::Directional { travel, .. } = &mut lighting.lights[0] {
                    *travel = -d;
                }
                r.set_lighting(&lighting).unwrap();
                sun = Some(d);
            }
            let report = render(r, &color, camera, &mut content.draws());
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
        let (hashes, spans) = run(&mut r, &mut content, name, *settings, write_frames);
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
            let (_, spans) = run(&mut r, &mut content, name, *settings, false);
            records[name.as_str()]["visits"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(spans));
        }
    }
    let mut replay_hashes = vec![];
    let replay = if m["replay"].as_bool().unwrap_or(true) {
        let (name, settings) = &variants[0];
        let (hashes, _) = run(&mut r, &mut content, name, *settings, false);
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
    // Recorded, not asserted, so one path's replay never cancels the plan's other paths;
    // the F8 gate reads `replay_identical` (and cross-process hashes).
    if replay == Some(false) {
        eprintln!("TR1 replay differs");
    }
}
