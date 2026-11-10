//! Pixel-centre raster reference shared by diagnostic and programmable mesh shading.
use super::reference::{Image, half, sample};
use glam::{Mat3, Vec2, Vec4};
use renderlab::{Mat4, Vec3, Vertex};

/// The material values read by diagnostic shading; remaining ABI fields use defaults.
#[derive(Clone, Copy, Debug)]
pub struct Material {
    /// Linear RGBA material tint.
    pub base_color: [f32; 4],
    /// Metallic multiplier for mode 2.
    pub metallic: f32,
    /// Roughness multiplier for mode 3.
    pub roughness: f32,
    /// Whether source back faces survive fragment rejection.
    pub double_sided: bool,
}

impl Material {
    /// Exact twelve-word, 48-byte `Material` uniform for the unchanged shader.
    pub fn words(self) -> [u32; 12] {
        [
            self.base_color[0].to_bits(),
            self.base_color[1].to_bits(),
            self.base_color[2].to_bits(),
            self.base_color[3].to_bits(),
            self.metallic.to_bits(),
            self.roughness.to_bits(),
            u32::from(self.double_sided),
            0,
            1f32.to_bits(),
            0,
            0,
            0,
        ]
    }
}

/// Source streams and resources for one indexed, three-instance draw.
pub struct Fixture {
    /// Packed 48-byte source vertex records.
    pub vertices: Vec<Vertex>,
    /// U32 triangle-list indices, shared by all instances.
    pub indices: Vec<u32>,
    /// Model indices in instance-stream order, deliberately distinct from draw order.
    pub slots: Vec<u32>,
    /// Column-major affine model matrices.
    pub models: Vec<Mat4>,
    /// Four padded columns: inverse-transpose XYZ and determinant sign in word 12.
    pub normals: Vec<[f32; 16]>,
    /// Camera projection; camera uniform's final four words hold the selected mode.
    pub view_projection: Mat4,
    /// One material shared by all triangles and instances.
    pub material: Material,
    /// Stored RGBA16Float base-color texels, sampled with linear clamp.
    pub base: Image,
    /// Stored RGBA16Float metallic/roughness texels, sampled with linear clamp.
    pub mr: Image,
    /// Color/depth attachment dimensions.
    pub viewport: [u32; 2],
    /// Color clear; depth clears to zero.
    pub background: [f32; 4],
}

/// Color and reversed depth after one nonblended, single-sample indexed draw.
pub struct Rasterized {
    /// RGBA32Float output, with top-left row-major origin.
    pub image: Image,
    /// Depth32Float values; strictly larger depths win and discarded faces never write.
    pub depth: Vec<f32>,
}

/// Perspective-interpolated inputs at one surviving pixel centre.
#[derive(Clone, Copy, Debug)]
pub struct Fragment {
    /// World position on the rasterized triangle.
    pub world: Vec3,
    /// Interpolated vertex normal before fragment normalization or back-face reversal.
    pub normal: Vec3,
    /// Unit triangle-plane normal in the source orientation, including mirrored models.
    pub geometric: Vec3,
    /// Perspective-correct texture coordinates.
    pub uv: [f32; 2],
    /// Perspective-correct linear vertex color.
    pub color: [f32; 4],
    /// Whether the camera sees the source-front side, after model winding correction.
    pub source_front: bool,
}

#[derive(Clone, Copy)]
struct Projected {
    screen: Vec2,
    reciprocal_w: f32,
    depth: f32,
    world: Vec3,
    normal: Vec3,
    uv: Vec2,
    color: Vec4,
}

/// Scale first to normalize finite vectors without overflowing the squared length.
pub(super) fn unit(value: Vec3) -> Vec3 {
    let scale = value.abs().max_element().max(1e-30);
    let scaled = value / scale;
    scaled / scaled.length_squared().max(1e-30).sqrt()
}

fn edge(a: Vec2, b: Vec2, point: Vec2) -> f32 {
    (b - a).perp_dot(point - a)
}

/// Top and left edges are inclusive for positive-area triangles in Y-down pixels.
fn inclusive(a: Vec2, b: Vec2) -> bool {
    let delta = b - a;
    delta.y < 0. || (delta.y == 0. && delta.x > 0.)
}

/// Render the original diagnostic fragment modes on the shared scalar rasterizer.
pub fn rasterize(fixture: &Fixture, mode: u32) -> Rasterized {
    rasterize_with(fixture, |fragment| {
        let base = sample(&fixture.base, fragment.uv);
        let mr = sample(&fixture.mr, fragment.uv);
        let color = match mode {
            1 => unit(fragment.normal) * 0.5 + Vec3::splat(0.5),
            2 => Vec3::splat(fixture.material.metallic * mr[2]),
            3 => Vec3::splat(fixture.material.roughness * mr[1]),
            _ => Vec3::new(
                fixture.material.base_color[0] * fragment.color[0] * base[0],
                fixture.material.base_color[1] * fragment.color[1] * base[1],
                fixture.material.base_color[2] * fragment.color[2] * base[2],
            ),
        };
        [color.x, color.y, color.z, 1.]
    })
}

/// Rasterize unclipped triangles, shading pixels that pass source-face and depth tests.
///
/// This bounded oracle requires all vertices inside the homogeneous clip volume;
/// it asserts that precondition rather than silently approximating clipped polygons.
/// Coverage uses exact pixel centres and the top-left rule, without a GPU-specific
/// fixed-point subpixel grid. The fixed fixture keeps edges away from pixel centres.
pub fn rasterize_with(
    fixture: &Fixture,
    mut shade: impl FnMut(Fragment) -> [f32; 4],
) -> Rasterized {
    let [width, height] = fixture.viewport;
    assert!(width > 0 && height > 0);
    assert!(fixture.indices.len().is_multiple_of(3));
    let mut result = Rasterized {
        image: Image {
            size: fixture.viewport,
            pixels: vec![fixture.background; (width * height) as usize],
        },
        depth: vec![0.; (width * height) as usize],
    };
    for &slot in &fixture.slots {
        let slot = slot as usize;
        let record = fixture.normals[slot];
        let normal_matrix = Mat3::from_cols(
            Vec3::new(record[0], record[1], record[2]),
            Vec3::new(record[4], record[5], record[6]),
            Vec3::new(record[8], record[9], record[10]),
        );
        let projected: Vec<_> = fixture
            .vertices
            .iter()
            .map(|vertex| {
                let world = fixture.models[slot] * Vec3::from_array(vertex.position).extend(1.);
                let clip = fixture.view_projection * world;
                assert!(
                    clip.w > 0.
                        && clip.z >= 0.
                        && clip.z <= clip.w
                        && clip.x.abs() <= clip.w
                        && clip.y.abs() <= clip.w,
                    "the bounded mesh oracle requires unclipped vertices"
                );
                let ndc = clip.truncate() / clip.w;
                Projected {
                    screen: Vec2::new(
                        (ndc.x * 0.5 + 0.5) * width as f32,
                        (0.5 - ndc.y * 0.5) * height as f32,
                    ),
                    reciprocal_w: 1. / clip.w,
                    depth: ndc.z,
                    world: world.truncate(),
                    normal: unit(normal_matrix * unit(Vec3::from_array(vertex.normal))),
                    uv: Vec2::from_array(vertex.uv),
                    color: Vec4::from_array(vertex.color),
                }
            })
            .collect();
        for triangle in fixture.indices.as_chunks::<3>().0.iter() {
            let mut vertices = [
                projected[triangle[0] as usize],
                projected[triangle[1] as usize],
                projected[triangle[2] as usize],
            ];
            let area = edge(vertices[0].screen, vertices[1].screen, vertices[2].screen);
            if area == 0. {
                continue;
            }
            // A CCW triangle in NDC has negative area after the viewport Y flip.
            let front = area < 0.;
            let source_front = front == (record[12] > 0.);
            if !fixture.material.double_sided && !source_front {
                continue;
            }
            // The derivative cross has this plane's direction, with a viewport-
            // orientation sign. Restoring source orientation also restores mirrors.
            let geometric = unit(
                unit(vertices[1].world - vertices[0].world)
                    .cross(unit(vertices[2].world - vertices[0].world)),
            ) * if record[12] > 0. { 1. } else { -1. };
            if area < 0. {
                vertices.swap(1, 2);
            }
            let [a, b, c] = vertices;
            let area = area.abs();
            let inclusions = [
                inclusive(b.screen, c.screen),
                inclusive(c.screen, a.screen),
                inclusive(a.screen, b.screen),
            ];
            for y in 0..height {
                for x in 0..width {
                    let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let edges = [
                        edge(b.screen, c.screen, point),
                        edge(c.screen, a.screen, point),
                        edge(a.screen, b.screen, point),
                    ];
                    if edges.iter().zip(inclusions).any(|(&distance, inclusive)| {
                        distance < 0. || (distance == 0. && !inclusive)
                    }) {
                        continue;
                    }
                    let barycentric = edges.map(|distance| distance / area);
                    // Post-divide depth is affine in screen space, unlike varyings.
                    let depth = a.depth * barycentric[0]
                        + b.depth * barycentric[1]
                        + c.depth * barycentric[2];
                    let index = (y * width + x) as usize;
                    if depth <= result.depth[index] {
                        continue;
                    }
                    let homogeneous = [
                        barycentric[0] * a.reciprocal_w,
                        barycentric[1] * b.reciprocal_w,
                        barycentric[2] * c.reciprocal_w,
                    ];
                    let sum = homogeneous.iter().sum::<f32>();
                    let weights = homogeneous.map(|weight| weight / sum);
                    let uv = a.uv * weights[0] + b.uv * weights[1] + c.uv * weights[2];
                    let fragment = Fragment {
                        world: a.world * weights[0] + b.world * weights[1] + c.world * weights[2],
                        normal: a.normal * weights[0]
                            + b.normal * weights[1]
                            + c.normal * weights[2],
                        geometric,
                        uv: uv.to_array(),
                        color: (a.color * weights[0] + b.color * weights[1] + c.color * weights[2])
                            .to_array(),
                        source_front,
                    };
                    result.depth[index] = depth;
                    result.image.pixels[index] = shade(fragment);
                }
            }
        }
    }
    result
}

/// Padded inverse-transpose columns and determinant sign for fixture uploads.
pub fn normal_record(model: Mat4) -> [f32; 16] {
    let linear = Mat3::from_mat4(model);
    let inverse_transpose = linear.inverse().transpose();
    let scale = inverse_transpose
        .to_cols_array()
        .into_iter()
        .map(f32::abs)
        .fold(0., f32::max);
    let matrix = (inverse_transpose / scale).to_cols_array();
    let mut record = [0.; 16];
    for column in 0..3 {
        for row in 0..3 {
            record[column * 4 + row] = matrix[column * 3 + row];
        }
    }
    record[12] = linear.determinant().signum();
    record
}

/// Quarter-pixel source edges, perspective-varying W, mirror and overlapping instances.
/// A nearer source-back triangle becomes visible only when double-sided is enabled.
pub fn fixture() -> Fixture {
    let viewport = [32, 24];
    // Screen coordinates prescribe stable coverage while independent W values make
    // affine interpolation of UV, color or normals observably incorrect.
    let points = [
        ([3.25, 4.25], 2.),
        ([3.25, 17.25], 4.),
        ([18.25, 17.25], 3.),
        ([18.25, 4.25], 2.5),
        ([7.25, 8.25], 1.5),
        ([13.25, 8.25], 1.75),
        ([10.25, 14.25], 2.),
    ];
    let vertices = points
        .into_iter()
        .enumerate()
        .map(|(i, ([x, y], w))| {
            let k = i as f32;
            Vertex {
                position: [
                    (2. * x / viewport[0] as f32 - 1.) * w,
                    (1. - 2. * y / viewport[1] as f32) * w,
                    -w,
                ],
                normal: Vec3::new(0.15 + 0.09 * k, -0.3 + 0.07 * k, 1.)
                    .normalize()
                    .to_array(),
                uv: [
                    [-0.2, -0.1],
                    [0.1, 1.2],
                    [1.15, 0.85],
                    [0.9, 0.05],
                    [0.2, 0.2],
                    [0.8, 0.35],
                    [0.4, 0.9],
                ][i],
                color: [
                    0.2 + 0.11 * k,
                    0.9 - 0.09 * k,
                    0.35 + 0.06 * k,
                    0.4 + 0.08 * k,
                ],
            }
        })
        .collect();
    let models = vec![
        Mat4::IDENTITY,
        Mat4::from_scale(Vec3::new(-1.25, 1.25, 1.25)),
        Mat4::from_cols(
            Vec4::new(0.375, 0., 0., 0.),
            Vec4::new(0., 0.375, 0., 0.),
            Vec4::new(-0.28125, -0.1875, 0.75, 0.),
            Vec4::W,
        ),
    ];
    let normals = models.iter().copied().map(normal_record).collect();
    let texture = |metallic: bool| Image {
        size: [4, 3],
        pixels: (0..12)
            .map(|index| {
                let x = (index % 4) as f32 / 3.;
                let y = (index / 4) as f32 / 2.;
                half(if metallic {
                    [0.1, 0.15 + 0.7 * y + 0.05 * x, 0.2 + 0.65 * x - 0.1 * y, 1.]
                } else {
                    [0.1 + 0.8 * x, 0.15 + 0.7 * y, 0.2 + 0.4 * x * y, 0.8]
                })
            })
            .collect(),
    };
    Fixture {
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6],
        slots: vec![1, 0, 2],
        models,
        normals,
        view_projection: Mat4::from_cols(
            Vec4::X,
            Vec4::Y,
            Vec4::new(0., 0., 0., -1.),
            Vec4::new(0., 0., 0.5, 0.),
        ),
        material: Material {
            base_color: [0.8, 0.65, 0.9, 0.7],
            metallic: 0.7,
            roughness: 0.85,
            double_sided: false,
        },
        base: texture(false),
        mr: texture(true),
        viewport,
        background: [0.015625, 0.03125, 0.0625, 1.],
    }
}
