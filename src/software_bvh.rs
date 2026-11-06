//! Test-only software BVH for the non-RT ray backend probe (docs/non-rt-path.md, unit N0).
//!
//! One BVH8 per mesh: a binned-SAH binary tree (32 bins, leaves of at most `max_leaf`
//! triangles) collapsed to 8-wide nodes with uncompressed f32 child boxes. Leaves reference
//! triangles through a reordered triangle list into the mesh's own vertex and index buffers;
//! no triangle is copied. The CPU traversal here and `shaders/software_bvh.wgsl` share the
//! node layout, the traversal order and the watertight triangle test, so they agree hit for
//! hit up to f32 division rounding. The f64 replay helpers classify mismatches against the
//! hardware path.
use glam::{DMat4, DVec3, Mat4, Vec3};
use std::time::Instant;

pub const BINS: usize = 32;
pub const WIDTH: usize = 8;
/// Child kind word: 0 is an empty slot (children are packed first), `INNER` an internal
/// node, and 1..=max_leaf a leaf of that many triangles.
pub const INNER: u32 = u32::MAX;
pub const NONE: u32 = u32::MAX;
pub const CHILD_BYTES: usize = 32;
pub const NODE_BYTES: usize = CHILD_BYTES * WIDTH;
pub const INSTANCE_BYTES: usize = 96;
/// Relative widening of slab distances, as in the WGSL: rounding in the slab arithmetic
/// never culls a box the ray reaches (robust traversal, Ize 2013).
pub const WIDEN: f32 = 1e-6;
/// The traversal WGSL; everything after `COMPOSED` needs the traced soft module.
pub const WGSL: &str = include_str!("../shaders/software_bvh.wgsl");
const COMPOSED: &str = "// ---- composed traced modules only ----";

/// One mesh's triangles: f32 object-space positions and the u32 index list, in the
/// renderer's order (triangle t is indices 3t..3t+3).
#[derive(Clone, Copy)]
pub struct Triangles<'a> {
    pub positions: &'a [[f32; 3]],
    pub indices: &'a [u32],
}
impl Triangles<'_> {
    pub fn len(&self) -> usize {
        self.indices.len() / 3
    }
    pub fn vertices(&self, t: u32) -> [Vec3; 3] {
        let i = 3 * t as usize;
        [0, 1, 2].map(|k| Vec3::from_array(self.positions[self.indices[i + k] as usize]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub lo: Vec3,
    pub hi: Vec3,
}
impl Aabb {
    pub const EMPTY: Self = Self {
        lo: Vec3::splat(f32::INFINITY),
        hi: Vec3::splat(f32::NEG_INFINITY),
    };
    fn grow(&mut self, other: &Self) {
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
    }
    fn point(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }
    fn area(&self) -> f32 {
        let d = (self.hi - self.lo).max(Vec3::ZERO);
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
    pub fn contains(&self, other: &Self) -> bool {
        self.lo.cmple(other.lo).all() && self.hi.cmpge(other.hi).all()
    }
    fn padded(&self, pad: f32) -> Self {
        Self {
            lo: self.lo - pad,
            hi: self.hi + pad,
        }
    }
}

/// f32 unit in the last place at magnitude `m` (normal range).
pub fn ulp(m: f32) -> f32 {
    let m = m.abs().max(f32::MIN_POSITIVE);
    f32::from_bits(m.to_bits() + 1) - m
}

/// One BVH8 child, as the GPU reads it: lo.xyz, index, hi.xyz, kind (32 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Child {
    pub lo: [f32; 3],
    /// An internal child's node, or a leaf's first entry in `Bvh::prims`.
    pub index: u32,
    pub hi: [f32; 3],
    pub kind: u32,
}
impl Child {
    const EMPTY: Self = Self {
        lo: [0.0; 3],
        index: 0,
        hi: [0.0; 3],
        kind: 0,
    };
}
pub type Node = [Child; WIDTH];

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Stats {
    pub triangles: usize,
    pub nodes: usize,
    pub leaves: usize,
    /// Mean children per node and triangles per leaf.
    pub mean_children: f64,
    pub mean_leaf: f64,
    pub depth: u32,
    /// Closest-hit traversal stack entries this tree can need.
    pub stack: u32,
    /// Binned-SAH binary build and 8-wide collapse, single threaded.
    pub build_ms: f64,
    /// Node and leaf-reference bytes on the GPU (vertex and index buffers are shared).
    pub bytes: usize,
}

pub struct Bvh {
    /// Node 0 is the root. Leaf children index `prims`; internal children index `nodes`.
    pub nodes: Vec<Node>,
    /// Triangle indices in leaf order.
    pub prims: Vec<u32>,
    /// Unpadded object-space bounds of every triangle.
    pub bounds: Aabb,
    /// Build-time box padding: `pad_ulps` ulps of the largest coordinate magnitude, so the
    /// f32 triangle test never accepts a ray its leaf box culled.
    pub pad: f32,
    pub stats: Stats,
}

/// Binary nodes cover the contiguous `order` range [first, first + count).
enum Binary {
    Leaf {
        bounds: Aabb,
        first: u32,
        count: u32,
    },
    Inner {
        bounds: Aabb,
        first: u32,
        count: u32,
        children: [u32; 2],
    },
}
impl Binary {
    fn bounds(&self) -> &Aabb {
        match self {
            Self::Leaf { bounds, .. } | Self::Inner { bounds, .. } => bounds,
        }
    }
    fn range(&self) -> (u32, u32) {
        match *self {
            Self::Leaf { first, count, .. } | Self::Inner { first, count, .. } => (first, count),
        }
    }
}

fn bin(c: f32, lo: f32, scale: f32) -> usize {
    (((c - lo) * scale) as usize).min(BINS - 1)
}

/// In-place partition; returns the count of `left` entries, which come first.
fn partition(order: &mut [u32], left: impl Fn(u32) -> bool) -> usize {
    let (mut i, mut j) = (0, order.len());
    while i < j {
        if left(order[i]) {
            i += 1;
        } else {
            j -= 1;
            order.swap(i, j);
        }
    }
    i
}

/// The SAH split of `order` (all entries share `bounds` and centroid bounds `centre`), or
/// None for a leaf. Leaf cost is the triangle count; a split costs one traversal step plus
/// the area-weighted child counts.
fn split(
    boxes: &[Aabb],
    centroids: &[Vec3],
    order: &mut [u32],
    bounds: &Aabb,
    centre: &Aabb,
    max_leaf: usize,
) -> Option<usize> {
    let count = order.len();
    if count <= 1 {
        return None;
    }
    let extent = centre.hi - centre.lo;
    let scale = Vec3::select(extent.cmpgt(Vec3::ZERO), BINS as f32 / extent, Vec3::ZERO);
    let mut bins = [[(Aabb::EMPTY, 0u32); BINS]; 3];
    for &t in order.iter() {
        let c = centroids[t as usize];
        for a in 0..3 {
            if extent[a] > 0.0 {
                let b = &mut bins[a][bin(c[a], centre.lo[a], scale[a])];
                b.0.grow(&boxes[t as usize]);
                b.1 += 1;
            }
        }
    }
    let mut best = (f32::INFINITY, 0, 0);
    for (a, bins) in bins.iter().enumerate() {
        if extent[a] <= 0.0 {
            continue;
        }
        let mut right = [(0f32, 0u32); BINS];
        let (mut acc, mut n) = (Aabb::EMPTY, 0);
        for b in (1..BINS).rev() {
            acc.grow(&bins[b].0);
            n += bins[b].1;
            right[b] = (acc.area(), n);
        }
        let (mut acc, mut n) = (Aabb::EMPTY, 0);
        for b in 0..BINS - 1 {
            acc.grow(&bins[b].0);
            n += bins[b].1;
            let (area, m) = right[b + 1];
            if n == 0 || m == 0 {
                continue;
            }
            let cost = acc.area() * n as f32 + area * m as f32;
            if cost < best.0 {
                best = (cost, a, b);
            }
        }
    }
    if best.0.is_finite() {
        let split_cost = 1.0 + best.0 / bounds.area().max(f32::MIN_POSITIVE);
        if count <= max_leaf && count as f32 <= split_cost {
            return None;
        }
        let (a, b) = (best.1, best.2);
        let (lo, s) = (centre.lo[a], scale[a]);
        return Some(partition(order, |t| {
            bin(centroids[t as usize][a], lo, s) <= b
        }));
    }
    // Every centroid coincides: only an arbitrary split remains.
    (count > max_leaf).then_some(count / 2)
}

/// Builds one mesh's BVH8. `max_leaf` is at most 4 for the probe's step 1; `pad_ulps`
/// pads every box by that many ulps of the mesh's largest coordinate magnitude.
pub fn build(tris: Triangles<'_>, max_leaf: u32, pad_ulps: f32) -> Bvh {
    let start = Instant::now();
    let n = tris.len();
    assert!(n > 0 && n < NONE as usize && (1..=8).contains(&max_leaf));
    let boxes: Vec<Aabb> = (0..n as u32)
        .map(|t| {
            let mut b = Aabb::EMPTY;
            tris.vertices(t).into_iter().for_each(|p| b.point(p));
            b
        })
        .collect();
    let centroids: Vec<Vec3> = boxes.iter().map(|b| (b.lo + b.hi) * 0.5).collect();
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut binary = Vec::with_capacity(n);
    binary.push(Binary::Leaf {
        bounds: Aabb::EMPTY,
        first: 0,
        count: 0,
    });
    // Each task fills binary[slot] from order[first..end].
    let mut tasks = vec![(0usize, 0usize, n)];
    while let Some((slot, first, end)) = tasks.pop() {
        let (mut bounds, mut centre) = (Aabb::EMPTY, Aabb::EMPTY);
        for &t in &order[first..end] {
            bounds.grow(&boxes[t as usize]);
            centre.point(centroids[t as usize]);
        }
        let range = &mut order[first..end];
        binary[slot] = match split(
            &boxes,
            &centroids,
            range,
            &bounds,
            &centre,
            max_leaf as usize,
        ) {
            None => Binary::Leaf {
                bounds,
                first: first as u32,
                count: (end - first) as u32,
            },
            Some(mid) => {
                let left = binary.len();
                for _ in 0..2 {
                    binary.push(Binary::Leaf {
                        bounds: Aabb::EMPTY,
                        first: 0,
                        count: 0,
                    });
                }
                tasks.push((left, first, first + mid));
                tasks.push((left + 1, first + mid, end));
                Binary::Inner {
                    bounds,
                    first: first as u32,
                    count: (end - first) as u32,
                    children: [left as u32, left as u32 + 1],
                }
            }
        };
    }
    let bounds = *binary[0].bounds();
    let pad = pad_ulps * ulp(bounds.lo.abs().max(bounds.hi.abs()).max_element().max(1.0));
    let mut nodes = vec![];
    let (_, stack, depth) = collapse(&binary, 0, &mut nodes, pad, max_leaf);
    let leaves = nodes
        .iter()
        .flatten()
        .filter(|c| c.kind != 0 && c.kind != INNER)
        .count();
    let children = nodes.iter().flatten().filter(|c| c.kind != 0).count();
    let stats = Stats {
        triangles: n,
        nodes: nodes.len(),
        leaves,
        mean_children: children as f64 / nodes.len() as f64,
        mean_leaf: n as f64 / leaves as f64,
        depth,
        stack,
        build_ms: start.elapsed().as_secs_f64() * 1e3,
        bytes: nodes.len() * NODE_BYTES + n * 4,
    };
    Bvh {
        nodes,
        prims: order,
        bounds,
        pad,
        stats,
    }
}

/// Makes the BVH8 node for binary node `slot` by repeatedly opening its largest-area
/// internal child. Any binary subtree of at most `max_leaf` triangles becomes one leaf
/// (its range is contiguous). Returns the node index, the stack entries its subtree can
/// need (k internal children push k entries; descending keeps k - 1 of them) and its depth.
fn collapse(
    binary: &[Binary],
    slot: u32,
    nodes: &mut Vec<Node>,
    pad: f32,
    max_leaf: u32,
) -> (u32, u32, u32) {
    let index = nodes.len();
    nodes.push([Child::EMPTY; WIDTH]);
    let leaf = |c: u32| binary[c as usize].range().1 <= max_leaf;
    let mut open = match binary[slot as usize] {
        Binary::Inner { children, .. } if !leaf(slot) => children.to_vec(),
        _ => vec![slot],
    };
    while open.len() < WIDTH {
        let widest = open
            .iter()
            .enumerate()
            .filter(|(_, c)| !leaf(**c))
            .max_by(|a, b| {
                let area = |c: u32| binary[c as usize].bounds().area();
                area(*a.1).total_cmp(&area(*b.1))
            });
        let Some((i, &c)) = widest else { break };
        let Binary::Inner { children, .. } = binary[c as usize] else {
            unreachable!()
        };
        open[i] = children[0];
        open.push(children[1]);
    }
    let mut node = [Child::EMPTY; WIDTH];
    let (mut inner, mut need, mut depth) = (0, 0, 0);
    for (k, &c) in open.iter().enumerate() {
        let b = binary[c as usize].bounds().padded(pad);
        let (index, kind) = match binary[c as usize].range() {
            (first, count) if count <= max_leaf => (first, count),
            _ => {
                let (child, n, d) = collapse(binary, c, nodes, pad, max_leaf);
                inner += 1;
                need = need.max(n);
                depth = depth.max(d);
                (child, INNER)
            }
        };
        node[k] = Child {
            lo: b.lo.to_array(),
            index,
            hi: b.hi.to_array(),
            kind,
        };
    }
    nodes[index] = node;
    let need = if inner == 0 {
        0
    } else {
        inner - 1 + need.max(1)
    };
    (index as u32, need, depth + 1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
    pub tmin: f32,
    pub tmax: f32,
}
/// A hit's distance, triangle, and the weights of its second and third vertices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub t: f32,
    pub primitive: u32,
    pub bary: [f32; 2],
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Closest hit; equal distances go to the lower primitive.
    Closest,
    /// Any hit in range, stopping at the first.
    Any,
}

/// a*b - c*d with an exact sign (Kahan, with fma): neighbours sharing an edge get
/// edge functions of opposite sign or both zero, so no ray passes between them.
fn difference(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let w = c * d;
    a.mul_add(b, -w) + (-c).mul_add(d, w)
}

/// The per-ray constants of the watertight test (Woop, Benthin and Wald 2013).
struct Sheared {
    origin: Vec3,
    inverse: Vec3,
    shear: Vec3,
    axes: [usize; 3],
}
impl Sheared {
    fn new(origin: Vec3, direction: Vec3) -> Self {
        let a = direction.abs();
        let z = if a.x >= a.y && a.x >= a.z {
            0
        } else if a.y >= a.z {
            1
        } else {
            2
        };
        let (mut x, mut y) = ((z + 1) % 3, (z + 2) % 3);
        // Swapping keeps the winding, so edge function signs mean the same on both faces.
        if direction[z] < 0.0 {
            std::mem::swap(&mut x, &mut y);
        }
        let dz = direction[z];
        let safe = Vec3::select(
            a.cmplt(Vec3::splat(1e-30)),
            Vec3::select(
                direction.cmpge(Vec3::ZERO),
                Vec3::splat(1e-30),
                Vec3::splat(-1e-30),
            ),
            direction,
        );
        Self {
            origin,
            inverse: Vec3::ONE / safe,
            shear: Vec3::new(direction[x] / dz, direction[y] / dz, 1.0 / dz),
            axes: [x, y, z],
        }
    }
    /// Widened box entry, or infinity when the box is missed within [tmin, tmax].
    fn slab(&self, child: &Child, tmin: f32, tmax: f32) -> f32 {
        let a = (Vec3::from_array(child.lo) - self.origin) * self.inverse;
        let b = (Vec3::from_array(child.hi) - self.origin) * self.inverse;
        let (n, f) = (a.min(b), a.max(b));
        let near = n.max_element();
        let far = f.min_element();
        let entry = (near - near.abs() * WIDEN).max(tmin);
        let exit = (far + far.abs() * WIDEN).min(tmax);
        if entry <= exit { entry } else { f32::INFINITY }
    }
    /// (t, weights of vertices 1 and 2) when the triangle is hit within [tmin, tmax].
    fn triangle(&self, v: [Vec3; 3], tmin: f32, tmax: f32) -> Option<(f32, [f32; 2])> {
        let [x, y, z] = self.axes;
        let s = self.shear;
        let [a, b, c] = v.map(|p| p - self.origin);
        let ax = (-s.x).mul_add(a[z], a[x]);
        let ay = (-s.y).mul_add(a[z], a[y]);
        let bx = (-s.x).mul_add(b[z], b[x]);
        let by = (-s.y).mul_add(b[z], b[y]);
        let cx = (-s.x).mul_add(c[z], c[x]);
        let cy = (-s.y).mul_add(c[z], c[y]);
        let u = difference(cx, by, cy, bx);
        let v = difference(ax, cy, ay, cx);
        let w = difference(bx, ay, by, ax);
        if (u < 0.0 || v < 0.0 || w < 0.0) && (u > 0.0 || v > 0.0 || w > 0.0) {
            return None;
        }
        let det = u + v + w;
        if det == 0.0 {
            return None;
        }
        let (az, bz, cz) = (s.z * a[z], s.z * b[z], s.z * c[z]);
        let t = u.mul_add(az, v.mul_add(bz, w * cz)) / det;
        (t >= tmin && t <= tmax).then_some((t, [v / det, w / det]))
    }
}

/// The f32 test of one triangle: (t, weights of vertices 1 and 2) within [tmin, tmax].
pub fn intersect(tris: Triangles<'_>, ray: &Ray, primitive: u32) -> Option<(f32, [f32; 2])> {
    Sheared::new(ray.origin, ray.direction).triangle(tris.vertices(primitive), ray.tmin, ray.tmax)
}

impl Bvh {
    /// CPU reference traversal, in the WGSL's order: children in slot order, leaves
    /// tested on sight, internal children pushed nearest-on-top for closest hits, and
    /// popped entries beyond the current hit skipped.
    pub fn trace(&self, tris: Triangles<'_>, ray: &Ray, mode: Mode) -> Option<Hit> {
        let r = Sheared::new(ray.origin, ray.direction);
        let mut best = Hit {
            t: ray.tmax,
            primitive: NONE,
            bary: [0.0; 2],
        };
        let mut stack = vec![(0u32, 0f32); self.stats.stack as usize];
        let (mut sp, mut node) = (0, 0u32);
        loop {
            let base = sp;
            for child in &self.nodes[node as usize] {
                if child.kind == 0 {
                    break;
                }
                let near = r.slab(child, ray.tmin, best.t);
                if near > best.t {
                    continue;
                }
                if child.kind == INNER {
                    let mut j = sp;
                    if mode == Mode::Closest {
                        while j > base && stack[j - 1].1 < near {
                            stack[j] = stack[j - 1];
                            j -= 1;
                        }
                    }
                    stack[j] = (child.index, near);
                    sp += 1;
                    continue;
                }
                for i in 0..child.kind {
                    let p = self.prims[(child.index + i) as usize];
                    if let Some((t, bary)) = r.triangle(tris.vertices(p), ray.tmin, best.t) {
                        let hit = Hit {
                            t,
                            primitive: p,
                            bary,
                        };
                        if mode == Mode::Any {
                            return Some(hit);
                        }
                        if t < best.t || p < best.primitive {
                            best = hit;
                        }
                    }
                }
            }
            loop {
                if sp == 0 {
                    return (best.primitive != NONE).then_some(best);
                }
                sp -= 1;
                if stack[sp].1 <= best.t {
                    node = stack[sp].0;
                    break;
                }
            }
        }
    }

    /// Structural invariants: every triangle in exactly one leaf of at most `max_leaf`,
    /// every child box containing its subtree's boxes and triangles, children packed.
    pub fn check(&self, tris: Triangles<'_>, max_leaf: u32) -> Result<(), String> {
        let mut seen = vec![false; tris.len()];
        let mut stack = vec![(0u32, None::<Aabb>)];
        let mut visited = 0;
        while let Some((node, parent)) = stack.pop() {
            visited += 1;
            let node = &self.nodes[node as usize];
            let used = node.iter().take_while(|c| c.kind != 0).count();
            if used == 0 || node[used..].iter().any(|c| c.kind != 0) {
                return Err(format!("node {node:?} has unpacked or no children"));
            }
            for child in &node[..used] {
                let b = Aabb {
                    lo: Vec3::from_array(child.lo),
                    hi: Vec3::from_array(child.hi),
                };
                if parent.is_some_and(|p| !p.contains(&b)) {
                    return Err("child box outside its parent".into());
                }
                if child.kind == INNER {
                    stack.push((child.index, Some(b)));
                    continue;
                }
                if child.kind > max_leaf {
                    return Err(format!("leaf of {} triangles", child.kind));
                }
                for i in 0..child.kind {
                    let p = self.prims[(child.index + i) as usize];
                    if std::mem::replace(&mut seen[p as usize], true) {
                        return Err(format!("triangle {p} in two leaves"));
                    }
                    let mut t = Aabb::EMPTY;
                    tris.vertices(p).into_iter().for_each(|v| t.point(v));
                    if !b.contains(&t) {
                        return Err(format!("triangle {p} outside its leaf box"));
                    }
                }
            }
        }
        if visited != self.nodes.len() || seen.iter().any(|s| !s) {
            return Err("unreachable node or triangle in no leaf".into());
        }
        Ok(())
    }
}

/// Brute force over every triangle with the same f32 test and tie rule. A conservative
/// f64 bounding-sphere prefilter skips triangles the ray cannot reach.
pub struct Brute {
    spheres: Vec<[f64; 4]>,
}
impl Brute {
    pub fn new(tris: Triangles<'_>, scale: f32) -> Self {
        // Far wider than the f32 test's reach outside the exact triangle.
        let slack = 64.0 * f64::from(ulp(scale));
        let spheres = (0..tris.len() as u32)
            .map(|t| {
                let v = tris.vertices(t).map(|p| p.as_dvec3());
                let c = (v[0] + v[1] + v[2]) / 3.0;
                let r = v.iter().map(|p| p.distance(c)).fold(0.0, f64::max);
                [c.x, c.y, c.z, r * (1.0 + 1e-9) + slack]
            })
            .collect();
        Self { spheres }
    }
    pub fn trace(&self, tris: Triangles<'_>, ray: &Ray, mode: Mode) -> Option<Hit> {
        let r = Sheared::new(ray.origin, ray.direction);
        let o = ray.origin.as_dvec3();
        let length = ray.direction.as_dvec3().length();
        let d = ray.direction.as_dvec3() / length;
        let (near, far) = (f64::from(ray.tmin) * length, f64::from(ray.tmax) * length);
        let mut best = Hit {
            t: ray.tmax,
            primitive: NONE,
            bary: [0.0; 2],
        };
        for (p, s) in self.spheres.iter().enumerate() {
            let v = DVec3::new(s[0], s[1], s[2]) - o;
            let along = v.dot(d);
            if along + s[3] < near
                || along - s[3] > far
                || v.cross(d).length_squared() > s[3] * s[3]
            {
                continue;
            }
            if let Some((t, bary)) = r.triangle(tris.vertices(p as u32), ray.tmin, best.t) {
                let hit = Hit {
                    t,
                    primitive: p as u32,
                    bary,
                };
                if mode == Mode::Any {
                    return Some(hit);
                }
                if t < best.t || (p as u32) < best.primitive {
                    best = hit;
                }
            }
        }
        (best.primitive != NONE).then_some(best)
    }
}

/// GPU buffers of several meshes' BVHs, one node and one leaf-list buffer per mesh (bound
/// as binding arrays, like the vertex and index buffers), with mesh-local indices. Per-mesh
/// buffers keep every binding under WebGPU's default 128 MiB storage-binding limit.
pub struct Packed {
    pub nodes: Vec<Vec<u8>>,
    pub prims: Vec<Vec<u8>>,
    /// Stack entries the deepest tree can need (at least 1).
    pub stack: u32,
}
impl Packed {
    pub fn bytes(&self) -> usize {
        self.nodes.iter().chain(&self.prims).map(Vec::len).sum()
    }
}
pub fn pack(bvhs: &[&Bvh]) -> Packed {
    let mut packed = Packed {
        nodes: vec![],
        prims: vec![],
        stack: 1,
    };
    for bvh in bvhs {
        packed.stack = packed.stack.max(bvh.stats.stack);
        let words = bvh.nodes.iter().flatten().flat_map(|c| {
            [
                c.lo[0].to_bits(),
                c.lo[1].to_bits(),
                c.lo[2].to_bits(),
                c.index,
            ]
            .into_iter()
            .chain([
                c.hi[0].to_bits(),
                c.hi[1].to_bits(),
                c.hi[2].to_bits(),
                c.kind,
            ])
        });
        packed
            .nodes
            .push(words.flat_map(u32::to_le_bytes).collect());
        packed
            .prims
            .push(bvh.prims.iter().flat_map(|p| p.to_le_bytes()).collect());
    }
    packed
}

/// World bounds of a placement of `bvh`, from its padded object box in f64 and padded again.
pub fn world_bounds(model: Mat4, bvh: &Bvh) -> Aabb {
    let m = model.as_dmat4();
    let b = bvh.bounds.padded(bvh.pad);
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for corner in 0..8 {
        let pick = |bit: usize, a: usize| if corner & bit == 0 { b.lo[a] } else { b.hi[a] };
        let p = m.transform_point3(DVec3::new(
            f64::from(pick(1, 0)),
            f64::from(pick(2, 1)),
            f64::from(pick(4, 2)),
        ));
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let pad = f64::from(16.0 * ulp(lo.abs().max(hi.abs()).max_element().max(1.0) as f32));
    Aabb {
        lo: (lo - pad).as_vec3(),
        hi: (hi + pad).as_vec3(),
    }
}

/// One placement's record: world-to-object rows (f64 inverse rounded to f32), padded world
/// bounds, root node (0), mesh slot and the single-sided flag (camera rays cull back faces).
pub fn instance(model: Mat4, bvh: &Bvh, mesh: u32, single_sided: bool) -> Vec<u8> {
    let inverse = model.as_dmat4().inverse();
    let rows = inverse.transpose();
    let b = world_bounds(model, bvh);
    let mut words: Vec<u32> = vec![];
    for r in 0..3 {
        words.extend(rows.col(r).as_vec4().to_array().map(f32::to_bits));
    }
    words.extend(b.lo.extend(0.0).to_array().map(f32::to_bits));
    words.extend(b.hi.extend(0.0).to_array().map(f32::to_bits));
    words.extend([0, mesh, u32::from(single_sided), 0]);
    assert_eq!(words.len() * 4, INSTANCE_BYTES);
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// Module constants the traversal WGSL declares nothing for.
pub fn constants(meshes: usize, instances: usize, stack: u32) -> String {
    format!(
        "const SOFTWARE_MESHES:u32={meshes}u;\nconst SOFTWARE_INSTANCES:u32={instances}u;\nconst SOFTWARE_STACK:u32={stack}u;\n"
    )
}
/// The traversal alone (fixture ray kernels), after `constants`.
pub fn core_source(constants: &str) -> String {
    format!("{constants}{}", WGSL.split_once(COMPOSED).unwrap().0)
}

/// Ray-query text of the traced soft module and its software replacement. Every edit
/// must match exactly once, so the probe always traces the production rays.
const SOFT_EDITS: [(&str, &str); 6] = [
    ("enable wgpu_ray_query_vertex_return;\n", ""),
    ("enable wgpu_ray_query;\n", ""),
    (
        "@group(0) @binding(2) var fresh_scene:acceleration_structure;\n",
        "",
    ),
    (
        "@group(0) @binding(19) var primary_scene:acceleration_structure<vertex_return>;\n",
        "",
    ),
    (
        "    // Function scope avoids Naga 30's loop-local opaque null initializer.\n    var query:ray_query;\n",
        "",
    ),
    (
        "                let flags=RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT;\n                rayQueryInitialize(&query,fresh_scene,RayDesc(flags,255u,tmin,tmax,ray_origin,direction));\n                while rayQueryProceed(&query) {}\n                blocked=rayQueryGetCommittedIntersection(&query).kind!=0u;\n",
        "                blocked=software_occluded(ray_origin,direction,tmin,tmax);\n",
    ),
];
/// Start and end of the hardware camera-ray functions the WGSL replaces: the raw closest
/// hit, its edge-repair wrapper and the repair's probe queries (edge repair, d72b02c).
const CLOSEST: (&str, &str) = (
    "// The full near-to-far segment with forced opaque geometry and no first-hit flag.\nfn primary_raw_closest(",
    "// The pixel's camera ray, or a miss",
);

/// The traced soft kernel module (`primary_ray_gpu::shader_sources(..)[1]`) with every
/// hardware ray query replaced by software traversal: same receivers, ray setup, ε,
/// directions, flags and outputs. Without the ray-query enables, any remaining hardware
/// ray use would fail to compile.
pub fn traced_source(soft: &str, constants: &str) -> String {
    let mut source = soft.to_owned();
    for (find, replace) in SOFT_EDITS {
        assert_eq!(source.matches(find).count(), 1, "{find}");
        source = source.replacen(find, replace, 1);
    }
    let start = source.find(CLOSEST.0).expect("hardware camera rays");
    let end = start + source[start..].find(CLOSEST.1).expect("camera ray end");
    source.replace_range(start..end, "");
    assert!(!source.contains("rayQuery") && !source.contains("acceleration_structure"));
    format!("{source}\n{constants}{WGSL}")
}

/// f64 replay of one ray against one world-space triangle.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Replay {
    /// Ray parameter of the plane crossing (NaN when parallel).
    pub t: f64,
    /// The segment [tmin, tmax] crosses the triangle interior.
    pub inside: bool,
    /// Distance from the segment to the triangle's boundary: its edges, and the triangle
    /// from either segment end. A perturbation smaller than this cannot change `inside`.
    pub margin: f64,
    /// The margin is the distance from a segment end, not from an edge.
    pub endpoint: bool,
}
pub fn replay(o: DVec3, d: DVec3, tmin: f64, tmax: f64, v: [DVec3; 3]) -> Replay {
    let (e1, e2) = (v[1] - v[0], v[2] - v[0]);
    let p = d.cross(e2);
    let det = e1.dot(p);
    let (mut t, mut inside) = (f64::NAN, false);
    if det != 0.0 {
        let s = o - v[0];
        let q = s.cross(e1);
        let (u, w) = (s.dot(p) / det, d.dot(q) / det);
        t = e2.dot(q) / det;
        inside = u >= 0.0 && w >= 0.0 && u + w <= 1.0 && t >= tmin && t <= tmax;
    }
    let (a, b) = (o + d * tmin, o + d * tmax);
    let edge = [(0, 1), (1, 2), (2, 0)]
        .iter()
        .map(|&(i, j)| segment_distance(a, b, v[i], v[j]))
        .fold(f64::INFINITY, f64::min);
    let end = point_triangle(a, v).min(point_triangle(b, v));
    Replay {
        t,
        inside,
        margin: edge.min(end),
        endpoint: end < edge,
    }
}

/// Distance between segments p1q1 and p2q2 (Ericson, Real-Time Collision Detection 5.1.9).
fn segment_distance(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> f64 {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t);
    if a <= f64::MIN_POSITIVE && e <= f64::MIN_POSITIVE {
        return r.length();
    }
    if a <= f64::MIN_POSITIVE {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= f64::MIN_POSITIVE {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom != 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    ((p1 + d1 * s) - (p2 + d2 * t)).length()
}

/// Distance from p to triangle abc (Ericson 5.1.5).
fn point_triangle(p: DVec3, [a, b, c]: [DVec3; 3]) -> f64 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length();
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return (p - (a + ab * (d1 / (d1 - d3)))).length();
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return (p - (a + ac * (d2 / (d2 - d6)))).length();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return (p - (b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6))))).length();
    }
    let denom = 1.0 / (va + vb + vc);
    (p - (a + ab * (vb * denom) + ac * (vc * denom))).length()
}

impl Bvh {
    /// Triangles whose padded leaf boxes, widened by `pad`, meet the f64 object-space
    /// segment [tmin, tmax]: the candidates an f64 replay must consider.
    pub fn candidates(
        &self,
        o: DVec3,
        d: DVec3,
        tmin: f64,
        tmax: f64,
        pad: f64,
        out: &mut Vec<u32>,
    ) {
        let inverse = DVec3::ONE / d;
        let reach = |lo: [f32; 3], hi: [f32; 3]| {
            let lo = Vec3::from_array(lo).as_dvec3() - pad;
            let hi = Vec3::from_array(hi).as_dvec3() + pad;
            let (mut near, mut far) = (tmin, tmax);
            for a in 0..3 {
                if d[a] == 0.0 {
                    if o[a] < lo[a] || o[a] > hi[a] {
                        return false;
                    }
                    continue;
                }
                let (t0, t1) = ((lo[a] - o[a]) * inverse[a], (hi[a] - o[a]) * inverse[a]);
                near = near.max(t0.min(t1));
                far = far.min(t0.max(t1));
            }
            near <= far
        };
        let mut stack = vec![0u32];
        while let Some(node) = stack.pop() {
            for child in self.nodes[node as usize].iter().take_while(|c| c.kind != 0) {
                if !reach(child.lo, child.hi) {
                    continue;
                }
                if child.kind == INNER {
                    stack.push(child.index);
                } else {
                    out.extend(
                        &self.prims[child.index as usize..(child.index + child.kind) as usize],
                    );
                }
            }
        }
    }
}

/// A placement's f64 transforms.
pub fn transforms(model: Mat4) -> (DMat4, DMat4) {
    let m = model.as_dmat4();
    (m, m.inverse())
}
