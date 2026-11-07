//! Deterministic legal cluster cuts and a simple spatial BVH4 builder.
//! Quantized child boxes follow the idea in Ylitie/Karras/Laine (HPG 2017),
//! implemented here from the paper. No NVIDIA code or compressed-stack scheme.
use crate::{Error, Result, ensure};
use renderlab_asset::{
    cluster_bvh::*,
    virtual_mesh::{self, ClusterId, Package, PageGeometry, SelectionBudget, Span, VirtualLimits},
};
use std::collections::BTreeSet;

pub const ALGORITHM: &[u8] = b"renderlab-cluster-bvh/1/legal-uniform-cuts/bvh4-median/byte-outward-boxes/shared-page-positions/f64-reference";
#[derive(Clone, Debug)]
pub struct Settings {
    /// Ascending object-space accumulated geometric errors, not projected pixels.
    pub errors: Vec<f32>,
    pub limits: Limits,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure(
            !self.errors.is_empty()
                && self.errors.len() <= u32::MAX as usize
                && self.errors.iter().all(|e| e.is_finite() && *e >= 0.)
                && self.errors.windows(2).all(|w| w[0] < w[1]),
            "BVH errors must be finite nonnegative ascending values within u32 count width",
        )
    }
}
pub fn cut(
    package: &Package,
    mesh: usize,
    error: f32,
    limits: VirtualLimits,
) -> Result<Vec<ClusterId>> {
    let m = &package.meshes[mesh];
    if error == 0. {
        return Ok(m
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| c.refined.is_none())
            .map(|(i, _)| ClusterId(i as u32))
            .collect());
    }
    let errors: Vec<_> = m
        .groups
        .iter()
        .map(|g| f64::from(g.simplified.error))
        .collect();
    let selection = virtual_mesh::select(
        m,
        &errors,
        f64::from(error),
        &vec![true; package.pages.len()],
        SelectionBudget {
            geometry_bytes: usize::MAX,
            max_candidates: usize::MAX,
            max_clusters: usize::MAX,
            max_scratch_bytes: limits.max_metadata_bytes,
        },
    )?;
    ensure(
        selection.debt.unresolved_groups == 0 && selection.debt.reasons.is_empty(),
        "BVH cut has unresolved quality debt",
    )?;
    Ok(selection.cut.clusters)
}

/// Reads only selected pages, one at a time. Each unique cluster and referenced
/// page vertex is stored once across the levels of a mesh. Caller owns IO/hash policy.
pub fn prepare(
    package: &Package,
    settings: &Settings,
    limits: VirtualLimits,
    mut load_page: impl FnMut(usize) -> Result<PageGeometry>,
) -> Result<Bvh> {
    settings.validate()?;
    package.validate(limits)?;
    let mut bvh = Bvh::default();
    for (mi, mesh) in package.meshes.iter().enumerate() {
        let mut cut_bytes = settings
            .errors
            .len()
            .checked_mul(std::mem::size_of::<Vec<ClusterId>>())
            .ok_or_else(|| Error("BVH cut scratch overflow".into()))?;
        ensure(
            cut_bytes <= settings.limits.max_bytes,
            "BVH level scratch byte budget",
        )?;
        let mut cuts = Vec::with_capacity(settings.errors.len());
        for &e in &settings.errors {
            let selected = cut(package, mi, e, limits)?;
            cut_bytes = cut_bytes
                .checked_add(
                    selected
                        .len()
                        .checked_mul(4)
                        .ok_or_else(|| Error("BVH cut scratch overflow".into()))?,
                )
                .ok_or_else(|| Error("BVH cut scratch overflow".into()))?;
            ensure(
                cut_bytes <= settings.limits.max_bytes,
                "BVH cut scratch byte budget",
            )?;
            cuts.push(selected);
        }
        let unique: BTreeSet<_> = cuts.iter().flatten().copied().collect();
        let mut leaf_map = vec![EMPTY; mesh.clusters.len()];
        let mut order: Vec<_> = unique.iter().copied().collect();
        order.sort_unstable_by_key(|id| (mesh.clusters[id.index()].page, *id));
        let index_count = order.iter().try_fold(0usize, |n, id| {
            n.checked_add(mesh.clusters[id.index()].indices.count as usize)
                .ok_or_else(|| Error("BVH indices overflow".into()))
        })?;
        // Worst-case page-local duplication, before retaining any geometry. IDs and
        // counts retain structural caps even if the caller disables the byte policy.
        let upper_vertices = order
            .iter()
            .map(|id| mesh.clusters[id.index()].page)
            .collect::<BTreeSet<_>>()
            .iter()
            .try_fold(0usize, |n, p| {
                n.checked_add(package.pages[p.index()].vertices as usize)
                    .ok_or_else(|| Error("BVH vertices overflow".into()))
            })?;
        let extra = index_count
            .checked_mul(4)
            .and_then(|n| n.checked_add(upper_vertices.checked_mul(16)?))
            .and_then(|n| n.checked_add(unique.len().checked_mul(LEAF_BYTES)?))
            .ok_or_else(|| Error("BVH preparation size overflow".into()))?;
        ensure(
            bvh.gpu_bytes()
                .checked_add(extra)
                .is_some_and(|n| n <= settings.limits.max_bytes),
            "BVH geometry exceeds byte budget (choose coarser errors)",
        )?;
        ensure(
            bvh.indices
                .len()
                .checked_add(index_count)
                .is_some_and(|n| n <= u32::MAX as usize)
                && bvh
                    .positions
                    .len()
                    .checked_add(upper_vertices)
                    .is_some_and(|n| n <= u32::MAX as usize)
                && bvh
                    .leaves
                    .len()
                    .checked_add(unique.len())
                    .is_some_and(|n| n < (LEAF_BIT - 1) as usize),
            "BVH geometry ID overflow",
        )?;
        bvh.indices.reserve_exact(index_count);
        bvh.positions.reserve_exact(upper_vertices);
        bvh.leaves.reserve_exact(unique.len());
        let mut current = None;
        let mut geometry = PageGeometry {
            vertices: vec![],
            indices: vec![],
            tangents: None,
        };
        let mut vertices = vec![];
        for id in order {
            let c = &mesh.clusters[id.index()];
            if current != Some(c.page) {
                geometry = load_page(c.page.index())?;
                ensure(
                    geometry.vertices.len() == package.pages[c.page.index()].vertices as usize
                        && geometry.indices.len() == package.pages[c.page.index()].indices as usize,
                    "BVH page geometry extent",
                )?;
                vertices = vec![EMPTY; geometry.vertices.len()];
                current = Some(c.page);
            }
            let offset = bvh.indices.len() as u32;
            for &source in &geometry.indices[c.indices.range(geometry.indices.len())?] {
                let mapped = vertices
                    .get_mut(source as usize)
                    .ok_or_else(|| Error("BVH source vertex index".into()))?;
                if *mapped == EMPTY {
                    *mapped = bvh.positions.len() as u32;
                    let p = geometry.vertices[source as usize].position;
                    bvh.positions.push([p[0], p[1], p[2], 0.]);
                }
                bvh.indices.push(*mapped);
            }
            let error = c
                .refined
                .map_or(0., |r| mesh.groups[r.index()].simplified.error);
            leaf_map[id.index()] = bvh.leaves.len() as u32;
            bvh.leaves.push(Leaf {
                mesh: mi as u32,
                cluster: id.0,
                page: c.page.0,
                source_index_offset: c.indices.offset,
                indices: Span {
                    offset,
                    count: c.indices.count,
                },
                error,
                material: c.material,
            });
        }
        for (&requested_error, cut) in settings.errors.iter().zip(cuts) {
            let mut ids: Vec<_> = cut.iter().map(|c| leaf_map[c.index()]).collect();
            ids.sort_unstable();
            let max_error = ids
                .iter()
                .map(|&id| bvh.leaves[id as usize].error)
                .fold(0f32, f32::max);
            ensure(
                max_error <= requested_error,
                "BVH selected cut exceeds geometric error",
            )?;
            let leaves = Span {
                offset: bvh
                    .level_leaves
                    .len()
                    .try_into()
                    .map_err(|_| Error("BVH cut offset overflow".into()))?,
                count: ids.len() as u32,
            };
            bvh.level_leaves.extend(&ids);
            let root = build_tree(&mut bvh, &mut ids, settings.limits)?;
            bvh.levels.push(Level {
                mesh: mi as u32,
                root,
                requested_error,
                max_error,
                leaves,
            });
        }
    }
    bvh.positions.shrink_to_fit();
    bvh.validate(settings.limits)?;
    Ok(bvh)
}
#[derive(Clone, Copy)]
struct Child {
    id: u32,
    lo: [f64; 3],
    hi: [f64; 3],
    error: f32,
}
fn build_tree(b: &mut Bvh, ids: &mut [u32], limits: Limits) -> Result<u32> {
    Ok(build(b, ids, limits)?.id)
}
fn build(b: &mut Bvh, ids: &mut [u32], limits: Limits) -> Result<Child> {
    ensure(!ids.is_empty(), "empty BVH cut")?;
    if ids.len() == 1 {
        let leaf = &b.leaves[ids[0] as usize];
        let (lo, hi) = b.leaf_bounds(leaf);
        return Ok(Child {
            id: LEAF_BIT | ids[0],
            lo,
            hi,
            error: leaf.error,
        });
    }
    // Stable IDs break centroid ties. Precompute bounds once per recursion level,
    // rather than walking triangles from inside the sorting comparator.
    let mut items: Vec<_> = ids
        .iter()
        .map(|&id| {
            let (lo, hi) = b.leaf_bounds(&b.leaves[id as usize]);
            (id, lo, hi)
        })
        .collect();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (_, a, z) in &items {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(a[axis] + z[axis]);
            hi[axis] = hi[axis].max(a[axis] + z[axis]);
        }
    }
    let axis = (0..3)
        .max_by(|&a, &z| {
            (hi[a] - lo[a])
                .total_cmp(&(hi[z] - lo[z]))
                .then_with(|| z.cmp(&a))
        })
        .unwrap();
    items.sort_unstable_by(|a, z| {
        (a.1[axis] + a.2[axis])
            .total_cmp(&(z.1[axis] + z.2[axis]))
            .then_with(|| a.0.cmp(&z.0))
    });
    for (id, item) in ids.iter_mut().zip(items) {
        *id = item.0;
    }
    let chunk = ids.len().div_ceil(4);
    let children = ids
        .chunks_mut(chunk)
        .map(|c| build(b, c, limits))
        .collect::<Result<Vec<_>>>()?;
    let mut n = Node {
        children: [EMPTY; 4],
        ..Node::default()
    };
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for c in &children {
        for a in 0..3 {
            lo[a] = lo[a].min(c.lo[a]);
            hi[a] = hi[a].max(c.hi[a]);
        }
    }
    for a in 0..3 {
        // Leave a cell of headroom on either side. Frame coordinates are rounded
        // out in f32; loops below verify the *decoded* planes before accepting.
        let scale = lo[a].abs().max(hi[a].abs()).max(1.);
        let pad = scale * f64::from(f32::EPSILON) * 8.;
        n.origin[a] = ((lo[a] - pad) as f32).next_down();
        n.step[a] = (((hi[a] + pad - f64::from(n.origin[a])) / 253.) as f32).next_up();
        ensure(
            n.origin[a].is_finite() && n.step[a].is_finite() && n.step[a] > 0.,
            "BVH quantization domain overflow",
        )?;
        for (slot, c) in children.iter().enumerate() {
            let qlo = ((c.lo[a] - f64::from(n.origin[a])) / f64::from(n.step[a])).floor();
            let qhi = ((c.hi[a] - f64::from(n.origin[a])) / f64::from(n.step[a])).ceil();
            ensure(qlo >= 0. && qhi <= 255., "BVH quantization frame")?;
            n.lower[a][slot] = qlo as u8;
            n.upper[a][slot] = qhi as u8;
            while n.child_bounds(slot).0[a].max(n.child_bounds_fused(slot).0[a]) > c.lo[a] {
                n.lower[a][slot] = n.lower[a][slot]
                    .checked_sub(1)
                    .ok_or_else(|| Error("BVH lower rounding".into()))?;
            }
            while n.child_bounds(slot).1[a].min(n.child_bounds_fused(slot).1[a]) < c.hi[a] {
                n.upper[a][slot] = n.upper[a][slot]
                    .checked_add(1)
                    .ok_or_else(|| Error("BVH upper rounding".into()))?;
            }
        }
    }
    let mut error = 0f32;
    let mut decoded_lo = [f64::INFINITY; 3];
    let mut decoded_hi = [f64::NEG_INFINITY; 3];
    for (slot, c) in children.iter().enumerate() {
        n.children[slot] = c.id;
        n.errors[slot] = c.error;
        error = error.max(c.error);
        let (a, z) = n.child_bounds(slot);
        let (fa, fz) = n.child_bounds_fused(slot);
        for axis in 0..3 {
            decoded_lo[axis] = decoded_lo[axis].min(a[axis]).min(fa[axis]);
            decoded_hi[axis] = decoded_hi[axis].max(z[axis]).max(fz[axis]);
        }
    }
    ensure(
        b.nodes.len() < LEAF_BIT as usize
            && b.gpu_bytes()
                .checked_add(NODE_BYTES)
                .is_some_and(|n| n <= limits.max_bytes),
        "BVH node byte/ID budget",
    )?;
    let id = b.nodes.len() as u32;
    b.nodes.push(n);
    Ok(Child {
        id,
        lo: decoded_lo,
        hi: decoded_hi,
        error,
    })
}
