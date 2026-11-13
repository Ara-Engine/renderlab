//! M7-R1n: the geometry software reflection rays trace on the non-RT path
//! (docs/m7-r1n.md; shaders/reflections_software.wgsl traverses it).
//!
//! Each uploaded mesh of a non-RT visibility-buffer renderer gets a BVH2 over its drawn
//! triangles (or, under `RENDERLAB_RS_BVH=4`, GI's binned-SAH BVH4, `bvh4`), built once at
//! upload and never refit. Leaves keep each triangle's primitive
//! index in the mesh's own index buffer and its corners in index order, so a hit is the same
//! `(placement + 1, primitive, barycentrics)` record a hardware ray query returns and R1's
//! hit reconstruction (`rl_hit_input`) shades it unchanged. Each frame packs the meshes and
//! a flat instance list (world-to-object rows, world bounds, the placement slot) into one
//! storage buffer, with a top-level BVH over the instance records (src/instance_bvh.rs,
//! U-RTVIS). Rays are per-pixel work items traversed from the top: no cells, no hit
//! registration, no shared projection (P0's W8 and C7, docs/m7-p0-claims.md).
use crate::Mat4;

/// Largest mesh that gets software reflection geometry (about 64 bytes a triangle): larger meshes
/// are not hit by non-RT reflection rays (docs/m7-r1n.md, gaps).
pub(crate) const MAX_TRIANGLES: usize = 1 << 19;
/// Triangles per leaf at most; the WGSL leaf tag holds the count in three bits.
const LEAF_TRIANGLES: usize = 4;
/// Words per node: child 0 bounds (lo, hi), child 1 bounds, then the two tagged child IDs.
const NODE_WORDS: usize = 16;
/// Words per triangle: corner a, edges b - a and c - a, the drawn primitive index, then two
/// pad words. P-NRT: every node, triangle and instance record starts on a 16-byte boundary,
/// so the WGSL traversal reads them as whole vec4 words.
const TRIANGLE_WORDS: usize = 12;
/// A child ID's leaf tag: `LEAF | first_triangle << 3 | count` (BVH4: `<< 4`, up to eight).
const LEAF: u32 = 0x8000_0000;
/// A BVH4 node's absent child (renderlab_asset::cluster_bvh::EMPTY; WGSL RS_EMPTY).
const EMPTY: u32 = u32::MAX;

/// `RENDERLAB_RS_BVH=4` (candidate, off by default; TLAS step 2): every mesh's reflection BLAS is
/// GI's binned-SAH BVH4 (`renderlab_asset::near_field::build_bvh_sah_order`, docs/m7-gc1.md):
/// 16-word quantized nodes (origin, step, four tagged children, the children's lower and upper
/// bytes an axis a word, as src/gi_bvh.rs packs them), leaves of up to eight of today's triangle
/// records. Each triangle's record and test are unchanged, so a hit's (primitive, barycentrics,
/// t) are; only equal-distance hits may resolve in another order. Unset: the median BVH2.
pub(crate) fn bvh4() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RS_BVH").is_ok_and(|v| v == "4"))
}
/// Words before the instance records; WGSL RS_HEADER_WORDS must match. Word 0 is the count,
/// words 1 and 2 the top level's base word and root (src/instance_bvh.rs).
pub(crate) const HEADER_WORDS: usize = 8;
/// One instance record; WGSL RS_INSTANCE_WORDS must match. Words 0-11 world-to-object rows,
/// 12-14 world lo, 15-17 world hi, 18 node base word, 19 triangle base word, 20 root child,
/// 21 placement slot.
pub(crate) const INSTANCE_WORDS: usize = 24;

/// One mesh's software reflection geometry, in object space.
pub(crate) struct Blas {
    nodes: Vec<u32>,
    triangles: Vec<u32>,
    root: u32,
    lo: [f32; 3],
    hi: [f32; 3],
    /// `RENDERLAB_TLAS=tight`: the object boxes its instances' world boxes are made from
    /// (`instance_bvh::hull`; empty without the switch).
    hull: Vec<crate::instance_bvh::Bounds>,
}

impl Blas {
    /// The mesh's reflection BLAS over `indices` (a triangle list into `positions`): the BVH4
    /// under `RENDERLAB_RS_BVH=4` (`bvh4`), else the BVH2. None for an empty list.
    pub(crate) fn build(positions: &[[f32; 3]], indices: &[u32]) -> Option<Self> {
        if bvh4() {
            Self::build4(positions, indices)
        } else {
            Self::build2(positions, indices)
        }
    }
    /// The median-split BVH2, whatever the switch (tests that read its layout build it).
    /// Deterministic: ties on the split axis break by triangle index.
    pub(crate) fn build2(positions: &[[f32; 3]], indices: &[u32]) -> Option<Self> {
        let count = indices.len() / 3;
        if count == 0 || count >= (1 << 28) {
            return None;
        }
        let corner = |t: usize, k: usize| positions[indices[t * 3 + k] as usize];
        let centroids: Vec<[f32; 3]> = (0..count)
            .map(|t| {
                std::array::from_fn(|a| (corner(t, 0)[a] + corner(t, 1)[a] + corner(t, 2)[a]) / 3.0)
            })
            .collect();
        let mut builder = Builder {
            positions,
            indices,
            centroids: &centroids,
            nodes: Vec::new(),
            triangles: Vec::with_capacity(count * TRIANGLE_WORDS),
        };
        let mut order: Vec<u32> = (0..count as u32).collect();
        let (root, lo, hi) = builder.subtree(&mut order);
        Some(Self {
            nodes: builder.nodes,
            triangles: builder.triangles,
            root,
            lo,
            hi,
            hull: hull(positions, indices),
        })
    }
    /// The `RENDERLAB_RS_BVH=4` build: GI's SAH BVH4 topology with each leaf triangle written as
    /// today's record, in the leaf order the builder returns. None where the asset builder
    /// rejects the list (an index or position out of range).
    fn build4(positions: &[[f32; 3]], indices: &[u32]) -> Option<Self> {
        // The leaf tag keeps four bits for the count, 27 for the first triangle.
        let count = indices.len() / 3;
        if count == 0 || count >= 1 << 27 {
            return None;
        }
        let (bvh, order) =
            crate::asset::near_field::build_bvh_sah_order(positions, indices).ok()?;
        let mut triangles = Vec::with_capacity(order.len() * TRIANGLE_WORDS);
        for &t in &order {
            triangle_record(positions, indices, t, &mut triangles);
        }
        // Leaves copy their triangles' indices contiguously in leaf order: leaf L's first
        // triangle is its index span's offset / 3.
        let child = |c: u32| -> u32 {
            if c == EMPTY || c & LEAF == 0 {
                return c;
            }
            let span = bvh.leaves[(c & !LEAF) as usize].indices;
            LEAF | (span.offset / 3) << 4 | span.count / 3
        };
        let lane = |bytes: [u8; 4]| u32::from_le_bytes(bytes);
        let mut nodes = Vec::with_capacity(bvh.nodes.len() * NODE_WORDS);
        for n in &bvh.nodes {
            nodes.extend(n.origin.iter().chain(&n.step).map(|v| v.to_bits()));
            nodes.extend(n.children.map(child));
            nodes.extend(n.lower.map(lane));
            nodes.extend(n.upper.map(lane));
        }
        let (lo, hi) = bounds(indices.iter().map(|&i| positions[i as usize]));
        Some(Self {
            nodes,
            triangles,
            root: child(bvh.levels[0].root),
            lo,
            hi,
            hull: hull(positions, indices),
        })
    }
    fn words(&self) -> usize {
        self.nodes.len() + self.triangles.len()
    }
    /// Bytes this mesh adds to the packed buffer (logged at upload in test builds).
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> u64 {
        (self.words() * 4) as u64
    }
}

struct Builder<'a> {
    positions: &'a [[f32; 3]],
    indices: &'a [u32],
    centroids: &'a [[f32; 3]],
    nodes: Vec<u32>,
    triangles: Vec<u32>,
}
impl Builder<'_> {
    /// The tagged child ID of the subtree over `order`, and its bounds.
    fn subtree(&mut self, order: &mut [u32]) -> (u32, [f32; 3], [f32; 3]) {
        if order.len() <= LEAF_TRIANGLES {
            return self.leaf(order);
        }
        let (lo, hi) = bounds(order.iter().map(|&t| self.centroids[t as usize]));
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap();
        let mid = order.len() / 2;
        let centroids = self.centroids;
        order.select_nth_unstable_by(mid, |&a, &b| {
            centroids[a as usize][axis]
                .total_cmp(&centroids[b as usize][axis])
                .then(a.cmp(&b))
        });
        let node = self.nodes.len() / NODE_WORDS;
        self.nodes.extend([0; NODE_WORDS]);
        let (left, right) = order.split_at_mut(mid);
        let a = self.subtree(left);
        let b = self.subtree(right);
        let words = &mut self.nodes[node * NODE_WORDS..(node + 1) * NODE_WORDS];
        for (i, v) in [a.1, a.2, b.1, b.2].iter().flatten().enumerate() {
            words[i] = v.to_bits();
        }
        words[12] = a.0;
        words[13] = b.0;
        let lo = std::array::from_fn(|i| a.1[i].min(b.1[i]));
        let hi = std::array::from_fn(|i| a.2[i].max(b.2[i]));
        (node as u32, lo, hi)
    }
    fn leaf(&mut self, order: &[u32]) -> (u32, [f32; 3], [f32; 3]) {
        let first = (self.triangles.len() / TRIANGLE_WORDS) as u32;
        let mut points = Vec::with_capacity(order.len() * 3);
        for &t in order {
            triangle_record(self.positions, self.indices, t, &mut self.triangles);
            points
                .extend((0..3).map(|k| self.positions[self.indices[t as usize * 3 + k] as usize]));
        }
        let (lo, hi) = bounds(points.into_iter());
        (LEAF | first << 3 | order.len() as u32, lo, hi)
    }
}

/// Triangle `t`'s record: corner a, edges b - a and c - a (index order), its primitive index,
/// two pad words.
fn triangle_record(positions: &[[f32; 3]], indices: &[u32], t: u32, out: &mut Vec<u32>) {
    let [a, b, c] = std::array::from_fn(|k| positions[indices[t as usize * 3 + k] as usize]);
    let e1: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let e2: [f32; 3] = std::array::from_fn(|i| c[i] - a[i]);
    out.extend([a, e1, e2].iter().flatten().map(|v| v.to_bits()));
    out.extend([t, 0, 0]);
}
/// `RENDERLAB_TLAS=tight`: the object boxes a mesh's instances' world boxes are made from.
fn hull(positions: &[[f32; 3]], indices: &[u32]) -> Vec<crate::instance_bvh::Bounds> {
    if !crate::instance_bvh::tlas().tight {
        return vec![];
    }
    let points: Vec<[f32; 3]> = indices.iter().map(|&i| positions[i as usize]).collect();
    crate::instance_bvh::hull(&points)
}

fn bounds(points: impl Iterator<Item = [f32; 3]>) -> ([f32; 3], [f32; 3]) {
    points.fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(lo, hi), p| {
            (
                std::array::from_fn(|i| lo[i].min(p[i])),
                std::array::from_fn(|i| hi[i].max(p[i])),
            )
        },
    )
}

/// The packed meshes: each distinct BLAS once, after `instances` instance records.
pub(crate) struct Packed {
    /// Per distinct mesh, its node and triangle base words.
    pub bases: Vec<[u32; 2]>,
    pub words: Vec<u32>,
}
/// Words of the header, `instances` records and their top level, before the meshes.
pub(crate) fn records_words(instances: usize) -> usize {
    HEADER_WORDS + instances * INSTANCE_WORDS + crate::instance_bvh::reserve(instances)
}
/// Packs `meshes` (distinct) behind room for `instances` records and their top level; the
/// header, records and top level stay zero until `instance_words` fills them.
pub(crate) fn pack(meshes: &[&Blas], instances: usize) -> Packed {
    let mut at = records_words(instances);
    let total = at + meshes.iter().map(|b| b.words()).sum::<usize>();
    let mut words = vec![0u32; total.max(HEADER_WORDS)];
    let bases = meshes
        .iter()
        .map(|b| {
            let nodes = at;
            words[at..at + b.nodes.len()].copy_from_slice(&b.nodes);
            at += b.nodes.len();
            let triangles = at;
            words[at..at + b.triangles.len()].copy_from_slice(&b.triangles);
            at += b.triangles.len();
            [nodes as u32, triangles as u32]
        })
        .collect();
    Packed { bases, words }
}

/// The header and instance records: per traced instance its placement slot, model, mesh and
/// that mesh's bases from `pack`. Singular models are skipped (they draw nothing). Under
/// `RENDERLAB_TLAS=tight` a record's world box comes from its mesh's hull, and is copied from
/// `previous` (the last frame's words for the same pack, or empty) when the record's rows and
/// mesh words are unchanged, so only moved instances pay the hull's 128 corners.
pub(crate) fn instance_words(
    records: &[(u32, Mat4, &Blas, [u32; 2])],
    previous: &[u32],
) -> Vec<u32> {
    let mut out = vec![0u32; HEADER_WORDS];
    let mut count = 0u32;
    let tight = crate::instance_bvh::tlas().tight;
    for &(slot, model, blas, [nodes, triangles]) in records {
        if model.determinant() == 0.0 {
            continue;
        }
        let inverse = model.inverse();
        if !inverse.is_finite() {
            continue;
        }
        let at = out.len();
        let rows = inverse.transpose();
        for r in 0..3 {
            out.extend(rows.col(r).to_array().map(f32::to_bits));
        }
        let mesh = [nodes, triangles, blas.root, slot];
        let kept = previous
            .get(at..at + INSTANCE_WORDS)
            .filter(|p| tight && p[..12] == out[at..at + 12] && p[18..22] == mesh)
            .map(|p| p[12..18].to_vec());
        match kept {
            Some(words) => out.extend(words),
            None => {
                let (lo, hi) = if tight && !blas.hull.is_empty() {
                    world_bounds(&model, &blas.hull)
                } else {
                    world_bounds(&model, &[(blas.lo, blas.hi)])
                };
                out.extend(lo.iter().chain(&hi).map(|v| v.to_bits()));
            }
        }
        out.extend(mesh);
        out.extend([0, 0]);
        count += 1;
    }
    out[0] = count;
    out
}

/// The world box of object `boxes` under `model`, widened by a relative margin that covers
/// the traversal's f32 transform of the ray (the BVH is tested in object space; this box
/// only rejects whole instances).
fn world_bounds(model: &Mat4, boxes: &[crate::instance_bvh::Bounds]) -> ([f32; 3], [f32; 3]) {
    let (lo, hi) = crate::instance_bvh::world_box(model, boxes);
    let margin = (0..3)
        .map(|i| hi[i].abs().max(lo[i].abs()).max(hi[i] - lo[i]))
        .fold(0.0f32, f32::max)
        * 1e-5;
    (lo.map(|v| v - margin), hi.map(|v| v + margin))
}

#[cfg(test)]
#[path = "reflection_bvh4_tests.rs"]
mod bvh4_tests;
#[cfg(test)]
#[path = "reflection_tlas_tests.rs"]
mod tlas_tests;
#[cfg(test)]
mod tests {
    use super::*;

    /// CPU replay of the WGSL traversal (shaders/reflections_software.wgsl): the closest
    /// hit's (primitive, u, v, t) in object space.
    pub(super) fn closest(b: &Blas, o: [f32; 3], d: [f32; 3]) -> Option<(u32, f32, f32, f32)> {
        let v =
            |w: &[u32], i: usize| -> [f32; 3] { std::array::from_fn(|k| f32::from_bits(w[i + k])) };
        let mut best: Option<(u32, f32, f32, f32)> = None;
        let mut stack = vec![b.root];
        while let Some(child) = stack.pop() {
            if child & LEAF != 0 {
                let first = ((child & !LEAF) >> 3) as usize;
                for k in 0..(child & 7) as usize {
                    let x = (first + k) * TRIANGLE_WORDS;
                    let (a, e1, e2) = (
                        v(&b.triangles, x),
                        v(&b.triangles, x + 3),
                        v(&b.triangles, x + 6),
                    );
                    let (a, e1, e2, o, d) = (
                        glam::Vec3::from(a),
                        glam::Vec3::from(e1),
                        glam::Vec3::from(e2),
                        glam::Vec3::from(o),
                        glam::Vec3::from(d),
                    );
                    let p = d.cross(e2);
                    let det = e1.dot(p);
                    if det.abs() < 1e-30 {
                        continue;
                    }
                    let s = o - a;
                    let u = s.dot(p) / det;
                    let q = s.cross(e1);
                    let w = d.dot(q) / det;
                    let t = e2.dot(q) / det;
                    if u >= 0.0
                        && w >= 0.0
                        && u + w <= 1.0
                        && t > 0.0
                        && best.is_none_or(|h| t < h.3)
                    {
                        best = Some((b.triangles[x + 9], u, w, t));
                    }
                }
                continue;
            }
            let n = child as usize * NODE_WORDS;
            for c in 0..2 {
                let (lo, hi) = (v(&b.nodes, n + c * 6), v(&b.nodes, n + c * 6 + 3));
                let (mut near, mut far) = (0.0f32, f32::INFINITY);
                for i in 0..3 {
                    let (a, z) = ((lo[i] - o[i]) / d[i], (hi[i] - o[i]) / d[i]);
                    near = near.max(a.min(z));
                    far = far.min(a.max(z));
                }
                if near <= far {
                    stack.push(b.nodes[n + 12 + c]);
                }
            }
        }
        best
    }

    /// A grid of quads: every triangle is found by a ray through its centroid, with its own
    /// primitive index and barycentrics in index-buffer corner order, and the tree's leaves
    /// hold each triangle exactly once.
    #[test]
    fn reflection_bvh_keeps_primitive_identity() {
        let n = 23;
        let mut positions = vec![];
        let mut indices = vec![];
        for y in 0..=n {
            for x in 0..=n {
                positions.push([x as f32, y as f32, ((x * 7 + y * 3) % 5) as f32 * 0.01]);
            }
        }
        for y in 0..n {
            for x in 0..n {
                let i = (y * (n + 1) + x) as u32;
                let w = n as u32 + 1;
                indices.extend([i, i + 1, i + w + 1, i, i + w + 1, i + w]);
            }
        }
        let b = Blas::build2(&positions, &indices).unwrap();
        let mut seen: Vec<u32> = b.triangles.chunks(TRIANGLE_WORDS).map(|t| t[9]).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..indices.len() as u32 / 3).collect::<Vec<_>>());
        for t in 0..indices.len() / 3 {
            let p = |k: usize| glam::Vec3::from(positions[indices[t * 3 + k] as usize]);
            // Off-centre so the barycentrics distinguish the corners.
            let target = p(0) * 0.5 + p(1) * 0.3 + p(2) * 0.2;
            let o = target + glam::Vec3::new(0.01, -0.02, 3.0);
            let d = target - o;
            let (prim, u, v, t_hit) = closest(&b, o.to_array(), d.to_array()).unwrap();
            assert_eq!(prim as usize, t, "triangle {t}");
            assert!((u - 0.3).abs() < 1e-4 && (v - 0.2).abs() < 1e-4, "{u} {v}");
            assert!((t_hit - 1.0).abs() < 1e-4);
        }
        // Deterministic: a rebuild is word-identical.
        let again = Blas::build2(&positions, &indices).unwrap();
        assert_eq!((b.nodes, b.triangles), (again.nodes, again.triangles));
    }

    /// A grid of `n` x `n` quads, gently bent so no box is perfectly flat everywhere.
    fn grid(n: u32) -> (Vec<[f32; 3]>, Vec<u32>) {
        let mut positions = vec![];
        let mut indices = vec![];
        for y in 0..=n {
            for x in 0..=n {
                let z = ((x * 7 + y * 3) % 5) as f32 * 0.01;
                positions.push([x as f32, y as f32, z]);
            }
        }
        for y in 0..n {
            for x in 0..n {
                let (i, w) = (y * (n + 1) + x, n + 1);
                indices.extend([i, i + 1, i + w + 1, i, i + w + 1, i + w]);
            }
        }
        (positions, indices)
    }

    /// The tree's depth below `child` (a leaf is 0).
    fn depth(b: &Blas, child: u32) -> usize {
        if child & LEAF != 0 {
            return 0;
        }
        let n = child as usize * NODE_WORDS;
        1 + depth(b, b.nodes[n + 12]).max(depth(b, b.nodes[n + 13]))
    }

    /// The traversal stack (RS_STACK = 32 in shaders/reflections_software.wgsl) holds at
    /// most depth + 1 children: it keeps one sibling per level. The median split keeps the
    /// depth at ceil(log2(triangles / 4)), so 32 holds every mesh under about 2^33
    /// triangles; half a million stay at 17.
    #[test]
    fn reflection_bvh_depth_fits_the_traversal_stack() {
        let (positions, indices) = grid(512);
        let triangles = indices.len() / 3;
        let b = Blas::build2(&positions, &indices).unwrap();
        let d = depth(&b, b.root);
        let balanced = (triangles.div_ceil(LEAF_TRIANGLES) as f64).log2().ceil() as usize;
        assert!(
            d <= balanced,
            "depth {d} for {triangles} triangles (balanced {balanced})"
        );
        assert!(d < 32, "depth {d} overflows the 32-entry stack");
    }

    /// The closest hit under the shader's own box arithmetic: slabs as (lo - o) * inv, or
    /// with `fma` (RENDERLAB_RS_BOX=fma) as fma(lo, inv, -o * inv), near child first.
    fn closest_shader(b: &Blas, o: glam::Vec3, d: glam::Vec3, fma: bool) -> Option<(u32, u32)> {
        let v = |w: &[u32], i: usize| -> glam::Vec3 {
            glam::Vec3::from_array(std::array::from_fn(|k| f32::from_bits(w[i + k])))
        };
        let inv = d.map(|c| {
            1.0 / if c.abs() < 1e-30 {
                1e-30f32.copysign(c)
            } else {
                c
            }
        });
        let oi = o * inv;
        let slab = |lo: glam::Vec3, hi: glam::Vec3, tmax: f32| -> f32 {
            let (a, z) = if fma {
                (
                    glam::Vec3::new(
                        lo.x.mul_add(inv.x, -oi.x),
                        lo.y.mul_add(inv.y, -oi.y),
                        lo.z.mul_add(inv.z, -oi.z),
                    ),
                    glam::Vec3::new(
                        hi.x.mul_add(inv.x, -oi.x),
                        hi.y.mul_add(inv.y, -oi.y),
                        hi.z.mul_add(inv.z, -oi.z),
                    ),
                )
            } else {
                ((lo - o) * inv, (hi - o) * inv)
            };
            let near = a.min(z).max_element().max(0.0);
            let far = a.max(z).min_element().min(tmax);
            if near <= far { near } else { f32::MAX }
        };
        let mut best: Option<(u32, f32)> = None;
        let mut stack = vec![b.root];
        while let Some(mut child) = stack.pop() {
            loop {
                let tmax = best.map_or(f32::MAX, |h| h.1);
                if child & LEAF != 0 {
                    let first = ((child & !LEAF) >> 3) as usize;
                    for k in 0..(child & 7) as usize {
                        let x = (first + k) * TRIANGLE_WORDS;
                        let (a, e1, e2) = (
                            v(&b.triangles, x),
                            v(&b.triangles, x + 3),
                            v(&b.triangles, x + 6),
                        );
                        let p = d.cross(e2);
                        let det = e1.dot(p);
                        if det.abs() < 1e-30 {
                            continue;
                        }
                        let s = o - a;
                        let u = s.dot(p) / det;
                        let q = s.cross(e1);
                        let w = d.dot(q) / det;
                        let t = e2.dot(q) / det;
                        if u >= 0.0
                            && w >= 0.0
                            && u + w <= 1.0
                            && t > 0.0
                            && t < best.map_or(f32::MAX, |h| h.1)
                        {
                            best = Some((b.triangles[x + 9], t));
                        }
                    }
                    break;
                }
                let n = child as usize * NODE_WORDS;
                let t0 = slab(v(&b.nodes, n), v(&b.nodes, n + 3), tmax);
                let t1 = slab(v(&b.nodes, n + 6), v(&b.nodes, n + 9), tmax);
                let (c0, c1) = (b.nodes[n + 12], b.nodes[n + 13]);
                match (t0 <= tmax, t1 <= tmax) {
                    (true, true) => {
                        let near_first = t0 <= t1;
                        stack.push(if near_first { c1 } else { c0 });
                        child = if near_first { c0 } else { c1 };
                    }
                    (true, false) => child = c0,
                    (false, true) => child = c1,
                    (false, false) => break,
                }
            }
        }
        best.map(|(prim, t)| (prim, t.to_bits()))
    }

    /// RENDERLAB_RS_BOX=fma changes only the rounding of the slab distances. Over rays at
    /// every angle, grazing ones included, the closest hit (primitive and t's bits) is the
    /// same as with the subtract-and-multiply form.
    #[test]
    fn fma_box_test_finds_the_same_closest_hits() {
        let (positions, indices) = grid(48);
        let b = Blas::build2(&positions, &indices).unwrap();
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        let (mut rays, mut hits, mut differ) = (0, 0, 0);
        for _ in 0..200_000 {
            let target = glam::Vec3::new(next() * 48.0, next() * 48.0, 0.02);
            // Elevations from grazing (a few thousandths) to steep.
            let elevation = 0.002 + next().powi(3) * 1.5;
            let azimuth = next() * std::f32::consts::TAU;
            let back = glam::Vec3::new(
                azimuth.cos() * elevation.cos(),
                azimuth.sin() * elevation.cos(),
                elevation.sin(),
            );
            let o = target + back * (1.0 + next() * 40.0);
            let d = target - o;
            let (plain, fused) = (
                closest_shader(&b, o, d, false),
                closest_shader(&b, o, d, true),
            );
            rays += 1;
            hits += usize::from(plain.is_some());
            differ += usize::from(plain != fused);
        }
        println!("RS_BOX=fma: {differ} of {rays} rays' closest hits differ ({hits} hit)");
        assert_eq!(differ, 0, "{differ} of {rays} rays' closest hits differ");
    }

    /// Instance records hold the inverse rows and a world box enclosing the transformed mesh.
    #[test]
    fn reflection_instance_records() {
        let b = Blas::build2(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], &[0, 1, 2]).unwrap();
        let model = Mat4::from_scale_rotation_translation(
            glam::Vec3::splat(2.0),
            glam::Quat::from_rotation_y(0.5),
            glam::Vec3::new(3.0, -1.0, 4.0),
        );
        let packed = pack(&[&b], 1);
        let words = instance_words(
            &[
                (5, model, &b, packed.bases[0]),
                (6, Mat4::ZERO, &b, packed.bases[0]),
            ],
            &[],
        );
        assert_eq!(words.len(), HEADER_WORDS + INSTANCE_WORDS);
        assert_eq!(words[0], 1);
        let r = &words[HEADER_WORDS..];
        let f = |i: usize| f32::from_bits(r[i]);
        let world = model.transform_point3(glam::Vec3::new(1.0, 0.0, 0.0));
        let object: [f32; 3] = std::array::from_fn(|k| {
            f(k * 4) * world.x + f(k * 4 + 1) * world.y + f(k * 4 + 2) * world.z + f(k * 4 + 3)
        });
        for (a, e) in object.iter().zip([1.0, 0.0, 0.0]) {
            assert!((a - e).abs() < 1e-5);
        }
        for k in 0..3 {
            assert!(f(12 + k) <= world[k] && world[k] <= f(15 + k));
        }
        assert_eq!(
            &r[18..22],
            &[packed.bases[0][0], packed.bases[0][1], b.root, 5]
        );
    }
}
