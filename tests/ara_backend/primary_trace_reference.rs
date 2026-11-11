//! Independent production primary-trace/receiver snapshots, without edge repair.
use glam::{DMat4, DVec3, DVec4, Mat4};

/// Whether the original query forces opacity or confirms source-facing candidates.
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    /// Both placements are double sided, and primary_frame.sided.x is zero.
    ForcedOpaque,
    /// Placement four is single sided; its source-back primitive must be rejected.
    ConfirmSingleSided,
}

/// The two original production entry points have different output write masks.
#[derive(Clone, Copy, Debug)]
pub enum Entry {
    /// primary_trace writes hit records and epsilon at hits.
    Trace,
    /// primary_trace_receivers writes hit records and world/normal at hits.
    Receivers,
}

/// One distinct mesh owner in each 64-entry vertex/index binding array.
pub struct Batch {
    /// Logical owner index; physical vector order is deliberately unrelated.
    pub index: u32,
    /// Original 48-byte vertices, represented by three vec4 words per vertex.
    pub vertices: Vec<[u32; 4]>,
    /// Reordered primitive corners; six words, with no adjacency extension.
    pub indices: Vec<u32>,
}

/// Native TLAS instance, independent of placement-buffer or owner-vector order.
pub struct Instance {
    /// Logical batch whose actual BLAS must be referenced.
    pub owner: u32,
    /// Row-major 3-by-4 affine object-to-world transform, matching Ara's ABI.
    pub transform: [f32; 12],
    /// Actual placement slot, four or nine; output records add one.
    pub custom_index: u32,
    /// Tested against the query's fixed 255 visibility mask.
    pub mask: u8,
}

/// Complete initialized or resulting image snapshots, in row-major pixel order.
#[derive(Clone)]
pub struct Outputs {
    /// RGBA32Uint primary_hit_output, including stale records in empty tiles.
    pub hit: Vec<[u32; 4]>,
    /// R32Float precision image; receivers leave the entire image untouched.
    pub epsilon: Vec<f32>,
    /// RGBA32Float world XYZ and precision; trace leaves this untouched.
    pub world: Vec<[f32; 4]>,
    /// RGBA32Float geometric normal XYZ and thinness sign; trace leaves this untouched.
    pub normal: Vec<[f32; 4]>,
    /// R32Uint tile coverage, with one pixel per 8-by-4 trace tile.
    pub coverage: Vec<u32>,
}

/// Sparse placement inputs, distinct BLAS owners, and both entry-point snapshots.
pub struct Fixture {
    /// Odd image dimensions, also used to construct the original dispatch grid.
    pub size: [u32; 2],
    /// PrimaryFrame: inverse64, clear16, sided16, scene16, sampling16, reflection16.
    pub frame: [u32; 36],
    /// Ten 64-byte model slots; only four and nine are live.
    pub models: Vec<[u32; 16]>,
    /// Ten 64-byte NormalTransforms, with inverse transpose and determinant info.x.
    pub normals: Vec<[u32; 16]>,
    /// Low 29 bits are batch; bit30 single-sided and bit29 thin, plus tangent bit31.
    pub placements: Vec<u32>,
    /// Physical owner order three then 37, distinct from TLAS and placement order.
    pub batches: Vec<Batch>,
    /// TLAS insertion order nine then four, with custom identities preserved.
    pub instances: Vec<Instance>,
    /// Canaries to upload before independently running each entry point.
    pub initial: Outputs,
    /// Expected result of primary_trace from initial.
    pub expected_trace: Outputs,
    /// Expected result of primary_trace_receivers from initial.
    pub expected_receivers: Outputs,
}

impl Fixture {
    /// Workgroup counts for the original 8-by-4-by-1 production entries.
    pub fn dispatch(&self) -> [u32; 3] {
        [self.size[0].div_ceil(8), self.size[1].div_ceil(4), 1]
    }

    /// Exact coverage, identities and canaries; tolerant floating receiver values.
    pub fn assert_matches(&self, entry: Entry, actual: &Outputs) {
        let expected = match entry {
            Entry::Trace => &self.expected_trace,
            Entry::Receivers => &self.expected_receivers,
        };
        assert_eq!(actual.coverage, expected.coverage);
        assert_eq!(actual.hit.len(), expected.hit.len());
        for (pixel, (actual, expected)) in actual.hit.iter().zip(&expected.hit).enumerate() {
            if *expected == self.initial.hit[pixel] || expected[0] == 0 {
                assert_eq!(actual, expected, "hit/miss/canary at pixel {pixel}");
            } else {
                assert_eq!(actual[..2], expected[..2], "hit identity at pixel {pixel}");
                for i in 2..4 {
                    close(
                        f32::from_bits(actual[i]),
                        f32::from_bits(expected[i]),
                        2e-5,
                        pixel,
                    );
                }
            }
        }
        assert_float_image(
            &actual.epsilon,
            &expected.epsilon,
            &self.initial.epsilon,
            1e-9,
        );
        assert_eq!(actual.world.len(), expected.world.len());
        assert_eq!(actual.normal.len(), expected.normal.len());
        for pixel in 0..expected.world.len() {
            assert_float_image(
                &actual.world[pixel],
                &expected.world[pixel],
                &self.initial.world[pixel],
                2e-5,
            );
            assert_float_image(
                &actual.normal[pixel],
                &expected.normal[pixel],
                &self.initial.normal[pixel],
                2e-5,
            );
        }
    }
}

fn close(actual: f32, expected: f32, tolerance: f32, pixel: usize) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tolerance,
        "float at pixel {pixel}: {actual} versus {expected}, tolerance {tolerance}"
    );
}

fn assert_float_image(actual: &[f32], expected: &[f32], initial: &[f32], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for i in 0..expected.len() {
        if expected[i].to_bits() == initial[i].to_bits() {
            assert_eq!(
                actual[i].to_bits(),
                expected[i].to_bits(),
                "untouched word {i}"
            );
        } else {
            close(actual[i], expected[i], tolerance, i);
        }
    }
}

fn matrix(words: &[u32]) -> DMat4 {
    DMat4::from_cols_array(&std::array::from_fn(|i| {
        f64::from(f32::from_bits(words[i]))
    }))
}

fn object_triangle(batch: &Batch, primitive: usize) -> [DVec3; 3] {
    std::array::from_fn(|corner| {
        let p = batch.vertices[batch.indices[primitive * 3 + corner] as usize * 3];
        DVec3::new(
            f64::from(f32::from_bits(p[0])),
            f64::from(f32::from_bits(p[1])),
            f64::from(f32::from_bits(p[2])),
        )
    })
}

/// World plane intersection followed by a Gram-system solution, using f64 throughout.
fn intersection(origin: DVec3, direction: DVec3, length: f64, p: [DVec3; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = p;
    let ab = b - a;
    let ac = c - a;
    let n = ab.cross(ac);
    let denominator = n.dot(direction);
    if denominator.abs() < 1e-14 {
        return None;
    }
    let distance = n.dot(a - origin) / denominator;
    if !(0. ..=length).contains(&distance) {
        return None;
    }
    let ap = origin + direction * distance - a;
    let aa = ab.length_squared();
    let bb = ab.dot(ac);
    let cc = ac.length_squared();
    let det = aa * cc - bb * bb;
    let u = (ap.dot(ab) * cc - ap.dot(ac) * bb) / det;
    let v = (ap.dot(ac) * aa - ap.dot(ab) * bb) / det;
    (u >= 0. && v >= 0. && u + v <= 1.).then_some([distance, u, v])
}

/// Precision follows the f32 exponent spacing, independently of next-bit subtraction.
fn epsilon(vertices: [DVec3; 3]) -> f32 {
    let maximum = vertices
        .into_iter()
        .flat_map(|v| v.abs().to_array())
        .fold(0., f64::max) as f32;
    let exponent = ((maximum.to_bits() >> 23) & 255) as i32;
    let spacing = if exponent == 0 {
        2f64.powi(-149)
    } else {
        2f64.powi(exponent - 127 - 23)
    };
    (16. * spacing).max(1e-5) as f32
}

struct Hit {
    record: [u32; 4],
    world: [f32; 4],
    normal: [f32; 4],
}

/// CPU proof for well-separated triangle interiors, with original candidate sidedness.
///
/// Receiver points are reconstructed from object-space positions and rounded hit
/// barycentrics, then transformed. Source normal orientation comes from the normal
/// matrix, so a mirrored model does not reverse the declared source-front side.
/// No hit in this fixture enters the precision-width repair strip: this function
/// checks its physical edge clearance and refuses unsupported edge-repair scenes.
pub fn reference(fixture: &Fixture, entry: Entry) -> Outputs {
    let [width, height] = fixture.size;
    let inverse = matrix(&fixture.frame[..16]);
    let crop = &fixture.frame[24..28];
    let mut per_pixel: Vec<Option<Hit>> = (0..width * height).map(|_| None).collect();
    let mut outputs = fixture.initial.clone();
    outputs.coverage.fill(0);
    let tile_width = width.div_ceil(8);
    for y in 0..height {
        for x in 0..width {
            if x < crop[0] || y < crop[1] || x >= crop[2] || y >= crop[3] {
                continue;
            }
            let nx = (f64::from(x) + 0.5) / f64::from(width) * 2. - 1.;
            let ny = 1. - (f64::from(y) + 0.5) / f64::from(height) * 2.;
            let a = inverse * DVec4::new(nx, ny, 1., 1.);
            let b = inverse * DVec4::new(nx, ny, 0., 1.);
            let origin = a.truncate() / a.w;
            let delta = b.truncate() / b.w - origin;
            let length = delta.length();
            let direction = delta / length;
            let mut nearest = f64::INFINITY;
            let mut hit = None;
            for instance in &fixture.instances {
                if instance.mask == 0 {
                    continue;
                }
                let slot = instance.custom_index as usize;
                let placement = fixture.placements[slot];
                assert_eq!(placement & 0x1fff_ffff, instance.owner);
                let batch = fixture
                    .batches
                    .iter()
                    .find(|b| b.index == instance.owner)
                    .expect("real BLAS owner");
                assert_eq!(
                    batch.indices.len() % 3,
                    0,
                    "original-only indices have no adjacency"
                );
                let model = matrix(&fixture.models[slot]);
                let object_direction = model.inverse().transform_vector3(direction);
                for primitive in 0..batch.indices.len() / 3 {
                    let object = object_triangle(batch, primitive);
                    let source_normal = (object[1] - object[0]).cross(object[2] - object[0]);
                    if fixture.frame[20] != 0
                        && placement & 0x4000_0000 != 0
                        && source_normal.dot(object_direction) >= 0.
                    {
                        continue;
                    }
                    let vertices = object.map(|p| model.transform_point3(p));
                    let Some([distance, u, v]) = intersection(origin, direction, length, vertices)
                    else {
                        continue;
                    };
                    if distance >= nearest {
                        continue;
                    }
                    let precision = epsilon(vertices);
                    let exact_point = origin + direction * distance;
                    let clearance = (0..3)
                        .map(|i| {
                            let a = vertices[(i + 1) % 3];
                            let axis = vertices[(i + 2) % 3] - a;
                            (exact_point - a).cross(axis).length() / axis.length()
                        })
                        .fold(f64::INFINITY, f64::min);
                    assert!(
                        clearance > 8. * f64::from(precision),
                        "fixture must avoid the repair strip"
                    );
                    let u = u as f32;
                    let v = v as f32;
                    let weights = [1. - f64::from(u) - f64::from(v), f64::from(u), f64::from(v)];
                    let object_point: DVec3 =
                        object.into_iter().zip(weights).map(|(p, w)| p * w).sum();
                    let point = model.transform_point3(object_point);
                    let normal = matrix(&fixture.normals[slot])
                        .transform_vector3(source_normal.normalize())
                        .normalize();
                    nearest = distance;
                    hit = Some(Hit {
                        record: [
                            instance.custom_index + 1,
                            primitive as u32,
                            u.to_bits(),
                            v.to_bits(),
                        ],
                        world: [point.x as f32, point.y as f32, point.z as f32, precision],
                        normal: [
                            normal.x as f32,
                            normal.y as f32,
                            normal.z as f32,
                            if placement & 0x2000_0000 != 0 {
                                -1.
                            } else {
                                1.
                            },
                        ],
                    });
                }
            }
            if hit.is_some() {
                outputs.coverage[(y / 4 * tile_width + x / 8) as usize] = 1;
            }
            per_pixel[(y * width + x) as usize] = hit;
        }
    }
    for y in 0..height {
        for x in 0..width {
            if outputs.coverage[(y / 4 * tile_width + x / 8) as usize] == 0 {
                continue;
            }
            let pixel = (y * width + x) as usize;
            outputs.hit[pixel] = per_pixel[pixel].as_ref().map_or([0; 4], |h| h.record);
            if let Some(hit) = &per_pixel[pixel] {
                match entry {
                    Entry::Trace => outputs.epsilon[pixel] = hit.world[3],
                    Entry::Receivers => {
                        outputs.world[pixel] = hit.world;
                        outputs.normal[pixel] = hit.normal;
                    }
                }
            }
        }
    }
    outputs
}

fn model(rows: [f32; 12]) -> Mat4 {
    Mat4::from_cols_array(&std::array::from_fn(|i| {
        let row = i % 4;
        let column = i / 4;
        if row == 3 {
            if column == 3 { 1. } else { 0. }
        } else {
            rows[row * 4 + column]
        }
    }))
}

fn packed_positions(positions: &[[f32; 3]]) -> Vec<[u32; 4]> {
    positions
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            [
                [p[0], p[1], p[2], 0.25].map(f32::to_bits),
                [0.5, 0.75, i as f32 / 8., 0.25].map(f32::to_bits),
                [0.125, 0.5, 0.75, 1.].map(f32::to_bits),
            ]
        })
        .collect()
}

/// Distinct sloping meshes, sparse slots and shuffled instance/vertex/corner orders.
///
/// No adjacency is supplied. The fixture proves interior traversal and receiver
/// publication, not the unchanged shader's edge probes, folds, or ray-query repair.
pub fn fixture(mode: Mode) -> Fixture {
    let size = [17, 11];
    let batches = vec![
        Batch {
            index: 3,
            // Physical C,F,B,D,A,E; primitive one deliberately has source-back winding.
            vertices: packed_positions(&[
                [-0.3125, 0.6875, -0.0625],
                [0.3125, 0.6875, -0.25],
                [0.6875, -0.4375, 0.125],
                [-0.5625, -0.6875, -0.1875],
                [-0.6875, -0.5625, 0.],
                [0.6875, -0.3125, -0.125],
            ]),
            indices: vec![4, 2, 0, 3, 1, 5],
        },
        Batch {
            index: 37,
            // Physical E,C,D,B,F,A; source-back primitive zero hides front primitive one.
            vertices: packed_positions(&[
                [0.75, -0.375, -0.0625],
                [-0.375, 0.75, 0.125],
                [-0.5, -0.75, -0.125],
                [0.625, -0.5, -0.0625],
                [0.375, 0.625, -0.1875],
                [-0.75, -0.625, 0.0625],
            ]),
            indices: vec![5, 1, 3, 2, 0, 4],
        },
    ];
    let instances = vec![
        Instance {
            owner: 3,
            transform: [
                -0.75, 0.125, 0., 64.3125, 0.125, 0.75, 0., -0.125, 0., 0., 0.5, 0.8125,
            ],
            custom_index: 9,
            mask: 0xf0,
        },
        Instance {
            owner: 37,
            transform: [
                0.75, 0.125, 0., 63.75, -0.125, 0.875, 0., 0.125, 0., 0., 0.5, 0.625,
            ],
            custom_index: 4,
            mask: 0x0f,
        },
    ];
    let mut models = vec![Mat4::IDENTITY.to_cols_array().map(f32::to_bits); 10];
    let mut normals = models.clone();
    let mut placements = vec![0; 10];
    for instance in &instances {
        let slot = instance.custom_index as usize;
        let m = model(instance.transform);
        models[slot] = m.to_cols_array().map(f32::to_bits);
        let mut normal = m.inverse().transpose().to_cols_array();
        normal[3] = 0.;
        normal[7] = 0.;
        normal[11] = 0.;
        normal[12..].copy_from_slice(&[m.determinant().signum(), 0., 0., 0.]);
        normals[slot] = normal.map(f32::to_bits);
        placements[slot] = instance.owner | 0x8000_0000;
    }
    placements[9] |= 0x2000_0000;
    let mut frame = [0; 36];
    frame[..16].copy_from_slice(
        &Mat4::from_translation(glam::Vec3::new(64., 0., 0.))
            .to_cols_array()
            .map(f32::to_bits),
    );
    frame[16..20].copy_from_slice(&[0.125f32, 0.25, 0.5, 1.].map(f32::to_bits));
    frame[24..28].copy_from_slice(&[1, 1, 16, 10]);
    if matches!(mode, Mode::ConfirmSingleSided) {
        frame[20] = 1;
        placements[4] |= 0x4000_0000;
    }
    let pixels = size[0] * size[1];
    let initial = Outputs {
        hit: (0..pixels)
            .map(|p| {
                [
                    0xa000_0000 | p,
                    0xb000_0000 | p,
                    0xc000_0000 | p,
                    0xd000_0000 | p,
                ]
            })
            .collect(),
        epsilon: (0..pixels).map(|p| 1000. + p as f32).collect(),
        world: (0..pixels)
            .map(|p| [2000. + p as f32, 3000., 4000., 5000.])
            .collect(),
        normal: (0..pixels)
            .map(|p| [-2000. - p as f32, -3000., -4000., -5000.])
            .collect(),
        coverage: vec![0xfeed_beef; (size[0].div_ceil(8) * size[1].div_ceil(4)) as usize],
    };
    let mut fixture = Fixture {
        size,
        frame,
        models,
        normals,
        placements,
        batches,
        instances,
        initial: initial.clone(),
        expected_trace: initial.clone(),
        expected_receivers: initial,
    };
    fixture.expected_trace = reference(&fixture, Entry::Trace);
    fixture.expected_receivers = reference(&fixture, Entry::Receivers);
    fixture
}
