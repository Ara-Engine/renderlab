//! Scalar depth-winning receiver attachments, with translated geometry and mirrors.
use super::{mesh_reference as mesh, reference::Image};
use renderlab::{Mat4, Vec3};

/// All fixture vertices occupy one f32 precision bin, above the epsilon floor.
pub const EPSILON: f32 = 1. / 2048.;

/// Perspective mesh fixture translated away from the origin, with compensated camera.
/// Quarter-pixel edges, reordered instance slots, shear, reflection and the nearer
/// source-back triangle come from the shared bounded scalar raster fixture.
pub fn fixture() -> mesh::Fixture {
    let mut fixture = mesh::fixture();
    let translation = Vec3::new(384., 0., 0.);
    for model in &mut fixture.models {
        *model = Mat4::from_translation(translation) * *model;
    }
    fixture.view_projection *= Mat4::from_translation(-translation);
    fixture.background = [0.; 4];
    check_precision_bin(&fixture);
    fixture
}

/// Both original RGBA32Float attachments and their shared reversed-Z depth target.
pub struct Replayed {
    /// Interpolated world XYZ and the winning triangle's precision epsilon.
    pub world: Image,
    /// Raw world-triangle unit normal and thin-material sign (one or minus one).
    pub normal: Image,
    /// Greater-depth wins, with zero denoting an uncovered pixel.
    pub depth: Vec<f32>,
}

/// Derive triangle precision from f64 bin exponents, independently of WGSL's
/// next-float bit arithmetic. Every triangle in this fixture has the same result.
fn check_precision_bin(fixture: &mesh::Fixture) {
    for &slot in &fixture.slots {
        for indices in fixture.indices.as_chunks::<3>().0.iter() {
            let magnitude = indices
                .iter()
                .map(|&index| {
                    fixture.models[slot as usize]
                        .transform_point3(Vec3::from_array(
                            fixture.vertices[index as usize].position,
                        ))
                        .abs()
                        .max_element() as f64
                })
                .fold(0., f64::max);
            assert!(magnitude.is_finite() && magnitude > 256. && magnitude < 512.);
            let exponent = magnitude.log2().floor() as i32;
            let epsilon = (16. * 2_f64.powi(exponent - 23)).max(1e-5) as f32;
            assert_eq!(epsilon, EPSILON);
        }
    }
}

/// Rasterize both attachments using the shared scalar coverage/interpolation rule.
///
/// One-instance snapshots let this oracle undo the shared helper's source-normal
/// orientation correction: receiver normals retain the raw world triangle order,
/// including reflections. Merging in original slot order with strict greater depth
/// preserves first-winner ties. Tangent streams, clipping and MSAA are outside this
/// bounded single-sample replay; its vertices must remain inside the clip volume.
pub fn replay(fixture: &mesh::Fixture, thin: bool) -> Replayed {
    check_precision_bin(fixture);
    assert_eq!(fixture.background, [0.; 4]);
    let size = fixture.viewport;
    let count = (size[0] * size[1]) as usize;
    let mut result = Replayed {
        world: Image {
            size,
            pixels: vec![[0.; 4]; count],
        },
        normal: Image {
            size,
            pixels: vec![[0.; 4]; count],
        },
        depth: vec![0.; count],
    };
    for &slot in &fixture.slots {
        let single = mesh::Fixture {
            vertices: fixture.vertices.clone(),
            indices: fixture.indices.clone(),
            slots: vec![slot],
            models: fixture.models.clone(),
            normals: fixture.normals.clone(),
            view_projection: fixture.view_projection,
            material: fixture.material,
            base: fixture.base.clone(),
            mr: fixture.mr.clone(),
            viewport: fixture.viewport,
            background: [0.; 4],
        };
        let world = mesh::rasterize_with(&single, |fragment| {
            [
                fragment.world.x,
                fragment.world.y,
                fragment.world.z,
                EPSILON,
            ]
        });
        let determinant_sign = fixture.normals[slot as usize][12];
        assert!(determinant_sign == 1. || determinant_sign == -1.);
        let normal = mesh::rasterize_with(&single, |fragment| {
            let raw = fragment.geometric * determinant_sign;
            [raw.x, raw.y, raw.z, if thin { -1. } else { 1. }]
        });
        assert_eq!(world.depth, normal.depth);
        for pixel in 0..count {
            if world.depth[pixel] > result.depth[pixel] {
                result.depth[pixel] = world.depth[pixel];
                result.world.pixels[pixel] = world.image.pixels[pixel];
                result.normal.pixels[pixel] = normal.image.pixels[pixel];
            }
        }
    }
    assert!(result.depth.iter().any(|&depth| depth > 0.));
    assert!(result.depth.contains(&0.));
    result
}
