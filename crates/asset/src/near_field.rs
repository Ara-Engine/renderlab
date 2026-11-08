//! Cook-time near-field sky occlusion for the world radiance cache's software-ray path
//! (M7 G1c, docs/m7-g1c.md). No GPU or shading-model dependency.
//!
//! For every vertex of a static mesh's far-field cut (a legal cluster-DAG cut, whose
//! vertices are source vertices on the full-detail surface), the cook traces one exact ray
//! per world-space octahedral direction bin, `reach` long, against every static
//! placement's full-detail triangles, and keeps one blocked bit per bin. At runtime a cell
//! whose query point lies on that cut reads its near field from the three vertex masks
//! (a fixed barycentric vote per direction) and traces software rays against the cut only
//! from `near` on; moving placements are traced over the whole ray. Masks store visibility,
//! never a program's output, and there is no per-triangle grid (docs/m7-p0-claims.md 4.1).
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
    /// World distance at which runtime far-field rays start against static placements.
    pub near: f32,
    /// World length of the cook's exact rays. At least `near`, and longer by about a
    /// triangle, so a query point inside a triangle has every occluder its far rays skip
    /// within reach of the triangle's vertices.
    pub reach: f32,
    /// Octahedral bins per side; a vertex mask holds bins² bits (a multiple of 32).
    pub bins: u32,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        let ok = self.lod_error.is_finite()
            && self.lod_error >= 0.
            && self.near.is_finite()
            && self.near > 0.
            && self.reach.is_finite()
            && self.reach >= self.near
            && [8, 16, 32].contains(&self.bins);
        if ok {
            Ok(())
        } else {
            Err(Error("invalid near-field settings"))
        }
    }
    /// u32 words per vertex mask.
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
    bvh.validate(cluster_bvh::Limits {
        max_bytes: usize::MAX,
    })?;
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

// ---- encoding: explicit little-endian fields, never Rust layout ----
// Header: magic, version, mesh count, placement count, bins, then lod_error, near and reach
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
        s.near.to_bits(),
        s.reach.to_bits(),
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
        near: f32::from_bits(r.u32()?),
        reach: f32::from_bits(r.u32()?),
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
            near: 0.25,
            reach: 0.5,
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
        bad.settings.reach = 0.1;
        assert!(encode(&bad, Limits::default()).is_err());
        assert!(encode(&field, Limits { max_bytes: 64 }).is_err());
    }
}
