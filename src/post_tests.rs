//! PP1's one-process paired effect sweep. Run only through scripts/n8-run.py.
use super::*;

#[test]
fn integrated_reflection_spirv_has_no_ray_query_null() {
    let sources = primary_ray_gpu::shading_sources(
        include_str!("../shaders/scene.wgsl"),
        crate::STANDARD_PROGRAM,
    )
    .unwrap();
    for (name, source) in ["hard", "soft"].into_iter().zip(sources) {
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for entry in &module.entry_points {
            let (module, info) = naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                Some((entry.stage, &entry.name)),
                &Default::default(),
            )
            .unwrap();
            let words = naga::back::spv::write_vec(
                &module,
                &info,
                &naga::back::spv::Options {
                    lang_version: (1, 6),
                    ..Default::default()
                },
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: entry.stage,
                    entry_point: entry.name.clone(),
                }),
            )
            .unwrap();
            let mut at = 5;
            let mut queries = vec![];
            let mut nulls = vec![];
            while at < words.len() {
                let n = (words[at] >> 16) as usize;
                match words[at] & 0xffff {
                    4472 => queries.push(words[at + 1]), // OpTypeRayQueryKHR
                    46 => nulls.push(words[at + 1]),     // OpConstantNull's result type
                    _ => {}
                }
                at += n;
            }
            assert!(
                !nulls.iter().any(|ty| queries.contains(ty)),
                "{name}/{} contains an illegal ray-query null",
                entry.name
            );
        }
    }
}

fn neutral() -> PostSettings {
    PostSettings {
        bloom: 0.0,
        motion_blur: false,
        vignette: 0.0,
        ..Default::default()
    }
}
fn variants() -> Vec<(&'static str, PostSettings)> {
    let n = neutral();
    vec![
        ("off", PostSettings::disabled()),
        ("neutral", n),
        ("default", PostSettings::default()),
        ("bloom", PostSettings { bloom: 0.035, ..n }),
        (
            "motion",
            PostSettings {
                motion_blur: true,
                ..n
            },
        ),
        (
            "dof",
            PostSettings {
                depth_of_field: Some(DepthOfField {
                    focus_distance_m: 3.0,
                    f_number: 1.4,
                    ..Default::default()
                }),
                ..n
            },
        ),
        (
            "dof-near",
            PostSettings {
                depth_of_field: Some(DepthOfField {
                    focus_distance_m: 6.0,
                    f_number: 0.8,
                    ..Default::default()
                }),
                ..n
            },
        ),
        (
            "grade",
            PostSettings {
                white_balance_kelvin: 5800.0,
                contrast: 1.05,
                saturation: 1.05,
                ..n
            },
        ),
        (
            "filmic",
            PostSettings {
                tonemapper: Tonemapper::Filmic,
                ..n
            },
        ),
        (
            "vignette",
            PostSettings {
                vignette: 0.035,
                ..n
            },
        ),
        ("grain", PostSettings { grain: 0.006, ..n }),
        (
            "aberration",
            PostSettings {
                chromatic_aberration_px: 0.5,
                ..n
            },
        ),
        ("lut", n),
    ]
}
fn png(path: &std::path::Path, size: [u32; 2], bytes: &[u8]) {
    let mut e = png::Encoder::new(fs::File::create(path).unwrap(), size[0], size[1]);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(bytes).unwrap();
}
fn timings(r: &mut Renderer) -> Vec<serde_json::Value> {
    r.poll()
        .unwrap()
        .gpu
        .into_iter()
        .map(|g| {
            serde_json::json!({
        "post_tone_ms":g.present_pass.map(|d|d.as_secs_f64()*1000.0),
        "span_ms":g.render_span.as_secs_f64()*1000.0})
        })
        .collect()
}
fn sweep(
    r: &mut Renderer,
    out: &std::path::Path,
    cameras: &[Camera],
    size: [u32; 2],
    mut draw: impl FnMut(&mut Renderer, &wgpu::Texture, &Camera),
) {
    fs::create_dir_all(out).unwrap();
    let colour = target(r, size[0], size[1]);
    let mut rows = serde_json::Map::new();
    let mut off = None;
    for (name, settings) in variants() {
        r.test_drain("PP1 setting boundary");
        r.set_post_settings(settings).unwrap();
        let lut = if name == "lut" {
            let mut lut = ColourLut::identity(16).unwrap();
            for rgb in &mut lut.rgb {
                rgb[0] = (rgb[0] * 1.03).min(1.0);
                rgb[2] *= 0.97;
            }
            Some(lut)
        } else {
            None
        };
        r.set_colour_lut(lut).unwrap();
        r.set_reconstruction(Reconstruction::off()).unwrap();
        let scale = match manifest()["render_scale"].as_str() {
            Some("quality") => RenderScale::Quality,
            Some("performance") => RenderScale::Performance,
            _ => RenderScale::Native,
        };
        r.set_reconstruction(Reconstruction {
            scale,
            ..Reconstruction::identity()
        })
        .unwrap();
        r.reset_virtual_shadow_cache();
        // Replay the same bounded GI refill for every post variant.
        r.reset_indirect();
        r.cut_post_motion();
        r.cut_reconstruction();
        for _ in 0..8 {
            draw(r, &colour, &cameras[0]);
        }
        r.test_drain("PP1 static");
        let static_image = read_texture(r, &colour);
        png(&out.join(format!("{name}-still.png")), size, &static_image);
        let hdr = capture(r);
        if name == "off" {
            off = Some(hdr.clone());
        }
        if name == "neutral" {
            assert!(
                off.as_ref().unwrap() == &hdr,
                "post must not feed back into HDR/TR1"
            );
        }
        let _ = timings(r);
        // Eight moving frames: one loaded scene, no full-buffer readback for timings.
        let mut costs = vec![];
        for camera in cameras.iter().cycle().take(8) {
            draw(r, &colour, camera);
            r.test_drain("PP1 effect timing");
            costs.extend(timings(r));
        }
        let moving = read_texture(r, &colour);
        png(&out.join(format!("{name}-moving.png")), size, &moving);
        rows.insert(name.into(),serde_json::json!({"frames":costs,"hdr_sha256":hex(&hdr),"display_sha256":hex(&moving)}));
    }
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "PP1 owned GPU lease; both paths, quarry, bright-sky fixture, per-effect timings"]
fn gpu_post_sweep() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    let path = m["path"].as_str().unwrap_or("nonrt");
    let _validation = validation::begin(true);
    let mut r = if path == "rt" || path == "raster" {
        renderer(true)
    } else {
        no_rt_renderer()
    };
    if path == "rt" {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    // AT1 is intentionally off in ordinary lib-test constructors. Exercise the
    // product sky/fog here, including its contribution before TR1 resolves.
    r.set_atmosphere(Atmosphere::default()).unwrap();
    r.set_indirect(IndirectSettings::default()).unwrap();
    r.set_reconstruction(Reconstruction::identity()).unwrap();
    if m["fixture_only"].as_bool() != Some(true) {
        let (mut asset, cameras) = if path == "rt" || path == "raster" {
            load_quarry(&m, &mut r)
        } else {
            let (asset, cameras, _) = load_quarry_lod(&m, &mut r);
            (asset, cameras)
        };
        let poses: Vec<_> = cameras
            .frames
            .iter()
            .take(8)
            .map(|f| f.camera().unwrap())
            .collect();
        sweep(
            &mut r,
            &out.join("quarry"),
            &poses,
            [1920, 1080],
            |r, c, camera| {
                render(r, c, camera, &mut asset.draws(Visibility::Frustum));
            },
        );
        drop(asset);
        r.test_drain("PP1 quarry release");
    }
    // High-radiance sky, dark near silhouettes, far wall and a bright small emitter.
    r.set_environment(None, EnvironmentParameters::default())
        .unwrap();
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: -Vec3::new(0.4, 0.8, 0.3).normalize(),
                lux: [120_000.0; 3],
            }],
            environment: [5.0, 7.0, 12.0],
            exposure_stops: -10.0,
            ..Default::default()
        },
        &Shadows::default(),
    )
    .unwrap();
    r.test_reload_standard(&standard_returning("input.color.rgb"))
        .unwrap();
    let positions = [
        [-1.8, -1.2, 0.0],
        [0.2, -1.2, 0.0],
        [0.2, 1.2, 0.0],
        [-1.8, 1.2, 0.0],
        [0.6, -1.0, -5.0],
        [4.0, -1.0, -5.0],
        [4.0, 2.0, -5.0],
        [0.6, 2.0, -5.0],
        [0.2, 0.4, -2.0],
        [0.4, 0.4, -2.0],
        [0.3, 0.6, -2.0],
    ];
    let vertices: Vec<_> = positions
        .into_iter()
        .enumerate()
        .map(|(i, position)| Vertex {
            position: [position[0], position[1] + 1.5, position[2]],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0; 2],
            color: if i < 4 {
                [0.01, 0.015, 0.025, 1.0]
            } else if i < 8 {
                [0.25, 0.3, 0.35, 1.0]
            } else {
                [50.0, 25.0, 10.0, 1.0]
            },
        })
        .collect();
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10],
        })
        .unwrap()
        .unwrap();
    r.test_drain("PP1 sky fixture upload");
    if path == "nonrt" {
        let world: Vec<_> = vertices.iter().map(|v| v.position).collect();
        r.set_indirect_geometry(
            &mesh,
            &world,
            &[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10],
        )
        .unwrap();
    }
    let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("PP1 sky fixture placements");
    let poses: Vec<_> = (0..8)
        .map(|i| {
            Camera::look_at(
                Vec3::new(i as f32 * 0.025, 1.5, 4.0),
                Vec3::new(i as f32 * 0.025, 1.5, 0.0),
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                100.0,
            )
            .unwrap()
        })
        .collect();
    if path == "rt" && m["fixture_only"].as_bool() == Some(true) {
        // A fixed-size raster -> traced switch must prepare the new post surface ABI.
        let colour = target(&r, 1280, 720);
        r.set_post_settings(PostSettings::default()).unwrap();
        r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
        render(
            &mut r,
            &colour,
            &poses[0],
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("PP1 raster before path switch");
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        render(
            &mut r,
            &colour,
            &poses[0],
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("PP1 traced after path switch");
    }
    sweep(
        &mut r,
        &out.join("bright-sky"),
        &poses,
        [1280, 720],
        |r, c, camera| {
            render(r, c, camera, &mut [Draw::new(&mesh, &mut instances)]);
        },
    );
    r.test_drain("PP1 finished");
    assert!(r.fault.get().is_none());
}
