//! Object-space software-ray geometry for the world radiance cache on the non-RT path
//! (M7 G1). Regular meshes build a full-detail BVH4 in R0c's cluster-BVH record layout
//! (`renderlab_asset::cluster_bvh`, docs/m7-r0c.md), so the one GPU traversal in
//! shaders/gi_rays_software.wgsl also reads R0c's cooked LOD sidecars. `pack` lays out
//! a scene's instances (a flat top level, rebuilt when transforms change) and the
//! meshes' records in one storage buffer.
use crate::asset::cluster_bvh::{Bvh, EMPTY, LEAF_BIT, Leaf, Level, Node};
use crate::asset::virtual_mesh::Span;
use crate::{Error, Mat4};

/// Triangles per leaf.
const LEAF_TRIANGLES: usize = 8;
/// Words before the instance records; shaders/gi_rays_software.wgsl reads count at 0.
pub(crate) const HEADER_WORDS: usize = 16;
/// One instance record; WGSL GI_INSTANCE_WORDS must match.
pub(crate) const INSTANCE_WORDS: usize = 32;

/// Builds a one-level, zero-error BVH over `indices` (triangle list) by deterministic
/// median splits on the longest centroid axis, ties broken by triangle order. Child
/// bounds are quantized outward so both fused and unfused f32 decodes enclose them.
pub(crate) fn build(positions: &[[f32; 3]], indices: &[u32]) -> Result<Bvh, Error> {
    if indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err(Error::InvalidInput(
            "indirect geometry needs a nonempty triangle list",
        ));
    }
    if indices.iter().any(|&i| i as usize >= positions.len())
        || positions.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(Error::InvalidInput(
            "indirect geometry has an invalid index or position",
        ));
    }
    let triangles: Vec<usize> = (0..indices.len() / 3).collect();
    let centroid = |t: usize| -> [f32; 3] {
        std::array::from_fn(|a| {
            (0..3)
                .map(|k| positions[indices[t * 3 + k] as usize][a])
                .sum::<f32>()
                / 3.0
        })
    };
    let centroids: Vec<[f32; 3]> = triangles.iter().map(|&t| centroid(t)).collect();
    let mut builder = Builder {
        positions,
        indices,
        centroids: &centroids,
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
    bvh.validate(Default::default())
        .map_err(|_| Error::InvalidInput("indirect geometry BVH failed validation"))?;
    Ok(bvh)
}

struct Builder<'a> {
    positions: &'a [[f32; 3]],
    indices: &'a [u32],
    centroids: &'a [[f32; 3]],
    bvh: Bvh,
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
    fn split(&self, mut g: Vec<usize>) -> (Vec<usize>, Vec<usize>) {
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
        for (slot, (&child, (a, b))) in children.iter().zip(&boxes).enumerate() {
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
        let id = self.bvh.nodes.len() as u32;
        self.bvh.nodes.push(node);
        id
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

/// One mesh's records in the packed buffer and its object-space bounds.
pub(crate) struct Blas {
    pub bvh: Bvh,
    pub root: u32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}
impl Blas {
    /// The finest level of `bvh` for mesh `mesh` (a zero-error regular build has one).
    pub(crate) fn new(bvh: Bvh, level: usize) -> Result<Self, Error> {
        let root = bvh
            .levels
            .get(level)
            .ok_or(Error::InvalidInput("indirect geometry level"))?
            .root;
        let (lo, hi) = bounds(bvh.positions.iter().map(|p| [p[0], p[1], p[2]]));
        Ok(Self { bvh, root, lo, hi })
    }
    fn words(&self) -> usize {
        self.bvh.nodes.len() * 20
            + self.bvh.leaves.len() * 8
            + self.bvh.positions.len() * 4
            + self.bvh.indices.len()
    }
}

/// Word offsets of each mesh's arrays after the instance records.
pub(crate) fn layout(meshes: &[&Blas], instances: usize) -> (Vec<[u32; 4]>, usize) {
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
    (offsets, at)
}

/// Every mesh's records at `offsets` (from `layout`), as little-endian words.
pub(crate) fn mesh_words(meshes: &[&Blas], offsets: &[[u32; 4]], total: usize) -> Vec<u32> {
    let mut words = vec![0u32; total];
    for (b, o) in meshes.iter().zip(offsets) {
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
    }
    words
}

/// The header and instance records: per placement, world-to-object rows, the world
/// bounds of its mesh, and that mesh's array offsets and root.
pub(crate) fn instance_words(instances: &[(Mat4, &Blas, [u32; 4])]) -> Result<Vec<u32>, Error> {
    let mut words = vec![0u32; HEADER_WORDS + instances.len() * INSTANCE_WORDS];
    words[0] = instances.len() as u32;
    for (i, (model, blas, offsets)) in instances.iter().enumerate() {
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

    #[test]
    fn builds_valid_deterministic_bvh4() {
        let (p, i) = grid(24);
        let a = build(&p, &i).unwrap();
        let b = build(&p, &i).unwrap();
        assert_eq!(a, b);
        assert!(
            a.nodes
                .iter()
                .all(|n| n.children.iter().filter(|&&c| c != EMPTY).count() >= 2)
        );
        assert_eq!(a.indices.len(), i.len());
        // A single small mesh is one leaf root.
        let small = build(&p, &i[..6]).unwrap();
        assert!(small.nodes.is_empty() && small.levels[0].root & LEAF_BIT != 0);
    }

    #[test]
    fn rejects_bad_geometry() {
        assert!(build(&[[0.0; 3]], &[0, 0]).is_err());
        assert!(build(&[[0.0; 3]], &[0, 0, 1]).is_err());
        assert!(build(&[[f32::NAN; 3]; 3], &[0, 1, 2]).is_err());
    }

    #[test]
    fn packs_instances_and_meshes() {
        let (p, i) = grid(6);
        let blas = Blas::new(build(&p, &i).unwrap(), 0).unwrap();
        let (offsets, total) = layout(&[&blas], 2);
        let words = mesh_words(&[&blas], &offsets, total);
        assert_eq!(words.len(), total);
        assert_eq!(offsets[0][0] as usize, HEADER_WORDS + 2 * INSTANCE_WORDS);
        let model = Mat4::from_translation(glam::Vec3::new(5.0, 0.0, 0.0));
        let head = instance_words(&[
            (model, &blas, offsets[0]),
            (Mat4::IDENTITY, &blas, offsets[0]),
        ])
        .unwrap();
        assert_eq!(head[0], 2);
        let lo = f32::from_bits(head[HEADER_WORDS + 12]);
        assert!((lo - 5.0).abs() < 1e-3);
        // World-to-object of a translation by +5 maps x=5 to 0.
        let row0: Vec<f32> = head[HEADER_WORDS..HEADER_WORDS + 4]
            .iter()
            .map(|&w| f32::from_bits(w))
            .collect();
        assert!((row0[0] * 5.0 + row0[3]).abs() < 1e-6);
        assert!(instance_words(&[(Mat4::ZERO, &blas, offsets[0])]).is_err());
    }
}
