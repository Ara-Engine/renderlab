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

type Bounds = ([f32; 3], [f32; 3]);

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

/// Median-split BVH2 nodes over `boxes` (at least two), root first. Deterministic: centroid
/// ties on the split axis break by instance index.
pub(crate) fn build(boxes: &[Bounds]) -> Vec<u32> {
    let mut nodes = Vec::with_capacity(reserve(boxes.len()));
    let mut order: Vec<u32> = (0..boxes.len() as u32).collect();
    subtree(boxes, &mut order, &mut nodes);
    nodes
}

/// The tagged id of the subtree over `order` and its box.
fn subtree(boxes: &[Bounds], order: &mut [u32], nodes: &mut Vec<u32>) -> (u32, Bounds) {
    if let [i] = order {
        return (LEAF | *i, boxes[*i as usize]);
    }
    let centre = |i: u32| {
        let (lo, hi) = boxes[i as usize];
        std::array::from_fn::<f32, 3, _>(|a| lo[a] + hi[a])
    };
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
    let node = nodes.len() / NODE_WORDS;
    nodes.extend([0; NODE_WORDS]);
    let (left, right) = order.split_at_mut(mid);
    let a = subtree(boxes, left, nodes);
    let b = subtree(boxes, right, nodes);
    let words = &mut nodes[node * NODE_WORDS..(node + 1) * NODE_WORDS];
    for (k, v) in [a.1.0, a.1.1, b.1.0, b.1.1].iter().flatten().enumerate() {
        words[k] = v.to_bits();
    }
    words[12] = a.0;
    words[13] = b.0;
    (node as u32, union([a.1, b.1].into_iter()))
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

    /// Every instance is reached exactly once, every node's child box contains what is under
    /// it, and the build fits its reserve.
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
        let nodes = build(&boxes);
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
        let start = std::time::Instant::now();
        let runs = 20;
        for _ in 0..runs {
            std::hint::black_box(build(std::hint::black_box(&boxes)));
        }
        println!(
            "instance_bvh build, 10,000 boxes: {:.3} ms",
            start.elapsed().as_secs_f64() * 1e3 / runs as f64
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
