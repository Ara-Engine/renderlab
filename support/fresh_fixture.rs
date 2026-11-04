//! Registered thin/contact and speck/wire planes. Casters participate in both
//! camera rasterization and ray queries; the stress set adds twelve rotated wires.
use crate::{Mat4, Vec3};

#[derive(Clone, Copy, serde::Serialize)]
pub struct Rectangle {
    pub center: [f32; 3],
    pub half: [f32; 2],
    pub angle: f32,
}
impl Rectangle {
    pub fn transform(self) -> Mat4 {
        // The quad lies at z = 0, so z scale cannot move geometry or the normalized normal.
        // Matching it to the thinner axis keeps the renderer's transform-condition bound for wires.
        let (x, y) = (self.half[0] / 8., self.half[1] / 8.);
        Mat4::from_scale_rotation_translation(
            Vec3::new(x, y, x.min(y)),
            glam::Quat::from_rotation_z(self.angle),
            Vec3::from_array(self.center),
        )
    }
}

pub fn rectangles(stress: bool) -> Vec<Rectangle> {
    let mut out = vec![];
    let mut rect = |center, half, angle| {
        out.push(Rectangle {
            center,
            half,
            angle,
        })
    };
    if !stress {
        rect([0.6, 0., 0.6], [0.01, 1.6], 0.);
        rect([1., 0.4, 0.03], [0.24, 0.24], 0.);
        return out;
    }
    for (i, h) in [0.1, 0.2, 0.4].into_iter().enumerate() {
        for j in 0..6 {
            for k in 0..4 {
                rect(
                    [
                        -2. + 0.55 * j as f32 + 0.13 * k as f32 + 0.37 * h,
                        -1.6 + 0.35 * k as f32 + 1.05 * i as f32 + 0.017 * j as f32,
                        h,
                    ],
                    [0.005; 2],
                    0.,
                );
            }
        }
    }
    for (i, h) in [0.1, 0.3].into_iter().enumerate() {
        for j in 0..3 {
            let x = 0.9 + 0.35 * j as f32 + 0.6 * i as f32;
            let y = -1.5 + 0.2 * j as f32;
            rect([x, y + 0.45, h], [0.0005, 0.45], 0.);
            rect(
                [x - 0.15, 1.0 + 0.25 * j as f32 + 0.4 * i as f32, h],
                [0.45, 0.0005],
                0.,
            );
        }
    }
    for j in 0..12 {
        rect(
            [-1.8 + 0.32 * j as f32, -1.85, 0.3],
            [0.12, 0.0005],
            j as f32 * std::f32::consts::PI / 12.,
        );
    }
    out
}
