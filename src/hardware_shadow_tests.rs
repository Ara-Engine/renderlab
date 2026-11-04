use super::*;

pub(super) fn hardware_device() -> (wgpu::Device, wgpu::Queue) {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"),
        "GPU tests require explicit caller acknowledgment"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    assert!(
        adapter
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
    );
    assert!(
        adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::MULTISAMPLED_SHADING)
    );
    let limits = wgpu::Limits {
        max_blas_primitive_count: 4096,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: 64,
        max_acceleration_structures_per_shader_stage: 1,
        ..Default::default()
    };
    assert!(limits.check_limits(&adapter.limits()));
    eprintln!("hardware_shadow_adapter={:?}", adapter.get_info());
    // SAFETY: the test caller explicitly accepts wgpu's experimental implementation risk.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
        required_limits: limits,
        experimental_features,
        ..Default::default()
    }))
    .unwrap()
}
fn hardware_renderer() -> Renderer {
    let (device, queue) = hardware_device();
    Renderer::new_with_hardware_shadows(
        device,
        queue,
        FORMAT,
        Limits::default(),
        RendererOptions::default(),
        HardwareShadowOptions::default(),
        SOURCE,
    )
    .unwrap()
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
fn shadows() -> Shadows {
    Shadows {
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![3., 10.],
        }],
        ..Default::default()
    }
}
fn lighting() -> Lighting {
    Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-1., 0., -1.),
            lux: [1000.; 3],
        }],
        exposure_stops: -5.,
        ..Default::default()
    }
}
fn lit(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
) -> FrameReport {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(frame) = r
            .try_render_view(color, camera, draws, ViewMode::Lit)
            .unwrap()
        {
            return frame;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
fn center(image: &[u8]) -> u8 {
    image[(96 * 256 + 128) * 4]
}

#[test]
#[ignore = "explicit experimental-feature acknowledgment and root GPU slot; no window"]
fn gpu_hardware_shadow_orientation_blocker_motion_reload_and_release() {
    let _validation = validation::begin(true);
    let mut r = hardware_renderer();
    let color = target(r.device(), 256, 192);
    let receiver = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
    let blocker = upload(&mut r, &quad(0.4), &[0, 1, 2, 0, 2, 3]);
    let mut a = instances(&mut r, &[Mat4::IDENTITY]);
    let mut b = instances(&mut r, &[Mat4::from_translation(Vec3::new(2., 0., 2.))]);
    let camera = camera(0.);
    r.set_lighting(&lighting()).unwrap();
    lit(&mut r, &color, &camera, &mut [Draw::new(&receiver, &mut a)]);
    let disabled = r.test_capture_color(&color);
    eprintln!("hardware_disabled_center={}", center(&disabled));
    assert!(center(&disabled) > 50);
    r.set_shadows(&shadows()).unwrap();
    lit(&mut r, &color, &camera, &mut [Draw::new(&receiver, &mut a)]);
    let clear = r.test_capture_color(&color);
    // The derivative normal faces away from the light. Omitting orientation offsets
    // the ray into the receiver and fails this complete-image equality regression.
    assert_eq!(
        clear, disabled,
        "light-facing geometric-normal origin contract"
    );
    lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    let blocked = r.test_capture_color(&color);
    assert!(center(&blocked) < 3, "camera-invisible blocker must cast");
    let built = r.hardware_shadow_stats().unwrap();
    let geometry = r.stats().uploads.geometry;
    let frame = lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    assert_eq!(frame.uploads.geometry, 0);
    assert_eq!(
        r.hardware_shadow_stats().unwrap().tlas_builds,
        built.tlas_builds
    );
    b.set(0, Mat4::from_translation(Vec3::new(4., 0., 2.)))
        .unwrap();
    let moved = lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    assert_eq!(moved.uploads.models, 64);
    assert_eq!(r.stats().uploads.geometry, geometry);
    assert_eq!(
        r.hardware_shadow_stats().unwrap().blas_builds,
        built.blas_builds
    );
    assert_eq!(
        r.hardware_shadow_stats().unwrap().tlas_builds,
        built.tlas_builds + 1
    );
    assert_eq!(r.test_capture_color(&color), disabled);
    let revision = r.stats().revision;
    assert!(
        r.reload(&SOURCE.replace("// renderlab: direct visibility end", ""))
            .is_err()
    );
    assert!(r.reload("broken shader").is_err());
    assert!(
        r.reload(&SOURCE.replace("const TINT:", "const INVALID ??? TINT:"))
            .is_err(),
        "syntactically invalid full bundle must retain both pipeline families"
    );
    assert_eq!(r.stats().revision, revision);
    lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    assert_eq!(
        r.test_capture_color(&color),
        disabled,
        "rejected reload retained all pipelines"
    );
    r.reload(SOURCE).unwrap();
    b.set(0, Mat4::from_translation(Vec3::new(2., 0., 2.)))
        .unwrap();
    lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    assert!(
        center(&r.test_capture_color(&color)) < 3,
        "corrected reload must restore positive hardware occlusion"
    );
    b.set(0, Mat4::from_translation(Vec3::new(4., 0., 2.)))
        .unwrap();
    let finite = Shadows {
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![1.],
        }],
        ..Default::default()
    };
    r.set_shadows(&finite).unwrap();
    lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    assert_eq!(
        r.test_capture_color(&color),
        disabled,
        "receiver outside finite shadow coverage"
    );
    r.set_shadows(&shadows()).unwrap();
    r.test_hardware_busy(&color, &camera, &receiver, &mut a);
    a.set(0, Mat4::IDENTITY).unwrap();
    let admitted = lit(
        &mut r,
        &color,
        &camera,
        &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
    );
    assert_eq!(
        admitted.uploads.models, 64,
        "busy transform edit survives until admission"
    );
    assert_eq!(r.test_capture_color(&color), disabled);
    r.set_shadows(&Shadows::default()).unwrap();
    r.test_drain("hardware disabled scene retirement");
    assert_eq!(r.stats().ownership.get(ResourceKind::Tlas).live, 0);
    assert_eq!(r.hardware_shadow_stats().unwrap().pending_builds, 0);
    drop(receiver);
    drop(blocker);
    assert_eq!(r.stats().ownership.get(ResourceKind::Blas).live, 0);
    assert_eq!(r.stats().ownership.hardware_as_estimated_bytes.live, 0);
    assert!(
        r.stats().ownership.shadow_bytes.live < 4096,
        "no raster maps in hardware frames"
    );
}

#[test]
#[ignore = "root GPU slot; bounded replacement/release with queued hardware frames"]
fn gpu_hardware_shadow_replacement_with_queued_work() {
    let _validation = validation::begin(true);
    let mut r = hardware_renderer();
    let color = target(r.device(), 256, 192);
    let receiver = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
    let mut blocker = upload(&mut r, &quad(0.4), &[0, 1, 2, 0, 2, 3]);
    let mut a = instances(&mut r, &[Mat4::IDENTITY]);
    let mut b = instances(&mut r, &[Mat4::from_translation(Vec3::new(2., 0., 2.))]);
    r.set_lighting_and_shadows(&lighting(), &shadows()).unwrap();
    let mut incomplete = 0;
    for i in 0..12 {
        if i % 3 == 0 {
            blocker = upload(&mut r, &quad(0.4), &[0, 1, 2, 0, 2, 3]);
        }
        b.set(
            0,
            Mat4::from_translation(Vec3::new(2. + (i % 2) as f32, 0., 2.)),
        )
        .unwrap();
        lit(
            &mut r,
            &color,
            &camera(0.),
            &mut [Draw::new(&receiver, &mut a), Draw::new(&blocker, &mut b)],
        );
        incomplete += r.test_overlap().0;
        assert!(r.hardware_shadow_stats().unwrap().retained_generations <= 3);
        assert!(r.hardware_shadow_stats().unwrap().pending_builds <= 3);
    }
    eprintln!("hardware_replacement_observed_incomplete_frames={incomplete}");
    assert!(
        incomplete > 0,
        "no actual incomplete frame observed; overlap verification is inconclusive"
    );
    r.set_shadows(&Shadows::default()).unwrap();
    drop(receiver);
    drop(blocker);
    r.test_drain("queued hardware replacement release");
    assert_eq!(r.stats().ownership.get(ResourceKind::Blas).live, 0);
    assert_eq!(r.stats().ownership.get(ResourceKind::Tlas).live, 0);
    assert_eq!(r.stats().ownership.hardware_as_estimated_bytes.live, 0);
    assert_eq!(r.stats().ownership.hardware_scratch_estimated_bytes.live, 0);
}

#[test]
#[ignore = "root GPU slot; featureless and unsupported configuration boundaries"]
fn gpu_hardware_shadow_rejects_unsupported_before_writes_and_preserves_featureless() {
    let _validation = validation::begin(true);
    let (device, queue) = device(false);
    assert!(
        Renderer::new_with_hardware_shadows(
            device.clone(),
            queue.clone(),
            FORMAT,
            Limits::default(),
            RendererOptions::default(),
            HardwareShadowOptions::default(),
            SOURCE
        )
        .is_err()
    );
    let mut ordinary = Renderer::new(device, queue, FORMAT, Limits::default(), SOURCE).unwrap();
    assert!(ordinary.hardware_shadow_stats().is_none());
    let color = target(ordinary.device(), 256, 192);
    let mesh = upload(&mut ordinary, &quad(1.), &[0, 1, 2, 0, 2, 3]);
    let mut slots = instances(&mut ordinary, &[Mat4::IDENTITY]);
    render(
        &mut ordinary,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut slots)],
    );
    assert!(center(&ordinary.test_capture_color(&color)) > 200);
    let (device, queue) = hardware_device();
    let options = RendererOptions {
        samples: SampleCount::Four,
        ..Default::default()
    };
    let mut r = Renderer::new_with_hardware_shadows(
        device,
        queue,
        FORMAT,
        Limits::default(),
        options,
        HardwareShadowOptions::default(),
        SOURCE,
    )
    .unwrap();
    r.set_lighting_and_shadows(&lighting(), &shadows()).unwrap();
    let color = target(r.device(), 256, 192);
    let mesh = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
    let mut slots = instances(&mut r, &[Mat4::IDENTITY]);
    r.test_drain("hardware material creation");
    let material = r
        .try_create_material(&MaterialDescriptor::default())
        .unwrap()
        .unwrap();
    r.test_drain("hardware rejection baseline");
    let before = r.stats();
    assert!(
        r.try_render_view(
            &color,
            &camera(0.),
            &mut [Draw::new(&mesh, &mut slots).with_material(&material)],
            ViewMode::Lit
        )
        .is_err()
    );
    assert_eq!(r.stats().uploads, before.uploads);
    assert_eq!(r.stats().ownership, before.ownership);
    assert!(r.reject_virtual_hardware_shadows(ViewMode::Lit).is_err());
    assert!(
        r.reject_virtual_hardware_shadows(ViewMode::BaseColor)
            .is_ok()
    );
    let local = Lighting {
        lights: vec![Light::Point {
            position: Vec3::Z,
            candela: [1.; 3],
            range: None,
        }],
        ..Default::default()
    };
    let local_shadow = Shadows {
        requests: vec![ShadowRequest::Point {
            near: 0.1,
            far: 10.,
        }],
        ..Default::default()
    };
    assert!(r.set_lighting_and_shadows(&local, &local_shadow).is_err());
    assert_eq!(r.stats().uploads, before.uploads);
}

#[test]
#[ignore = "root GPU slot; normal map must not move hardware ray origin"]
fn gpu_hardware_shadow_normal_map_uses_geometric_origin() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        normal_maps::geometric_shadow_case(samples, true);
    }
}

#[test]
#[ignore = "root GPU slot; analytical per-sample edge, moving thin grazing contact, centre impostor"]
fn gpu_hardware_shadow_msaa_sample_receivers() {
    let _validation = validation::begin(true);
    let camera = camera(0.);
    let pixel_world = 8. * (55_f32.to_radians() * 0.5).tan() / 97.;
    let visibility = SOURCE.replace(
        "return vec4<f32>(exposed,1.0);",
        "return vec4<f32>(vec3<f32>(hard_shadow(lights[0],input.world,geometric,-lights[0].travel_outer.xyz)),1.0);",
    );
    let reference = SOURCE.replace(
        "return vec4<f32>(exposed,1.0);",
        "return vec4<f32>(input.color.rgb,1.0);",
    );
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = hardware_device();
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions {
                samples,
                ..Default::default()
            },
            HardwareShadowOptions::default(),
            &visibility,
        )
        .unwrap();
        let color = target(r.device(), 129, 97);
        let receiver = upload(&mut r, &quad(1.5), &[0, 1, 2, 0, 2, 3]);
        let blocker = upload(&mut r, &quad(1.), &[0, 1, 2, 0, 2, 3]);
        let mut receivers = instances(&mut r, &[Mat4::IDENTITY]);
        let mut blockers = instances(&mut r, &[Mat4::IDENTITY]);
        let mut proxies = instances(&mut r, &[Mat4::IDENTITY]);
        let settings = Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10.],
            }],
            ..Default::default()
        };
        let mut moving_images = vec![];
        for pose in 0..4 {
            let (to_light, height, half_width, offset) = if pose == 0 {
                (
                    Vec3::new(1., 0., 1.).normalize(),
                    2.,
                    0.5,
                    0.5 + pixel_world * 0.05,
                )
            } else {
                (
                    Vec3::new(1., 0., 0.12).normalize(),
                    0.03,
                    pixel_world * 0.15,
                    pixel_world * (pose - 1) as f32 * 0.5 - settings.normal_bias / 0.12,
                )
            };
            let model = Mat4::from_translation(Vec3::new(
                height * to_light.x / to_light.z + offset,
                0.,
                height,
            )) * Mat4::from_scale(Vec3::new(half_width, 0.7, 1.));
            blockers.set(0, model).unwrap();
            let mut footprint = quad(1.);
            for vertex in &mut footprint {
                let p = model.transform_point3(Vec3::from_array(vertex.position));
                // Intersect the light ray with the receiver's biased geometric plane.
                let origin_offset = Vec3::Z * settings.normal_bias;
                vertex.position =
                    (p - origin_offset - to_light * ((p.z - origin_offset.z) / to_light.z))
                        .to_array();
                vertex.color = [0., 0., 0., 1.];
            }
            let proxy = upload(&mut r, &footprint, &[0, 1, 2, 0, 2, 3]);
            let lights = Lighting {
                lights: vec![Light::Directional {
                    travel: -to_light,
                    lux: [1000.; 3],
                }],
                exposure_stops: -5.,
                ..Default::default()
            };
            r.reload(&visibility).unwrap();
            r.set_lighting_and_shadows(&lights, &settings).unwrap();
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&receiver, &mut receivers),
                    Draw::new(&blocker, &mut blockers),
                ],
            );
            let actual = lighting_tests::hdr(&mut r);
            let display = r.test_capture_color(&color);
            let name = format!("hardware-msaa-{pose}-{}x", samples as u32);
            save(
                &name,
                129,
                97,
                &display
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect::<Vec<_>>(),
            );
            fs::create_dir_all(evidence_dir()).unwrap();
            fs::write(
                evidence_dir().join(format!("{name}.hdr-f32")),
                actual
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();

            r.reload(&reference).unwrap();
            r.set_shadows(&Shadows::default()).unwrap();
            // Equal-depth first-winner coverage is independent of interpolated receiver values.
            lit(
                &mut r,
                &color,
                &camera,
                &mut [
                    Draw::new(&proxy, &mut proxies),
                    Draw::new(&receiver, &mut receivers),
                    Draw::new(&blocker, &mut blockers),
                ],
            );
            let expected = lighting_tests::hdr(&mut r);
            let region = (30..67).flat_map(|y| (40..89).map(move |x| (y * 129 + x) * 4));
            let max_error = region
                .clone()
                .map(|i| (actual[i] - expected[i]).abs())
                .fold(0_f32, f32::max);
            assert!(
                max_error < 0.001,
                "sample={} pose={pose} footprint error={max_error}",
                samples as u32
            );
            let middle = (48 * 129 + 64) * 4;
            if pose == 0 {
                if samples == SampleCount::One {
                    assert_eq!(actual[middle], 1., "1x centre lies outside the shadow");
                } else {
                    assert_eq!(
                        expected[middle], 0.5,
                        "analytical edge splits four covered samples"
                    );
                    assert_eq!(actual[middle], 0.5);
                    assert!(
                        (i16::from(display[middle]) - 85).abs() <= 1,
                        "visibility resolves in HDR before tone mapping"
                    );
                    let impostor = visibility.replace(
                        "hard_shadow(lights[0],input.world,geometric,",
                        &format!("hard_shadow(lights[0],vec3<f32>((floor(input.position.x)+0.5-64.5)*{pixel_world},(48.5-floor(input.position.y)-0.5)*{pixel_world},0.0),geometric,"),
                    );
                    r.reload(&impostor).unwrap();
                    r.set_shadows(&settings).unwrap();
                    lit(
                        &mut r,
                        &color,
                        &camera,
                        &mut [
                            Draw::new(&receiver, &mut receivers),
                            Draw::new(&blocker, &mut blockers),
                        ],
                    );
                    let wrong = lighting_tests::hdr(&mut r);
                    assert_eq!(
                        wrong[middle], 1.,
                        "centre impostor must differ from the half-visible reference"
                    );
                }
            } else {
                moving_images.push(actual);
            }
        }
        assert!(
            moving_images.windows(2).all(|pair| pair[0] != pair[1]),
            "thin contact motion must change the observed mask"
        );
        r.test_drain("analytical hardware MSAA complete");
    }
}
