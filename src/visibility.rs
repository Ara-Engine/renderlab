use crate::Mat4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn from_positions(positions: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut positions = positions.into_iter();
        let first = positions.next()?;
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for position in std::iter::once(first).chain(positions) {
            if !position.iter().all(|v| v.is_finite()) {
                return None;
            }
            for (axis, value) in position.into_iter().enumerate() {
                bounds.min[axis] = bounds.min[axis].min(value);
                bounds.max[axis] = bounds.max[axis].max(value);
            }
        }
        Some(bounds)
    }
}

/// Conservative homogeneous clipping; uncertain arithmetic keeps the object visible.
/// The shader must evaluate camera * model * the original bounded vertex position.
pub(crate) fn may_be_visible(clip: &Mat4, model: &Mat4, bounds: &Aabb) -> bool {
    let clip = clip.to_cols_array_2d();
    let model = model.to_cols_array_2d();
    let inputs = clip
        .iter()
        .flatten()
        .chain(model.iter().flatten())
        .chain(&bounds.min)
        .chain(&bounds.max);
    let mut magnitude = 1.0_f64;
    for &value in inputs {
        if !value.is_finite() {
            return true;
        }
        magnitude = magnitude.max(f64::from(value).abs());
    }
    if 128.0 * magnitude.powi(3) >= f64::from(f32::MAX) {
        return true;
    }
    let mut center = [0.0; 4];
    let mut extent = [0.0; 4];
    let mut absolute_position = [0.0; 4];
    for axis in 0..3 {
        let low = f64::from(bounds.min[axis]);
        let high = f64::from(bounds.max[axis]);
        center[axis] = (low + high) * 0.5;
        extent[axis] = (high - low) * 0.5;
        absolute_position[axis] = low.abs().max(high.abs());
    }
    center[3] = 1.0;
    absolute_position[3] = 1.0;

    let mut absolute_world = [0.0; 4];
    for (row, value) in absolute_world.iter_mut().enumerate() {
        *value = (0..4)
            .map(|col| f64::from(model[col][row]).abs() * absolute_position[col])
            .sum();
    }
    let mut absolute_clip = [0.0; 4];
    let mut product = [[0.0; 4]; 4]; // Row-major f64 composition.
    for row in 0..4 {
        absolute_clip[row] = (0..4)
            .map(|col| f64::from(clip[col][row]).abs() * absolute_world[col])
            .sum();
        for (col, value) in product[row].iter_mut().enumerate() {
            *value = (0..4)
                .map(|k| f64::from(clip[k][row]) * f64::from(model[col][k]))
                .sum();
        }
    }

    // Allow faithful rounding/reassociation and amplified subnormal flushing.
    // Derivation and falsification contract: docs/m2-m4-design.md, Visibility and reload.
    const RELATIVE: f64 = 1.0 / 32768.0;
    let absolute = f64::from(f32::MIN_POSITIVE) * 256.0 * magnitude.powi(2);
    for plane in 0..6 {
        let mut coefficients = [0.0; 4];
        let envelope;
        if plane == 4 {
            coefficients = product[2];
            envelope = absolute_clip[2];
        } else {
            let axis = plane / 2;
            let sign = if plane % 2 == 0 { 1.0 } else { -1.0 };
            for (col, value) in coefficients.iter_mut().enumerate() {
                *value = product[3][col] + sign * product[axis][col];
            }
            envelope = absolute_clip[3] + absolute_clip[axis];
        }
        let maximum: f64 = (0..4)
            .map(|axis| coefficients[axis] * center[axis] + coefficients[axis].abs() * extent[axis])
            .sum();
        if maximum + RELATIVE * envelope + absolute < 0.0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Vec3};

    fn cube() -> Aabb {
        Aabb::from_positions([[-0.5; 3], [0.5; 3]]).unwrap()
    }

    #[test]
    fn bounds_and_clip_planes() {
        assert!(Aabb::from_positions([]).is_none());
        assert!(Aabb::from_positions([[0.0, f32::NAN, 1.0]]).is_none());
        assert_eq!(
            Aabb::from_positions([[2.0, -3.0, 4.0], [-2.0, 1.0, 0.0]]),
            Some(Aabb {
                min: [-2.0, -3.0, 0.0],
                max: [2.0, 1.0, 4.0]
            })
        );
        let bounds = cube();
        assert!(may_be_visible(&Mat4::IDENTITY, &Mat4::IDENTITY, &bounds));
        for translation in [
            Vec3::X * 3.0,
            Vec3::X * -3.0,
            Vec3::Y * 3.0,
            Vec3::Y * -3.0,
            Vec3::Z * 3.0,
            Vec3::Z * -3.0,
        ] {
            assert!(!may_be_visible(
                &Mat4::IDENTITY,
                &Mat4::from_translation(translation),
                &bounds
            ));
        }
        for x in [1.5, -1.5, 1.5_f32.next_up(), (-1.5_f32).next_down()] {
            assert!(may_be_visible(
                &Mat4::IDENTITY,
                &Mat4::from_translation(Vec3::new(x, 0.0, 0.5)),
                &bounds
            ));
        }
        assert!(may_be_visible(
            &Mat4::IDENTITY,
            &Mat4::from_translation(Vec3::splat(f32::MAX)),
            &bounds
        ));
    }

    #[derive(Clone, Copy, Debug)]
    struct Arithmetic {
        direction: i8,
        flush: bool,
        accumulation: u8,
    }

    impl Arithmetic {
        fn input(self, value: f32) -> f64 {
            f64::from(if self.flush && value.is_subnormal() {
                0.0_f32.copysign(value)
            } else {
                value
            })
        }

        fn round(self, exact: f64) -> f32 {
            let mut rounded = exact as f32;
            if self.direction < 0 && f64::from(rounded) > exact {
                rounded = rounded.next_down();
            } else if self.direction > 0 && f64::from(rounded) < exact {
                rounded = rounded.next_up();
            }
            if self.flush && rounded.is_subnormal() {
                0.0_f32.copysign(rounded)
            } else {
                rounded
            }
        }

        fn dot(self, a: [f32; 4], b: [f32; 4]) -> f32 {
            let mut terms = [0.0; 4];
            if self.accumulation == 3 {
                let mut result = 0.0;
                for index in 0..4 {
                    result = self
                        .round(self.input(a[index]) * self.input(b[index]) + self.input(result));
                }
                return result;
            }
            for index in 0..4 {
                terms[index] = self.round(self.input(a[index]) * self.input(b[index]));
            }
            if self.accumulation == 2 {
                let left = self.round(self.input(terms[0]) + self.input(terms[1]));
                let right = self.round(self.input(terms[2]) + self.input(terms[3]));
                return self.round(self.input(left) + self.input(right));
            }
            if self.accumulation == 1 {
                terms.reverse();
            }
            terms.into_iter().fold(0.0, |sum, term| {
                self.round(self.input(sum) + self.input(term))
            })
        }

        fn transform(self, matrix: [[f32; 4]; 4], point: [f32; 4]) -> [f32; 4] {
            std::array::from_fn(|row| self.dot(std::array::from_fn(|col| matrix[col][row]), point))
        }
    }

    fn outside_mask(p: [f32; 4]) -> u8 {
        // Compare coordinates directly, independently of the predicate's plane arithmetic.
        [
            p[0] < -p[3],
            p[0] > p[3],
            p[1] < -p[3],
            p[1] > p[3],
            p[2] < 0.0,
            p[2] > p[3],
        ]
        .into_iter()
        .enumerate()
        .fold(0, |mask, (bit, outside)| mask | (u8::from(outside) << bit))
    }

    fn assert_rejection_is_conservative(clip: Mat4, model: Mat4, bounds: Aabb) {
        if may_be_visible(&clip, &model, &bounds) {
            return;
        }
        let clip = clip.to_cols_array_2d();
        let model = model.to_cols_array_2d();
        let mut common_outside = 0b11_1111;
        for direction in [-1, 0, 1] {
            for flush in [false, true] {
                for accumulation in 0..4 {
                    let arithmetic = Arithmetic {
                        direction,
                        flush,
                        accumulation,
                    };
                    let composed =
                        std::array::from_fn(|col| arithmetic.transform(clip, model[col]));
                    for x in 0..3 {
                        for y in 0..3 {
                            for z in 0..3 {
                                let mut point = [0.0; 4];
                                for (axis, step) in [x, y, z].into_iter().enumerate() {
                                    point[axis] = match step {
                                        0 => bounds.min[axis],
                                        1 => {
                                            (f64::from(bounds.min[axis]) * 0.5
                                                + f64::from(bounds.max[axis]) * 0.5)
                                                as f32
                                        }
                                        _ => bounds.max[axis],
                                    };
                                }
                                point[3] = 1.0;
                                for result in [
                                    arithmetic.transform(composed, point),
                                    arithmetic.transform(clip, arithmetic.transform(model, point)),
                                ] {
                                    common_outside &= outside_mask(result);
                                    assert_ne!(
                                        common_outside, 0,
                                        "false rejection: arithmetic={arithmetic:?} point={point:?} clip={clip:?} model={model:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn visibility_matches_independent_f32_evaluation() {
        let clip = Camera::look_at(
            Vec3::new(0.0, 0.0, 4.0),
            Vec3::ZERO,
            Vec3::Y,
            1.0,
            0.1,
            100.0,
        )
        .unwrap()
        .matrix(641, 479);
        let mut seed = 0x93d7_a531_u32;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
        };
        let mut rejected = 0;
        let mut retained = 0;
        for case in 0..512 {
            let mut values = Mat4::IDENTITY.to_cols_array();
            for axis in 0..3 {
                values[axis * 5] = random() * 3.0;
                values[12 + axis] = random() * 30.0;
            }
            values[4] = random();
            values[9] = random();
            if case % 4 == 0 {
                values[3] = random();
                values[7] = random();
            }
            if case % 7 == 0 {
                values[0] = 0.0;
                values[1] = 0.0;
                values[2] = 0.0;
            }
            let model = Mat4::from_cols_array(&values);
            if may_be_visible(&clip, &model, &cube()) {
                retained += 1;
            } else {
                rejected += 1;
            }
            assert_rejection_is_conservative(clip, model, cube());
        }
        assert!(
            rejected > 100 && retained > 20,
            "rejected={rejected} retained={retained}"
        );
        for scale in [
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            1e-20,
            1.0,
            1e10,
            1e20,
            f32::MAX,
        ] {
            for sign in [-1.0, 1.0] {
                let model = Mat4::from_scale(Vec3::new(sign * scale, scale, scale));
                let mut projective = Mat4::IDENTITY.to_cols_array();
                projective[3] = scale;
                projective[15] = scale;
                assert_rejection_is_conservative(clip, model, cube());
                assert_rejection_is_conservative(Mat4::from_cols_array(&projective), model, cube());
            }
        }
    }
}
