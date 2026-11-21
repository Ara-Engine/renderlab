//! Measurement, default off (`RENDERLAB_GI_WIDE=1`): the lighting cache's software rays trace
//! the static world through one flattened 8-wide BVH, the compressed wide form GPU path tracers
//! use (Ylitie, Karras and Laine, "Efficient Incoherent Ray Traversal on GPUs Through
//! Compressed Wide BVHs", 2017): instances baked to world space, eight children a node with
//! 8-bit child boxes quantized against the node's origin and power-of-two scales, 80 bytes a
//! node. Movers, skinned meshes, cooked near fields and the merged world stay on the two-level
//! structure (src/gi_bvh.rs, src/instance_bvh.rs); a ray takes the nearer of the two answers
//! (shaders/gi_rays_wide.wgsl, called from gi_traverse's seam).
//!
//! Exact: child boxes are rounded outward, triangles are tested in full precision, and a hit
//! returns what the two-level structure returns (instance, the triangle's position indices in
//! its mesh, barycentrics, the source-winding normal from the instance's rows).
//! `RENDERLAB_GI_WIDE_COUNT=1` also counts nodes visited and triangles tested per ray, on
//! both structures, into counter words 360-365 (shaders/gi_rays_wide.wgsl).
use glam::Vec3;
use std::sync::OnceLock;

pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_WIDE").as_deref() == Ok("1"))
}
/// Meshes placed more often than this stay two-level (`RENDERLAB_GI_WIDE_MAX_PLACEMENTS`, default
/// every mesh): the flattened world copies each placement's triangles, so heavily instanced
/// meshes multiply its memory.
pub(crate) fn max_placements() -> usize {
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| std::env::var("RENDERLAB_GI_WIDE_MAX_PLACEMENTS").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX))
}
pub(crate) fn counting() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_WIDE_COUNT").as_deref() == Ok("1"))
}

const NODE_WORDS: usize = 20;
const TRI_WORDS: usize = 11;
/// Leaves hold at most this many triangles (three bits of a child's meta byte).
const LEAF_MAX: usize = 3;
const BINS: usize = 16;
/// Header words of the geometry buffer the shader reads: the node and triangle array bases
/// (0: no flattened world).
pub(crate) const HEADER_NODES: usize = 8;
pub(crate) const HEADER_TRIS: usize = 9;

/// One static triangle in world space: its instance record (`inst`), the word of its first
/// position index in the geometry buffer (`index`), and whether its instance casts no shadow.
pub(crate) struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub inst: u32,
    pub index: u32,
    pub no_shadow: bool,
}

#[derive(Clone, Copy)]
struct Aabb {
    lo: Vec3,
    hi: Vec3,
}
impl Aabb {
    const EMPTY: Self = Self { lo: Vec3::splat(f32::INFINITY), hi: Vec3::splat(f32::NEG_INFINITY) };
    fn grow(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }
    fn merge(&mut self, o: &Self) {
        self.lo = self.lo.min(o.lo);
        self.hi = self.hi.max(o.hi);
    }
    fn area(&self) -> f32 {
        let d = (self.hi - self.lo).max(Vec3::ZERO);
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
}
enum Node {
    Inner { bounds: Aabb, left: usize, right: usize },
    Leaf { bounds: Aabb, first: usize, count: usize },
}
impl Node {
    fn bounds(&self) -> &Aabb {
        match self {
            Node::Inner { bounds, .. } | Node::Leaf { bounds, .. } => bounds,
        }
    }
}

/// A binned-SAH binary BVH over `order` (triangle indices, reordered in place).
fn split(tris: &[Tri], boxes: &[Aabb], centres: &[Vec3], order: &mut [u32], first: usize, nodes: &mut Vec<Node>) -> usize {
    let mut bounds = Aabb::EMPTY;
    let mut cb = Aabb::EMPTY;
    for &t in order.iter() {
        bounds.merge(&boxes[t as usize]);
        cb.grow(centres[t as usize]);
    }
    let count = order.len();
    let leaf = |nodes: &mut Vec<Node>| {
        nodes.push(Node::Leaf { bounds, first, count });
        nodes.len() - 1
    };
    if count <= LEAF_MAX {
        return leaf(nodes);
    }
    let extent = cb.hi - cb.lo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z { 0 } else if extent.y >= extent.z { 1 } else { 2 };
    let mid = if extent[axis] <= 0.0 {
        // Every centre in one place: split the list in half.
        count / 2
    } else {
        let bin = |c: Vec3| (((c[axis] - cb.lo[axis]) / extent[axis] * BINS as f32) as usize).min(BINS - 1);
        let mut bin_boxes = [Aabb::EMPTY; BINS];
        let mut bin_counts = [0usize; BINS];
        for &t in order.iter() {
            let b = bin(centres[t as usize]);
            bin_boxes[b].merge(&boxes[t as usize]);
            bin_counts[b] += 1;
        }
        // The split after bin s minimising area-weighted counts.
        let (mut best, mut best_cost) = (0, f32::INFINITY);
        for s in 0..BINS - 1 {
            let (mut l, mut r) = (Aabb::EMPTY, Aabb::EMPTY);
            let (mut nl, mut nr) = (0, 0);
            for b in 0..=s {
                l.merge(&bin_boxes[b]);
                nl += bin_counts[b];
            }
            for b in s + 1..BINS {
                r.merge(&bin_boxes[b]);
                nr += bin_counts[b];
            }
            if nl == 0 || nr == 0 {
                continue;
            }
            let cost = l.area() * nl as f32 + r.area() * nr as f32;
            if cost < best_cost {
                (best, best_cost) = (s, cost);
            }
        }
        if best_cost.is_infinite() {
            count / 2
        } else {
            // Partition by bin.
            let mut i = 0;
            for j in 0..count {
                if bin(centres[order[j] as usize]) <= best {
                    order.swap(i, j);
                    i += 1;
                }
            }
            i
        }
    };
    if extent[axis] <= 0.0 || mid == 0 || mid == count {
        order.sort_by(|&a, &b| centres[a as usize][axis].total_cmp(&centres[b as usize][axis]));
    }
    let mid = mid.clamp(1, count - 1);
    let (lo, hi) = order.split_at_mut(mid);
    let left = split(tris, boxes, centres, lo, first, nodes);
    let right = split(tris, boxes, centres, hi, first + mid, nodes);
    nodes.push(Node::Inner { bounds, left, right });
    nodes.len() - 1
}

/// The flattened world: node words, then triangle words (`node_count` nodes from word 0 and
/// the triangles from `tri_word`), and what it cost to build.
pub(crate) struct Wide {
    pub words: Vec<u32>,
    pub nodes: usize,
    pub tri_word: usize,
    pub triangles: usize,
    pub seconds: f64,
}

/// Builds the flattened 8-wide BVH over `tris`.
pub(crate) fn build(tris: &[Tri]) -> Wide {
    let started = std::time::Instant::now();
    let boxes: Vec<Aabb> = tris
        .iter()
        .map(|t| {
            let mut b = Aabb::EMPTY;
            b.grow(t.a);
            b.grow(t.b);
            b.grow(t.c);
            b
        })
        .collect();
    let centres: Vec<Vec3> = boxes.iter().map(|b| (b.lo + b.hi) * 0.5).collect();
    let mut order: Vec<u32> = (0..tris.len() as u32).collect();
    let mut binary = vec![];
    let root = split(tris, &boxes, &centres, &mut order, 0, &mut binary);
    // Collapse to eight children a node, breadth first: a node's inner children are
    // consecutive nodes from its child base, its leaves' triangles consecutive from its
    // triangle base.
    struct Wnode {
        bounds: Aabb,
        slots: Vec<usize>,
    }
    let expand = |b: usize| -> Vec<usize> {
        let mut slots = match &binary[b] {
            Node::Inner { left, right, .. } => vec![*left, *right],
            Node::Leaf { .. } => vec![b],
        };
        while slots.len() < 8 {
            let pick = slots
                .iter()
                .enumerate()
                .filter(|&(_, &s)| matches!(binary[s], Node::Inner { .. }))
                .max_by(|a, b| binary[*a.1].bounds().area().total_cmp(&binary[*b.1].bounds().area()))
                .map(|(i, _)| i);
            let Some(i) = pick else { break };
            let Node::Inner { left, right, .. } = binary[slots[i]] else { unreachable!() };
            slots[i] = left;
            slots.push(right);
        }
        slots
    };
    let mut wide = vec![Wnode { bounds: *binary[root].bounds(), slots: expand(root) }];
    let mut node_words = vec![];
    let mut tri_order: Vec<u32> = vec![];
    let mut at = 0;
    while at < wide.len() {
        let slots = wide[at].slots.clone();
        let bounds = wide[at].bounds;
        let child_base = wide.len() as u32;
        let tri_base = tri_order.len() as u32;
        let mut meta = [0u8; 8];
        let mut child_boxes = [Aabb::EMPTY; 8];
        let mut inner = 0u32;
        for (k, &s) in slots.iter().enumerate() {
            child_boxes[k] = *binary[s].bounds();
            match binary[s] {
                Node::Inner { .. } => {
                    meta[k] = 0x40 | inner as u8;
                    inner += 1;
                    let next = Wnode { bounds: *binary[s].bounds(), slots: expand(s) };
                    wide.push(next);
                }
                Node::Leaf { first, count, .. } => {
                    let offset = tri_order.len() as u32 - tri_base;
                    meta[k] = 0x80 | (((count - 1) as u8) << 5) | offset as u8;
                    tri_order.extend_from_slice(&order[first..first + count]);
                }
            }
        }
        node_words.extend(encode(&bounds, &child_boxes, &meta, child_base, tri_base));
        at += 1;
    }
    let nodes = wide.len();
    let tri_word = node_words.len();
    let mut words = node_words;
    for &t in &tri_order {
        let t = &tris[t as usize];
        for p in [t.a, t.b, t.c] {
            words.extend(p.to_array().map(f32::to_bits));
        }
        words.push(t.inst | (u32::from(t.no_shadow) << 31));
        words.push(t.index);
    }
    Wide { words, nodes, tri_word, triangles: tri_order.len(), seconds: started.elapsed().as_secs_f64() }
}

/// One node's 20 words: origin, three exponent bytes, child and triangle bases, eight meta
/// bytes, then the children's 8-bit low and high corners by axis (eight bytes an axis).
/// Corners round outward, checked against the shader's decode (`p + q * 2^e`), so every
/// decoded child box holds its triangles.
fn encode(bounds: &Aabb, children: &[Aabb; 8], meta: &[u8; 8], child_base: u32, tri_base: u32) -> [u32; NODE_WORDS] {
    let p = bounds.lo;
    let mut e = [0i32; 3];
    let mut scale = [0f32; 3];
    for a in 0..3 {
        let extent = (bounds.hi[a] - p[a]).max(f32::MIN_POSITIVE);
        let mut x = (extent / 255.0).log2().ceil() as i32;
        let s = |x: i32| f32::from_bits(((x.clamp(-126, 127) + 127) as u32) << 23);
        while p[a] + 255.0 * s(x) < bounds.hi[a] {
            x += 1;
        }
        e[a] = x.clamp(-126, 127);
        scale[a] = s(x);
    }
    let mut lo = [[0u8; 8]; 3];
    let mut hi = [[0u8; 8]; 3];
    for (k, c) in children.iter().enumerate() {
        if meta[k] == 0 {
            continue;
        }
        for a in 0..3 {
            let mut ql = ((c.lo[a] - p[a]) / scale[a]).floor().clamp(0.0, 255.0) as u32;
            while ql > 0 && p[a] + ql as f32 * scale[a] > c.lo[a] {
                ql -= 1;
            }
            let mut qh = ((c.hi[a] - p[a]) / scale[a]).ceil().clamp(0.0, 255.0) as u32;
            while qh < 255 && p[a] + qh as f32 * scale[a] < c.hi[a] {
                qh += 1;
            }
            lo[a][k] = ql as u8;
            hi[a][k] = qh as u8;
        }
    }
    let pack = |b: &[u8; 8], half: usize| u32::from_le_bytes([b[half * 4], b[half * 4 + 1], b[half * 4 + 2], b[half * 4 + 3]]);
    let exponents = e.map(|x| (x + 127) as u32);
    let mut w = [0u32; NODE_WORDS];
    w[0..3].copy_from_slice(&p.to_array().map(f32::to_bits));
    w[3] = exponents[0] | exponents[1] << 8 | exponents[2] << 16;
    w[4] = child_base;
    w[5] = tri_base;
    w[6] = pack(meta, 0);
    w[7] = pack(meta, 1);
    for a in 0..3 {
        w[8 + 2 * a] = pack(&lo[a], 0);
        w[9 + 2 * a] = pack(&lo[a], 1);
        w[14 + 2 * a] = pack(&hi[a], 0);
        w[15 + 2 * a] = pack(&hi[a], 1);
    }
    w
}

/// The software rays' source with the flattened world composed in: its traversal appended and
/// called from gi_traverse's seam, and with `RENDERLAB_GI_WIDE_COUNT=1` the per-ray node and
/// triangle counts on both structures. Unchanged when the switch is off.
pub(crate) fn shader_text(rays: String) -> String {
    if !on() {
        return rays;
    }
    const SEAM: &str = "    // gi_wide: the static world's flattened BVH (src/gi_wide_bvh.rs) seam\n";
    assert_eq!(rays.matches(SEAM).count(), 1, "gi_traverse's wide seam");
    let mut text = rays.replacen(
        SEAM,
        "    gi_wide_traverse(origin,direction,world_inv,tmin,any_hit,&best);\n    if any_hit && best.hit { return best; }\n",
        1,
    );
    if counting() {
        for (from, to) in [
            ("        let n=nodes+child*16u;\n", "        gi_count_nodes+=1u;\n        let n=nodes+child*16u;\n"),
            ("        let n=top+item*GI_TOP_NODE;\n", "        gi_count_nodes+=1u;\n        let n=top+item*GI_TOP_NODE;\n"),
            ("                let h=gi_triangle(o,d,a,b,c,tmin,gi_limit(*best,i));\n", "                gi_count_tris+=select(1u,2u,two);\n                let h=gi_triangle(o,d,a,b,c,tmin,gi_limit(*best,i));\n"),
            ("fn gi_traverse(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32, any_hit:bool) -> GiHit {", "fn gi_traverse_counted(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32, any_hit:bool) -> GiHit {"),
        ] {
            assert_eq!(text.matches(from).count(), 1, "RENDERLAB_GI_WIDE_COUNT seam {from}");
            text = text.replacen(from, to, 1);
        }
        text.push_str(
            "\nfn gi_traverse(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32, any_hit:bool) -> GiHit {\n    \
             gi_count_nodes=0u;gi_count_tris=0u;\n    let h=gi_traverse_counted(origin,direction,tmin,tmax,any_hit);\n    \
             let k=select(0u,3u,any_hit);\n    atomicAdd(&gi_work[360u+k],1u);atomicAdd(&gi_work[361u+k],gi_count_nodes);atomicAdd(&gi_work[362u+k],gi_count_tris);\n    \
             return h;\n}\n",
        );
        text.push_str("var<private> gi_count_nodes:u32;\nvar<private> gi_count_tris:u32;\nconst GI_WIDE_COUNT:bool=true;\n");
    } else {
        text.push_str("const GI_WIDE_COUNT:bool=false;\nvar<private> gi_count_nodes:u32;\nvar<private> gi_count_tris:u32;\n");
    }
    text.push('\n');
    text.push_str(&crate::program::engine_shader("gi_rays_wide.wgsl", include_str!("../shaders/gi_rays_wide.wgsl")));
    text
}

/// Counter words 360-365 as (closest rays, nodes, triangles, any-hit rays, nodes, triangles),
/// read with the cache's other counters (`RENDERLAB_GI_WIDE_COUNT`).
pub(crate) const COUNT_WORDS: std::ops::Range<usize> = 360..366;

/// Hides the baked instances from the two-level structure after `instance_bvh::place` built its
/// top level: rewrites that top level over the movers alone (`baked[i]` false), its leaves the
/// movers' own record indices, so refits keep working; with no movers, a root with no children.
/// Writes the flattened world's bases into the header.
pub(crate) fn place_movers(head: &mut [u32], header: usize, stride: usize, baked: &[bool], bases: (u32, u32)) {
    use crate::instance_bvh::{EMPTY, LEAF, NODE_WORDS as BINARY, WIDE_WORDS, build, wide};
    head[HEADER_NODES] = bases.0;
    head[HEADER_TRIS] = bases.1;
    let base = head[1] as usize;
    if base == 0 {
        return;
    }
    let movers: Vec<usize> = (0..baked.len()).filter(|&i| !baked[i]).collect();
    let boxes: Vec<([f32; 3], [f32; 3])> = movers
        .iter()
        .map(|&i| {
            let r = header + i * stride;
            let f = |k: usize| f32::from_bits(head[r + k]);
            ([f(12), f(13), f(14)], [f(15), f(16), f(17)])
        })
        .collect();
    let (node_words, ids) = if wide() { (WIDE_WORDS, 24..28) } else { (BINARY, 12..14) };
    match movers.len() {
        0 => {
            // One node whose children are all empty.
            let node = &mut head[base..base + node_words];
            node.fill(f32::NEG_INFINITY.to_bits());
            if wide() {
                node[0..12].fill(f32::INFINITY.to_bits());
            } else {
                for c in 0..2 {
                    node[c * 6..c * 6 + 3].fill(f32::INFINITY.to_bits());
                }
            }
            node[ids].fill(EMPTY);
            head[2] = 0;
        }
        1 => head[2] = LEAF | movers[0] as u32,
        _ => {
            let mut nodes = build(&boxes);
            for node in nodes.chunks_exact_mut(node_words) {
                for id in &mut node[ids.clone()] {
                    if *id != EMPTY && *id & LEAF != 0 {
                        *id = LEAF | movers[(*id & !LEAF) as usize] as u32;
                    }
                }
            }
            head[base..base + nodes.len()].copy_from_slice(&nodes);
            head[2] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_grid(n: usize) -> Vec<Tri> {
        let mut tris = vec![];
        for j in 0..n {
            for i in 0..n {
                let p = |x: usize, z: usize| Vec3::new(x as f32 * 0.37, ((x * 7 + z * 3) % 5) as f32 * 0.1, z as f32 * 0.41);
                for (a, b, c) in [(p(i, j), p(i + 1, j), p(i + 1, j + 1)), (p(i, j), p(i + 1, j + 1), p(i, j + 1))] {
                    let index = tris.len() as u32 * 3;
                    tris.push(Tri { a, b, c, inst: 0, index, no_shadow: false });
                }
            }
        }
        tris
    }

    /// Decodes a node's child box as the shader does.
    fn child_box(w: &[u32], k: usize) -> (Vec3, Vec3) {
        let p = Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]));
        let scale = Vec3::from_array(std::array::from_fn(|a| f32::from_bits(((w[3] >> (8 * a)) & 0xff) << 23)));
        let byte = |base: usize, k: usize| ((w[base + k / 4] >> (8 * (k % 4))) & 0xff) as f32;
        let lo = Vec3::new(byte(8, k), byte(10, k), byte(12, k));
        let hi = Vec3::new(byte(14, k), byte(16, k), byte(18, k));
        (p + lo * scale, p + hi * scale)
    }

    #[test]
    fn every_triangle_sits_in_one_leaf_inside_every_decoded_box_above_it() {
        let tris = quad_grid(40);
        let wide = build(&tris);
        assert_eq!(wide.triangles, tris.len());
        // Walk the tree: each leaf slot's decoded box holds its triangles; each inner slot's box
        // holds its child's whole bounds (checked through the triangles reached below it).
        let mut seen = vec![0u32; tris.len()];
        let mut stack = vec![(0usize, Vec3::splat(f32::NEG_INFINITY), Vec3::splat(f32::INFINITY))];
        while let Some((node, plo, phi)) = stack.pop() {
            let w = &wide.words[node * NODE_WORDS..(node + 1) * NODE_WORDS];
            let meta = |k: usize| ((w[6 + k / 4] >> (8 * (k % 4))) & 0xff) as u8;
            for k in 0..8 {
                let m = meta(k);
                if m == 0 {
                    continue;
                }
                let (lo, hi) = child_box(w, k);
                if m & 0x80 != 0 {
                    let count = ((m >> 5) & 3) as usize + 1;
                    for j in 0..count {
                        let t = wide.tri_word + (w[5] as usize + (m & 31) as usize + j) * TRI_WORDS;
                        let index = wide.words[t + 10] as usize / 3;
                        seen[index] += 1;
                        for v in [&tris[index].a, &tris[index].b, &tris[index].c] {
                            assert!(v.cmpge(lo).all() && v.cmple(hi).all(), "triangle outside its leaf box");
                            assert!(v.cmpge(plo).all() && v.cmple(phi).all(), "triangle outside an ancestor box");
                        }
                    }
                } else {
                    stack.push((w[4] as usize + (m & 7) as usize, lo.max(plo), hi.min(phi)));
                }
            }
        }
        assert!(seen.iter().all(|&s| s == 1), "every triangle exactly once");
        assert!(wide.nodes * 8 >= tris.len() / LEAF_MAX, "eight-wide nodes");
    }
}
