use super::*;
use crate::lighting::{DFG, DFG_SIZE, PackedLighting, dirty_span};
use glam::DVec3;

#[test]
fn lighting_validation_and_packing() {
    let mut value = Lighting {
        lights: vec![
            Light::Directional {
                travel: Vec3::new(0., 0., -20.),
                lux: [10.; 3],
            },
            Light::Point {
                position: Vec3::ONE,
                candela: [20.; 3],
                range: Some(2.),
            },
            Light::Spot {
                position: Vec3::Y,
                travel: -Vec3::Y,
                candela: [30.; 3],
                range: None,
                inner: 0.25,
                outer: 0.5,
            },
        ],
        ..Default::default()
    };
    let packed = PackedLighting::new(&value, 3).unwrap();
    assert_eq!(packed.records.len(), 192);
    let word = |index| f32::from_ne_bytes(packed.records[index..index + 4].try_into().unwrap());
    assert_eq!(word(24), -1.);
    assert_eq!(word(64 + 12), 2.);
    assert_eq!(
        u32::from_ne_bytes(packed.records[128 + 48..128 + 52].try_into().unwrap()),
        2
    );
    assert!(dirty_span(&packed.records, &packed.records).is_none());
    value.lights[1] = Light::Point {
        position: Vec3::X,
        candela: [22.; 3],
        range: Some(3.),
    };
    let edit = PackedLighting::new(&value, 3).unwrap();
    let span = dirty_span(&packed.records, &edit.records).unwrap();
    assert!(span.start >= 64 && span.end <= 128);
    assert_eq!(span.start % 4, 0);
    assert_eq!(span.end % 4, 0);
    assert!(PackedLighting::new(&value, 2).is_err());
    for stops in [f32::NAN, f32::INFINITY, 128., -127.] {
        value.exposure_stops = stops;
        assert!(PackedLighting::new(&value, 3).is_err());
    }
    value.exposure_stops = 0.;
    for light in [
        Light::Directional {
            travel: Vec3::ZERO,
            lux: [1.; 3],
        },
        Light::Directional {
            travel: Vec3::X,
            lux: [-1.; 3],
        },
        Light::Point {
            position: Vec3::NAN,
            candela: [1.; 3],
            range: None,
        },
        Light::Point {
            position: Vec3::ZERO,
            candela: [1.; 3],
            range: Some(0.),
        },
        Light::Spot {
            position: Vec3::ZERO,
            travel: Vec3::Z,
            candela: [1.; 3],
            range: None,
            inner: 0.5,
            outer: 0.5,
        },
    ] {
        value.lights = vec![light];
        assert!(PackedLighting::new(&value, 3).is_err());
    }
    value.lights.clear();
    value.environment = [f32::INFINITY; 3];
    assert!(PackedLighting::new(&value, 3).is_err());
}

// Independent midpoint product quadrature over GGX's inverse CDF and azimuth.
// Unlike preparation this has no radical-inverse sequence. Two resolutions check
// convergence independently of finite LUT/interpolation/half precision errors.
pub(super) fn integrate_dfg(nv: f64, r: f64, nu: usize, np: usize) -> [f64; 2] {
    let a = r.powi(2).max(1e-4);
    let a2 = a * a;
    let mut sum = [0.; 2];
    for y in 0..nu {
        let t = (y as f64 + 0.5) / nu as f64;
        let u = 1. - (1. - t).powi(3);
        let jacobian = 3. * (1. - t).powi(2);
        let hz = ((1. - u) / (1. - u + a2 * u)).sqrt();
        let hs = (1. - hz * hz).max(0.).sqrt();
        for x in 0..np {
            let phi = std::f64::consts::TAU * (x as f64 + 0.5) / np as f64;
            let vh = ((1. - nv * nv).sqrt() * hs * phi.cos() + nv * hz).max(0.);
            let nl = (2. * vh * hz - nv).max(0.);
            let lambda_l = nv * (a2 + (1. - a2) * nl * nl).sqrt();
            let lambda_v = nl * (a2 + (1. - a2) * nv * nv).sqrt();
            let weight = 2. * nl * vh / (hz * (lambda_l + lambda_v).max(1e-30)) * jacobian;
            let f = (1. - vh).powi(5);
            sum[0] += weight * (1. - f);
            sum[1] += weight * f;
        }
    }
    sum.map(|v| v / (nu * np) as f64)
}
fn lut(nv: f64, r: f64) -> [f64; 2] {
    let x = nv.clamp(0., 1.) * f64::from(DFG_SIZE - 1);
    let y = r * f64::from(DFG_SIZE - 1);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (tx, ty) = (x - x.floor(), y - y.floor());
    std::array::from_fn(|c| {
        let sample = |i: usize, j: usize| {
            let offset = (j.min(127) * 128 + i.min(127)) * 4 + c * 2;
            f64::from(decode_f16(u16::from_le_bytes(
                DFG[offset..offset + 2].try_into().unwrap(),
            )))
        };
        (sample(x0, y0) * (1. - tx) + sample(x0 + 1, y0) * tx) * (1. - ty)
            + (sample(x0, y0 + 1) * (1. - tx) + sample(x0 + 1, y0 + 1) * tx) * ty
    })
}
#[test]
fn dfg_lut_matches_f64() {
    for (nv, r) in [
        (1., 0.),
        (0.8, 0.1),
        (0.5, 0.5),
        (0.02, 0.1),
        (0.02, 0.03),
        (0.1, 0.9),
        (0.5, 1.),
        (1., 1.),
        (0.001, 0.5),
    ] {
        let coarse = integrate_dfg(nv, r, 512, 1024);
        let fine = integrate_dfg(nv, r, 1024, 2048);
        let stored = lut(nv, r);
        eprintln!("dfg nv={nv} rough={r} coarse={coarse:?} converged={fine:?} lut={stored:?}");
        for c in 0..2 {
            assert!(
                (fine[c] - coarse[c]).abs() < 0.0015,
                "quadrature did not converge"
            );
            assert!(
                (stored[c] - fine[c]).abs() < 0.005,
                "DFG approximation exceeds .005 absolute at {nv}/{r}"
            );
        }
    }
}
fn lit_renderer(samples: SampleCount) -> Renderer {
    let (d, q) = device(true);
    suite_renderer(
        d,
        q,
        FORMAT,
        RendererOptions {
            samples,
            ..Default::default()
        },
        SOURCE,
    )
    .unwrap()
}
fn lit_render(
    r: &mut Renderer,
    t: &wgpu::Texture,
    c: &Camera,
    draws: &mut [Draw<'_>],
) -> FrameReport {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(report) = r.try_render_view(t, c, draws, ViewMode::Lit).unwrap() {
            return report;
        }
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
}
pub(super) fn hdr(r: &mut Renderer) -> Vec<f32> {
    r.test_drain("HDR copy admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("HDR copy");
    let bytes = capture.try_read().unwrap().unwrap();
    crate::renderer::test_record_capture("hdr", &bytes);
    bytes
        .chunks_exact(2)
        .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
        .collect()
}

#[test]
#[ignore = "active Vulkan validation and GPU slot required"]
fn gpu_lit_edits_and_bundle_recovery() {
    let _validation = validation::begin(true);
    let mut r = lit_renderer(SampleCount::Four);
    let t = target(r.device(), 129, 97);
    let c = camera(0.);
    let mesh = upload(&mut r, &triangle([0.18, 0.4, 0.7]), &INDICES);
    let mut inst = instances(&mut r, &[Mat4::IDENTITY]);
    r.test_lighting_busy(&t, &c, &mesh, &mut inst);
    lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
    let expected = r.test_capture_color(&t);
    let expected_hdr = hdr(&mut r);
    let before = r.stats();
    assert!(
        r.set_lighting(&Lighting {
            environment: [f32::NAN; 3],
            ..Default::default()
        })
        .is_err()
    );
    lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
    assert_eq!(expected, r.test_capture_color(&t));
    // Prelude edits, then a standard-program type error.
    let reload = |r: &mut Renderer, (program, source): &(bool, String)| {
        if *program {
            r.test_reload_standard(source)
        } else {
            r.reload(source)
        }
    };
    for bad in [
        (false, "invalid WGSL".to_owned()),
        (
            false,
            SOURCE.replace(
                "@group(0) @binding(1) var exposed_hdr",
                "@group(0) @binding(7) var exposed_hdr",
            ),
        ),
        (
            true,
            crate::STANDARD_PROGRAM.replace(
                "var exposed=exposed_product",
                "var exposed:u32=exposed_product",
            ),
        ),
    ] {
        assert!(matches!(
            reload(&mut r, &bad),
            Err(ReloadError::Rejected { .. })
        ));
        assert_eq!(r.stats().revision, before.revision);
        lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
        assert_eq!(expected, r.test_capture_color(&t));
        assert_eq!(expected_hdr, hdr(&mut r));
        render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
        let diag = r.test_capture_color(&t);
        assert!(reload(&mut r, &bad).is_err());
        render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
        assert_eq!(diag, r.test_capture_color(&t));
    }
    r.reload(SOURCE).unwrap();
    assert_eq!(r.stats().revision.0, before.revision.0 + 1);
    lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
    assert_eq!(expected, r.test_capture_color(&t));
    // Exposure edits do not touch light records, and buffer growth remains charged
    // until previous queue uses have completed. Shrinks retain bounded capacity.
    r.set_lighting(&Lighting {
        lights: vec![Light::Directional {
            travel: -Vec3::Z,
            lux: [30.; 3],
        }],
        exposure_stops: -1.,
        ..Default::default()
    })
    .unwrap();
    let report = lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
    assert_eq!(report.uploads.lights, 0);
    assert_eq!(report.uploads.lighting, 4);
    for count in [8, 32, 2, 64, 1] {
        r.set_lighting(&Lighting {
            lights: vec![
                Light::Point {
                    position: Vec3::Z,
                    candela: [1.; 3],
                    range: None
                };
                count
            ],
            ..Default::default()
        })
        .unwrap();
        lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
    }
    r.test_drain("light generations");
    assert_eq!(r.lighting_stats().retained_light_generations, 0);
    assert_eq!(r.stats().ownership.light_bytes.live, 64 * 64 + 48);
    let generation = r.lighting_stats().target_generation;
    let other = target(r.device(), 145, 113);
    for color in [&other, &t] {
        lit_render(&mut r, color, &c, &mut [Draw::new(&mesh, &mut inst)]);
        assert!(r.lighting_stats().retained_target_generations > 0);
    }
    r.test_drain("HDR target generations");
    assert_eq!(r.lighting_stats().target_generation, generation + 2);
    assert_eq!(r.lighting_stats().retained_target_generations, 0);
    // Invalid normal domain applies even to the default double-sided material.
    inst.set(0, Mat4::from_scale(Vec3::new(0., 1., 1.)))
        .unwrap();
    assert!(matches!(
        r.try_render_view(&t, &c, &mut [Draw::new(&mesh, &mut inst)], ViewMode::Lit),
        Err(Error::InvalidInput(_))
    ));
    render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut inst)]);
    inst.set(0, Mat4::IDENTITY).unwrap();
    // A representable exposed result must survive huge scene units with a tiny
    // exposure. A pre-exposure arithmetic ceiling would incorrectly dim this.
    r.test_drain("large dynamic range material");
    let range_mat = r
        .try_create_material(&MaterialDescriptor {
            metallic: 0.,
            roughness: 1.,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
    r.set_lighting(&Lighting {
        lights: vec![Light::Directional {
            travel: -Vec3::Z,
            lux: [1e38; 3],
        }],
        exposure_stops: -120.,
        ..Default::default()
    })
    .unwrap();
    lit_render(
        &mut r,
        &t,
        &c,
        &mut [Draw::new(&mesh, &mut inst).with_material(&range_mat)],
    );
    let image = hdr(&mut r);
    let expected = reference(
        DVec3::Z,
        DVec3::Z,
        DVec3::Z,
        DVec3::new(0.18, 0.4, 0.7),
        0.,
        1.,
    ) * f64::from(1e38_f32)
        * 2_f64.powi(-120);
    for ch in 0..3 {
        let actual = f64::from(image[(48 * 129 + 64) * 4 + ch]);
        assert!(
            (actual - expected[ch]).abs() < 0.002 + expected[ch] * 0.01,
            "finite exposed dynamic range got {actual} expected {}",
            expected[ch]
        );
    }
    // Smooth GGX peaks and grazing views stay finite; stored clipping is allowed,
    // explicitly excluded from photometry and radiance-parity evidence.
    r.test_drain("extreme material");
    let mat = r
        .try_create_material(&MaterialDescriptor {
            metallic: 0.,
            roughness: 0.,
            double_sided: true,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
    for eye in [Vec3::new(0., 0., 4.), Vec3::new(4., 0., 0.08)] {
        let c = Camera::look_at(eye, Vec3::ZERO, Vec3::Y, 0.8, 0.001, 100.).unwrap();
        r.set_lighting(&Lighting {
            lights: vec![
                Light::Directional {
                    travel: -Vec3::Z,
                    lux: [1e5; 3],
                },
                Light::Point {
                    position: Vec3::new(0., 0., 0.1),
                    candela: [1e5; 3],
                    range: None,
                },
            ],
            environment: [1.; 3],
            ..Default::default()
        })
        .unwrap();
        lit_render(
            &mut r,
            &t,
            &c,
            &mut [Draw::new(&mesh, &mut inst).with_material(&mat)],
        );
        let image = hdr(&mut r);
        assert!(image.iter().all(|v| v.is_finite()));
        assert!(image.chunks_exact(4).any(|p| p[0] > 1.));
        eprintln!(
            "extreme eye={eye:?} stored_resolved_saturated_texels={} not_per_sample_detector",
            image
                .chunks_exact(4)
                .filter(|p| p[..3].iter().any(|v| *v >= 65504.))
                .count()
        );
    }
}
fn reference(n: DVec3, v: DVec3, l: DVec3, base: DVec3, metal: f64, rough: f64) -> DVec3 {
    let nl = n.dot(l).max(0.);
    let nv = n.dot(v).max(1e-7);
    let h = (l + v).normalize();
    let nh = n.dot(h).max(0.);
    let vh = v.dot(h).clamp(0., 1.);
    let a = rough.powi(2).max(1e-4);
    let a2 = a * a;
    let d = a2 / (std::f64::consts::PI * (1. + nh * nh * (a2 - 1.)).powi(2));
    let vis = 0.5
        / (nl * (a2 + (1. - a2) * nv * nv).sqrt() + nv * (a2 + (1. - a2) * nl * nl).sqrt())
            .max(1e-20);
    let f0 = DVec3::splat(0.04).lerp(base, metal);
    let f = f0 + (DVec3::ONE - f0) * (1. - vh).powi(5);
    (base * ((1. - metal) / std::f64::consts::PI) + d * vis * f) * nl
}
#[test]
#[ignore = "active Vulkan validation and GPU slot required"]
fn gpu_lit_photometry_1x_4x() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let mut r = lit_renderer(samples);
        let t = target(r.device(), 128, 96);
        let vertices =
            [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: [1.; 4],
            });
        let mesh = upload(&mut r, &vertices, &[0, 1, 2, 0, 2, 3]);
        let mut instances = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.poll().unwrap();
        };
        let c = camera(0.);
        for metal in [0., 1.] {
            r.test_drain("alpha-floor photometry material");
            let material = r
                .try_create_material(&MaterialDescriptor {
                    base_color: [0.18, 0.4, 0.7, 1.],
                    metallic: metal,
                    roughness: 0.,
                    ..Default::default()
                })
                .unwrap()
                .unwrap();
            r.set_lighting(&Lighting {
                lights: vec![Light::Directional {
                    travel: -Vec3::Z,
                    lux: [1000.; 3],
                }],
                exposure_stops: -8.,
                ..Default::default()
            })
            .unwrap();
            lit_render(
                &mut r,
                &t,
                &c,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            let image = hdr(&mut r);
            for (x, y) in [(64, 48), (63, 48), (65, 48), (64, 47), (64, 49)] {
                let tan = (f64::from(c.fov_y()) / 2.).tan();
                let p = DVec3::new(
                    (2. * (x as f64 + 0.5) / 128. - 1.) * tan * 4. * 128. / 96.,
                    (1. - 2. * (y as f64 + 0.5) / 96.) * tan * 4.,
                    0.,
                );
                let expected = reference(
                    DVec3::Z,
                    (DVec3::new(0., 0., 4.) - p).normalize(),
                    DVec3::Z,
                    DVec3::new(0.18, 0.4, 0.7),
                    f64::from(metal),
                    0.,
                ) * 1000.
                    / 256.;
                for channel in 0..3 {
                    let actual = f64::from(image[(y * 128 + x) * 4 + channel]);
                    assert!(actual.is_finite() && actual < 16376.);
                    assert!(
                        (actual - expected[channel]).abs() <= 0.002 + expected[channel] * 0.01,
                        "alpha-floor {samples:?} m={metal} {x},{y} channel={channel}: {actual} expected {}",
                        expected[channel]
                    );
                }
            }
        }
        for (metal, rough) in [
            (0., 0.1),
            (0., 0.5),
            (0., 1.),
            (1., 0.1),
            (1., 0.5),
            (1., 1.),
        ] {
            r.test_drain("material");
            let material = r
                .try_create_material(&MaterialDescriptor {
                    base_color: [0.18, 0.4, 0.7, 1.],
                    metallic: metal,
                    roughness: rough,
                    ..Default::default()
                })
                .unwrap()
                .unwrap();
            let lights = [
                Light::Directional {
                    travel: Vec3::new(-0.25, 0., -1.),
                    lux: [1000.; 3],
                },
                Light::Point {
                    position: Vec3::Z,
                    candela: [100.; 3],
                    range: None,
                },
                Light::Point {
                    position: Vec3::Z * 2.,
                    candela: [100.; 3],
                    range: None,
                },
                Light::Point {
                    position: Vec3::Z,
                    candela: [100.; 3],
                    range: Some(0.9),
                },
                Light::Spot {
                    position: Vec3::Z * 2.,
                    travel: -Vec3::Z,
                    candela: [100.; 3],
                    range: Some(4.),
                    inner: 0.25,
                    outer: 0.5,
                },
            ];
            for light in lights {
                r.set_lighting(&Lighting {
                    lights: vec![light],
                    exposure_stops: -8.,
                    ..Default::default()
                })
                .unwrap();
                lit_render(
                    &mut r,
                    &t,
                    &c,
                    &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
                );
                let image = hdr(&mut r);
                let display = r.test_capture_color(&t);
                for y in (18..78).step_by(11) {
                    for x in (18..110).step_by(13) {
                        let f = f64::from(c.fov_y());
                        let px =
                            (2. * (x as f64 + 0.5) / 128. - 1.) * (f / 2.).tan() * 4. * 128. / 96.;
                        let py = (1. - 2. * (y as f64 + 0.5) / 96.) * (f / 2.).tan() * 4.;
                        let p = DVec3::new(px, py, 0.);
                        let v = (DVec3::new(0., 0., 4.) - p).normalize();
                        let (l, energy) = match light {
                            Light::Directional { travel, lux } => (
                                -travel.as_dvec3().normalize(),
                                DVec3::from_array(lux.map(f64::from)),
                            ),
                            Light::Point {
                                position,
                                candela,
                                range,
                            }
                            | Light::Spot {
                                position,
                                candela,
                                range,
                                ..
                            } => {
                                let delta = position.as_dvec3() - p;
                                let distance = delta.length();
                                let l = delta / distance;
                                let mut a = 1. / distance.powi(2).max(1e-4);
                                if let Some(range) = range {
                                    a *= (1. - (distance / f64::from(range)).powi(4))
                                        .clamp(0., 1.)
                                        .powi(2);
                                }
                                if let Light::Spot {
                                    travel,
                                    inner,
                                    outer,
                                    ..
                                } = light
                                {
                                    a *= ((-l.dot(travel.as_dvec3().normalize())
                                        - f64::from(outer).cos())
                                        / (f64::from(inner).cos() - f64::from(outer).cos())
                                            .max(1e-4))
                                    .clamp(0., 1.)
                                    .powi(2);
                                }
                                (l, DVec3::from_array(candela.map(f64::from)) * a)
                            }
                        };
                        let expected = reference(
                            DVec3::Z,
                            v,
                            l,
                            DVec3::new(0.18, 0.4, 0.7),
                            f64::from(metal),
                            f64::from(rough),
                        ) * energy
                            / 256.;
                        let idx = (y * 128 + x) * 4;
                        for channel in 0..3 {
                            let actual = f64::from(image[idx + channel]);
                            assert!(actual.is_finite() && actual < 65504.);
                            assert!(
                                (actual - expected[channel]).abs()
                                    <= 0.002 + expected[channel] * 0.01,
                                "{samples:?} {light:?} m={metal} r={rough} {x},{y} channel={channel} got {actual} expected {}",
                                expected[channel]
                            );
                            let mapped = actual / (1. + actual);
                            assert!(
                                (f64::from(display[idx + channel]) / 255. - mapped).abs()
                                    < 1. / 255.
                            );
                        }
                    }
                }
            }
            // Environment reference is independently integrated, not sampled from this LUT.
            r.set_lighting(&Lighting {
                environment: [0.3; 3],
                ..Default::default()
            })
            .unwrap();
            lit_render(
                &mut r,
                &t,
                &c,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            let image = hdr(&mut r);
            let x = 64;
            let y = 48;
            let px = (2. * 64.5 / 128. - 1.) * (f64::from(c.fov_y()) / 2.).tan() * 4. * 128. / 96.;
            let py = (1. - 2. * 48.5 / 96.) * (f64::from(c.fov_y()) / 2.).tan() * 4.;
            let nv = 4. / (16. + px * px + py * py).sqrt();
            let ab = integrate_dfg(nv, f64::from(rough), 512, 1024);
            for ch in 0..3 {
                let base = [0.18, 0.4, 0.7][ch];
                let expected = 0.3
                    * (base * (1. - f64::from(metal))
                        + (0.04 * (1. - f64::from(metal)) + base * f64::from(metal)) * ab[0]
                        + ab[1]);
                assert!(
                    (f64::from(image[(y * 128 + x) * 4 + ch]) - expected).abs()
                        < 0.002 + 0.01 * expected
                );
            }
        }
        // Tilt, nonuniform mirrored transforms and source-back double-sided shading.
        // Intersect the actual pixel ray with the transformed plane in f64.
        r.test_drain("normal photometry material");
        let material = r
            .try_create_material(&MaterialDescriptor {
                base_color: [0.3; 4],
                metallic: 0.,
                roughness: 0.5,
                double_sided: true,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        for (model, eye) in [
            (
                Mat4::from_rotation_y(0.4) * Mat4::from_scale(Vec3::new(1.2, 0.7, 2.)),
                Vec3::new(0., 0., 4.),
            ),
            (
                Mat4::from_rotation_y(0.4) * Mat4::from_scale(Vec3::new(-1.2, 0.7, 2.)),
                Vec3::new(0., 0., 4.),
            ),
            (Mat4::IDENTITY, Vec3::new(0., 0., -4.)),
        ] {
            instances.set(0, model).unwrap();
            let cam = Camera::look_at(eye, Vec3::ZERO, Vec3::Y, 0.8, 0.1, 100.).unwrap();
            let mut n = model
                .inverse()
                .transpose()
                .transform_vector3(Vec3::Z)
                .as_dvec3()
                .normalize();
            if eye.z < 0. {
                n = -n;
            }
            let travel = -n.as_vec3();
            r.set_lighting(&Lighting {
                lights: vec![Light::Directional {
                    travel,
                    lux: [1000.; 3],
                }],
                exposure_stops: -8.,
                ..Default::default()
            })
            .unwrap();
            lit_render(
                &mut r,
                &t,
                &cam,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            let image = hdr(&mut r);
            let near = cam
                .matrix(128, 96)
                .inverse()
                .as_dmat4()
                .project_point3(DVec3::new(2. * 64.5 / 128. - 1., 1. - 2. * 48.5 / 96., 1.));
            let ray = (near - eye.as_dvec3()).normalize();
            let p = eye.as_dvec3() + ray * (-n.dot(eye.as_dvec3()) / n.dot(ray));
            let v = (eye.as_dvec3() - p).normalize();
            let expected = reference(n, v, n, DVec3::splat(0.3), 0., 0.5) * 1000. / 256.;
            for ch in 0..3 {
                assert!(
                    (f64::from(image[(48 * 128 + 64) * 4 + ch]) - expected[ch]).abs()
                        < 0.002 + expected[ch] * 0.01,
                    "normal-domain photometry {model:?} {eye:?}"
                );
            }
        }
        instances.set(0, Mat4::IDENTITY).unwrap();
        // Explicit per-sample resolve oracle; no raster coverage assumptions. Programs
        // cannot read the sample index, so this edits the generated entry itself.
        if samples == SampleCount::Four {
            let composed = crate::program::compose(
                SOURCE,
                crate::STANDARD_PROGRAM,
                crate::program::Stage::Raster,
                false,
            )
            .source;
            let probe = [
                (
                    "@fragment fn fs_lit(input:VertexOutput, @builtin(front_facing) front:bool)",
                    "@fragment fn fs_lit(input:VertexOutput, @builtin(front_facing) front:bool, @builtin(sample_index) sample:u32)",
                ),
                (
                    "return vec4<f32>(rl_finite(shade(surface_output,surface_input)),1.0);",
                    "return vec4<f32>(vec3<f32>(select(0.0,16.0,sample==3u)),1.0);",
                ),
            ]
            .iter()
            .fold(composed, |source, (find, replace)| {
                assert_eq!(source.matches(find).count(), 1);
                source.replace(find, replace)
            });
            let mut probe = r.test_lit_pair(&probe, false);
            std::mem::swap(&mut r.test_standard().lit, &mut probe);
            lit_render(&mut r, &t, &c, &mut [Draw::new(&mesh, &mut instances)]);
            let image = hdr(&mut r);
            assert_eq!(image[(48 * 128 + 64) * 4], 4.);
            assert_eq!(r.test_capture_color(&t)[(48 * 128 + 64) * 4], 204);
            std::mem::swap(&mut r.test_standard().lit, &mut probe);
        }
    }
}
