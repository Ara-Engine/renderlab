//! Geometric edge-check reference for interior, missing-neighbour and planar ties.
use glam::{DMat4, DVec3, DVec4};
use renderlab::{Mat4, Vec3};
use std::collections::BTreeSet;

/// One actual geometry owner among the original 64 logical array entries.
pub struct Batch {
    /// Scattered logical binding index, independent of vector position.
    pub index: u32,
    /// Three vec4s per source vertex; XYZ of the first vec4 is its position.
    pub vertices: Vec<[u32; 4]>,
    /// Original triangle indices, optionally followed by adjacency and padding.
    pub indices: Vec<u32>,
}

/// Exact repair words and unordered retry-index semantics.
pub struct Expected {
    /// Unmodified repair header, selectively changed active items and untouched guards.
    pub repairs: Vec<u32>,
    /// Indirect dispatch X/Y/Z followed by retry count.
    pub retry_header: [u32; 4],
    /// Repair-item indices to retry; atomic append order is intentionally unspecified.
    pub retry_set: BTreeSet<u32>,
}

/// Raw source-layout fixture for the composer's explicitly ray-free check kernel.
pub struct Fixture {
    /// Factory-specific PrimaryFrame, 128 bytes rather than the ordinary 144 bytes.
    pub frame: [u32; 32],
    /// Two current 64-byte placement transforms, including exact translation.
    pub models: Vec<[u32; 16]>,
    /// Two padded normal matrices with winding in fourth-column X.
    pub normals: Vec<[u32; 16]>,
    /// Logical batch indices in low 29 bits, with real high placement flags.
    pub placements: Vec<u32>,
    /// Owners at logical indices 37 and 3; all other references may share a dummy.
    pub batches: Vec<Batch>,
    /// 16-byte image/count header, six 36-byte active items and three guard items.
    pub repairs: Vec<u32>,
    /// Zero 16-byte retry header, then nine initialized index words.
    pub retries: Vec<u32>,
    /// Two 32-lane workgroups, deliberately exceeding the six active items.
    pub dispatch: [u32; 3],
    /// Expected records, queue arguments and unordered retry indices.
    pub expected: Expected,
}

impl Fixture {
    /// Check exact words for this dyadic fixture without imposing retry append order.
    pub fn assert_matches(&self, repairs: &[u32], retries: &[u32]) {
        assert_eq!(repairs, self.expected.repairs.as_slice());
        assert_eq!(retries.len(), self.retries.len());
        assert_eq!(&retries[..4], self.expected.retry_header.as_slice());
        let count = self.expected.retry_set.len();
        let indices: BTreeSet<_> = retries[4..4 + count].iter().copied().collect();
        assert_eq!(indices.len(), count, "duplicate retry indices");
        assert_eq!(indices, self.expected.retry_set);
        assert_eq!(&retries[4 + count..], &self.retries[4 + count..]);
    }
}

fn matrix(words: &[u32]) -> DMat4 {
    DMat4::from_cols_array(&std::array::from_fn(|i| {
        f64::from(f32::from_bits(words[i]))
    }))
}

fn triangle(batch: &Batch, primitive: u32, model: DMat4) -> [DVec3; 3] {
    std::array::from_fn(|corner| {
        let index = batch.indices[primitive as usize * 3 + corner] as usize;
        let words = batch.vertices[index * 3];
        model.transform_point3(DVec3::new(
            f64::from(f32::from_bits(words[0])),
            f64::from(f32::from_bits(words[1])),
            f64::from(f32::from_bits(words[2])),
        ))
    })
}

fn record_point(positions: [DVec3; 3], weights: [f64; 3]) -> DVec3 {
    positions.into_iter().zip(weights).map(|(p, w)| p * w).sum()
}

/// Test rays are orthographic and independent of the shader's contraction ordering.
fn ray(fixture: &Fixture, pixel: u32) -> (DVec3, DVec3) {
    let width = fixture.repairs[0];
    let height = fixture.repairs[1];
    let ndc_x = (f64::from(pixel % width) + 0.5) / f64::from(width) * 2. - 1.;
    let ndc_y = 1. - (f64::from(pixel / width) + 0.5) / f64::from(height) * 2.;
    let inverse = matrix(&fixture.frame[..16]);
    let near = inverse * DVec4::new(ndc_x, ndc_y, 1., 1.);
    let far = inverse * DVec4::new(ndc_x, ndc_y, 0., 1.);
    let origin = near.truncate() / near.w;
    (origin, (far.truncate() / far.w - origin).normalize())
}

/// Resolve only the fixture's provable geometric classes, not the full repair algorithm.
///
/// Interior weights are far from the epsilon strip. Exact edge hits with absent
/// adjacency delegate unchanged geometry. For coplanar same-side overlap at an
/// exactly shared edge, the source's equal-slope tie rule selects the lower primitive;
/// the camera-edge intersection determines its new barycentrics independently.
/// Concave fold insets, near-edge nonzero weights and competing neighbours are outside
/// this bounded oracle and assert rather than inventing expected repairs.
pub fn reference(fixture: &Fixture) -> Expected {
    assert_eq!(&fixture.retries[..4], &[0; 4]);
    let count = fixture.repairs[3] as usize;
    assert!(fixture.repairs.len() >= 4 + count * 9);
    assert!(fixture.dispatch[0] * fixture.dispatch[1] * 32 >= count as u32);
    assert_eq!(fixture.dispatch[2], 1);
    let mut repairs = fixture.repairs.clone();
    let mut retry_set = BTreeSet::new();
    for index in 0..count {
        let start = 4 + index * 9;
        let item = &fixture.repairs[start..start + 9];
        let placement = (item[1] - 1) as usize;
        let batch_index = fixture.placements[placement] & 0x1fff_ffff;
        let batch = fixture
            .batches
            .iter()
            .find(|batch| batch.index == batch_index)
            .expect("repair placement has a real batch owner");
        let model = matrix(&fixture.models[placement]);
        let positions = triangle(batch, item[2], model);
        let u = f64::from(f32::from_bits(item[3]));
        let v = f64::from(f32::from_bits(item[4]));
        let weights = [1. - u - v, u, v];
        let point = record_point(positions, weights);
        let edges: Vec<_> = weights
            .iter()
            .enumerate()
            .filter_map(|(edge, &w)| (w == 0.).then_some(edge))
            .collect();
        if edges.is_empty() {
            assert!(
                weights.into_iter().all(|w| w >= 0.125),
                "only safely interior points supported"
            );
            continue;
        }
        assert_eq!(edges.len(), 1, "vertex repair is outside this fixture");
        let edge = edges[0];
        let neighbour = if batch.indices.len() % 3 == 1 {
            batch.indices[(batch.indices.len() - 1) / 2 + item[2] as usize * 3 + edge]
        } else {
            u32::MAX
        };
        if neighbour == u32::MAX {
            repairs[start + 8] = 1;
            retry_set.insert(index as u32);
            continue;
        }
        let other = triangle(batch, neighbour, model);
        let a = positions[(edge + 1) % 3];
        let b = positions[(edge + 2) % 3];
        let axis = b - a;
        let normal = (positions[1] - positions[0])
            .cross(positions[2] - positions[0])
            .normalize();
        let other_tip = other
            .into_iter()
            .find(|&p| p != a && p != b)
            .expect("adjacent triangle has an opposite vertex");
        assert!(other.contains(&a) && other.contains(&b));
        assert!(
            normal.dot(other_tip - a).abs() < 1e-12,
            "only coplanar overlaps supported"
        );
        let side = normal.cross(axis);
        assert!(
            side.dot(positions[edge] - a) * side.dot(other_tip - a) > 0.,
            "only same-side overlap supported"
        );
        let (origin, direction) = ray(fixture, item[0]);
        assert!(
            normal.dot(direction) < -0.5,
            "fixture facets face the test ray"
        );
        // Intersect the camera line with the plane, then locate it on the shared edge.
        let intersection = origin + direction * (normal.dot(a - origin) / normal.dot(direction));
        let t = (intersection - a).dot(axis) / axis.length_squared();
        assert!((0. ..=1.).contains(&t));
        let edge_point = a + axis * t;
        assert!(edge_point.distance(point) < 1e-10);
        assert!(edge_point.distance(intersection) < 1e-10);
        assert!(
            neighbour < item[2],
            "fixture tie must select a lower primitive"
        );
        let new_weights = other.map(|p| {
            if p == a {
                1. - t
            } else if p == b {
                t
            } else {
                0.
            }
        });
        repairs[start + 2] = neighbour;
        repairs[start + 3] = (new_weights[1] as f32).to_bits();
        repairs[start + 4] = (new_weights[2] as f32).to_bits();
        repairs[start + 5..start + 8]
            .copy_from_slice(&edge_point.as_vec3().to_array().map(f32::to_bits));
        repairs[start + 8] = 0;
        retry_set.insert(index as u32);
    }
    let retries = retry_set.len() as u32;
    let blocks = retries.div_ceil(32);
    Expected {
        repairs,
        retry_header: [blocks.min(256), blocks.div_ceil(256), 1, retries],
        retry_set,
    }
}

fn vertices(positions: &[[f32; 3]]) -> Vec<[u32; 4]> {
    positions
        .iter()
        .flat_map(|p| {
            [
                [p[0], p[1], p[2], 0.].map(f32::to_bits),
                [0f32, 1., 0., 0.].map(f32::to_bits),
                [1f32; 4].map(f32::to_bits),
            ]
        })
        .collect()
}

fn item(pixel: u32, placement: u32, primitive: u32, u: f32, v: f32, point: [f32; 3]) -> [u32; 9] {
    [
        pixel,
        placement,
        primitive,
        u.to_bits(),
        v.to_bits(),
        point[0].to_bits(),
        point[1].to_bits(),
        point[2].to_bits(),
        0,
    ]
}

/// Six unique pixels: three interiors, one known planar canonicalization, two fallbacks.
/// Batch three's adjacency contains 6*T+1 words; batch 37 has original-only indices.
pub fn fixture() -> Fixture {
    let inverse = Mat4::from_scale_rotation_translation(
        Vec3::new(4.5, 3.5, 2.),
        glam::Quat::IDENTITY,
        Vec3::new(0., 0., -2.),
    );
    let mut frame = [0; 32];
    frame[..16].copy_from_slice(&inverse.to_cols_array().map(f32::to_bits));
    frame[16..20].copy_from_slice(&[
        0.1f32.to_bits(),
        0.2f32.to_bits(),
        0.3f32.to_bits(),
        1f32.to_bits(),
    ]);
    frame[20] = 1;
    frame[24..28].copy_from_slice(&[0, 0, 9, 7]);
    let models = [
        Mat4::IDENTITY,
        Mat4::from_translation(Vec3::new(0.25, -0.25, 0.)),
    ]
    .map(|model| model.to_cols_array().map(f32::to_bits))
    .to_vec();
    let mut normal = Mat4::IDENTITY.to_cols_array();
    normal[12] = 1.;
    normal[15] = 0.;
    let normals = vec![normal.map(f32::to_bits); 2];
    let placements = vec![0xc000_0003, 0x6000_0025];
    let batches = vec![
        Batch {
            index: 37,
            vertices: vertices(&[[-0.25, 3.25, -1.], [-4.25, -0.75, -1.], [3.75, -0.75, -1.]]),
            indices: vec![0, 1, 2],
        },
        Batch {
            index: 3,
            vertices: vertices(&[[-2., 0., -1.], [2., 0., -1.], [0., 2., -1.], [0., 4., -1.]]),
            indices: vec![
                0,
                1,
                3,
                2,
                0,
                1,
                u32::MAX,
                u32::MAX,
                1,
                0,
                u32::MAX,
                u32::MAX,
                0,
            ],
        },
    ];
    let items = [
        item(4 + 2 * 9, 1, 1, 0.25, 0.25, [0., 1., -1.]),
        item(4 + 3 * 9, 1, 1, 0.5, 0.5, [0., 0., -1.]),
        item(4 + 4 * 9, 2, 0, 0.5, 0.5, [0., -1., -1.]),
        item(3 + 4 * 9, 2, 0, 0.625, 0.375, [-1., -1., -1.]),
        item(3 + 3 * 9, 2, 0, 0.5, 0.25, [-1., 0., -1.]),
        item(4 + 9, 1, 0, 0.25, 0.5, [0., 2., -1.]),
    ];
    let mut repairs = vec![9, 7, 1, items.len() as u32];
    repairs.extend(items.into_iter().flatten());
    repairs.extend((0..27).map(|i| 0xdead_0000 | i));
    let mut retries = vec![0; 4];
    retries.extend((0..9).map(|i| 0xcafe_0000 | i));
    let mut fixture = Fixture {
        frame,
        models,
        normals,
        placements,
        batches,
        repairs,
        retries,
        dispatch: [2, 1, 1],
        expected: Expected {
            repairs: Vec::new(),
            retry_header: [0; 4],
            retry_set: BTreeSet::new(),
        },
    };
    fixture.expected = reference(&fixture);
    fixture
}
