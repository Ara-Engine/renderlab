//! Independent per-layer coloured transmission for planar, constant-texture panes.
use super::{reference::Image, transparency_glass_reference as glass};
use glam::{DMat4, DVec3};
use half::f16;

/// Each draw owns a separately cleared texture-array layer in the source renderer.
#[derive(Clone, Copy, Debug)]
pub enum Case {
    /// Original pane order and sun perpendicular to the planes.
    Ordered,
    /// Reverse layer placement and use an oblique sun for Schlick's angular term.
    Reversed,
    /// Reverse the mirrored pane's triangles; transmission does not cull it.
    BackFacing,
    /// Packed opacity is zero; the original shader still applies tint and Fresnel.
    ZeroOpacity,
}

/// Original pane inputs and independent white/one-cleared layer snapshots.
pub struct Fixture {
    /// Geometry, transforms, packed materials and one stored half texel per pane.
    /// Its glass-specific lighting/background/expected fields are unused here.
    pub scene: glass::Fixture,
    /// Unit direction toward the sun, packed as the transmission frame's XYZ.
    pub sun: [f32; 3],
    /// Pane indices in output texture-array layer order.
    pub order: Vec<usize>,
    /// Half-rounded RGBA transmission/depth for each independently rendered layer.
    pub expected: Vec<Image>,
    /// Full-float depth attachment after each independently rendered layer.
    pub depth: Vec<Vec<f32>>,
}

/// Reuse the fixed 8x6 plane assets, retaining original zero-opacity semantics.
pub fn fixture(case: Case) -> Fixture {
    let mut scene = glass::fixture(match case {
        Case::BackFacing => glass::Case::BackFacingMirroredPane,
        Case::ZeroOpacity => glass::Case::ZeroOpacity,
        Case::Ordered | Case::Reversed => glass::Case::Ordered,
    });
    scene.panes[0].base.pixels[0][0] = 4.; // HDR input makes RGB clamping observable.
    let sun = if matches!(case, Case::Ordered) {
        [0., 0., 1.]
    } else {
        [0.8, 0., 0.6]
    };
    let order = if matches!(case, Case::Reversed) {
        vec![1, 0]
    } else {
        vec![0, 1]
    };
    let (expected, depth) = replay(&scene, sun, &order);
    Fixture {
        scene,
        sun,
        order,
        expected,
        depth,
    }
}

/// Validate the rectangle and plane from actual transformed indexed geometry.
/// Identity light projection makes world Z the rasterized forward depth.
fn plane(scene: &glass::Fixture, pane: &glass::Pane) -> (DVec3, f32) {
    assert_eq!(pane.vertices.len(), 4);
    assert!(
        pane.indices == [0, 1, 2, 0, 2, 3] || pane.indices == [0, 2, 1, 0, 3, 2],
        "fixture triangles must cover the same quad in either winding"
    );
    let matrix = DMat4::from_cols_array(
        &scene.models[pane.slot as usize].map(|v| f64::from(f32::from_bits(v))),
    );
    let points: Vec<_> = pane
        .vertices
        .iter()
        .map(|v| matrix.transform_point3(DVec3::from_array(v.position.map(f64::from))))
        .collect();
    let [left, top, right, bottom] = pane.bounds;
    assert!(left < right && top < bottom);
    assert!(right <= scene.size[0] && bottom <= scene.size[1]);
    let mut corners = [false; 4];
    for (&point, vertex) in points.iter().zip(&pane.vertices) {
        assert_eq!(vertex.color, pane.vertices[0].color);
        assert_eq!(point.z, points[0].z);
        assert!(point.is_finite() && (0.0..1.0).contains(&point.z));
        let pixel = [
            (point.x * 0.5 + 0.5) * f64::from(scene.size[0]),
            (0.5 - point.y * 0.5) * f64::from(scene.size[1]),
        ];
        let x = if (pixel[0] - f64::from(left)).abs() < 2e-5 {
            0
        } else {
            assert!((pixel[0] - f64::from(right)).abs() < 2e-5);
            1
        };
        let y = if (pixel[1] - f64::from(top)).abs() < 2e-5 {
            0
        } else {
            assert!((pixel[1] - f64::from(bottom)).abs() < 2e-5);
            1
        };
        assert!(!corners[x + y * 2], "repeated rectangle corner");
        corners[x + y * 2] = true;
    }
    assert!(corners.into_iter().all(|present| present));
    let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| points[pane.indices[i] as usize]);
    let normal = (b - a).cross(c - a).normalize();
    assert!(
        normal.is_finite()
            && normal.x == 0.
            && normal.y == 0.
            && (normal.z.abs() - 1.).abs() < 1e-12
    );
    (normal, points[0].z as f32)
}

/// Per-layer planar solution: no depth or color passes between distinct draws.
/// Constant UV samples and vertex color make interpolation immaterial. The source
/// takes abs(normal dot sun), so derivative orientation and source winding agree.
/// Multi-instance winners, varying textures, clipping and nonplanar derivatives
/// are outside this bounded oracle.
fn replay(scene: &glass::Fixture, sun: [f32; 3], order: &[usize]) -> (Vec<Image>, Vec<Vec<f32>>) {
    let sun = DVec3::from_array(sun.map(f64::from));
    assert!(sun.is_finite() && (sun.length_squared() - 1.).abs() < 1e-6);
    let mut images = Vec::new();
    let mut depths = Vec::new();
    for &index in order {
        let pane = &scene.panes[index];
        let (normal, z) = plane(scene, pane);
        assert_eq!(pane.base.size, [1, 1]);
        assert_eq!(pane.base.pixels.len(), 1);
        let base = pane.base.pixels[0].map(|v| f64::from(f16::from_f32(v).to_f32()));
        let material = pane.material.map(|v| f64::from(f32::from_bits(v)));
        let color = pane.vertices[0].color.map(f64::from);
        let sampled: [f64; 4] =
            std::array::from_fn(|channel| base[channel] * material[channel] * color[channel]);
        assert!(sampled.into_iter().all(|v| v.is_finite()));
        let packed = pane.material[11];
        let opacity = f64::from(f16::from_bits(packed as u16).to_f32());
        let ior = f64::from(f16::from_bits((packed >> 16) as u16).to_f32()).max(1.);
        assert!(opacity.is_finite() && ior.is_finite());
        let coverage = (opacity * sampled[3]).clamp(0., 1.);
        let reflectance = ((ior - 1.) / (ior + 1.)).powi(2);
        let cosine = normal.dot(sun).abs().clamp(0., 1.);
        let fresnel = reflectance + (1. - reflectance) * (1. - cosine).powi(5);
        let transmitted: [f32; 4] = std::array::from_fn(|channel| {
            if channel == 3 {
                f16::from_f32(z).to_f32()
            } else {
                let value = (1. - coverage) * (1. - fresnel) * sampled[channel].clamp(0., 1.);
                f16::from_f64(value).to_f32()
            }
        });
        // Zero opacity deliberately does not turn the source's transmission white.
        if opacity == 0. {
            assert!(transmitted[..3].iter().any(|&value| value < 1.));
        }
        let count = (scene.size[0] * scene.size[1]) as usize;
        let mut image = Image {
            size: scene.size,
            pixels: vec![[1.; 4]; count],
        };
        let mut depth = vec![1.; count];
        let [left, top, right, bottom] = pane.bounds;
        for y in top..bottom {
            for x in left..right {
                let pixel = (y * scene.size[0] + x) as usize;
                assert!(z < depth[pixel]);
                image.pixels[pixel] = transmitted;
                depth[pixel] = z;
            }
        }
        images.push(image);
        depths.push(depth);
    }
    (images, depths)
}

impl Fixture {
    /// Exact clears/coverage/alpha, two half ULPs for RGB arithmetic, and bounded
    /// f32 depth error. Every expected entry starts from its own cleared layer.
    pub fn assert_matches(&self, images: &[Image], depth: &[Vec<f32>]) {
        assert_eq!(images.len(), self.expected.len());
        assert_eq!(depth.len(), self.depth.len());
        for (layer, ((actual, expected), (actual_depth, expected_depth))) in images
            .iter()
            .zip(&self.expected)
            .zip(depth.iter().zip(&self.depth))
            .enumerate()
        {
            assert_eq!(actual.size, self.scene.size);
            assert_eq!(actual.pixels.len(), expected.pixels.len());
            assert_eq!(actual_depth.len(), expected_depth.len());
            for (pixel, ((a, e), (&az, &ez))) in actual
                .pixels
                .iter()
                .zip(&expected.pixels)
                .zip(actual_depth.iter().zip(expected_depth))
                .enumerate()
            {
                if ez == 1. {
                    assert_eq!(*a, [1.; 4], "clear layer{layer} pixel{pixel}");
                    assert_eq!(az, 1., "depth clear layer{layer} pixel{pixel}");
                    continue;
                }
                assert!(
                    az.is_finite() && (az - ez).abs() <= 2e-6,
                    "depth layer{layer} pixel{pixel}: {az} versus {ez}"
                );
                assert_eq!(a[3], e[3], "alpha layer{layer} pixel{pixel}");
                for (channel, (&value, &expected_value)) in a[..3].iter().zip(&e[..3]).enumerate() {
                    let half = f16::from_f32(value);
                    assert!(value.is_finite() && value >= 0.);
                    assert_eq!(value, half.to_f32(), "non-half output");
                    assert!(
                        half.to_bits()
                            .abs_diff(f16::from_f32(expected_value).to_bits())
                            <= 2,
                        "RGB layer{layer} pixel{pixel} channel{channel}: {} versus {}",
                        value,
                        expected_value
                    );
                }
            }
        }
    }
}
