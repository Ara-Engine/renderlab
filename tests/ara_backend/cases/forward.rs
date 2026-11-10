//! The assembled forward lit frame against Renderlab's own closed-form photometry.
//!
//! The scene, sample points, expected radiance and tolerance are the ones
//! `gpu_lit_photometry_1x_4x` holds the wgpu renderer to: a floor facing the camera,
//! one light at a time, the BRDF and attenuation evaluated in f64, and the uniform
//! environment integrated by an independent quadrature rather than read from the LUT.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, render},
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
            },
        )?;
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
