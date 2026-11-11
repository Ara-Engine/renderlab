//! Scalar bounded-ray visibility for the original 256/8-direction fresh estimator.
use glam::{DMat4, DVec3, DVec4};

/// Source-layout receiver images, one light/view, and the independently intersected mesh.
pub struct Fixture {
    /// Odd dimensions exercise partial 2x2 and 8x4 dispatch tiles.
    pub size: [u32; 2],
    /// RGBA32Float receiver world XYZ and strictly positive precision epsilon.
    pub world: Vec<[f32; 4]>,
    /// Unit geometric normal XYZ; W is opaque1, thin-1, or uncovered0.
    pub normal: Vec<[f32; 4]>,
    /// Original LightingUniform48B, with the eye behind the receiving plane.
    pub lighting: [u32; 12],
    /// Original Light80B: travel-Z, view0, and zero/one enabled cascade.
    pub light: [u32; 20],
    /// Original ShadowView112B; clip XY is world/8, clip Z is0.75-world.z/8.
    pub view: [u32; 28],
    /// Two-triangle rectangular opaque blocker at Z2, X[0,3], Y[-3,3].
    pub vertices: Vec<[f32; 3]>,
    /// Triangle-list indices for the blocker's identity-transform instance.
    pub indices: Vec<u32>,
}

fn put(words: &mut [u32], offset: usize, values: &[f32]) {
    for (word, value) in words[offset..].iter_mut().zip(values) {
        *word = value.to_bits();
    }
}

/// Fixed lit/blocked/penumbra receivers, uncovered and skipped pixels, and a
/// close-to-blocker opaque/thin pair whose opposite bias changes visibility.
pub fn fixture(slope: f32, enabled: bool) -> Fixture {
    assert!(slope.is_finite() && (0.0..=0.25).contains(&slope));
    let size = [7, 5];
    let epsilon = 1. / 8192.;
    let mut world = Vec::new();
    let mut normal = Vec::new();
    for y in [-0.75, -0.25, 0.25, 0.75, 1.25] {
        for x in [-1.25, -0.5, -0.125, 0.025, 0.15, 0.5, 1.25] {
            world.push([x, y, 0., epsilon]);
            normal.push([
                0.,
                0.,
                1.,
                if world.len().is_multiple_of(2) {
                    -1.
                } else {
                    1.
                },
            ]);
        }
    }
    // Pixel6 would otherwise be fully blocked, so uncovered handling is observable.
    normal[0][3] = 0.;
    normal[6][3] = 0.;
    world[1] = [12., 0., 0., epsilon]; // Outside the light-space XY box.
    world[2] = [1., 0., 4., epsilon]; // Inside the box, outside camera-depth coverage.
    for (pixel, sign) in [(28, 1.), (29, -1.)] {
        world[pixel] = [1., 0., 2. - 1. / 16384., epsilon];
        normal[pixel] = [0., 0., 1., sign];
    }
    let mut lighting = [0; 12];
    put(&mut lighting, 0, &[0., 0., -4.]);
    lighting[3] = 1;
    lighting[7] = 1f32.to_bits();
    let mut light = [0; 20];
    put(&mut light, 4, &[0., 0., -1., 0.]);
    put(&mut light, 8, &[1., 1., 1., 0.]);
    light[14] = u32::from(enabled);
    let mut view = [0; 28];
    put(
        &mut view,
        0,
        &[
            0.125, 0., 0., 0., 0., 0.125, 0., 0., 0., 0., -0.125, 0., 0., 0., 0.75, 1.,
        ],
    );
    put(&mut view, 16, &[-1., 3., 0., 0.]);
    put(&mut view, 20, &[0., 0., 1., 0.]);
    put(&mut view, 24, &[1. / 512., 0.125, slope, 0.]);
    Fixture {
        size,
        world,
        normal,
        lighting,
        light,
        view,
        vertices: vec![[0., -3., 2.], [3., -3., 2.], [3., 3., 2.], [0., 3., 2.]],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn hash(mut value: u32) -> u32 {
    value = (value ^ (value >> 16)).wrapping_mul(0x7feb_352d);
    value = (value ^ (value >> 15)).wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn vec3(words: &[u32]) -> DVec3 {
    DVec3::new(
        f32::from_bits(words[0]).into(),
        f32::from_bits(words[1]).into(),
        f32::from_bits(words[2]).into(),
    )
}

/// Preserve the estimator's f32 Kronecker coordinates, then evaluate the cone in
/// f64. This is a mathematical sequence, not a call to the generated GPU program.
fn direction(index: u32, pixel: u32, slope: f32) -> DVec3 {
    let seed = hash(pixel);
    let shift = [seed >> 8, hash(seed ^ 0xa511_e9b3) >> 8].map(|bits| bits as f32 / 16_777_216.);
    let steps = [0.754_877_7_f32, 0.569_840_3_f32];
    let r: [f32; 2] =
        std::array::from_fn(|axis| (index as f32 * steps[axis] + shift[axis]).fract());
    let radius = f64::from(slope) * f64::from(r[0]).sqrt();
    let angle = std::f64::consts::TAU * f64::from(r[1]);
    DVec3::new(radius * angle.cos(), radius * angle.sin(), 1.).normalize()
}

/// Two-sided Möller–Trumbore intersection in f64, bounded by the caller's segment.
fn blocked(fixture: &Fixture, origin: DVec3, ray: DVec3, minimum: f64, maximum: f64) -> bool {
    fixture.indices.as_chunks::<3>().0.iter().any(|indices| {
        let [a, b, c] =
            indices.map(|index| DVec3::from_array(fixture.vertices[index as usize].map(f64::from)));
        let ab = b - a;
        let ac = c - a;
        let p = ray.cross(ac);
        let determinant = ab.dot(p);
        if determinant.abs() < 1e-14 {
            return false;
        }
        let offset = origin - a;
        let u = offset.dot(p) / determinant;
        let q = offset.cross(ab);
        let v = ray.dot(q) / determinant;
        let distance = ac.dot(q) / determinant;
        u >= 0. && v >= 0. && u + v <= 1. && distance >= minimum && distance <= maximum
    })
}

/// Evaluate every pixel independently; no workgroups, atomics or GPU hit records.
/// Covered skips and uncovered pixels are256, while8 directions are scaled by32.
/// The finite fixture stays away from external blocker edges at sampled rays;
/// exact integer comparison is intended. Tangential rays, arbitrary transforms,
/// multiple cascades, and general projection matrices are outside this oracle.
pub fn visibility(fixture: &Fixture, directions: u32) -> Vec<u32> {
    assert!(directions == 8 || directions == 256);
    let count = (fixture.size[0] * fixture.size[1]) as usize;
    assert_eq!(fixture.world.len(), count);
    assert_eq!(fixture.normal.len(), count);
    assert_eq!(fixture.light[13], 0);
    assert!(fixture.light[14] <= 1);
    assert_eq!(fixture.indices.len(), 6);
    assert!(
        fixture
            .world
            .iter()
            .flatten()
            .chain(fixture.normal.iter().flatten())
            .all(|v| v.is_finite())
    );
    let matrix = DMat4::from_cols_array(&std::array::from_fn(|i| {
        f64::from(f32::from_bits(fixture.view[i]))
    }));
    let eye = vec3(&fixture.lighting[..3]);
    let travel = vec3(&fixture.light[4..7]);
    assert_eq!(travel, DVec3::NEG_Z);
    let coverage = [fixture.view[16], fixture.view[17]].map(|v| f64::from(f32::from_bits(v)));
    let camera_depth = DVec4::from_array(std::array::from_fn(|i| {
        f64::from(f32::from_bits(fixture.view[20 + i]))
    }));
    let slope = f32::from_bits(fixture.view[26]);
    let output: Vec<_> = fixture
        .world
        .iter()
        .zip(&fixture.normal)
        .enumerate()
        .map(|(pixel, (world, normal))| {
            let world_position =
                DVec3::from_array([world[0].into(), world[1].into(), world[2].into()]);
            let geometric =
                DVec3::from_array([normal[0].into(), normal[1].into(), normal[2].into()]);
            let epsilon = f64::from(world[3]);
            assert_eq!(geometric, DVec3::Z);
            assert!(epsilon > 0. && [-1., 0., 1.].contains(&normal[3]));
            let depth = camera_depth.dot(world_position.extend(1.));
            if normal[3] == 0.
                || fixture.light[14] == 0
                || depth < coverage[0]
                || depth > coverage[1]
            {
                return 256;
            }
            let thin = normal[3] < 0.;
            let toward = if thin { DVec3::Z } else { eye - world_position };
            let sign = if geometric.dot(toward) >= 0. { 1. } else { -1. };
            // Powers-of-two epsilon and axis-aligned normals make this f32 rounding exact.
            let origin = DVec3::from_array(std::array::from_fn(|axis| {
                (world[axis] + (sign * geometric[axis] * epsilon) as f32) as f64
            }));
            let projected = matrix * origin.extend(1.);
            if projected.x.abs() > 1.
                || projected.y.abs() > 1.
                || !(0.0..=1.0).contains(&projected.z)
            {
                return 256;
            }
            let mut visible = 0;
            for sample in 0..directions {
                let ray = direction(sample, pixel as u32, slope);
                // For this +Z cone, the thin bias direction never changes sign.
                assert!(ray.z > 0.);
                let delta = matrix * ray.extend(0.);
                let mut maximum = projected.z / -delta.z;
                for axis in 0..2 {
                    if delta[axis] > 0. {
                        maximum = maximum.min((1. - projected[axis]) / delta[axis]);
                    }
                    if delta[axis] < 0. {
                        maximum = maximum.min((-1. - projected[axis]) / delta[axis]);
                    }
                }
                let plane_distance = (2. - origin.z) / ray.z;
                if plane_distance >= epsilon && plane_distance <= maximum {
                    let hit = origin + ray * plane_distance;
                    let exterior_margin = [
                        hit.x.abs(),
                        (hit.x - 3.).abs(),
                        (hit.y - 3.).abs(),
                        (hit.y + 3.).abs(),
                    ]
                    .into_iter()
                    .fold(f64::INFINITY, f64::min);
                    assert!(
                        exterior_margin > 5e-5,
                        "sample is too close to a blocker silhouette"
                    );
                }
                if maximum <= epsilon || !blocked(fixture, origin, ray, epsilon, maximum) {
                    visible += 1;
                }
            }
            visible * (256 / directions)
        })
        .collect();
    assert!(output.iter().all(|&count| count <= 256));
    for pixel in [0, 1, 2, 6] {
        assert_eq!(output[pixel], 256);
    }
    if fixture.light[14] != 0 {
        assert!(output.contains(&0) && output.contains(&256));
        assert_eq!(
            (output[28], output[29]),
            (0, 256),
            "opaque/thin bias must differ"
        );
        if slope > 0. {
            assert!(output.iter().any(|&value| value > 0 && value < 256));
        }
    } else {
        assert!(output.iter().all(|&value| value == 256));
    }
    output
}
