//! Cook-time near-field sky occlusion for the world radiance cache's software-ray path
//! (M7 G1c, docs/m7-g1c.md). No GPU or shading-model dependency.
//!
//! Each static mesh's far field is a legal cluster-DAG cut (R0c's selector) at `lod_error`,
//! rebuilt as a BVH4 with eight triangles a leaf; runtime software rays trace it in place of
//! full detail. Optionally (`bins` > 0), every vertex of the cut (a source vertex on the
//! full-detail surface) also gets two masks: one exact ray per world-space octahedral bin,
//! `reach` long, against every static placement's full-detail triangles, one blocked bit per
//! bin and side. At runtime a cell whose query point lies on the cut blocks a direction its
//! three vertex masks all block, in addition to its rays against the cut (additive: the cut
//! is still traced from the query point). G1c measured these masks at no gain on the quarry,
//! so cooks default to none (docs/m7-g1c.md). Masks store visibility, never a program's
//! output, and there is no per-triangle grid (docs/m7-p0-claims.md 4.1).
use crate::{
    Error, Result,
    cluster_bvh::{self, Bvh, EMPTY, LEAF_BIT, Leaf, Level, Node},
    virtual_mesh::Span,
};

pub const VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"RNF\0";
/// Triangles per leaf of the far-field BVH (and of runtime full-detail BVHs).
pub const LEAF_TRIANGLES: usize = 8;

/// What a near field was cooked with; every field is part of its cache key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Object-space error of the far-field cut (cluster-DAG legal cut, as R0c selects).
    pub lod_error: f32,
    /// World length of the masks' exact rays.
    pub reach: f32,
    /// World distance within which the masks' rays ignore hits (a vertex's own surface).
    pub skip: f32,
    /// Octahedral bins per side; a vertex mask holds bins² bits (a multiple of 32). 0: no
    /// masks, the cut alone.
    pub bins: u32,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        let ok = self.lod_error.is_finite()
            && self.lod_error >= 0.
            && self.reach.is_finite()
            && self.reach > 0.
            && self.skip.is_finite()
            && self.skip >= 0.
            && self.skip < self.reach
            && [0, 8, 16, 32].contains(&self.bins);
        if ok {
            Ok(())
        } else {
            Err(Error("invalid near-field settings"))
        }
    }
    /// u32 words per vertex mask (0 without masks).
    pub fn words(&self) -> usize {
        (self.bins * self.bins / 32) as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NearPlacement {
    pub mesh: u32,
    /// Column-major placement transform the masks were cooked under; a renderer applies
    /// them only to an instance with these exact bits.
    pub transform: [[f32; 4]; 4],
    /// Per position of the mesh's far-field BVH, in position order, two masks of `words()`:
    /// the front side (rays from just off the surface along the vertex normal) then the back
    /// side (just off it against the normal). Bit b is set when bin b's cook ray meets static
    /// geometry within `reach`. Two sides, so a surface seen from behind has its own field.
    pub masks: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NearField {
    pub settings: Settings,
    /// SHA-256 of the source geometry the cook read.
    pub source: [u8; 32],
    /// One far-field cut per mesh: a single zero-error level over the cut's triangles,
    /// LEAF_TRIANGLES per leaf, in R0c's record layout (docs/m7-r0c.md).
    pub meshes: Vec<Bvh>,
    pub placements: Vec<NearPlacement>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 160_000_000,
        }
    }
}

impl NearField {
    /// GPU-resident bytes: every mesh's records and every placement's masks.
    pub fn gpu_bytes(&self) -> usize {
        self.meshes.iter().map(Bvh::gpu_bytes).sum::<usize>()
            + self
                .placements
                .iter()
                .map(|p| p.masks.len() * 4)
                .sum::<usize>()
    }
    pub fn validate(&self, limits: Limits) -> Result<()> {
        self.settings.validate()?;
        if self.gpu_bytes() > limits.max_bytes || self.meshes.is_empty() {
            return Err(Error("near field byte limit or no meshes"));
        }
        let bvh_limits = cluster_bvh::Limits {
            max_bytes: limits.max_bytes,
        };
        for b in &self.meshes {
            b.validate(bvh_limits)?;
            if b.levels.len() != 1 {
                return Err(Error("near-field mesh must have one level"));
            }
        }
        let words = self.settings.words();
        for p in &self.placements {
            let mesh = self
                .meshes
                .get(p.mesh as usize)
                .ok_or(Error("near-field placement mesh"))?;
            if p.masks.len() != mesh.positions.len() * 2 * words
                || !p.transform.iter().flatten().all(|v| v.is_finite())
            {
                return Err(Error("near-field placement masks or transform"));
            }
        }
        Ok(())
    }
}

/// Octahedral bin of a direction (need not be unit length), row-major in `bins`²:
/// the square [-1, 1]² of the octahedral map split into bins² cells. shaders/gi_rays_software.wgsl
/// `gi_near_bin` computes the same cell.
pub fn bin(d: [f32; 3], bins: u32) -> u32 {
    let [u, v] = octahedral(d);
    let n = bins as f32;
    let cell = |x: f32| (((x * 0.5 + 0.5) * n).floor() as i64).clamp(0, i64::from(bins) - 1) as u32;
    cell(v) * bins + cell(u)
}
/// The unit direction through the centre of `bin`.
pub fn bin_direction(bin: u32, bins: u32) -> [f32; 3] {
    let n = bins as f32;
    let u = ((bin % bins) as f32 + 0.5) / n * 2. - 1.;
    let v = ((bin / bins) as f32 + 0.5) / n * 2. - 1.;
    let z = 1. - u.abs() - v.abs();
    let t = (-z).max(0.);
    let x = u + if u >= 0. { -t } else { t };
    let y = v + if v >= 0. { -t } else { t };
    let l = (x * x + y * y + z * z).sqrt();
    [x / l, y / l, z / l]
}
fn octahedral(d: [f32; 3]) -> [f32; 2] {
    let s = d[0].abs() + d[1].abs() + d[2].abs();
    let (x, y) = (d[0] / s, d[1] / s);
    if d[2] >= 0. {
        [x, y]
    } else {
        let sign = |a: f32| if a >= 0. { 1. } else { -1. };
        [(1. - y.abs()) * sign(x), (1. - x.abs()) * sign(y)]
    }
}

/// Builds a one-level, zero-error BVH4 over `indices` (a triangle list) by deterministic
/// median splits on the longest centroid axis, ties broken by triangle order, with at most
/// LEAF_TRIANGLES per leaf. Child bounds are quantized outward so both fused and unfused
/// f32 decodes enclose them (R0c's record layout and `Bvh::validate`).
pub fn build_bvh(positions: &[[f32; 3]], indices: &[u32]) -> Result<Bvh> {
    build(positions, indices, false)
}

/// Software rays' deepest traversal: shaders/gi_rays_software.wgsl keeps a 48-entry stack, and
/// a BVH4 node pushes at most four children after popping one, so 15 node levels fit.
pub const MAX_SAH_DEPTH: usize = 15;
/// As `build_bvh`, but each binary split takes the binned surface-area-heuristic plane (16
/// bins per axis over the centroids, ties to the lower axis and bin), which runtime software
/// rays traverse with fewer node and triangle tests than median splits. Deterministic. A tree
/// deeper than MAX_SAH_DEPTH node levels falls back to the median build. Cooked packages keep
/// `build_bvh` (their bytes are cache keys); runtime builds use this (M7-GC1, docs/m7-gc1.md).
pub fn build_bvh_sah(positions: &[[f32; 3]], indices: &[u32]) -> Result<Bvh> {
    let bvh = build(positions, indices, true)?;
    if depth(&bvh, bvh.levels[0].root) > MAX_SAH_DEPTH {
        return build_bvh(positions, indices);
    }
    Ok(bvh)
}
/// Node levels below `child` (0 for a leaf).
fn depth(bvh: &Bvh, child: u32) -> usize {
    if child & LEAF_BIT != 0 {
        return 0;
    }
    let node = &bvh.nodes[child as usize];
    1 + node
        .children
        .iter()
        .filter(|&&c| c != EMPTY)
        .map(|&c| depth(bvh, c))
        .max()
        .unwrap_or(0)
}

/// Runtime software-ray BVH4 with at most `leaf_max` triangles a leaf (1 to LEAF_TRIANGLES) and,
/// with `spatial`, spatial splits (Stich, Friedrich and Dietrich 2009, "Spatial Splits in Bounding
/// Volume Hierarchies"): a binary split may cut triangle references at a plane, so a long triangle
/// stops inflating every box it crosses. A triangle can then sit in several leaves, each holding
/// the box of its part there; a ray's hit on it lies in one of those parts, so the hits are a
/// whole-triangle build's. Leaf boxes are the parts' rather than the triangles', so
/// `Bvh::validate`'s enclosure check does not apply (the tree's own test checks its hits). Not
/// refittable: deforming meshes keep `build_bvh_sah`. A tree deeper than MAX_SAH_DEPTH falls back
/// to `build_bvh_sah`.
pub fn build_bvh_split(positions: &[[f32; 3]], indices: &[u32], leaf_max: usize, spatial: bool) -> Result<Bvh> {
    if indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err(Error("BVH geometry needs a nonempty triangle list"));
    }
    if indices.iter().any(|&i| i as usize >= positions.len()) || positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error("BVH geometry has an invalid index or position"));
    }
    let refs: Vec<Ref> = (0..indices.len() / 3)
        .map(|t| {
            let (lo, hi) = bounds((0..3).map(|k| positions[indices[t * 3 + k] as usize]));
            Ref { tri: t as u32, lo, hi }
        })
        .collect();
    let (lo, hi) = refs.iter().fold(NO_BOX, |b, r| merge(b, (r.lo, r.hi)));
    let mut builder = SplitBuilder {
        positions,
        indices,
        leaf_max: leaf_max.clamp(1, LEAF_TRIANGLES),
        spatial,
        root_area: half_area(lo, hi),
        // At most as many extra references as triangles: memory stays within 2x however much the
        // triangles overlap.
        budget: refs.len(),
        leaf_boxes: vec![],
        bvh: Bvh { positions: positions.iter().map(|p| [p[0], p[1], p[2], 0.0]).collect(), ..Default::default() },
    };
    let root = builder.subtree(refs);
    let mut bvh = builder.bvh;
    if depth(&bvh, root) > MAX_SAH_DEPTH {
        return build_bvh_sah(positions, indices);
    }
    let leaves = bvh.leaves.len() as u32;
    bvh.level_leaves = (0..leaves).collect();
    bvh.levels.push(Level { mesh: 0, root, requested_error: 0.0, max_error: 0.0, leaves: Span { offset: 0, count: leaves } });
    Ok(bvh)
}
/// One triangle reference: the triangle, and the box of its part this subtree holds.
#[derive(Clone, Copy)]
struct Ref {
    tri: u32,
    lo: [f32; 3],
    hi: [f32; 3],
}
impl Ref {
    fn centroid(&self, axis: usize) -> f32 {
        0.5 * (self.lo[axis] + self.hi[axis])
    }
}
/// A split's overlap must exceed this share of the root's area before spatial splits are tried
/// (Stich et al.'s alpha).
const SPLIT_ALPHA: f32 = 1e-5;
type Boxed = ([f32; 3], [f32; 3]);
const NO_BOX: Boxed = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
struct SplitBuilder<'a> {
    positions: &'a [[f32; 3]],
    indices: &'a [u32],
    leaf_max: usize,
    spatial: bool,
    root_area: f32,
    /// Extra references spatial splits may still add.
    budget: usize,
    /// Per leaf, the union of its references' boxes (the box its parent stores).
    leaf_boxes: Vec<Boxed>,
    bvh: Bvh,
}
impl SplitBuilder<'_> {
    fn subtree(&mut self, refs: Vec<Ref>) -> u32 {
        if refs.len() <= self.leaf_max {
            return self.leaf(&refs);
        }
        let mut groups = vec![refs];
        for _ in 0..2 {
            groups = groups
                .into_iter()
                .flat_map(|g| {
                    if g.len() <= self.leaf_max {
                        return vec![g];
                    }
                    let (a, b) = self.split(g);
                    vec![a, b]
                })
                .collect();
        }
        let children: Vec<u32> = groups.into_iter().map(|g| self.subtree(g)).collect();
        let boxes: Vec<_> = children.iter().map(|&c| self.child_bounds(c)).collect();
        let id = self.bvh.nodes.len() as u32;
        self.bvh.nodes.push(quantized_node(&children, &boxes));
        id
    }
    /// The cheaper of the binned object split and (when the object split's children overlap and
    /// the budget allows) the binned spatial split, by the surface-area heuristic.
    fn split(&mut self, g: Vec<Ref>) -> (Vec<Ref>, Vec<Ref>) {
        let n = g.len();
        let object = self.object_split(&g);
        if self.spatial && self.budget > 0 {
            let overlap = object.as_ref().map_or(f32::INFINITY, |o| {
                let lo: [f32; 3] = std::array::from_fn(|a| o.left.0[a].max(o.right.0[a]));
                let hi: [f32; 3] = std::array::from_fn(|a| o.left.1[a].min(o.right.1[a]));
                if (0..3).all(|a| lo[a] <= hi[a]) { half_area(lo, hi) } else { 0.0 }
            });
            if overlap > SPLIT_ALPHA * self.root_area
                && let Some((cost, axis, plane)) = self.spatial_split(&g)
                && object.as_ref().is_none_or(|o| cost < o.cost)
            {
                let (mut left, mut right) = (vec![], vec![]);
                for r in &g {
                    if r.hi[axis] <= plane {
                        left.push(*r);
                    } else if r.lo[axis] >= plane {
                        right.push(*r);
                    } else {
                        if let Some(b) = self.clip(r, axis, f32::NEG_INFINITY, plane) {
                            left.push(Ref { tri: r.tri, lo: b.0, hi: b.1 });
                        }
                        if let Some(b) = self.clip(r, axis, plane, f32::INFINITY) {
                            right.push(Ref { tri: r.tri, lo: b.0, hi: b.1 });
                        }
                    }
                }
                let extra = (left.len() + right.len()).saturating_sub(n);
                if !left.is_empty() && !right.is_empty() && left.len() < n && right.len() < n && extra <= self.budget {
                    self.budget -= extra;
                    return (left, right);
                }
            }
        }
        match object {
            Some(o) => g.into_iter().partition(|r| o.bin(r) <= o.plane),
            None => {
                // Every centroid in one bin on every axis: halve by count in a fixed order.
                let (lo, hi) = g.iter().fold(NO_BOX, |b, r| merge(b, (r.lo, r.hi)));
                let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
                let mut g = g;
                g.sort_by(|a, b| a.centroid(axis).total_cmp(&b.centroid(axis)).then(a.tri.cmp(&b.tri)));
                let b = g.split_off(g.len() / 2);
                (g, b)
            }
        }
    }
    fn object_split(&self, g: &[Ref]) -> Option<ObjectSplit> {
        let (lo, hi) = g.iter().fold(NO_BOX, |b, r| {
            let c: [f32; 3] = std::array::from_fn(|a| r.centroid(a));
            merge(b, (c, c))
        });
        let mut best: Option<ObjectSplit> = None;
        for axis in 0..3 {
            let extent = hi[axis] - lo[axis];
            if extent.is_nan() || extent <= 0.0 {
                continue;
            }
            let split = ObjectSplit { cost: 0.0, axis, lo: lo[axis], extent, plane: 0, left: NO_BOX, right: NO_BOX };
            let mut count = [0usize; SAH_BINS];
            let mut boxes = [NO_BOX; SAH_BINS];
            for r in g {
                let b = split.bin(r);
                count[b] += 1;
                boxes[b] = merge(boxes[b], (r.lo, r.hi));
            }
            let mut right = [(0.0f32, NO_BOX); SAH_BINS];
            let (mut acc, mut k) = (NO_BOX, 0usize);
            for b in (1..SAH_BINS).rev() {
                acc = merge(acc, boxes[b]);
                k += count[b];
                right[b] = (half_area(acc.0, acc.1) * k as f32, acc);
            }
            let (mut acc, mut k) = (NO_BOX, 0usize);
            for b in 0..SAH_BINS - 1 {
                acc = merge(acc, boxes[b]);
                k += count[b];
                if k == 0 || k == g.len() {
                    continue;
                }
                let cost = half_area(acc.0, acc.1) * k as f32 + right[b + 1].0;
                if best.as_ref().is_none_or(|o| cost < o.cost) {
                    best = Some(ObjectSplit { cost, plane: b, left: acc, right: right[b + 1].1, ..split });
                }
            }
        }
        best
    }
    /// The binned spatial split over the references' common box: per axis SAH_BINS slabs, each
    /// reference's clipped part added to every slab it crosses, entries and exits counted
    /// (Stich et al. 2009, section 4). Returns the cost, axis and plane.
    fn spatial_split(&self, g: &[Ref]) -> Option<(f32, usize, f32)> {
        let (lo, hi) = g.iter().fold(NO_BOX, |b, r| merge(b, (r.lo, r.hi)));
        let mut best: Option<(f32, usize, f32)> = None;
        for axis in 0..3 {
            let extent = hi[axis] - lo[axis];
            if extent.is_nan() || extent <= 0.0 {
                continue;
            }
            let w = extent / SAH_BINS as f32;
            let plane = |k: usize| lo[axis] + w * k as f32;
            let bin = |x: f32| (((x - lo[axis]) / w) as usize).min(SAH_BINS - 1);
            let mut boxes = [NO_BOX; SAH_BINS];
            let (mut entry, mut exit) = ([0usize; SAH_BINS], [0usize; SAH_BINS]);
            for r in g {
                let (first, last) = (bin(r.lo[axis]), bin(r.hi[axis]));
                entry[first] += 1;
                exit[last] += 1;
                for b in first..=last {
                    let part = if first == last { Some((r.lo, r.hi)) } else { self.clip(r, axis, plane(b), plane(b + 1)) };
                    if let Some(part) = part {
                        boxes[b] = merge(boxes[b], part);
                    }
                }
            }
            let mut right = [0.0f32; SAH_BINS];
            let (mut acc, mut k) = (NO_BOX, 0usize);
            for b in (1..SAH_BINS).rev() {
                acc = merge(acc, boxes[b]);
                k += exit[b];
                right[b] = half_area(acc.0, acc.1) * k as f32;
            }
            let (mut acc, mut k) = (NO_BOX, 0usize);
            for b in 0..SAH_BINS - 1 {
                acc = merge(acc, boxes[b]);
                k += entry[b];
                let rest: usize = exit[b + 1..].iter().sum();
                if k == 0 || rest == 0 {
                    continue;
                }
                let cost = half_area(acc.0, acc.1) * k as f32 + right[b + 1];
                if best.is_none_or(|(c, _, _)| cost < c) {
                    best = Some((cost, axis, plane(b + 1)));
                }
            }
        }
        best
    }
    /// The box of reference `r`'s triangle between `s0` and `s1` on `axis`, within `r`'s box, or
    /// None when the triangle does not reach that slab. Points on the clip planes come from the
    /// edges and are padded outward by a few ulps, so the part's box encloses the exact part.
    fn clip(&self, r: &Ref, axis: usize, s0: f32, s1: f32) -> Option<Boxed> {
        let t = r.tri as usize * 3;
        let v: [[f32; 3]; 3] = std::array::from_fn(|k| self.positions[self.indices[t + k] as usize]);
        let mut b = NO_BOX;
        for i in 0..3 {
            let (a, c) = (v[i], v[(i + 1) % 3]);
            if a[axis] >= s0 && a[axis] <= s1 {
                b = merge(b, (a, a));
            }
            for s in [s0, s1] {
                if s.is_finite() && (a[axis] - s) * (c[axis] - s) < 0.0 {
                    let f = (s - a[axis]) / (c[axis] - a[axis]);
                    let mut p: [f32; 3] = std::array::from_fn(|k| a[k] + (c[k] - a[k]) * f);
                    p[axis] = s;
                    b = merge(b, (p, p));
                }
            }
        }
        if b.0[0] > b.1[0] {
            return None;
        }
        let pad = |x: f32| x.abs() * 4.0 * f32::EPSILON + f32::MIN_POSITIVE;
        let lo: [f32; 3] = std::array::from_fn(|k| (b.0[k] - pad(b.0[k])).max(r.lo[k]));
        let hi: [f32; 3] = std::array::from_fn(|k| (b.1[k] + pad(b.1[k])).min(r.hi[k]));
        Some((lo, std::array::from_fn(|k| hi[k].max(lo[k]))))
    }
    fn leaf(&mut self, refs: &[Ref]) -> u32 {
        let offset = self.bvh.indices.len() as u32;
        for r in refs {
            let t = r.tri as usize * 3;
            self.bvh.indices.extend_from_slice(&self.indices[t..t + 3]);
        }
        let id = self.bvh.leaves.len() as u32;
        self.bvh.leaves.push(Leaf {
            mesh: 0,
            cluster: id,
            page: 0,
            source_index_offset: refs[0].tri * 3,
            indices: Span { offset, count: (refs.len() * 3) as u32 },
            error: 0.0,
            material: 0,
        });
        self.leaf_boxes.push(refs.iter().fold(NO_BOX, |b, r| merge(b, (r.lo, r.hi))));
        id | LEAF_BIT
    }
    fn child_bounds(&self, child: u32) -> Boxed {
        if child & LEAF_BIT != 0 {
            return self.leaf_boxes[(child & !LEAF_BIT) as usize];
        }
        let n = &self.bvh.nodes[child as usize];
        let mut out = NO_BOX;
        for slot in 0..4 {
            if n.children[slot] == EMPTY {
                continue;
            }
            for (a, b) in [n.child_bounds(slot), n.child_bounds_fused(slot)] {
                out = merge(out, (a.map(|x| x as f32), b.map(|x| x as f32)));
            }
        }
        out
    }
}
/// An object split's plane: references whose centroid bin is at most `plane` go left.
#[derive(Clone, Copy)]
struct ObjectSplit {
    cost: f32,
    axis: usize,
    lo: f32,
    extent: f32,
    plane: usize,
    left: Boxed,
    right: Boxed,
}
impl ObjectSplit {
    fn bin(&self, r: &Ref) -> usize {
        (((r.centroid(self.axis) - self.lo) / self.extent * SAH_BINS as f32) as usize).min(SAH_BINS - 1)
    }
}

fn build(positions: &[[f32; 3]], indices: &[u32], sah: bool) -> Result<Bvh> {
    if indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err(Error("BVH geometry needs a nonempty triangle list"));
    }
    if indices.iter().any(|&i| i as usize >= positions.len())
        || positions.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(Error("BVH geometry has an invalid index or position"));
    }
    let triangles: Vec<usize> = (0..indices.len() / 3).collect();
    let centroids: Vec<[f32; 3]> = triangles
        .iter()
        .map(|&t| {
            std::array::from_fn(|a| {
                (0..3)
                    .map(|k| positions[indices[t * 3 + k] as usize][a])
                    .sum::<f32>()
                    / 3.0
            })
        })
        .collect();
    let boxes: Vec<([f32; 3], [f32; 3])> = if sah {
        (0..indices.len() / 3)
            .map(|t| bounds((0..3).map(|k| positions[indices[t * 3 + k] as usize])))
            .collect()
    } else {
        vec![]
    };
    let mut builder = Builder {
        positions,
        indices,
        centroids: &centroids,
        boxes: &boxes,
        bvh: Bvh {
            positions: positions.iter().map(|p| [p[0], p[1], p[2], 0.0]).collect(),
            ..Default::default()
        },
    };
    let root = builder.subtree(triangles);
    let mut bvh = builder.bvh;
    let leaves = bvh.leaves.len() as u32;
    bvh.level_leaves = (0..leaves).collect();
    bvh.levels.push(Level {
        mesh: 0,
        root,
        requested_error: 0.0,
        max_error: 0.0,
        leaves: Span {
            offset: 0,
            count: leaves,
        },
    });
    bvh.validate(cluster_bvh::Limits {
        max_bytes: usize::MAX,
    })?;
    Ok(bvh)
}

struct Builder<'a> {
    positions: &'a [[f32; 3]],
    indices: &'a [u32],
    centroids: &'a [[f32; 3]],
    /// Per triangle, its bounds: present for SAH builds (`build_bvh_sah`), empty for median.
    boxes: &'a [([f32; 3], [f32; 3])],
    bvh: Bvh,
}
/// Bins per axis of the SAH split.
const SAH_BINS: usize = 16;
fn half_area(lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let e: [f32; 3] = std::array::from_fn(|a| (hi[a] - lo[a]).max(0.0));
    e[0] * e[1] + e[1] * e[2] + e[2] * e[0]
}
fn merge(a: ([f32; 3], [f32; 3]), b: ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
    (
        std::array::from_fn(|i| a.0[i].min(b.0[i])),
        std::array::from_fn(|i| a.1[i].max(b.1[i])),
    )
}
impl Builder<'_> {
    /// Child ID of the subtree over `triangles` (a leaf, or a node in postorder).
    fn subtree(&mut self, triangles: Vec<usize>) -> u32 {
        if triangles.len() <= LEAF_TRIANGLES {
            return self.leaf(&triangles);
        }
        let mut groups = vec![triangles];
        // Two rounds of halving: at most four children.
        for _ in 0..2 {
            groups = groups
                .into_iter()
                .flat_map(|g| {
                    if g.len() <= LEAF_TRIANGLES {
                        return vec![g];
                    }
                    let (a, b) = self.split(g);
                    vec![a, b]
                })
                .collect();
        }
        let children: Vec<u32> = groups.into_iter().map(|g| self.subtree(g)).collect();
        self.node(&children)
    }
    fn split(&self, g: Vec<usize>) -> (Vec<usize>, Vec<usize>) {
        if !self.boxes.is_empty()
            && let Some(halves) = self.split_sah(&g)
        {
            return halves;
        }
        self.split_median(g)
    }
    /// The binned SAH plane over `g`'s centroids, or None when every centroid shares one bin
    /// on every axis (the median split then decides).
    fn split_sah(&self, g: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
        let (lo, hi) = bounds(g.iter().map(|&t| self.centroids[t]));
        let empty = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        let mut best: Option<(f32, usize, usize)> = None;
        for axis in 0..3 {
            let extent = hi[axis] - lo[axis];
            // Flat (or non-finite) along this axis: no plane separates the centroids.
            if extent.is_nan() || extent <= 0.0 {
                continue;
            }
            let bin = |t: usize| {
                let f = (self.centroids[t][axis] - lo[axis]) / extent * SAH_BINS as f32;
                (f as usize).min(SAH_BINS - 1)
            };
            let mut count = [0usize; SAH_BINS];
            let mut boxes = [empty; SAH_BINS];
            for &t in g {
                let b = bin(t);
                count[b] += 1;
                boxes[b] = merge(boxes[b], self.boxes[t]);
            }
            // Costs of the planes after bins 0..SAH_BINS-1: area(L) n(L) + area(R) n(R).
            let mut right = [0.0f32; SAH_BINS];
            let (mut acc, mut n) = (empty, 0usize);
            for b in (1..SAH_BINS).rev() {
                acc = merge(acc, boxes[b]);
                n += count[b];
                right[b] = half_area(acc.0, acc.1) * n as f32;
            }
            let (mut acc, mut n) = (empty, 0usize);
            for b in 0..SAH_BINS - 1 {
                acc = merge(acc, boxes[b]);
                n += count[b];
                if n == 0 || n == g.len() {
                    continue;
                }
                let cost = half_area(acc.0, acc.1) * n as f32 + right[b + 1];
                if best.is_none_or(|(c, _, _)| cost < c) {
                    best = Some((cost, axis, b));
                }
            }
        }
        let (_, axis, plane) = best?;
        let extent = hi[axis] - lo[axis];
        let (a, b): (Vec<usize>, Vec<usize>) = g.iter().partition(|&&t| {
            let f = (self.centroids[t][axis] - lo[axis]) / extent * SAH_BINS as f32;
            (f as usize).min(SAH_BINS - 1) <= plane
        });
        Some((a, b))
    }
    fn split_median(&self, mut g: Vec<usize>) -> (Vec<usize>, Vec<usize>) {
        let (lo, hi) = bounds(g.iter().map(|&t| self.centroids[t]));
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap();
        g.sort_by(|&a, &b| {
            self.centroids[a][axis]
                .total_cmp(&self.centroids[b][axis])
                .then(a.cmp(&b))
        });
        let b = g.split_off(g.len() / 2);
        (g, b)
    }
    fn leaf(&mut self, triangles: &[usize]) -> u32 {
        let offset = self.bvh.indices.len() as u32;
        for &t in triangles {
            self.bvh
                .indices
                .extend_from_slice(&self.indices[t * 3..t * 3 + 3]);
        }
        let id = self.bvh.leaves.len() as u32;
        self.bvh.leaves.push(Leaf {
            mesh: 0,
            cluster: id,
            page: 0,
            source_index_offset: (triangles[0] * 3) as u32,
            indices: Span {
                offset,
                count: (triangles.len() * 3) as u32,
            },
            error: 0.0,
            material: 0,
        });
        id | LEAF_BIT
    }
    fn child_bounds(&self, child: u32) -> ([f32; 3], [f32; 3]) {
        if child & LEAF_BIT != 0 {
            let leaf = &self.bvh.leaves[(child & !LEAF_BIT) as usize];
            let span =
                leaf.indices.offset as usize..(leaf.indices.offset + leaf.indices.count) as usize;
            bounds(
                self.bvh.indices[span]
                    .iter()
                    .map(|&i| self.positions[i as usize]),
            )
        } else {
            let n = &self.bvh.nodes[child as usize];
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for slot in 0..4 {
                if n.children[slot] == EMPTY {
                    continue;
                }
                // Either decode may be used, so the parent encloses both.
                for (a, b) in [n.child_bounds(slot), n.child_bounds_fused(slot)] {
                    for axis in 0..3 {
                        lo[axis] = lo[axis].min(a[axis] as f32);
                        hi[axis] = hi[axis].max(b[axis] as f32);
                    }
                }
            }
            (lo, hi)
        }
    }
    fn node(&mut self, children: &[u32]) -> u32 {
        let boxes: Vec<_> = children.iter().map(|&c| self.child_bounds(c)).collect();
        let node = quantized_node(children, &boxes);
        let id = self.bvh.nodes.len() as u32;
        self.bvh.nodes.push(node);
        id
    }
}

/// Refits a `build`/`build_bvh_sah` tree to new positions of the same vertices: the topology,
/// leaves and indices stay, every node's child boxes are recomputed (nodes are in postorder,
/// children first) and quantized as the build quantizes them. The same triangles, so the same
/// hits; only the boxes, and so the traversal cost, follow the new positions (a deforming mesh,
/// such as a skinned character, refits each pose in place of a rebuild).
pub fn refit_bvh(bvh: &mut Bvh, positions: &[[f32; 3]]) -> Result<()> {
    if positions.len() != bvh.positions.len() || positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error("BVH refit needs finite positions of the same vertices"));
    }
    bvh.positions = positions.iter().map(|p| [p[0], p[1], p[2], 0.0]).collect();
    for n in 0..bvh.nodes.len() {
        let children: Vec<u32> = bvh.nodes[n].children.iter().copied().filter(|&c| c != EMPTY).collect();
        let boxes: Vec<_> = children.iter().map(|&c| refit_child_bounds(bvh, positions, c)).collect();
        bvh.nodes[n] = quantized_node(&children, &boxes);
    }
    Ok(())
}
fn refit_child_bounds(bvh: &Bvh, positions: &[[f32; 3]], child: u32) -> ([f32; 3], [f32; 3]) {
    if child & LEAF_BIT != 0 {
        let leaf = &bvh.leaves[(child & !LEAF_BIT) as usize];
        let span = leaf.indices.offset as usize..(leaf.indices.offset + leaf.indices.count) as usize;
        return bounds(bvh.indices[span].iter().map(|&i| positions[i as usize]));
    }
    let n = &bvh.nodes[child as usize];
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for slot in 0..4 {
        if n.children[slot] == EMPTY {
            continue;
        }
        for (a, b) in [n.child_bounds(slot), n.child_bounds_fused(slot)] {
            for axis in 0..3 {
                lo[axis] = lo[axis].min(a[axis] as f32);
                hi[axis] = hi[axis].max(b[axis] as f32);
            }
        }
    }
    (lo, hi)
}
/// A node over `children` with `boxes`, its child bounds quantized outward so both the fused and
/// the unfused f32 decode enclose them.
fn quantized_node(children: &[u32], boxes: &[([f32; 3], [f32; 3])]) -> Node {
    {
        let (lo, hi) = boxes.iter().fold(
            ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |(lo, hi), (a, b)| {
                (
                    std::array::from_fn(|i| lo[i].min(a[i])),
                    std::array::from_fn(|i| hi[i].max(b[i])),
                )
            },
        );
        let mut node = Node {
            origin: lo,
            step: [0.0; 3],
            children: [EMPTY; 4],
            ..Default::default()
        };
        for axis in 0..3 {
            let extent = hi[axis] - lo[axis];
            let mut step = (extent / 255.0).max(f32::MIN_POSITIVE.max(lo[axis].abs() * 1e-7));
            // Grow until code 255 covers the top in both decodes.
            while lo[axis] + step * 255.0 < hi[axis] || step.mul_add(255.0, lo[axis]) < hi[axis] {
                step *= 1.0 + 1.0 / 1048576.0;
            }
            node.step[axis] = step;
        }
        for (slot, (&child, (a, b))) in children.iter().zip(boxes).enumerate() {
            node.children[slot] = child;
            for axis in 0..3 {
                let (o, s) = (node.origin[axis], node.step[axis]);
                let decode = |q: u8| [o + s * f32::from(q), s.mul_add(f32::from(q), o)];
                let mut low = (((a[axis] - o) / s).floor().clamp(0.0, 255.0)) as u8;
                while low > 0 && decode(low).iter().any(|&v| v > a[axis]) {
                    low -= 1;
                }
                let mut high = (((b[axis] - o) / s).ceil().clamp(0.0, 255.0)) as u8;
                while high < 255 && decode(high).iter().any(|&v| v < b[axis]) {
                    high += 1;
                }
                node.lower[axis][slot] = low;
                node.upper[axis][slot] = high;
            }
        }
        node
    }
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

// ---- encoding: explicit little-endian fields, never Rust layout ----
// Header: magic, version, mesh count, placement count, bins, then lod_error, reach and skip
// (f32 bits), then the source digest. Each mesh: u64 length and an RLB blob
// (cluster_bvh::encode). Each placement: mesh, 16 transform floats, u64 mask word count,
// then the mask words.
const HEADER_BYTES: usize = 32 + 32;

pub fn encode(field: &NearField, limits: Limits) -> Result<Vec<u8>> {
    field.validate(limits)?;
    let s = field.settings;
    let mut out = MAGIC.to_vec();
    for v in [
        VERSION,
        u32::try_from(field.meshes.len()).map_err(|_| Error("near-field mesh count"))?,
        u32::try_from(field.placements.len()).map_err(|_| Error("near-field placement count"))?,
        s.bins,
        s.lod_error.to_bits(),
        s.reach.to_bits(),
        s.skip.to_bits(),
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(field.source);
    let bvh_limits = cluster_bvh::Limits {
        max_bytes: limits.max_bytes,
    };
    for b in &field.meshes {
        let blob = cluster_bvh::encode(b, bvh_limits)?;
        out.extend((blob.len() as u64).to_le_bytes());
        out.extend(blob);
    }
    for p in &field.placements {
        out.extend(p.mesh.to_le_bytes());
        for v in p.transform.iter().flatten() {
            out.extend(v.to_le_bytes());
        }
        out.extend((p.masks.len() as u64).to_le_bytes());
        for w in &p.masks {
            out.extend(w.to_le_bytes());
        }
    }
    if out.len() > limits.max_bytes.saturating_add(HEADER_BYTES + 4096) {
        return Err(Error("near-field encoded size"));
    }
    Ok(out)
}

pub fn decode(bytes: &[u8], limits: Limits) -> Result<NearField> {
    if bytes.len() < HEADER_BYTES
        || &bytes[..4] != MAGIC
        || bytes.len() > limits.max_bytes.saturating_add(1 << 24)
    {
        return Err(Error("near-field header or size"));
    }
    let mut r = Reader { bytes, at: 4 };
    if r.u32()? != VERSION {
        return Err(Error("unsupported near-field version"));
    }
    let meshes = r.u32()? as usize;
    let placements = r.u32()? as usize;
    let settings = Settings {
        bins: r.u32()?,
        lod_error: f32::from_bits(r.u32()?),
        reach: f32::from_bits(r.u32()?),
        skip: f32::from_bits(r.u32()?),
    };
    settings.validate()?;
    let source: [u8; 32] = r.take(32)?.try_into().unwrap();
    // Counts are bounded by the bytes that must follow them.
    if meshes > bytes.len() / 8 || placements > bytes.len() / 76 {
        return Err(Error("near-field counts exceed the file"));
    }
    let bvh_limits = cluster_bvh::Limits {
        max_bytes: limits.max_bytes,
    };
    let mut field = NearField {
        settings,
        source,
        meshes: Vec::with_capacity(meshes),
        placements: Vec::with_capacity(placements),
    };
    for _ in 0..meshes {
        let n = r.u64()?;
        let blob = r.take(usize::try_from(n).map_err(|_| Error("near-field mesh length"))?)?;
        field.meshes.push(cluster_bvh::decode(blob, bvh_limits)?);
    }
    for _ in 0..placements {
        let mesh = r.u32()?;
        let mut transform = [[0f32; 4]; 4];
        for column in &mut transform {
            for v in column.iter_mut() {
                *v = f32::from_bits(r.u32()?);
            }
        }
        let words = usize::try_from(r.u64()?).map_err(|_| Error("near-field mask length"))?;
        let raw = r.take(
            words
                .checked_mul(4)
                .ok_or(Error("near-field mask length"))?,
        )?;
        let masks = raw
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        field.placements.push(NearPlacement {
            mesh,
            transform,
            masks,
        });
    }
    if r.at != bytes.len() {
        return Err(Error("near-field trailing bytes"));
    }
    field.validate(limits)?;
    Ok(field)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or(Error("near-field truncated"))?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refitting to the built positions gives the built tree; refitting to moved positions keeps
    /// its topology, leaves and indices, and every leaf's vertices stay inside the box its parent
    /// stores for it, under both decodes.
    #[test]
    fn refit_keeps_the_tree_and_encloses_moved_triangles() {
        let (positions, indices) = grid(24);
        let built = build_bvh_sah(&positions, &indices).unwrap();
        let mut same = built.clone();
        refit_bvh(&mut same, &positions).unwrap();
        assert_eq!(same, built);
        let moved: Vec<[f32; 3]> = positions
            .iter()
            .enumerate()
            .map(|(i, p)| [p[0] * 1.7 + (i as f32 * 0.37).sin() * 0.2, (i as f32 * 0.11).cos(), p[2] - 3.0])
            .collect();
        let mut refit = built.clone();
        refit_bvh(&mut refit, &moved).unwrap();
        assert_eq!((refit.nodes.len(), &refit.leaves, &refit.indices), (built.nodes.len(), &built.leaves, &built.indices));
        let mut leaves = 0;
        for node in &refit.nodes {
            for slot in 0..4 {
                let child = node.children[slot];
                if child == EMPTY || child & LEAF_BIT == 0 {
                    continue;
                }
                let leaf = &refit.leaves[(child & !LEAF_BIT) as usize];
                let span = leaf.indices.offset as usize..(leaf.indices.offset + leaf.indices.count) as usize;
                for (lo, hi) in [node.child_bounds(slot), node.child_bounds_fused(slot)] {
                    for &i in &refit.indices[span.clone()] {
                        let v = moved[i as usize];
                        assert!((0..3).all(|a| lo[a] <= f64::from(v[a]) && f64::from(v[a]) <= hi[a]));
                    }
                }
                leaves += 1;
            }
        }
        assert_eq!(leaves, refit.leaves.len());
        assert!(refit_bvh(&mut refit, &moved[1..]).is_err());
    }

    fn grid(n: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
        let mut p = vec![];
        let mut i = vec![];
        for y in 0..=n {
            for x in 0..=n {
                p.push([x as f32 / n as f32, 0.0, y as f32 / n as f32]);
            }
        }
        for y in 0..n {
            for x in 0..n {
                let a = (y * (n + 1) + x) as u32;
                let b = a + 1;
                let c = a + n as u32 + 1;
                i.extend_from_slice(&[a, c, b, b, c, c + 1]);
            }
        }
        (p, i)
    }

    #[test]
    fn builds_valid_deterministic_bvh4() {
        let (p, i) = grid(24);
        let a = build_bvh(&p, &i).unwrap();
        let b = build_bvh(&p, &i).unwrap();
        assert_eq!(a, b);
        assert!(
            a.nodes
                .iter()
                .all(|n| n.children.iter().filter(|&&c| c != EMPTY).count() >= 2)
        );
        assert_eq!(a.indices.len(), i.len());
        // A single small mesh is one leaf root.
        let small = build_bvh(&p, &i[..6]).unwrap();
        assert!(small.nodes.is_empty() && small.levels[0].root & LEAF_BIT != 0);
        assert!(build_bvh(&[[0.0; 3]], &[0, 0]).is_err());
        assert!(build_bvh(&[[f32::NAN; 3]; 3], &[0, 1, 2]).is_err());
    }

    #[test]
    fn bins_round_trip_through_their_centres() {
        for bins in [8, 16, 32] {
            for b in 0..bins * bins {
                let d = bin_direction(b, bins);
                assert!(((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) - 1.).abs() < 1e-5);
                assert_eq!(bin(d, bins), b, "bins {bins} bin {b}");
            }
        }
        // Axes and their opposites land in distinct bins.
        let axes = [
            [1., 0., 0.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
        ];
        let set: std::collections::BTreeSet<u32> = axes.iter().map(|&d| bin(d, 16)).collect();
        assert_eq!(set.len(), 6);
    }

    #[test]
    fn encodes_validates_and_rejects_corruption() {
        let (p, i) = grid(6);
        let bvh = build_bvh(&p, &i).unwrap();
        let settings = Settings {
            lod_error: 1.,
            reach: 0.5,
            skip: 0.01,
            bins: 16,
        };
        let words = settings.words();
        let field = NearField {
            settings,
            source: [7; 32],
            placements: vec![NearPlacement {
                mesh: 0,
                transform: [
                    [1., 0., 0., 0.],
                    [0., 1., 0., 0.],
                    [0., 0., 1., 0.],
                    [2., 0., 0., 1.],
                ],
                masks: (0..bvh.positions.len() * 2 * words)
                    .map(|k| (k as u32).wrapping_mul(2654435761))
                    .collect(),
            }],
            meshes: vec![bvh],
        };
        let bytes = encode(&field, Limits::default()).unwrap();
        assert_eq!(decode(&bytes, Limits::default()).unwrap(), field);
        assert!(decode(&bytes[..bytes.len() - 1], Limits::default()).is_err());
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(decode(&extra, Limits::default()).is_err());
        let mut version = bytes.clone();
        version[4] = 9;
        assert!(decode(&version, Limits::default()).is_err());
        let mut short = field.clone();
        short.placements[0].masks.pop();
        assert!(encode(&short, Limits::default()).is_err());
        let mut bad = field.clone();
        bad.settings.skip = 1.0;
        assert!(encode(&bad, Limits::default()).is_err());
        assert!(encode(&field, Limits { max_bytes: 64 }).is_err());
    }

    /// Closest hit along (o, d) over `tris` triangles (each three corners), by Moller-Trumbore in
    /// f64: the same per-triangle arithmetic for the brute force and the tree walk.
    fn hit(v: [[f32; 3]; 3], o: [f64; 3], d: [f64; 3]) -> Option<f64> {
        let p = |k: usize| v[k].map(f64::from);
        let (a, b, c) = (p(0), p(1), p(2));
        let sub = |x: [f64; 3], y: [f64; 3]| [x[0] - y[0], x[1] - y[1], x[2] - y[2]];
        let cross = |x: [f64; 3], y: [f64; 3]| [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
        let dot = |x: [f64; 3], y: [f64; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
        let (e1, e2) = (sub(b, a), sub(c, a));
        let q = cross(d, e2);
        let det = dot(e1, q);
        if det.abs() < 1e-14 {
            return None;
        }
        let s = sub(o, a);
        let u = dot(s, q) / det;
        let r = cross(s, e1);
        let w = dot(d, r) / det;
        let t = dot(e2, r) / det;
        (u >= 0.0 && w >= 0.0 && u + w <= 1.0 && t > 1e-9).then_some(t)
    }
    /// The tree's closest hit: a stack walk over the quantized child boxes (unfused decode).
    fn walk(bvh: &Bvh, o: [f64; 3], d: [f64; 3]) -> Option<f64> {
        let mut best = f64::INFINITY;
        let mut stack = vec![bvh.levels[0].root];
        while let Some(child) = stack.pop() {
            if child & LEAF_BIT != 0 {
                let leaf = &bvh.leaves[(child & !LEAF_BIT) as usize];
                let span = leaf.indices.offset as usize..(leaf.indices.offset + leaf.indices.count) as usize;
                for t in bvh.indices[span].chunks_exact(3) {
                    let v = std::array::from_fn(|k| {
                        let p = bvh.positions[t[k] as usize];
                        [p[0], p[1], p[2]]
                    });
                    if let Some(t) = hit(v, o, d) {
                        best = best.min(t);
                    }
                }
                continue;
            }
            let n = &bvh.nodes[child as usize];
            for slot in 0..4 {
                if n.children[slot] == EMPTY {
                    continue;
                }
                let (lo, hi) = n.child_bounds(slot);
                let (mut near, mut far) = (0.0f64, best);
                for a in 0..3 {
                    if d[a] == 0.0 {
                        if o[a] < lo[a] || o[a] > hi[a] {
                            far = -1.0;
                        }
                        continue;
                    }
                    let (t0, t1) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
                    near = near.max(t0.min(t1));
                    far = far.min(t0.max(t1));
                }
                if near <= far {
                    stack.push(n.children[slot]);
                }
            }
        }
        best.is_finite().then_some(best)
    }

    /// Spatial splits and small leaves keep every closest hit of a whole-triangle brute force:
    /// a grid crossed by long slivers in every direction (the case spatial splits cut), random
    /// rays from inside and around it. Splits add references (each triangle sits in at least one
    /// leaf) and stay within the 2x reference budget.
    #[test]
    fn split_builds_keep_every_closest_hit() {
        let (mut positions, mut indices) = grid(16);
        let mut seed = 0x9e37_79b9u32;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        for _ in 0..200 {
            let base = positions.len() as u32;
            let a = [rand() * 1.4 - 0.2, rand(), rand() * 1.4 - 0.2];
            let b = [rand() * 1.4 - 0.2, rand(), rand() * 1.4 - 0.2];
            let c = [a[0] + (rand() - 0.5) * 0.05, a[1] + (rand() - 0.5) * 0.05, a[2] + (rand() - 0.5) * 0.05];
            positions.extend_from_slice(&[a, b, c]);
            indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
        let tris: Vec<[[f32; 3]; 3]> = indices
            .chunks_exact(3)
            .map(|t| std::array::from_fn(|k| positions[t[k] as usize]))
            .collect();
        for (leaf_max, spatial) in [(8, false), (2, false), (4, true), (2, true), (1, true)] {
            let bvh = build_bvh_split(&positions, &indices, leaf_max, spatial).unwrap();
            let refs = bvh.indices.len() / 3;
            assert!(refs >= tris.len() && refs <= 2 * tris.len(), "{leaf_max} {spatial}: {refs} references");
            assert!(bvh.leaves.iter().all(|l| l.indices.count as usize <= 3 * leaf_max));
            if spatial {
                assert!(refs > tris.len(), "the slivers are split");
            }
            let mut hits = 0;
            for _ in 0..10000 {
                let o = [rand() * 1.8 - 0.4, rand() * 1.4 - 0.2, rand() * 1.8 - 0.4].map(f64::from);
                let z = f64::from(rand()) * 2.0 - 1.0;
                let phi = f64::from(rand()) * std::f64::consts::TAU;
                let r = (1.0 - z * z).sqrt();
                let d = [r * phi.cos(), z, r * phi.sin()];
                let brute = tris.iter().filter_map(|&v| hit(v, o, d)).min_by(f64::total_cmp);
                assert_eq!(walk(&bvh, o, d), brute, "leaf {leaf_max} spatial {spatial} ray {o:?} {d:?}");
                hits += usize::from(brute.is_some());
            }
            assert!(hits > 1000, "{hits} of 10000 rays hit");
        }
    }
}
