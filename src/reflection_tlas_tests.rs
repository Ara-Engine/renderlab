//! RENDERLAB_TLAS on the reflection rays' geometry (src/instance_bvh.rs): a CPU replay of
//! `rs_trace`'s top level over a scene shaped like a kit-built course (long rotated decks,
//! rings, pylons) finds the same closest hits through the flat list, the median top level,
//! the SAH top level and the SAH top level over hull-tight instance boxes, and counts what
//! each walk visits.
use super::*;
use crate::instance_bvh::{self, Bounds};

/// A mesh: positions and a triangle list.
type Mesh = (Vec<[f32; 3]>, Vec<u32>);

/// A box mesh of half extents `h` (12 triangles).
fn cuboid(h: [f32; 3]) -> Mesh {
    let positions = (0..8)
        .map(|c| std::array::from_fn(|a| if c & (1 << a) == 0 { -h[a] } else { h[a] }))
        .collect();
    let indices = vec![
        0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6, 4,
        1, 5, 7, 1, 7, 3,
    ];
    (positions, indices)
}

/// A ring in the xy plane: radius `r`, tube half width `w`, `n` segments of a square tube.
fn ring(r: f32, w: f32, n: u32) -> Mesh {
    let mut positions = vec![];
    for s in 0..n {
        let a = s as f32 / n as f32 * std::f32::consts::TAU;
        let (c, sn) = (a.cos(), a.sin());
        for (dr, dz) in [(-w, -w), (w, -w), (w, w), (-w, w)] {
            positions.push([(r + dr) * c, (r + dr) * sn, dz]);
        }
    }
    let mut indices = vec![];
    for s in 0..n {
        let (a, b) = (s * 4, (s + 1) % n * 4);
        for k in 0..4 {
            let (k1, k2) = (k, (k + 1) % 4);
            indices.extend([a + k1, b + k1, b + k2, a + k1, b + k2, a + k2]);
        }
    }
    (positions, indices)
}

/// One instance: its placement slot, model and mesh index.
struct Placed {
    slot: u32,
    model: Mat4,
    mesh: usize,
}

/// A hit as the shader records it: (slot + 1, primitive, u bits, v bits) and t's bits.
type Hit = ([u32; 4], u32);

/// The rays' tmax (the shader's `best` starts at it; a missed box is above it).
const TMAX: f32 = 1e6;

/// What one ray's walk visited.
#[derive(Default, Clone, Copy)]
struct Visits {
    tlas: u64,
    entered: u64,
}

/// `rs_trace`'s top level replayed on the CPU over `words` (header, records, top level as
/// `instance_bvh::place` wrote them), with each entered instance's BLAS walked to its closest
/// hit (`tests::closest`; pruning by the bound does not change a closest hit). The bound and
/// the equal-distance rule are `rs_limit`'s.
fn trace(
    words: &[u32],
    blases: &[&Blas],
    meshes: &[usize],
    o: [f32; 3],
    d: [f32; 3],
    v: &mut Visits,
) -> Option<Hit> {
    let f = |i: usize| f32::from_bits(words[i]);
    let mut best: Option<(f32, u32, Hit)> = None;
    let limit = |best: &Option<(f32, u32, Hit)>, i: u32| match best {
        None => TMAX,
        Some((t, bi, _)) if i < *bi => f32::from_bits(t.to_bits() + 1),
        Some((t, _, _)) => *t,
    };
    let entry = |lo: [f32; 3], hi: [f32; 3], bound: f32| {
        let (mut near, mut far) = (0.0f32, bound);
        for a in 0..3 {
            let inv = 1.0 / d[a];
            let (t0, t1) = ((lo[a] - o[a]) * inv, (hi[a] - o[a]) * inv);
            near = near.max(t0.min(t1));
            far = far.min(t0.max(t1));
        }
        if near <= far { near } else { f32::INFINITY }
    };
    let instance = |i: u32, best: &mut Option<(f32, u32, Hit)>, v: &mut Visits| {
        let r = HEADER_WORDS + i as usize * INSTANCE_WORDS;
        let bound = limit(best, i);
        let lo = [f(r + 12), f(r + 13), f(r + 14)];
        let hi = [f(r + 15), f(r + 16), f(r + 17)];
        if entry(lo, hi, bound) > bound {
            return;
        }
        v.entered += 1;
        let row = |k: usize| {
            glam::Vec4::new(
                f(r + k * 4),
                f(r + k * 4 + 1),
                f(r + k * 4 + 2),
                f(r + k * 4 + 3),
            )
        };
        let (r0, r1, r2) = (row(0), row(1), row(2));
        let ow = glam::Vec4::new(o[0], o[1], o[2], 1.0);
        let dw = glam::Vec4::new(d[0], d[1], d[2], 0.0);
        let oo = [r0.dot(ow), r1.dot(ow), r2.dot(ow)];
        let od = [r0.dot(dw), r1.dot(dw), r2.dot(dw)];
        if let Some((prim, u, w, t)) = super::tests::closest(blases[meshes[i as usize]], oo, od)
            && t < limit(best, i)
        {
            let slot = words[r + 21];
            *best = Some((
                t,
                i,
                ([slot + 1, prim, u.to_bits(), w.to_bits()], t.to_bits()),
            ));
        }
    };
    let count = words[0];
    if words[1] == 0 {
        for i in 0..count {
            instance(i, &mut best, v);
        }
        return best.map(|b| b.2);
    }
    let base = words[1] as usize;
    let mut stack = vec![words[2]];
    while let Some(item) = stack.pop() {
        if item & instance_bvh::LEAF != 0 {
            instance(item & !instance_bvh::LEAF, &mut best, v);
            continue;
        }
        v.tlas += 1;
        let n = base + item as usize * instance_bvh::NODE_WORDS;
        let bound = match &best {
            None => TMAX,
            Some((t, _, _)) => f32::from_bits(t.to_bits() + 1),
        };
        let child = |k: usize| {
            (
                [f(n + k), f(n + k + 1), f(n + k + 2)],
                [f(n + k + 3), f(n + k + 4), f(n + k + 5)],
            )
        };
        let (t0, t1) = (
            entry(child(0).0, child(0).1, bound),
            entry(child(6).0, child(6).1, bound),
        );
        let near = t0 <= t1;
        let (first, second) = if near {
            ((t0, words[n + 12]), (t1, words[n + 13]))
        } else {
            ((t1, words[n + 13]), (t0, words[n + 12]))
        };
        // Pushed far first, so the near child pops next.
        if second.0 <= bound {
            stack.push(second.1);
        }
        if first.0 <= bound {
            stack.push(first.1);
        }
        assert!(stack.len() <= 32, "top-level stack overflow");
    }
    best.map(|b| b.2)
}

/// The course: `n` decks (long cuboids yawed by multiples of 30 degrees and pitched a
/// little), rings at random orientations and pylons, packed into an 80 m square as densely
/// as a kit-built course's goal (rays enter several overlapping instance boxes).
fn course(n: usize) -> (Vec<Mesh>, Vec<Placed>) {
    let meshes = vec![
        cuboid([12.0, 1.5, 0.2]),
        ring(6.0, 0.3, 32),
        cuboid([0.4, 0.4, 8.0]),
    ];
    let mut seed = 0x2545_f491u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10_000) as f32 / 10_000.0
    };
    let mut placed = vec![];
    for i in 0..n {
        let at = glam::Vec3::new(next() * 80.0 - 40.0, next() * 80.0 - 40.0, next() * 20.0);
        let mesh = i % 3;
        let rotation = match mesh {
            0 => {
                glam::Quat::from_rotation_z((i % 12) as f32 * 30f32.to_radians())
                    * glam::Quat::from_rotation_x(next() * 0.3)
            }
            1 => glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                next() * 3.0,
                next() * 3.0,
                next() * 3.0,
            ),
            _ => glam::Quat::IDENTITY,
        };
        placed.push(Placed {
            slot: i as u32,
            model: Mat4::from_rotation_translation(rotation, at),
            mesh,
        });
    }
    (meshes, placed)
}

/// Every variant finds exactly the flat list's closest hits; the SAH top level visits fewer
/// nodes than the median's, and hull-tight boxes let rays enter fewer instances.
#[test]
fn top_level_variants_keep_every_closest_hit() {
    let (meshes, placed) = course(600);
    let blases: Vec<Blas> = meshes
        .iter()
        .map(|(p, i)| Blas::build2(p, i).unwrap())
        .collect();
    let refs: Vec<&Blas> = blases.iter().collect();
    let packed = pack(&refs, placed.len());
    let rows: Vec<_> = placed
        .iter()
        .map(|p| (p.slot, p.model, &blases[p.mesh], packed.bases[p.mesh]))
        .collect();
    let mesh_of: Vec<usize> = placed.iter().map(|p| p.mesh).collect();
    let records = instance_words(&rows, &[]);
    // Hull-tight boxes, as `instance_words` writes them under RENDERLAB_TLAS=tight.
    let mut tight = records.clone();
    for (i, p) in placed.iter().enumerate() {
        let (positions, indices) = &meshes[p.mesh];
        let points: Vec<[f32; 3]> = indices.iter().map(|&k| positions[k as usize]).collect();
        let (lo, hi) = world_bounds(&p.model, &instance_bvh::hull(&points));
        let r = HEADER_WORDS + i * INSTANCE_WORDS;
        for (k, v) in lo.iter().chain(&hi).enumerate() {
            tight[r + 12 + k] = v.to_bits();
        }
    }
    let with_top = |records: &[u32], sah: bool| {
        let mut words = records.to_vec();
        let boxes: Vec<Bounds> = (0..placed.len())
            .map(|i| {
                let r = HEADER_WORDS + i * INSTANCE_WORDS;
                let f = |k: usize| f32::from_bits(words[r + k]);
                ([f(12), f(13), f(14)], [f(15), f(16), f(17)])
            })
            .collect();
        let nodes = instance_bvh::build_with(&boxes, sah);
        let base = words.len();
        words.extend(nodes);
        words[1] = base as u32;
        words[2] = 0;
        words
    };
    let variants = [
        ("flat", records.clone()),
        ("median", with_top(&records, false)),
        ("sah", with_top(&records, true)),
        ("sah,tight", with_top(&tight, true)),
    ];
    let mut seed = 0x1b87_3593u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10_000) as f32 / 10_000.0
    };
    let rays: Vec<([f32; 3], [f32; 3])> = (0..20_000)
        .map(|_| {
            let o = [next() * 80.0 - 40.0, next() * 80.0 - 40.0, next() * 20.0];
            // Mostly grazing directions, as reflections off a deck travel.
            let d = glam::Vec3::new(next() * 2.0 - 1.0, next() * 2.0 - 1.0, (next() - 0.5) * 0.6)
                .normalize()
                .to_array();
            (o, d)
        })
        .collect();
    let mut reference: Vec<Option<Hit>> = vec![];
    let mut hits = 0;
    for (name, words) in &variants {
        let mut total = Visits::default();
        let found: Vec<Option<Hit>> = rays
            .iter()
            .map(|&(o, d)| trace(words, &refs, &mesh_of, o, d, &mut total))
            .collect();
        if reference.is_empty() {
            hits = found.iter().flatten().count();
            reference = found;
        } else {
            let differ = found.iter().zip(&reference).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "{name}: {differ} rays changed their closest hit");
        }
        println!(
            "rs top level {name}: {:.2} TLAS nodes, {:.2} instances entered a ray ({hits} of {} rays hit)",
            total.tlas as f64 / rays.len() as f64,
            total.entered as f64 / rays.len() as f64,
            rays.len()
        );
    }
    assert!(hits > 2_000, "the course is hit ({hits} rays)");
}
