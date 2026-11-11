//! DC1's one-process-per-path fixture, quarry images and eight paired timing samples.
use super::*;
use crate::{Decal, DecalBlend, DecalTexture, DecalTextureKind, DecalTextures};
#[path = "../support/validation.rs"]
mod validation;

fn box_transform(center: Vec3, normal: Vec3, size: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        size,
        glam::Quat::from_rotation_arc(Vec3::Z, normal.normalize()),
        center,
    )
}
fn painted_texture(kind: usize) -> DecalTexture {
    let size = 128u32;
    let mut data = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let p = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / size as f32 * 2.
                - glam::Vec2::ONE;
            let noise = ((x * 17 + y * 31 + (x * y) % 47) % 79) as f32 / 79.;
            let (rgb, alpha) = match kind {
                0 => (
                    [45. + noise * 38., 85. + noise * 55., 20. + noise * 28.],
                    ((0.94 - p.length() + 0.2 * noise) * 8.).clamp(0., 1.),
                ),
                1 => (
                    [245., 90., 24.],
                    if (p.x - p.y * 0.35).abs() < 0.16
                        || (p.y - 0.3).abs() < 0.12 && p.x.abs() < 0.6
                    {
                        0.95
                    } else {
                        0.
                    },
                ),
                _ => (
                    [38., 48., 54.],
                    ((0.88 - (p * glam::Vec2::new(1., 1.3)).length() + 0.08 * noise) * 10.)
                        .clamp(0., 1.),
                ),
            };
            data.extend([
                rgb[0] as u8,
                rgb[1] as u8,
                rgb[2] as u8,
                (alpha * 255.) as u8,
            ]);
        }
    }
    DecalTexture::from_rgba8(DecalTextureKind::BaseColorSrgb, [size, size], &data).unwrap()
}
fn fixture(path: Path_, device: (wgpu::Device, wgpu::Queue), out: &Path) -> Json {
    let sv = Sv {
        meshes: vec![BenchMesh {
            name: "receiver".into(),
            positions: vec![[-2., 0., -2.], [-2., 0., 2.], [2., 0., 2.], [2., 0., -2.]],
            normals: vec![[0., 1., 0.]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            material: 0,
        }],
        materials: vec![[0.2, 0.3, 0.4, 0.1, 0.8]],
        poses: vec![],
        sky: [0.5; 3],
        exposure: 0.,
        size: [256, 256],
        fov: 0.8,
        populations: vec![],
        sha: "dc1".into(),
    };
    let mut rig = rig(&sv, path, device);
    let standard = rig.r.standard_program().clone();
    let flat = crate::STANDARD_PROGRAM.replace("    return exposed;", "    return s.base;");
    rig.r.reload_program(&standard, &flat).unwrap();
    let materials = std::mem::take(&mut rig.materials[0]);
    let eye = Vec3::new(0., 4., 0.01);
    let render = |rig: &mut Rig| {
        rig.frame(&sv, eye, Vec3::ZERO, &materials);
        rig.radiance(&sv)
    };
    let baseline = render(&mut rig);
    let mut decal = Decal {
        transform: box_transform(Vec3::ZERO, Vec3::Y, Vec3::new(1.5, 1.5, 0.5)),
        base_color: [0.8, 0.1, 0.05, 1.],
        edge_fade: 0.,
        ..Default::default()
    };
    let center = 128 * 256 + 128;
    let close = |actual: [f32; 3], expected: [f32; 3]| {
        for (a, b) in actual.into_iter().zip(expected) {
            assert!((a - b).abs() < 0.003, "{actual:?} != {expected:?}");
        }
    };
    close(baseline[center], [0.2, 0.3, 0.4]);
    let mut results = vec![];
    for (name, blend, opacity, expected) in [
        ("alpha", DecalBlend::Alpha, 1., [0.8, 0.1, 0.05]),
        ("half-opacity", DecalBlend::Alpha, 0.5, [0.5, 0.2, 0.225]),
        ("multiply", DecalBlend::Multiply, 1., [0.16, 0.03, 0.02]),
        ("add", DecalBlend::Add, 1., [1., 0.4, 0.45]),
    ] {
        decal.blend = blend;
        decal.opacity = opacity;
        rig.r.set_decals(&[decal.clone()]).unwrap();
        let image = render(&mut rig);
        close(image[center], expected);
        png(
            &out.join(format!("fixture-{name}.png")),
            sv.size,
            image.iter().copied(),
            1.,
        );
        assert_eq!(
            image[center + 100],
            baseline[center + 100],
            "outside box changed"
        );
        results.push(json!({"case":name,"center":image[center]}));
    }
    decal.blend = DecalBlend::Alpha;
    decal.opacity = 1.;
    decal.transform = box_transform(Vec3::ZERO, Vec3::X, Vec3::splat(2.));
    rig.r.set_decals(&[decal.clone()]).unwrap();
    close(render(&mut rig)[center], baseline[center]);
    decal.transform = box_transform(Vec3::ZERO, Vec3::Y, Vec3::new(1.5, 1.5, 0.5));
    decal.channels = [0., 1., 1., 0.];
    decal.roughness = 0.04;
    decal.metallic = 0.7;
    let parameters = crate::STANDARD_PROGRAM.replace(
        "    return exposed;",
        "    return vec3<f32>(s.rough,s.metal,material_opacity(input));",
    );
    rig.r.reload_program(&standard, &parameters).unwrap();
    rig.r.set_decals(&[decal.clone()]).unwrap();
    close(render(&mut rig)[center], [0.04, 0.7, 1.]);
    let normal =
        DecalTexture::from_rgba8(DecalTextureKind::Normal, [1, 1], &[255, 128, 255, 255]).unwrap();
    decal.channels = [0., 0., 0., 1.];
    decal.textures.normal = Some(normal);
    rig.r
        .reload_program(
            &standard,
            &crate::STANDARD_PROGRAM
                .replace("    return exposed;", "    return s.n*0.5+vec3<f32>(0.5);"),
        )
        .unwrap();
    rig.r.set_decals(&[decal.clone()]).unwrap();
    let image = render(&mut rig);
    assert!(image[center][0] > 0.8 && image[center][1] > 0.8);
    png(
        &out.join("fixture-normal.png"),
        sv.size,
        image.into_iter(),
        1.,
    );
    // Stable order, including when input order differs from authored sort order.
    rig.r.reload_program(&standard, &flat).unwrap();
    let low = Decal {
        sort_order: -1,
        base_color: [0., 1., 0., 1.],
        channels: [1., 0., 0., 0.],
        textures: Default::default(),
        ..decal.clone()
    };
    let high = Decal {
        sort_order: 2,
        base_color: [1., 0., 0., 1.],
        ..low.clone()
    };
    rig.r.set_decals(&[high, low]).unwrap();
    close(render(&mut rig)[center], [1., 0., 0.]);
    rig.r.set_decals(&[]).unwrap();
    let empty = render(&mut rig);
    assert_eq!(baseline, empty);
    // Custom programs can deliberately ignore the decal inputs.
    let custom = "struct Surface { c:vec3<f32> }; fn surface(input:SurfaceInput)->Surface {return Surface(vec3<f32>(0.3,0.2,0.1));} fn shade(s:Surface,input:SurfaceInput)->vec3<f32>{return s.c;}";
    rig.r.reload_program(&standard, custom).unwrap();
    rig.r.set_decals(&[decal]).unwrap();
    close(render(&mut rig)[center], [0.3, 0.2, 0.1]);
    json!({"cases":results,"angle_fade":true,"roughness_metallic_opacity":true,"normal":true,"sort_order":true,"empty_restores":true,"custom_program_control":true})
}

fn quarry(path: Path_, device: (wgpu::Device, wgpu::Queue), out: &Path) -> Json {
    let manifest: Json =
        serde_json::from_slice(&std::fs::read(env("RENDERLAB_DC1_QUARRY").unwrap()).unwrap())
            .unwrap();
    let path_of = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let (policy, _) = quarry_policy::Policy::args(vec![
        "--limits".into(),
        path_of("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let source =
        crate::asset::decode(&std::fs::read(path_of("asset")).unwrap(), policy.prepared()).unwrap();
    let triangles: usize = source
        .meshes
        .iter()
        .flat_map(|m| &m.primitives)
        .map(|p| p.indices.len() / 3)
        .sum();
    let (device, queue) = device;
    let limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let mut r = if path == Path_::NonRt {
        Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            limits,
            RendererOptions::default(),
            SOURCE,
        )
        .unwrap()
    } else {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            limits,
            RendererOptions::default(),
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r.set_soft_shadows(SoftShadows::Interactive);
        r
    };
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-0.5, -0.8, -0.3),
                lux: [60000., 56000., 48000.],
            }],
            environment: [0.; 3],
            exposure_stops: -12.,
        },
        &Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![4., 12., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let map =
        quarry_environment::load_environment(&path_of("environment"), Default::default()).unwrap();
    let map = loop {
        if let Some(map) = r.try_upload_environment(&map).unwrap() {
            break map;
        }
        r.test_drain("DC1 map");
    };
    r.set_environment(Some(&map), Default::default()).unwrap();
    let mut asset =
        quarry_prepared::Prepared::new_with_limits(source, policy.prepared(), 1_000_000).unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("DC1 quarry upload");
    let cameras = quarry_cameras::Cameras::load(&path_of("cameras")).unwrap();
    let pose = &cameras.frames[0];
    let camera = pose.camera().unwrap();
    let size = [pose.width, pose.height];
    let target = color_target(&r, size);
    let render = |r: &mut Renderer, asset: &mut quarry_prepared::Prepared, camera: &Camera| {
        let mut draws = asset.draws(Visibility::Frustum);
        while r
            .try_render_view(&target, camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            r.test_drain("DC1 frame admission");
        }
    };
    // Interactive traces visibility directly from hit records and need not populate
    // the separate receiver attachments. Populate them once for authored placement.
    if path == Path_::Rt {
        r.set_soft_shadows(SoftShadows::Exact);
    }
    render(&mut r, &mut asset, &camera);
    r.test_drain("DC1 receiver setup");
    let t = r.view.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let world = read_texture(&r, &t.world.texture, 16);
    let normals = read_texture(&r, &t.normal.texture, 16);
    if path == Path_::Rt {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    for _ in 0..8 {
        render(&mut r, &mut asset, &camera);
    }
    let baseline = capture_radiance(&mut r, 0.);
    png(
        &out.join("quarry-off.png"),
        size,
        baseline.iter().copied(),
        1.,
    );
    let v4 = |bytes: &[u8], i: usize| -> [f32; 4] {
        std::array::from_fn(|k| {
            f32::from_ne_bytes(
                bytes[i * 16 + k * 4..i * 16 + k * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        })
    };
    let eye = Vec3::from_array(pose.eye);
    let mut decals = vec![];
    let mut anchors = vec![];
    for (kind, seed) in [[0.42, 0.58], [0.63, 0.57], [0.47, 0.76]]
        .into_iter()
        .enumerate()
    {
        let p = [
            (seed[0] * size[0] as f32) as i32,
            (seed[1] * size[1] as f32) as i32,
        ];
        let mut best = None;
        for dy in -75i32..=75 {
            for dx in -75i32..=75 {
                let x = p[0] + dx;
                let y = p[1] + dy;
                if x < 0 || y < 0 || x >= size[0] as i32 || y >= size[1] as i32 {
                    continue;
                }
                let i = x as usize + y as usize * size[0] as usize;
                let w = v4(&world, i);
                let n = v4(&normals, i);
                if n[3] == 0. {
                    continue;
                }
                let point = Vec3::new(w[0], w[1], w[2]);
                let mut normal = Vec3::new(n[0], n[1], n[2]).normalize();
                if normal.dot(eye - point) < 0. {
                    normal = -normal;
                }
                if kind == 2 && normal.y < 0.5 {
                    continue;
                }
                let score = (dx * dx + dy * dy) as f32
                    + if kind == 2 {
                        (1. - normal.y) * 2000.
                    } else {
                        0.
                    };
                if best.as_ref().is_none_or(|(s, _, _)| score < *s) {
                    best = Some((score, point, normal));
                }
            }
        }
        let (_, point, normal) = best.expect("decal anchor must hit a quarry rock");
        anchors.push(json!({"kind":kind,"world":point.to_array(),"normal":normal.to_array()}));
        decals.push(Decal {
            transform: box_transform(
                point,
                normal,
                if kind == 2 {
                    Vec3::new(1.2, 0.9, 0.4)
                } else {
                    Vec3::new(1.1, 1.1, 0.5)
                },
            ),
            textures: DecalTextures {
                base_color: Some(painted_texture(kind)),
                ..Default::default()
            },
            roughness: if kind == 2 { 0.035 } else { 0.95 },
            channels: [1., 1., 0., if kind == 2 { 0.8 } else { 0. }],
            edge_fade: 0.08,
            angle_fade_start: 50f32.to_radians(),
            angle_fade_end: 85f32.to_radians(),
            sort_order: kind as i32,
            ..Default::default()
        });
    }
    r.set_decals(&decals).unwrap();
    for _ in 0..8 {
        render(&mut r, &mut asset, &camera);
    }
    let image = capture_radiance(&mut r, 0.);
    png(&out.join("quarry-on.png"), size, image.iter().copied(), 1.);
    let changed = image
        .iter()
        .zip(&baseline)
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| (a - b).abs() > 0.0001))
        .count();
    assert!(
        changed > 1000,
        "decals must change a visible quarry region: {changed}"
    );
    // A one-centimetre camera translation checks the decals remain world attached.
    let moved = cameras.frames[1].camera().unwrap();
    render(&mut r, &mut asset, &moved);
    let motion = capture_radiance(&mut r, 0.);
    png(&out.join("quarry-moved.png"), size, motion.into_iter(), 1.);
    // One timing campaign: eight paired blocks, alternating order. No per-frame readback.
    let stats = r.decal_stats();
    let mut samples = vec![];
    for pair in 0..8 {
        for enabled in if pair % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            r.test_drain("DC1 timing begin");
            r.poll().unwrap();
            r.set_decals(if enabled { &decals } else { &[] }).unwrap();
            render(&mut r, &mut asset, &camera);
            r.test_drain("DC1 table warm");
            r.poll().unwrap();
            for _ in 0..4 {
                render(&mut r, &mut asset, &camera);
            }
            r.test_drain("DC1 timed frames");
            for g in r.poll().unwrap().gpu {
                samples.push(json!({"pair":pair,"enabled":enabled,"shading_ms":g.mesh_pass.as_secs_f64()*1e3,"span_ms":g.render_span.as_secs_f64()*1e3}));
            }
        }
    }
    json!({"path":format!("{path:?}"),"size":size,"triangles":triangles,"decals":decals.len(),"anchors":anchors,"changed_pixels":changed,
        "table_bytes":stats.bytes,"grid_cells":stats.grid_cells,"max_cell_decals":stats.max_cell_decals,"timing_samples":samples,
        "quality":"native 1x, anisotropy 1, TR1 off, GI off, soft shadows Interactive RT / Virtual non-RT; validation enabled"})
}

#[test]
#[ignore = "shared GPU lease; RENDERLAB_DC1_OUT and RENDERLAB_DC1_QUARRY"]
fn gpu_decals() {
    let _validation = validation::begin(true);
    let out = PathBuf::from(env("RENDERLAB_DC1_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let path = if env("RENDERLAB_DC1_PATH").as_deref() == Some("nonrt") {
        Path_::NonRt
    } else {
        Path_::Rt
    };
    let device = || {
        if path == Path_::NonRt {
            no_rt_device_with(true)
        } else {
            rt_device()
        }
    };
    let fixture = fixture(path, device(), &out);
    let quarry = quarry(path, device(), &out);
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&json!({"fixture":fixture,"quarry":quarry})).unwrap(),
    )
    .unwrap();
}
