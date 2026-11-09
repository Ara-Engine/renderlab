//! Object-space software-ray geometry for the world radiance cache on the non-RT path
//! (M7 G1, G1c). Meshes trace a BVH4 in R0c's cluster-BVH record layout
//! (`renderlab_asset::cluster_bvh`, docs/m7-r0c.md): a full-detail build made at load
//! (`build`), or a cooked near field's far-field cut (`renderlab_asset::near_field`,
//! docs/m7-g1c.md), whose placements also carry per-vertex near-field masks. `layout` and
//! `mesh_words` pack the meshes' records and those masks into one storage buffer behind a
//! flat instance list (a top level rebuilt on the CPU when transforms change,
//! `instance_words`), which shaders/gi_rays_software.wgsl traverses.
use crate::asset::cluster_bvh::Bvh;
use crate::{Error, Mat4};

/// Words before the instance records; shaders/gi_rays_software.wgsl reads count at 0.
pub(crate) const HEADER_WORDS: usize = 16;
/// One instance record; WGSL GI_INSTANCE_WORDS must match. Words 0-11 world-to-object
/// rows, 12-17 world bounds, 18-21 the mesh's array offsets, 22 its root, 23 the
/// instance's near-field mask base (0: none, the instance is traced over whole rays).
pub(crate) const INSTANCE_WORDS: usize = 32;

/// Builds a full-detail one-level BVH4 over a triangle list (eight triangles a leaf), by binned
/// SAH splits (GC1: software rays visit fewer nodes and triangles than over median splits; the
/// same hits, docs/m7-gc1.md).
pub(crate) fn build(positions: &[[f32; 3]], indices: &[u32]) -> Result<Bvh, Error> {
    crate::asset::near_field::build_bvh_sah(positions, indices)
        .map_err(|_| Error::InvalidInput("indirect geometry is not a valid triangle list"))
}

/// One cooked placement's near-field masks (two sides of `words` per BVH position) and the
/// exact transform bits they were cooked under.
pub(crate) struct NearMasks {
    pub model: Mat4,
    pub masks: Vec<u32>,
}

/// One mesh's records in the packed buffer, its object-space bounds, and the near-field
/// masks of its cooked placements (empty for full-detail geometry).
pub(crate) struct Blas {
    pub bvh: Bvh,
    pub root: u32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub near: Vec<NearMasks>,
}
impl Blas {
    /// Level `level` of `bvh` (a zero-error regular build has one).
    pub(crate) fn new(bvh: Bvh, level: usize) -> Result<Self, Error> {
        let root = bvh
            .levels
            .get(level)
            .ok_or(Error::InvalidInput("indirect geometry level"))?
            .root;
        let (lo, hi) = bounds(bvh.positions.iter().map(|p| [p[0], p[1], p[2]]));
        Ok(Self {
            bvh,
            root,
            lo,
            hi,
            near: vec![],
        })
    }
    /// Words this mesh's records take, rounded up to a 16-byte word (P-NRT2): every mesh then
    /// starts on one, so the shader reads nodes, leaves and positions as whole vec4s.
    fn words(&self) -> usize {
        (self.bvh.nodes.len() * 20
            + self.bvh.leaves.len() * 8
            + self.bvh.positions.len() * 4
            + self.bvh.indices.len())
        .next_multiple_of(4)
    }
    /// The base word of the near-field masks cooked under exactly `model`, given this
    /// mesh's mask bases in the packed buffer; 0 when none matches.
    pub(crate) fn near_base(&self, model: &Mat4, bases: &[u32]) -> u32 {
        let bits = |m: &Mat4| m.to_cols_array().map(f32::to_bits);
        self.near
            .iter()
            .zip(bases)
            .find(|(n, _)| bits(&n.model) == bits(model))
            .map_or(0, |(_, &b)| b)
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

/// Word offsets of each mesh's arrays after the instance records, then the base word of
/// each of its near placements' masks, and the total.
pub(crate) fn layout(meshes: &[&Blas], instances: usize) -> (Vec<[u32; 4]>, Vec<Vec<u32>>, usize) {
    let mut at = HEADER_WORDS + instances * INSTANCE_WORDS;
    let offsets = meshes
        .iter()
        .map(|b| {
            let nodes = at;
            let leaves = nodes + b.bvh.nodes.len() * 20;
            let positions = leaves + b.bvh.leaves.len() * 8;
            let indices = positions + b.bvh.positions.len() * 4;
            at += b.words();
            [nodes, leaves, positions, indices].map(|v| v as u32)
        })
        .collect();
    let near = meshes
        .iter()
        .map(|b| {
            b.near
                .iter()
                .map(|n| {
                    let base = at as u32;
                    at += n.masks.len();
                    base
                })
                .collect()
        })
        .collect();
    (offsets, near, at)
}

/// Every mesh's records and near masks at the offsets `layout` gave, as words.
pub(crate) fn mesh_words(
    meshes: &[&Blas],
    offsets: &[[u32; 4]],
    near: &[Vec<u32>],
    total: usize,
) -> Vec<u32> {
    let mut words = vec![0u32; total];
    for ((b, o), bases) in meshes.iter().zip(offsets).zip(near) {
        let mut at = o[0] as usize;
        for n in &b.bvh.nodes {
            let lane = |bytes: [u8; 4]| u32::from_le_bytes(bytes);
            let record = [n.origin.map(f32::to_bits), n.step.map(f32::to_bits)]
                .concat()
                .into_iter()
                .chain(n.children)
                .chain(n.lower.map(lane))
                .chain(n.upper.map(lane))
                .chain(n.errors.map(f32::to_bits));
            for w in record {
                words[at] = w;
                at += 1;
            }
        }
        for l in &b.bvh.leaves {
            let record = [
                l.mesh,
                l.cluster,
                l.page,
                l.source_index_offset,
                l.indices.offset,
                l.indices.count,
                l.error.to_bits(),
                l.material,
            ];
            words[at..at + 8].copy_from_slice(&record);
            at += 8;
        }
        for p in &b.bvh.positions {
            words[at..at + 4].copy_from_slice(&p.map(f32::to_bits));
            at += 4;
        }
        words[at..at + b.bvh.indices.len()].copy_from_slice(&b.bvh.indices);
        for (n, &base) in b.near.iter().zip(bases) {
            words[base as usize..base as usize + n.masks.len()].copy_from_slice(&n.masks);
        }
    }
    words
}

/// The header and instance records: per placement, world-to-object rows, the world
/// bounds of its mesh, that mesh's array offsets and root, and its near-field mask base.
pub(crate) fn instance_words(
    instances: &[(Mat4, &Blas, [u32; 4], u32)],
) -> Result<Vec<u32>, Error> {
    let mut words = vec![0u32; HEADER_WORDS + instances.len() * INSTANCE_WORDS];
    words[0] = instances.len() as u32;
    for (i, (model, blas, offsets, near)) in instances.iter().enumerate() {
        let inverse = model.inverse();
        if !inverse.is_finite() {
            return Err(Error::InvalidInput(
                "indirect geometry transform is singular",
            ));
        }
        let r = HEADER_WORDS + i * INSTANCE_WORDS;
        for row in 0..3 {
            words[r + row * 4..r + row * 4 + 4]
                .copy_from_slice(&inverse.row(row).to_array().map(f32::to_bits));
        }
        // World bounds from the eight transformed corners, rounded outward.
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for c in 0..8 {
            let p = glam::Vec3::from(std::array::from_fn(|a| {
                if c & (1 << a) == 0 {
                    blas.lo[a]
                } else {
                    blas.hi[a]
                }
            }));
            let w = model.transform_point3(p);
            for a in 0..3 {
                lo[a] = lo[a].min(w[a]);
                hi[a] = hi[a].max(w[a]);
            }
        }
        for a in 0..3 {
            let pad = 1e-5 * (1.0 + lo[a].abs().max(hi[a].abs()));
            words[r + 12 + a] = (lo[a] - pad).to_bits();
            words[r + 15 + a] = (hi[a] + pad).to_bits();
        }
        words[r + 18..r + 22].copy_from_slice(offsets);
        words[r + 22] = blas.root;
        words[r + 23] = *near;
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The SAH build (GC1) keeps every triangle in exactly one leaf of at most eight, within
    /// the software traversal's stack depth, on an uneven scene (a dense patch beside a sparse
    /// plane, where SAH and median planes differ).
    #[test]
    fn sah_build_partitions_every_triangle() {
        use crate::asset::cluster_bvh::{EMPTY, LEAF_BIT};
        use crate::asset::near_field::{LEAF_TRIANGLES, MAX_SAH_DEPTH};
        let (mut p, mut i) = grid(40);
        let (q, j) = grid(4);
        let base = p.len() as u32;
        p.extend(
            q.iter()
                .map(|v| [v[0] * 30.0 + 2.0, v[1] + 0.5, v[2] * 30.0]),
        );
        i.extend(j.iter().map(|k| k + base));
        let bvh = build(&p, &i).unwrap();
        let mut seen = vec![0u32; i.len() / 3];
        let mut stack = vec![(bvh.levels[0].root, 0usize)];
        let mut deepest = 0;
        while let Some((child, d)) = stack.pop() {
            if child & LEAF_BIT != 0 {
                let leaf = &bvh.leaves[(child & !LEAF_BIT) as usize];
                assert!(leaf.indices.count as usize <= 3 * LEAF_TRIANGLES);
                let span = leaf.indices.offset as usize
                    ..(leaf.indices.offset + leaf.indices.count) as usize;
                for t in bvh.indices[span].chunks_exact(3) {
                    let at = i.chunks_exact(3).position(|s| s == t).unwrap();
                    seen[at] += 1;
                }
                continue;
            }
            deepest = deepest.max(d + 1);
            for &c in bvh.nodes[child as usize]
                .children
                .iter()
                .filter(|&&c| c != EMPTY)
            {
                stack.push((c, d + 1));
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
        assert!(deepest <= MAX_SAH_DEPTH);
    }

    #[test]
    fn packs_instances_meshes_and_near_masks() {
        let (p, i) = grid(6);
        let mut blas = Blas::new(build(&p, &i).unwrap(), 0).unwrap();
        let model = Mat4::from_translation(glam::Vec3::new(5.0, 0.0, 0.0));
        let masks: Vec<u32> = (0..blas.bvh.positions.len() as u32 * 16).collect();
        blas.near.push(NearMasks {
            model,
            masks: masks.clone(),
        });
        let (offsets, near, total) = layout(&[&blas], 2);
        let words = mesh_words(&[&blas], &offsets, &near, total);
        assert_eq!(words.len(), total);
        assert_eq!(offsets[0][0] as usize, HEADER_WORDS + 2 * INSTANCE_WORDS);
        let base = near[0][0] as usize;
        assert_eq!(&words[base..base + masks.len()], &masks[..]);
        assert_eq!(base + masks.len(), total);
        // Only the instance with the cooked transform's exact bits gets the masks.
        let moved = Mat4::from_translation(glam::Vec3::new(5.0, 1e-3, 0.0));
        assert_eq!(blas.near_base(&model, &near[0]), base as u32);
        assert_eq!(blas.near_base(&moved, &near[0]), 0);
        let head = instance_words(&[
            (model, &blas, offsets[0], base as u32),
            (Mat4::IDENTITY, &blas, offsets[0], 0),
        ])
        .unwrap();
        assert_eq!(head[0], 2);
        assert_eq!(head[HEADER_WORDS + 23], base as u32);
        assert_eq!(head[HEADER_WORDS + INSTANCE_WORDS + 23], 0);
        let lo = f32::from_bits(head[HEADER_WORDS + 12]);
        assert!((lo - 5.0).abs() < 1e-3);
        // World-to-object of a translation by +5 maps x=5 to 0.
        let row0: Vec<f32> = head[HEADER_WORDS..HEADER_WORDS + 4]
            .iter()
            .map(|&w| f32::from_bits(w))
            .collect();
        assert!((row0[0] * 5.0 + row0[3]).abs() < 1e-6);
        assert!(instance_words(&[(Mat4::ZERO, &blas, offsets[0], 0)]).is_err());
        assert!(build(&[[0.0; 3]], &[0, 0]).is_err());
    }
}
