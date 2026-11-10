use super::*;
use crate::asset::environment::{
    self as env, CubeFace, CubeLevel, Metadata, PreparedEnvironment, RadianceUnits,
};

// M7-ES1: dominant environment lobes and occluded environment specular.
#[path = "environment_specular_tests.rs"]
mod specular;

fn level(size: u32, value: impl Fn([f64; 3]) -> [f32; 3]) -> CubeLevel {
    let mut pixels = vec![];
    for face in CubeFace::ALL {
        for y in 0..size {
            for x in 0..size {
                pixels.push(value(face.direction([
                    2. * (x as f64 + 0.5) / size as f64 - 1.,
                    2. * (y as f64 + 0.5) / size as f64 - 1.,
                ])));
            }
        }
    }
    CubeLevel::new(size, pixels, env::Limits::default()).unwrap()
}
fn prepared(diffuse: CubeLevel, specular: Vec<CubeLevel>, yaw: f32) -> PreparedEnvironment {
    PreparedEnvironment::new(
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            source_radiance_scale: 123., // Provenance must never become an unpack multiplier.
            rotation_y_radians: yaw,
            generator_hash: [7; 32],
        },
        diffuse,
        specular,
        env::Limits::default(),
    )
    .unwrap()
}
fn constant(rgb: [f32; 3]) -> PreparedEnvironment {
    prepared(
        level(2, |_| rgb.map(|v| v * std::f32::consts::PI)),
        vec![level(4, |_| rgb), level(2, |_| rgb), level(1, |_| rgb)],
        0.,
    )
}

#[test]
fn environment_upload_packing_limits_and_parameters() {
    let input = constant([1e-20, 7., 1e30]);
    let packed = crate::environment::pack_level(input.specular_radiance().first().unwrap());
    let first = packed[..16]
        .chunks_exact(4)
        .map(|x| f32::from_ne_bytes(x.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(first, [1e-20, 7., 1e30, 1.]);
    let bytes = input.texel_count() * 16;
    assert_eq!(
        crate::environment::upload_bytes(&input, EnvironmentLimits::default(), 4).unwrap(),
        bytes
    );
    assert!(crate::environment::upload_bytes(&input, EnvironmentLimits::default(), 2).is_err());
    assert!(
        crate::environment::upload_bytes(
            &input,
            EnvironmentLimits {
                max_map_bytes: bytes - 1,
                ..Default::default()
            },
            4
        )
        .is_err()
    );
    for p in [
        EnvironmentParameters {
            rotation_y_radians: f32::NAN,
            intensity: 1.,
        },
        EnvironmentParameters {
            rotation_y_radians: 0.,
            intensity: -1.,
        },
        EnvironmentParameters {
            rotation_y_radians: 0.,
            intensity: f32::INFINITY,
        },
    ] {
        assert!(p.validate().is_err());
    }
    assert!(
        EnvironmentParameters {
            rotation_y_radians: f32::MAX,
            intensity: f32::MAX
        }
        .validate()
        .is_ok()
    );
}

fn environment_renderer(samples: SampleCount) -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!(
        "environment_adapter={:?} float32_filterable={}",
        adapter.get_info(),
        adapter
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    );
    let (d, q) = request(
        &adapter,
        adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
    );
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
fn upload_environment(r: &mut Renderer, data: &PreparedEnvironment) -> EnvironmentMap {
    r.test_drain("environment upload admission");
    r.try_upload_environment(data).unwrap().unwrap()
}
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

#[test]
#[ignore = "exclusive Vulkan slot; real filtered f32 cubes and independent BRDF integration"]
fn gpu_environment_constant_photometry_and_compatibility() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let mut r = environment_renderer(samples);
        let color = target(r.device(), 65, 65);
        let map = upload_environment(&mut r, &constant([2., 3., 4.]));
        let mesh = upload(&mut r, &triangle([1.; 3]), &INDICES);
        r.test_drain("environment instances");
        let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        for (metal, rough, eye) in [
            (0., 0.5, Vec3::new(0., 0., 4.)),
            (1., 0.1, Vec3::new(0., 0., 4.)),
            (1., 1., Vec3::new(0., 0., 4.)),
            (1., 0.5, Vec3::new(4., 0., 0.2)),
        ] {
            let base = [0.7, 0.4, 0.2];
            r.test_drain("environment material");
            let material = r
                .try_create_material(&MaterialDescriptor {
                    base_color: [base[0], base[1], base[2], 1.],
                    metallic: metal,
                    roughness: rough,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap()
                .unwrap();
            let camera = Camera::look_at(eye, Vec3::ZERO, Vec3::Y, 0.8, 0.1, 100.).unwrap();
            r.set_environment(None, Default::default()).unwrap();
            r.set_lighting(&Lighting {
                environment: [2., 3., 4.],
                ..Default::default()
            })
            .unwrap();
            lit(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            let expected = lighting_tests::hdr(&mut r);
            r.set_environment(Some(&map), Default::default()).unwrap();
            let report = lit(
                &mut r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
            );
            assert_eq!(report.uploads.environment, 0);
            let actual = lighting_tests::hdr(&mut r);
            let center = (32 * 65 + 32) * 4;
            // Constant background stays the declared constant fallback in both modes.
            for (a, b) in actual.iter().zip(&expected) {
                assert!((a - b).abs() <= 0.004 + b.abs() * 0.002);
            }
            let nv = eye.normalize().z as f64;
            let dfg = lighting_tests::integrate_dfg(nv, rough as f64, 512, 1024);
            for c in 0..3 {
                let f0 = 0.04 * (1. - metal as f64) + base[c] as f64 * metal as f64;
                let reference =
                    [2., 3., 4.][c] * (base[c] as f64 * (1. - metal as f64) + f0 * dfg[0] + dfg[1]);
                let error = (actual[center + c] as f64 - reference).abs();
                assert!(
                    error < 0.012 + reference * 0.015,
                    "BRDF metal={metal} rough={rough} nv={nv} channel={c} actual={} reference={reference}",
                    actual[center + c]
                );
            }
        }
        let dim_map = upload_environment(&mut r, &constant([1e-20; 3]));
        r.set_environment(
            Some(&dim_map),
            EnvironmentParameters {
                intensity: 1e20,
                ..Default::default()
            },
        )
        .unwrap();
        r.set_lighting(&Lighting::default()).unwrap();
        r.test_drain("dim cube material");
        let material = r
            .try_create_material(&MaterialDescriptor {
                metallic: 1.,
                roughness: 0.,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        lit(
            &mut r,
            &color,
            &camera(0.),
            &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
        );
        let image = lighting_tests::hdr(&mut r);
        assert!(
            (image[(32 * 65 + 32) * 4] - 1.).abs() < 0.005,
            "dim cube lost before intensity"
        );
        // Extreme physical energy and dim response remain finite after exposure/intensity cancellation.
        let map = upload_environment(&mut r, &constant([1e30; 3]));
        r.set_environment(
            Some(&map),
            EnvironmentParameters {
                intensity: 1e20,
                ..Default::default()
            },
        )
        .unwrap();
        r.set_lighting(&Lighting {
            exposure_stops: -100.,
            ..Default::default()
        })
        .unwrap();
        r.test_drain("dim material");
        let material = r
            .try_create_material(&MaterialDescriptor {
                base_color: [1e-20, 1e-20, 1e-20, 1.],
                metallic: 1.,
                roughness: 0.,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        lit(
            &mut r,
            &color,
            &camera(0.),
            &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
        );
        let actual = lighting_tests::hdr(&mut r);
        let center = (32 * 65 + 32) * 4;
        assert!(actual.iter().all(|v| v.is_finite()));
        assert!(
            (actual[center] - 0.78886).abs() < 0.01,
            "range cancellation {}",
            actual[center]
        );
    }
}

#[derive(Clone, Copy)]
struct Probe {
    direction: [f64; 3],
    lod: f64,
    diffuse: bool,
}
fn probe_source(probes: &[Probe]) -> String {
    let records = probes
        .iter()
        .map(|p| {
            format!(
                "vec4<f32>({:.9},{:.9},{:.9},{:.9})",
                p.direction[0],
                p.direction[1],
                p.direction[2],
                if p.diffuse { -1. } else { p.lod }
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    // A program: each pixel column evaluates one probe through the engine environment.
    format!(
        "struct Surface {{ column:u32 }};\nfn surface(input:SurfaceInput)->Surface {{ return Surface(input.pixel.x); }}\nfn shade(s:Surface,input:SurfaceInput)->vec3<f32> {{\n let probes=array<vec4<f32>,{}>({records});\n let probe=probes[min(s.column,{}u)];\n let d=environment_direction(probe.xyz);\n var rgb=vec3<f32>(0.0);\n if lighting.map.w<0.0 {{ rgb=environment_radiance(d,max(probe.w,0.0)); }} else {{ rgb=environment_legacy_lod(d,max(probe.w,0.0)); }}\n if probe.w<0.0 {{ rgb=environment_diffuse_at(d); }}\n return environment_product(rgb,vec3<f32>(1.0));\n}}\n",
        probes.len(),
        probes.len() - 1,
    )
}
fn run_probes(r: &mut Renderer, probes: &[Probe]) -> Vec<[f32; 3]> {
    r.test_reload_standard(&probe_source(probes)).unwrap();
    let color = target(r.device(), probes.len() as u32, 1);
    // The camera's horizontal frustum grows with this deliberately one-row target's aspect ratio.
    let span = (probes.len() as f32 * 4.0).max(1000.0);
    let vertices = [
        [-span, -1000., 0.],
        [span, -1000., 0.],
        [span, 1000., 0.],
        [-span, 1000., 0.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let mesh = upload(r, &vertices, &[0, 1, 2, 0, 2, 3]);
    r.test_drain("probe instances");
    let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    lit(
        r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    lighting_tests::hdr(r)
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

#[test]
#[ignore = "exclusive Vulkan slot; filtered RLE2 bank split, r^4 interpolation and replacement"]
fn gpu_environment_independent_banks_filtered() {
    let _validation = validation::begin(true);
    let mut r = environment_renderer(SampleCount::One);
    assert!(
        r.device()
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    );
    let knots = env::roughness_knots();
    let data = PreparedEnvironment::new_independent(
        constant([1.; 3]).metadata().clone(),
        level(2, |_| [1.; 3]),
        knots
            .iter()
            .map(|k| level(2, |_| [1.0 + k.powi(4), 2.0, 1e-20]))
            .collect(),
        knots,
        env::FieldGenerator::F64HostF32GpuV1,
        env::Limits::default(),
    )
    .unwrap();
    let map = upload_environment(&mut r, &data);
    r.set_environment(Some(&map), Default::default()).unwrap();
    let probes: Vec<_> = [0., 0.009, 0.011, 0.137, 0.42, 0.43, 0.573, 0.941, 1.]
        .into_iter()
        .flat_map(|rough| {
            [[1., 0., 0.], [1., 1., 1.], [-1., -1., -1.], [0., 1., 0.]]
                .into_iter()
                .map(move |direction| Probe {
                    direction,
                    lod: rough,
                    diffuse: false,
                })
        })
        .collect();
    let values = run_probes(&mut r, &probes);
    for (p, value) in probes.iter().zip(values) {
        assert!(
            (f64::from(value[0]) - (1.0 + p.lod.powi(4))).abs() < 0.0011,
            "r={} value={value:?}",
            p.lod
        );
        assert_eq!(value[1], 2.0);
    }
    r.test_reload_standard(crate::STANDARD_PROGRAM).unwrap();
    r.set_environment(None, Default::default()).unwrap();
    drop(map);
    r.test_drain("RLE2 retirement");
    let settled = r.stats().ownership.environment_bytes.live;
    for _ in 0..3 {
        let map = upload_environment(&mut r, &data);
        r.set_environment(Some(&map), Default::default()).unwrap();
        r.set_environment(None, Default::default()).unwrap();
        drop(map);
        r.test_drain("RLE2 replacement");
        assert_eq!(r.stats().ownership.environment_bytes.live, settled);
    }
}

#[test]
#[ignore = "exclusive Vulkan slot; complete RLE2 artifact against 538 fixed independent reconstruction references"]
fn gpu_environment_independent_registered_runtime_reconstruction() {
    let _validation = validation::begin(true);
    let path = std::env::var_os("RENDERLAB_ENVIRONMENT_FIELD").expect("RLE2 field path");
    let references = std::env::var_os("RENDERLAB_ENVIRONMENT_FIELD_REFERENCES")
        .expect("accepted a-screen.json path");
    let data = env::decode(&fs::read(path).unwrap(), env::Limits::default()).unwrap();
    assert!(matches!(
        data.specular_layout(),
        env::SpecularLayout::IndependentRoughness { .. }
    ));
    assert_eq!(data.specular_radiance()[0].size(), 128);
    let references: serde_json::Value =
        serde_json::from_slice(&fs::read(references).unwrap()).unwrap();
    let mut unique = std::collections::BTreeMap::new();
    for row in references["rows"].as_array().unwrap() {
        let d: [f64; 3] = std::array::from_fn(|c| row["direction"][c].as_f64().unwrap());
        let rough = row["roughness"].as_f64().unwrap();
        unique.insert(
            [
                d[0].to_bits(),
                d[1].to_bits(),
                d[2].to_bits(),
                rough.to_bits(),
            ],
            (row, d, rough),
        );
    }
    assert_eq!(unique.len(), 538);
    let probes: Vec<_> = unique
        .values()
        .map(|(_, direction, r)| Probe {
            direction: *direction,
            lod: *r,
            diffuse: false,
        })
        .collect();
    let mut r = environment_renderer(SampleCount::One);
    let map = upload_environment(&mut r, &data);
    r.set_environment(Some(&map), Default::default()).unwrap();
    r.set_lighting(&Lighting {
        exposure_stops: -8.,
        ..Default::default()
    })
    .unwrap();
    let values = run_probes(&mut r, &probes);
    let mut maximum = 0.0_f64;
    let mut output = vec![];
    for (((row, _, rough), value), probe) in unique.values().zip(values).zip(&probes) {
        let actual = value.map(|v| f64::from(v) * 256.0);
        for c in 0..3 {
            let reference = row["reference_rgb"][c].as_f64().unwrap();
            let ratio = (actual[c] - reference).abs() / (0.04 * reference + 0.001);
            maximum = maximum.max(ratio);
            assert!(
                ratio <= 1.0,
                "runtime r={rough} d={:?} actual={actual:?} reference={} c={c} allowance={ratio}",
                probe.direction,
                row["reference_rgb"]
            );
        }
        output.push(serde_json::json!({"case":row["case"],"r":rough,"direction":probe.direction,"actual":actual,"reference":row["reference_rgb"]}));
    }
    let sun = glam::DVec3::new(0.36, 0.48, 0.8);
    let tangent = sun.cross(glam::DVec3::Y).normalize();
    let bitangent = sun.cross(tangent);
    let delta: Vec<_> = (-8..=8)
        .flat_map(|y| {
            (-8..=8).map(move |x| Probe {
                direction: (sun
                    + tangent * (f64::from(x) * 0.035)
                    + bitangent * (f64::from(y) * 0.035))
                    .normalize()
                    .to_array(),
                lod: 0.,
                diffuse: false,
            })
        })
        .collect();
    let values = run_probes(&mut r, &delta);
    let mut peak_fraction = 0.0_f64;
    let mut peak = (f32::NEG_INFINITY, glam::DVec3::ZERO);
    for (p, value) in delta.iter().zip(values) {
        let reference = sun_sky(glam::DVec3::from_array(p.direction));
        for c in 0..3 {
            peak_fraction = peak_fraction
                .max((f64::from(value[c]) * 256.0 - reference[c]).abs() / [2e6, 1.6e6, 1e6][c]);
        }
        if value[0] > peak.0 {
            peak = (value[0], glam::DVec3::from_array(p.direction));
        }
    }
    let peak_angle = peak.1.dot(sun).clamp(-1.0, 1.0).acos().to_degrees();
    assert!(
        peak_angle <= 1.0,
        "mip-zero sun peak direction {peak_angle} degrees"
    );
    assert!(
        peak_fraction <= 0.03,
        "mip-zero sun peak-relative error {peak_fraction}"
    );
    fs::create_dir_all(evidence_dir()).unwrap();
    fs::write(evidence_dir().join("rle2-runtime-reconstruction.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "queries":538,"max_allowance_ratio":maximum,"delta_queries":289,"mip0_peak_fraction":peak_fraction,"mip0_peak_angle_degrees":peak_angle,
        "float32_filter_feature":true,"lookup":"actual renderer WGSL with r^4 and filtered cube-array samples; exposed f16 readback", "rows":output
    })).unwrap()).unwrap();
    eprintln!(
        "RLE2 actual runtime 538 queries max allowance={maximum}; delta 289 peak fraction={peak_fraction}"
    );
}

#[test]
#[ignore = "exclusive Vulkan slot; axes/poles/12 edges and trilinear mip boundary probes"]
fn gpu_environment_axes_yaw_poles_seams_and_mips() {
    let _validation = validation::begin(true);
    let mut r = environment_renderer(SampleCount::One);
    let analytic =
        |d: [f64; 3], m: f64| std::array::from_fn(|c| (0.5 + 0.15 * d[c] + m * 0.1) as f32);
    let data = prepared(
        level(32, |d| analytic(d, 0.)),
        (0..6)
            .map(|i| level(32 >> i, |d| analytic(d, i as f64)))
            .collect(),
        0.37,
    );
    let map = upload_environment(&mut r, &data);
    r.set_environment(
        Some(&map),
        EnvironmentParameters {
            rotation_y_radians: 1.2,
            ..Default::default()
        },
    )
    .unwrap();
    let mut directions = vec![
        [1., 0., 0.],
        [-1., 0., 0.],
        [0., 1., 0.],
        [0., -1., 0.],
        [0., 0., 1.],
        [0., 0., -1.],
    ];
    for zero in 0..3 {
        for a in [-1., 1.] {
            for b in [-1., 1.] {
                for side in [-1e-5, 1e-5] {
                    let mut d = [0.; 3];
                    d[(zero + 1) % 3] = a * (1. + side);
                    d[(zero + 2) % 3] = b;
                    directions.push(d);
                }
            }
        }
    }
    let mut probes = vec![];
    for lod in [0., 0.999, 1., 1.001, 2.5, 4., 5.] {
        for &direction in &directions {
            probes.push(Probe {
                direction,
                lod,
                diffuse: false,
            });
        }
    }
    probes.extend(directions.iter().map(|&direction| Probe {
        direction,
        lod: 0.,
        diffuse: true,
    }));
    let values = run_probes(&mut r, &probes);
    let (sin, cos) = (0.37_f64 + 1.2).sin_cos();
    for (p, value) in probes.iter().zip(values) {
        let d = glam::DVec3::from_array(p.direction).normalize();
        let source = [cos * d.x - sin * d.z, d.y, sin * d.x + cos * d.z];
        let lod = if p.diffuse { 0. } else { p.lod };
        let lower = lod.floor();
        let t = lod - lower;
        // Lipschitz directional field. Pixel angular footprint bounds the coarse-level deviation.
        let error_bound = 0.002
            + 0.23
                * ((1. - t) / f64::from(32u32 >> lower as u32)
                    + t / f64::from(32u32 >> (lower as u32 + 1).min(5)));
        for c in 0..3 {
            let reference = 0.5 + 0.15 * source[c] + lod * 0.1;
            assert!(
                (value[c] as f64 - reference).abs() < error_bound,
                "probe {:?} lod={lod} channel={c} actual={} reference={reference} bound={error_bound}",
                p.direction,
                value[c]
            );
        }
    }
    // Observe actual seam continuity on each source-space edge, including both poles' adjacent edges.
    r.set_environment(
        Some(&map),
        EnvironmentParameters {
            rotation_y_radians: -0.37,
            ..Default::default()
        },
    )
    .unwrap();
    let seam_probes = (0..6)
        .flat_map(|lod| {
            directions[6..].iter().map(move |&direction| Probe {
                direction,
                lod: lod as f64,
                diffuse: false,
            })
        })
        .collect::<Vec<_>>();
    let values = run_probes(&mut r, &seam_probes);
    for pair in values.chunks_exact(2) {
        for c in 0..3 {
            assert!(
                (pair[0][c] - pair[1][c]).abs() < 0.002,
                "cube seam {:?}",
                pair
            );
        }
    }
}

fn sun_sky(direction: glam::DVec3) -> [f64; 3] {
    let power = 2e6
        * direction
            .dot(glam::DVec3::new(0.36, 0.48, 0.8))
            .max(0.)
            .powi(128);
    [0.02 + power, 0.04 + power * 0.8, 0.1 + power * 0.5]
}

// Product quadrature over global light directions, independent of the cooker's half-vector samples.
fn sun_reference(n: glam::DVec3, roughness: Option<f64>, resolution: u32) -> [f64; 3] {
    let mut sum = [0.; 3];
    let mut norm = 0.;
    let omega = 4. * std::f64::consts::PI / f64::from(resolution * resolution * 2);
    for y in 0..resolution {
        let z = 2. * (f64::from(y) + 0.5) / f64::from(resolution) - 1.;
        let radius = (1. - z * z).sqrt();
        for x in 0..resolution * 2 {
            let phi = std::f64::consts::TAU * (f64::from(x) + 0.5) / f64::from(resolution * 2);
            let l = glam::DVec3::new(radius * phi.cos(), z, radius * phi.sin());
            let nl = n.dot(l).max(0.);
            let mut weight = nl * omega;
            if let Some(r) = roughness {
                let a2 = r.powi(4).max(1e-8);
                let denom = 1. + (a2 - 1.) * (1. + nl) / 2.;
                weight *= a2 / (4. * std::f64::consts::PI * denom * denom);
            }
            for (dst, value) in sum.iter_mut().zip(sun_sky(l)) {
                *dst += weight * value;
            }
            norm += weight;
        }
    }
    if roughness.is_some() {
        sum.map(|v| v / norm)
    } else {
        sum
    }
}

#[test]
#[ignore = "exclusive Vulkan slot; actual refined HDR, explicit spatial angular-error acceptance"]
fn gpu_environment_refined_sun_angular_and_convolution_reference() {
    let _validation = validation::begin(true);
    let path = std::env::var_os("RENDERLAB_ENVIRONMENT_QUALITY")
        .map(std::path::PathBuf::from)
        .expect(
            "set RENDERLAB_ENVIRONMENT_QUALITY to the spatially refined environment_quality RLE1",
        );
    let data = env::decode(&fs::read(&path).unwrap(), env::Limits::default()).unwrap();
    let mut r = environment_renderer(SampleCount::One);
    let map = upload_environment(&mut r, &data);
    r.set_environment(Some(&map), Default::default()).unwrap();
    r.set_lighting(&Lighting {
        exposure_stops: -16.,
        ..Default::default()
    })
    .unwrap();
    let sun = glam::DVec3::new(0.36, 0.48, 0.8);
    let tangent = sun.cross(glam::DVec3::Y).normalize();
    let bitangent = sun.cross(tangent);
    let probes = (-8..=8)
        .flat_map(|y| {
            (-8..=8).map(move |x| Probe {
                direction: (sun + tangent * (x as f64 * 0.035) + bitangent * (y as f64 * 0.035))
                    .normalize()
                    .to_array(),
                lod: 0.,
                diffuse: false,
            })
        })
        .collect::<Vec<_>>();
    let values = run_probes(&mut r, &probes);
    let mut max_peak_fraction = 0_f64;
    let mut peak = (f64::NEG_INFINITY, glam::DVec3::ZERO);
    for (p, value) in probes.iter().zip(&values) {
        let d = glam::DVec3::from_array(p.direction);
        let reference = sun_sky(d);
        for c in 0..3 {
            max_peak_fraction = max_peak_fraction
                .max((value[c] as f64 * 65536. - reference[c]).abs() / [2e6, 1.6e6, 1e6][c]);
        }
        if value[0] as f64 > peak.0 {
            peak = (value[0] as f64, d);
        }
    }
    let angular_error = peak.1.dot(sun).clamp(-1., 1.).acos().to_degrees();
    let report = serde_json::json!({"path":path,"specular_size":data.specular_radiance()[0].size(),"peak_fraction_error":max_peak_fraction,"peak_angle_degrees":angular_error,"gate_peak_fraction":0.03,"gate_peak_angle_degrees":1.0,"probes":probes.len()});
    fs::create_dir_all(evidence_dir()).unwrap();
    fs::write(
        evidence_dir().join("sun-angular.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("sun angular {report}");
    assert!(
        max_peak_fraction <= 0.03,
        "sun spatial interpolation misses fixed 3% peak-radiance gate"
    );
    assert!(
        angular_error <= 1.,
        "sun direction misses fixed 1 degree gate"
    );

    r.set_lighting(&Lighting {
        exposure_stops: -8.,
        ..Default::default()
    })
    .unwrap();
    let levels = data.specular_radiance().len();
    let mut probes = vec![];
    let mut references = vec![];
    for (mip, diffuse) in [(0, true), ((levels - 1) / 2, false), (levels - 1, false)] {
        let size = if diffuse {
            data.diffuse_irradiance().size()
        } else {
            data.specular_radiance()[mip].size()
        };
        for face in [
            CubeFace::PositiveX,
            CubeFace::PositiveY,
            CubeFace::NegativeZ,
        ] {
            let uv = if size == 1 { 0. } else { 1. / size as f64 };
            let d = face.direction([uv, uv]);
            let roughness = (!diffuse).then_some(mip as f64 / (levels - 1) as f64);
            let n = glam::DVec3::from_array(d);
            let fine = sun_reference(n, roughness, 512);
            let coarse = sun_reference(n, roughness, 256);
            for c in 0..3 {
                assert!(
                    (fine[c] - coarse[c]).abs() < fine[c] * 0.002 + 0.01,
                    "sun oracle did not converge"
                );
            }
            probes.push(Probe {
                direction: d,
                lod: mip as f64,
                diffuse,
            });
            references.push(fine);
        }
    }
    let values = run_probes(&mut r, &probes);
    for ((p, value), reference) in probes.iter().zip(values).zip(references) {
        for c in 0..3 {
            let actual = value[c] as f64 * 256.;
            assert!(
                (actual - reference[c]).abs() < reference[c] * 0.04 + 0.001,
                "sun convolution d={:?} diffuse={} lod={} actual={actual} ref={} channel={c}",
                p.direction,
                p.diffuse,
                p.lod,
                reference[c]
            );
        }
    }
}

#[test]
#[ignore = "exclusive Vulkan slot; affinity, busy publication, in-flight generations and transactional reload"]
fn gpu_environment_ownership_admission_and_reload() {
    let _validation = validation::begin(true);
    let mut r = environment_renderer(SampleCount::One);
    let data = constant([2., 3., 4.]);
    let map = upload_environment(&mut r, &data);
    let color = target(r.device(), 257, 257);
    let mesh = upload(&mut r, &triangle([1.; 3]), &INDICES);
    r.test_drain("environment lifetime instances");
    let mut instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.set_environment(Some(&map), Default::default()).unwrap();
    r.test_environment_busy(&data, &color, &camera(0.), &mesh, &mut instances);
    let report = lit(
        &mut r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    assert!(report.uploads.lighting > 0);
    let image = lighting_tests::hdr(&mut r);
    let before = r.stats();
    for source in [
        SOURCE.to_owned() + "\nnot_wgsl",
        SOURCE.replace(
            "var environment_diffuse:texture_cube_array<f32>",
            "var environment_diffuse:texture_2d<f32>",
        ),
    ] {
        assert!(matches!(
            r.reload(&source),
            Err(ReloadError::Rejected { .. })
        ));
        assert_eq!(r.stats().revision, before.revision);
        lit(
            &mut r,
            &color,
            &camera(0.),
            &mut [Draw::new(&mesh, &mut instances)],
        );
        assert_eq!(image, lighting_tests::hdr(&mut r));
    }
    r.reload(SOURCE).unwrap();
    lit(
        &mut r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    assert_eq!(image, lighting_tests::hdr(&mut r));
    r.set_environment(None, Default::default()).unwrap();
    lit(
        &mut r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    let fallback = lighting_tests::hdr(&mut r);
    assert_ne!(image, fallback);
    r.set_environment(
        Some(&map),
        EnvironmentParameters {
            rotation_y_radians: 0.7,
            intensity: 0.5,
        },
    )
    .unwrap();
    lit(
        &mut r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    assert_eq!(image, lighting_tests::hdr(&mut r));
    let unchanged = r.stats();
    for _ in 0..3 {
        let report = lit(
            &mut r,
            &color,
            &camera(0.),
            &mut [Draw::new(&mesh, &mut instances)],
        );
        assert_eq!(report.uploads.environment, 0);
        assert_eq!(report.uploads.lighting, 0);
        assert_eq!(
            r.stats().ownership.environment_creations,
            unchanged.ownership.environment_creations
        );
    }
    r.test_drain("environment parameter edit");
    let before = r.stats();
    r.set_environment(
        Some(&map),
        EnvironmentParameters {
            rotation_y_radians: 2.0,
            intensity: 2.0,
        },
    )
    .unwrap();
    let changed = lit(
        &mut r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    assert_eq!(changed.uploads.environment, 0);
    assert_eq!(changed.uploads.lights, 0);
    assert!(changed.uploads.lighting > 0);
    assert_eq!(before.ownership.created(), r.stats().ownership.created());
    r.test_drain("environment cap rejection");
    r.set_environment_limits(EnvironmentLimits {
        max_owned_bytes: data.texel_count() * 16,
        ..Default::default()
    })
    .unwrap();
    let before = r.stats();
    assert!(r.try_upload_environment(&data).is_err());
    assert_eq!(before.uploads, r.stats().uploads);
    assert_eq!(before.ownership, r.stats().ownership);
    r.set_environment_limits(Default::default()).unwrap();
    r.test_drain("environment affinity");
    let mut foreign = environment_renderer(SampleCount::One);
    foreign.test_drain("foreign environment setup");
    let before = foreign.stats();
    assert!(
        foreign
            .set_environment(Some(&map), Default::default())
            .is_err()
    );
    assert_eq!(before.uploads, foreign.stats().uploads);
    assert_eq!(before.ownership, foreign.stats().ownership);
    drop(foreign);
    // Replace maps under incomplete prior submissions; cache references own the last submitted use.
    let map_bytes = data.texel_count() * 16;
    let mut overlap = 0;
    let mut settled = vec![];
    for _ in 0..2 {
        for _ in 0..12 {
            let next = loop {
                if let Some(next) = r.try_upload_environment(&data).unwrap() {
                    break next;
                }
                r.poll().unwrap();
            };
            r.set_environment(Some(&next), Default::default()).unwrap();
            lit(
                &mut r,
                &color,
                &camera(0.),
                &mut [Draw::new(&mesh, &mut instances)],
            );
            drop(next);
            overlap += usize::from(r.test_overlap().0 > 0);
            assert!(r.stats().ownership.environment_bytes.live <= 48 + map_bytes * 6);
        }
        r.test_drain("environment replacement settled");
        settled.push(r.stats().ownership.environment_bytes.live);
    }
    assert_eq!(settled[0], settled[1]);
    assert!(overlap > 0, "no observed incomplete prior environment uses");
    eprintln!("environment replacement overlap={overlap} settled_bytes={settled:?}");
    r.set_environment(None, Default::default()).unwrap();
    lit(
        &mut r,
        &color,
        &camera(0.),
        &mut [Draw::new(&mesh, &mut instances)],
    );
    r.test_drain("environment fallback retirement");
    drop(map);
    r.test_drain("environment caller release");
    assert_eq!(r.stats().ownership.environment_bytes.live, 48);
    let mut featureless = renderer(false);
    featureless.test_drain("featureless environment");
    let map = upload_environment(&mut featureless, &data);
    featureless
        .set_environment(Some(&map), Default::default())
        .unwrap();
    assert_eq!(
        featureless.stats().ownership.environment_bytes.live,
        48 + map_bytes
    );
}

#[test]
#[ignore = "exclusive Vulkan slot; corrected normal fixture with active shadows and mapped environment"]
fn gpu_environment_normal_shadow_regular_virtual_images() {
    normal_shadow_images(&constant([0.2, 0.3, 0.4]), 0.0);
}

#[test]
#[ignore = "exclusive Vulkan slot; complete independent field, normal/shadow regular-virtual parity at 1x/4x"]
fn gpu_environment_independent_normal_shadow_regular_virtual_images() {
    let path = std::env::var_os("RENDERLAB_ENVIRONMENT_FIELD").expect("RLE2 field path");
    let data = env::decode(&fs::read(path).unwrap(), env::Limits::default()).unwrap();
    assert!(matches!(
        data.specular_layout(),
        env::SpecularLayout::IndependentRoughness { .. }
    ));
    normal_shadow_images(&data, -14.0);
}

fn normal_shadow_images(data: &PreparedEnvironment, exposure_stops: f32) {
    use super::materials::prepared::Prepared;
    use crate::virtual_geometry::*;
    let _validation = validation::begin(true);
    let directory = std::env::var_os("RENDERLAB_NORMAL_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .expect("run the CPU normal_render_fixture example first");
    for samples in [SampleCount::One, SampleCount::Four] {
        let mut r = environment_renderer(samples);
        r.set_lighting(&Lighting {
            exposure_stops,
            lights: vec![Light::Directional {
                travel: Vec3::new(-0.4, -0.7, -1.),
                lux: [2.; 3],
            }],
            ..Default::default()
        })
        .unwrap();
        r.set_shadows(&Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![20.],
            }],
            resolution: 512,
            ..Default::default()
        })
        .unwrap();
        let map = upload_environment(&mut r, data);
        r.set_environment(
            Some(&map),
            EnvironmentParameters {
                rotation_y_radians: 0.8,
                ..Default::default()
            },
        )
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
