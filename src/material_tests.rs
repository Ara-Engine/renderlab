use super::*;
use crate::asset::{Filter, Mip, MipmapFilter, Sampler, TextureRole, Wrap};
#[path = "../viewer/src/prepared.rs"]
pub(super) mod prepared;
#[path = "material_response_tests.rs"]
mod response;
#[path = "compressed_texture_tests.rs"]
mod compressed;

const SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
fn material_renderer(samples: SampleCount) -> Renderer {
    let (device, queue) = device(true);
    suite_renderer(
        device,
        queue,
        SRGB,
        RendererOptions {
            samples,
            ..Default::default()
        },
        SOURCE,
    )
    .unwrap()
}
fn color_target(r: &Renderer, w: u32, h: u32) -> wgpu::Texture {
    r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("M3 sRGB color"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SRGB,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn texture(r: &mut Renderer, data: &asset::Texture) -> Texture {
    let start = Instant::now();
    loop {
        if let Some(t) = r.try_upload_texture(data).unwrap() {
            return t;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::yield_now();
    }
}
fn material(r: &mut Renderer, data: &MaterialDescriptor<'_>) -> Material {
    let start = Instant::now();
    loop {
        if let Some(t) = r.try_create_material(data).unwrap() {
            return t;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::yield_now();
    }
}
fn pixels(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    draws: &mut [Draw<'_>],
    mode: ViewMode,
) -> Vec<u8> {
    let start = Instant::now();
    loop {
        if r.try_render_view(color, camera, draws, mode)
            .unwrap()
            .is_some()
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::yield_now();
    }
    r.test_capture_color(color)
}
fn srgb(value: f64) -> u8 {
    ((if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }) * 255.0)
        .round() as u8
}
fn linear(value: u8) -> f64 {
    let v = f64::from(value) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn close(actual: &[u8], expected: [u8; 3]) {
    for (&actual, expected) in actual.iter().take(3).zip(expected) {
        assert!(
            actual.abs_diff(expected) <= 2,
            "actual={actual} expected={expected}"
        );
    }
}
fn swatch_vertices(uv: [f32; 2]) -> [Vertex; 4] {
    [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ]
    .map(|position| Vertex {
        position,
        normal: [1.0, 1.0, 1.0],
        uv,
        color: [1.0; 4],
    })
}
fn save_color(name: &str, w: u32, h: u32, rgba: &[u8]) {
    fs::create_dir_all(evidence_dir().join("captures")).unwrap();
    let mut file = fs::File::create(evidence_dir().join(format!("captures/{name}.ppm"))).unwrap();
    write!(file, "P6\n{w} {h}\n255\n").unwrap();
    file.write_all(
        &rgba
            .chunks_exact(4)
            .flat_map(|p| p[..3].iter().copied())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    fs::write(evidence_dir().join(format!("captures/{name}.rgba8")), rgba).unwrap();
}
#[test]
#[ignore = "Vulkan GPU; numeric material/sampler/mip/normal/sidedness contracts"]
fn gpu_material_values_sampler_normals_and_sidedness() {
    let _validation = validation::begin(true);
    let mut r = material_renderer(SampleCount::One);
    let color = color_target(&r, 193, 193);
    let cam = camera(0.0);
    let center = (96 * 193 + 96) * 4;
    let base = texture(
        &mut r,
        &asset::Texture {
            role: TextureRole::BaseColorSrgb,
            mips: vec![Mip {
                width: 1,
                height: 1,
                rgba8: vec![128, 64, 32, 20],
            }],
        },
    );
    let mr = texture(
        &mut r,
        &asset::Texture {
            role: TextureRole::MetallicRoughnessLinear,
            mips: vec![Mip {
                width: 1,
                height: 1,
                rgba8: vec![253, 64, 192, 255],
            }],
        },
    );
    let before = r.stats();
    assert!(
        r.try_create_material(&MaterialDescriptor {
            base_color_texture: Some(TextureBinding {
                texture: &mr,
                sampler: Sampler::default()
            }),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        r.try_create_material(&MaterialDescriptor {
            roughness: f32::NAN,
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(before.ownership, r.stats().ownership);
    assert_eq!(before.uploads, r.stats().uploads);
    let data = MaterialDescriptor {
        base_color: [0.8, 0.6, 0.4, 0.0],
        metallic: 0.5,
        roughness: 0.75,
        base_color_texture: Some(TextureBinding {
            texture: &base,
            sampler: Sampler::default(),
        }),
        metallic_roughness_texture: Some(TextureBinding {
            texture: &mr,
            sampler: Sampler::default(),
        }),
        double_sided: false,
        ..Default::default()
    };
    let m = material(&mut r, &data);
    let m2 = material(&mut r, &data);
    assert_eq!(before.uploads.textures, r.stats().uploads.textures);
    assert_eq!(
        before.ownership.texture_creations,
        r.stats().ownership.texture_creations
    );
    assert_eq!(r.stats().uploads.materials - before.uploads.materials, 96);
    drop(m2);
    drop(base);
    drop(mr);
    let mut vertices = swatch_vertices([0.5; 2]);
    for v in &mut vertices {
        v.color = [0.5, 0.75, 1.0, 0.0];
    }
    let mesh = upload(&mut r, &vertices, &[0, 1, 2, 0, 2, 3]);
    let mut inst = instances(&mut r, &[Mat4::IDENTITY]);
    for (mode, expected) in [
        (
            ViewMode::BaseColor,
            [
                srgb(linear(128) * 0.8 * 0.5),
                srgb(linear(64) * 0.6 * 0.75),
                srgb(linear(32) * 0.4),
            ],
        ),
        (ViewMode::Metallic, [srgb(192.0 / 255.0 * 0.5); 3]),
        (ViewMode::Roughness, [srgb(64.0 / 255.0 * 0.75); 3]),
    ] {
        let image = pixels(
            &mut r,
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst).with_material(&m)],
            mode,
        );
        close(&image[center..], expected);
        assert_eq!(image[center + 3], 255);
    }
    for (i, model) in [
        Mat4::IDENTITY,
        Mat4::from_scale(Vec3::new(-2.0, 3.0, 0.5)),
        Mat4::from_rotation_z(0.3) * Mat4::from_scale(Vec3::new(0.5, 2.0, 1.0)),
    ]
    .into_iter()
    .enumerate()
    {
        inst.set(0, model).unwrap();
        let image = pixels(
            &mut r,
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst).with_material(&m)],
            ViewMode::Normal,
        );
        let expected = (glam::Mat3::from_mat4(model).inverse().transpose() * Vec3::ONE).normalize()
            * 0.5
            + Vec3::splat(0.5);
        close(
            &image[center..],
            expected.to_array().map(|v| srgb(v.into())),
        );
        save_color(&format!("normal-transform-{i}"), 193, 193, &image);
    }
    inst.set(0, Mat4::IDENTITY).unwrap();
    let reversed = upload(&mut r, &vertices, &[2, 1, 0, 3, 2, 0]);
    let empty = pixels(&mut r, &color, &cam, &mut [], ViewMode::BaseColor);
    for model in [Mat4::IDENTITY, Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))] {
        inst.set(0, model).unwrap();
        let rejected = pixels(
            &mut r,
            &color,
            &cam,
            &mut [Draw::new(&reversed, &mut inst).with_material(&m)],
            ViewMode::BaseColor,
        );
        assert_eq!(rejected, empty);
        let visible = pixels(
            &mut r,
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst).with_material(&m)],
            ViewMode::BaseColor,
        );
        assert_ne!(visible, empty);
    }
    let double = material(
        &mut r,
        &MaterialDescriptor {
            double_sided: true,
            ..Default::default()
        },
    );
    let visible = pixels(
        &mut r,
        &color,
        &cam,
        &mut [Draw::new(&reversed, &mut inst).with_material(&double)],
        ViewMode::BaseColor,
    );
    assert_ne!(visible, empty);
    let mut both = instances(
        &mut r,
        &[
            Mat4::from_translation(Vec3::new(-0.8, 0.0, 0.0)) * Mat4::from_scale(Vec3::splat(0.6)),
            Mat4::from_translation(Vec3::new(0.8, 0.0, 0.0))
                * Mat4::from_scale(Vec3::new(-0.6, 0.6, 0.6)),
        ],
    );
    let mixed = pixels(
        &mut r,
        &color,
        &cam,
        &mut [Draw::new(&mesh, &mut both).with_material(&m)],
        ViewMode::BaseColor,
    );
    save_color("single-sided-mixed-mirror", 193, 193, &mixed);
    for x in [70, 122] {
        assert_ne!(
            &mixed[(96 * 193 + x) * 4..(96 * 193 + x) * 4 + 3],
            &empty[(96 * 193 + x) * 4..(96 * 193 + x) * 4 + 3]
        );
    }
    for model in [
        Mat4::from_scale(Vec3::new(1.0, 1.0, 0.0)),
        Mat4::perspective_rh(1.0, 1.0, 0.1, 10.0),
    ] {
        inst.set(0, model).unwrap();
        assert!(
            r.try_render_view(
                &color,
                &cam,
                &mut [Draw::new(&mesh, &mut inst)],
                ViewMode::Normal
            )
            .is_err()
        );
        assert!(
            r.try_render(
                &color,
                &cam,
                &mut [Draw::new(&mesh, &mut inst).with_material(&m)]
            )
            .is_err()
        );
        pixels(
            &mut r,
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst)],
            ViewMode::BaseColor,
        );
    }
    inst.set(0, Mat4::IDENTITY).unwrap();
    sampler_mips(&mut r, &color, &cam, &mut inst, center);
    let mut foreign = material_renderer(SampleCount::One);
    let foreign_material = material(&mut foreign, &MaterialDescriptor::default());
    let before = r.stats();
    assert!(
        r.try_render(
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst).with_material(&foreign_material)]
        )
        .is_err()
    );
    assert_eq!(before.ownership, r.stats().ownership);
    assert_eq!(before.uploads, r.stats().uploads);
    let before = foreign.stats();
    assert!(
        foreign
            .try_create_material(&MaterialDescriptor {
                base_color_texture: Some(TextureBinding {
                    texture: &m._textures[0],
                    sampler: Sampler::default()
                }),
                ..Default::default()
            })
            .is_err()
    );
    assert_eq!(before.ownership, foreign.stats().ownership);
    foreign.test_drain("foreign affinity test cleanup");
    r.test_drain("material tests final");
}
fn sampler_mips(
    r: &mut Renderer,
    color: &wgpu::Texture,
    cam: &Camera,
    inst: &mut Instances,
    center: usize,
) {
    let mut data = asset::Texture {
        role: TextureRole::BaseColorSrgb,
        mips: vec![
            Mip {
                width: 2,
                height: 2,
                rgba8: vec![
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
            },
            Mip {
                width: 1,
                height: 1,
                rgba8: vec![128, 128, 128, 255],
            },
        ],
    };
    let tex = texture(r, &data);
    let mesh = upload(r, &swatch_vertices([1.25, 0.25]), &[0, 1, 2, 0, 2, 3]);
    for (wrap, expected) in [
        (Wrap::Repeat, [255, 0, 0]),
        (Wrap::Clamp, [0, 255, 0]),
        (Wrap::Mirror, [0, 255, 0]),
    ] {
        let m = material(
            r,
            &MaterialDescriptor {
                base_color_texture: Some(TextureBinding {
                    texture: &tex,
                    sampler: Sampler {
                        wrap_u: wrap,
                        wrap_v: Wrap::Clamp,
                        mag: Filter::Nearest,
                        min: Filter::Nearest,
                        mipmap: MipmapFilter::None,
                    },
                }),
                ..Default::default()
            },
        );
        let image = pixels(
            r,
            color,
            cam,
            &mut [Draw::new(&mesh, inst).with_material(&m)],
            ViewMode::BaseColor,
        );
        close(&image[center..], expected);
    }
    let linear_mesh = upload(r, &swatch_vertices([0.5, 0.5]), &[0, 1, 2, 0, 2, 3]);
    let m = material(
        r,
        &MaterialDescriptor {
            base_color_texture: Some(TextureBinding {
                texture: &tex,
                sampler: Sampler {
                    mipmap: MipmapFilter::None,
                    ..Default::default()
                },
            }),
            ..Default::default()
        },
    );
    let image = pixels(
        r,
        color,
        cam,
        &mut [Draw::new(&linear_mesh, inst).with_material(&m)],
        ViewMode::BaseColor,
    );
    close(&image[center..], [srgb(0.5); 3]);
    let mut minified_vertices = swatch_vertices([0.0; 2]);
    for vertex in &mut minified_vertices {
        vertex.uv = [
            vertex.position[0] * 1000.0 + 0.5,
            vertex.position[1] * 1000.0 + 0.5,
        ];
    }
    let minified = upload(r, &minified_vertices, &[0, 1, 2, 0, 2, 3]);
    for (mag, min) in [
        (Filter::Linear, Filter::Nearest),
        (Filter::Nearest, Filter::Linear),
    ] {
        let m = material(
            r,
            &MaterialDescriptor {
                base_color_texture: Some(TextureBinding {
                    texture: &tex,
                    sampler: Sampler {
                        mag,
                        min,
                        mipmap: MipmapFilter::None,
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
        );
        let image = pixels(
            r,
            color,
            cam,
            &mut [Draw::new(&minified, inst).with_material(&m)],
            ViewMode::BaseColor,
        );
        save_color(
            &format!("no-mips-min-{min:?}-mag-{mag:?}"),
            color.width(),
            color.height(),
            &image,
        );
        if min == Filter::Nearest {
            assert!(
                [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]]
                    .iter()
                    .any(|texel| image[center..center + 3] == texel[..]),
                "minified nearest must select a base texel: {:?}",
                &image[center..center + 3]
            );
        } else {
            close(&image[center..], [srgb(0.5); 3]);
        }
    }
    data.mips = vec![
        Mip {
            width: 4,
            height: 4,
            rgba8: [255, 0, 0, 255].repeat(16),
        },
        Mip {
            width: 2,
            height: 2,
            rgba8: [0, 255, 0, 255].repeat(4),
        },
        Mip {
            width: 1,
            height: 1,
            rgba8: vec![0, 0, 255, 255],
        },
    ];
    let tex = texture(r, &data);
    let mut vertices = swatch_vertices([0.0; 2]);
    for v in &mut vertices {
        v.uv = [v.position[0] * 1000.0, v.position[1] * 1000.0];
    }
    let mesh = upload(r, &vertices, &[0, 1, 2, 0, 2, 3]);
    for (mipmap, expected) in [
        (MipmapFilter::None, [255, 0, 0]),
        (MipmapFilter::Nearest, [0, 0, 255]),
        (MipmapFilter::Linear, [0, 0, 255]),
    ] {
        let m = material(
            r,
            &MaterialDescriptor {
                base_color_texture: Some(TextureBinding {
                    texture: &tex,
                    sampler: Sampler {
                        mipmap,
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
        );
        let image = pixels(
            r,
            color,
            cam,
            &mut [Draw::new(&mesh, inst).with_material(&m)],
            ViewMode::BaseColor,
        );
        close(&image[center..], expected);
    }
    let before = r.stats();
    data.mips[1].width = 1;
    assert!(r.try_upload_texture(&data).is_err());
    assert_eq!(before.ownership, r.stats().ownership);
    assert_eq!(before.uploads, r.stats().uploads);
}

fn load_asset(name: &str) -> prepared::Prepared {
    let hash = match name {
        "boxes" => "d62f14452111fd34280b1cd56841bf08d79d6d432ee38270c6d44773e25c219c",
        _ => "8fabb94f03e50e703e4d4d6eb1371be44c6f9e790c06bae1a65c5ccdae10cd4e",
    };
    let path = std::env::var_os("RENDERLAB_M3_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "measurements/m3-assets/20261103-005837/cache/assets".into());
    prepared::Prepared::load(&path.join(hash)).unwrap()
}
fn upload_asset(r: &mut Renderer, asset: &mut prepared::Prepared) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !asset.ready() {
        asset.advance(r).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn prepared_viewer_bounds_batch_expansion_before_upload() {
    let mut source = asset::Asset {
        meshes: vec![asset::Mesh {
            primitives: vec![
                asset::Primitive {
                    tangents: None,
                    vertices: swatch_vertices([0.0; 2]).to_vec(),
                    indices: vec![0, 1, 2],
                    material: 0
                };
                64
            ],
        }],
        placements: vec![
            asset::Placement {
                mesh: 0,
                transform: Mat4::IDENTITY.to_cols_array_2d()
            };
            16_384
        ],
        materials: vec![asset::Material::default()],
        textures: vec![],
    };
    assert!(
        prepared::Prepared::new(source.clone())
            .err()
            .unwrap()
            .to_string()
            .contains("slot limit")
    );
    source.meshes[0].primitives.truncate(1);
    source.placements.truncate(1);
    source.placements[0].transform = Mat4::from_scale(Vec3::splat(1e30)).to_cols_array_2d();
    assert!(
        prepared::Prepared::new(source.clone())
            .err()
            .unwrap()
            .to_string()
            .contains("numerical range")
    );
    source.placements[0].transform = Mat4::IDENTITY.to_cols_array_2d();
    let prepared = prepared::Prepared::new(source.clone()).unwrap();
    assert!(prepared.camera(0.4, 0.25, Vec3::ZERO, false, 1.0).is_ok());
    source.placements[0].transform = Mat4::from_translation(Vec3::splat(1e10)).to_cols_array_2d();
    let distant = prepared::Prepared::new(source).unwrap();
    assert!(distant.camera(0.4, 0.25, Vec3::ZERO, false, 1.0).is_err());
    assert_eq!(prepared.max_instances(), 1);
    assert!(prepared.radius.is_finite() && prepared.radius > 1.0);
}
#[test]
#[ignore = "Vulkan GPU; both prepared assets, diagnostics, motion and transactional reload at 1x/4x"]
fn gpu_assets_aa_reload_and_reuse() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let mut r = material_renderer(samples);
        for name in ["boxes", "curved"] {
            let mut asset = load_asset(name);
            assert!(asset.max_instances() > 0);
            upload_asset(&mut r, &mut asset);
            let color = color_target(&r, 641, 359);
            let camera = asset
                .camera(0.4, 0.25, Vec3::ZERO, false, 641.0 / 359.0)
                .unwrap();
            let baseline = pixels(
                &mut r,
                &color,
                &camera,
                &mut asset.draws(Visibility::Frustum),
                ViewMode::BaseColor,
            );
            assert!(
                baseline
                    .chunks_exact(4)
                    .filter(|p| p[0] > 80 || p[1] > 80 || p[2] > 90)
                    .count()
                    > 2000
            );
            save_color(
                &format!("{name}-{}x-color", samples as u32),
                641,
                359,
                &baseline,
            );
            let unculled = pixels(
                &mut r,
                &color,
                &camera,
                &mut asset.draws(Visibility::Unculled),
                ViewMode::BaseColor,
            );
            assert_eq!(baseline, unculled);
            for mode in [ViewMode::Normal, ViewMode::Metallic, ViewMode::Roughness] {
                let image = pixels(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                    mode,
                );
                assert_ne!(baseline, image);
                save_color(
                    &format!("{name}-{}x-{mode:?}", samples as u32),
                    641,
                    359,
                    &image,
                );
            }
            pixels(
                &mut r,
                &color,
                &camera,
                &mut asset.draws(Visibility::Frustum),
                ViewMode::BaseColor,
            );
            let before = r.stats();
            for _ in 0..40 {
                let report = render(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                );
                assert_eq!(report.uploads, UploadBytes::default());
            }
            r.test_drain("asset reuse");
            assert_eq!(before.uploads, r.stats().uploads);
            assert_eq!(before.ownership.created(), r.stats().ownership.created());
            assert_eq!(before.pipelines, r.stats().pipelines);
            let revision = r.stats().revision;
            for source in [
                "bad WGSL".to_owned(),
                SOURCE.replace("@group(2) @binding(1)", "@group(2) @binding(7)"),
            ] {
                assert!(matches!(
                    r.reload(&source),
                    Err(ReloadError::Rejected { .. })
                ));
                assert_eq!(revision, r.stats().revision);
                assert_eq!(
                    baseline,
                    pixels(
                        &mut r,
                        &color,
                        &camera,
                        &mut asset.draws(Visibility::Frustum),
                        ViewMode::BaseColor
                    )
                );
            }
            r.reload(&SOURCE.replace("vec3<f32>(1.0, 1.0, 1.0)", "vec3<f32>(0.5, 1.0, 1.0)"))
                .unwrap();
            assert_ne!(
                baseline,
                pixels(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                    ViewMode::BaseColor
                )
            );
            r.reload(SOURCE).unwrap();
            assert_eq!(
                baseline,
                pixels(
                    &mut r,
                    &color,
                    &camera,
                    &mut asset.draws(Visibility::Frustum),
                    ViewMode::BaseColor
                )
            );
            for frame in 0..4 {
                asset.animate((frame + 1) as f32 * 0.18).unwrap();
                let moving_camera = asset
                    .camera(
                        0.4 + frame as f32 * 0.12,
                        0.25,
                        Vec3::ZERO,
                        false,
                        641.0 / 359.0,
                    )
                    .unwrap();
                let motion = pixels(
                    &mut r,
                    &color,
                    &moving_camera,
                    &mut asset.draws(Visibility::Frustum),
                    ViewMode::BaseColor,
                );
                assert_ne!(baseline, motion);
                save_color(
                    &format!("{name}-{}x-motion-{frame}", samples as u32),
                    641,
                    359,
                    &motion,
                );
            }
            let portrait = color_target(&r, 359, 641);
            let camera = asset
                .camera(0.4, 0.25, Vec3::ZERO, false, 359.0 / 641.0)
                .unwrap();
            let image = pixels(
                &mut r,
                &portrait,
                &camera,
                &mut asset.draws(Visibility::Frustum),
                ViewMode::BaseColor,
            );
            save_color(
                &format!("{name}-{}x-portrait", samples as u32),
                359,
                641,
                &image,
            );
            assert_eq!(
                r.stats().ownership.target_bytes.live,
                359 * 641 * 4 * if samples == SampleCount::One { 1 } else { 8 }
            );
            eprintln!(
                "asset_gpu_evidence name={name} samples={samples:?} stats={:?}",
                r.stats()
            );
        }
    }
}

#[test]
#[ignore = "Vulkan GPU; MSAA edges against 8x spatial reference, numerical image error"]
fn gpu_aa_edge_reference() {
    let _validation = validation::begin(true);
    let mut images = Vec::new();
    for (samples, width, height) in [
        (SampleCount::One, 193, 127),
        (SampleCount::Four, 193, 127),
        (SampleCount::One, 193 * 8, 127 * 8),
    ] {
        let mut r = material_renderer(samples);
        let color = color_target(&r, width, height);
        let vertices =
            [[-1.4, -0.7, 0.0], [1.3, -0.49, 0.0], [-0.12, 1.2, 0.0]].map(|position| Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv: [0.0; 2],
                color: [1.0; 4],
            });
        let mesh = upload(&mut r, &vertices, &[0, 1, 2]);
        let mut inst = instances(&mut r, &[Mat4::from_rotation_z(0.17)]);
        let image = pixels(
            &mut r,
            &color,
            &camera(0.0),
            &mut [Draw::new(&mesh, &mut inst)],
            ViewMode::BaseColor,
        );
        save_color(
            &format!("edge-{width}-{}x", samples as u32),
            width,
            height,
            &image,
        );
        images.push(image);
    }
    let mut reference = vec![0; 193 * 127 * 4];
    for y in 0..127 {
        for x in 0..193 {
            for c in 0..3 {
                let mut sum = 0.0;
                for dy in 0..8 {
                    for dx in 0..8 {
                        sum += linear(images[2][((y * 8 + dy) * 193 * 8 + x * 8 + dx) * 4 + c]);
                    }
                }
                reference[(y * 193 + x) * 4 + c] = srgb(sum / 64.0);
            }
            reference[(y * 193 + x) * 4 + 3] = 255;
        }
    }
    let error = |image: &[u8]| {
        image
            .chunks_exact(4)
            .zip(reference.chunks_exact(4))
            .map(|(a, b)| (linear(a[0]) - linear(b[0])).abs())
            .sum::<f64>()
    };
    let one = error(&images[0]);
    let four = error(&images[1]);
    assert!(four < one * 0.65, "1x={one} 4x={four}");
    save_color("edge-8x-reference", 193, 127, &reference);
    fs::write(evidence_dir().join("captures/edge-errors.txt"),format!("extent=193x127 reference=8x8 linear-light box reduction linear_r_absolute_sum_1x={one} linear_r_absolute_sum_4x={four}\n")).unwrap();
    eprintln!("aa_edge_error one={one} four={four} reference=8x8 linear_space=true");
}

#[test]
#[ignore = "Vulkan GPU; material/texture/MSAA replacement with incomplete prior frames"]
fn gpu_material_target_replacement_overlap() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let mut r = material_renderer(samples);
        let vertices = swatch_vertices([0.5; 2]);
        let indices: Vec<_> = [0, 1, 2, 0, 2, 3]
            .into_iter()
            .cycle()
            .take(2048 * 3)
            .collect();
        let mesh = upload(&mut r, &vertices, &indices);
        let models: Vec<_> = (0..48)
            .map(|i| Mat4::from_translation(Vec3::new(0.0, 0.0, i as f32 * 0.0001)))
            .collect();
        let mut inst = instances(&mut r, &models);
        let data = asset::Texture {
            role: TextureRole::BaseColorSrgb,
            mips: (0..7)
                .map(|i| {
                    let side = 64 >> i;
                    Mip {
                        width: side,
                        height: side,
                        rgba8: [128, 192, 64, 255].repeat((side * side) as usize),
                    }
                })
                .collect(),
        };
        let mut tex = texture(&mut r, &data);
        fn make(tex: &Texture) -> MaterialDescriptor<'_> {
            MaterialDescriptor {
                base_color_texture: Some(TextureBinding {
                    texture: tex,
                    sampler: Sampler::default(),
                }),
                ..Default::default()
            }
        }
        let mut mat = material(&mut r, &make(&tex));
        let mut color = color_target(&r, 512, 384);
        render(
            &mut r,
            &color,
            &camera(0.0),
            &mut [Draw::new(&mesh, &mut inst).with_material(&mat)],
        );
        r.test_drain("M3 replacement setup");
        let setup = r.stats();
        let mut waves = [HighWater::default(); 3];
        let mut texture_overlap = [0; 3];
        let mut material_overlap = [0; 3];
        let mut target_overlap = [0; 3];
        let mut log = fs::File::create(
            evidence_dir().join(format!("material-stress-{}x.csv", samples as u32)),
        )
        .unwrap();
        writeln!(log,"frame,incomplete_before_upload,incomplete_before_material,incomplete_before_resize,buffer_bytes,texture_bytes,allocated,reserved,target_owned_bytes").unwrap();
        for frame in 0..(32 + 3 * 96) {
            let (before_texture, s) = r.test_overlap();
            let before_material;
            if frame % 2 == 0 {
                tex = texture(&mut r, &data);
                before_material = 0;
            } else {
                before_material = r.test_overlap().0;
                mat = material(&mut r, &make(&tex));
            }
            let (before_target, _) = r.test_overlap();
            color = color_target(&r, 512 + (frame % 2) as u32, 384);
            render(
                &mut r,
                &color,
                &camera(0.0),
                &mut [Draw::new(&mesh, &mut inst).with_material(&mat)],
            );
            let after = r.stats();
            assert!(after.flights <= 3);
            assert!(
                after.ownership.texture_bytes.live
                    <= setup.ownership.texture_bytes.live
                        + data.mips.iter().map(|m| m.rgba8.len() as u64).sum::<u64>()
            );
            assert_eq!(
                after.ownership.material_bytes.live,
                setup.ownership.material_bytes.live
            );
            if frame >= 32 {
                let wave = (frame - 32) / 96;
                waves[wave].observe(&s);
                waves[wave].observe(&after);
                texture_overlap[wave] += usize::from(frame % 2 == 0 && before_texture > 0);
                material_overlap[wave] += usize::from(before_material > 0);
                target_overlap[wave] += usize::from(before_target > 0);
            }
            writeln!(
                log,
                "{frame},{before_texture},{before_material},{before_target},{},{},{},{},{}",
                after.hal.buffer_bytes,
                after.hal.texture_bytes,
                after.allocator.unwrap().allocated_bytes,
                after.allocator.unwrap().reserved_bytes,
                after.ownership.target_bytes.live
            )
            .unwrap();
        }
        r.test_drain("M3 replacement final");
        let drained = r.stats();
        for wave in &waves[1..] {
            assert!((wave.buffer_bytes - waves[0].buffer_bytes).abs() < 512 * 1024);
            assert!((wave.texture_bytes - waves[0].texture_bytes).abs() < 32 * 1024 * 1024);
            assert!(wave.reserved.abs_diff(waves[0].reserved) < 64 * 1024 * 1024);
        }
        assert!(
            texture_overlap
                .iter()
                .chain(&material_overlap)
                .chain(&target_overlap)
                .all(|n| *n >= 2),
            "incomplete prior work required for each replacement type: texture={texture_overlap:?} material={material_overlap:?} target={target_overlap:?}"
        );
        assert_eq!(drained.hal.buffers, setup.hal.buffers);
        assert_eq!(
            drained.ownership.material_bytes.live,
            setup.ownership.material_bytes.live
        );
        assert_eq!(
            drained.ownership.texture_bytes.live,
            setup.ownership.texture_bytes.live
        );
        fs::write(evidence_dir().join(format!("material-stress-{}x.txt",samples as u32)),format!("no_per_frame_waits=true frames=320 warmup=32 waves=3 frames_per_wave=96\ntexture_overlap={texture_overlap:?}\nmaterial_overlap={material_overlap:?}\ntarget_overlap={target_overlap:?}\nwaves={waves:#?}\nsetup={setup:#?}\ndrained={drained:#?}\n")).unwrap();
        eprintln!(
            "material_stress samples={samples:?} texture_overlap={texture_overlap:?} material_overlap={material_overlap:?} target_overlap={target_overlap:?}"
        );
    }
}

#[test]
#[ignore = "Vulkan GPU; alternating descriptive AA pass-cost observations, dev build"]
fn gpu_aa_asset_cost_observations() {
    let _validation = validation::begin(true);
    fs::create_dir_all(evidence_dir()).unwrap();
    let mut log = fs::File::create(evidence_dir().join("aa-cost.csv")).unwrap();
    writeln!(log,"asset,round,samples,frames,gpu_samples,gpu_pass_median_us,gpu_pass_min_us,gpu_pass_max_us,whole_loop_ms,owned_target_bytes,hal_texture_bytes,allocator_bytes,geometry_bytes,texture_bytes").unwrap();
    for name in ["boxes", "curved"] {
        for round in 0..5 {
            for samples in if round % 2 == 0 {
                [SampleCount::One, SampleCount::Four]
            } else {
                [SampleCount::Four, SampleCount::One]
            } {
                let mut r = material_renderer(samples);
                let mut asset = load_asset(name);
                upload_asset(&mut r, &mut asset);
                let color = color_target(&r, 641, 359);
                let cam = asset
                    .camera(0.4, 0.25, Vec3::ZERO, false, 641.0 / 359.0)
                    .unwrap();
                for _ in 0..20 {
                    let report =
                        render(&mut r, &color, &cam, &mut asset.draws(Visibility::Frustum));
                    assert_eq!(report.scene.draws, 2);
                    r.poll().unwrap();
                }
                r.test_drain("AA cost warmup");
                r.poll().unwrap();
                let before = r.stats();
                let start = Instant::now();
                let mut gpu = Vec::new();
                for _ in 0..120 {
                    let report =
                        render(&mut r, &color, &cam, &mut asset.draws(Visibility::Frustum));
                    assert_eq!(report.uploads, UploadBytes::default());
                    assert!(report.scene.triangles > 0);
                    gpu.extend(
                        r.poll()
                            .unwrap()
                            .gpu
                            .into_iter()
                            .map(|g| g.mesh_pass.as_secs_f64() * 1e6),
                    );
                }
                r.test_drain("AA cost final");
                gpu.extend(
                    r.poll()
                        .unwrap()
                        .gpu
                        .into_iter()
                        .map(|g| g.mesh_pass.as_secs_f64() * 1e6),
                );
                let whole = start.elapsed();
                gpu.sort_by(f64::total_cmp);
                assert_eq!(gpu.len(), 120);
                let after = r.stats();
                assert_eq!(after.uploads, before.uploads);
                assert_eq!(after.pipelines, before.pipelines);
                assert_eq!(after.ownership.created(), before.ownership.created());
                assert_eq!(after.timing_failures, 0);
                writeln!(
                    log,
                    "{name},{round},{},120,{},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{}",
                    samples as u32,
                    gpu.len(),
                    gpu[60],
                    gpu[0],
                    gpu[119],
                    whole.as_secs_f64() * 1000.0,
                    after.ownership.target_bytes.live,
                    after.hal.texture_bytes,
                    after.allocator.unwrap().allocated_bytes,
                    after.uploads.geometry,
                    after.uploads.textures
                )
                .unwrap();
            }
        }
    }
    eprintln!(
        "aa_cost_observations rounds=5 warmup=20 samples_per_round=120 extent=641x359 development_build=true timestamps=render_pass_boundaries production_comparison=inconclusive_no_profiler_or_release_build"
    );
}
