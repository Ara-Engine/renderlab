//! Independent ray/triangle oracle for the original non-RT visibility resolve.
use glam::{DMat4, DVec3};
use renderlab::{Mat4, Vec3};

/// One actual owner at a deliberately scattered logical binding-array index.
pub struct Batch {
    /// Index shared by the visibility, primary-index and primary-vertex arrays.
    pub index: u32,
    /// Packed cluster positions, triangle corners and optional primitive table.
    pub words: Vec<u32>,
    /// Source primitive indices; lengths are multiples of three (no adjacency).
    pub indices: Vec<u32>,
    /// Three vec4s per source vertex, with position in the first vec4.
    pub vertices: Vec<[f32; 4]>,
}

/// Initial or read-back images in their native, tightly packed texel order.
#[derive(Clone, Default)]
pub struct Outputs {
    /// RGBA32Uint placement+1, primitive and two bitcast f32 barycentrics.
    pub hit: Vec<[u32; 4]>,
    /// RGBA32Float receiver position and epsilon.
    pub world: Vec<[f32; 4]>,
    /// RGBA32Float source-front normal; w is -1 for thin placements.
    pub normal: Vec<[f32; 4]>,
    /// R32Float receiver epsilon.
    pub epsilon: Vec<f32>,
    /// R32Uint coverage flag for each 8x4 tile.
    pub tiles: Vec<u32>,
}

/// Expected images, consumed visibility and a mask separating hits from canaries.
#[derive(Default)]
pub struct Expected {
    /// Outputs after a single resolve with WRITE_RECEIVERS=true and LOD_LIFT=0.
    pub outputs: Outputs,
    /// Cleared consumed hits, unconsumed misses/outside-scene data and tail guards.
    pub visibility: Vec<u64>,
    /// Only true hits write world, normal and epsilon images.
    pub receiver_written: Vec<bool>,
}

/// Raw source-layout inputs, uploaded by their original WGSL group/binding keys.
pub struct Fixture {
    /// Hit/world/normal/epsilon image dimensions; intentionally partial edge tiles.
    pub viewport: [u32; 2],
    /// Coverage image dimensions, ceil(viewport / [8,4]).
    pub tile_size: [u32; 2],
    /// (0,10): 144-byte PrimaryFrame, including translated inverse and scene bounds.
    pub primary_frame: [u32; 36],
    /// (1,0): column-major affine matrices, including a mirrored placement.
    pub models: Vec<[u32; 16]>,
    /// (1,1): padded inverse-transpose columns and winding sign.
    pub normals: Vec<[u32; 16]>,
    /// (1,5): logical batch indices and the second placement's thin bit.
    pub placements: Vec<u32>,
    /// (0,31): original 64-byte VisibilityCluster records.
    pub clusters: Vec<[u32; 16]>,
    /// (0,33): one 32-byte LOD group; nonzero error must not lift at LOD_LIFT=0.
    pub groups: Vec<[u32; 8]>,
    /// (0,30): zero scan policies, excluding adjacency/edge repair from this leaf.
    pub repair_scan: Vec<u32>,
    /// Logical owners for (2,8), (1,3) and (1,2), respectively.
    pub batches: Vec<Batch>,
    /// (0,29): packed visibility, depth high32 and cluster/triangle low32.
    pub visibility: Vec<u64>,
    /// Canary initialization for every output image.
    pub initial: Outputs,
    /// Independent expected result, including untouched image/buffer regions.
    pub expected: Expected,
}

fn matrix(words: &[u32; 16]) -> DMat4 {
    DMat4::from_cols_array(&words.map(|v| f64::from(f32::from_bits(v))))
}

fn close(actual: f32, expected: f32, tolerance: f32, label: &str, pixel: usize) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tolerance,
        "{label} pixel {pixel}: actual {actual}, expected {expected}, tolerance {tolerance}"
    );
}

impl Fixture {
    /// Compare IDs/clears/canaries exactly, and independent f64 geometry with f32
    /// tolerances. Translation by 32 introduces up to one 2^-18 world-coordinate
    /// ULP in the shader's unprojection; barycentrics divide that error by edges.
    pub fn assert_matches(&self, actual: &Outputs, visibility: &[u64]) {
        let expected = &self.expected.outputs;
        assert_eq!(visibility, self.expected.visibility);
        assert_eq!(actual.tiles, expected.tiles);
        assert_eq!(actual.hit.len(), expected.hit.len());
        assert_eq!(actual.world.len(), expected.world.len());
        assert_eq!(actual.normal.len(), expected.normal.len());
        assert_eq!(actual.epsilon.len(), expected.epsilon.len());
        for i in 0..expected.hit.len() {
            if self.expected.receiver_written[i] {
                assert_eq!(actual.hit[i][..2], expected.hit[i][..2], "hit IDs {i}");
                for component in 2..4 {
                    close(
                        f32::from_bits(actual.hit[i][component]),
                        f32::from_bits(expected.hit[i][component]),
                        3e-6,
                        "barycentric",
                        i,
                    );
                }
                for component in 0..3 {
                    close(
                        actual.world[i][component],
                        expected.world[i][component],
                        8e-6,
                        "world",
                        i,
                    );
                    close(
                        actual.normal[i][component],
                        expected.normal[i][component],
                        1e-6,
                        "normal",
                        i,
                    );
                }
                assert_eq!(
                    actual.normal[i][3].to_bits(),
                    expected.normal[i][3].to_bits(),
                    "thin sign {i}"
                );
                assert_eq!(
                    actual.world[i][3].to_bits(),
                    expected.world[i][3].to_bits(),
                    "world epsilon {i}"
                );
                assert_eq!(
                    actual.epsilon[i].to_bits(),
                    expected.epsilon[i].to_bits(),
                    "epsilon {i}"
                );
            } else {
                assert_eq!(actual.hit[i], expected.hit[i], "miss/stale hit {i}");
                assert_eq!(
                    actual.world[i].map(f32::to_bits),
                    expected.world[i].map(f32::to_bits),
                    "untouched world {i}"
                );
                assert_eq!(
                    actual.normal[i].map(f32::to_bits),
                    expected.normal[i].map(f32::to_bits),
                    "untouched normal {i}"
                );
                assert_eq!(
                    actual.epsilon[i].to_bits(),
                    expected.epsilon[i].to_bits(),
                    "untouched epsilon {i}"
                );
            }
        }
    }
}

/// Reconstruct the chosen triangle and intersect its world plane with the camera
/// ray, then solve barycentrics using a Gram matrix in f64. This intentionally
/// differs from WGSL's rebased Moller-Trumbore solve and pinned f32 FMA sequence.
pub fn resolve(fixture: &Fixture) -> Expected {
    let [width, height] = fixture.viewport;
    let mut expected = Expected {
        outputs: fixture.initial.clone(),
        visibility: fixture.visibility.clone(),
        receiver_written: vec![false; (width * height) as usize],
    };
    expected.outputs.tiles.fill(0);
    let inverse = matrix(&std::array::from_fn(|i| fixture.primary_frame[i]));
    let scene = &fixture.primary_frame[24..28];
    for y in scene[1]..scene[3].min(height) {
        for x in scene[0]..scene[2].min(width) {
            let at = (y * width + x) as usize;
            let packed = fixture.visibility[at];
            if packed >> 32 == 0 {
                continue;
            }
            let payload = packed as u32;
            let cluster = fixture.clusters[(payload >> 7) as usize];
            let local = (payload & 127) as usize;
            let placement = cluster[12] as usize;
            let batch = fixture
                .batches
                .iter()
                .find(|b| b.index == fixture.placements[placement] & 0x1fff_ffff)
                .expect("every fixture placement has an actual owner");
            let triangle = batch.words[cluster[7] as usize + local];
            let object: [DVec3; 3] = std::array::from_fn(|corner| {
                let vertex = ((triangle >> (corner * 8)) & 255) as usize;
                let offset = cluster[3] as usize + vertex * 3;
                DVec3::from_array(std::array::from_fn(|axis| {
                    f64::from(f32::from_bits(batch.words[offset + axis]))
                }))
            });
            let model = matrix(&fixture.models[placement]);
            let world = object.map(|p| model.transform_point3(p));
            let ndc = [
                2. * (f64::from(x) + 0.5) / f64::from(width) - 1.,
                1. - 2. * (f64::from(y) + 0.5) / f64::from(height),
            ];
            let near = inverse.project_point3(DVec3::new(ndc[0], ndc[1], 1.));
            let far = inverse.project_point3(DVec3::new(ndc[0], ndc[1], 0.));
            let ray = far - near;
            let ab = world[1] - world[0];
            let ac = world[2] - world[0];
            let plane = ab.cross(ac);
            let t = (world[0] - near).dot(plane) / ray.dot(plane);
            assert!((0.0..=1.0).contains(&t));
            let point = near + ray * t;
            let p = point - world[0];
            let aa = ab.dot(ab);
            let cc = ac.dot(ac);
            let cross = ab.dot(ac);
            let determinant = aa * cc - cross * cross;
            let u = (p.dot(ab) * cc - p.dot(ac) * cross) / determinant;
            let v = (p.dot(ac) * aa - p.dot(ab) * cross) / determinant;
            // Interior hits make all repair branches irrelevant, independently of
            // the valid zero-scan sentinel policy and source-index buffer lengths.
            assert!(u.min(v).min(1. - u - v) > 0.005);
            let normal = matrix(&fixture.normals[placement])
                .transform_vector3((object[1] - object[0]).cross(object[2] - object[0]))
                .normalize();
            let magnitude = world
                .iter()
                .map(|p| p.abs().max_element())
                .fold(0., f64::max) as f32;
            let epsilon = (16. * (f32::from_bits(magnitude.to_bits() + 1) - magnitude)).max(1e-5);
            let primitive = if cluster[11] & 0x10000 != 0 {
                batch.words[cluster[10] as usize + local]
            } else {
                cluster[10] + local as u32
            };
            expected.outputs.hit[at] = [
                placement as u32 + 1,
                primitive,
                (u as f32).to_bits(),
                (v as f32).to_bits(),
            ];
            expected.outputs.world[at] = [point.x as f32, point.y as f32, point.z as f32, epsilon];
            expected.outputs.normal[at] = [
                normal.x as f32,
                normal.y as f32,
                normal.z as f32,
                if fixture.placements[placement] & 0x2000_0000 != 0 {
                    -1.
                } else {
                    1.
                },
            ];
            expected.outputs.epsilon[at] = epsilon;
            expected.receiver_written[at] = true;
            expected.visibility[at] = 0;
            expected.outputs.tiles[((y / 4) * fixture.tile_size[0] + x / 8) as usize] = 1;
        }
    }
    // Covered workgroups overwrite every in-image hit record, even scene misses;
    // empty workgroups preserve all hit/receiver texels. Receivers are hit-only.
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) as usize;
            let tile = ((y / 4) * fixture.tile_size[0] + x / 8) as usize;
            if expected.outputs.tiles[tile] != 0 && !expected.receiver_written[at] {
                expected.outputs.hit[at] = [0; 4];
            }
        }
    }
    expected
}

fn batch(
    index: u32,
    positions: &[[f32; 3]],
    corners: &[[u8; 3]],
    primitives: &[u32],
) -> (Batch, [u32; 16]) {
    let mut words = vec![0xfeed_0000 | index; 5];
    let vertex_offset = words.len() as u32;
    words.extend(positions.iter().flat_map(|p| p.map(f32::to_bits)));
    words.extend([0xbaad_f00d; 2]);
    let triangle_offset = words.len() as u32;
    words.extend(
        corners
            .iter()
            .map(|t| u32::from(t[0]) | u32::from(t[1]) << 8 | u32::from(t[2]) << 16),
    );
    let primitive_offset = words.len() as u32;
    words.extend(primitives);
    let mut indices =
        vec![0; (primitives.iter().copied().max().expect("one primitive") as usize + 1) * 3];
    for (triangle, &primitive) in corners.iter().zip(primitives) {
        indices[primitive as usize * 3..primitive as usize * 3 + 3]
            .copy_from_slice(&triangle.map(u32::from));
    }
    let vertices = positions
        .iter()
        .flat_map(|p| [[p[0], p[1], p[2], 1.], [0., 0., 1., 0.], [0.; 4]])
        .collect();
    let mut cluster = [0; 16];
    cluster[3] = vertex_offset;
    cluster[7] = triangle_offset;
    cluster[9] = 0; // Nonzero group error is ignored with the default zero lift.
    cluster[10] = primitive_offset;
    cluster[11] = 0x10000 | corners.len() as u32 | (positions.len() as u32) << 8;
    (
        Batch {
            index,
            words,
            indices,
            vertices,
        },
        cluster,
    )
}

/// Sparse packed hits cover two batches, primitive tables, a mirrored thin owner,
/// misses, empty tiles, partial edge tiles and visibility/image stale canaries.
pub fn fixture() -> Fixture {
    let viewport = [10, 9];
    let tile_size = [2, 3];
    let transforms = [
        Mat4::from_translation(Vec3::new(32., 0., 0.)),
        Mat4::from_translation(Vec3::new(32.125, -0.25, 0.125))
            * Mat4::from_scale(Vec3::new(-1., 2., 1.)),
    ];
    let (owner3, mut c0) = batch(
        3,
        &[[-1., -1., 0.5], [1., -1., 0.5], [-1., 1., 0.5]],
        &[[0, 1, 2]],
        &[11],
    );
    c0[10] = 11;
    c0[11] &= !0x10000; // First owner exercises linear primitive IDs.
    let (owner37, mut c1) = batch(
        37,
        &[
            [2.125, -0.875, 0.375],
            [-1.875, -0.875, 0.375],
            [-1.875, 1.125, 0.375],
            [2.125, 1.125, 0.375],
        ],
        &[[0, 1, 2], [0, 2, 3]],
        &[7, 2],
    );
    c1[12] = 1;
    let count = (viewport[0] * viewport[1]) as usize;
    let mut visibility = vec![0x1234_5678; count]; // Depth zero does not consume payload.
    for (x, y, cluster, triangle) in [(2, 3, 0, 0), (1, 1, 1, 1), (8, 5, 1, 0), (8, 8, 1, 0)] {
        // Quantized depth is deliberately unrelated to the triangle plane.
        visibility[(y * viewport[0] + x) as usize] =
            (u64::from(0.75f32.to_bits()) << 32) | (cluster << 7) | triangle;
    }
    // Outside the scene rectangle, including an otherwise covered workgroup.
    visibility[2 * viewport[0] as usize] = 0x3f00_0000_ffff_ffff;
    visibility[count - 1] = 0x3f00_0000_eeee_eeee;
    visibility.extend([0xf00d_0001_dead_0001, 0xf00d_0002_dead_0002]);
    let mut primary_frame = [0; 36];
    primary_frame[..16].copy_from_slice(&transforms[0].to_cols_array().map(f32::to_bits));
    primary_frame[24..28].copy_from_slice(&[1, 1, 9, 9]);
    let mut group = [0; 8];
    group[4] = 0.125f32.to_bits();
    let initial = Outputs {
        hit: vec![[0xdead_0001, 0xdead_0002, 0xdead_0003, 0xdead_0004]; count],
        world: vec![[-77., -78., -79., -80.]; count],
        normal: vec![[55., 56., 57., 58.]; count],
        epsilon: vec![42.; count],
        tiles: vec![0xdead_ffff; (tile_size[0] * tile_size[1]) as usize],
    };
    let mut fixture = Fixture {
        viewport,
        tile_size,
        primary_frame,
        models: transforms
            .map(|m| m.to_cols_array().map(f32::to_bits))
            .to_vec(),
        normals: transforms
            .into_iter()
            .map(|m| {
                let mut words = m.inverse().transpose().to_cols_array();
                words[12..16].copy_from_slice(&[m.determinant().signum(), 0., 0., 0.]);
                words.map(f32::to_bits)
            })
            .collect(),
        placements: vec![3, 0x2000_0025],
        clusters: vec![c0, c1],
        groups: vec![group],
        repair_scan: vec![0; 64],
        batches: vec![owner37, owner3],
        visibility,
        initial,
        expected: Expected::default(),
    };
    fixture.expected = resolve(&fixture);
    assert_eq!(fixture.expected.outputs.tiles, [1, 0, 0, 1, 0, 1]);
    assert_eq!(
        fixture
            .expected
            .receiver_written
            .iter()
            .filter(|&&hit| hit)
            .count(),
        4
    );
    fixture
}
