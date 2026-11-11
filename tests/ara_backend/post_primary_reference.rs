//! Independent f64 reference for hit-based post velocity and linear view depth.
use glam::{DMat4, DVec2, DVec3, DVec4};
use renderlab::{Camera, Mat4, Vec3};

/// Real data owner for one scattered logical binding-array index.
pub struct Batch {
    /// Logical index, 3 or 37; physical owner order deliberately differs.
    pub index: u32,
    /// Packed vertex words: position begins every third vec4, with no header.
    pub vertices: Vec<[u32; 4]>,
    /// Triangle indices, including nonidentity vertex ordering.
    pub indices: Vec<u32>,
    /// Previous submitted vertex vec4s; skinned data has one extra padding vec4.
    pub previous_vertices: Vec<[u32; 4]>,
}

/// Raw original-source inputs and expected RGBA32Float pixels for four histories.
pub struct Fixture {
    /// Primary hit dimensions, smaller than output.
    pub internal: [u32; 2],
    /// Odd post output dimensions.
    pub output: [u32; 2],
    /// Row-major RGBA32Uint placement/primitive/barycentric hit records.
    pub hit: Vec<[u32; 4]>,
    /// Primary trace-tile coverage dimensions (8x4 pixels per tile).
    pub coverage_extent: [u32; 2],
    /// The final internal row belongs to an uncovered tile with stale nonzero hits.
    pub coverage: Vec<u32>,
    /// PrimaryFrame's 144 bytes, including inverse perspective and sampling/reflection.
    pub primary_frame: [u32; 36],
    /// Exactly four matrices: current placements 0/1, then previous placements 0/1.
    pub models: Vec<[u32; 16]>,
    /// Logical batch indices in low 29 bits; high placement flags must be masked.
    pub placements: Vec<u32>,
    /// Two actual vertex/index/previous-vertex owners, at logical indices 37 and 3.
    pub batches: Vec<Batch>,
    /// 272-byte PostFrames: motion, radius clamp, history off, previous clip behind.
    pub post_frames: Vec<[u32; 68]>,
    /// Names in the same order as post_frames/expected.
    pub frame_names: Vec<&'static str>,
    /// Original 8x8 kernel dispatch workgroup counts.
    pub dispatch: [u32; 3],
    /// Output-space velocity XY, linear depth or 1e8 sky depth, and alpha one.
    pub expected: Vec<Vec<[f32; 4]>>,
}

fn matrix(words: &[u32]) -> DMat4 {
    DMat4::from_cols_array(&std::array::from_fn(|i| {
        f64::from(f32::from_bits(words[i]))
    }))
}

fn position(vertices: &[[u32; 4]], vertex: u32) -> DVec3 {
    let first = vertices[vertex as usize * 3];
    DVec3::new(
        f64::from(f32::from_bits(first[0])),
        f64::from(f32::from_bits(first[1])),
        f64::from(f32::from_bits(first[2])),
    )
}

/// Reconstruct corresponding current/previous material points from independent sums.
fn geometry(fixture: &Fixture, record: [u32; 4]) -> (DVec4, DVec4) {
    let placement = (record[0] - 1) as usize;
    let batch_index = fixture.placements[placement] & 0x1fff_ffff;
    let batch = fixture
        .batches
        .iter()
        .find(|batch| batch.index == batch_index)
        .expect("hit placement has a real batch owner");
    let u = f64::from(f32::from_bits(record[2]));
    let v = f64::from(f32::from_bits(record[3]));
    let weights = [1. - u - v, u, v];
    let mut current = DVec3::ZERO;
    let mut previous = DVec3::ZERO;
    for (corner, weight) in weights.into_iter().enumerate() {
        let index = batch.indices[record[1] as usize * 3 + corner];
        current += position(&batch.vertices, index) * weight;
        previous += position(&batch.previous_vertices, index) * weight;
    }
    let current = matrix(&fixture.models[placement]) * current.extend(1.);
    let previous =
        matrix(&fixture.models[fixture.models.len() / 2 + placement]) * previous.extend(1.);
    (current, previous)
}

/// Unproject a pixel-centre ray through raw inverse-camera data, then discard origin.
fn sky_direction(fixture: &Fixture, pixel: [u32; 2]) -> DVec3 {
    let ndc = DVec2::new(
        (f64::from(pixel[0]) + 0.5) / f64::from(fixture.internal[0]) * 2. - 1.,
        1. - (f64::from(pixel[1]) + 0.5) / f64::from(fixture.internal[1]) * 2.,
    );
    let inverse = matrix(&fixture.primary_frame[..16]);
    let near = inverse * DVec4::new(ndc.x, ndc.y, 1., 1.);
    let far = inverse * DVec4::new(ndc.x, ndc.y, 0., 1.);
    (far.truncate() / far.w - near.truncate() / near.w).normalize()
}

fn screen(clip: DVec4, size: [u32; 2]) -> DVec2 {
    DVec2::new(
        (clip.x / clip.w * 0.5 + 0.5) * f64::from(size[0]),
        (0.5 - clip.y / clip.w * 0.5) * f64::from(size[1]),
    )
}

/// Independent projection/reprojection without reproducing shader FMA ordering.
///
/// Geometry comes from indexed current/previous streams and the two model halves.
/// Missing or uncovered samples use a direction with W=0, so camera translation
/// cannot add sky parallax. Sampling is nearest hit selection at output-pixel
/// centres. All math after decoding source f32 values is f64, then stored as f32;
/// compare velocity/depth numerically rather than requiring arithmetic bit identity.
pub fn render(fixture: &Fixture, post_frame: &[u32; 68]) -> Vec<[f32; 4]> {
    assert_eq!(fixture.models.len(), fixture.placements.len() * 2);
    assert_eq!(
        fixture.hit.len(),
        (fixture.internal[0] * fixture.internal[1]) as usize
    );
    assert_eq!(
        fixture.coverage.len(),
        (fixture.coverage_extent[0] * fixture.coverage_extent[1]) as usize
    );
    let previous = matrix(&post_frame[..16]);
    let current = matrix(&post_frame[16..32]);
    let view = matrix(&post_frame[48..64]);
    let scale = f64::from(f32::from_bits(post_frame[64]));
    let radius = f64::from(f32::from_bits(post_frame[65]));
    let history = f32::from_bits(post_frame[66]) > 0.5;
    (0..fixture.output[0] * fixture.output[1])
        .map(|pixel| {
            let x = pixel % fixture.output[0];
            let y = pixel / fixture.output[0];
            let source = [
                (((f64::from(x) + 0.5) * f64::from(fixture.internal[0])
                    / f64::from(fixture.output[0])) as u32)
                    .min(fixture.internal[0] - 1),
                (((f64::from(y) + 0.5) * f64::from(fixture.internal[1])
                    / f64::from(fixture.output[1])) as u32)
                    .min(fixture.internal[1] - 1),
            ];
            let covered = fixture.coverage
                [((source[1] / 4) * fixture.coverage_extent[0] + source[0] / 8) as usize]
                != 0;
            let record = fixture.hit[(source[1] * fixture.internal[0] + source[0]) as usize];
            let (point, old_point, depth) = if !covered || record[0] == 0 {
                let direction = sky_direction(fixture, source).extend(0.);
                (direction, direction, 1e8)
            } else {
                let (point, old_point) = geometry(fixture, record);
                let depth = (-(view * point).z).max(0.001);
                (point, old_point, depth)
            };
            let old_clip = previous * old_point;
            let mut velocity = DVec2::ZERO;
            if history && old_clip.w > 0. {
                velocity = (screen(current * point, fixture.output)
                    - screen(old_clip, fixture.output))
                    * scale;
                let length = velocity.length();
                velocity *= (radius / length.max(1e-5)).min(1.);
            }
            [velocity.x as f32, velocity.y as f32, depth as f32, 1.]
        })
        .collect()
}

fn vertices(positions: &[[f32; 3]]) -> Vec<[u32; 4]> {
    positions
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            let k = i as f32;
            [
                [p[0], p[1], p[2], 0.2 + k * 0.03].map(f32::to_bits),
                [0.7, 0.6, k * 0.1, 0.2 + k * 0.07].map(f32::to_bits),
                [0.3 + k * 0.05, 0.5, 0.7, 1.].map(f32::to_bits),
            ]
        })
        .collect()
}

fn post_frame(
    current: Camera,
    previous: Mat4,
    size: [u32; 2],
    radius: f32,
    history: bool,
) -> [u32; 68] {
    let matrices = [
        previous,
        current.matrix(size[0], size[1]),
        current.matrix(size[0], size[1]).inverse(),
        current.view(),
    ];
    let mut words = [0; 68];
    for (index, matrix) in matrices.into_iter().enumerate() {
        words[index * 16..index * 16 + 16]
            .copy_from_slice(&matrix.to_cols_array().map(f32::to_bits));
    }
    words[64..].copy_from_slice(&[
        0.5f32.to_bits(),
        radius.to_bits(),
        if history { 1f32.to_bits() } else { 0 },
        0,
    ]);
    words
}

/// Small upsampled-hit replay with rigid and skinned motion, sky and history variants.
pub fn fixture() -> Fixture {
    let internal = [7, 5];
    let output = [13, 9];
    let camera = Camera::look_at(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y, 0.9, 0.0001, 50.)
        .expect("finite fixture camera");
    let old_eye = Vec3::new(0.15, -0.05, 0.1);
    let old_camera = Camera::look_at(
        old_eye,
        old_eye + Vec3::new(0.12, -0.04, -1.),
        Vec3::Y,
        0.9,
        0.0001,
        50.,
    )
    .expect("finite previous fixture camera");
    let backward = Camera::look_at(Vec3::ZERO, Vec3::Z, Vec3::Y, 0.9, 0.0001, 50.)
        .expect("finite reversed fixture camera");
    let mut primary_frame = [0; 36];
    primary_frame[..16].copy_from_slice(
        &camera
            .matrix(internal[0], internal[1])
            .inverse()
            .to_cols_array()
            .map(f32::to_bits),
    );
    primary_frame[16..20].copy_from_slice(&[
        0.1f32.to_bits(),
        0.2f32.to_bits(),
        0.3f32.to_bits(),
        1f32.to_bits(),
    ]);
    primary_frame[20] = 1;
    primary_frame[24..28].copy_from_slice(&[0, 0, internal[0], internal[1]]);
    let rigid = vertices(&[
        [-0.8, -0.4, -3.],
        [0.6, -0.3, -3.5],
        [-0.1, 0.7, -2.75],
        [-0.0001, -0.00008, -0.0005],
        [0.0001, -0.00008, -0.0005],
        [0., 0.0001, -0.0005],
    ]);
    let skinned_positions = [
        [-0.4, -0.2, -2.],
        [0.5, -0.2, -2.4],
        [0.45, 0.55, -2.1],
        [-0.35, 0.6, -2.25],
    ];
    let previous_positions = [
        [-0.3, -0.05, -2.1],
        [0.3, -0.15, -2.2],
        [0.3, 0.45, -2.35],
        [-0.15, 0.4, -2.1],
    ];
    let skinned = vertices(&skinned_positions);
    let mut previous_skin = vertices(&previous_positions);
    previous_skin.push([13f32, 17., 19., 23.].map(f32::to_bits));
    let batches = vec![
        Batch {
            index: 37,
            vertices: skinned,
            indices: vec![2, 0, 1, 3, 0, 2],
            previous_vertices: previous_skin,
        },
        Batch {
            index: 3,
            vertices: rigid.clone(),
            indices: vec![2, 0, 1, 5, 3, 4],
            previous_vertices: rigid,
        },
    ];
    let current_skin = Mat4::from_translation(Vec3::new(0.15, -0.2, -0.25));
    let models = [
        Mat4::IDENTITY,
        current_skin,
        Mat4::from_translation(Vec3::new(-0.4, 0.1, -0.25)),
        current_skin,
    ]
    .map(|model| model.to_cols_array().map(f32::to_bits))
    .to_vec();
    let placements = vec![0x4000_0003, 0xa000_0025];
    let coverage_extent = [1, 2];
    let coverage = vec![1, 0];
    let hit = (0..internal[0] * internal[1])
        .map(|pixel| {
            let x = pixel % internal[0];
            let y = pixel / internal[0];
            if y == 4 {
                return [2, 1, 0.375f32.to_bits(), 0.25f32.to_bits()];
            }
            if (x + y) % 5 == 0 {
                return [0, 0xffff_ffff, 0xdead_beef, 0xface_1234];
            }
            let placement = (x + y) % 2;
            let primitive = if placement == 0 {
                u32::from((x == 4 && y == 0) || (x == 2 && y == 2))
            } else {
                (x + 2 * y) % 2
            };
            [
                placement + 1,
                primitive,
                (0.125 + (x % 3) as f32 * 0.125).to_bits(),
                (0.125 + (y % 2) as f32 * 0.125).to_bits(),
            ]
        })
        .collect();
    let old = old_camera.matrix(output[0], output[1]);
    let post_frames = vec![
        post_frame(camera, old, output, 64., true),
        post_frame(camera, old, output, 0.25, true),
        post_frame(camera, old, output, 64., false),
        post_frame(
            camera,
            backward.matrix(output[0], output[1]),
            output,
            64.,
            true,
        ),
    ];
    let mut fixture = Fixture {
        internal,
        output,
        hit,
        coverage_extent,
        coverage,
        primary_frame,
        models,
        placements,
        batches,
        post_frames,
        frame_names: vec![
            "motion",
            "radius clamp",
            "history off",
            "previous clip behind",
        ],
        dispatch: [output[0].div_ceil(8), output[1].div_ceil(8), 1],
        expected: Vec::new(),
    };
    fixture.expected = fixture
        .post_frames
        .iter()
        .map(|frame| render(&fixture, frame))
        .collect();
    fixture
}
