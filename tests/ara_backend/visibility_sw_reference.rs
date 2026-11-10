//! Exact integer-coverage reference for bounded software/hardware visibility raster.
use glam::{DMat4, DVec3};
use renderlab::{Mat4, Vec3};

/// One real member of the shader's 64-element logical buffer array.
pub struct Batch {
    /// Logical binding-array index, deliberately independent of owner-vector order.
    pub index: u32,
    /// `U32s.data`, including nonzero word offsets and padding between records.
    pub words: Vec<u32>,
}

/// Original source-layout records for independently replaying either raster path.
pub struct Fixture {
    /// 176-byte Config; info.x is the hardware-list base and info.w packs dimensions.
    pub config: [u32; 44],
    /// 64-byte Cluster records with absolute batch-word offsets.
    pub clusters: Vec<[u32; 16]>,
    /// Column-major affine model matrices: identity, mirror, translation.
    pub models: Vec<[u32; 16]>,
    /// Padded inverse-transpose columns and winding sign, 64 bytes per placement.
    pub normals: Vec<[u32; 16]>,
    /// Batch indices in low 29 bits; bit 30 requests source-side culling.
    pub placements: Vec<u32>,
    /// Shuffled SW cluster indices, then an independent HW list at config.info.x.
    pub lists: Vec<u32>,
    /// Original 128-byte argument buffer; software reads its first four words.
    pub args: [u32; 32],
    /// Software workgroups, including three padding groups after sw_count.
    pub dispatch: [u32; 3],
    /// Original hardware draw arguments: 384 vertices and one instance per cluster.
    pub hardware_args: [u32; 4],
    /// Two actual data owners at logical array indices 3 and 37.
    pub batches: Vec<Batch>,
    /// Visibility source dimensions.
    pub viewport: [u32; 2],
    /// Clear pixel words followed by four guard words outside the image.
    pub visibility: Vec<u64>,
    /// Exact software visibility, including untouched allocation guards.
    pub expected: Vec<u64>,
    /// Exact top-left hardware result for the same unclipped-depth fixture.
    pub hardware_expected: Vec<u64>,
}

impl Fixture {
    /// Check depth bits, cluster/triangle payload and allocation guards exactly.
    pub fn assert_matches(&self, actual: &[u64], hardware: bool) {
        let expected = if hardware {
            &self.hardware_expected
        } else {
            &self.expected
        };
        assert_eq!(actual.len(), expected.len());
        for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                actual, expected,
                "visibility word {index}: depth high32, payload low32"
            );
        }
    }
}

fn matrix(words: &[u32; 16]) -> DMat4 {
    DMat4::from_cols_array(&words.map(|word| f64::from(f32::from_bits(word))))
}

#[derive(Clone, Copy)]
struct Projected {
    subpixel: [i64; 2],
    depth: f32,
}

fn orient(a: [i64; 2], b: [i64; 2], p: [i64; 2]) -> i64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn top_left(a: [i64; 2], b: [i64; 2]) -> bool {
    b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])
}

/// Fold triangle coverage into 64-bit maxima; no workgroups or atomics are emulated.
///
/// Source coordinates/matrices are dyadic and depth is constant within each triangle.
/// f64 projection therefore preserves this fixture's exact snapped vertices. Coverage
/// uses i64 edge determinants; the shader's corresponding i32 values fit without
/// overflow. Flat dyadic depth and power-of-two triangle areas make its f32 depth
/// interpolation exact. Near/eye clipping and nonconstant-depth triangles are excluded.
fn rasterize_path(fixture: &Fixture, hardware: bool) -> Vec<u64> {
    let [width, height] = fixture.viewport;
    assert_eq!(fixture.config[43], width | height << 16);
    assert_eq!(fixture.config[40] as usize, fixture.clusters.len());
    assert!(fixture.visibility.len() >= (width * height) as usize);
    let vp = matrix(&std::array::from_fn(|i| fixture.config[i]));
    let indices: Vec<_> = if hardware {
        assert_eq!(fixture.hardware_args[0], 384);
        assert_eq!(fixture.hardware_args[2..], [0, 0]);
        let base = fixture.config[40] as usize;
        fixture.lists[base..base + fixture.hardware_args[1] as usize].to_vec()
    } else {
        let mut indices = Vec::new();
        for y in 0..fixture.dispatch[1] {
            for x in 0..fixture.dispatch[0] {
                let slot = x + y * 65_535;
                if slot < fixture.args[3] {
                    indices.push(fixture.lists[slot as usize]);
                }
            }
        }
        assert_eq!(fixture.dispatch[2], 1);
        indices
    };
    let mut visibility = fixture.visibility.clone();
    for index in indices {
        let cluster = fixture.clusters[index as usize];
        let placement = cluster[12] as usize;
        let packed = fixture.placements[placement];
        let logical_batch = packed & 0x1fff_ffff;
        let data = &fixture
            .batches
            .iter()
            .find(|batch| batch.index == logical_batch)
            .expect("fixture placement has a real batch owner")
            .words;
        let model = matrix(&fixture.models[placement]);
        let count = ((cluster[11] >> 8) & 255) as usize;
        assert!(count <= 128 && cluster[11] & 255 <= 128);
        let vertices: Vec<_> = (0..count)
            .map(|vertex| {
                let offset = cluster[3] as usize + vertex * 3;
                let local = DVec3::from_array(std::array::from_fn(|axis| {
                    f64::from(f32::from_bits(data[offset + axis]))
                }));
                let clip = vp * (model * local.extend(1.));
                assert!(
                    clip.w > 0. && clip.z > 0. && clip.z < clip.w,
                    "fixture excludes near/far/eye clipping"
                );
                let ndc = clip.truncate() / clip.w;
                let pixel = [
                    (ndc.x * 0.5 + 0.5) * f64::from(width),
                    (0.5 - ndc.y * 0.5) * f64::from(height),
                ];
                let subpixel = pixel.map(|coordinate| {
                    let scaled = coordinate * 256.;
                    assert_eq!(scaled, scaled.round(), "fixture vertices must snap exactly");
                    scaled as i64
                });
                Projected {
                    subpixel,
                    depth: ndc.z as f32,
                }
            })
            .collect();
        for triangle in 0..(cluster[11] & 255) {
            let packed_triangle = data[cluster[7] as usize + triangle as usize];
            let mut points = [
                vertices[(packed_triangle & 255) as usize],
                vertices[((packed_triangle >> 8) & 255) as usize],
                vertices[((packed_triangle >> 16) & 255) as usize],
            ];
            let area = orient(points[0].subpixel, points[1].subpixel, points[2].subpixel);
            if area == 0 {
                continue;
            }
            assert!(area.abs() <= i64::from(i32::MAX));
            let front = area < 0;
            if packed & 0x4000_0000 != 0
                && front != (f32::from_bits(fixture.normals[placement][12]) > 0.)
            {
                continue;
            }
            if area < 0 {
                points.swap(1, 2);
            }
            let [a, b, c] = points;
            assert_eq!(a.depth.to_bits(), b.depth.to_bits());
            assert_eq!(a.depth.to_bits(), c.depth.to_bits());
            assert!(area.unsigned_abs().is_power_of_two());
            let inclusive = [
                top_left(b.subpixel, c.subpixel),
                top_left(c.subpixel, a.subpixel),
                top_left(a.subpixel, b.subpixel),
            ];
            for y in 0..height {
                for x in 0..width {
                    let pixel = [i64::from(x) * 256 + 128, i64::from(y) * 256 + 128];
                    let edges = [
                        orient(b.subpixel, c.subpixel, pixel),
                        orient(c.subpixel, a.subpixel, pixel),
                        orient(a.subpixel, b.subpixel, pixel),
                    ];
                    if edges
                        .iter()
                        .zip(inclusive)
                        .any(|(&edge, inclusive)| edge < 0 || (hardware && edge == 0 && !inclusive))
                    {
                        continue;
                    }
                    // For these constant-depth triangles, barycentric integration
                    // is the same constant, independent of the coverage weights.
                    let depth = a.depth;
                    let word =
                        (u64::from(depth.to_bits()) << 32) | u64::from((index << 7) | triangle);
                    let at = (y * width + x) as usize;
                    visibility[at] = visibility[at].max(word);
                }
            }
        }
    }
    visibility
}

/// Original software inclusive-edge coverage with exact packed depth/ID selection.
pub fn rasterize(fixture: &Fixture) -> Vec<u64> {
    rasterize_path(fixture, false)
}

/// Top-left hardware coverage for the same strictly in-depth-range geometry.
/// A GreaterEqual depth attachment can reject only fragments that cannot win the
/// visibility max, so independently folding all covered candidates gives its result.
pub fn hardware_rasterize(fixture: &Fixture) -> Vec<u64> {
    rasterize_path(fixture, true)
}

/// Append a padded cluster whose supplied coordinates describe desired screen pixels.
#[allow(clippy::too_many_arguments)]
fn append(
    batch: &mut Batch,
    model: Mat4,
    placement: u32,
    viewport: [u32; 2],
    pixels: &[[f32; 2]],
    depth: f32,
    triangles: &[[u8; 3]],
    primitive: u32,
) -> [u32; 16] {
    let inverse = model.inverse();
    let positions: Vec<_> = pixels
        .iter()
        .map(|p| {
            inverse.transform_point3(Vec3::new(
                p[0] / viewport[0] as f32 * 2. - 1.,
                1. - p[1] / viewport[1] as f32 * 2.,
                depth,
            ))
        })
        .collect();
    batch.words.push(0xbaad_f00d);
    let vertices = batch.words.len() as u32;
    batch.words.extend(
        positions
            .iter()
            .flat_map(|p| p.to_array().map(f32::to_bits)),
    );
    batch.words.extend([0xfeed_0001, 0xfeed_0002]);
    let indices = batch.words.len() as u32;
    batch.words.extend(
        triangles
            .iter()
            .map(|t| u32::from(t[0]) | u32::from(t[1]) << 8 | u32::from(t[2]) << 16),
    );
    let low = positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let high = positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    let mut cluster = [0; 16];
    cluster[..3].copy_from_slice(&low.to_array().map(f32::to_bits));
    cluster[3] = vertices;
    cluster[4..7].copy_from_slice(&high.to_array().map(f32::to_bits));
    cluster[7] = indices;
    cluster[9] = u32::MAX;
    cluster[10] = primitive;
    cluster[11] = triangles.len() as u32 | (positions.len() as u32) << 8;
    cluster[12] = placement;
    cluster
}

/// Twelve clusters test batch selection, depth/ID ties, both side modes and clipping.
/// Projected X coordinates are quarter pixels and Y coordinates eighth pixels;
/// horizontal, vertical and diagonal edges miss pixel centres on both raster paths.
pub fn fixture() -> Fixture {
    let viewport = [16, 16];
    let transforms = [
        Mat4::IDENTITY,
        Mat4::from_scale(Vec3::new(-1., 1., 1.)),
        Mat4::from_translation(Vec3::new(0.25, -0.125, 0.)),
    ];
    let models = transforms
        .map(|m| m.to_cols_array().map(f32::to_bits))
        .to_vec();
    let normals = [1f32, -1., 1.]
        .into_iter()
        .map(|winding| {
            let mut record = Mat4::IDENTITY.to_cols_array();
            record[0] = winding;
            record[12] = winding;
            record[15] = 0.;
            record.map(f32::to_bits)
        })
        .collect();
    let placements = vec![0xc000_0003, 0xc000_0025, 0x8000_0003];
    // Physical owner order is deliberately opposite logical index order.
    let mut batches = vec![
        Batch {
            index: 37,
            words: vec![0xdead_0037; 7],
        },
        Batch {
            index: 3,
            words: vec![0xdead_0003; 5],
        },
    ];
    let mut clusters = Vec::new();
    let mut add =
        |owner: usize, placement: usize, pixels: &[[f32; 2]], depth, triangles: &[[u8; 3]]| {
            clusters.push(append(
                &mut batches[owner],
                transforms[placement],
                placement as u32,
                viewport,
                pixels,
                depth,
                triangles,
                200 + clusters.len() as u32 * 11,
            ));
        };
    add(
        1,
        0,
        &[
            [2.25, 2.125],
            [2.25, 10.125],
            [10.25, 10.125],
            [10.25, 2.125],
        ],
        0.5,
        &[[0, 1, 2], [0, 1, 2], [0, 2, 3]],
    );
    add(
        1,
        0,
        &[[4.25, 4.125], [4.25, 8.125], [8.25, 8.125]],
        0.5,
        &[[0, 1, 2]],
    );
    add(
        0,
        1,
        &[[6.25, 6.125], [10.25, 10.125], [6.25, 10.125]],
        0.75,
        &[[0, 1, 2]],
    );
    add(
        1,
        0,
        &[[10.25, 2.125], [14.25, 6.125], [10.25, 6.125]],
        0.875,
        &[[0, 1, 2]],
    );
    add(
        1,
        2,
        &[[10.25, 2.125], [14.25, 6.125], [10.25, 6.125]],
        0.5,
        &[[0, 1, 2]],
    );
    add(
        0,
        1,
        &[[1.25, 10.125], [1.25, 14.125], [5.25, 14.125]],
        0.875,
        &[[0, 1, 2]],
    );
    add(
        0,
        1,
        &[[1.25, 10.125], [5.25, 14.125], [1.25, 14.125]],
        0.375,
        &[[0, 1, 2]],
    );
    add(
        1,
        0,
        &[[-2.75, 8.125], [-2.75, 16.125], [5.25, 16.125]],
        0.625,
        &[[0, 1, 2]],
    );
    add(
        1,
        0,
        &[[11.25, 11.125], [13.25, 13.125], [15.25, 15.125]],
        0.875,
        &[[0, 1, 2]],
    );
    add(
        1,
        0,
        &[[18.25, 2.125], [18.25, 6.125], [22.25, 6.125]],
        0.75,
        &[[0, 1, 2]],
    );
    add(
        1,
        0,
        &[[10.25, 10.125], [10.25, 14.125], [14.25, 14.125]],
        0.25,
        &[[0, 1, 2]],
    );
    add(
        1,
        0,
        &[[6.25, 6.125], [6.25, 10.125], [10.25, 10.125]],
        0.125,
        &[[0, 1, 2]],
    );
    let count = clusters.len() as u32;
    let permutation = [11, 5, 2, 9, 0, 7, 4, 8, 1, 10, 6, 3];
    let lists = permutation
        .into_iter()
        .chain(permutation.into_iter().rev())
        .chain([u32::MAX; 3])
        .collect();
    let mut args = [0; 32];
    args[..8].copy_from_slice(&[count, 1, 1, count, 384, count, 0, 0]);
    let mut config = [0; 44];
    config[..16].copy_from_slice(&Mat4::IDENTITY.to_cols_array().map(f32::to_bits));
    config[16..32].copy_from_slice(&Mat4::IDENTITY.to_cols_array().map(f32::to_bits));
    config[40..44].copy_from_slice(&[count, 0, 0, 16 | 16 << 16]);
    let mut visibility = vec![0; (viewport[0] * viewport[1]) as usize];
    visibility.extend((0..4).map(|i| 0xf00d_0000_0000_0000 | i));
    let mut fixture = Fixture {
        config,
        clusters,
        models,
        normals,
        placements,
        lists,
        args,
        dispatch: [count + 3, 1, 1],
        hardware_args: [384, count, 0, 0],
        batches,
        viewport,
        visibility,
        expected: Vec::new(),
        hardware_expected: Vec::new(),
    };
    fixture.expected = rasterize(&fixture);
    fixture.hardware_expected = hardware_rasterize(&fixture);
    assert_eq!(
        fixture.expected, fixture.hardware_expected,
        "fixture edges must agree across raster paths"
    );
    fixture
}
