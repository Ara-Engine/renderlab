//! Independent f64 segment/triangle reference for the diagnostic primary ray probe.
use glam::{DMat4, DVec3, DVec4};

/// One TLAS instance of the fixture's shared two-primitive BLAS.
pub struct Instance {
    /// Row-major affine object-to-world matrix, matching Ara's AccelerationInstance.
    pub transform: [f32; 12],
    /// Application identity; the hit texture stores this value plus one.
    pub custom_index: u32,
    /// Visibility mask tested against the probe's fixed 255 mask.
    pub mask: u8,
}

/// Tightly packed row-major output images, with f32-rounded expected values.
pub struct Expected {
    /// Instance custom index plus one, primitive index, and two barycentric bit patterns.
    pub hits: Vec<[u32; 4]>,
    /// Reprojected clip Z/W, or exactly zero for a miss.
    pub depths: Vec<f32>,
    /// World XYZ and clip depth when WRITE_WORLD is enabled; misses are all zero.
    pub worlds: Option<Vec<[f32; 4]>>,
}

/// Raw uploads and independent results for original primary_ray_probe.wgsl.
pub struct Fixture {
    /// Original 144-byte Camera: column-major VP, inverse VP, size and two pad words.
    pub camera: Vec<u32>,
    /// Odd output size also exercises padded 8-by-4 dispatch lanes.
    pub size: [u32; 2],
    /// Shared BLAS positions, tightly packed at a 12-byte vertex stride.
    pub vertices: Vec<[f32; 3]>,
    /// Two indexed triangles in one geometry range; primitive IDs are zero and one.
    pub indices: Vec<u32>,
    /// Two real nonidentity instances, including a reflected affine transform.
    pub instances: Vec<Instance>,
    /// Set the original override to this value; false leaves worlds unwritten.
    pub write_world: bool,
    /// Scalar reference images, independent of GPU traversal and shader output.
    pub expected: Expected,
}

impl Fixture {
    /// Original probe workgroup dimensions are 8-by-4-by-1.
    pub fn dispatch(&self) -> [u32; 3] {
        [self.size[0].div_ceil(8), self.size[1].div_ceil(4), 1]
    }

    /// Require exact identities/misses and bounded f32 arithmetic/traversal error.
    ///
    /// World output is checked only when WRITE_WORLD is true. When false, its image
    /// may be absent after override compaction; no write or clear is expected.
    pub fn assert_matches(&self, hits: &[[u32; 4]], depths: &[f32], worlds: Option<&[[f32; 4]]>) {
        assert_eq!(hits.len(), self.expected.hits.len());
        assert_eq!(depths.len(), self.expected.depths.len());
        for (pixel, (actual, expected)) in hits.iter().zip(&self.expected.hits).enumerate() {
            assert_eq!(actual[..2], expected[..2], "hit identity at pixel {pixel}");
            if expected[0] == 0 {
                assert_eq!(*actual, [0; 4], "miss record at pixel {pixel}");
                assert_eq!(depths[pixel], 0., "miss depth at pixel {pixel}");
            } else {
                for channel in 2..4 {
                    close(
                        f32::from_bits(actual[channel]),
                        f32::from_bits(expected[channel]),
                        pixel,
                        "barycentric",
                    );
                }
                close(depths[pixel], self.expected.depths[pixel], pixel, "depth");
            }
        }
        if let Some(expected) = &self.expected.worlds {
            let actual = worlds.expect("WRITE_WORLD requires world readback");
            assert_eq!(actual.len(), expected.len());
            for (pixel, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                if self.expected.hits[pixel][0] == 0 {
                    assert_eq!(*actual, [0.; 4], "miss world at pixel {pixel}");
                } else {
                    for channel in 0..4 {
                        close(actual[channel], expected[channel], pixel, "world");
                    }
                }
            }
        }
    }
}

fn close(actual: f32, expected: f32, pixel: usize, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= 2e-5,
        "{label} at pixel {pixel}: {actual} versus {expected}"
    );
}

fn camera_matrix(words: &[u32]) -> DMat4 {
    DMat4::from_cols_array(&std::array::from_fn(|i| {
        f64::from(f32::from_bits(words[i]))
    }))
}

fn transform(instance: &Instance, point: [f32; 3]) -> DVec3 {
    DVec3::from_array(std::array::from_fn(|row| {
        f64::from(instance.transform[row * 4 + 3])
            + (0..3)
                .map(|column| {
                    f64::from(instance.transform[row * 4 + column]) * f64::from(point[column])
                })
                .sum::<f64>()
    }))
}

/// Intersect the plane first, then solve its two-dimensional Gram system.
/// This uses world-space triangles, avoiding native AS traversal or inverse-ray math.
fn intersect(
    origin: DVec3,
    direction: DVec3,
    length: f64,
    triangle: [DVec3; 3],
) -> Option<[f64; 3]> {
    let [a, b, c] = triangle;
    let ab = b - a;
    let ac = c - a;
    let normal = ab.cross(ac);
    let denominator = normal.dot(direction);
    if denominator.abs() < 1e-14 {
        return None;
    }
    let distance = normal.dot(a - origin) / denominator;
    if !(0. ..=length).contains(&distance) {
        return None;
    }
    let ap = origin + direction * distance - a;
    let aa = ab.length_squared();
    let bb = ab.dot(ac);
    let cc = ac.length_squared();
    let determinant = aa * cc - bb * bb;
    let u = (ap.dot(ab) * cc - ap.dot(ac) * bb) / determinant;
    let v = (ap.dot(ac) * aa - ap.dot(ab) * bb) / determinant;
    (u >= 0. && v >= 0. && u + v <= 1.).then_some([distance, u, v])
}

/// Trace finite near-to-far camera segments against transformed world triangles.
///
/// The probe forces opaque intersections and does not cull either face orientation.
/// The fixed scene avoids shared-edge samples, equal-distance ties, degenerates and
/// near/far-plane intersections; their native boundary rules are outside this oracle.
pub fn reference(fixture: &Fixture) -> Expected {
    assert_eq!(fixture.camera.len(), 36);
    assert_eq!(&fixture.camera[32..34], &fixture.size);
    assert_eq!(fixture.indices.len() % 3, 0);
    let view_projection = camera_matrix(&fixture.camera[..16]);
    let inverse = camera_matrix(&fixture.camera[16..32]);
    let [width, height] = fixture.size;
    let mut hits = Vec::with_capacity((width * height) as usize);
    let mut depths = Vec::with_capacity(hits.capacity());
    let mut worlds = fixture
        .write_world
        .then(|| Vec::with_capacity(hits.capacity()));
    for y in 0..height {
        for x in 0..width {
            let nx = (f64::from(x) + 0.5) / f64::from(width) * 2. - 1.;
            let ny = 1. - (f64::from(y) + 0.5) / f64::from(height) * 2.;
            let near = inverse * DVec4::new(nx, ny, 1., 1.);
            let far = inverse * DVec4::new(nx, ny, 0., 1.);
            let origin = near.truncate() / near.w;
            let delta = far.truncate() / far.w - origin;
            let length = delta.length();
            let direction = delta / length;
            let mut closest: Option<(f64, [u32; 4])> = None;
            for instance in &fixture.instances {
                if instance.mask == 0 {
                    continue;
                }
                for (primitive, indices) in fixture.indices.as_chunks::<3>().0.iter().enumerate() {
                    let triangle = std::array::from_fn(|i| {
                        transform(instance, fixture.vertices[indices[i] as usize])
                    });
                    if let Some([distance, u, v]) = intersect(origin, direction, length, triangle)
                        && closest.as_ref().is_none_or(|(prior, _)| distance < *prior)
                    {
                        closest = Some((
                            distance,
                            [
                                instance.custom_index + 1,
                                primitive as u32,
                                (u as f32).to_bits(),
                                (v as f32).to_bits(),
                            ],
                        ));
                    }
                }
            }
            let (hit, world) = if let Some((distance, hit)) = closest {
                let p = origin + direction * distance;
                let clip = view_projection * p.extend(1.);
                (
                    hit,
                    [p.x as f32, p.y as f32, p.z as f32, (clip.z / clip.w) as f32],
                )
            } else {
                ([0; 4], [0.; 4])
            };
            hits.push(hit);
            depths.push(world[3]);
            if let Some(worlds) = &mut worlds {
                worlds.push(world);
            }
        }
    }
    Expected {
        hits,
        depths,
        worlds,
    }
}

/// Two overlapping triangles and two affine instances under an identity camera.
///
/// Closest-hit counts are 24/6 for custom ID 3's primitives and 25/9 for custom
/// ID 37's primitives, plus 123 misses. Twenty-three rays have multiple candidates.
/// Every triangle edge misses pixel centres by at least 1.67e-4 barycentric units.
pub fn fixture(write_world: bool) -> Fixture {
    let size = [17, 11];
    let mut camera = Vec::with_capacity(36);
    for _ in 0..2 {
        camera.extend(glam::Mat4::IDENTITY.to_cols_array().map(f32::to_bits));
    }
    camera.extend([size[0], size[1], 0, 0]);
    let mut fixture = Fixture {
        camera,
        size,
        vertices: vec![
            [-0.75, -0.625, 0.],
            [0.625, -0.5, 0.],
            [-0.375, 0.75, 0.],
            [-0.5, -0.75, -0.125],
            [0.75, -0.375, -0.125],
            [0.375, 0.625, -0.125],
        ],
        indices: vec![0, 1, 2, 3, 4, 5],
        instances: vec![
            Instance {
                transform: [
                    0.75, 0.125, 0., -0.25, -0.125, 0.875, 0., 0.125, 0., 0., 1., 0.625,
                ],
                custom_index: 3,
                mask: 0x0f,
            },
            Instance {
                transform: [
                    -0.75, 0.125, 0., 0.3125, 0.125, 0.75, 0., -0.125, 0., 0., 0.5, 0.8125,
                ],
                custom_index: 37,
                mask: 0xf0,
            },
        ],
        write_world,
        expected: Expected {
            hits: Vec::new(),
            depths: Vec::new(),
            worlds: None,
        },
    };
    fixture.expected = reference(&fixture);
    fixture
}
