//! VF2's deterministic slatted wall and the existing quarry loader. No external assets
//! are needed by the fixture; all images are exposed HDR rendered through the real paths.
use super::*;
use serde_json::json;

fn fog(density: f32, g: f32, history: f32) -> Atmosphere {
    Atmosphere {
        height_fog: Some(HeightFog {
            anisotropy: g,
            ..Default::default()
        }),
        volumetric_fog: Some(VolumetricFog {
            extra_density: density,
            history_weight: history,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn record(r: &mut Renderer, out: &Path, name: &str, size: [u32; 2]) -> serde_json::Value {
    let image = capture(r);
    assert!(image.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
    png(&out.join(format!("{name}.png")), size, &image);
    // Keep a small linear scan of the empty medium, so shaft contrast is inspectable
    // without retaining full-frame raw dumps or confusing tone mapping with lighting.
    let row = size[1] / 2;
    let profile: Vec<_> = (0..size[0])
        .map(|x| image[(row * size[0] + x) as usize])
        .collect();
    json!({"midrow_exposed_rgb":profile})
}

#[test]
#[ignore = "VF2 owned GPU lease; evaluates the real backdrop distance shader"]
fn gpu_ground_limit() {
    let (device, queue) = no_rt_device();
    let source = format!(
        "{}\n{}",
        api_source(),
        r#"
@group(3) @binding(3) var dfg_sampler:sampler;
@group(0) @binding(0) var<storage,read_write> result:array<f32>;
@compute @workgroup_size(1) fn check_ground() {
    result[0]=at_fog_backdrop_distance(vec3<f32>(0.0,-0.17364818,0.98480775));
    result[1]=at_fog_backdrop_distance(vec3<f32>(0.0,-1.0,0.0));
    result[2]=at_fog_backdrop_distance(vec3<f32>(0.0,1.0,0.0));
    result[3]=at_fog_backdrop_distance(vec3<f32>(1.0,0.0,0.0));
}"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("VF2 ground bound"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("check_ground"),
        compilation_options: Default::default(),
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 432,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut bytes = [0u8; 432];
    bytes[64..68].copy_from_slice(&6360f32.to_ne_bytes());
    bytes[140..144].copy_from_slice(&0.001f32.to_ne_bytes());
    queue.write_buffer(&uniform, 0, &bytes);
    let result = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let groups: Vec<_> = (0..4)
        .map(|i| {
            let entries = match i {
                0 => vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: result.as_entire_binding(),
                }],
                3 => vec![wgpu::BindGroupEntry {
                    binding: 50,
                    resource: uniform.as_entire_binding(),
                }],
                _ => vec![],
            };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(i),
                entries: &entries,
            })
        })
        .collect();
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        for (i, g) in groups.iter().enumerate() {
            pass.set_bind_group(i as u32, g, &[]);
        }
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, 16);
    let done = std::sync::Arc::new(std::sync::OnceLock::new());
    let callback = done.clone();
    encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |r| {
        callback.set(r).unwrap();
    });
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    done.get().unwrap().as_ref().unwrap();
    let view = readback.get_mapped_range(..).unwrap();
    let values: Vec<_> = view
        .chunks_exact(4)
        .map(|v| f32::from_ne_bytes(v.try_into().unwrap()))
        .collect();
    assert!((values[0] - 5.758785).abs() < 0.01, "{values:?}");
    assert!((values[1] - 1.).abs() < 0.001, "{values:?}");
    assert!(values[2] > 1e20 && values[3] > 1e20, "{values:?}");
    println!("ground distances metres {values:?}");
    drop(view);
    readback.unmap();
}

#[test]
#[ignore = "VF2 owned GPU lease; VF2_OUT and VF2_PATH required"]
fn gpu_shafts() {
    let path = env("VF2_PATH").unwrap();
    let out = PathBuf::from(env("VF2_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let (device, queue) = if path == "nonrt" {
        no_rt_device()
    } else {
        rt_device()
    };
    let mut r = if path == "nonrt" {
        Renderer::new_with_options(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
            Limits::default(),
            RendererOptions::default(),
            include_str!("../shaders/scene.wgsl"),
        )
        .unwrap()
    } else {
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
            Limits::default(),
            RendererOptions::default(),
            HardwareShadowOptions::default(),
            include_str!("../shaders/scene.wgsl"),
        )
        .unwrap()
    };
    if path != "nonrt" {
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    let size = [960, 540];
    let target = r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("VF2 slatted wall"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut block = |centre: Vec3, scale: Vec3| {
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
                    position: (centre + scale * 0.5 * (normal + s * u + t * v)).to_array(),
                    normal: normal.to_array(),
                    uv: [0.; 2],
                    color: [0.12, 0.14, 0.16, 1.],
                });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    };
    let windows = env("VF2_WINDOWS").as_deref() == Some("1");
    if windows {
        // Two bounded apertures avoid the many repeating slits averaging to an
        // almost constant lit fraction along oblique camera rays.
        block(Vec3::new(0., 0.75, 12.), Vec3::new(16., 1.5, 0.3));
        block(Vec3::new(0., 4.75, 12.), Vec3::new(16., 2.5, 0.3));
        for (x, width) in [(-4.6, 6.8), (1., 2.8), (5.6, 4.8)] {
            block(Vec3::new(x, 2.5, 12.), Vec3::new(width, 2., 0.3));
        }
    } else {
        // The original narrow-gap sampling witness, matched to the VF1 baseline.
        for x in -5..=5 {
            block(Vec3::new(x as f32 * 1.2, 3., 12.), Vec3::new(0.75, 6., 0.3));
        }
    }
    block(Vec3::new(0., -0.2, 8.), Vec3::new(24., 0.4, 40.));
    block(Vec3::new(-8., 3., 8.), Vec3::new(0.4, 6., 32.));
    block(Vec3::new(8., 3., 8.), Vec3::new(0.4, 6., 32.));
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })
        .unwrap()
        .unwrap();
    let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("VF2 upload");
    let shadows = Shadows {
        directional_disk_angle: 0.00465,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![8., 24., 64.],
        }],
        ..Default::default()
    };
    let sun = |r: &mut Renderer, angle: f32| {
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: -if windows {
                        Vec3::new(0., angle.to_radians().sin(), angle.to_radians().cos())
                    } else {
                        Vec3::new(0.32, angle.to_radians().sin(), 1.).normalize()
                    },
                    lux: [120_000.; 3],
                }],
                environment: [0.; 3],
                exposure_stops: -14.5,
            },
            &shadows,
        )
        .unwrap();
    };
    sun(&mut r, 8.);
    let camera = |offset: f32| {
        Camera::look_at(
            Vec3::new(offset, 2., -4.),
            Vec3::new(offset, 2., 12.),
            Vec3::Y,
            0.9,
            0.05,
            200.,
        )
        .unwrap()
    };
    let mut render = |r: &mut Renderer, c: &Camera| {
        let until = Instant::now() + Duration::from_secs(30);
        while r
            .try_render_view(
                &target,
                c,
                &mut [Draw::new(&mesh, &mut instances)],
                ViewMode::Lit,
            )
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < until);
            r.test_drain("VF2 admission");
        }
        r.poll().unwrap();
    };
    let mut log = json!({"path":path,"size":size,"scene":if windows {"two windows"} else {"slats"},"configs":{}});
    for (name, a) in [
        (
            "off",
            Atmosphere {
                volumetric_fog: None,
                ..Default::default()
            },
        ),
        ("default", Atmosphere::default()),
        ("dense-g06", fog(0.01, 0.6, 0.9)),
        ("dense-g08", fog(0.01, 0.8, 0.9)),
        ("dense-nohistory", fog(0.01, 0.6, 0.)),
    ] {
        r.set_atmosphere(a).unwrap();
        for _ in 0..32 {
            render(&mut r, &camera(0.));
        }
        log["configs"][name] = record(&mut r, &out, name, size);
    }
    r.set_atmosphere(fog(0.01, 0.8, 0.9)).unwrap();
    for (name, tile, samples) in [
        ("coarse-one", 16, 1),
        ("fine-one", 8, 1),
        ("coarse-two", 16, 2),
        ("fine-two", 8, 2),
    ] {
        r.test_fog_quality(tile, samples);
        for _ in 0..32 {
            render(&mut r, &camera(0.));
        }
        log["configs"][name] = record(&mut r, &out, name, size);
        r.test_drain("VF2 ablation timing warm");
        let start = Instant::now();
        for _ in 0..32 {
            render(&mut r, &camera(0.));
        }
        r.test_drain("VF2 ablation timing");
        log["ablation_timing_ms"][name] = json!(start.elapsed().as_secs_f64() * 1e3 / 32.);
    }
    r.test_fog_quality(8, 2);
    // Cross the shafts at an angle so empty lit air is visible against the dark
    // side wall. Looking straight through a slit mostly sees the bright sky.
    let angled = Camera::look_at(
        Vec3::new(-6., 2., -4.),
        Vec3::new(6., 2., 8.),
        Vec3::Y,
        0.9,
        0.05,
        200.,
    )
    .unwrap();
    for (name, tile, samples, a) in [
        ("angle-default", 8, 2, Atmosphere::default()),
        ("angle-coarse-one", 16, 1, fog(0.01, 0.8, 0.9)),
        ("angle-dense-g06", 8, 2, fog(0.01, 0.6, 0.9)),
        ("angle-dense-g08", 8, 2, fog(0.01, 0.8, 0.9)),
    ] {
        r.test_fog_quality(tile, samples);
        r.set_atmosphere(a).unwrap();
        for _ in 0..32 {
            render(&mut r, &angled);
        }
        log["configs"][name] = record(&mut r, &out, name, size);
    }
    r.test_fog_quality(8, 2);
    r.set_atmosphere(fog(0.01, 0.8, 0.9)).unwrap();
    for step in 0..32 {
        render(&mut r, &camera(step.min(15) as f32 * 0.06));
        if [0, 7, 15, 16, 23, 31].contains(&step) {
            log["configs"][format!("motion-{step:02}")] =
                record(&mut r, &out, &format!("motion-{step:02}"), size);
        }
    }
    for step in 0..24 {
        sun(&mut r, 8. + step as f32 * 0.15);
        render(&mut r, &camera(0.9));
        if [0, 7, 15, 23].contains(&step) {
            log["configs"][format!("sun-{step:02}")] =
                record(&mut r, &out, &format!("sun-{step:02}"), size);
        }
    }
    sun(&mut r, 8.);
    for moving_sun in [false, true] {
        for step in 0..24 {
            let offset = Vec3::X * (step.min(15) as f32 * 0.06);
            let c = Camera::look_at(
                Vec3::new(-6., 2., -4.) + offset,
                Vec3::new(6., 2., 8.) + offset,
                Vec3::Y,
                0.9,
                0.05,
                200.,
            )
            .unwrap();
            sun(
                &mut r,
                8. + if moving_sun { step as f32 * 0.15 } else { 0. },
            );
            render(&mut r, &c);
            if [0, 7, 15, 23].contains(&step) {
                let name = format!(
                    "angle-{}-{step:02}",
                    if moving_sun { "sun" } else { "motion" }
                );
                log["configs"][&name] = record(&mut r, &out, &name, size);
            }
        }
    }
    sun(&mut r, 8.);
    for (name, a) in [
        (
            "off",
            Atmosphere {
                volumetric_fog: None,
                ..Default::default()
            },
        ),
        ("default", Atmosphere::default()),
        ("dense", fog(0.01, 0.8, 0.9)),
    ] {
        r.set_atmosphere(a).unwrap();
        for _ in 0..16 {
            render(&mut r, &camera(0.));
        }
        r.test_drain("VF2 timing warm");
        let start = Instant::now();
        for _ in 0..32 {
            render(&mut r, &camera(0.));
        }
        r.test_drain("VF2 timing");
        log["timing_ms"][name] = json!(start.elapsed().as_secs_f64() * 1e3 / 32.);
    }
    r.poll().unwrap();
    std::fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&log).unwrap(),
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
pub(super) fn quarry(
    r: &mut Renderer,
    asset: &mut quarry_prepared::Prepared,
    camera: &Camera,
    size: [u32; 2],
    out: &Path,
    path: &str,
    azimuth: Vec3,
    sun: impl Fn(&mut Renderer, Vec3, [f32; 3]),
    render: impl Fn(&mut Renderer, &mut quarry_prepared::Prepared, &Camera),
) {
    r.set_indirect(IndirectSettings::default()).unwrap();
    let mut log = json!({"path":path,"size":size,"configs":{}});
    for (name, a, angle) in [
        ("default", Atmosphere::default(), 25.0f32),
        ("low-default", Atmosphere::default(), 4.),
        ("low-dense", fog(0.01, 0.8, 0.9), 4.),
    ] {
        r.set_atmosphere(a).unwrap();
        r.set_environment(None, Default::default()).unwrap();
        sun(r, sun_travel(azimuth, angle.to_radians()), [120_000.; 3]);
        for _ in 0..32 {
            render(r, asset, camera);
        }
        log["configs"][name] = record(r, out, name, size);
        r.test_drain("VF2 quarry timing warm");
        let start = Instant::now();
        for _ in 0..32 {
            render(r, asset, camera);
        }
        r.test_drain("VF2 quarry timing");
        log["timing_ms"][name] = json!(start.elapsed().as_secs_f64() * 1e3 / 32.);
    }
    for step in 0..24 {
        let eye =
            camera.view().inverse().w_axis.truncate() + Vec3::X * (step.min(15) as f32 * 0.03);
        let forward = -camera.view().inverse().z_axis.truncate();
        let c = Camera::look_at(eye, eye + forward, Vec3::Y, 0.9, 0.05, 1000.).unwrap();
        sun(
            r,
            sun_travel(azimuth, (4. + step as f32 * 0.05).to_radians()),
            [120_000.; 3],
        );
        render(r, asset, &c);
        if [0, 7, 15, 23].contains(&step) {
            log["configs"][format!("motion-{step:02}")] =
                record(r, out, &format!("motion-{step:02}"), size);
        }
    }
    std::fs::write(
        out.join("vf2.json"),
        serde_json::to_vec_pretty(&log).unwrap(),
    )
    .unwrap();
}
