//! MAT1's material attribution images. One asset upload serves every shader/pose.
use super::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "../examples/m4/cameras.rs"]
mod cameras;

/// An untextured plane separates the standard model from scan texture filtering.
#[test]
#[ignore = "MAT1 manifest, shared GPU lease and experimental RT acknowledgement"]
fn gpu_environment_specular_response() {
    use crate::reference;
    let manifest: Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_MAT1_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let environment = asset::environment::decode(
        &fs::read(manifest["environment"].as_str().unwrap()).unwrap(),
        asset::environment::Limits::unbounded(),
    )
    .unwrap();
    let (d, q) = device(false);
    // Sequential measurement tracer submissions are outside the renderer's ledger.
    let reference_device = d.clone();
    let reference_queue = q.clone();
    let mut r = suite_renderer(d, q, SRGB, RendererOptions::default(), SOURCE).unwrap();
    let sky = loop {
        if let Some(s) = r.try_upload_environment(&environment).unwrap() {
            break s;
        }
        r.test_drain("MAT1 sky fixture admission");
    };
    let vertices = [
        [-100., 0., -100.],
        [100., 0., -100.],
        [100., 0., 100.],
        [-100., 0., 100.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0., 1., 0.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let indices = [0, 2, 1, 0, 3, 2];
    let mesh = upload(&mut r, &vertices, &indices);
    let mut inst = instances(&mut r, &[Mat4::IDENTITY]);
    let mat = material(
        &mut r,
        &MaterialDescriptor {
            base_color: [0.1, 0.1, 0.1, 1.],
            roughness: 0.85,
            metallic: 0.,
            double_sided: true,
            ..Default::default()
        },
    );
    let mut scene = reference::Scene {
        meshes: vec![reference::Mesh {
            positions: vertices.iter().map(|v| v.position).collect(),
            normals: vec![[0., 1., 0.]; 4],
            indices: indices.to_vec(),
            ..Default::default()
        }],
        instances: vec![reference::Instance {
            casts_shadows: true,
            mesh: 0,
            material: 0,
            transform: Mat4::IDENTITY,
        }],
        materials: vec![reference::Material {
            base_color: [0.1; 3],
            roughness: 0.85,
            ..Default::default()
        }],
        ..Default::default()
    };
    let target = color_target(&r, 17, 17);
    let mut rows = vec![];
    for (name, map) in [("constant", false), ("quarry-map", true)] {
        r.set_lighting(&Lighting {
            environment: [16384.; 3],
            exposure_stops: -14.,
            ..Default::default()
        })
        .unwrap();
        r.set_environment(map.then_some(&sky), Default::default())
            .unwrap();
        scene.sky = if map { [0.; 3] } else { [16384.; 3] };
        scene.environment = map.then(|| reference::Environment {
            radiance: environment.specular_radiance()[0].clone(),
            yaw: environment.metadata().rotation_y_radians,
            intensity: 1.,
        });
        for nv in [0.3f32, 0.8] {
            let eye = Vec3::new((1. - nv * nv).sqrt(), nv, 0.) * 10.;
            let camera = Camera::look_at(eye, Vec3::ZERO, Vec3::Y, 0.01, 0.01, 1000.).unwrap();
            let pose = reference::Pose {
                eye,
                target: Vec3::ZERO,
            };
            r.test_drain("MAT1 reference submission boundary");
            let image = reference::render(
                &reference_device,
                &reference_queue,
                &scene,
                &reference::View {
                    open: pose,
                    close: pose,
                    up: Vec3::Y,
                    fov_y: 0.01,
                    width: 17,
                    height: 17,
                },
                &reference::Settings {
                    samples: 16384,
                    samples_per_dispatch: 64,
                    max_depth: 2,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(image.invalid_pixels, 0);
            let center = 8 * 17 + 8;
            let mut measured = vec![];
            for variant in manifest["programs"].as_array().unwrap() {
                if !map && variant["name"] != "baseline" {
                    continue;
                }
                let source = variant["source"].as_str().map_or_else(
                    || crate::STANDARD_PROGRAM.to_owned(),
                    |p| fs::read_to_string(p).unwrap(),
                );
                r.test_reload_standard(&source).unwrap();
                pixels(
                    &mut r,
                    &target,
                    &camera,
                    &mut [Draw::new(&mesh, &mut inst).with_material(&mat)],
                    ViewMode::Lit,
                );
                let hdr = super::super::lighting_tests::hdr(&mut r);
                if !map {
                    for c in 0..3 {
                        let actual = hdr[center * 4 + c] * 16384.;
                        let expected = image.mean[center][c];
                        assert!(
                            (actual - expected).abs()
                                < 0.01 * expected + 4. * image.standard_error[center][c],
                            "constant-sky material control: {actual} vs {expected}"
                        );
                    }
                }
                measured.push(json!({"program":variant["name"],"radiance":hdr[center*4..center*4+3].iter().map(|v|v*16384.).collect::<Vec<_>>()}));
            }
            rows.push(json!({"sky":name,"nv":nv,"reference":image.mean[center],"reference_se":image.standard_error[center],"measurements":measured}));
        }
    }
    fs::write(
        out.join("sky-fixture.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "MAT1 manifest, shared GPU lease and experimental RT acknowledgement"]
fn gpu_quarry_material_response() {
    let manifest: Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_MAT1_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let path = |key: &str| Path::new(manifest[key].as_str().unwrap());
    let out = PathBuf::from(path("output"));
    fs::create_dir_all(&out).unwrap();
    let source = asset::decode(
        &fs::read(path("asset")).unwrap(),
        asset::Limits::unlimited(),
    )
    .unwrap();
    let frames = cameras::Cameras::load(path("cameras")).unwrap();
    let environment = asset::environment::decode(
        &fs::read(path("environment")).unwrap(),
        asset::environment::Limits::unbounded(),
    )
    .unwrap();
    let lights: Value = serde_json::from_slice(&fs::read(path("lighting")).unwrap()).unwrap();
    let v3 = |v: &Value| Vec3::from_array(std::array::from_fn(|k| v[k].as_f64().unwrap() as f32));
    let traced = traced_suite();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let features = crate::RECEIVER_FEATURES
        | if traced {
            crate::TRACED_FEATURES | wgpu::Features::EXPERIMENTAL_RAY_QUERY
        } else {
            crate::VIRTUAL_FEATURES | crate::VISIBILITY_FEATURES | crate::VISIBILITY_RASTER_FEATURES
        };
    if traced {
        assert_eq!(
            std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
            Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        );
    }
    let (d, q) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: crate::traced_limits(wgpu::Limits {
            max_blas_primitive_count: 2_000_000,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096,
            max_acceleration_structures_per_shader_stage: 1,
            max_storage_buffers_per_shader_stage: 16,
            max_storage_buffer_binding_size: adapter.limits().max_storage_buffer_binding_size,
            max_buffer_size: adapter.limits().max_buffer_size,
            ..Default::default()
        }),
        // The ignored test's caller accepts wgpu's experimental contract above.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .unwrap();
    let limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    // AA state (docs/iq1-quarry-image.md): 1x, TR1 off; anisotropy from the manifest,
    // where 1 (the default) is the shipping value.
    let options = RendererOptions {
        samples: SampleCount::One,
        anisotropy_clamp: manifest["anisotropy"].as_u64().unwrap_or(1) as u16,
    };
    let mut r = if traced {
        let mut r = Renderer::new_with_hardware_shadows(
            d,
            q,
            SRGB,
            limits,
            options,
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r.set_soft_shadows(SoftShadows::Interactive);
        r
    } else {
        let mut r = Renderer::new_with_options(d, q, SRGB, limits, options, SOURCE).unwrap();
        r.set_soft_shadows(SoftShadows::Virtual);
        r
    };
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: v3(&lights["lights"][0]["travel"]),
                lux: v3(&lights["lights"][0]["lux"]).to_array(),
            }],
            environment: [0.; 3],
            exposure_stops: -14.,
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
    let sky = loop {
        if let Some(sky) = r.try_upload_environment(&environment).unwrap() {
            break sky;
        }
        r.test_drain("MAT1 environment admission");
    };
    r.set_environment(Some(&sky), Default::default()).unwrap();
    drop(environment);
    let mut prepared =
        prepared::Prepared::new_with_limits(source, asset::Limits::unlimited(), 1_000_000).unwrap();
    while !prepared.ready() {
        prepared.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("MAT1 prepared asset");
    let mut rows = vec![];
    for variant in manifest["programs"].as_array().unwrap() {
        let name = variant["name"].as_str().unwrap();
        let source = variant["source"].as_str().map_or_else(
            || crate::STANDARD_PROGRAM.to_owned(),
            |p| fs::read_to_string(p).unwrap(),
        );
        r.test_reload_standard(&source).unwrap();
        for (pose, frame) in frames.frames.iter().take(2).enumerate() {
            let target = color_target(&r, frame.width, frame.height);
            pixels(
                &mut r,
                &target,
                &frame.camera().unwrap(),
                &mut prepared.draws(Visibility::Unculled),
                ViewMode::Lit,
            );
            let hdr = super::super::lighting_tests::hdr(&mut r);
            let raw: Vec<_> = hdr
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().flat_map(|v| (v * 16384.).to_le_bytes()))
                .collect();
            fs::write(out.join(format!("{name}-p{pose}.f32")), raw).unwrap();
            rows.push(
                json!({"program":name,"pose":pose,"width":frame.width,"height":frame.height}),
            );
        }
    }
    fs::write(
        out.join("capture.json"),
        serde_json::to_vec_pretty(
            &json!({"adapter":format!("{:?}",adapter.get_info()),"traced":traced,"rows":rows}),
        )
        .unwrap(),
    )
    .unwrap();
}
