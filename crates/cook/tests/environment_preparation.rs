use renderlab_asset::environment::{self as asset, CubeFace, Metadata, RadianceUnits};
use renderlab_cook::{environment::*, environment_fixture as fixture, environment_hdr::*};
use std::f64::consts::{PI, TAU};

fn metadata() -> Metadata {
    Metadata {
        units: RadianceUnits::CandelasPerSquareMetre,
        rotation_y_radians: 0.0,
        source_radiance_scale: 1.0,
        generator_hash: [0; 32],
    }
}
fn source(width: u32, height: u32, f: impl Fn([f64; 3]) -> [f64; 3]) -> LinearEnvironment {
    let pixels = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| f(fixture::direction(x, y, width, height)).map(|v| v as f32))
        .collect();
    LinearEnvironment::new(width, height, pixels, CookLimits::default()).unwrap()
}
fn close(actual: f32, expected: f64, relative: f64, absolute: f64) {
    assert!(
        (actual as f64 - expected).abs() <= absolute + relative * expected.abs(),
        "actual={actual:e} expected={expected:e} relative_error={:e}",
        (actual as f64 - expected).abs() / expected.abs().max(1e-20)
    );
}

#[test]
fn constant_environment_recovers_pi_l_and_constant_radiance_without_clipping() {
    let input = source(16, 8, |_| [1e-20, 7.0, 1e30]);
    let m = Metadata {
        source_radiance_scale: 2.0,
        rotation_y_radians: 1.2,
        ..metadata()
    };
    let p = prepare_environment(
        &input,
        PreparationSettings::default(),
        m,
        CookLimits::default(),
    )
    .unwrap();
    for pixel in p.diffuse_irradiance().texels() {
        for (c, l) in pixel.iter().zip([1e-20, 7.0, 1e30]) {
            close(*c, PI * l * 2.0, 1e-6, 0.0);
        }
    }
    for level in p.specular_radiance() {
        for pixel in level.texels() {
            for (c, l) in pixel.iter().zip([1e-20, 7.0, 1e30]) {
                close(*c, l * 2.0, 1e-6, 0.0);
            }
        }
    }
    assert_eq!(p.metadata().rotation_y_radians, 1.2);
    assert!(
        prepare_environment(
            &source(4, 2, |_| [1e-20; 3]),
            PreparationSettings::default(),
            Metadata {
                source_radiance_scale: f32::from_bits(1),
                ..metadata()
            },
            CookLimits::default()
        )
        .is_err(),
        "positive values underflowing RGB32F must reject explicitly"
    );
    let input = source(4, 2, |_| [f32::MAX as f64; 3]);
    assert!(
        prepare_environment(
            &input,
            PreparationSettings::default(),
            metadata(),
            CookLimits::default()
        )
        .is_err(),
        "unrepresentable irradiance must reject, not clip"
    );
}

#[test]
fn axis_lobe_checks_all_faces_texels_poles_edges_and_irradiance_units() {
    let lobe = |d: [f64; 3]| [2.0 + d[0], 3.0 + 2.0 * d[1], 4.0 + 3.0 * d[2]];
    let input = source(256, 128, lobe);
    let settings = PreparationSettings {
        diffuse_size: 3,
        specular_size: 8,
        diffuse_samples: 4096,
        specular_samples: 4096,
    };
    let p = prepare_environment(&input, settings, metadata(), CookLimits::default()).unwrap();
    for (face_index, face) in CubeFace::ALL.into_iter().enumerate() {
        for level in [p.diffuse_irradiance(), &p.specular_radiance()[0]] {
            let size = level.size();
            for y in 0..size {
                for x in 0..size {
                    let uv = [
                        2.0 * (x as f64 + 0.5) / size as f64 - 1.0,
                        2.0 * (y as f64 + 0.5) / size as f64 - 1.0,
                    ];
                    let n = face.direction(uv);
                    let expected = if size == settings.diffuse_size {
                        [
                            PI * (2.0 + 2.0 * n[0] / 3.0),
                            PI * (3.0 + 4.0 * n[1] / 3.0),
                            PI * (4.0 + 2.0 * n[2]),
                        ]
                    } else {
                        lobe(n)
                    };
                    let rgb = level.face(face)[(y * size + x) as usize];
                    for i in 0..3 {
                        close(rgb[i], expected[i], 0.001, 0.001);
                    }
                }
            }
        }
        // At r=1, GGX D is constant and normalized NoL integration equals irradiance/pi.
        let axes = [
            [1., 0., 0.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
        ];
        let n = axes[face_index];
        let expected = [
            2.0 + 2.0 * n[0] / 3.0,
            3.0 + 4.0 * n[1] / 3.0,
            4.0 + 2.0 * n[2],
        ];
        for (actual, expected) in p.specular_radiance().last().unwrap().face(face)[0]
            .iter()
            .zip(expected)
        {
            close(*actual, expected, 0.002, 0.001);
        }
    }
    // Independent equirectangular rectangular solid angles sum to 4pi, linear lobe integral=constant*4pi.
    let mut energy = [0.0; 3];
    let mut solid_angle = 0.0;
    for y in 0..128 {
        let omega =
            TAU / 256.0 * ((PI * y as f64 / 128.0).cos() - (PI * (y + 1) as f64 / 128.0).cos());
        for x in 0..256 {
            solid_angle += omega;
            for (c, value) in energy
                .iter_mut()
                .zip(input.pixels()[(y * 256 + x) as usize])
            {
                *c += value as f64 * omega;
            }
        }
    }
    assert!((solid_angle - 4.0 * PI).abs() < 1e-11);
    for (actual, expected) in energy.into_iter().zip([8.0 * PI, 12.0 * PI, 16.0 * PI]) {
        assert!((actual - expected).abs() < 1e-6);
    }
}

// Independent f64 oracle integrates light directions over global equal-solid-angle z/phi cells.
// It neither uses Hammersley/GGX half-vector importance samples nor the production image sampler.
fn reference(n: [f64; 3], roughness: Option<f64>, nz: u32, nphi: u32) -> [f64; 3] {
    let mut rgb = [0.0; 3];
    let mut normalization = 0.0;
    let omega = 4.0 * PI / (nz as f64 * nphi as f64);
    for iz in 0..nz {
        let z = 2.0 * (iz as f64 + 0.5) / nz as f64 - 1.0;
        let radial = (1.0 - z * z).sqrt();
        for iphi in 0..nphi {
            let phi = TAU * (iphi as f64 + 0.5) / nphi as f64;
            let l = [radial * phi.cos(), z, radial * phi.sin()];
            let nl = n.iter().zip(l).map(|(a, b)| a * b).sum::<f64>().max(0.0);
            let mut weight = nl * omega;
            if let Some(r) = roughness {
                let alpha = (r * r).max(1e-4);
                let nh2 = (1.0 + nl) / 2.0;
                let denominator = 1.0 + (alpha * alpha - 1.0) * nh2;
                weight *= alpha * alpha / (4.0 * PI * denominator * denominator);
            }
            let value = fixture::sun_sky(l);
            for i in 0..3 {
                rgb[i] += value[i] * weight;
            }
            normalization += weight;
        }
    }
    if roughness.is_some() {
        rgb.map(|v| v / normalization)
    } else {
        rgb
    }
}
fn normalized(d: [f64; 3]) -> [f64; 3] {
    let length = d.iter().map(|v| v * v).sum::<f64>().sqrt();
    d.map(|v| v / length)
}

#[test]
fn high_frequency_sun_sky_matches_independently_converged_angular_reference() {
    let input = source(1024, 512, fixture::sun_sky);
    let settings = PreparationSettings {
        diffuse_size: 2,
        specular_size: 4,
        diffuse_samples: 16384,
        specular_samples: 16384,
    };
    let p = prepare_environment(&input, settings, metadata(), CookLimits::default()).unwrap();
    let preview = prepare_environment(
        &input,
        PreparationSettings {
            diffuse_samples: 1024,
            specular_samples: 1024,
            ..settings
        },
        metadata(),
        CookLimits::default(),
    )
    .unwrap();
    let mut max_refined_relative = 0.0_f64;
    let mut max_preview_relative = 0.0_f64;
    assert!(
        p.specular_radiance()[0]
            .texels()
            .iter()
            .flatten()
            .any(|v| *v > 65504.0)
    );
    // Delta mip must retain the narrow lobe rather than applying a hidden mip-energy scale.
    for face in CubeFace::ALL {
        for y in 0..4 {
            for x in 0..4 {
                let n = face.direction([
                    2.0 * (x as f64 + 0.5) / 4.0 - 1.0,
                    2.0 * (y as f64 + 0.5) / 4.0 - 1.0,
                ]);
                let expected = fixture::sun_sky(n);
                let actual = p.specular_radiance()[0].face(face)[(y * 4 + x) as usize];
                for c in 0..3 {
                    close(actual[c], expected[c], 0.03, 0.005);
                }
            }
        }
    }
    // Known integral of max(S dot L,0)^128 on the sphere = 2pi/129.
    let mut energy = 0.0;
    for y in 0..input.height() {
        let omega = TAU / input.width() as f64
            * ((PI * y as f64 / input.height() as f64).cos()
                - (PI * (y + 1) as f64 / input.height() as f64).cos());
        for x in 0..input.width() {
            energy += input.pixels()[(y * input.width() + x) as usize][0] as f64 * omega;
        }
    }
    assert!((energy / (4.0 * PI * 0.02 + 2e6 * TAU / 129.0) - 1.0).abs() < 0.001);
    for (face, normal) in [
        (CubeFace::PositiveX, normalized([1., 0.5, 0.5])),
        (CubeFace::PositiveY, normalized([-0.5, 1., -0.5])),
        (CubeFace::NegativeZ, normalized([0.5, 0.5, -1.])),
    ] {
        for roughness in [None, Some(0.5)] {
            let low = reference(normal, roughness, 256, 512);
            let high = reference(normal, roughness, 512, 1024);
            let actual = if roughness.is_some() {
                p.specular_radiance()[1].face(face)[0]
            } else {
                p.diffuse_irradiance().face(face)[0]
            };
            let quick = if roughness.is_some() {
                preview.specular_radiance()[1].face(face)[0]
            } else {
                preview.diffuse_irradiance().face(face)[0]
            };
            eprintln!("sun sky {face:?} roughness={roughness:?} actual={actual:?} ref={high:?}");
            for i in 0..3 {
                assert!(
                    (low[i] - high[i]).abs() <= high[i] * 0.002 + 0.01,
                    "reference not converged"
                );
                close(actual[i], high[i], 0.04, 0.0005);
                max_refined_relative =
                    max_refined_relative.max((actual[i] as f64 / high[i] - 1.0).abs());
                max_preview_relative =
                    max_preview_relative.max((quick[i] as f64 / high[i] - 1.0).abs());
            }
        }
    }
    for (face, n) in [
        (CubeFace::PositiveX, [1., 0., 0.]),
        (CubeFace::PositiveY, [0., 1., 0.]),
        (CubeFace::NegativeZ, [0., 0., -1.]),
    ] {
        let low = reference(n, Some(1.0), 256, 512);
        let high = reference(n, Some(1.0), 512, 1024);
        eprintln!(
            "sun sky {face:?} roughness=1 actual={:?} ref={high:?}",
            p.specular_radiance().last().unwrap().face(face)[0]
        );
        for i in 0..3 {
            assert!((low[i] - high[i]).abs() <= high[i] * 0.002 + 0.01);
            close(
                p.specular_radiance().last().unwrap().face(face)[0][i],
                high[i],
                0.04,
                0.0005,
            );
            max_refined_relative = max_refined_relative.max(
                (p.specular_radiance().last().unwrap().face(face)[0][i] as f64 / high[i] - 1.0)
                    .abs(),
            );
            max_preview_relative = max_preview_relative.max(
                (preview.specular_radiance().last().unwrap().face(face)[0][i] as f64 / high[i]
                    - 1.0)
                    .abs(),
            );
        }
    }
    eprintln!(
        "selected 27 channels: max_refined_relative={max_refined_relative:.8}, max_1024_sample_preview_relative={max_preview_relative:.8}; same diffuse2/specular4 layout; preview quality is measured, not accepted by this test"
    );
}

#[test]
fn actual_quantized_hdr_file_decodes_cooks_and_roundtrips_via_production_path() {
    // This session sets the override to D:. Other hosts can run the test without that drive.
    let dir = std::env::var_os("RENDERLAB_ENVIRONMENT_TEST_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("renderlab-environment-{}", std::process::id()))
        });
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("owned-production-path.hdr");
    fixture::write_hdr(std::fs::File::create(&path).unwrap(), 64, 32, |d| {
        fixture::sun_sky(d).map(|v| v as f32)
    })
    .unwrap();
    let limits = CookLimits::default();
    let input = read_rec709_hdr(&path, limits).unwrap();
    for y in 0..32 {
        for x in 0..64 {
            let original = fixture::sun_sky(fixture::direction(x, y, 64, 32)).map(|v| v as f32);
            let encoded = fixture::rgbe(original).unwrap();
            let quantum = 2.0_f64.powi(encoded[3] as i32 - 128 - 8);
            let decoded = input.pixels()[(y * 64 + x) as usize];
            for i in 0..3 {
                assert!((decoded[i] as f64 - original[i] as f64).abs() <= quantum);
                assert_eq!(decoded[i] as f64, encoded[i] as f64 * quantum);
            }
        }
    }
    let settings = PreparationSettings {
        specular_size: 4,
        diffuse_size: 2,
        ..PreparationSettings::default()
    };
    let p = prepare_environment(&input, settings, metadata(), limits).unwrap();
    let bytes = asset::encode(&p, limits.prepared).unwrap();
    assert_eq!(asset::decode(&bytes, limits.prepared).unwrap(), p);
    assert_eq!(
        asset::encode(
            &prepare_environment(&input, settings, metadata(), limits).unwrap(),
            limits.prepared
        )
        .unwrap(),
        bytes
    );
    std::fs::write(dir.join("owned-production-path.rle"), bytes).unwrap();
    assert!(input.pixels().iter().flatten().any(|v| *v > 65504.0));
    assert!(
        input
            .pixels()
            .iter()
            .flatten()
            .any(|v| *v > 0.0 && *v < 0.05)
    );
}

#[test]
fn hdr_boundary_and_cook_policy_fail_before_expensive_work() {
    let mut hdr = Vec::new();
    fixture::write_hdr(&mut hdr, 8, 4, |_| [0.1, 1.0, 1e6]).unwrap();
    let limits = CookLimits::default();
    for cut in [0, 10, 100, hdr.len() - 1] {
        assert!(decode_rec709_hdr(&hdr[..cut], limits).is_err());
    }
    for restricted in [
        CookLimits {
            max_source_file_bytes: 1,
            ..limits
        },
        CookLimits {
            max_source_texels: 1,
            ..limits
        },
        CookLimits {
            max_header_bytes: 20,
            ..limits
        },
        CookLimits {
            max_working_bytes: 32,
            ..limits
        },
    ] {
        assert!(decode_rec709_hdr(&hdr, restricted).is_err());
    }
    let mut extra = hdr.clone();
    extra.push(0);
    assert!(decode_rec709_hdr(&extra, limits).is_err());
    for calibration in [
        "EXPOSURE=2",
        "COLORCORR=1 2 1",
        "PRIMARIES=0.6 0.3 0.3 0.6 0.1 0.1 0.3 0.3",
        "GAMMA=2.2",
    ] {
        let bytes = [
            format!("#?RADIANCE\n{calibration}\n").as_bytes(),
            &hdr[11..],
        ]
        .concat();
        assert!(decode_rec709_hdr(&bytes, limits).is_err());
    }
    let payload_offset = hdr.windows(4).position(|w| w == [2, 2, 0, 8]).unwrap();
    for (at, value) in [
        (payload_offset + 3, 9),
        (payload_offset + 4, 0),
        (payload_offset + 4, 127),
    ] {
        let mut bad = hdr.clone();
        bad[at] = value;
        assert!(decode_rec709_hdr(&bad, limits).is_err());
    }
    // Uncompressed narrow rows are usable; chained old-RLE repeat markers are rejected before
    // image's multiplier can overflow. The file need not actually trigger that dependency panic.
    let mut narrow = Vec::new();
    fixture::write_hdr(&mut narrow, 4, 1, |_| [1.0, 2.0, 3.0]).unwrap();
    assert!(decode_rec709_hdr(&narrow, limits).is_ok());
    let end = narrow.len();
    narrow[end - 4..].copy_from_slice(&[1, 1, 1, 0]);
    assert!(decode_rec709_hdr(&narrow, limits).is_err());
    let input = decode_rec709_hdr(&hdr, limits).unwrap();
    let settings = PreparationSettings::default();
    for restricted in [
        CookLimits {
            max_sample_evaluations: 1,
            ..limits
        },
        CookLimits {
            max_working_bytes: 1,
            ..limits
        },
    ] {
        assert!(prepare_environment(&input, settings, metadata(), restricted).is_err());
    }
    assert!(
        prepare_environment(
            &input,
            PreparationSettings {
                diffuse_samples: 0,
                ..settings
            },
            metadata(),
            limits
        )
        .is_err()
    );
    assert!(LinearEnvironment::new(1, 1, vec![[f32::NAN; 3]], limits).is_err());
    assert!(LinearEnvironment::new(0, 1, vec![], limits).is_err());
    let a = prepare_environment(&input, settings, metadata(), limits).unwrap();
    let b = prepare_environment(
        &input,
        settings,
        Metadata {
            rotation_y_radians: 0.5,
            ..metadata()
        },
        limits,
    )
    .unwrap();
    assert_eq!(a.diffuse_irradiance(), b.diffuse_irradiance());
    assert_eq!(a.specular_radiance(), b.specular_radiance());
    assert_ne!(a.metadata().generator_hash, b.metadata().generator_hash);
}
