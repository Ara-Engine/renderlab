use crate::Mat4;

/// Padded inverse-transpose columns and determinant sign. Zero sign means unsupported.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NormalTransform(pub [[f32; 4]; 4]);
impl NormalTransform {
    pub fn new(model: Mat4) -> Self {
        let m = model.to_cols_array_2d();
        let invalid = Self([[0.0; 4]; 4]);
        if m[0][3] != 0.0 || m[1][3] != 0.0 || m[2][3] != 0.0 || m[3][3] != 1.0 {
            return invalid;
        }
        let a = glam::DMat3::from_cols(
            glam::DVec3::new(m[0][0].into(), m[0][1].into(), m[0][2].into()),
            glam::DVec3::new(m[1][0].into(), m[1][1].into(), m[1][2].into()),
            glam::DVec3::new(m[2][0].into(), m[2][1].into(), m[2][2].into()),
        );
        let det = a.determinant();
        if det == 0.0 || !det.is_finite() {
            return invalid;
        }
        let inverse = a.inverse();
        let max = |a: glam::DMat3| {
            a.to_cols_array()
                .into_iter()
                .map(f64::abs)
                .fold(0.0, f64::max)
        };
        // A conservative numerical domain, independent of absolute uniform scale.
        if !inverse.is_finite() || max(a) * max(inverse) > 1e4 {
            return invalid;
        }
        let normal = inverse.transpose() / max(inverse);
        let mut result = [[0.0; 4]; 4];
        for (dst, src) in result.iter_mut().zip(normal.to_cols_array_2d()) {
            for (dst, src) in dst.iter_mut().zip(src) {
                *dst = src as f32;
            }
        }
        result[3][0] = det.signum() as f32;
        Self(result)
    }
    pub fn valid(self) -> bool {
        self.0[3][0] != 0.0
    }
    pub fn bytes(self) -> [u8; 64] {
        let mut bytes = [0; 64];
        for (dst, value) in bytes.chunks_exact_mut(4).zip(self.0.into_iter().flatten()) {
            dst.copy_from_slice(&value.to_ne_bytes());
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;
    #[test]
    fn affine_normals_are_inverse_transpose_and_numerical_domain_is_explicit() {
        for m in [
            Mat4::IDENTITY,
            Mat4::from_scale(Vec3::new(-2.0, 3.0, 0.5)),
            Mat4::from_rotation_y(0.7) * Mat4::from_scale(Vec3::new(0.3, 2.0, 3.0)),
        ] {
            let n = NormalTransform::new(m);
            assert!(n.valid());
            let actual = glam::Mat3::from_cols_array_2d(&[
                n.0[0][..3].try_into().unwrap(),
                n.0[1][..3].try_into().unwrap(),
                n.0[2][..3].try_into().unwrap(),
            ]);
            let source = Vec3::new(1.0, 2.0, 3.0).normalize();
            let expected = (glam::Mat3::from_mat4(m).inverse().transpose() * source).normalize();
            assert!(((actual * source).normalize() - expected).length() < 1e-6);
            assert_eq!(n.0[3][0], m.determinant().signum());
        }
        for m in [
            Mat4::ZERO,
            Mat4::from_scale(Vec3::new(1e-12, 1.0, 1.0)),
            Mat4::perspective_rh(1.0, 1.0, 0.1, 10.0),
        ] {
            assert!(!NormalTransform::new(m).valid());
        }
        assert!(NormalTransform::new(Mat4::from_scale(Vec3::splat(1e-20))).valid());
    }
}
