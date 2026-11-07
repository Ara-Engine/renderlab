//! Cooked object-space software-ray geometry. No shading model or GPU dependency.
//!
//! A level is a complete legal cut of the cluster DAG. Select one level per ray,
//! never individual shards: independent shard refinement would overlap geometry.
mod codec;
mod reference;
use crate::{Error, Result, virtual_mesh::Span};
pub use codec::{decode, encode};
pub use reference::{Cone, Hit, Ray};

pub const VERSION: u32 = 1;
pub const LEAF_BIT: u32 = 1 << 31;
pub const EMPTY: u32 = u32::MAX;
pub const NODE_BYTES: usize = 80;
pub const LEAF_BYTES: usize = 32;
pub const LEVEL_BYTES: usize = 24;

/// Six packed u32s hold the axis-major 8-bit child bounds on the GPU.
/// Empty slots have child=EMPTY and zero bounds/error. Child error is the
/// maximum accumulated simplifier error below that child, in object units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Node {
    pub origin: [f32; 3],
    pub step: [f32; 3],
    pub children: [u32; 4],
    pub lower: [[u8; 4]; 3],
    pub upper: [[u8; 4]; 3],
    pub errors: [f32; 4],
}
impl Node {
    /// Outward rounding also covers the f32 multiply/add used by a future GPU
    /// decoder. Quantization enlarges traversal boxes, never triangle geometry.
    pub fn child_bounds(&self, slot: usize) -> ([f64; 3], [f64; 3]) {
        self.decode_bounds(slot, false)
    }
    pub fn child_bounds_fused(&self, slot: usize) -> ([f64; 3], [f64; 3]) {
        self.decode_bounds(slot, true)
    }
    fn decode_bounds(&self, slot: usize, fused: bool) -> ([f64; 3], [f64; 3]) {
        let plane = |axis: usize, q: u8| {
            let value = if fused {
                self.step[axis].mul_add(f32::from(q), self.origin[axis])
            } else {
                self.origin[axis] + self.step[axis] * f32::from(q)
            };
            f64::from(value)
        };
        (
            std::array::from_fn(|a| plane(a, self.lower[a][slot])),
            std::array::from_fn(|a| plane(a, self.upper[a][slot])),
        )
    }
}

/// Geometry indices are local to this sidecar. Source page/span and cluster ID
/// preserve identity for later programmable hit shading; no material output is baked.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Leaf {
    pub mesh: u32,
    pub cluster: u32,
    pub page: u32,
    pub source_index_offset: u32,
    pub indices: Span,
    pub error: f32,
    pub material: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Level {
    pub mesh: u32,
    pub root: u32,
    pub requested_error: f32,
    pub max_error: f32,
    pub leaves: Span,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bvh {
    pub nodes: Vec<Node>,
    pub leaves: Vec<Leaf>,
    pub levels: Vec<Level>,
    pub level_leaves: Vec<u32>,
    /// xyz plus zero padding; directly usable as a storage-buffer vec4 array.
    pub positions: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Encoded file and owned decoded arrays are each checked against this cap.
    pub max_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 150_000_000,
        }
    }
}
impl Bvh {
    /// Bind an attachment to its cluster identities (page data is verified by IO).
    pub fn validate_virtual(&self, package: &crate::virtual_mesh::Package) -> Result<()> {
        for leaf in &self.leaves {
            let mesh = package
                .meshes
                .get(leaf.mesh as usize)
                .ok_or(Error("BVH source mesh"))?;
            let c = mesh
                .clusters
                .get(leaf.cluster as usize)
                .ok_or(Error("BVH source cluster"))?;
            let error = match c.refined {
                Some(r) => {
                    mesh.groups
                        .get(r.index())
                        .ok_or(Error("BVH source refined group"))?
                        .simplified
                        .error
                }
                None => 0.,
            };
            if c.page.0 != leaf.page
                || c.indices.offset != leaf.source_index_offset
                || c.indices.count != leaf.indices.count
                || c.material != leaf.material
                || error != leaf.error
            {
                return Err(Error("BVH source cluster identity/error"));
            }
        }
        Ok(())
    }
    pub fn gpu_bytes(&self) -> usize {
        self.nodes.len() * NODE_BYTES
            + self.leaves.len() * LEAF_BYTES
            + self.levels.len() * LEVEL_BYTES
            + self.level_leaves.len() * 4
            + self.positions.len() * 16
            + self.indices.len() * 4
    }
    /// w and k are in object space. For an expanding cone, use its minimum
    /// width over the traced interval; this conservatively satisfies e <= k*w(t).
    /// No matching cooked level is explicit (None), never a silent quality fallback.
    pub fn select_level(&self, mesh: u32, width: f32, k: f32) -> Option<usize> {
        if !width.is_finite() || !k.is_finite() || width < 0. || k < 0. {
            return None;
        }
        let budget = f64::from(width) * f64::from(k);
        self.select_budget(mesh, budget)
    }
    /// Conservatively choose one complete cut at the cone's narrowest point in
    /// the ray interval. Growing cones can never violate this choice farther out.
    pub fn select_cone_level(&self, mesh: u32, ray: Ray, cone: Cone, k: f64) -> Option<usize> {
        if !ray.valid() || !cone.valid() || !k.is_finite() || k < 0. {
            return None;
        }
        self.select_budget(mesh, cone.width_at(ray.t_min) * k)
    }
    fn select_budget(&self, mesh: u32, budget: f64) -> Option<usize> {
        self.levels
            .iter()
            .enumerate()
            .filter(|(_, l)| l.mesh == mesh && f64::from(l.max_error) <= budget)
            .max_by(|(_, a), (_, b)| a.requested_error.total_cmp(&b.requested_error))
            .map(|(i, _)| i)
    }
    /// Checks topology, ranges, quantized enclosure, error envelopes, and exact
    /// agreement between every root's reachable leaves and its legal-cut list.
    pub fn validate(&self, limits: Limits) -> Result<()> {
        if self.gpu_bytes() > limits.max_bytes
            || self.nodes.len() >= LEAF_BIT as usize
            || self.leaves.len() >= (LEAF_BIT - 1) as usize
            || self.levels.is_empty()
        {
            return Err(Error("cluster BVH byte/count limit or empty levels"));
        }
        for p in &self.positions {
            if !p.iter().all(|v| v.is_finite()) || p[3] != 0. {
                return Err(Error("invalid cluster BVH position"));
            }
        }
        if self
            .indices
            .iter()
            .any(|&i| i as usize >= self.positions.len())
        {
            return Err(Error("cluster BVH vertex index"));
        }
        let mut end = 0;
        for l in &self.leaves {
            let r = l.indices.range(self.indices.len())?;
            if r.start != end
                || r.is_empty()
                || !r.len().is_multiple_of(3)
                || !l.error.is_finite()
                || l.error < 0.
            {
                return Err(Error("invalid cluster BVH leaf"));
            }
            end = r.end;
        }
        if end != self.indices.len() {
            return Err(Error("unowned cluster BVH indices"));
        }
        let mut boxes = Vec::with_capacity(self.nodes.len());
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.origin.iter().chain(&n.step).all(|v| v.is_finite())
                || n.step.iter().any(|&v| v <= 0.)
                || n.children.iter().all(|&c| c == EMPTY)
            {
                return Err(Error("invalid cluster BVH node frame"));
            }
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            let mut error = 0f32;
            for slot in 0..4 {
                let child = n.children[slot];
                if child == EMPTY {
                    if n.errors[slot] != 0.
                        || (0..3).any(|a| n.lower[a][slot] != 0 || n.upper[a][slot] != 0)
                    {
                        return Err(Error("noncanonical empty BVH child"));
                    }
                    continue;
                }
                let (clo, chi, ce) = if child & LEAF_BIT != 0 {
                    let leaf = self
                        .leaves
                        .get((child & !LEAF_BIT) as usize)
                        .ok_or(Error("BVH leaf reference"))?;
                    let (a, b) = self.leaf_bounds(leaf);
                    (a, b, leaf.error)
                } else {
                    if child as usize >= i {
                        return Err(Error("BVH unordered/cyclic node"));
                    }
                    boxes[child as usize]
                };
                let (a, b) = n.child_bounds(slot);
                let (fa, fb) = n.child_bounds_fused(slot);
                if !n.errors[slot].is_finite()
                    || n.errors[slot] != ce
                    || (0..3).any(|axis| {
                        !a[axis].is_finite()
                            || !b[axis].is_finite()
                            || !fa[axis].is_finite()
                            || !fb[axis].is_finite()
                            || a[axis] > clo[axis]
                            || b[axis] < chi[axis]
                            || fa[axis] > clo[axis]
                            || fb[axis] < chi[axis]
                    })
                {
                    return Err(Error("BVH child bounds/error do not enclose geometry"));
                }
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(a[axis]).min(fa[axis]);
                    hi[axis] = hi[axis].max(b[axis]).max(fb[axis]);
                }
                error = error.max(ce);
            }
            boxes.push((lo, hi, error));
        }
        let mut used_nodes = vec![false; self.nodes.len()];
        let mut used_leaves = vec![false; self.leaves.len()];
        let mut cut_end = 0;
        let mut previous: Option<(u32, f32)> = None;
        for level in &self.levels {
            if !level.requested_error.is_finite()
                || level.requested_error < 0.
                || !level.max_error.is_finite()
                || level.max_error < 0.
                || level.max_error > level.requested_error
                || previous.is_some_and(|(m, e)| {
                    level.mesh < m || (level.mesh == m && level.requested_error <= e)
                })
            {
                return Err(Error("invalid BVH level/error order"));
            }
            previous = Some((level.mesh, level.requested_error));
            let r = level.leaves.range(self.level_leaves.len())?;
            if r.start != cut_end || r.is_empty() {
                return Err(Error("invalid BVH cut span"));
            }
            cut_end = r.end;
            let expected = &self.level_leaves[r];
            if expected.windows(2).any(|w| w[0] >= w[1]) {
                return Err(Error("BVH duplicate/unordered cut leaf"));
            }
            let mut found = Vec::with_capacity(expected.len());
            let mut stack = vec![level.root];
            let mut visits = 0usize;
            let mut error = 0f32;
            while let Some(child) = stack.pop() {
                visits += 1;
                if visits > self.nodes.len() + self.leaves.len() {
                    return Err(Error("BVH repeated subtree"));
                }
                if child == EMPTY {
                    return Err(Error("BVH empty root"));
                }
                if child & LEAF_BIT != 0 {
                    let id = (child & !LEAF_BIT) as usize;
                    let leaf = self.leaves.get(id).ok_or(Error("BVH cut leaf reference"))?;
                    if leaf.mesh != level.mesh {
                        return Err(Error("BVH cross-mesh cut"));
                    }
                    used_leaves[id] = true;
                    error = error.max(leaf.error);
                    found.push(id as u32);
                } else {
                    let n = self
                        .nodes
                        .get(child as usize)
                        .ok_or(Error("BVH root reference"))?;
                    used_nodes[child as usize] = true;
                    stack.extend(n.children.into_iter().filter(|&c| c != EMPTY));
                }
            }
            found.sort_unstable();
            if found != expected || error != level.max_error {
                return Err(Error("BVH cut/root disagreement"));
            }
        }
        if cut_end != self.level_leaves.len()
            || used_nodes.contains(&false)
            || used_leaves.contains(&false)
        {
            return Err(Error("unreachable cluster BVH records"));
        }
        Ok(())
    }
    pub fn leaf_bounds(&self, leaf: &Leaf) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        // Validated ranges are a precondition for all reference operations.
        for &id in &self.indices
            [leaf.indices.offset as usize..(leaf.indices.offset + leaf.indices.count) as usize]
        {
            for a in 0..3 {
                lo[a] = lo[a].min(f64::from(self.positions[id as usize][a]));
                hi[a] = hi[a].max(f64::from(self.positions[id as usize][a]));
            }
        }
        (lo, hi)
    }
}
