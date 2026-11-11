//! Two-pass semantic culling oracle using f64 geometry and constant depth pyramids.
use glam::{DMat4, DVec3};
use renderlab::{Mat4, Vec3};
use std::{collections::BTreeSet, ops::Range};

/// One pass's unordered outputs and exact cull/finish buffer snapshots.
pub struct PassExpected {
    /// Clusters routed to software rasterization.
    pub software: BTreeSet<u32>,
    /// Clusters routed to hardware clipping/rasterization.
    pub hardware: BTreeSet<u32>,
    /// Shared pass-zero reject indices, retained throughout pass one.
    pub rejects: BTreeSet<u32>,
    /// All 128 argument bytes after culling, before the finish kernel.
    pub args_cull: [u32; 32],
    /// All 128 argument bytes after finish generates dispatch/draw parameters.
    pub args_finish: [u32; 32],
    /// The 400-byte shared reject header after culling.
    pub header_cull: [u32; 100],
    /// The shared header after this pass's pyramid dispatches are written.
    pub header_finish: [u32; 100],
}

/// Reset snapshots run in pass-one, then pass-zero order.
pub struct ResetExpected {
    /// Config.info.z for this reset invocation.
    pub pass: u32,
    /// Working words zeroed and all sixteen old working words copied to history.
    pub args: [u32; 32],
    /// Pass one preserves this header; pass zero zeros all one hundred words.
    pub header: [u32; 100],
}

/// One coherent two-pass scene, followed by both reset variants.
pub struct Expected {
    /// Pass-zero cull against the previous pyramid, then current-pyramid retesting.
    pub passes: [PassExpected; 2],
    /// Reset order is pass one first, so header preservation can be observed.
    pub resets: [ResetExpected; 2],
}

/// Raw source-layout inputs; all words can be uploaded without struct repacking.
pub struct Fixture {
    /// Two 176-byte Config records; only info.z differs between them.
    pub configs: [[u32; 44]; 2],
    /// Object bounds, offsets, owner/refined groups, counts and placement: 64 bytes.
    pub clusters: Vec<[u32; 16]>,
    /// Sphere/error and padding: 32 bytes per LOD group.
    pub groups: Vec<[u32; 8]>,
    /// Column-major 64-byte affine matrices, including mirror and shear.
    pub models: Vec<[u32; 16]>,
    /// Source visibility dimensions; HZB level zero is half this size.
    pub viewport: [u32; 2],
    /// Uniform reversed depths in previous and current pyramids respectively.
    pub hzb_depths: [f32; 2],
    /// Five mip levels: 16,8,4,2,1 for the 32x32 source.
    pub hzb_levels: u32,
    /// Two initial 128-byte argument buffers with zero counters and history canaries.
    pub args: [[u32; 32]; 2],
    /// Each pass has N software words followed by N hardware words, initially canaries.
    pub lists: [Vec<u32>; 2],
    /// One 400-byte header followed by N reject-index words, initially canaries.
    pub rejects: Vec<u32>,
    /// Unordered sets, counters, indirect arguments, pyramid dispatches and resets.
    pub expected: Expected,
}

fn assert_segment(actual: &[u32], initial: &[u32], range: Range<usize>, expected: &BTreeSet<u32>) {
    let count = expected.len();
    assert!(range.start + count <= range.end);
    let found: BTreeSet<_> = actual[range.start..range.start + count]
        .iter()
        .copied()
        .collect();
    assert_eq!(found.len(), count, "duplicate cull-list entries");
    assert_eq!(&found, expected);
    assert_eq!(
        actual[range.start + count..range.end],
        initial[range.start + count..range.end]
    );
}

impl Fixture {
    /// Check either post-cull or post-finish readback without assuming append order.
    pub fn assert_pass(
        &self,
        pass: usize,
        finished: bool,
        lists: &[u32],
        args: &[u32],
        rejects: &[u32],
    ) {
        let expected = &self.expected.passes[pass];
        assert_eq!(lists.len(), self.lists[pass].len());
        assert_eq!(rejects.len(), self.rejects.len());
        let count = self.clusters.len();
        assert_segment(lists, &self.lists[pass], 0..count, &expected.software);
        assert_segment(
            lists,
            &self.lists[pass],
            count..count * 2,
            &expected.hardware,
        );
        let (arguments, header) = if finished {
            (&expected.args_finish, &expected.header_finish)
        } else {
            (&expected.args_cull, &expected.header_cull)
        };
        assert_eq!(args, arguments.as_slice());
        assert_eq!(&rejects[..100], header.as_slice());
        assert_segment(rejects, &self.rejects, 100..100 + count, &expected.rejects);
    }

    /// Check reset ordinal zero (pass one) or ordinal one (pass zero).
    pub fn assert_reset(&self, ordinal: usize, args: &[u32], rejects: &[u32]) {
        let expected = &self.expected.resets[ordinal];
        assert_eq!(args, expected.args.as_slice());
        assert_eq!(rejects.len(), self.rejects.len());
        assert_eq!(&rejects[..100], expected.header.as_slice());
        assert_segment(
            rejects,
            &self.rejects,
            100..self.rejects.len(),
            &self.expected.passes[0].rejects,
        );
    }
}

fn matrix(words: &[u32]) -> DMat4 {
    DMat4::from_cols_array(&std::array::from_fn(|i| {
        f64::from(f32::from_bits(words[i]))
    }))
}

/// Affine sphere/error evaluation in f64, without the shader's f32 rounding guards.
fn group_error(fixture: &Fixture, group: u32, placement: u32) -> f64 {
    let data = fixture.groups[group as usize].map(|word| f64::from(f32::from_bits(word)));
    if data[4] >= 3e38 {
        return f64::INFINITY;
    }
    let model = matrix(&fixture.models[placement as usize]);
    let columns = model.to_cols_array_2d();
    let one_norm = (0..3)
        .map(|c| (0..3).map(|r| columns[c][r].abs()).sum::<f64>())
        .fold(0., f64::max);
    let infinity_norm = (0..3)
        .map(|r| (0..3).map(|c| columns[c][r].abs()).sum::<f64>())
        .fold(0., f64::max);
    let scale = (one_norm * infinity_norm).sqrt();
    let centre = model.transform_point3(DVec3::new(data[0], data[1], data[2]));
    let config = fixture.configs[0].map(|word| f64::from(f32::from_bits(word)));
    let eye = DVec3::new(config[32], config[33], config[34]);
    let distance = (centre.distance(eye) - data[3] * scale).max(config[37]);
    data[4] * scale * config[35] / distance
}

#[derive(Clone, Copy)]
struct Bounds {
    low: DVec3,
    high: DVec3,
}

/// Project eight corners in f64. Near/eye crossings conservatively cover the screen.
fn project(fixture: &Fixture, index: u32, previous: bool) -> Bounds {
    let cluster = fixture.clusters[index as usize];
    let model = matrix(&fixture.models[cluster[12] as usize]);
    let vp = matrix(&fixture.configs[0][if previous { 16..32 } else { 0..16 }]);
    let mut bounds = Bounds {
        low: DVec3::splat(f64::INFINITY),
        high: DVec3::splat(f64::NEG_INFINITY),
    };
    for corner in 0..8 {
        let point = DVec3::from_array(std::array::from_fn(|axis| {
            f64::from(f32::from_bits(
                cluster[axis + if corner & (1 << axis) == 0 { 0 } else { 4 }],
            ))
        }));
        let clip = vp * (model * point.extend(1.));
        if clip.w <= 0. || clip.z >= clip.w {
            return Bounds {
                low: DVec3::new(-1., -1., 0.),
                high: DVec3::ONE,
            };
        }
        let ndc = clip.truncate() / clip.w;
        bounds.low = bounds.low.min(ndc);
        bounds.high = bounds.high.max(ndc);
    }
    bounds
}

fn outside(bounds: Bounds) -> bool {
    bounds.low.x > 1.
        || bounds.high.x < -1.
        || bounds.low.y > 1.
        || bounds.high.y < -1.
        || bounds.high.z < 0.
}

fn occluded(bounds: Bounds, depth: f32) -> bool {
    bounds.high.z < 1. && bounds.high.z < f64::from(depth)
}

fn software(fixture: &Fixture, bounds: Bounds) -> bool {
    let extent = bounds.high - bounds.low;
    bounds.high.z < 1.
        && (extent.x * f64::from(fixture.viewport[0]) * 0.5)
            .max(extent.y * f64::from(fixture.viewport[1]) * 0.5)
            <= f64::from(f32::from_bits(fixture.configs[0][38]))
}

fn triangles(fixture: &Fixture, indices: &BTreeSet<u32>) -> u32 {
    indices
        .iter()
        .map(|&index| fixture.clusters[index as usize][11] & 255)
        .sum()
}

#[allow(clippy::too_many_arguments)]
fn pass_expected(
    fixture: &Fixture,
    pass: usize,
    sw: BTreeSet<u32>,
    hw: BTreeSet<u32>,
    cut: &BTreeSet<u32>,
    rejects: &BTreeSet<u32>,
    header: &mut [u32; 100],
) -> PassExpected {
    let mut args = fixture.args[pass];
    for i in [3, 5, 8, 9, 10, 11] {
        assert_eq!(args[i], 0);
    }
    args[3] = sw.len() as u32;
    args[5] = hw.len() as u32;
    args[8] = cut.len() as u32;
    args[9] = triangles(fixture, cut);
    args[10] = triangles(fixture, &sw);
    args[11] = triangles(fixture, &hw);
    header[3] = rejects.len() as u32;
    let args_cull = args;
    let header_cull = *header;
    args[0] = args[3].min(65_535);
    args[1] = args[3].div_ceil(65_535);
    args[2] = 1;
    args[4] = 384;
    args[6] = 0;
    args[7] = 0;
    if pass == 0 {
        let groups = (rejects.len() as u32).div_ceil(64);
        args[12..15].copy_from_slice(&[groups.min(65_535), groups.div_ceil(65_535), 1]);
    }
    let build = if pass == 0 {
        !rejects.is_empty()
    } else {
        rejects.is_empty() || !sw.is_empty() || !hw.is_empty()
    };
    for level in 0..fixture.hzb_levels.min(16) {
        let dimensions = fixture
            .viewport
            .map(|n| (n.div_ceil(2).next_power_of_two() >> level).max(1));
        let offset = 4 + pass * 48 + level as usize * 3;
        header[offset..offset + 3].copy_from_slice(&[
            if build { dimensions[0].div_ceil(8) } else { 0 },
            if build { dimensions[1].div_ceil(8) } else { 0 },
            1,
        ]);
    }
    PassExpected {
        software: sw,
        hardware: hw,
        rejects: rejects.clone(),
        args_cull,
        args_finish: args,
        header_cull,
        header_finish: *header,
    }
}

/// Classify this constant-HZB scene without emulating atomic/workgroup algorithms.
/// The fixture avoids f32 classification boundaries, so projection/error use f64
/// instead of copying cluster_box's conservative epsilon expansion. Constant mip
/// values make the four-tap rectangle test exactly a comparison with nearest depth.
pub fn reference(fixture: &Fixture) -> Expected {
    assert_eq!(fixture.configs[0][42], 0);
    assert_eq!(fixture.configs[1][42], 1);
    let mut second_config = fixture.configs[0];
    second_config[42] = 1;
    assert_eq!(fixture.configs[1], second_config);
    assert_eq!(fixture.configs[0][40] as usize, fixture.clusters.len());
    assert_eq!(
        fixture.configs[0][43],
        fixture.viewport[0] | fixture.viewport[1] << 16
    );
    assert_eq!(fixture.rejects.len(), 100 + fixture.clusters.len());
    assert_eq!(fixture.rejects[3], 0);
    let threshold = f64::from(f32::from_bits(fixture.configs[0][36]));
    let mut cut = BTreeSet::new();
    let mut rejected = BTreeSet::new();
    let mut sw = BTreeSet::new();
    let mut hw = BTreeSet::new();
    for (index, c) in fixture.clusters.iter().enumerate() {
        let selected = group_error(fixture, c[8], c[12]) > threshold
            && (c[9] == u32::MAX || group_error(fixture, c[9], c[12]) <= threshold);
        if !selected {
            continue;
        }
        let index = index as u32;
        cut.insert(index);
        let bounds = project(fixture, index, false);
        if outside(bounds) {
            continue;
        }
        if fixture.configs[0][41] != 0
            && occluded(project(fixture, index, true), fixture.hzb_depths[0])
        {
            rejected.insert(index);
        } else if software(fixture, bounds) {
            sw.insert(index);
        } else {
            hw.insert(index);
        }
    }
    let mut header: [u32; 100] = std::array::from_fn(|i| fixture.rejects[i]);
    let first = pass_expected(fixture, 0, sw, hw, &cut, &rejected, &mut header);
    let mut sw = BTreeSet::new();
    let mut hw = BTreeSet::new();
    for &index in &rejected {
        let bounds = project(fixture, index, false);
        if outside(bounds) || occluded(bounds, fixture.hzb_depths[1]) {
            continue;
        }
        if software(fixture, bounds) {
            sw.insert(index);
        } else {
            hw.insert(index);
        }
    }
    let second = pass_expected(fixture, 1, sw, hw, &BTreeSet::new(), &rejected, &mut header);
    let reset_args = |mut args: [u32; 32]| {
        let working: [u32; 16] = std::array::from_fn(|i| args[i]);
        args[16..32].copy_from_slice(&working);
        args[..16].fill(0);
        args
    };
    let resets = [
        ResetExpected {
            pass: 1,
            args: reset_args(second.args_finish),
            header,
        },
        ResetExpected {
            pass: 0,
            args: reset_args(first.args_finish),
            header: [0; 100],
        },
    ];
    Expected {
        passes: [first, second],
        resets,
    }
}

fn group(centre: [f32; 3], radius: f32, error: f32) -> [u32; 8] {
    [centre[0], centre[1], centre[2], radius, error, 0., 0., 0.].map(f32::to_bits)
}

fn cluster(
    lo: [f32; 3],
    hi: [f32; 3],
    owner: u32,
    refined: u32,
    placement: u32,
    triangles: u32,
) -> [u32; 16] {
    let mut words = [0; 16];
    words[..3].copy_from_slice(&lo.map(f32::to_bits));
    words[4..7].copy_from_slice(&hi.map(f32::to_bits));
    words[8] = owner;
    words[9] = refined;
    words[11] = triangles | 8 << 8;
    words[12] = placement;
    words
}

/// A partial workgroup with LOD rejection, frustum rejection, near/eye fallback,
/// SW/HW routing, recovered HZB rejects and one reject that stays occluded.
pub fn fixture() -> Fixture {
    let viewport = [32, 32];
    let vp = Mat4::from_cols(
        glam::Vec4::X,
        glam::Vec4::Y,
        glam::Vec4::new(0., 0., 0.125, -1.),
        glam::Vec4::new(0., 0., 0.625, 0.),
    );
    let models = vec![
        Mat4::IDENTITY,
        Mat4::from_scale_rotation_translation(
            Vec3::new(-1.5, 0.75, 1.25),
            glam::Quat::IDENTITY,
            Vec3::new(0.15, -0.1, 0.),
        ),
        Mat4::from_cols(
            glam::Vec4::new(1., 0.25, 0., 0.),
            glam::Vec4::new(0.5, 1.5, 0., 0.),
            glam::Vec4::new(0., 0., 0.75, 0.),
            glam::Vec4::new(-0.25, 0.1, 0., 1.),
        ),
    ]
    .into_iter()
    .map(|m| m.to_cols_array().map(f32::to_bits))
    .collect();
    let groups = vec![
        group([0., 0., -2.], 10., f32::MAX),
        group([0., 0., -2.1], 0.5, 0.005),
        group([0., 0., -1.8], 1.1, 0.3),
        group([0., 0., -1.9], 1.2, 0.003),
        group([0., 0., -2.35], 0.25, 0.055),
    ];
    let none = u32::MAX;
    let clusters = vec![
        cluster([-0.12, -0.1, -0.95], [0.02, 0.04, -0.8], 0, none, 0, 3),
        cluster([-0.45, -0.4, -1.], [0.45, 0.4, -0.8], 0, none, 0, 7),
        cluster([0.1, 0.2, -2.], [0.3, 0.4, -1.6], 0, none, 0, 5),
        cluster([-0.3, -0.2, -4.3], [-0.1, 0., -3.8], 0, none, 0, 11),
        cluster([-0.2, -0.2, -2.], [0.2, 0.2, -1.8], 1, none, 0, 13),
        cluster([-0.2, -0.2, -2.], [0.2, 0.2, -1.8], 0, 2, 0, 17),
        cluster([-0.8, -0.6, -2.], [0.8, 0.6, -1.6], 2, 3, 0, 19),
        cluster([2.5, -0.1, -1.1], [3., 0.1, -0.9], 0, none, 0, 23),
        cluster([-0.2, -0.2, -8.], [0.2, 0.2, -7.], 0, none, 0, 29),
        cluster([-0.12, -0.1, -0.75], [0.12, 0.1, -0.4], 0, none, 0, 31),
        cluster([-0.1, -0.1, -0.2], [0.1, 0.1, 0.2], 0, none, 0, 37),
        cluster([0.1, -0.1, -1.6], [0.22, 0.02, -1.3], 2, 3, 1, 41),
        cluster([-0.1, -0.1, -2.5], [0.1, 0.1, -2.2], 4, 3, 2, 43),
        cluster([-0.1, -0.1, -2.5], [0.1, 0.1, -2.2], 1, none, 2, 47),
        cluster([-0.1, -3., -1.1], [0.1, -2.5, -0.9], 0, none, 0, 53),
        cluster([-0.1, -0.1, -0.55], [0.1, 0.1, -0.3], 0, none, 1, 59),
    ];
    let configs = std::array::from_fn(|pass| {
        let mut config = [0; 44];
        config[..16].copy_from_slice(&vp.to_cols_array().map(f32::to_bits));
        config[16..32].copy_from_slice(&vp.to_cols_array().map(f32::to_bits));
        config[35] = 16f32.to_bits();
        config[36..40].copy_from_slice(&[1f32.to_bits(), 0.25f32.to_bits(), 6f32.to_bits(), 0]);
        config[40..44].copy_from_slice(&[clusters.len() as u32, 1, pass as u32, 32 | 32 << 16]);
        config
    });
    let args = std::array::from_fn(|pass| {
        let mut words = [0; 32];
        words[15] = 0xd000_0000 | pass as u32;
        for (i, word) in words[16..].iter_mut().enumerate() {
            *word = 0xab00_0000 | (pass as u32) << 8 | i as u32;
        }
        words
    });
    let lists = std::array::from_fn(|_| vec![0xfeed_face; clusters.len() * 2]);
    let mut rejects = vec![0; 100];
    rejects[..3].copy_from_slice(&[0xc001_0000, 0xc001_0001, 0xc001_0002]);
    rejects.extend((0..clusters.len()).map(|i| 0xface_0000 | i as u32));
    let mut fixture = Fixture {
        configs,
        clusters,
        groups,
        models,
        viewport,
        hzb_depths: [0.5, 0.16],
        hzb_levels: 5,
        args,
        lists,
        rejects,
        expected: Expected {
            passes: std::array::from_fn(|_| PassExpected {
                software: BTreeSet::new(),
                hardware: BTreeSet::new(),
                rejects: BTreeSet::new(),
                args_cull: [0; 32],
                args_finish: [0; 32],
                header_cull: [0; 100],
                header_finish: [0; 100],
            }),
            resets: std::array::from_fn(|pass| ResetExpected {
                pass: pass as u32,
                args: [0; 32],
                header: [0; 100],
            }),
        },
    };
    fixture.expected = reference(&fixture);
    fixture
}
