//! Independent linear-depth oracle; shares only the primary fixture's raw inputs.
use super::post_primary_reference;
use glam::{DMat4, DVec3};
use renderlab::{Camera, Mat4, Vec3};

/// Four views of the same packed primary hits, including cropping and empty scene.
pub struct Fixture {
    /// Raw hit/coverage/model/placement and scattered vertex/index array owners.
    pub source: post_primary_reference::Fixture,
    /// Original 336-byte TpFrames: forward, reverse, cropped oblique, empty scene.
    pub frames: Vec<[u32; 84]>,
    /// Tightly packed R32Float depths at source.internal dimensions, per frame.
    pub expected: Vec<Vec<f32>>,
}

/// Reconstruct the selected material point in f64 from indexed source vertices,
/// then project its displacement onto the stored forward vector. No post velocity,
/// camera unprojection, previous pose or sibling CPU-render result is used.
pub fn render(source: &post_primary_reference::Fixture, frame: &[u32; 84]) -> Vec<f32> {
    let [width, height] = source.internal;
    assert_eq!(source.hit.len(), (width * height) as usize);
    let vector = |offset| {
        DVec3::from_array(std::array::from_fn(|axis| {
            f64::from(f32::from_bits(frame[offset + axis]))
        }))
    };
    let eye = vector(48);
    let forward = vector(52);
    (0..width * height)
        .map(|pixel| {
            let x = pixel % width;
            let y = pixel / width;
            let scene = &frame[76..80];
            if x < scene[0] || y < scene[1] || x >= scene[2] || y >= scene[3] {
                return 1e30;
            }
            let tile = (y / 4) * source.coverage_extent[0] + x / 8;
            if source.coverage[tile as usize] == 0 {
                return 1e30;
            }
            let hit = source.hit[pixel as usize];
            if hit[0] == 0 {
                return 1e30;
            }
            let placement = (hit[0] - 1) as usize;
            let owner = source.placements[placement] & 0x1fff_ffff;
            let batch = source
                .batches
                .iter()
                .find(|batch| batch.index == owner)
                .expect("primary fixture hit has an actual owner");
            let u = f64::from(f32::from_bits(hit[2]));
            let v = f64::from(f32::from_bits(hit[3]));
            let local: DVec3 = [1. - u - v, u, v]
                .into_iter()
                .enumerate()
                .map(|(corner, weight)| {
                    let index = batch.indices[hit[1] as usize * 3 + corner] as usize;
                    let position = batch.vertices[index * 3];
                    DVec3::from_array(std::array::from_fn(|axis| {
                        f64::from(f32::from_bits(position[axis]))
                    })) * weight
                })
                .sum();
            let model = DMat4::from_cols_array(
                &source.models[placement].map(|word| f64::from(f32::from_bits(word))),
            );
            let world = model.transform_point3(local);
            (world - eye).dot(forward) as f32
        })
        .collect()
}

impl Fixture {
    /// Misses must retain TP_MISS's exact bits. A few metres of geometry allow
    /// 2e-6 absolute f32 interpolation/dot error; the unclamped 0.0005-metre
    /// triangles use 2e-9 so a near-depth floor cannot pass unnoticed.
    pub fn assert_matches(&self, frame: usize, actual: &[f32]) {
        let expected = &self.expected[frame];
        assert_eq!(actual.len(), expected.len());
        for (pixel, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            if expected.to_bits() == 1e30f32.to_bits() {
                assert_eq!(
                    actual.to_bits(),
                    expected.to_bits(),
                    "miss at frame {frame}, pixel {pixel}"
                );
            } else {
                let tolerance = if expected.abs() < 0.001 { 2e-9 } else { 2e-6 };
                assert!(
                    actual.is_finite() && (actual - expected).abs() <= tolerance,
                    "depth at frame {frame}, pixel {pixel}: actual {actual}, expected {expected}, tolerance {tolerance}"
                );
            }
        }
    }
}

fn frame(size: [u32; 2], eye: [f32; 3], forward: [f32; 3], scene: [u32; 4]) -> [u32; 84] {
    let eye = Vec3::from_array(eye);
    let forward = Vec3::from_array(forward);
    let camera = Camera::look_at(eye, eye + forward, Vec3::Y, 0.9, 0.0001, 50.)
        .expect("finite transparency fixture view");
    let projection = camera.matrix(size[0], size[1]);
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward).normalize();
    let mut words = [0; 84];
    for (i, matrix) in [projection, projection.inverse(), Mat4::IDENTITY]
        .into_iter()
        .enumerate()
    {
        words[i * 16..i * 16 + 16].copy_from_slice(&matrix.to_cols_array().map(f32::to_bits));
    }
    for (offset, value) in [
        (48, eye.extend(1.)),
        (52, forward.extend(0.)),
        (56, right.extend(0.)),
        (60, up.extend(0.)),
    ] {
        words[offset..offset + 4].copy_from_slice(&value.to_array().map(f32::to_bits));
    }
    words[64..68].copy_from_slice(
        &[
            size[0] as f32,
            size[1] as f32,
            1. / size[0] as f32,
            1. / size[1] as f32,
        ]
        .map(f32::to_bits),
    );
    words[69] = 1f32.to_bits(); // Valid sun direction; visibility remains disabled.
    words[76..80].copy_from_slice(&scene);
    words
}

/// Both logical owners and reordered corners contribute to 23 full-scene hits.
/// Forward/reverse views retain two tiny signed depths; cropping adds scene misses.
pub fn fixture() -> Fixture {
    let source = post_primary_reference::fixture();
    let size = source.internal;
    assert_eq!(size, [7, 5]);
    let frames = vec![
        frame(size, [0.; 3], [0., 0., -1.], [0, 0, 7, 5]),
        frame(size, [0.; 3], [0., 0., 1.], [0, 0, 7, 5]),
        frame(size, [0.125, -0.25, -2.5], [0.6, 0., -0.8], [1, 1, 6, 4]),
        frame(size, [0.; 3], [0., 0., -1.], [3, 2, 3, 4]),
    ];
    let expected: Vec<_> = frames.iter().map(|frame| render(&source, frame)).collect();
    // Pin the fixture's coverage rather than accidentally weakening it after a
    // shared primary-fixture change. Columns: misses, positive, negative, tiny.
    let counts: Vec<_> = expected
        .iter()
        .map(|depths| {
            let mut counts = [0; 4];
            for &depth in depths {
                if depth.to_bits() == 1e30f32.to_bits() {
                    counts[0] += 1;
                } else {
                    counts[1] += usize::from(depth > 0.);
                    counts[2] += usize::from(depth < 0.);
                    counts[3] += usize::from(depth.abs() < 0.001);
                }
            }
            counts
        })
        .collect();
    assert_eq!(
        counts,
        [[12, 23, 0, 2], [12, 0, 23, 2], [23, 9, 3, 0], [35, 0, 0, 0]]
    );
    Fixture {
        source,
        frames,
        expected,
    }
}
