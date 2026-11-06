//! S1: one uploaded procedural scene, production modes and independent footprint integration.
use super::*;

#[derive(serde::Deserialize)]
struct Bench {
    size: [u32; 2],
    poses: Vec<Pose>,
    to_light: [f32; 3],
    sun_lux: [f32; 3],
    exposure_stops: f32,
    meshes: Vec<Geometry>,
}
#[derive(serde::Deserialize)]
struct Pose {
    eye: [f32; 3],
    target: [f32; 3],
}
#[derive(serde::Deserialize)]
struct Geometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    thin: bool,
}

fn lighting(b: &Bench, environment: f32, radius: f32) -> (Lighting, Shadows) {
    (
        Lighting {
            lights: vec![Light::Directional {
                travel: -Vec3::from_array(b.to_light),
                lux: b.sun_lux,
            }],
            environment: [environment; 3],
            exposure_stops: b.exposure_stops,
        },
        Shadows {
            directional_disk_angle: radius,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10., 40.],
            }],
            ..Default::default()
        },
    )
}

#[test]
#[ignore = "S1 owned GPU lease, 4 GiB start/2 GiB abort; one batched offscreen process"]
fn gpu_contact_fold_batch() {
    let m = manifest();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let root = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&root).unwrap();
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
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
        r.test_drain("S1 mesh upload");
        let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("S1 instance upload");
        let material = r
            .try_create_material(&MaterialDescriptor {
                base_color: [0.5, 0.5, 0.5, 1.],
                metallic: 0.,
                roughness: 0.6,
                double_sided: true,
                thin: g.thin,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        r.test_drain("S1 material upload");
        scene.push((mesh, instances, material));
    }
    let mut timings = vec![];
    for (pose, p) in b.poses.iter().enumerate() {
        let camera = Camera::look_at(
            Vec3::from_array(p.eye),
            Vec3::from_array(p.target),
            Vec3::Y,
            45f32.to_radians(),
            0.01,
            100.,
        )
        .unwrap();
        let color = target(&r, b.size[0], b.size[1]);
        for environment in [0., 1.] {
            let out = root.join(format!(
                "p{pose}-{}",
                if environment == 0. {
                    "black"
                } else {
                    "environment"
                }
            ));
            fs::create_dir_all(&out).unwrap();
            let (lit, shadows) = lighting(&b, environment, 0.04);
            r.set_lighting_and_shadows(&lit, &shadows).unwrap();
            let frame = |r: &mut Renderer, scene: &mut Vec<(Mesh, Instances, Material)>| {
                let mut draws: Vec<_> = scene
                    .iter_mut()
                    .map(|(mesh, instances, material)| {
                        Draw::new(mesh, instances).with_material(material)
                    })
                    .collect();
                render(r, &color, &camera, &mut draws)
            };
            r.set_soft_shadows(SoftShadows::Exact);
            frame(&mut r, &mut scene);
            integrate_with(&mut r, &camera, &out, pose, true, None);
            // Centre receivers for the temporal diagnostic's frozen surface condition.
            frame(&mut r, &mut scene);
            let [world, _, _] = receivers(&r);
            let hit = hit_record(&r);
            let forward = (Vec3::from_array(p.target) - Vec3::from_array(p.eye)).normalize();
            let centre: Vec<u8> = world
                .chunks_exact(16)
                .zip(hit.chunks_exact(16))
                .flat_map(|(w, h)| {
                    let xyz = Vec3::from_array(std::array::from_fn(|i| {
                        f32::from_le_bytes(w[i * 4..i * 4 + 4].try_into().unwrap())
                    }));
                    h[..4]
                        .iter()
                        .copied()
                        .chain((xyz - Vec3::from_array(p.eye)).dot(forward).to_le_bytes())
                        .collect::<Vec<_>>()
                })
                .collect();
            for (name, mode, radius) in [
                ("hard", SoftShadows::Exact, 0.),
                ("Exact", SoftShadows::Exact, 0.04),
                ("Interactive", SoftShadows::Interactive, 0.04),
                ("Temporal", SoftShadows::Temporal, 0.04),
            ] {
                let (lit, shadows) = lighting(&b, environment, radius);
                r.set_lighting_and_shadows(&lit, &shadows).unwrap();
                r.set_soft_shadows(mode);
                let conditioned = if mode == SoftShadows::Temporal {
                    Some(ConditionedVisibility::new(&r, &centre, forward))
                } else {
                    None
                };
                let frames = if mode == SoftShadows::Temporal {
                    128
                } else {
                    32
                };
                for n in 1..=frames {
                    frame(&mut r, &mut scene);
                    if [1, 2, 4, 8, 32, 128].contains(&n) {
                        fs::write(out.join(format!("{name}-n{n}.rgba16f")), settled(&mut r))
                            .unwrap();
                    }
                    if let Some(c) = &conditioned {
                        c.accumulate(&r, (n % 2) as usize);
                    }
                }
                if let Some(c) = &conditioned {
                    for half in 0..2 {
                        fs::write(
                            out.join(format!("{name}-visibility-{half}.u32")),
                            read_buffer(&mut r, &c.result[half], c.result[half].size()),
                        )
                        .unwrap();
                    }
                } else {
                    // A visibility-only program reads the actual hard/soft production estimator.
                    let program = crate::program::standard_returning(VISIBILITY);
                    let mut probe = centre_shading(&r, &program, true);
                    std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut probe);
                    frame(&mut r, &mut scene);
                    fs::write(
                        out.join(format!("{name}-visibility.rgba16f")),
                        settled(&mut r),
                    )
                    .unwrap();
                    std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut probe);
                }
                // Price production only: no captures/diagnostic passes inside the measured block.
                for _ in 0..8 {
                    frame(&mut r, &mut scene);
                }
                r.test_drain("S1 warm timing");
                r.poll().unwrap();
                let mut cpu = vec![];
                for _ in 0..8 {
                    let start = Instant::now();
                    frame(&mut r, &mut scene);
                    cpu.push(start.elapsed().as_secs_f64() * 1000.);
                }
                r.test_drain("S1 timing end");
                let gpu: Vec<_> = r
                    .poll()
                    .unwrap()
                    .gpu
                    .into_iter()
                    .map(|s| s.render_span.as_secs_f64() * 1000.)
                    .collect();
                timings.push(serde_json::json!({"pose":pose,"environment":environment,"mode":name,"gpu_ms":gpu,"cpu_service_ms":cpu,
                    "scope":"first rendered pass through final tone pass, offscreen; no OS presentation"}));
                eprintln!("S1 pose={pose} environment={environment} mode={name} complete");
            }
        }
    }
    fs::write(
        root.join("timings.json"),
        serde_json::to_vec_pretty(&timings).unwrap(),
    )
    .unwrap();
}
