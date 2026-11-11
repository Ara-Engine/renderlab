//! CPU skinning oracle, independent of generated shader code and GPU history.
use glam::{Mat3, Mat4, Quat, Vec3};
use renderlab::asset::{Vertex, skin::Influence};

/// Deformed streams in the shader's packed vertex and tangent layouts.
pub struct Deformed {
    /// Position, normal, UV and linear RGBA, in source vertex order.
    pub vertices: Vec<[f32; 12]>,
    /// Orthonormalized tangent XYZ and reflected handedness; empty when absent.
    pub tangents: Vec<[f32; 4]>,
}

/// Fixed source streams, two poses and padded cluster-copy destinations.
pub struct Fixture {
    /// Nine bind-pose vertices, leaving a partial 64-thread workgroup.
    pub vertices: Vec<Vertex>,
    /// Normalized weights and joint indices for every source vertex.
    pub influences: Vec<Influence>,
    /// Unit source tangents orthogonal to their corresponding normals.
    pub tangents: Vec<[f32; 4]>,
    /// Initial and updated joint palettes; each joint is nonsingular affine.
    pub palettes: [Vec<Mat4>; 2],
    /// Destination word offset and source vertex index for each position copy.
    pub mappings: Vec<[u32; 2]>,
    /// Initial cluster words, including padding that must remain untouched.
    pub cluster_words: Vec<u32>,
}

fn normalize_or(value: Vec3, fallback: Vec3) -> Vec3 {
    let squared = value.length_squared();
    if squared > 1e-20 {
        value / squared.sqrt()
    } else {
        fallback
    }
}

/// Scalar minors retain the shader's defined behavior even at a singular blend.
fn singular_normal(linear: Mat3, normal: Vec3) -> Vec3 {
    let [a, d, g, b, e, h, c, f, i] = linear.to_cols_array();
    Vec3::new(
        (e * i - f * h) * normal.x + (f * g - d * i) * normal.y + (d * h - e * g) * normal.z,
        (c * h - b * i) * normal.x + (a * i - c * g) * normal.y + (b * g - a * h) * normal.z,
        (b * f - c * e) * normal.x + (c * d - a * f) * normal.y + (a * e - b * d) * normal.z,
    )
}

/// Deform the bind pose without modifying UVs, colors, ordering or source data.
///
/// Nonsingular normals use inverse transpose rather than the shader's cross-product
/// construction. Scaling by |determinant| preserves its small-vector fallback
/// threshold. Singular blends use scalar minors and retain the source normal when
/// those minors vanish. Tangent output is empty when no source stream is supplied.
pub fn deform(
    vertices: &[Vertex],
    influences: &[Influence],
    palette: &[Mat4],
    tangents: Option<&[[f32; 4]]>,
) -> Deformed {
    assert_eq!(vertices.len(), influences.len());
    if let Some(tangents) = tangents {
        assert_eq!(vertices.len(), tangents.len());
    }
    let mut result = Deformed {
        vertices: Vec::with_capacity(vertices.len()),
        tangents: Vec::with_capacity(tangents.map_or(0, <[_]>::len)),
    };
    for (index, (vertex, influence)) in vertices.iter().zip(influences).enumerate() {
        let joints = influence
            .joints
            .map(|joint| palette[usize::from(joint)].to_cols_array());
        let blended = Mat4::from_cols_array(&std::array::from_fn(|element| {
            (0..4)
                .map(|joint| joints[joint][element] * influence.weights[joint])
                .sum()
        }));
        let position = blended.transform_point3(Vec3::from_array(vertex.position));
        let linear = Mat3::from_mat4(blended);
        let determinant = linear.determinant();
        let sign = if determinant < 0. { -1. } else { 1. };
        let original_normal = Vec3::from_array(vertex.normal);
        let transformed_normal = if determinant == 0. {
            singular_normal(linear, original_normal)
        } else {
            (linear.inverse().transpose() * original_normal) * determinant.abs()
        };
        let normal = normalize_or(transformed_normal, original_normal);
        result.vertices.push([
            position.x,
            position.y,
            position.z,
            normal.x,
            normal.y,
            normal.z,
            vertex.uv[0],
            vertex.uv[1],
            vertex.color[0],
            vertex.color[1],
            vertex.color[2],
            vertex.color[3],
        ]);
        if let Some(tangents) = tangents {
            let [x, y, z, handedness] = tangents[index];
            let original_tangent = Vec3::new(x, y, z);
            let transformed = linear * original_tangent;
            let tangent = normalize_or(
                transformed - normal * transformed.dot(normal),
                original_tangent,
            );
            result
                .tangents
                .push([tangent.x, tangent.y, tangent.z, handedness * sign]);
        }
    }
    result
}

/// Mixed poses include reflection, four-joint weights and two singular blends.
///
/// Vertex 4 blends opposite linear transforms to zero, exercising both source
/// fallbacks. Vertex 5 has a rank-two blend, whose nonzero cofactor normal remains
/// defined. Their constituent joints are all individually invertible in both poses.
pub fn fixture() -> Fixture {
    let positions = [
        [-1.25, 0.5, 0.75],
        [0.25, -0.75, 1.5],
        [1.5, 0.25, -0.5],
        [-0.5, 1.25, 0.25],
        [0.75, 0.5, -1.25],
        [1.25, -1., 0.5],
        [-0.75, -0.25, -0.75],
        [0.5, 1.5, 1.25],
        [-1.5, 0.75, -0.25],
    ];
    let vertices: Vec<_> = positions
        .into_iter()
        .enumerate()
        .map(|(i, position)| {
            let k = i as f32;
            Vertex {
                position,
                normal: Vec3::new(0.35 + 0.04 * k, 0.8 - 0.03 * k, -0.45)
                    .normalize()
                    .to_array(),
                uv: [if i == 0 { -0. } else { k * 0.125 }, 0.9 - k * 0.1],
                color: [0.1 + k * 0.08, 0.8 - k * 0.06, 0.25, 0.3 + k * 0.075],
            }
        })
        .collect();
    let tangents = vertices
        .iter()
        .enumerate()
        .map(|(i, vertex)| {
            let normal = Vec3::from_array(vertex.normal);
            let seed = Vec3::new(0.9, -0.3, 0.65);
            let tangent = (seed - normal * seed.dot(normal)).normalize();
            [
                tangent.x,
                tangent.y,
                tangent.z,
                if i % 2 == 0 { 1. } else { -1. },
            ]
        })
        .collect();
    let influences = [
        ([0, 0, 0, 0], [1., 0., 0., 0.]),
        ([1, 0, 0, 0], [1., 0., 0., 0.]),
        ([0, 1, 0, 0], [0.75, 0.25, 0., 0.]),
        ([0, 1, 2, 3], [0.125, 0.25, 0.375, 0.25]),
        ([3, 4, 0, 0], [0.5, 0.5, 0., 0.]),
        ([3, 5, 0, 0], [0.5, 0.5, 0., 0.]),
        ([1, 2, 0, 0], [0.25, 0.75, 0., 0.]),
        ([4, 0, 0, 0], [1., 0., 0., 0.]),
        ([0, 2, 0, 0], [0.5, 0.5, 0., 0.]),
    ]
    .into_iter()
    .map(|(joints, weights)| Influence { joints, weights })
    .collect();
    let joint = |scale: [f32; 3], axis: Vec3, angle, translation: [f32; 3]| {
        Mat4::from_scale_rotation_translation(
            Vec3::from_array(scale),
            Quat::from_axis_angle(axis.normalize(), angle),
            Vec3::from_array(translation),
        )
    };
    let palettes = [
        vec![
            joint(
                [1.4, 0.7, 1.1],
                Vec3::new(1., 2., -1.),
                0.37,
                [0.5, -0.25, 0.75],
            ),
            joint(
                [-0.8, 1.3, 0.6],
                Vec3::new(-1., 1., 2.),
                -0.51,
                [-0.75, 0.5, 0.25],
            ),
            joint(
                [0.9, 1.6, 1.2],
                Vec3::new(2., -1., 1.),
                0.23,
                [0.25, 0.75, -0.5],
            ),
            joint([1., 2., 0.5], Vec3::Y, 0., [1., -0.5, 0.25]),
            joint([-1., -2., -0.5], Vec3::Y, 0., [-0.5, 1., -0.75]),
            joint([-1., 2., 0.5], Vec3::Y, 0., [0.75, 0.25, 1.]),
        ],
        vec![
            joint(
                [0.8, 1.5, 1.2],
                Vec3::new(1., -2., 1.),
                -0.42,
                [-0.25, 0.75, -0.5],
            ),
            joint(
                [-1.2, 0.65, 1.4],
                Vec3::new(2., 1., -1.),
                0.63,
                [1., -0.25, 0.5],
            ),
            joint(
                [1.3, 0.9, 0.75],
                Vec3::new(-1., 2., 1.),
                -0.31,
                [-0.5, -0.75, 0.25],
            ),
            joint([1.5, 0.75, 2.], Vec3::Y, 0., [-0.25, 0.5, 1.25]),
            joint([-1.5, -0.75, -2.], Vec3::Y, 0., [1.25, -0.5, 0.75]),
            joint([-1.5, 0.75, 2.], Vec3::Y, 0., [-0.75, 1.25, -0.5]),
        ],
    ];
    // Out-of-order, duplicated sources and gaps detect incorrect stride/index use.
    let mappings = vec![
        [5, 8],
        [14, 0],
        [23, 5],
        [32, 2],
        [41, 7],
        [50, 4],
        [59, 1],
        [68, 6],
        [77, 3],
        [86, 0],
        [95, 5],
    ];
    let cluster_words = (0..104).map(|i| 0x5a00_0000 | i).collect();
    Fixture {
        vertices,
        influences,
        tangents,
        palettes,
        mappings,
        cluster_words,
    }
}
