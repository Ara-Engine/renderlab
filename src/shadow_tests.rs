use super::*;

fn lit(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
) -> FrameReport {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(report) = r
            .try_render_view(color, camera, draws, ViewMode::Lit)
            .unwrap()
        {
            return report;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
fn quad(half: f32) -> [Vertex; 4] {
    [
        [-half, -half, 0.],
        [half, -half, 0.],
        [half, half, 0.],
        [-half, half, 0.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    })
}
fn policy(request: ShadowRequest) -> Shadows {
    Shadows {
        requests: vec![request],
        resolution: 1024,
        normal_bias: 0.,
        ..Default::default()
    }
}
fn center(image: &[u8], width: usize, height: usize) -> u8 {
    image[((height / 2) * width + width / 2) * 4]
}

#[test]
#[ignore = "Vulkan GPU; offscreen blocker, local seam, atomic rejection, reload, mirrored source sidedness"]
fn gpu_shadow_offscreen_local_and_reload() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(true);
        let mut r = Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap();
        let color = target(r.device(), 256, 192);
        let receiver = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
        let blocker = upload(&mut r, &quad(0.4), &[0, 1, 2, 0, 2, 3]);
        let reversed = upload(&mut r, &quad(0.4), &[2, 1, 0, 3, 2, 0]);
        let mut receivers = instances(&mut r, &[Mat4::IDENTITY]);
        let mut blockers = instances(&mut r, &[Mat4::from_translation(Vec3::new(2., 0., 2.))]);
        let deadline = Instant::now() + Duration::from_secs(20);
        let material = loop {
            if let Some(m) = r
                .try_create_material(&MaterialDescriptor {
                    roughness: 1.,
                    metallic: 0.,
                    double_sided: false,
                    ..Default::default()
                })
                .unwrap()
            {
                break m;
            }
            assert!(Instant::now() < deadline);
        };
        let camera = camera(0.);
        r.test_drain("shadow sided material admission");
        let double_sided = r
            .try_create_material(&MaterialDescriptor {
                roughness: 1.,
                metallic: 0.,
                double_sided: true,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        let cases = [
            (
                Light::Directional {
                    travel: Vec3::new(-1., 0., -1.),
                    lux: [100.; 3],
                },
                ShadowRequest::Directional {
                    split_ends: vec![3., 10.],
                },
            ),
            (
                Light::Point {
                    position: Vec3::new(4., 0., 4.),
                    candela: [3200.; 3],
                    range: None,
                },
                ShadowRequest::Point {
                    near: 0.001,
                    far: 20.,
                },
            ),
            (
                Light::Spot {
                    position: Vec3::new(4., 0., 4.),
                    travel: Vec3::new(-1., 0., -1.),
                    candela: [3200.; 3],
                    range: None,
                    inner: 0.3,
                    outer: 0.6,
                },
                ShadowRequest::Spot {
                    near: 0.001,
                    far: 20.,
                },
            ),
        ];
        for (case, (light, request)) in cases.into_iter().enumerate() {
            let lighting = Lighting {
                lights: vec![light],
                exposure_stops: -5.,
                ..Default::default()
            };
            r.set_lighting_and_shadows(&lighting, &Shadows::default())
                .unwrap();
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&blocker, &mut blockers).with_material(&material),
                ],
            );
            let clear = r.test_capture_color(&color);
            assert!(center(&clear, 256, 192) > 50);
            let shadows = policy(request);
            r.set_shadows(&shadows).unwrap();
            let report = lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&blocker, &mut blockers).with_material(&material),
                ],
            );
            assert_eq!(
                report.scene.visible_instances, 1,
                "blocker must be camera-invisible"
            );
            let shadowed = r.test_capture_color(&color);
            assert!(
                center(&shadowed, 256, 192) < 3,
                "offscreen blocker failed for case {case}"
            );
            save(
                &format!("m6-shadow-case-{case}-{}x", samples as u32),
                256,
                192,
                &shadowed
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect::<Vec<_>>(),
            );
            // The point-light ray through the centre is exactly on an X/Z cube seam.
            for x in [125usize, 126, 127, 128, 129, 130] {
                assert!(shadowed[(96 * 256 + x) * 4] < 3);
            }
            blockers
                .set(
                    0,
                    Mat4::from_translation(Vec3::new(2., 0., 2.))
                        * Mat4::from_scale(Vec3::new(-1., 1., 1.)),
                )
                .unwrap();
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&blocker, &mut blockers).with_material(&material),
                ],
            );
            assert_eq!(
                shadowed,
                r.test_capture_color(&color),
                "mirrored source winding"
            );
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&reversed, &mut blockers).with_material(&material),
                ],
            );
            assert!(
                clear == r.test_capture_color(&color),
                "source backface must not cast"
            );
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&reversed, &mut blockers).with_material(&double_sided),
                ],
            );
            assert!(
                shadowed == r.test_capture_color(&color),
                "double-sided backface must cast"
            );
            let revision = r.stats().revision;
            for bad in [
                "not WGSL".to_owned(),
                SOURCE.replace(
                    "var<uniform> shadow_view:ShadowView",
                    "var<uniform> shadow_view:mat4x4<f32>",
                ),
                SOURCE.replace("fn vs_shadow(", "fn removed_shadow("),
            ] {
                assert!(matches!(r.reload(&bad), Err(ReloadError::Rejected { .. })));
                assert_eq!(revision, r.stats().revision);
            }
            assert!(r.set_lighting(&Lighting::default()).is_err());
            let mut no_fit = shadows.clone();
            no_fit.max_bytes = Some(1);
            assert!(r.set_shadows(&no_fit).is_err());
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&blocker, &mut blockers).with_material(&material),
                ],
            );
            assert_eq!(
                shadowed,
                r.test_capture_color(&color),
                "rejected transactions retain image"
            );
            r.reload(SOURCE).unwrap();
            assert!(r.stats().revision.0 > revision.0);
            blockers
                .set(0, Mat4::from_translation(Vec3::new(3.5, 0., 2.)))
                .unwrap();
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers).with_material(&material),
                    Draw::new(&blocker, &mut blockers).with_material(&material),
                ],
            );
            let moved = r.test_capture_color(&color);
            save(
                &format!("m6-shadow-moved-case-{case}-{}x", samples as u32),
                256,
                192,
                &moved
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect::<Vec<_>>(),
            );
            assert!(
                center(&moved, 256, 192) > 50,
                "moving blocker must reveal light case={case} samples={} clear={} moved={}",
                samples as u32,
                center(&clear, 256, 192),
                center(&moved, 256, 192)
            );
            assert!(
                clear == moved,
                "unblocked planar receiver must stay wholly lit"
            );
            blockers
                .set(0, Mat4::from_translation(Vec3::new(2., 0., 2.)))
                .unwrap();
        }
        r.test_drain("shadow sample cleanup");
        assert_eq!(r.shadow_stats().retained_generations, 0);
        let generation = r.shadow_stats().generation;
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&receiver, &mut receivers)],
        );
        assert_eq!(report.uploads.shadows, 0);
        assert_eq!(r.shadow_stats().generation, generation);
        r.set_shadows(&Shadows::default()).unwrap();
        let report = lit(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&receiver, &mut receivers)],
        );
        assert_eq!(report.uploads.shadows, 0);
    }
}

#[test]
#[ignore = "Vulkan GPU; identical resident virtual cut, regular source geometry and page stamps"]
fn gpu_shadow_virtual_exact_cut() {
    use crate::asset::virtual_mesh::{PAGE_BYTES, decode_page};
    use crate::virtual_geometry::tests::{directory, fixture, write_pages};
    use crate::virtual_geometry::*;
    use sha2::{Digest, Sha256};
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(true);
        let mut r = Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap();
        let (mut package, blobs) = fixture(1);
        for material in &mut package.resources.materials {
            material.base_color = [1.; 4];
            material.roughness = 1.;
        }
        let dir = directory("m6-shadow-exact");
        write_pages(&package, &blobs, &dir);
        let geometry = decode_page(
            &package.pages[0],
            &blobs[0],
            Sha256::digest(&blobs[0]).into(),
        )
        .unwrap();
        let mesh = upload(&mut r, &geometry.vertices, &geometry.indices);
        let models = [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(2., 0., 2.)) * Mat4::from_scale(Vec3::splat(0.6)),
        ];
        let mut regular = instances(&mut r, &models);
        let mut asset = VirtualAsset::new(
            package,
            dir,
            StreamOptions {
                geometry_bytes: 2 * PAGE_BYTES,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::new(
            &asset,
            &models.map(|transform| VirtualPlacement { mesh: 0, transform }),
        )
        .unwrap();
        let color = target(r.device(), 256, 192);
        let camera = camera(0.);
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(-1., 0., -1.),
                    lux: [100.; 3],
                }],
                exposure_stops: -5.,
                ..Default::default()
            },
            &policy(ShadowRequest::Directional {
                split_ends: vec![3., 10.],
            }),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if r.try_render_virtual(&color, &camera, &mut asset, &mut scene, ViewMode::Lit)
                .unwrap()
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let virtual_image = r.test_capture_color(&color);
        lit(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut regular)],
        );
        let regular_image = r.test_capture_color(&color);
        for (name, image) in [("virtual", &virtual_image), ("regular", &regular_image)] {
            save(
                &format!("m6-shadow-{name}-{}x", samples as u32),
                256,
                192,
                &image
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect::<Vec<_>>(),
            );
        }
        assert!(
            virtual_image == regular_image,
            "one exact cut at {}x; differing channels={}",
            samples as u32,
            virtual_image
                .iter()
                .zip(&regular_image)
                .filter(|(a, b)| a != b)
                .count()
        );
        r.set_shadows(&Shadows::default()).unwrap();
        lit(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut regular)],
        );
        let unshadowed = r.test_capture_color(&color);
        let occluded = virtual_image
            .chunks_exact(4)
            .zip(unshadowed.chunks_exact(4))
            .filter(|(a, b)| i32::from(b[0]) - i32::from(a[0]) > 30)
            .count();
        assert!(
            occluded > 20,
            "equality fixture must actually contain shadows"
        );
        assert_eq!(asset.stats().states[PageStatus::Resident as usize], 1);
    }
}

#[test]
#[ignore = "Vulkan GPU; planar self-shadowing, grazing receiver, thin caster, cascade and light-basis crossings"]
fn gpu_shadow_planar_receiver_motion() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(true);
        let mut r = Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap();
        let color = target(r.device(), 256, 192);
        let receiver = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
        let blocker = upload(&mut r, &quad(0.4), &[0, 1, 2, 0, 2, 3]);
        let mut receivers = instances(&mut r, &[Mat4::IDENTITY]);
        let mut blockers = instances(
            &mut r,
            &[Mat4::from_translation(Vec3::new(0.6, 0.6, 2.))
                * Mat4::from_scale(Vec3::new(0.125, 1., 1.))],
        );
        r.test_drain("planar material admission");
        let material = r
            .try_create_material(&MaterialDescriptor {
                roughness: 1.,
                metallic: 0.,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        for (tilt, angle) in [0., -1.2].into_iter().enumerate() {
            receivers.set(0, Mat4::from_rotation_y(angle)).unwrap();
            for kind in 0..3 {
                for pose in 0..3 {
                    let delta = (pose as f32 - 1.) * 0.0001;
                    let travel = Vec3::new(-0.3 - delta, -0.3, -1.);
                    let position = Vec3::new(1.2 + delta, 1.2, 4.);
                    let (light, request) = match kind {
                        0 => (
                            Light::Directional {
                                travel,
                                lux: [1000.; 3],
                            },
                            ShadowRequest::Directional {
                                split_ends: vec![3., 10.],
                            },
                        ),
                        1 => (
                            Light::Spot {
                                position,
                                travel,
                                candela: [32000.; 3],
                                range: None,
                                inner: 0.3,
                                outer: 0.6,
                            },
                            ShadowRequest::Spot {
                                near: 0.05,
                                far: 20.,
                            },
                        ),
                        _ => (
                            Light::Point {
                                position,
                                candela: [32000.; 3],
                                range: None,
                            },
                            ShadowRequest::Point {
                                near: 0.05,
                                far: 20.,
                            },
                        ),
                    };
                    let lighting = Lighting {
                        lights: vec![light],
                        exposure_stops: -5.,
                        ..Default::default()
                    };
                    let camera = Camera::look_at(
                        Vec3::new(0., 0., 2.99 + pose as f32 * 0.01),
                        Vec3::ZERO,
                        Vec3::Y,
                        55_f32.to_radians(),
                        0.1,
                        100.,
                    )
                    .unwrap();
                    r.set_lighting_and_shadows(&lighting, &Shadows::default())
                        .unwrap();
                    lit(
                        &mut r,
                        &color,
                        &camera,
                        &mut [Draw::new(&receiver, &mut receivers).with_material(&material)],
                    );
                    let clear = r.test_capture_color(&color);
                    r.set_shadows(&policy(request)).unwrap();
                    lit(
                        &mut r,
                        &color,
                        &camera,
                        &mut [Draw::new(&receiver, &mut receivers).with_material(&material)],
                    );
                    let self_shadowed = r.test_capture_color(&color);
                    for (name, image) in [("clear", &clear), ("self", &self_shadowed)] {
                        save(
                            &format!("m6-planar-{name}-{tilt}-{kind}-{pose}-{}x", samples as u32),
                            256,
                            192,
                            &image
                                .chunks_exact(4)
                                .flat_map(|p| p[..3].iter().copied())
                                .collect::<Vec<_>>(),
                        );
                    }
                    assert!(
                        clear == self_shadowed,
                        "self-shadowing tilt={tilt} kind={kind} pose={pose} samples={}",
                        samples as u32
                    );
                    lit(
                        &mut r,
                        &color,
                        &camera,
                        &mut [
                            Draw::new(&receiver, &mut receivers).with_material(&material),
                            Draw::new(&blocker, &mut blockers).with_material(&material),
                        ],
                    );
                    let blocked = r.test_capture_color(&color);
                    save(
                        &format!("m6-planar-{tilt}-{kind}-{pose}-{}x", samples as u32),
                        256,
                        192,
                        &blocked
                            .chunks_exact(4)
                            .flat_map(|p| p[..3].iter().copied())
                            .collect::<Vec<_>>(),
                    );
                    assert!(
                        center(&clear, 256, 192) > 10 && center(&blocked, 256, 192) < 3,
                        "thin blocker on grazing/cascade receiver tilt={tilt} kind={kind} pose={pose} samples={} clear={} blocked={}",
                        samples as u32,
                        center(&clear, 256, 192),
                        center(&blocked, 256, 192)
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "Vulkan GPU; shadow admission, near-domain atomic rejection and resource retirement"]
fn gpu_shadow_admission_and_resources() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let color = target(r.device(), 128, 96);
    let mesh = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
    let mut inst = instances(&mut r, &[Mat4::IDENTITY]);
    let cam = camera(0.);
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: -Vec3::Z,
            lux: [100.; 3],
        }],
        ..Default::default()
    };
    r.set_lighting_and_shadows(
        &lighting,
        &policy(ShadowRequest::Directional {
            split_ends: vec![10.],
        }),
    )
    .unwrap();
    lit(&mut r, &color, &cam, &mut [Draw::new(&mesh, &mut inst)]);
    r.test_shadow_admission(&color, &cam, &mesh, &mut inst);
    let baseline = r.stats().ownership.shadow_bytes.live;
    for resolution in [128, 256, 512, 256] {
        let shadows = Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10.],
            }],
            resolution,
            ..Default::default()
        };
        r.set_shadows(&shadows).unwrap();
        lit(&mut r, &color, &cam, &mut [Draw::new(&mesh, &mut inst)]);
        assert!(r.shadow_stats().retained_generations > 0);
    }
    r.test_drain("shadow replacement cleanup");
    assert_eq!(r.shadow_stats().retained_generations, 0);
    assert_eq!(r.stats().ownership.shadow_bytes.live, baseline);
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Point {
                position: Vec3::ZERO,
                candela: [100.; 3],
                range: None,
            }],
            ..Default::default()
        },
        &policy(ShadowRequest::Point {
            near: 0.01,
            far: 10.,
        }),
    )
    .unwrap();
    let before = r.stats();
    assert!(matches!(
        r.try_render_view(
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst)],
            ViewMode::Lit
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(before.uploads, r.stats().uploads);
    assert_eq!(before.ownership, r.stats().ownership);
}
