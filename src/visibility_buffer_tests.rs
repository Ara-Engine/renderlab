//! N1b batched agreement/quality/timing on a real device with RT disabled.
use super::*;

fn no_rt_renderer() -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: if manifest()["validate"].as_bool() == Some(false) {
            wgpu::InstanceFlags::empty()
        } else {
            wgpu::InstanceFlags::debugging()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("N1b adapter {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VIRTUAL_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: crate::visibility_limits(wgpu::Limits::default()),
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
        SOURCE,
    )
    .unwrap();
    assert_eq!(r.primary_visibility(), PrimaryVisibility::VisibilityBuffer);
    assert_eq!(r.soft_shadows(), SoftShadows::Virtual);
    assert!(r.hardware_shadows.is_none());
    r
}
fn geometry(r: &mut Renderer, positions: &[[f32; 3]], indices: &[u32]) -> (Mesh, Instances) {
    let vertices: Vec<_> = positions
        .iter()
        .map(|&position| Vertex {
            position,
            normal: [0., 1., 0.],
            uv: [0.; 2],
            color: [1.; 4],
        })
        .collect();
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices,
        })
        .unwrap()
        .unwrap();
    r.test_drain("N1b upload");
    let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("N1b placements");
    (mesh, instances)
}
fn light(r: &mut Renderer) {
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: -Vec3::new(0.4, 0.8, 0.3).normalize(),
                lux: [1000.; 3],
            }],
            environment: [0.03; 3],
            exposure_stops: -8.,
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
}
fn timing(r: &mut Renderer, label: &str, rows: &mut Vec<serde_json::Value>) {
    for t in r.poll().unwrap().gpu {
        let ms = |v: Duration| v.as_secs_f64() * 1000.;
        rows.push(serde_json::json!({"label":label,"span_ms":ms(t.render_span),"primary_ms":t.receiver_pass.map(ms),"shadow_ms":t.visibility_compute.map(ms)}));
    }
}
fn snapshot(r: &mut Renderer, out: &std::path::Path, label: &str) {
    r.test_drain("N1b snapshot");
    let [hits, counts] = interactive_outputs(r);
    fs::write(out.join(format!("{label}.hit")), hits).unwrap();
    fs::write(out.join(format!("{label}.counts")), counts).unwrap();
    fs::write(out.join(format!("{label}.hdr")), capture(r)).unwrap();
    let world = &r.fresh.as_ref().unwrap().targets.as_ref().unwrap().world;
    fs::write(
        out.join(format!("{label}.world")),
        read_texture(r, &world.texture),
    )
    .unwrap();
    let matrix: Vec<_> = r
        .camera_shadow
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    fs::write(
        out.join(format!("{label}.camera.json")),
        serde_json::to_vec(&matrix).unwrap(),
    )
    .unwrap();
    if let Some(v) = &r.visibility_buffer {
        fs::write(
            out.join(format!("{label}.cull.json")),
            serde_json::to_vec(&v.test_counts(r)).unwrap(),
        )
        .unwrap();
    }
}

fn reference(
    r: &mut Renderer,
    pipelines: &[Owned<wgpu::ComputePipeline>],
    out: &std::path::Path,
    label: &str,
) {
    r.test_drain("N1b film reference start");
    let hit = hit_record(r);
    let n = hit.len() / 16;
    let mut sum = vec![0f64; n];
    let mut square = vec![0f64; n];
    for pipeline in pipelines {
        let mut e = r.device.create_command_encoder(&Default::default());
        r.encode_primary_fresh(&mut e, pipeline, None);
        r.queue.submit([e.finish()]);
        let t = &r
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture;
        for (i, c) in read_texture(r, t).chunks_exact(4).enumerate() {
            let value = f64::from(u32::from_le_bytes(c.try_into().unwrap())) / 256.;
            sum[i] += value;
            square[i] += value * value;
        }
    }
    let count = pipelines.len() as f64;
    let mut bytes = Vec::with_capacity(n * 8);
    for (s, q) in sum.into_iter().zip(square) {
        let mean = s / count;
        let sigma = (q / count - mean * mean).max(0.).sqrt();
        bytes.extend_from_slice(&(mean as f32).to_le_bytes());
        bytes.extend_from_slice(&(sigma as f32).to_le_bytes());
    }
    fs::write(out.join(format!("{label}.reference")), bytes).unwrap();
}

#[derive(serde::Deserialize)]
struct Bench {
    size: [u32; 2],
    meshes: Vec<BenchMesh>,
    #[serde(default)]
    poses: Vec<BenchPose>,
    #[serde(default)]
    tracks: Vec<BenchTrack>,
    #[serde(default)]
    to_light: Option<[f32; 3]>,
    #[serde(default)]
    fov_y_degrees: Option<f32>,
    sun_lux: [f32; 3],
    exposure_stops: f32,
    #[serde(default)]
    material: Option<BenchMaterial>,
}
#[derive(serde::Deserialize)]
struct BenchMaterial {
    base_color: [f32; 4],
    metallic: f32,
}
#[derive(serde::Deserialize)]
struct BenchMesh {
    #[serde(default)]
    name: String,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    thin: bool,
    #[serde(default)]
    roughness: Option<f32>,
}
#[derive(serde::Deserialize)]
struct BenchTrack {
    frames: Vec<BenchPose>,
}
#[derive(Clone, serde::Deserialize)]
struct BenchPose {
    eye: [f32; 3],
    target: [f32; 3],
    #[serde(default)]
    to_light: Option<[f32; 3]>,
    #[serde(default)]
    plate_x: f32,
    #[serde(default)]
    blocker_x: f32,
}

#[test]
#[ignore = "N1b owned GPU lease: S1/S2/S3 full-detail primaries and complete no-RT frame"]
fn gpu_visibility_bench() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let no_rt = m["no_rt"].as_bool().unwrap_or(false);
    let mut r = if no_rt {
        no_rt_renderer()
    } else {
        renderer_with_features(
            m["validate"].as_bool() != Some(false),
            crate::VIRTUAL_FEATURES,
        )
    };
    if !no_rt {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    let mut scene = vec![];
    for g in &b.meshes {
        let vertices: Vec<_> = g
            .positions
            .iter()
            .zip(&g.normals)
            .map(|(&position, &normal)| Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect();
        let mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &g.indices,
            })
            .unwrap()
            .unwrap();
        r.test_drain("N1b bench mesh");
        let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("N1b bench instance");
        let material = r
            .try_create_material(&MaterialDescriptor {
                double_sided: true,
                thin: g.thin,
                base_color: b
                    .material
                    .as_ref()
                    .map_or([0.5, 0.5, 0.5, 1.], |m| m.base_color),
                metallic: b.material.as_ref().map_or(0., |m| m.metallic),
                roughness: g.roughness.unwrap_or(0.6),
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        r.test_drain("N1b bench material");
        scene.push((mesh, instances, material));
    }
    let color = target(&r, b.size[0], b.size[1]);
    let mut rows = vec![];
    let poses: Vec<_> = if b.tracks.is_empty() {
        b.poses.clone()
    } else {
        let f = &b.tracks[0].frames;
        vec![f[0].clone(), f[f.len() / 2].clone(), f[0].clone()]
    };
    for (pose, p) in poses.iter().enumerate() {
        for (g, (_, instances, _)) in b.meshes.iter().zip(&mut scene) {
            let x = match g.name.as_str() {
                "reveal-plate" => p.plate_x,
                "shadow-blocker" => p.blocker_x,
                _ => 0.,
            };
            instances
                .set(0, Mat4::from_translation(Vec3::new(x, 0., 0.)))
                .unwrap();
        }
        let camera = Camera::look_at(
            Vec3::from_array(p.eye),
            Vec3::from_array(p.target),
            Vec3::Y,
            b.fov_y_degrees.unwrap_or(45.).to_radians(),
            0.01,
            100.,
        )
        .unwrap();
        let to = Vec3::from_array(p.to_light.or(b.to_light).unwrap()).normalize();
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: -to,
                    lux: b.sun_lux,
                }],
                environment: [0.; 3],
                exposure_stops: b.exposure_stops,
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
        r.set_soft_shadows(SoftShadows::Virtual);
        for mode in if no_rt {
            vec![PrimaryVisibility::VisibilityBuffer]
        } else {
            vec![
                PrimaryVisibility::Traced,
                PrimaryVisibility::VisibilityBuffer,
            ]
        } {
            r.primary_mode = mode;
            let label = format!("p{pose}-{mode:?}");
            let warm = if m["timing"].as_bool() == Some(true) {
                32
            } else {
                4
            };
            for i in 0..warm + 8 {
                let mut draws: Vec<_> = scene
                    .iter_mut()
                    .map(|(mesh, instances, material)| {
                        Draw::new(mesh, instances).with_material(material)
                    })
                    .collect();
                render(&mut r, &color, &camera, &mut draws);
                if warm == 4 || i + 1 == warm {
                    r.test_drain("N1b bench frame");
                }
                if i + 1 == warm {
                    let _ = r.poll().unwrap();
                } else {
                    timing(&mut r, if i < warm { "warm" } else { &label }, &mut rows);
                }
            }
            r.test_drain("N1b bench timing tail");
            timing(&mut r, &label, &mut rows);
            snapshot(&mut r, &out, &label);
        }
    }
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N1b owned GPU lease: 4 GiB start / 2 GiB abort"]
fn gpu_visibility_fixture() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = no_rt_renderer();
    light(&mut r);
    let (mesh, mut instances) = geometry(
        &mut r,
        &[
            [-2., 0., -2.],
            [2., 0., -2.],
            [2., 0., 2.],
            [-2., 0., 2.],
            [-0.4, 0., 0.],
            [0.4, 0., 0.],
            [0., 0.8, 0.],
        ],
        &[0, 2, 1, 0, 3, 2, 4, 5, 6],
    );
    let color = target(&r, 320, 192);
    let camera = Camera::look_at(
        Vec3::new(0.2, 1.2, 2.5),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let mut rows = vec![];
    for i in 0..12 {
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        assert_eq!(report.primary, PrimaryVisibility::VisibilityBuffer);
        assert_eq!(report.soft_shadows, SoftShadows::Virtual);
        r.test_drain("N1b fixture frame");
        timing(&mut r, if i < 4 { "warm" } else { "no-rt" }, &mut rows);
    }
    snapshot(&mut r, &out, "no-rt");
    let hit = hit_record(&r);
    assert!(hit.chunks_exact(16).any(|h| h[..4] != [0; 4]));
    r.reload(SOURCE).unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("N1b reload");
    assert_eq!(hit, hit_record(&r));
    // Geometry/transform motion and recovery; the return image must be identical.
    instances
        .set(0, Mat4::from_translation(Vec3::new(0.1, 0., 0.)))
        .unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("N1b moved");
    instances.set(0, Mat4::IDENTITY).unwrap();
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("N1b return");
    assert_eq!(hit, hit_record(&r));
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "N1b toon",
            source: include_str!("../shaders/programs/toon.wgsl"),
        })
        .unwrap();
    let material = r
        .try_create_material(&MaterialDescriptor {
            program: Some(&program),
            double_sided: true,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
    r.test_drain("N1b toon material");
    render(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
    );
    r.test_drain("N1b toon frame");
    assert_eq!(hit, hit_record(&r));
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N1b owned GPU lease: quarry agreement and paired cost, asset loaded once"]
fn gpu_visibility_quarry() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let no_rt = m["no_rt"].as_bool().unwrap_or(false);
    let mut r = if no_rt {
        no_rt_renderer()
    } else {
        renderer_with_features(
            m["validate"].as_bool() != Some(false),
            crate::VIRTUAL_FEATURES,
        )
    };
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    if !no_rt {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    }
    r.set_soft_shadows(SoftShadows::Virtual);
    let color = target(&r, 1920, 1080);
    let mut rows = vec![];
    let seeded = if !no_rt && m["reference"].as_bool() != Some(false) {
        let module = seeded_module(&r);
        (1..=64u32)
            .map(|seed| {
                r.primary.as_ref().unwrap().fresh_pipeline(
                    &r.device,
                    &r.ledger,
                    &module,
                    "fresh_compute",
                    &[("FRESH_SEED", f64::from(seed))],
                )
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    for (pose, index) in [0usize, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[index].camera().unwrap();
        if !seeded.is_empty() && pose < 2 {
            r.primary_mode = PrimaryVisibility::Traced;
            r.set_soft_shadows(SoftShadows::Exact);
            render(
                &mut r,
                &color,
                &camera,
                &mut asset.draws(Visibility::Frustum),
            );
            reference(&mut r, &seeded, &out, &format!("p{pose}"));
            r.set_soft_shadows(SoftShadows::Virtual);
        }
        let modes = if no_rt {
            vec![PrimaryVisibility::VisibilityBuffer]
        } else {
            vec![
                PrimaryVisibility::Traced,
                PrimaryVisibility::VisibilityBuffer,
            ]
        };
        for mode in modes {
            r.primary_mode = mode;
            let label = format!("p{pose}-{mode:?}");
            let warm = if m["timing"].as_bool() == Some(true) {
                32
            } else {
                4
            };
            for i in 0..warm + 8 {
                render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                if warm == 4 || i + 1 == warm {
                    r.test_drain("N1b quarry frame");
                }
                if i + 1 == warm {
                    let _ = r.poll().unwrap();
                } else {
                    timing(&mut r, if i < warm { "warm" } else { &label }, &mut rows);
                }
            }
            r.test_drain("N1b quarry timing tail");
            timing(&mut r, &label, &mut rows);
            snapshot(&mut r, &out, &label);
        }
    }
    fs::write(
        out.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
