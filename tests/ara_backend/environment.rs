//! Independent cosine/GGX integration in the renderer's six-face convention.
use super::{
    atmosphere_reference::Model,
    reference::{Image, half, sample},
};
use renderlab::Vec3;
const PI: f32 = std::f32::consts::PI;
pub fn sky() -> Image {
    let size = [33, 19];
    let pixels = (0..33 * 19)
        .map(|i| {
            let x = (i % 33) as f32 / 32.;
            let y = (i / 33) as f32 / 18.;
            half([
                0.1 + 1.3 * x,
                0.15 + 0.8 * y,
                0.1 + 0.7 * x * y + if i == 14 * 33 + 8 { 2. } else { 0. },
                1.,
            ])
        })
        .collect();
    Image { size, pixels }
}
pub fn cube(model: &Model, sky: &Image, side: u32, diffuse: bool) -> Image {
    let pixels = (0..6 * side * side)
        .map(|i| {
            let face = i / (side * side);
            let x = i % side;
            let y = i / side % side;
            let uv = glam::Vec2::new(
                2. * (x as f32 + 0.5) / side as f32 - 1.,
                2. * (y as f32 + 0.5) / side as f32 - 1.,
            );
            let n = match face {
                0 => Vec3::new(1., -uv.y, -uv.x),
                1 => Vec3::new(-1., -uv.y, uv.x),
                2 => Vec3::new(uv.x, 1., uv.y),
                3 => Vec3::new(uv.x, -1., -uv.y),
                4 => Vec3::new(uv.x, -uv.y, 1.),
                _ => Vec3::new(-uv.x, -uv.y, -1.),
            }
            .normalize();
            let up = if n.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
            let t = up.cross(n).normalize();
            let frame = glam::Mat3::from_cols(t, n.cross(t), n);
            let radiance = |d: Vec3| {
                let p = sample(sky, model.sky_uv(d));
                Vec3::new(p[0], p[1], p[2])
            };
            let rough = (128. / side as f32).log2() / 4.;
            let value = if diffuse {
                let mut sum = Vec3::ZERO;
                for tap in 0..256u32 {
                    let xi = [
                        (tap as f32 + 0.5) / 256.,
                        tap.reverse_bits() as f32 * 2.328_306_4e-10,
                    ];
                    let r = xi[1].sqrt();
                    let phi = 2. * PI * xi[0];
                    let d = frame
                        * Vec3::new(r * phi.cos(), r * phi.sin(), (1. - xi[1]).max(0.).sqrt());
                    sum += radiance(d);
                }
                PI * sum / 256.
            } else if rough > 0. {
                let a = rough * rough;
                let mut sum = Vec3::ZERO;
                let mut weight = 0.;
                for tap in 0..64u32 {
                    let xi = [
                        (tap as f32 + 0.5) / 64.,
                        tap.reverse_bits() as f32 * 2.328_306_4e-10,
                    ];
                    let ct = ((1. - xi[1]) / (1. + (a * a - 1.) * xi[1])).sqrt();
                    let st = (1. - ct * ct).max(0.).sqrt();
                    let phi = 2. * PI * xi[0];
                    let h = frame * Vec3::new(st * phi.cos(), st * phi.sin(), ct);
                    let d = 2. * n.dot(h) * h - n;
                    let nl = n.dot(d);
                    if nl > 0. {
                        sum += radiance(d) * nl;
                        weight += nl;
                    }
                }
                sum / weight.max(1e-6)
            } else {
                radiance(n)
            };
            [value.x, value.y, value.z, 1.]
        })
        .collect();
    Image {
        size: [side, side * 6],
        pixels,
    }
}

/// Diagnostic at the unsmoothed base level: emulate 8-bit filter fractions to
/// distinguish hardware interpolation precision from a direction/mip mismatch.
pub fn base_probe(model: &Model, sky: &Image, index: usize) -> [f32; 4] {
    let side = 128u32;
    let i = index as u32;
    let face = i / (side * side);
    let x = i % side;
    let y = i / side % side;
    let uv = glam::Vec2::new(
        2. * (x as f32 + 0.5) / side as f32 - 1.,
        2. * (y as f32 + 0.5) / side as f32 - 1.,
    );
    let d = match face {
        0 => Vec3::new(1., -uv.y, -uv.x),
        1 => Vec3::new(-1., -uv.y, uv.x),
        2 => Vec3::new(uv.x, 1., uv.y),
        3 => Vec3::new(uv.x, -1., -uv.y),
        4 => Vec3::new(uv.x, -uv.y, 1.),
        _ => Vec3::new(-uv.x, -uv.y, -1.),
    }
    .normalize();
    let uv = model.sky_uv(d);
    let uv = std::array::from_fn(|axis| {
        let n = sky.size[axis] as f32;
        let x = uv[axis] * n - 0.5;
        (x.floor() + ((x - x.floor()) * 256.).round() / 256. + 0.5) / n
    });
    sample(sky, uv)
}
