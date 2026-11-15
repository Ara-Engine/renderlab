//! U-RTVIS (docs/gi-gather-cost.md): the top level of the non-RT path's software rays. Both
//! software traversals (shaders/gi_rays_software.wgsl `gi_traverse`, the GI cache's rays, and
//! shaders/reflections_software_bvh4.wgsl `rs_trace`, reflection rays) packed their instances as
//! a flat list that every ray walked: one world-box test per instance per ray. `place` appends
//! a tree over those instance records' world boxes, built on the CPU whenever the records
//! change, so a ray visits O(log N) instance boxes. Traversal keeps the flat loop's hits
//! exactly: equal-distance hits resolve to the lower instance index, as the ascending loop did,
//! so any tree over the same boxes finds the same hits; only the boxes a ray visits differ.
//!
//! Layout, after a buffer's instance records (each `stride` words, world lo at 12..15 and hi at
//! 15..18): `reserve(n)` words of nodes. The default (`wide`) is a BVH4 of 32-word nodes: the
//! four children's float boxes axis-major (lo x, lo y, lo z, hi x, hi y, hi z, four words each),
//! then the four tagged children (a node index, `LEAF | instance`, or EMPTY), then four pad
//! words, so a ray tests all four boxes in one step. `RENDERLAB_INSTANCE_BVH_WIDE=0` writes the
//! earlier BVH2 of 16-word nodes: child 0's box (lo, hi), child 1's box, the two tagged
//! children, two pad words. Header word 1 is the nodes' base word (0: no top level, the shaders
//! walk the flat list), word 2 the root's tagged id and word 3 how many refits the nodes have
//! had since their build (CPU bookkeeping; the shaders do not read it).
//!
//! Large sets build off the frame: a rebuild over ASYNC_MIN or more instances runs on a worker
//! thread, and the frame keeps refitting its current tree (or, for a new set, takes a median
//! build at a fraction of the cost) until the worker's tree is swapped in and refitted.

/// A child tag: the child is one instance, `LEAF | index`.
pub(crate) const LEAF: u32 = 0x8000_0000;
/// A wide node's unused child slot.
pub(crate) const EMPTY: u32 = u32::MAX;
pub(crate) const NODE_WORDS: usize = 16;
pub(crate) const WIDE_WORDS: usize = 32;
/// The shaders' top-level stack (RS_TOP_STACK, GI_TOP_STACK): a wide tree pushes at most three
/// more entries a level, so its depth stays under STACK / 3.
const STACK: usize = 48;

/// Words the top level of `instances` records may take (either tree over N leaves has at most
/// N - 1 nodes; reserved at the wide size whichever is written).
pub(crate) fn reserve(instances: usize) -> usize {
    instances.saturating_sub(1).max(1) * WIDE_WORDS
}

/// Whether the top level is the BVH4 (`RENDERLAB_INSTANCE_BVH_WIDE=0`: the BVH2, which the
/// reflection rollback traversal `RENDERLAB_REFLECTION_BVH4=0` also needs).
pub(crate) fn wide() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RENDERLAB_INSTANCE_BVH_WIDE").map_or(true, |v| v != "0")
            && crate::reflection_software::bvh4_enabled()
    })
}

/// The traversal's top-level constants for this process's tree: `source` (a software traversal
/// shader) as written for the BVH4, or with its BVH2 path selected.
pub(crate) fn shader_text(source: &str) -> String {
    if wide() {
        return source.to_owned();
    }
    let mut source = source.to_owned();
    for (from, to) in [
        ("const RS_TOP_WIDE:bool=true;", "const RS_TOP_WIDE:bool=false;"),
        ("const RS_TOP_NODE:u32=8u;", "const RS_TOP_NODE:u32=4u;"),
        ("const GI_TOP_WIDE:bool=true;", "const GI_TOP_WIDE:bool=false;"),
        ("const GI_TOP_NODE:u32=32u;", "const GI_TOP_NODE:u32=16u;"),
    ] {
        source = source.replacen(from, to, 1);
    }
    source
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

/// Sets of at least this many instances build their SAH tree on a worker (about 0.75 ms of CPU
/// a thousand instances, release). `RENDERLAB_INSTANCE_BVH_ASYNC=0` builds every tree on the
/// frame.
pub(crate) const ASYNC_MIN: usize = 1024;
fn asynchronous() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_INSTANCE_BVH_ASYNC").map_or(true, |v| v != "0"))
}

/// A worker build per buffer layout (header and stride words: the reflection and GI buffers):
/// the instance count it was asked for, its finished nodes, and whether it still runs. A
/// finished tree fits any set of that many instances, since `place` refits it to the current
/// boxes before use.
struct Job {
    count: usize,
    done: Option<Vec<u32>>,
    running: bool,
}
type Jobs = std::sync::Mutex<std::collections::HashMap<(usize, usize), Job>>;
fn jobs() -> &'static Jobs {
    static JOBS: std::sync::OnceLock<Jobs> = std::sync::OnceLock::new();
    JOBS.get_or_init(Default::default)
}
/// The finished worker tree for `count` instances of this layout, if one waits.
fn take_ready(key: (usize, usize), count: usize) -> Option<Vec<u32>> {
    let mut table = jobs().lock().ok()?;
    let job = table.get_mut(&key)?;
    if job.count != count {
        return None;
    }
    job.done.take()
}
/// Starts a worker build over `boxes` unless one runs for this layout.
fn request(key: (usize, usize), boxes: &[Bounds]) {
    let Ok(mut table) = jobs().lock() else { return };
    let job = table.entry(key).or_insert(Job { count: 0, done: None, running: false });
    if job.running || (job.count == boxes.len() && job.done.is_some()) {
        return;
    }
    *job = Job { count: boxes.len(), done: None, running: true };
    let boxes = boxes.to_vec();
    let spawned = std::thread::Builder::new()
        .name("instance top level".into())
        .spawn(move || {
            let nodes = build(&boxes);
            if let Ok(mut table) = jobs().lock() {
                if let Some(job) = table.get_mut(&key) {
                    if job.count == boxes.len() {
                        job.done = Some(nodes);
                    }
                    job.running = false;
                }
            }
        });
    if spawned.is_err() {
        if let Some(job) = table.get_mut(&key) {
            job.running = false;
        }
    }
}

/// Appends the top level over the `words[0]` records of `words` (header `header` words, then
/// `stride` words a record) at `words.len()`, padded to `reserve(slots)`, and sets header
/// words 1 to 3. When `previous` holds the same records (and this layout), its top level is
/// reused; when only their boxes changed (the same instances, moved), it is refitted, and
/// rebuilt every REFITS refits (large sets: on a worker, refitting until it is ready).
pub(crate) fn place(
    words: &mut Vec<u32>,
    header: usize,
    stride: usize,
    slots: usize,
    previous: &[u32],
) {
    let key = (header, stride);
    let base = words.len();
    let count = words[0] as usize;
    let enabled = enabled(count);
    if enabled {
        words[1] = base as u32;
        words[2] = if count == 1 { LEAF } else { 0 };
    }
    let same_layout = previous.len() == base + reserve(slots) && previous[..3] == words[..3];
    let ready = if enabled && count >= 2 { take_ready(key, count) } else { None };
    if ready.is_none() && same_layout && previous[4..base] == words[4..] {
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
    let span = base..base + reserve(slots);
    if let Some(nodes) = ready {
        words[3] = 0;
        words[base..base + nodes.len()].copy_from_slice(&nodes);
        refit(&mut words[span], &boxes);
        return;
    }
    let deferred = count >= ASYNC_MIN && asynchronous();
    if same_layout && (previous[3] < REFITS || deferred) {
        if previous[3] >= REFITS {
            request(key, &boxes);
        }
        words[3] = (previous[3] + 1).min(REFITS);
        words[span.clone()].copy_from_slice(&previous[span.clone()]);
        refit(&mut words[span], &boxes);
        return;
    }
    words[3] = 0;
    let nodes = if deferred {
        request(key, &boxes);
        build_with(&boxes, false)
    } else {
        build(&boxes)
    };
    words[base..base + nodes.len()].copy_from_slice(&nodes);
}

/// Rewrites every node's child boxes for new instance `boxes`, keeping the topology (the root
/// is node 0; `nodes` may run past the tree).
pub(crate) fn refit(nodes: &mut [u32], boxes: &[Bounds]) {
    if wide() {
        refit_wide(nodes, 0, boxes);
        return;
    }
    // BVH2: children follow their parents in `build`'s order, so a reverse walk sees every
    // child first.
    for node in (0..boxes.len().saturating_sub(1)).rev() {
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
/// The wide node `node`'s boxes refitted under it; returns their union.
fn refit_wide(nodes: &mut [u32], node: u32, boxes: &[Bounds]) -> Bounds {
    let at = node as usize * WIDE_WORDS;
    let mut all = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for c in 0..4 {
        let id = nodes[at + 24 + c];
        if id == EMPTY {
            continue;
        }
        let b = if id & LEAF != 0 { boxes[(id & !LEAF) as usize] } else { refit_wide(nodes, id, boxes) };
        write_child(&mut nodes[at..at + WIDE_WORDS], c, b);
        all = union([all, b].into_iter());
    }
    all
}
/// Child `c`'s box into a wide node's axis-major planes.
fn write_child(node: &mut [u32], c: usize, (lo, hi): Bounds) {
    for a in 0..3 {
        node[a * 4 + c] = lo[a].to_bits();
        node[12 + a * 4 + c] = hi[a].to_bits();
    }
}

/// `RENDERLAB_INSTANCE_BVH_SAH=0`: split every node at the centroid median of its longest
/// axis, the earlier build. The default splits where the binned surface-area cost is least,
/// which keeps large instances (terrain, floors, walls) from widening every box over the small
/// ones. Either tree finds the same hits; only the boxes a ray visits differ.
fn median_split() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_INSTANCE_BVH_SAH").is_ok_and(|v| v == "0"))
}

/// Centroid bins a node's split is chosen among, per axis.
const BINS: usize = 16;

/// Half a box's surface area.
fn area((lo, hi): Bounds) -> f32 {
    let e: [f32; 3] = std::array::from_fn(|a| (hi[a] - lo[a]).max(0.0));
    e[0] * e[1] + e[1] * e[2] + e[2] * e[0]
}

/// The least-cost binned split of `order` (SAH: each side's box area times its instance count)
/// as its axis and the last bin of its left side, given the centroid bounds `lo`..`hi` (in
/// `centre`'s doubled units); none when every centroid falls in one bin on every axis.
fn sah_split(
    boxes: &[Bounds],
    order: &[u32],
    centre: impl Fn(u32) -> [f32; 3],
    (lo, hi): Bounds,
) -> Option<(usize, usize, f32, f32)> {
    let empty = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    let mut best: Option<(f32, usize, usize)> = None;
    for axis in 0..3 {
        let extent = hi[axis] - lo[axis];
        if !(extent > 0.0) {
            continue;
        }
        let scale = BINS as f32 / extent;
        let mut count = [0usize; BINS];
        let mut bounds = [empty; BINS];
        for &i in order {
            let b = (((centre(i)[axis] - lo[axis]) * scale) as usize).min(BINS - 1);
            count[b] += 1;
            bounds[b] = union([bounds[b], boxes[i as usize]].into_iter());
        }
        // Right-side areas and counts for splits after bins 0..BINS-1.
        let mut right = [(0.0f32, 0usize); BINS];
        let (mut acc, mut n) = (empty, 0);
        for b in (1..BINS).rev() {
            acc = union([acc, bounds[b]].into_iter());
            n += count[b];
            right[b - 1] = (area(acc), n);
        }
        let (mut acc, mut n) = (empty, 0);
        for b in 0..BINS - 1 {
            acc = union([acc, bounds[b]].into_iter());
            n += count[b];
            let (right_area, right_n) = right[b];
            if n == 0 || right_n == 0 {
                continue;
            }
            let cost = area(acc) * n as f32 + right_area * right_n as f32;
            if best.is_none_or(|(c, ..)| cost < c) {
                best = Some((cost, axis, b));
            }
        }
    }
    best.map(|(_, axis, b)| (axis, b, lo[axis], BINS as f32 / (hi[axis] - lo[axis])))
}

/// The top level's nodes over `boxes` (at least two), root first: the SAH tree (median splits
/// under `RENDERLAB_INSTANCE_BVH_SAH=0`), collapsed to the BVH4 when `wide`.
pub(crate) fn build(boxes: &[Bounds]) -> Vec<u32> {
    build_with(boxes, !median_split())
}
/// `build` with SAH (`sah`) or median splits. Deterministic: the split depends only on the
/// node's instances (bin ties go to the lower axis and bin; median ties on the split axis break
/// by instance index). A wide tree too deep for the shaders' stack is rebuilt from median splits
/// (balanced).
fn build_with(boxes: &[Bounds], sah: bool) -> Vec<u32> {
    let mut nodes = Vec::with_capacity(boxes.len().saturating_sub(1) * NODE_WORDS);
    let mut order: Vec<u32> = (0..boxes.len() as u32).collect();
    subtree(boxes, &mut order, &mut nodes, sah);
    if !wide() {
        return nodes;
    }
    let mut out = Vec::with_capacity(nodes.len());
    collapse(&nodes, 0, &mut out);
    if sah && 1 + 3 * wide_depth(&out, 0) > STACK {
        return build_with(boxes, false);
    }
    out
}
/// The BVH2 node `node`'s children as a wide node appended to `out` (preorder, so children
/// follow parents): its two children, then repeatedly the largest-area inner child replaced by
/// its own two, up to four. Returns its index.
fn collapse(binary: &[u32], node: u32, out: &mut Vec<u32>) -> u32 {
    let child = |node: u32, c: usize| -> (u32, Bounds) {
        let w = &binary[node as usize * NODE_WORDS..][..NODE_WORDS];
        let f = |k: usize| f32::from_bits(w[c * 6 + k]);
        (w[12 + c], ([f(0), f(1), f(2)], [f(3), f(4), f(5)]))
    };
    let index = out.len() / WIDE_WORDS;
    out.extend([0; WIDE_WORDS]);
    let mut kids = vec![child(node, 0), child(node, 1)];
    while kids.len() < 4 {
        let Some(i) = (0..kids.len())
            .filter(|&i| kids[i].0 & LEAF == 0)
            .max_by(|&a, &b| area(kids[a].1).total_cmp(&area(kids[b].1)).then(b.cmp(&a)))
        else {
            break;
        };
        let (id, _) = kids.remove(i);
        kids.insert(i, child(id, 1));
        kids.insert(i, child(id, 0));
    }
    for c in 0..4 {
        let at = index * WIDE_WORDS;
        let Some(&(id, b)) = kids.get(c) else {
            out[at + 24 + c] = EMPTY;
            continue;
        };
        let id = if id & LEAF != 0 { id } else { collapse(binary, id, out) };
        write_child(&mut out[at..at + WIDE_WORDS], c, b);
        out[at + 24 + c] = id;
    }
    index as u32
}
/// Inner levels under the wide node `node`, itself included.
fn wide_depth(nodes: &[u32], node: u32) -> usize {
    let at = node as usize * WIDE_WORDS;
    1 + (0..4)
        .map(|c| nodes[at + 24 + c])
        .filter(|&id| id != EMPTY && id & LEAF == 0)
        .map(|id| wide_depth(nodes, id))
        .max()
        .unwrap_or(0)
}

/// The tagged id of the BVH2 subtree over `order` and its box.
fn subtree(boxes: &[Bounds], order: &mut [u32], nodes: &mut Vec<u32>, sah: bool) -> (u32, Bounds) {
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
    let split = if sah { sah_split(boxes, order, centre, (lo, hi)) } else { None };
    let mid = if let Some((axis, last, base, scale)) = split {
        let bin = |i: u32| (((centre(i)[axis] - base) * scale) as usize).min(BINS - 1);
        // Left side first (an in-place partition: O(n) a node).
        let mut left = 0;
        for k in 0..order.len() {
            if bin(order[k]) <= last {
                order.swap(left, k);
                left += 1;
            }
        }
        left
    } else {
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap();
        let mid = order.len() / 2;
        order.select_nth_unstable_by(mid, |&a, &b| {
            centre(a)[axis].total_cmp(&centre(b)[axis]).then(a.cmp(&b))
        });
        mid
    };
    let node = nodes.len() / NODE_WORDS;
    nodes.extend([0; NODE_WORDS]);
    let (left, right) = order.split_at_mut(mid);
    let a = subtree(boxes, left, nodes, sah);
    let b = subtree(boxes, right, nodes, sah);
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
            for (child, b) in children(&nodes, id) {
                if let Some((lo, hi)) = parent {
                    assert!((0..3).all(|a| lo[a] <= b.0[a] && b.1[a] <= hi[a]));
                }
                stack.push((child, Some(b)));
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
        if wide() {
            assert!(1 + 3 * wide_depth(&nodes, 0) <= STACK);
        }
    }

    /// A node's tagged children and the boxes it records for them, in either layout.
    fn children(nodes: &[u32], id: u32) -> Vec<(u32, Bounds)> {
        let f = |k: usize| f32::from_bits(nodes[k]);
        if wide() {
            let at = id as usize * WIDE_WORDS;
            (0..4)
                .filter(|&c| nodes[at + 24 + c] != EMPTY)
                .map(|c| {
                    let b = std::array::from_fn(|a| f(at + a * 4 + c));
                    let t = std::array::from_fn(|a| f(at + 12 + a * 4 + c));
                    (nodes[at + 24 + c], (b, t))
                })
                .collect()
        } else {
            let at = id as usize * NODE_WORDS;
            let child = |k: usize| ([f(at + k), f(at + k + 1), f(at + k + 2)], [f(at + k + 3), f(at + k + 4), f(at + k + 5)]);
            vec![(nodes[at + 12], child(0)), (nodes[at + 13], child(6))]
        }
    }

    /// Every instance box a ray crosses is reached by a walk that enters only the boxes it
    /// crosses (the shaders' pruning with no hit yet), so the top level drops no candidate.
    #[test]
    fn top_level_walk_reaches_every_crossed_instance() {
        let mut seed = 0x0bad_5eedu32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 100.0
        };
        let boxes: Vec<Bounds> = (0..3000)
            .map(|i| {
                let (x, y, z) = (next(), next() * 0.1, next());
                let s = if i % 50 == 0 { 30.0 } else { 0.5 + (i % 7) as f32 * 0.3 };
                ([x, y, z], [x + s, y + s * 0.5, z + s])
            })
            .collect();
        let nodes = build(&boxes);
        let crosses = |(lo, hi): Bounds, o: [f32; 3], d: [f32; 3]| {
            let (mut near, mut far) = (0.0f32, f32::INFINITY);
            for a in 0..3 {
                let (p, q) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
                near = near.max(p.min(q));
                far = far.min(p.max(q));
            }
            near <= far
        };
        for r in 0..200 {
            let o = [next(), next() * 0.1 + 1.0, next()];
            let d = [next() - 50.0 + 0.01, (r % 5) as f32 * 0.1 - 0.2 + 0.003, next() - 50.0 + 0.02];
            let mut want: Vec<u32> = (0..boxes.len() as u32).filter(|&i| crosses(boxes[i as usize], o, d)).collect();
            let mut got = vec![];
            let mut stack = vec![0u32];
            while let Some(id) = stack.pop() {
                for (child, b) in children(&nodes, id) {
                    if crosses(b, o, d) {
                        if child & LEAF != 0 {
                            got.push(child & !LEAF);
                        } else {
                            stack.push(child);
                        }
                    }
                }
            }
            got.sort_unstable();
            want.sort_unstable();
            assert_eq!(got, want, "ray {r}");
        }
    }

    /// A large set's first `place` writes the median tree and asks a worker for the SAH tree;
    /// a later `place` over the same records swaps the worker's tree in (refit count 0).
    #[test]
    fn large_sets_swap_in_the_worker_tree() {
        if !asynchronous() {
            return;
        }
        let (header, stride, count) = (12, 20, ASYNC_MIN + 300);
        let mut records = vec![0u32; header + count * stride];
        records[0] = count as u32;
        let mut boxes = vec![];
        for i in 0..count {
            let r = header + i * stride;
            let x = (i * 37 % 101) as f32;
            let z = (i * 53 % 97) as f32;
            let b = ([x, 0.0, z], [x + 1.0 + (i % 5) as f32, 2.0, z + 1.0]);
            for k in 0..3 {
                records[r + 12 + k] = b.0[k].to_bits();
                records[r + 15 + k] = b.1[k].to_bits();
            }
            boxes.push(b);
        }
        let base = records.len();
        let mut words = records.clone();
        place(&mut words, header, stride, count, &[]);
        assert_eq!(&words[base..base + build_with(&boxes, false).len()], &build_with(&boxes, false)[..]);
        let sah = build(&boxes);
        for _ in 0..500 {
            let mut next = records.clone();
            place(&mut next, header, stride, count, &words);
            words = next;
            if words[base..base + sah.len()] == sah[..] {
                assert_eq!(words[3], 0);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the worker tree never arrived");
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
        let start = std::time::Instant::now();
        for _ in 0..runs {
            std::hint::black_box(build_with(std::hint::black_box(&boxes), false));
        }
        println!(
            "instance_bvh median build (a new large set's frame), 10,000 boxes: {:.3} ms",
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
