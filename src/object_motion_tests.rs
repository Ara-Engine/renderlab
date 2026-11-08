//! MV1's rigid-motion oracle and retained moving-object images. n8 lease required.
use super::*;

fn png(path: &std::path::Path, texture: &wgpu::Texture, r: &Renderer) {
    let mut e = png::Encoder::new(
        fs::File::create(path).unwrap(),
        texture.width(),
        texture.height(),
    );
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .unwrap()
        .write_image_data(&read_texture(r, texture))
        .unwrap();
}
fn words(r: &Renderer, texture: &wgpu::Texture) -> Vec<[u32; 4]> {
    read_texture(r, texture)
        .chunks_exact(16)
        .map(|b| {
            std::array::from_fn(|i| u32::from_ne_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
        })
        .collect()
}
fn costs(r: &mut Renderer) -> Vec<serde_json::Value> {
    r.poll()
        .unwrap()
        .gpu
        .into_iter()
        .map(|g| {
            serde_json::json!({
                "span_ms":g.render_span.as_secs_f64()*1000.,
                "post_ms":g.present_pass.map(|v|v.as_secs_f64()*1000.),
            })
        })
        .collect()
}
fn cube(r: &mut Renderer) -> (Mesh, Instances) {
    let mut vertices = vec![];
    let mut indices = vec![];
    for (normal, u, v) in [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ] {
        let base = vertices.len() as u32;
        for (s, t) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
            vertices.push(Vertex {
                position: (0.5 * (normal + s * u + t * v)).to_array(),
                normal: normal.to_array(),
                uv: [0.5 + 0.5 * s, 0.5 - 0.5 * t],
                color: [0.9, 0.12, 0.04, 1.],
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })
        .unwrap()
        .unwrap();
    r.test_drain("MV1 cube upload");
    let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("MV1 cube placements");
    (mesh, instances)
}

#[test]
#[ignore = "MV1 owned n8 GPU lease; RENDERLAB_MQ1_RESOLVE is the immutable main shader"]
fn gpu_mv1_mq1_static_identity() {
    use sha2::{Digest, Sha256};
    let baseline = fs::read_to_string(std::env::var("RENDERLAB_MQ1_RESOLVE").unwrap()).unwrap();
    for path in ["rt", "nonrt"] {
        let mut r = if path == "rt" {
            renderer(true)
        } else {
            no_rt_renderer()
        };
        if path == "rt" {
            r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        }
        r.set_shadows(&Shadows::default()).unwrap();
        r.set_indirect(IndirectSettings::off()).unwrap();
        r.set_reconstruction(Reconstruction::default()).unwrap();
        r.test_reload_standard(&standard_returning(
            "select(vec3<f32>(0.04,0.1,0.2),vec3<f32>(0.9,0.6,0.12),fract(input.uv.x*8.0)>0.5)",
        ))
        .unwrap();
        let (mesh, mut placement) = cube(&mut r);
        placement
            .set(
                0,
                Mat4::from_translation(Vec3::new(0.12, 0.07, 0.))
                    * Mat4::from_rotation_y(0.37)
                    * Mat4::from_rotation_z(0.12),
            )
            .unwrap();
        let color = target(&r, 160, 96);
        let camera = |f: u32, moving: bool| {
            Camera::look_at(
                Vec3::new(if moving { f as f32 * 0.005 } else { 0. }, 0.3, 3.),
                Vec3::ZERO,
                Vec3::Y,
                0.8,
                0.05,
                100.,
            )
            .unwrap()
        };
        render(
            &mut r,
            &color,
            &camera(0, false),
            &mut [Draw::new(&mesh, &mut placement)],
        );
        r.test_drain("MV1 static control warmup");
        for moving in [false, true] {
            let mut control = None;
            for mq1 in [true, false] {
                r.test_resolve_source(mq1.then_some(baseline.as_str()));
                r.cut_reconstruction();
                r.reconstruction_index = 0;
                r.cut_post_motion();
                for f in 0..32 {
                    render(
                        &mut r,
                        &color,
                        &camera(f, moving),
                        &mut [Draw::new(&mesh, &mut placement)],
                    );
                    r.test_drain("MV1 static identity");
                }
                let bytes = read_texture(&r, &r.lighting.hdr.as_ref().unwrap().resolved.texture);
                let hash = format!("{:x}", Sha256::digest(&bytes));
                eprintln!("MV1 static identity {path} camera_moving={moving} mq1={mq1} HDR={hash}");
                if let Some(expected) = &control {
                    assert!(
                        &bytes == expected,
                        "{path}, moving camera {moving}: static HDR changed ({hash})"
                    );
                } else {
                    control = Some(bytes);
                }
            }
        }
        assert!(r.fault.get().is_none());
    }
}

#[test]
#[ignore = "MV1 owned n8 GPU lease; RENDERLAB_PRIMARY_MANIFEST selects path/output/quarry"]
fn gpu_object_motion() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let _validation = validation::begin(true);
    let rt = m["path"] == "rt";
    let mut r = if rt { renderer(true) } else { no_rt_renderer() };
    if rt {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    r.set_shadows(&Shadows::default()).unwrap();
    r.set_indirect(IndirectSettings::off()).unwrap();
    r.test_reload_standard(&standard_returning(
        "select(vec3<f32>(0.06,0.1,0.2),vec3<f32>(0.9,0.6,0.12),fract(input.uv.x*8.0)>0.5)",
    ))
    .unwrap();
    let vertices: Vec<_> = [
        ([-0.8, -0.6, 0.], [0., 0.]),
        ([0.8, -0.6, 0.], [1., 0.]),
        ([0.8, 0.6, 0.], [1., 1.]),
        ([-0.8, 0.6, 0.], [0., 1.]),
    ]
    .into_iter()
    .map(|(position, uv)| Vertex {
        position,
        normal: [0., 0., 1.],
        uv,
        color: [1.; 4],
    })
    .collect();
    let indices = [0, 1, 2, 0, 2, 3];
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })
        .unwrap()
        .unwrap();
    r.test_drain("MV1 fixture mesh");
    let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("MV1 fixture placement");
    let camera =
        Camera::look_at(Vec3::new(0., 0., 4.), Vec3::ZERO, Vec3::Y, 0.8, 0.05, 100.).unwrap();
    let size = [320, 192];
    let color = target(&r, size[0], size[1]);
    let project = |point: Vec3| {
        let p = camera.matrix(size[0], size[1]) * point.extend(1.);
        (p.truncate().truncate() / p.w * glam::Vec2::new(0.5, -0.5) + glam::Vec2::splat(0.5))
            * glam::Vec2::new(size[0] as f32, size[1] as f32)
    };
    let transform = |frame: u32| {
        Mat4::from_translation(Vec3::new(
            -0.6 + frame as f32 * 0.075,
            0.15 * (frame as f32 * 0.3).sin(),
            0.,
        )) * Mat4::from_rotation_z(frame as f32 * 0.045)
    };
    let neutral = PostSettings {
        bloom: 0.,
        motion_blur: false,
        vignette: 0.,
        ..Default::default()
    };
    let mut rows = vec![];
    for blur in [false, true] {
        r.set_post_settings(PostSettings {
            motion_blur: blur,
            ..neutral
        })
        .unwrap();
        r.set_reconstruction(Reconstruction::identity()).unwrap();
        r.cut_reconstruction();
        r.cut_post_motion();
        for frame in 0..24 {
            let model = transform(frame);
            instances.set(0, model).unwrap();
            render(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut instances)],
            );
            r.test_drain("MV1 fixture frame");
            if frame > 0 {
                assert!(
                    !r.reconstruction_state.as_ref().unwrap().reset,
                    "rigid motion reset TR1"
                );
            }
            let timing = costs(&mut r);
            if frame >= 16 {
                rows.extend(timing);
            }
            if [0, 8, 16, 23].contains(&frame) {
                png(
                    &out.join(format!("fixture-blur{blur}-{frame:02}.png")),
                    &color,
                    &r,
                );
            }
            if blur && [0, 8, 23].contains(&frame) {
                let aux = words(&r, r.post.as_ref().unwrap().test_surface());
                let hits = words(
                    &r,
                    &r.primary
                        .as_ref()
                        .unwrap()
                        .targets
                        .as_ref()
                        .unwrap()
                        .hit
                        .texture,
                );
                let previous = if frame == 0 {
                    model
                } else {
                    transform(frame - 1)
                };
                let mut count = 0;
                let mut max_error = 0f32;
                let mut max_speed = 0f32;
                for (hit, v) in hits.iter().zip(&aux) {
                    if hit[0] != 1 || f32::from_bits(v[2]) > 1e6 {
                        continue;
                    }
                    let i = hit[1] as usize * 3;
                    let u = f32::from_bits(hit[2]);
                    let v_b = f32::from_bits(hit[3]);
                    let local = Vec3::from(vertices[indices[i] as usize].position) * (1. - u - v_b)
                        + Vec3::from(vertices[indices[i + 1] as usize].position) * u
                        + Vec3::from(vertices[indices[i + 2] as usize].position) * v_b;
                    let mut expected = (project(model.transform_point3(local))
                        - project(previous.transform_point3(local)))
                        * 0.25;
                    expected *= (16. / expected.length().max(1e-5)).min(1.);
                    let actual = glam::Vec2::new(f32::from_bits(v[0]), f32::from_bits(v[1]));
                    max_error = max_error.max((actual - expected).length());
                    max_speed = max_speed.max(actual.length());
                    count += 1;
                }
                assert!(count > 1000);
                assert!(max_error < 0.002, "motion error {max_error}");
                if frame > 0 {
                    assert!(max_speed > 0.8, "object has no motion");
                }
                rows.push(serde_json::json!({"frame":frame,"velocity_pixels":count,"max_error_px":max_error,"max_half_extent_px":max_speed}));
            }
        }
        let history = words(&r, r.reconstruction_state.as_ref().unwrap().test_history());
        let hits = words(
            &r,
            &r.primary
                .as_ref()
                .unwrap()
                .targets
                .as_ref()
                .unwrap()
                .hit
                .texture,
        );
        let tiles = &r
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .tiles
            .texture;
        let tile_width = tiles.width() as usize;
        let region = r.primary.as_ref().unwrap().scene;
        let [tile_x, tile_y] = primary_ray_gpu::TRACE_TILE.map(|v| v as usize);
        let flags: Vec<_> = read_texture(&r, tiles)
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let weights: Vec<_> = history
            .iter()
            .zip(&hits)
            .enumerate()
            .filter(|(p, (_, hit))| {
                let (x, y) = (p % size[0] as usize, p / size[0] as usize);
                hit[0] == 1
                    && x >= region[0] as usize
                    && x < region[2] as usize
                    && y >= region[1] as usize
                    && y < region[3] as usize
                    && flags[y / tile_y * tile_width + x / tile_x] != 0
            })
            .map(|(_, (v, _))| f32::from_bits(v[3]))
            .collect();
        let max_weight = weights.iter().copied().fold(0f32, f32::max);
        let continued = weights.iter().filter(|&&v| v > 2.).count();
        assert!(
            continued > weights.len() / 4,
            "moving surface history did not continue: {continued}/{}",
            weights.len()
        );
        rows.push(
            serde_json::json!({"blur":blur,"max_history_weight":max_weight,
            "moving_pixels":weights.len(),"continued_moving_pixels":continued}),
        );
        // A cleared strip behind the moving object must not retain its old colour.
        // Exclude the expanded current bounding box, including its antialiased edge.
        let corners: Vec<_> = vertices
            .iter()
            .map(|v| project(transform(23).transform_point3(Vec3::from(v.position))))
            .collect();
        let lo = corners
            .iter()
            .copied()
            .fold(glam::Vec2::splat(f32::INFINITY), glam::Vec2::min)
            - glam::Vec2::splat(3.);
        let hi = corners
            .iter()
            .copied()
            .fold(glam::Vec2::splat(f32::NEG_INFINITY), glam::Vec2::max)
            + glam::Vec2::splat(3.);
        let hdr = capture(&mut r);
        let clear = r.lit_clear().map(|v| v as f32);
        let mut max_trail = 0f32;
        for (pixel, b) in hdr.chunks_exact(8).enumerate() {
            let p = glam::Vec2::new(
                (pixel % size[0] as usize) as f32 + 0.5,
                (pixel / size[0] as usize) as f32 + 0.5,
            );
            if p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y {
                continue;
            }
            for c in 0..3 {
                let value = decode_f16(u16::from_le_bytes(b[c * 2..c * 2 + 2].try_into().unwrap()));
                max_trail = max_trail.max((value - clear[c]).abs());
            }
        }
        assert!(max_trail < 0.002, "TR1 left a ghost trail: {max_trail}");
        rows.push(serde_json::json!({"blur":blur,"max_hdr_background_trail":max_trail}));
    }
    // Stop: the next submitted frame must have no residual velocity.
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("MV1 stop");
    let max_stop = words(&r, r.post.as_ref().unwrap().test_surface())
        .iter()
        .map(|v| glam::Vec2::new(f32::from_bits(v[0]), f32::from_bits(v[1])).length())
        .fold(0f32, f32::max);
    assert!(
        max_stop < 0.002,
        "stopped object retained velocity {max_stop}"
    );
    rows.push(serde_json::json!({"max_stopped_half_extent_px":max_stop}));
    // A diagnostic frame advances instance history without advancing TR1/post's
    // camera history. Resuming lit output must cut the mismatched histories.
    let resumed =
        Camera::look_at(Vec3::new(0.3, 0., 4.), Vec3::ZERO, Vec3::Y, 0.8, 0.05, 100.).unwrap();
    instances.set(0, transform(24)).unwrap();
    loop {
        if r.try_render_view(
            &color,
            &resumed,
            &mut [Draw::new(&mesh, &mut instances)],
            ViewMode::Normal,
        )
        .unwrap()
        .is_some()
        {
            break;
        }
        r.collect().unwrap();
    }
    render(
        &mut r,
        &color,
        &resumed,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("MV1 resume after diagnostic view");
    assert!(r.reconstruction_state.as_ref().unwrap().reset);
    let max_resumed = words(&r, r.post.as_ref().unwrap().test_surface())
        .iter()
        .map(|v| glam::Vec2::new(f32::from_bits(v[0]), f32::from_bits(v[1])).length())
        .fold(0f32, f32::max);
    assert_eq!(max_resumed, 0.);
    rows.push(serde_json::json!({"diagnostic_resume_reset":true,"max_resumed_half_extent_px":max_resumed}));
    drop(mesh);
    drop(instances);
    r.test_drain("MV1 fixture release");
    if m["fixture_only"].as_bool() != Some(true) {
        r.test_reload_standard(crate::STANDARD_PROGRAM).unwrap();
        r.set_post_settings(PostSettings::default()).unwrap();
        r.set_indirect(IndirectSettings::default()).unwrap();
        r.set_atmosphere(Atmosphere::default()).unwrap();
        if rt {
            r.set_soft_shadows(SoftShadows::Interactive);
        }
        let (mut asset, cameras) = if rt {
            load_quarry(&m, &mut r)
        } else {
            let (a, c, _) = load_quarry_lod(&m, &mut r);
            (a, c)
        };
        let camera = cameras.frames[0].camera().unwrap();
        let basis = camera.view().inverse();
        let (mesh, mut placement) = cube(&mut r);
        let target = target(&r, 960, 540);
        r.cut_reconstruction();
        r.cut_post_motion();
        for frame in 0..24 {
            let pos = basis.transform_point3(Vec3::new((frame as f32 - 12.) * 0.06, -0.25, -4.));
            placement
                .set(
                    0,
                    Mat4::from_translation(pos)
                        * Mat4::from_rotation_y(frame as f32 * 0.08)
                        * Mat4::from_scale(Vec3::splat(0.75)),
                )
                .unwrap();
            let mut draws = asset.draws(Visibility::Frustum);
            draws.push(Draw::new(&mesh, &mut placement));
            render(&mut r, &target, &camera, &mut draws);
            r.test_drain("MV1 quarry moving caster");
            if [0, 8, 16, 23].contains(&frame) {
                png(&out.join(format!("quarry-{frame:02}.png")), &target, &r);
            }
            let t = costs(&mut r);
            if frame >= 16 {
                rows.extend(t);
            }
        }
    }
    fs::write(
        out.join("motion.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    assert!(r.fault.get().is_none());
}
