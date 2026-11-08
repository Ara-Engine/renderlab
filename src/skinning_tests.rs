use super::*;
use crate::asset::skin::Influence;
use crate::tests::validation;
#[allow(dead_code)]
#[path = "../examples/skinned-mesh.rs"]
mod demo;

fn read_buffer(r: &Renderer, source: &wgpu::Buffer) -> Vec<f32> {
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("SK1 oracle readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut e = r.device.create_command_encoder(&Default::default());
    e.copy_buffer_to_buffer(source, 0, &buffer, 0, source.size());
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    e.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
        callback.set(result).unwrap();
    });
    r.queue.submit([e.finish()]);
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    done.get().unwrap().as_ref().unwrap();
    let view = buffer.get_mapped_range(..).unwrap();
    let result = view
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
        .collect();
    drop(view);
    buffer.unmap();
    result
}
#[test]
#[ignore = "SK1 guarded GPU lease, validation layers and RT acknowledgement required"]
fn gpu_skinned_streams_history_and_paths() {
    let _validation = validation::begin(true);
    for rt in [false, true] {
        let mut r = demo::make_renderer(rt).unwrap();
        let p = asset::Primitive {
            vertices: [[-0.6, 0., 0.], [0.6, 0., 0.], [0., 1.4, 0.]]
                .map(|position| Vertex {
                    position,
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                    color: [1.; 4],
                })
                .to_vec(),
            indices: vec![0, 1, 2],
            tangents: None,
            material: 0,
        };
        let weights = vec![
            Influence {
                joints: [0; 4],
                weights: [1., 0., 0., 0.]
            };
            3
        ];
        let mut skin = loop {
            if let Some(s) = r.try_upload_skinned_mesh(&p, &weights, 1).unwrap() {
                break s;
            }
            r.test_drain("skin upload");
        };
        let mut instances = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.test_drain("skin instances");
        };
        let color = r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SK1 oracle image"),
            size: wgpu::Extent3d {
                width: 160,
                height: 120,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let camera = Camera::look_at(
            Vec3::new(0., 0.7, 3.),
            Vec3::new(0., 0.7, 0.),
            Vec3::Y,
            0.9,
            0.01,
            30.,
        )
        .unwrap();
        let first = Mat4::from_translation(Vec3::new(-0.3, 0., 0.));
        let second = Mat4::from_scale_rotation_translation(
            Vec3::new(1.5, 0.7, 1.1),
            glam::Quat::from_rotation_y(0.4) * glam::Quat::from_rotation_z(0.5),
            Vec3::new(0.3, 0., 0.),
        );
        for (frame, palette, previous) in
            [(0, first, first), (1, second, first), (2, second, second)]
        {
            skin.set_joint_palette(&[palette]).unwrap();
            loop {
                if r.try_render_view(
                    &color,
                    &camera,
                    &mut [Draw::new(skin.mesh(), &mut instances)],
                    ViewMode::Lit,
                )
                .unwrap()
                .is_some()
                {
                    break;
                }
                r.collect().unwrap();
            }
            r.test_drain("skin oracle frame");
            assert_eq!(
                r.reconstruction_state.as_ref().unwrap().reset,
                frame == 0,
                "animated TR1 must continue after the first frame on rt={rt}"
            );
            let current = read_buffer(&r, &skin.mesh().vertex);
            let old = read_buffer(&r, &skin.mesh().skin.as_ref().unwrap().previous);
            for (i, v) in p.vertices.iter().enumerate() {
                let expected = palette.transform_point3(Vec3::from(v.position));
                let before = previous.transform_point3(Vec3::from(v.position));
                assert!(
                    (Vec3::from_slice(&current[i * 12..i * 12 + 3]) - expected).length() < 1e-5,
                    "current pose rt={rt} frame={frame}"
                );
                assert!(
                    (Vec3::from_slice(&old[i * 12..i * 12 + 3]) - before).length() < 1e-5,
                    "previous submitted pose rt={rt} frame={frame}"
                );
                assert!(
                    (Vec3::from_slice(&current[i * 12 + 3..i * 12 + 6]).length() - 1.).abs() < 1e-5
                );
                let expected_normal = palette
                    .inverse()
                    .transpose()
                    .transform_vector3(Vec3::from(v.normal))
                    .normalize();
                assert!(
                    (Vec3::from_slice(&current[i * 12 + 3..i * 12 + 6]) - expected_normal).length()
                        < 1e-5
                );
            }
        }
        // Compare the integrated visibility/shadow result to ordinary immutable
        // geometry with the exact observed deformed stream. The independent CPU
        // position/normal checks above establish what those observations mean.
        r.set_reconstruction(Reconstruction {
            enabled: false,
            ..Default::default()
        })
        .unwrap();
        let observed = read_buffer(&r, &skin.mesh().vertex);
        let baked: Vec<_> = p
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| Vertex {
                position: std::array::from_fn(|k| observed[i * 12 + k]),
                normal: std::array::from_fn(|k| observed[i * 12 + 3 + k]),
                ..*v
            })
            .collect();
        let baked = loop {
            if let Some(m) = r
                .try_upload_mesh(&MeshData {
                    vertices: &baked,
                    indices: &p.indices,
                })
                .unwrap()
            {
                break m;
            }
            r.test_drain("skin baked oracle upload");
        };
        let mut images = vec![];
        for mesh in [skin.mesh(), &baked] {
            loop {
                if r.try_render_view(
                    &color,
                    &camera,
                    &mut [Draw::new(mesh, &mut instances)],
                    ViewMode::Lit,
                )
                .unwrap()
                .is_some()
                {
                    break;
                }
                r.collect().unwrap();
            }
            r.test_drain("skin integrated image oracle");
            images.push(r.test_capture_color(&color));
        }
        let max = images[0]
            .iter()
            .zip(&images[1])
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            max <= 2,
            "deformed and baked visibility/shadows disagree rt={rt} max_byte_error={max}"
        );
        eprintln!("SK1 deformed_vs_baked rt={rt} max_byte_error={max}");
        if rt {
            for mode in [
                SoftShadows::Exact,
                SoftShadows::Virtual,
                SoftShadows::Interactive,
            ] {
                r.set_soft_shadows(mode);
                skin.set_joint_palette(&[first]).unwrap();
                loop {
                    if r.try_render_view(
                        &color,
                        &camera,
                        &mut [Draw::new(skin.mesh(), &mut instances)],
                        ViewMode::Lit,
                    )
                    .unwrap()
                    .is_some()
                    {
                        break;
                    }
                    r.collect().unwrap();
                }
                r.test_drain("skin shadow methods");
            }
        } else {
            r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
            for (light, request) in [
                (
                    Light::Directional {
                        travel: Vec3::new(-0.4, -1., -0.3),
                        lux: [1000.; 3],
                    },
                    ShadowRequest::Directional {
                        split_ends: vec![5., 20.],
                    },
                ),
                (
                    Light::Point {
                        position: Vec3::new(3., 4., 3.),
                        candela: [500.; 3],
                        range: None,
                    },
                    ShadowRequest::Point {
                        near: 0.05,
                        far: 20.,
                    },
                ),
                (
                    Light::Spot {
                        position: Vec3::new(3., 4., 3.),
                        travel: Vec3::new(-3., -3.3, -3.),
                        candela: [500.; 3],
                        range: None,
                        inner: 0.4,
                        outer: 0.7,
                    },
                    ShadowRequest::Spot {
                        near: 0.05,
                        far: 20.,
                    },
                ),
            ] {
                r.set_lighting_and_shadows(
                    &Lighting {
                        lights: vec![light],
                        environment: [20.; 3],
                        exposure_stops: -9.,
                    },
                    &Shadows {
                        resolution: 256,
                        requests: vec![request],
                        ..Default::default()
                    },
                )
                .unwrap();
                skin.set_joint_palette(&[second]).unwrap();
                loop {
                    if r.try_render_view(
                        &color,
                        &camera,
                        &mut [Draw::new(skin.mesh(), &mut instances)],
                        ViewMode::Lit,
                    )
                    .unwrap()
                    .is_some()
                    {
                        break;
                    }
                    r.collect().unwrap();
                }
                r.test_drain("skin raster shadow light types");
            }
        }
        // A pose-changing draw uses the same programmable material ABI on raster too.
        r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
        r.set_shadows(&Shadows::default()).unwrap();
        loop {
            if r.try_render_view(
                &color,
                &camera,
                &mut [Draw::new(skin.mesh(), &mut instances)],
                ViewMode::Normal,
            )
            .unwrap()
            .is_some()
            {
                break;
            }
            r.collect().unwrap();
        }
        r.test_drain("skin normal raster");
        eprintln!(
            "SK1 component rt={rt} samples_ms={:?}",
            skin.mesh()
                .skin
                .as_ref()
                .unwrap()
                .test_profile(&r.device, &r.queue)
        );
        assert_eq!(r.stats().timing_failures, 0);
    }
}

#[test]
#[ignore = "SK1 guarded GPU lease; Fox.glb and optional quarry in RENDERLAB_SK1_* env"]
fn gpu_skinned_fox_walk_fixture_and_quarry() {
    let _validation = validation::begin(true);
    let source = std::env::var("RENDERLAB_SK1_SOURCE").unwrap();
    let out = std::env::var("RENDERLAB_SK1_OUT").unwrap();
    let quarry = std::env::var("RENDERLAB_SK1_QUARRY").ok();
    demo::run(
        std::path::Path::new(&source),
        std::path::Path::new(&out),
        quarry.as_deref().map(std::path::Path::new),
    )
    .unwrap();
}
