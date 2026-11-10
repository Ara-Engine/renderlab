//! Independent geometric oracle; it does not consume generated shader output.
use renderlab::{Camera, Light, Vec3};
pub const X: u32 = 32;
pub const Y: u32 = 18;
pub const Z: u32 = 24;
pub const COUNT: u32 = X * Y * Z;
pub const STRIDE: u32 = 65;
pub const ROW: u32 = 4;
pub fn expected(lights: &[Light], camera: Camera, size: [u32; 2], cluster: u32) -> Vec<u32> {
    let fov = camera.fov_y() * 0.5;
    let py = fov.cos() / fov.sin();
    let px = py / (size[0] as f32 / size[1] as f32);
    let near = camera.near();
    let far = camera.far();
    let scale = Z as f32 / (far / near).log2().max(1e-6);
    let depth = |slice: u32| {
        if slice >= Z {
            far
        } else {
            near * (slice as f32 / scale).exp2()
        }
    };
    let tx = cluster % X;
    let ty = (cluster / X) % Y;
    let tz = cluster / (X * Y);
    let x0 = -1. + 2. * tx as f32 / X as f32;
    let x1 = -1. + 2. * (tx + 1) as f32 / X as f32;
    let y1 = 1. - 2. * ty as f32 / Y as f32;
    let y0 = 1. - 2. * (ty + 1) as f32 / Y as f32;
    let d0 = depth(tz);
    let d1 = depth(tz + 1);
    let xs = [x0 * d0, x0 * d1, x1 * d0, x1 * d1].map(|x| x / px);
    let ys = [y0 * d0, y0 * d1, y1 * d0, y1 * d1].map(|y| y / py);
    let lo = Vec3::new(
        xs.into_iter().fold(f32::INFINITY, f32::min),
        ys.into_iter().fold(f32::INFINITY, f32::min),
        -d1,
    );
    let hi = Vec3::new(
        xs.into_iter().fold(f32::NEG_INFINITY, f32::max),
        ys.into_iter().fold(f32::NEG_INFINITY, f32::max),
        -d0,
    );
    lights
        .iter()
        .enumerate()
        .filter(|(_, light)| {
            let (mut centre, range, spot) = match **light {
                Light::Directional { .. } => return true,
                Light::Point {
                    position, range, ..
                } => (position, range, None),
                Light::Spot {
                    position,
                    range,
                    travel,
                    outer,
                    ..
                } => (position, range, Some((travel.normalize(), outer))),
            };
            let Some(mut radius) = range else {
                return true;
            };
            if let Some((axis, angle)) = spot {
                let c = angle.cos().clamp(0., 1.);
                if c > std::f32::consts::FRAC_1_SQRT_2 {
                    radius /= 2. * c;
                    centre += axis * radius;
                } else {
                    radius *= (1. - c * c).max(0.).sqrt();
                    centre += axis * (range.expect("bounded range") * c);
                }
            }
            let point = camera.view().transform_point3(centre);
            let radius = radius * 1.001 + 1e-4;
            point.distance_squared(point.clamp(lo, hi)) <= radius * radius
        })
        .map(|(i, _)| i as u32)
        .collect()
}
