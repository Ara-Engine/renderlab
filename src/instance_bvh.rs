//! U-RTVIS (docs/gi-gather-cost.md): the top level of the non-RT path's software rays. Both
//! software traversals (shaders/gi_rays_software.wgsl `gi_traverse`, the GI cache's rays, and
//! shaders/reflections_software.wgsl `rs_trace`, reflection rays) packed their instances as a
//! flat list that every ray walked: one world-box test per instance per ray. `place` appends
//! a BVH2 over those instance records' world boxes, built on the CPU whenever the records
//! change, so a ray visits O(log N) instance boxes. Traversal keeps the flat loop's hits
//! exactly: equal-distance hits resolve to the lower instance index, as the ascending loop did.
//!
//! Layout, after a buffer's instance records (each `stride` words, world lo at 12..15 and hi at
//! 15..18): `reserve(n)` words of 16-word nodes, each child 0's box (lo, hi), child 1's box,
//! then the two tagged children (a node index, or `LEAF | instance`), then two pad words; the
//! same layout as the reflection BLAS nodes. Header word 1 is the nodes' base word (0: no top
//! level, the shaders walk the flat list), word 2 the root's tagged id and word 3 how many
//! refits the nodes have had since their build (CPU bookkeeping; the shaders do not read it).

/// A child tag: the child is one instance, `LEAF | index`.
pub(crate) const LEAF: u32 = 0x8000_0000;
pub(crate) const NODE_WORDS: usize = 16;

/// Words the top level of `instances` records may take (a BVH2 over N leaves has N - 1 nodes).
pub(crate) fn reserve(instances: usize) -> usize {
    instances.saturating_sub(1).max(1) * NODE_WORDS
}

/// Fewer instances than this walk the flat list: the top level's per-ray overhead outweighs
/// the box tests it skips. Measured on the non-RT gathers (docs/gi-gather-cost.md): the game
/// flight's 30 instances gathered 4.4 ms flat against 5.7 through a top level, the bare
/// forest's 45 8.5 against 7.3, and the forest's 9,900 147 against 12.
pub(crate) const MIN_INSTANCES: usize = 40;

/// `RENDERLAB_INSTANCE_BVH=0` writes no top level, so both traversals walk the flat list
/// (measurement switch, U-RTVIS); `=1` writes one whatever the count; unset, MIN_INSTANCES
/// decides. The words stay reserved either way.
fn mode() -> Option<bool> {
    static MODE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        std::env::var("RENDERLAB_INSTANCE_BVH")
            .ok()
            .map(|v| v != "0")
    })
}
/// Whether `count` records get a top level.
pub(crate) fn enabled(count: usize) -> bool {
    mode().unwrap_or(count >= MIN_INSTANCES) && count > 0
}

pub(crate) type Bounds = ([f32; 3], [f32; 3]);

/// Refits a top level keeps before it is rebuilt: moving instances widen the boxes of the
/// subtrees they were built into, which costs rays visits but never changes a hit.
pub(crate) const REFITS: u32 = 64;

/// Appends the top level over the `words[0]` records of `words` (header `header` words, then
/// `stride` words a record) at `words.len()`, padded to `reserve(slots)`, and sets header
/// words 1 to 3. When `previous` holds the same records (and this layout), its top level is
/// reused; when only their boxes changed (the same instances, moved), it is refitted, and
/// rebuilt every REFITS refits.
pub(crate) fn place(
    words: &mut Vec<u32>,
    header: usize,
    stride: usize,
    slots: usize,
    previous: &[u32],
) {
    let base = words.len();
    let count = words[0] as usize;
    let enabled = enabled(count);
    if enabled {
        words[1] = base as u32;
        words[2] = if count == 1 { LEAF } else { 0 };
    }
    let same_layout = previous.len() == base + reserve(slots) && previous[..3] == words[..3];
    if same_layout && previous[4..base] == words[4..] {
        words[3] = previous[3];
        words.extend_from_slice(&previous[base..]);
        return;
    }
    words.resize(base + reserve(slots), 0);
    if !enabled || count < 2 {
        return;
    }
    let boxes: Vec<Bounds> = (0..count)
        .map(|i| {
            let r = header + i * stride;
            let f = |k: usize| f32::from_bits(words[r + k]);
            ([f(12), f(13), f(14)], [f(15), f(16), f(17)])
        })
        .collect();
    let used = (count - 1) * NODE_WORDS;
    if same_layout && previous[3] < REFITS {
        words[3] = previous[3] + 1;
        words[base..base + used].copy_from_slice(&previous[base..base + used]);
        refit(&mut words[base..base + used], &boxes);
    } else {
        words[3] = 0;
        let nodes = build(&boxes);
        words[base..base + nodes.len()].copy_from_slice(&nodes);
    }
}

/// Rewrites every node's child boxes for new instance `boxes`, keeping the topology. Children
/// follow their parents in `build`'s order, so a reverse walk sees every child first.
pub(crate) fn refit(nodes: &mut [u32], boxes: &[Bounds]) {
    for node in (0..nodes.len() / NODE_WORDS).rev() {
        for c in 0..2 {
            let id = nodes[node * NODE_WORDS + 12 + c];
            let (lo, hi) = if id & LEAF != 0 {
                boxes[(id & !LEAF) as usize]
            } else {
                let w = &nodes[id as usize * NODE_WORDS..][..12];
                let f = |k: usize| f32::from_bits(w[k]);
                union(
                    [
                        ([f(0), f(1), f(2)], [f(3), f(4), f(5)]),
                        ([f(6), f(7), f(8)], [f(9), f(10), f(11)]),
                    ]
                    .into_iter(),
                )
            };
            let at = node * NODE_WORDS + c * 6;
            for (k, v) in lo.iter().chain(&hi).enumerate() {
                nodes[at + k] = v.to_bits();
            }
        }
    }
}

/// `RENDERLAB_TLAS` (measurement, off by default): a comma-separated list of `sah` (binned
/// surface-area splits, `sah_split`, instead of the balanced median) and `tight` (instance
/// world boxes from the transformed boxes of the mesh's `hull` instead of its one object
/// box). Both software traversals' hits are unchanged: they keep the flat loop's
/// equal-distance rule whatever the tree, and a tighter box still holds every vertex.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tlas {
    pub sah: bool,
    pub tight: bool,
}
/// The process's `RENDERLAB_TLAS`, read once.
pub(crate) fn tlas() -> Tlas {
    static TLAS: std::sync::OnceLock<Tlas> = std::sync::OnceLock::new();
    *TLAS.get_or_init(|| {
        let value = std::env::var("RENDERLAB_TLAS").unwrap_or_default();
        let has = |name: &str| value.split(',').any(|v| v.trim() == name);
        Tlas {
            sah: has("sah"),
            tight: has("tight"),
        }
    })
}
/// BVH2 nodes over `boxes` (at least two), root first: `RENDERLAB_TLAS=sah`'s splits, else
/// the median's.
pub(crate) fn build(boxes: &[Bounds]) -> Vec<u32> {
    build_with(boxes, tlas().sah)
}
/// `build` with the split chosen: surface-area (`sah`) or the balanced median. Deterministic:
/// centroid ties on the median's axis break by instance index.
pub(crate) fn build_with(boxes: &[Bounds], sah: bool) -> Vec<u32> {
    let mut nodes = Vec::with_capacity(reserve(boxes.len()));
    let mut order: Vec<u32> = (0..boxes.len() as u32).collect();
    let centres: Vec<[f32; 3]> = boxes.iter().map(centre).collect();
    subtree(boxes, &centres, &mut order, &mut nodes, sah, 0);
    nodes
}
/// Twice a box's centre (the split coordinate; the factor moves no split).
fn centre(b: &Bounds) -> [f32; 3] {
    std::array::from_fn(|a| b.0[a] + b.1[a])
}
/// Half a box's surface area (the SAH's weight; an empty box weighs 0).
fn area((lo, hi): &Bounds) -> f32 {
    let d: [f32; 3] = std::array::from_fn(|a| (hi[a] - lo[a]).max(0.0));
    d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
}
/// The deepest leaf a build makes: both traversals' top-level stacks (RS_TOP_STACK,
/// GI_TOP_STACK: 32) hold one sibling a level plus the node in hand. Median splits stay at
/// ceil(log2 n); SAH splits, which can unbalance, give way to the median where they could
/// pass it.
pub(crate) const MAX_DEPTH: u32 = 30;
/// Bins per axis of `sah_split`.
const SAH_BINS: usize = 16;
/// Partitions `order` (at least two) at the binned surface-area split: per axis, centroids in
/// SAH_BINS equal bins, and the bin boundary minimising area(left) x count(left) +
/// area(right) x count(right). Returns the left side's length, or None when every centroid
/// coincides (no boundary separates them). Deterministic (the partition is in place, in
/// `order`'s order).
fn sah_split(boxes: &[Bounds], centres: &[[f32; 3]], order: &mut [u32]) -> Option<usize> {
    let (lo, hi) = union(order.iter().map(|&i| {
        let c = centres[i as usize];
        (c, c)
    }));
    let bin = |axis: usize, i: u32| {
        let scale = SAH_BINS as f32 / (hi[axis] - lo[axis]);
        (((centres[i as usize][axis] - lo[axis]) * scale) as usize).min(SAH_BINS - 1)
    };
    let empty = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    let grow = |a: &mut Bounds, b: &Bounds| {
        for k in 0..3 {
            a.0[k] = a.0[k].min(b.0[k]);
            a.1[k] = a.1[k].max(b.1[k]);
        }
    };
    // (cost, axis, last left bin)
    let mut best: Option<(f32, usize, usize)> = None;
    for axis in 0..3 {
        if hi[axis] - lo[axis] <= 0.0 {
            continue;
        }
        let mut bins = [(empty, 0usize); SAH_BINS];
        for &i in order.iter() {
            let b = &mut bins[bin(axis, i)];
            grow(&mut b.0, &boxes[i as usize]);
            b.1 += 1;
        }
        // right[k]: bins k+1.. (area, count).
        let mut right = [(0.0f32, 0usize); SAH_BINS];
        let mut acc = (empty, 0usize);
        for k in (1..SAH_BINS).rev() {
            grow(&mut acc.0, &bins[k].0);
            acc.1 += bins[k].1;
            right[k - 1] = (area(&acc.0), acc.1);
        }
        let mut left = (empty, 0usize);
        for k in 0..SAH_BINS - 1 {
            grow(&mut left.0, &bins[k].0);
            left.1 += bins[k].1;
            let (r_area, r_count) = right[k];
            if left.1 == 0 || r_count == 0 {
                continue;
            }
            let cost = area(&left.0) * left.1 as f32 + r_area * r_count as f32;
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, axis, k));
            }
        }
    }
    let (_, axis, last) = best?;
    let mut mid = 0;
    for k in 0..order.len() {
        if bin(axis, order[k]) <= last {
            order.swap(k, mid);
            mid += 1;
        }
    }
    Some(mid)
}
/// The tagged id of the subtree over `order` and its box.
fn subtree(
    boxes: &[Bounds],
    centres: &[[f32; 3]],
    order: &mut [u32],
    nodes: &mut Vec<u32>,
    sah: bool,
    depth: u32,
) -> (u32, Bounds) {
    if let [i] = order {
        return (LEAF | *i, boxes[*i as usize]);
    }
    // SAH only while a median below could still finish within MAX_DEPTH.
    let room = depth + order.len().next_power_of_two().trailing_zeros() < MAX_DEPTH;
    let split = if sah && room {
        sah_split(boxes, centres, order)
    } else {
        None
    };
    let mid = split.unwrap_or_else(|| {
        let centre = |i: u32| centres[i as usize];
        let (lo, hi) = union(order.iter().map(|&i| {
            let c = centre(i);
            (c, c)
        }));
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap();
        let mid = order.len() / 2;
        order.select_nth_unstable_by(mid, |&a, &b| {
            centre(a)[axis].total_cmp(&centre(b)[axis]).then(a.cmp(&b))
        });
        mid
    });
    let node = nodes.len() / NODE_WORDS;
    nodes.extend([0; NODE_WORDS]);
    let (left, right) = order.split_at_mut(mid);
    let a = subtree(boxes, centres, left, nodes, sah, depth + 1);
    let b = subtree(boxes, centres, right, nodes, sah, depth + 1);
    let words = &mut nodes[node * NODE_WORDS..(node + 1) * NODE_WORDS];
    for (k, v) in [a.1.0, a.1.1, b.1.0, b.1.1].iter().flatten().enumerate() {
        words[k] = v.to_bits();
    }
    words[12] = a.0;
    words[13] = b.0;
    (node as u32, union([a.1, b.1].into_iter()))
}

/// Boxes a `hull` has at most.
pub(crate) const HULL_BOXES: usize = 16;
/// Object boxes a mesh's instance box is made from under `RENDERLAB_TLAS=tight`: up to
/// HULL_BOXES boxes over groups of `points` (median splits on the longest axis). Their
/// transformed corners' world box holds every transformed point, and so every triangle,
/// and is tighter than the transformed object box wherever the mesh does not fill it
/// (rings, rotated decks, L-shapes).
pub(crate) fn hull(points: &[[f32; 3]]) -> Vec<Bounds> {
    fn split(points: &mut [[f32; 3]], depth: u32, out: &mut Vec<Bounds>) {
        let (lo, hi) = union(points.iter().map(|&p| (p, p)));
        if depth == 0 || points.len() < 8 {
            out.push((lo, hi));
            return;
        }
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap();
        let mid = points.len() / 2;
        points.select_nth_unstable_by(mid, |a, b| a[axis].total_cmp(&b[axis]));
        let (l, r) = points.split_at_mut(mid);
        split(l, depth - 1, out);
        split(r, depth - 1, out);
    }
    let mut points = points.to_vec();
    let mut out = Vec::with_capacity(HULL_BOXES);
    if !points.is_empty() {
        split(&mut points, HULL_BOXES.trailing_zeros(), &mut out);
    }
    out
}
/// The world box of object `boxes` under `model`: the bounds of their transformed corners.
pub(crate) fn world_box(model: &crate::Mat4, boxes: &[Bounds]) -> Bounds {
    union(boxes.iter().flat_map(|&(lo, hi)| {
        (0..8).map(move |c| {
            let p = glam::Vec3::from(std::array::from_fn(|a| {
                if c & (1 << a) == 0 { lo[a] } else { hi[a] }
            }));
            let w = model.transform_point3(p).to_array();
            (w, w)
        })
    }))
}

fn union(boxes: impl Iterator<Item = Bounds>) -> Bounds {
    boxes.fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(lo, hi), (l, h)| {
            (
                std::array::from_fn(|a| lo[a].min(l[a])),
                std::array::from_fn(|a| hi[a].max(h[a])),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deepest leaf under the tagged `id`.
    fn depth(nodes: &[u32], id: u32) -> u32 {
        if id & LEAF != 0 {
            return 0;
        }
        let w = &nodes[id as usize * NODE_WORDS..][..NODE_WORDS];
        1 + depth(nodes, w[12]).max(depth(nodes, w[13]))
    }

    /// Nested boxes of doubling size, the SAH's worst case (each split peels one box): the
    /// SAH build hands over to the median before MAX_DEPTH, so both top-level stacks hold
    /// the walk.
    #[test]
    fn sah_top_level_stays_within_the_stack() {
        let boxes: Vec<Bounds> = (0..5000)
            .map(|i| {
                let s = 1.0012f32.powi(i);
                ([0.0; 3], [s, s * 0.5, s * 0.25])
            })
            .collect();
        for sah in [false, true] {
            let nodes = build_with(&boxes, sah);
            let d = depth(&nodes, 0);
            assert!(d <= MAX_DEPTH, "sah {sah}: depth {d}");
        }
    }

    /// Every transformed point of a mesh lies in the world box of its hull, which is no
    /// larger than the transformed object box.
    #[test]
    fn hull_world_box_holds_every_point() {
        let mut seed = 0x6a09_e667u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 10_000.0
        };
        // A ring of points (the box is mostly air) and a long bar.
        let mut points: Vec<[f32; 3]> = (0..400)
            .map(|i| {
                let a = i as f32 * 0.0157;
                [6.0 * a.cos(), 6.0 * a.sin(), next() * 0.3]
            })
            .collect();
        points.extend((0..50).map(|_| [next() * 24.0 - 12.0, next() * 0.3, 8.0 + next() * 0.2]));
        let hull = hull(&points);
        assert!(hull.len() <= HULL_BOXES);
        let object = union(points.iter().map(|&p| (p, p)));
        for _ in 0..50 {
            let rotation = glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                next() * 6.0,
                next() * 6.0,
                next() * 6.0,
            );
            let model = crate::Mat4::from_scale_rotation_translation(
                glam::Vec3::splat(0.5 + next() * 2.0),
                rotation,
                glam::Vec3::new(next() * 50.0, next() * 50.0, next() * 50.0),
            );
            let (lo, hi) = world_box(&model, &hull);
            let (olo, ohi) = world_box(&model, &[object]);
            for p in &points {
                let w = model.transform_point3(glam::Vec3::from(*p)).to_array();
                assert!((0..3).all(|a| lo[a] <= w[a] && w[a] <= hi[a]));
            }
            assert!((0..3).all(|a| olo[a] <= lo[a] && hi[a] <= ohi[a]));
            assert!(area(&(lo, hi)) < area(&(olo, ohi)));
        }
    }

    /// Every instance is reached exactly once, every node's child box contains what is under
    /// it, and the build fits its reserve, with median and SAH splits.
    #[test]
    fn top_level_covers_every_instance_once() {
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 1000) as f32 / 10.0
        };
        let boxes: Vec<Bounds> = (0..257)
            .map(|_| {
                let (x, y, z) = (next(), next(), next());
                ([x, y, z], [x + 1.0, y + 2.0, z + 0.5])
            })
            .collect();
        for sah in [false, true] {
            let nodes = build_with(&boxes, sah);
            assert!(nodes.len() <= reserve(boxes.len()));
            let mut seen = vec![0; boxes.len()];
            // (tagged id, the box its parent recorded for it)
            let mut stack = vec![(0u32, None::<Bounds>)];
            while let Some((id, parent)) = stack.pop() {
                if id & LEAF != 0 {
                    let i = (id & !LEAF) as usize;
                    seen[i] += 1;
                    assert_eq!(parent.unwrap(), boxes[i]);
                    continue;
                }
                let w = &nodes[id as usize * NODE_WORDS..][..NODE_WORDS];
                let f = |k: usize| f32::from_bits(w[k]);
                let child = |k: usize| ([f(k), f(k + 1), f(k + 2)], [f(k + 3), f(k + 4), f(k + 5)]);
                if let Some((lo, hi)) = parent {
                    for (l, h) in [child(0), child(6)] {
                        assert!((0..3).all(|a| lo[a] <= l[a] && h[a] <= hi[a]));
                    }
                }
                stack.push((w[12], Some(child(0))));
                stack.push((w[13], Some(child(6))));
            }
            assert!(seen.iter().all(|&n| n == 1));
        }
    }

    /// Measurement (ignored): one build over 10,000 scattered boxes, the forest's scale, timed.
    #[test]
    #[ignore]
    fn top_level_build_time() {
        let mut seed = 0x9e37_79b9u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 100_000) as f32 / 500.0
        };
        let boxes: Vec<Bounds> = (0..10_000)
            .map(|_| {
                let (x, y, z) = (next(), next() * 0.05, next());
                ([x, y, z], [x + 2.0, y + 6.0, z + 2.0])
            })
            .collect();
        let runs = 20;
        for sah in [false, true] {
            let start = std::time::Instant::now();
            for _ in 0..runs {
                std::hint::black_box(build_with(std::hint::black_box(&boxes), sah));
            }
            println!(
                "instance_bvh build (sah {sah}), 10,000 boxes: {:.3} ms",
                start.elapsed().as_secs_f64() * 1e3 / runs as f64
            );
        }
        // RENDERLAB_TLAS=tight's per-instance cost when an instance moves: its hull's corners.
        let points: Vec<[f32; 3]> = (0..2000)
            .map(|i| {
                let a = i as f32 * 0.003;
                [6.0 * a.cos(), 6.0 * a.sin(), (i % 7) as f32 * 0.1]
            })
            .collect();
        let hull = hull(&points);
        let models: Vec<crate::Mat4> = (0..10_000)
            .map(|i| crate::Mat4::from_rotation_z(i as f32 * 0.01))
            .collect();
        let start = std::time::Instant::now();
        for model in &models {
            std::hint::black_box(world_box(model, std::hint::black_box(&hull)));
        }
        println!(
            "instance_bvh hull world boxes ({} boxes), 10,000 instances: {:.3} ms",
            hull.len(),
            start.elapsed().as_secs_f64() * 1e3
        );
        let mut nodes = build(&boxes);
        let start = std::time::Instant::now();
        for _ in 0..runs {
            refit(
                std::hint::black_box(&mut nodes),
                std::hint::black_box(&boxes),
            );
        }
        println!(
            "instance_bvh refit, 10,000 boxes: {:.3} ms",
            start.elapsed().as_secs_f64() * 1e3 / runs as f64
        );
    }

    /// A refit after moving every box gives each node the boxes a walk of its topology
    /// computes, and the same root a fresh build would have bounds of.
    #[test]
    fn refit_follows_moved_boxes() {
        let boxes: Vec<Bounds> = (0..50)
            .map(|i| {
                let x = (i * 7 % 50) as f32;
                ([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0])
            })
            .collect();
        let mut nodes = build(&boxes);
        let moved: Vec<Bounds> = boxes
            .iter()
            .map(|(lo, hi)| ([lo[0], lo[1] + 3.0, lo[2]], [hi[0], hi[1] + 3.0, hi[2]]))
            .collect();
        refit(&mut nodes, &moved);
        let fresh = build(&moved);
        // Same instances, same split order (the move is along y, the build splits on x): equal.
        assert_eq!(nodes, fresh);
    }

    /// `place` writes the header words, reuses an unchanged build and fills the reserve.
    #[test]
    fn place_sets_the_header_and_reuses_unchanged_records() {
        let (header, stride) = (8, 24);
        let mut words = vec![0u32; header + 3 * stride];
        words[0] = 3;
        for i in 0..3 {
            let r = header + i * stride;
            for k in 0..3 {
                words[r + 12 + k] = (i as f32 * 4.0).to_bits();
                words[r + 15 + k] = (i as f32 * 4.0 + 1.0).to_bits();
            }
        }
        let records = words.clone();
        place(&mut words, header, stride, 3, &[]);
        assert_eq!(words.len(), records.len() + reserve(3));
        if enabled(3) {
            assert_eq!((words[1], words[2]), (records.len() as u32, 0));
        }
        let mut again = records.clone();
        place(&mut again, header, stride, 3, &words);
        assert_eq!(again, words);
    }
}
