//! The assembled forward lit frame against Renderlab's own closed-form photometry.
//!
//! The scene, sample points, expected radiance and tolerance are the ones
//! `gpu_lit_photometry_1x_4x` holds the wgpu renderer to: a floor facing the camera,
//! one light at a time, the BRDF and attenuation evaluated in f64, and the uniform
//! environment integrated by an independent quadrature rather than read from the LUT.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, PostSettings, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, post_record, render},
};
const SIZE: [u32; 2] = [128, 96];
const BASE: [f32; 4] = [0.18, 0.4, 0.7, 1.];

fn brdf(n: DVec3, v: DVec3, l: DVec3, base: DVec3, metal: f64, rough: f64) -> DVec3 {
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
// Midpoint product quadrature over GGX's inverse CDF and azimuth.
fn integrate_dfg(nv: f64, r: f64, nu: usize, np: usize) -> [f64; 2] {
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
/// Direction to the light and its unexposed energy at floor point `p`.
fn arriving(light: &Light, p: DVec3) -> (DVec3, DVec3) {
    match *light {
        Light::Directional { travel, lux } => (
            -travel.as_dvec3().normalize(),
            DVec3::from_array(lux.map(f64::from)),
        ),
        Light::Point {
            position,
            candela,
            range,
            ..
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
            } = *light
            {
                a *= ((-l.dot(travel.as_dvec3().normalize()) - f64::from(outer).cos())
                    / (f64::from(inner).cos() - f64::from(outer).cos()).max(1e-4))
                .clamp(0., 1.)
                .powi(2);
            }
            (l, DVec3::from_array(candela.map(f64::from)) * a)
        }
    }
}
#[test]
fn forward_lit_frame_matches_closed_form_photometry() -> Result<()> {
    let path = root("forward");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let mut frame = |metal: f32, rough: f32, lighting: &Lighting| -> Result<Vec<f32>> {
        let bytes = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[ForwardObject {
                    vertices: &vertices,
                    indices: &[0, 1, 2, 0, 2, 3],
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        base_color: BASE,
                        metallic: metal,
                        roughness: rough,
                        ..Default::default()
                    },
                }],
                lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                post: None,
            },
        )?
        .hdr;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f16::from_bits(u16::from_le_bytes(*b)).to_f32())
            .collect())
    };
    // The floor point under a pixel centre, and the unit direction back to the eye.
    let tan = (f64::from(camera.fov_y()) / 2.).tan();
    let floor = |x: usize, y: usize| {
        let p = DVec3::new(
            (2. * (x as f64 + 0.5) / 128. - 1.) * tan * 4. * 128. / 96.,
            (1. - 2. * (y as f64 + 0.5) / 96.) * tan * 4.,
            0.,
        );
        (p, (DVec3::new(0., 0., 4.) - p).normalize())
    };
    let base = DVec3::from_array([BASE[0], BASE[1], BASE[2]].map(f64::from));
    let lights = [
        Light::Directional {
            travel: Vec3::new(-0.25, 0., -1.),
            lux: [1000.; 3],
        },
        Light::Point {
            position: Vec3::Z,
            candela: [100.; 3],
            range: None,
            source_radius: 0.0,
        },
        Light::Point {
            position: Vec3::Z * 2.,
            candela: [100.; 3],
            range: None,
            source_radius: 0.0,
        },
        Light::Point {
            position: Vec3::Z,
            candela: [100.; 3],
            range: Some(0.9),
            source_radius: 0.0,
        },
        Light::Spot {
            position: Vec3::Z * 2.,
            travel: -Vec3::Z,
            candela: [100.; 3],
            range: Some(4.),
            inner: 0.25,
            outer: 0.5,
            source_radius: 0.0,
        },
    ];
    let mut worst = 0f64;
    let mut samples = 0;
    for (metal, rough) in [
        (0., 0.1),
        (0., 0.5),
        (0., 1.),
        (1., 0.1),
        (1., 0.5),
        (1., 1.),
    ] {
        for light in &lights {
            let image = frame(
                metal,
                rough,
                &Lighting {
                    lights: vec![*light],
                    exposure_stops: -8.,
                    ..Default::default()
                },
            )?;
            for y in (18..78).step_by(11) {
                for x in (18..110).step_by(13) {
                    let (p, v) = floor(x, y);
                    let (l, energy) = arriving(light, p);
                    let expected = brdf(DVec3::Z, v, l, base, f64::from(metal), f64::from(rough))
                        * energy
                        / 256.;
                    for channel in 0..3 {
                        let actual = f64::from(image[(y * 128 + x) * 4 + channel]);
                        let error = (actual - expected[channel]).abs();
                        ensure!(
                            actual.is_finite() && error <= 0.002 + expected[channel] * 0.01,
                            "{light:?} m={metal} r={rough} {x},{y} channel={channel} got {actual} expected {}",
                            expected[channel]
                        );
                        worst = worst.max(error / (0.002 + expected[channel] * 0.01));
                        samples += 1;
                    }
                }
            }
        }
        // The uniform environment through the prepared DFG, at the centre pixel.
        let image = frame(
            metal,
            rough,
            &Lighting {
                environment: [0.3; 3],
                ..Default::default()
            },
        )?;
        let (p, _) = floor(64, 48);
        let nv = 4. / (16. + p.x * p.x + p.y * p.y).sqrt();
        let ab = integrate_dfg(nv, f64::from(rough), 512, 1024);
        for channel in 0..3 {
            let metal = f64::from(metal);
            let expected = 0.3
                * (base[channel] * (1. - metal)
                    + (0.04 * (1. - metal) + base[channel] * metal) * ab[0]
                    + ab[1]);
            let actual = f64::from(image[(48 * 128 + 64) * 4 + channel]);
            ensure!(
                (actual - expected).abs() < 0.002 + 0.01 * expected,
                "environment m={metal} r={rough} channel={channel} got {actual} expected {expected}"
            );
            samples += 1;
        }
    }
    println!(
        "forward frame: {samples} radiance samples within Renderlab's photometry tolerance; worst used {:.1}% of it",
        worst * 100.
    );
    gpu.assert_clean()?;
    Ok(())
}

/// The frame's own HDR and motion surface, pushed through the scalar post replay, must
/// give the image the GPU displayed: motion blur, six bloom levels, tone and transfer
/// run in the renderer's order with its default settings.
#[test]
fn forward_frame_display_matches_cpu_post_replay_of_its_hdr() -> Result<()> {
    let path = root("forward-display");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let size = [96u32, 64];
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let settings = PostSettings::default();
    // A glossy metal floor under a tilted sun: a highlight far above one for bloom.
    let images = render(
        &mut gpu,
        &cache,
        &ForwardScene {
            objects: &[ForwardObject {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: MaterialDescriptor {
                    base_color: BASE,
                    metallic: 1.,
                    roughness: 0.3,
                    ..Default::default()
                },
            }],
            lighting: &Lighting {
                lights: vec![
                    Light::Directional {
                        travel: Vec3::new(-0.25, 0., -1.),
                        lux: [1000.; 3],
                    },
                    Light::Point {
                        position: Vec3::new(1.5, -1., 1.),
                        candela: [60., 40., 20.],
                        range: Some(6.),
                        source_radius: 0.0,
                    },
                ],
                environment: [20., 25., 40.],
                exposure_stops: -8.,
            },
            camera: Camera::look_at(
                Vec3::new(0., 0., 4.),
                Vec3::ZERO,
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                100.,
            )?,
            size,
            atmosphere: renderlab::Atmosphere::off(),
            post: Some(settings),
        },
    )?;
    let image = |pixels| reference::Image { size, pixels };
    let hdr = image(
        images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
                })
            })
            .collect(),
    );
    let surface = images.surface.expect("motion blur is on by default");
    let aux = image(
        surface
            .as_chunks::<16>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f32::from_le_bytes([p[c * 4], p[c * 4 + 1], p[c * 4 + 2], p[c * 4 + 3]])
                })
            })
            .collect(),
    );
    let words = post_record(&settings, size[1]);
    let tiles = reference::tile_max(&aux);
    let blurred = reference::motion_blur(&hdr, &aux, &tiles, &words);
    let mut levels = Vec::new();
    let mut current = blurred.clone();
    for _ in 0..6 {
        current = reference::downsample(&current, current.size.map(|n| n.div_ceil(2)));
        levels.push(current.clone());
        if current.size == [1, 1] {
            break;
        }
    }
    let coarsest = levels.last().expect("bloom has a level");
    let mut bloom = reference::upsample(coarsest, coarsest);
    for level in levels[..levels.len() - 1].iter().rev() {
        bloom = reference::upsample(level, &bloom);
    }
    let fixture = reference::Fixture {
        hdr: hdr.clone(),
        lut: (0..8u32)
            .map(|i| [i & 1, (i >> 1) & 1, i >> 2, 1].map(|v| v as f32))
            .collect(),
        lut_size: 2,
        settings: words,
    };
    let tone = reference::tone(&blurred, &bloom, &fixture);
    // Tone agrees to 0.003 linear in the stage replays; the display adds UNORM rounding.
    let encode = |linear: &reference::Image| reference::display(linear).pixels;
    let shifted = |by: f32| {
        encode(&image(
            tone.pixels.iter().map(|p| p.map(|v| v + by)).collect(),
        ))
    };
    let (low, high) = (shifted(-0.003), shifted(0.003));
    let display = images.display.expect("post ran");
    let mut brightest = 0f32;
    let mut distinct = std::collections::BTreeSet::new();
    for (i, actual) in display.as_chunks::<4>().0.iter().enumerate() {
        distinct.insert(*actual);
        brightest = brightest.max(hdr.pixels[i][0].max(hdr.pixels[i][1]).max(hdr.pixels[i][2]));
        for channel in 0..3 {
            let value = f32::from(actual[channel]) / 255.;
            ensure!(
                value >= low[i][channel] - 0.6 / 255. && value <= high[i][channel] + 0.6 / 255.,
                "display pixel {i} channel {channel}: {value} outside [{}, {}]",
                low[i][channel],
                high[i][channel]
            );
        }
        ensure!(actual[3] == 255, "display alpha");
    }
    ensure!(
        brightest > 1.5 && distinct.len() > 64,
        "fixture lost its highlight or its gradient: peak {brightest}, {} colours",
        distinct.len()
    );
    println!(
        "forward display: {} pixels inside the CPU post replay's bounds; HDR peak {brightest}, {} distinct display colours",
        display.len() / 4,
        distinct.len()
    );
    gpu.assert_clean()?;
    Ok(())
}

/// The atmosphere on the frame: sun transmittance reaches the directional light exactly
/// once, the backdrop replaces the clear, and the procedural environment lights a
/// surface the sun cannot. The sky's own radiance is the stage replays' business and
/// the cross-layer comparison's; this holds the assembly.
#[test]
fn forward_frame_atmosphere_scales_the_sun_and_lights_from_the_sky() -> Result<()> {
    use renderlab::Atmosphere;
    let path = root("forward-atmosphere");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    // A floor that leaves the frame's corners to the backdrop.
    let vertices = [
        [-1.5, -1.5, 0.],
        [1.5, -1.5, 0.],
        [1.5, 1.5, 0.],
        [-1.5, 1.5, 0.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let mut frame = |travel: Vec3, atmosphere: Atmosphere| -> Result<Vec<[f32; 4]>> {
        let bytes = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[ForwardObject {
                    vertices: &vertices,
                    indices: &[0, 1, 2, 0, 2, 3],
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        base_color: BASE,
                        metallic: 0.,
                        roughness: 1.,
                        ..Default::default()
                    },
                }],
                lighting: &Lighting {
                    lights: vec![Light::Directional {
                        travel,
                        lux: [1000.; 3],
                    }],
                    exposure_stops: -8.,
                    ..Default::default()
                },
                camera,
                size: SIZE,
                atmosphere,
                post: None,
            },
        )?
        .hdr;
        Ok(bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
                })
            })
            .collect())
    };
    let centre = 48 * 128 + 64;
    let base = DVec3::from_array([BASE[0], BASE[1], BASE[2]].map(f64::from));

    // Sun in front of the floor, low enough for a strongly coloured transmittance.
    let travel = Vec3::new(-0.9, -0.2, -0.4);
    let sun_only = Atmosphere {
        sun_disk: false,
        aerial_perspective: false,
        environment: false,
        height_fog: None,
        volumetric_fog: None,
        ..Default::default()
    };
    let image = frame(travel, sun_only)?;
    let toward = -travel.normalize();
    let transmittance = sun_only.transmittance(0., toward);
    ensure!(
        transmittance[2] < 0.9 * transmittance[0] && transmittance[0] < 0.99,
        "fixture sun is not attenuated: {transmittance:?}"
    );
    let expected = brdf(DVec3::Z, DVec3::Z, toward.as_dvec3(), base, 0., 1.) * 1000. / 256.;
    for channel in 0..3 {
        let expected = expected[channel] * f64::from(transmittance[channel]);
        let actual = f64::from(image[centre][channel]);
        ensure!(
            (actual - expected).abs() <= 0.002 + expected * 0.01,
            "attenuated sun channel {channel}: got {actual} expected {expected}"
        );
    }
    let corner = image[0];
    ensure!(
        corner[..3].iter().all(|v| v.is_finite() && *v > 0.),
        "backdrop did not replace the black clear: {corner:?}"
    );

    // Sun behind the floor: only the sky can light it.
    let behind = Vec3::new(-0.3, -0.6, 0.4);
    let dark = frame(behind, sun_only)?[centre];
    let lit = frame(
        behind,
        Atmosphere {
            environment: true,
            ..sun_only
        },
    )?[centre];
    ensure!(
        dark[..3].iter().all(|v| *v < 1e-4),
        "floor facing away from the sun is lit without an environment: {dark:?}"
    );
    ensure!(
        lit[..3].iter().all(|v| v.is_finite() && *v > 1e-3) && lit[2] > lit[0],
        "procedural environment did not light the floor as a blue-weighted sky: {lit:?}"
    );
    println!(
        "forward atmosphere: sun transmittance {transmittance:?} applied once; backdrop {corner:?}; sky-lit floor {lit:?}"
    );
    gpu.assert_clean()?;
    Ok(())
}
