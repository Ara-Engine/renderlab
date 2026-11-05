use super::*;
use crate::asset::{Mip, Sampler, TangentBasis, TangentStream, TextureRole};

fn lit(r: &mut Renderer, t: &wgpu::Texture, c: &Camera, draws: &mut [Draw<'_>]) {
    let until = Instant::now() + Duration::from_secs(20);
    while r
        .try_render_view(t, c, draws, ViewMode::Lit)
        .unwrap()
        .is_none()
    {
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "Vulkan GPU slot and active validation required; CPU-only handoff must not run this"]
fn gpu_normal_maps_frame_oracle_and_tangent_bundle_recovery() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(false);
        let mut r = suite_renderer(
            device,
            queue,
            FORMAT,
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap();
        let target = target(r.device(), 65, 65);
        let vertices = triangle([1.; 3]);
        let normal_source = crate::program::standard_returning("n*0.5+vec3<f32>(0.5)");
        let white_source = crate::program::standard_returning("vec3<f32>(1.0)");
        assert_ne!(normal_source, crate::STANDARD_PROGRAM);
        let transforms = [
            Mat4::IDENTITY,
            Mat4::from_rotation_y(0.35) * Mat4::from_scale(Vec3::new(-1.5, 0.75, 0.5)),
        ];
        for (case, (model, eye_z, w, scale, reverse, pixel)) in [
            (transforms[0], 4., 1., 1., false, [128, 128, 255, 255]),
            (transforms[0], 4., 1., 1., false, [128, 204, 230, 255]),
            (transforms[0], 4., 1., -1., false, [128, 204, 230, 255]),
            (transforms[0], 4., 1., 0., false, [128, 204, 230, 255]),
            (transforms[0], 4., -1., 1., false, [128, 204, 230, 255]),
            (transforms[1], 4., 1., 1., false, [128, 204, 230, 255]),
            (transforms[1], 4., -1., 1., false, [128, 204, 230, 255]),
            (transforms[1], -4., -1., 1., false, [128, 204, 230, 255]),
            (transforms[0], 4., 1., 1., true, [128, 204, 230, 255]),
        ]
        .into_iter()
        .enumerate()
        {
            r.test_drain("normal oracle setup");
            let tangents = TangentStream {
                basis: TangentBasis::Authored,
                values: vec![[1., 0., 0., w]; 3],
            };
            let indices = if reverse { [0, 2, 1] } else { [0, 1, 2] };
            let mesh = r
                .try_upload_mesh_with_tangents(
                    &MeshData {
                        vertices: &vertices,
                        indices: &indices,
                    },
                    Some(&tangents),
                )
                .unwrap()
                .unwrap();
            r.test_drain("normal texture admission");
            let texture = r
                .try_upload_texture(&asset::Texture {
                    role: TextureRole::NormalLinear,
                    mips: vec![Mip {
                        width: 1,
                        height: 1,
                        rgba8: pixel.to_vec(),
                    }],
                })
                .unwrap()
                .unwrap();
            r.test_drain("normal material admission");
            let material = r
                .try_create_material(&MaterialDescriptor {
                    normal_texture: Some(TextureBinding {
                        texture: &texture,
                        sampler: Sampler::default(),
                    }),
                    normal_scale: scale,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap()
                .unwrap();
            let mut instances = instances(&mut r, &[model]);
            let camera = Camera::look_at(
                Vec3::new(0., 0., eye_z),
                Vec3::ZERO,
                Vec3::Y,
                0.8,
                0.1,
                100.,
            )
            .unwrap();
            r.test_reload_standard(&white_source).unwrap();
            lit(
                &mut r,
                &target,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            let coverage = lighting_tests::hdr(&mut r);
            r.test_reload_standard(&normal_source).unwrap();
            lit(
                &mut r,
                &target,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            let image = lighting_tests::hdr(&mut r);
            let a = glam::DMat3::from_cols_array(
                &glam::Mat3::from_mat4(model).to_cols_array().map(f64::from),
            );
            let n = (a.inverse().transpose() * glam::DVec3::Z).normalize();
            let t = a * glam::DVec3::X;
            let t = (t - n * n.dot(t)).normalize();
            let b = f64::from(w) * a.determinant().signum() * n.cross(t);
            let mut decoded = glam::DVec3::new(
                f64::from(pixel[0]) * 2. / 255. - 1.,
                f64::from(pixel[1]) * 2. / 255. - 1.,
                f64::from(pixel[2]) * 2. / 255. - 1.,
            );
            decoded.x *= f64::from(scale);
            decoded.y *= f64::from(scale);
            decoded = decoded.normalize();
            let mut expected = (t * decoded.x + b * decoded.y + n * decoded.z).normalize();
            if (eye_z < 0.) != reverse {
                expected = -expected;
            }
            let encoded = expected * 0.5 + glam::DVec3::splat(0.5);
            assert!(coverage.chunks_exact(4).any(|p| p[0] == 1.));
            for (i, (actual, mask)) in image
                .chunks_exact(4)
                .zip(coverage.chunks_exact(4))
                .enumerate()
            {
                for ch in 0..3 {
                    assert!(
                        (f64::from(actual[ch]) - encoded[ch] * f64::from(mask[0])).abs() < 0.002,
                        "case={case} sample={} pixel={i} actual={actual:?} expected={encoded:?} coverage={mask:?}",
                        samples as u32
                    );
                }
            }
            let directory = evidence_dir().join("normal-maps");
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join(format!("frame-{}-{case}.hdr-f32", samples as u32)),
                image
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let display = r.test_capture_color(&target);
            fs::write(
                directory.join(format!("frame-{}-{case}.rgba8", samples as u32)),
                &display,
            )
            .unwrap();
            let before = r.stats();
            // A rejected prelude edit keeps the normal-probe program rendering.
            let bad = SOURCE.replace(
                "@location(5) tangent:vec4<f32>",
                "@location(7) tangent:vec4<f32>",
            );
            assert_ne!(bad, SOURCE);
            assert!(r.reload(&bad).is_err());
            assert_eq!(r.stats().revision, before.revision);
            lit(
                &mut r,
                &target,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            assert_eq!(image, lighting_tests::hdr(&mut r));
            r.test_reload_standard(crate::STANDARD_PROGRAM).unwrap();
            let before = r.stats();
            lit(
                &mut r,
                &target,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            assert_eq!(r.stats().uploads.geometry, before.uploads.geometry);
        }
        if samples == SampleCount::One {
            let mut cancelling = vertices;
            cancelling[0].position = [-1., -1., 0.];
            cancelling[1].position = [1., -1., 0.];
            cancelling[2].position = [0., 1., 0.];
            cancelling[2].normal = [0., 0., -1.];
            let mesh = upload(&mut r, &cancelling, &INDICES);
            let mut slots = instances(&mut r, &[Mat4::IDENTITY]);
            r.test_reload_standard(&normal_source).unwrap();
            for eye_z in [4., -4.] {
                let c = Camera::look_at(
                    Vec3::new(0., 0., eye_z),
                    Vec3::ZERO,
                    Vec3::Y,
                    0.8,
                    0.1,
                    100.,
                )
                .unwrap();
                lit(&mut r, &target, &c, &mut [Draw::new(&mesh, &mut slots)]);
                let image = lighting_tests::hdr(&mut r);
                let centre = &image[(32 * 65 + 32) * 4..(32 * 65 + 32) * 4 + 3];
                let expected = if eye_z > 0. {
                    [0.5, 0.5, 1.]
                } else {
                    [0.5, 0.5, 0.]
                };
                for (actual, expected) in centre.iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 0.002,
                        "cancelled source normal fallback {centre:?}"
                    );
                }
            }
        }
        r.test_drain("normal oracle final");
    }
}

#[test]
#[ignore = "Vulkan GPU slot required; generate normal_render_fixture and set RENDERLAB_NORMAL_FIXTURE_DIR"]
fn gpu_normal_maps_prepared_regular_virtual_exact_resident_images() {
    use super::materials::prepared::Prepared;
    use crate::virtual_geometry::*;
    let _validation = validation::begin(true);
    let directory = std::env::var_os("RENDERLAB_NORMAL_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .expect("run the CPU normal_render_fixture example first");
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(false);
        let mut r = suite_renderer(
            device,
            queue,
            FORMAT,
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap();
        r.set_lighting(&Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-0.4, -0.7, -1.),
                lux: [2.; 3],
            }],
            ..Default::default()
        })
        .unwrap();
        let color = target(r.device(), 129, 97);
        let mut regular =
            Prepared::load_with_limits(&directory.join("mapped.rla"), asset::Limits::default(), 32)
                .unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        while !regular.ready() {
            regular.advance(&mut r).unwrap();
            r.poll().unwrap();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        let camera = Camera::look_at(
            Vec3::new(0., 0., 8.),
            Vec3::new(0., -0.5, 0.),
            Vec3::Y,
            0.8,
            0.1,
            100.,
        )
        .unwrap();
        lit(
            &mut r,
            &color,
            &camera,
            &mut regular.draws(Visibility::Unculled),
        );
        let expected = lighting_tests::hdr(&mut r);
        let display = r.test_capture_color(&color);
        let source = asset::decode(
            &fs::read(directory.join("mapped.rla")).unwrap(),
            asset::Limits::default(),
        )
        .unwrap();
        let mut virtual_asset = VirtualAsset::open(
            &directory.join("cache"),
            &directory.join("mapped.virtual"),
            StreamOptions {
                error_pixels: 0.,
                ..Default::default()
            },
        )
        .unwrap();
        let placements = source
            .placements
            .iter()
            .map(|p| VirtualPlacement {
                mesh: p.mesh,
                transform: Mat4::from_cols_array_2d(&p.transform),
            })
            .collect::<Vec<_>>();
        let mut scene = VirtualScene::new(&virtual_asset, &placements).unwrap();
        loop {
            let progress = virtual_asset
                .prepare_view(&mut r, &mut scene, &camera, color.height())
                .unwrap();
            match progress.state {
                PreparationState::Settled => break,
                PreparationState::Pending => {}
                state => panic!("normal fixture failed exact residency: {state:?}"),
            }
            r.poll().unwrap();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        while r
            .try_render_prepared_virtual(
                &color,
                &camera,
                &mut virtual_asset,
                &mut scene,
                ViewMode::Lit,
            )
            .unwrap()
            .is_none()
        {
            r.poll().unwrap();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(expected, lighting_tests::hdr(&mut r));
        assert_eq!(display, r.test_capture_color(&color));
        r.test_drain("normal regular/virtual fixture final");
    }
}

#[test]
#[ignore = "Vulkan GPU; bounded normal resource replacement with observed incomplete prior frames"]
fn gpu_normal_maps_integration_replacement_affinity_and_mixed_draws() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(false);
        let mut r = suite_renderer(
            device,
            queue,
            FORMAT,
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap();
        let vertices = triangle([0.7; 3]);
        let indices = [0, 1, 2].repeat(2048);
        let stream = TangentStream {
            basis: TangentBasis::Authored,
            values: vec![[1., 0., 0., 1.]; 3],
        };
        let data = MeshData {
            vertices: &vertices,
            indices: &indices,
        };
        let upload_tangent = |r: &mut Renderer| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let before = r.stats().uploads;
                if let Some(m) = r
                    .try_upload_mesh_with_tangents(&data, Some(&stream))
                    .unwrap()
                {
                    break m;
                }
                assert_eq!(r.stats().uploads, before);
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        let mut mesh = upload_tangent(&mut r);
        let basic = upload(&mut r, &vertices, &[0, 1, 2]);
        let models = (0..48)
            .map(|i| Mat4::from_translation(Vec3::new(0., 0., i as f32 * 0.0001)))
            .collect::<Vec<_>>();
        let mut slots = instances(&mut r, &models);
        let mut basic_slots = instances(&mut r, &[Mat4::from_translation(Vec3::new(0.8, 0., 0.1))]);
        let normal = asset::Texture {
            role: TextureRole::NormalLinear,
            mips: vec![Mip {
                width: 1,
                height: 1,
                rgba8: vec![128, 204, 230, 255],
            }],
        };
        let texture = |r: &mut Renderer| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let before = r.stats().uploads;
                if let Some(t) = r.try_upload_texture(&normal).unwrap() {
                    break t;
                }
                assert_eq!(r.stats().uploads, before);
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        let material = |r: &mut Renderer, t: &Texture| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let before = r.stats().uploads;
                if let Some(m) = r
                    .try_create_material(&MaterialDescriptor {
                        normal_texture: Some(TextureBinding {
                            texture: t,
                            sampler: Sampler::default(),
                        }),
                        metallic: 0.,
                        roughness: 0.4,
                        double_sided: true,
                        ..Default::default()
                    })
                    .unwrap()
                {
                    break m;
                }
                assert_eq!(r.stats().uploads, before);
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        let mut tex = texture(&mut r);
        let mut mat = material(&mut r, &tex);
        let color = target(r.device(), 512, 384);
        let camera = camera(0.);
        r.set_lighting(&Lighting {
            environment: [0.1; 3],
            ..Default::default()
        })
        .unwrap();
        lit(
            &mut r,
            &color,
            &camera,
            &mut [
                Draw::new(&mesh, &mut slots).with_material(&mat),
                Draw::new(&basic, &mut basic_slots),
            ],
        );
        r.test_drain("normal ownership setup");
        let baseline = r.stats();
        // A traced-suite 1x renderer adds the hardware (2) and fresh raster (3) pipelines.
        let raster = if traced_suite() && samples == SampleCount::One {
            11
        } else {
            6
        };
        assert_eq!(
            baseline.ownership.get(ResourceKind::RenderPipeline).live,
            raster
        );
        let mut overlap = [0; 3];
        let mut peaks = [HighWater::default(); 2];
        for frame in 0..36 {
            let (incomplete, _) = r.test_overlap();
            let kind = frame % 3;
            match kind {
                0 => tex = texture(&mut r),
                1 => mat = material(&mut r, &tex),
                _ => mesh = upload_tangent(&mut r),
            }
            overlap[kind] += usize::from(incomplete > 0);
            let before = r.stats().uploads.geometry;
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&basic, &mut basic_slots),
                    Draw::new(&mesh, &mut slots).with_material(&mat),
                ],
            );
            assert_eq!(r.stats().uploads.geometry, before);
            assert!(r.stats().flights <= 3);
            peaks[frame / 18].observe(&r.stats());
            if frame == 17 {
                let before = r.stats();
                assert!(
                    r.reload(&SOURCE.replace(
                        "@location(5) tangent:vec4<f32>",
                        "@location(7) tangent:vec4<f32>"
                    ))
                    .is_err()
                );
                assert_eq!(r.stats().revision, before.revision);
            }
        }
        r.reload(SOURCE).unwrap();
        r.test_drain("normal ownership final");
        let final_stats = r.stats();
        assert!(
            overlap.iter().all(|n| *n >= 2),
            "normal incomplete prior frames {overlap:?}"
        );
        assert_eq!(
            final_stats.ownership.material_bytes.live,
            baseline.ownership.material_bytes.live
        );
        assert_eq!(
            final_stats.ownership.texture_bytes.live,
            baseline.ownership.texture_bytes.live
        );
        assert_eq!(final_stats.hal.buffers, baseline.hal.buffers);
        assert!((peaks[1].buffer_bytes - peaks[0].buffer_bytes).abs() < 512 * 1024);
        assert!((peaks[1].texture_bytes - peaks[0].texture_bytes).abs() < 4 * 1024 * 1024);
        let mut other = renderer(false);
        let before_other = other.stats().uploads;
        assert!(
            other
                .try_create_material(&MaterialDescriptor {
                    normal_texture: Some(TextureBinding {
                        texture: &tex,
                        sampler: Sampler::default()
                    }),
                    ..Default::default()
                })
                .is_err()
        );
        assert!(
            r.try_render_view(
                &color,
                &camera,
                &mut [Draw::new(&basic, &mut basic_slots).with_material(&mat)],
                ViewMode::Lit
            )
            .is_err()
        );
        assert_eq!(other.stats().uploads, before_other);
        let other_color = target(other.device(), 129, 97);
        assert!(
            other
                .try_render_view(
                    &other_color,
                    &camera,
                    &mut [Draw::new(&mesh, &mut slots).with_material(&mat)],
                    ViewMode::Lit
                )
                .is_err()
        );
        assert_eq!(other.stats().uploads, before_other);
        let before = r.stats().uploads;
        assert_eq!(r.stats().uploads, before);
        fs::create_dir_all(evidence_dir()).unwrap();
        fs::write(evidence_dir().join(format!("normal-ownership-{}x.txt",samples as u32)),
            format!("overlap_texture_material_tangent={overlap:?}\npeaks={peaks:#?}\nbaseline={baseline:#?}\nfinal={final_stats:#?}\n")).unwrap();
    }
}

#[test]
#[ignore = "Vulkan GPU; actual occlusion must use geometric receiver plane with mapped normals"]
fn gpu_normal_maps_integration_shadow_plane_is_geometric() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        geometric_shadow_case(samples, false);
    }
}
pub(super) fn geometric_shadow_case(samples: SampleCount, hardware: bool) {
    let (device, queue) = if hardware {
        hardware_shadow_tests::hardware_device()
    } else {
        device(false)
    };
    let visibility = crate::program::standard_returning(
        "vec3<f32>(hard_shadow(lights[0],input.receiver,-lights[0].travel_outer.xyz))",
    );
    let options = RendererOptions {
        samples,
        ..Default::default()
    };
    let mut r = if hardware {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            Limits::default(),
            options,
            HardwareShadowOptions::default(),
            SOURCE,
        )
    } else {
        suite_renderer(device, queue, FORMAT, options, SOURCE)
    }
    .unwrap();
    r.test_reload_standard(&visibility).unwrap();
    let vertices = [
        ([-1., -1., 0.], [0., 1.]),
        ([1., -1., 0.], [1., 1.]),
        ([1., 1., 0.], [1., 0.]),
        ([-1., 1., 0.], [0., 0.]),
    ]
    .map(|(position, uv)| Vertex {
        position,
        uv,
        normal: [0., 0., 1.],
        color: [1.; 4],
    });
    let stream = TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[1., 0., 0., -1.]; 4],
    };
    let mesh = r
        .try_upload_mesh_with_tangents(
            &MeshData {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
            },
            Some(&stream),
        )
        .unwrap()
        .unwrap();
    let blocker = upload(&mut r, &triangle([1.; 3]), &INDICES);
    let mut receiver = instances(&mut r, &[Mat4::IDENTITY]);
    let mut caster = instances(
        &mut r,
        &[Mat4::from_translation(Vec3::new(-0.3, 0., 0.5)) * Mat4::from_scale(Vec3::splat(0.3))],
    );
    let color = target(r.device(), 129, 97);
    let camera = camera(0.);
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-0.4, -0.3, -1.),
                lux: [2.; 3],
            }],
            ..Default::default()
        },
        &Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10.],
            }],
            resolution: 128,
            ..Default::default()
        },
    )
    .unwrap();
    r.test_drain("geometric shadow map setup");
    let tex = r
        .try_upload_texture(&asset::Texture {
            role: TextureRole::NormalLinear,
            mips: vec![Mip {
                width: 1,
                height: 1,
                rgba8: vec![230, 204, 180, 255],
            }],
        })
        .unwrap()
        .unwrap();
    r.test_drain("geometric shadow material setup");
    let mapped = r
        .try_create_material(&MaterialDescriptor {
            normal_texture: Some(TextureBinding {
                texture: &tex,
                sampler: Sampler::default(),
            }),
            normal_scale: 5.,
            double_sided: true,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
    lit(
        &mut r,
        &color,
        &camera,
        &mut [
            Draw::new(&mesh, &mut receiver),
            Draw::new(&blocker, &mut caster),
        ],
    );
    let expected = lighting_tests::hdr(&mut r);
    let interior = (32..64).flat_map(|y| (45..84).map(move |x| (y * 129 + x) * 4));
    assert!(
        interior.clone().any(|i| expected[i] == 0.),
        "receiver interior needs actual occlusion"
    );
    assert!(
        interior.clone().any(|i| expected[i] == 1.),
        "receiver interior needs an unblocked region"
    );
    lit(
        &mut r,
        &color,
        &camera,
        &mut [
            Draw::new(&mesh, &mut receiver).with_material(&mapped),
            Draw::new(&blocker, &mut caster),
        ],
    );
    assert_eq!(
        expected,
        lighting_tests::hdr(&mut r),
        "normal map changed geometric shadow mask"
    );
    r.test_drain("geometric shadow final");
}
