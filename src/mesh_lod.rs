//! N1c: cooked clusterLOD hierarchies re-expressed over each mesh's own vertices.
//!
//! meshoptimizer's clusterlod simplifies by collapsing onto existing vertices, so every
//! triangle of a cooked virtual package, at any DAG level, indexes its source primitive's
//! vertex buffer. [`LodPackage::mesh_lod`] maps each page vertex back to its source vertex
//! by exact bits. Original-geometry clusters keep their source primitive IDs; coarser
//! triangles become primitives T..T+L, appended after the T source triangles of the same
//! index buffer. Hit records, receivers, edge repair and programmable shading therefore
//! read LOD triangles through the unchanged (placement, primitive) ABI.
use super::visibility_buffer::{Cluster, ClusterBuilder, NONE};
use super::*;
use crate::receiver_adjacency::MISSING;
use asset::virtual_mesh::{PAGE_BYTES, Page};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

struct PackageCluster {
    owner: u32,
    refined: u32,
    material: u32,
    page: u32,
    offset: u32,
    count: u32,
}
struct PackageMesh {
    groups: Vec<[f32; 5]>,
    clusters: Vec<PackageCluster>,
}

/// A cooked virtual package's LOD metadata: groups, clusters and page records. The
/// embedded materials/textures are skipped, not decoded. [`LodPackage::mesh_lod`] reads
/// and SHA-256 verifies the page files it needs. The package file's own digest is not
/// rehashed (that would read its textures); malformed metadata is rejected by range checks.
pub struct LodPackage {
    meshes: Vec<PackageMesh>,
    pages: Vec<Page>,
    directory: PathBuf,
}

/// One primitive's hierarchy over its source vertex buffer, for [`Renderer::try_attach_lod`].
pub struct MeshLod {
    vertices: usize,
    source_triangles: u32,
    /// Appended triangles' source vertex indices and within-cluster neighbours.
    indices: Vec<u32>,
    neighbours: Vec<u32>,
    clusters: Vec<Cluster>,
    words: Vec<u32>,
    groups: Vec<[f32; 5]>,
    pub stats: LodStats,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LodStats {
    pub groups: usize,
    pub clusters: usize,
    /// Clusters of original geometry that kept their source primitive IDs.
    pub source_clusters: usize,
    pub lod_triangles: usize,
    /// Original-geometry triangles not found in the source mesh, appended instead.
    pub unmatched_triangles: usize,
    pub pages: usize,
    pub read_ms: f64,
    pub build_ms: f64,
}

fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|v| format!("{v:02x}")).collect()
}
fn bits(v: &Vertex) -> [u32; 12] {
    let mut k = [0; 12];
    for (k, f) in k.iter_mut().zip(
        v.position
            .iter()
            .chain(&v.normal)
            .chain(&v.uv)
            .chain(&v.color),
    ) {
        *k = f.to_bits();
    }
    k
}
fn hash(bits: &[u32; 12]) -> u64 {
    bits.iter().fold(0xcbf2_9ce4_8422_2325, |h, &w| {
        (h ^ u64::from(w))
            .wrapping_mul(0x0100_0000_01b3)
            .rotate_left(23)
    })
}
/// Rotation that puts the smallest index first, preserving winding.
fn canonical(t: [u32; 3]) -> [u32; 3] {
    let k = (0..3).min_by_key(|&k| t[k]).unwrap();
    [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
}
struct Cursor<'a>(&'a [u8]);
impl Cursor<'_> {
    fn u32(&mut self) -> Result<u32, Error> {
        let (head, rest) = self
            .0
            .split_first_chunk::<4>()
            .ok_or(Error::InvalidInput("truncated virtual package metadata"))?;
        self.0 = rest;
        Ok(u32::from_le_bytes(*head))
    }
    fn f32(&mut self) -> Result<f32, Error> {
        self.u32().map(f32::from_bits)
    }
    fn skip(&mut self, n: usize) -> Result<(), Error> {
        self.0 = self
            .0
            .get(n..)
            .ok_or(Error::InvalidInput("truncated virtual package metadata"))?;
        Ok(())
    }
}

impl LodPackage {
    /// Reads `manifest`'s package metadata from a `virtual-v*` cache root's parent.
    pub fn open(cache: &Path, manifest: &Path) -> Result<Self, Error> {
        let io = |_| Error::InvalidInput("virtual package read failed");
        let mut m = [0u8; 80];
        File::open(manifest)
            .and_then(|mut f| f.read_exact(&mut m))
            .map_err(io)?;
        let version = u32::from_le_bytes(m[4..8].try_into().unwrap());
        if &m[..4] != b"RVM\0" || !matches!(version, 1 | 2) {
            return Err(Error::InvalidInput("invalid virtual manifest"));
        }
        let root = cache.join(format!("virtual-v{version}"));
        let digest: [u8; 32] = m[40..72].try_into().unwrap();
        let mut file = File::open(root.join("packages").join(hex(&digest))).map_err(io)?;
        let mut header = [0u8; 24];
        file.read_exact(&mut header).map_err(io)?;
        let mut h = Cursor(&header[4..]);
        let (format, meshes, placements, pages, resources) =
            (h.u32()?, h.u32()?, h.u32()?, h.u32()?, h.u32()?);
        if &header[..4] != b"RLV\0" || format != version {
            return Err(Error::InvalidInput(
                "virtual package/manifest version mismatch",
            ));
        }
        file.seek(SeekFrom::Start(24 + u64::from(resources)))
            .map_err(io)?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes).map_err(io)?;
        let mut r = Cursor(&bytes);
        let mut out = vec![];
        for _ in 0..meshes {
            let (groups, clusters) = (r.u32()? as usize, r.u32()? as usize);
            if groups > r.0.len() / 32 || clusters > r.0.len() / 44 {
                return Err(Error::InvalidInput("truncated virtual package metadata"));
            }
            let mut mesh = PackageMesh {
                groups: Vec::with_capacity(groups),
                clusters: Vec::with_capacity(clusters),
            };
            for _ in 0..groups {
                r.skip(4)?;
                mesh.groups
                    .push([r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
                r.skip(8)?;
            }
            for _ in 0..clusters {
                let (owner, refined) = (r.u32()?, r.u32()?);
                r.skip(20)?;
                let c = PackageCluster {
                    owner,
                    refined,
                    material: r.u32()?,
                    page: r.u32()?,
                    offset: r.u32()?,
                    count: r.u32()?,
                };
                if c.owner as usize >= groups
                    || (c.refined != NONE && c.refined as usize >= groups)
                    || c.page >= pages
                    || c.count == 0
                    || !c.count.is_multiple_of(3)
                    || c.count as usize > 3 * visibility_buffer::MAX_CLUSTER
                {
                    return Err(Error::InvalidInput("invalid virtual cluster record"));
                }
                mesh.clusters.push(c);
            }
            out.push(mesh);
        }
        r.skip(68 * placements as usize)?;
        let mut records = Vec::with_capacity(pages as usize);
        for _ in 0..pages {
            let (vertices, indices, payload_bytes) = (r.u32()?, r.u32()?, r.u32()?);
            let tangents = if version == 2 && r.u32()? != 0 {
                return Err(Error::InvalidInput(
                    "tangent virtual pages are not supported for LOD primaries",
                ));
            } else {
                None
            };
            let mut sha256 = [0; 32];
            for chunk in sha256.chunks_exact_mut(4) {
                chunk.copy_from_slice(&r.u32()?.to_le_bytes());
            }
            records.push(Page {
                tangents,
                vertices,
                indices,
                payload_bytes,
                sha256,
            });
        }
        if !r.0.is_empty() {
            return Err(Error::InvalidInput("trailing virtual package bytes"));
        }
        Ok(Self {
            meshes: out,
            pages: records,
            directory: root.join("pages"),
        })
    }
    pub fn meshes(&self) -> usize {
        self.meshes.len()
    }

    /// Rebuilds package mesh `mesh`'s clusters of `material` over `source`, the exact
    /// primitive it was cooked from. Reads and verifies the pages those clusters use.
    pub fn mesh_lod(
        &self,
        mesh: usize,
        material: u32,
        source: &MeshData<'_>,
    ) -> Result<MeshLod, Error> {
        let started = Instant::now();
        let m = self
            .meshes
            .get(mesh)
            .ok_or(Error::InvalidInput("virtual mesh index out of range"))?;
        let clusters: Vec<_> = m
            .clusters
            .iter()
            .filter(|c| c.material == material)
            .collect();
        if clusters.is_empty() {
            return Err(Error::InvalidInput(
                "virtual mesh has no clusters of this material",
            ));
        }
        let mut pages: Vec<u32> = clusters.iter().map(|c| c.page).collect();
        pages.sort_unstable();
        pages.dedup();
        let mut lookup: Vec<(u64, u32)> = source
            .vertices
            .par_iter()
            .enumerate()
            .map(|(i, v)| (hash(&bits(v)), i as u32))
            .collect();
        lookup.par_sort_unstable();
        let find = |v: &Vertex| -> Option<u32> {
            let key = bits(v);
            let h = hash(&key);
            let start = lookup.partition_point(|e| e.0 < h);
            lookup[start..]
                .iter()
                .take_while(|e| e.0 == h)
                .find(|e| bits(&source.vertices[e.1 as usize]) == key)
                .map(|e| e.1)
        };
        // Page-local vertex -> source vertex, and the page's local index array.
        let mapped = pages
            .par_iter()
            .map(|&p| -> Result<(Vec<u32>, Vec<u32>), Error> {
                let record = &self.pages[p as usize];
                let mut bytes = Vec::with_capacity(PAGE_BYTES);
                File::open(self.directory.join(hex(&record.sha256)))
                    .and_then(|f| f.take(PAGE_BYTES as u64 + 1).read_to_end(&mut bytes))
                    .map_err(|_| Error::InvalidInput("virtual page read failed"))?;
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                let page = asset::virtual_mesh::decode_page(record, &bytes, digest)
                    .map_err(|_| Error::InvalidInput("virtual page failed verification"))?;
                let remap = page
                    .vertices
                    .iter()
                    .map(find)
                    .collect::<Option<Vec<_>>>()
                    .ok_or(Error::InvalidInput(
                        "virtual page vertex is not a source vertex",
                    ))?;
                Ok((remap, page.indices))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let read_ms = started.elapsed().as_secs_f64() * 1000.;
        let mut triangles: Vec<([u32; 3], u32)> = source
            .indices
            .par_chunks_exact(3)
            .enumerate()
            .map(|(i, t)| (canonical([t[0], t[1], t[2]]), i as u32))
            .collect();
        triangles.par_sort_unstable();
        let primitive = |t: [u32; 3]| {
            let key = canonical(t);
            triangles
                .binary_search_by(|e| e.0.cmp(&key))
                .ok()
                .map(|i| triangles[i].1)
        };
        let source_triangles = (source.indices.len() / 3) as u32;
        let mut stats = LodStats {
            groups: m.groups.len(),
            pages: pages.len(),
            read_ms,
            ..Default::default()
        };
        let mut builder = ClusterBuilder::new(source.vertices.len());
        let mut indices = vec![];
        let mut owners = vec![];
        for c in &clusters {
            let (remap, page) = &mapped[pages.binary_search(&c.page).unwrap()];
            let span = page
                .get(c.offset as usize..(c.offset + c.count) as usize)
                .ok_or(Error::InvalidInput(
                    "virtual cluster span out of page range",
                ))?;
            let mut local = Vec::with_capacity(span.len() / 3);
            for t in span.chunks_exact(3) {
                let v = |k: usize| remap.get(t[k] as usize).copied();
                let (Some(a), Some(b), Some(c)) = (v(0), v(1), v(2)) else {
                    return Err(Error::InvalidInput("virtual page index out of range"));
                };
                local.push([a, b, c]);
            }
            let oversize =
                || Error::InvalidInput("virtual cluster exceeds 128 vertices or triangles");
            let ids = (c.refined == NONE)
                .then(|| {
                    local
                        .iter()
                        .map(|&t| primitive(t))
                        .collect::<Option<Vec<_>>>()
                })
                .flatten();
            if let Some(ids) = ids {
                // The index buffer's corner order, which cluster positions must follow.
                for id in ids {
                    let i = id as usize * 3;
                    let t = [
                        source.indices[i],
                        source.indices[i + 1],
                        source.indices[i + 2],
                    ];
                    builder.add(t, id).then_some(()).ok_or_else(oversize)?;
                }
                stats.source_clusters += 1;
            } else {
                if c.refined == NONE {
                    stats.unmatched_triangles += local.len();
                }
                for &t in &local {
                    let id = source_triangles + (indices.len() / 3) as u32;
                    indices.extend(t);
                    owners.push(builder.clusters.len() as u32);
                    builder.add(t, id).then_some(()).ok_or_else(oversize)?;
                }
            }
            builder.finish(source.vertices, c.owner, c.refined);
        }
        let neighbours = cluster_neighbours(source, &indices, &owners, source_triangles);
        stats.clusters = builder.clusters.len();
        stats.lod_triangles = indices.len() / 3;
        stats.build_ms = started.elapsed().as_secs_f64() * 1000. - read_ms;
        Ok(MeshLod {
            vertices: source.vertices.len(),
            source_triangles,
            indices,
            neighbours,
            clusters: builder.clusters,
            words: builder.words,
            groups: m.groups.clone(),
            stats,
        })
    }
}

/// Exact-position shared edges between appended triangles of the same cluster, in the
/// receiver adjacency's opposite-vertex order. Edges across clusters depend on the cut
/// and stay MISSING; the repair treats them as open.
fn cluster_neighbours(
    source: &MeshData<'_>,
    indices: &[u32],
    owners: &[u32],
    first: u32,
) -> Vec<u32> {
    let mut used = indices.to_vec();
    used.par_sort_unstable();
    used.dedup();
    let mut positions: Vec<([u32; 3], u32)> = used
        .iter()
        .map(|&i| {
            let p = source.vertices[i as usize].position;
            (p.map(|x| if x == 0. { 0 } else { x.to_bits() }), i)
        })
        .collect();
    positions.par_sort_unstable();
    // Smallest index of each exact position, as the receiver adjacency welds.
    let mut weld = vec![MISSING; source.vertices.len()];
    for run in positions.chunk_by(|a, b| a.0 == b.0) {
        for &(_, i) in run {
            weld[i as usize] = run[0].1;
        }
    }
    let canonical = |v: u32| weld[v as usize];
    let mut records: Vec<(u32, u32, u32, u32)> = vec![];
    for (triangle, t) in indices.chunks_exact(3).enumerate() {
        for edge in 0..3 {
            let a = canonical(t[(edge + 1) % 3]);
            let b = canonical(t[(edge + 2) % 3]);
            if a != b {
                records.push((
                    owners[triangle],
                    a.min(b),
                    a.max(b),
                    (triangle * 3 + edge) as u32,
                ));
            }
        }
    }
    records.par_sort_unstable();
    let mut neighbours = vec![MISSING; indices.len()];
    for run in records.chunk_by(|a, b| a.0 == b.0 && a.1 == b.1 && a.2 == b.2) {
        if let [a, b] = run {
            let opposite = |h: u32| canonical(indices[h as usize]);
            if a.3 / 3 != b.3 / 3 && opposite(a.3) != opposite(b.3) {
                neighbours[a.3 as usize] = first + b.3 / 3;
                neighbours[b.3 as usize] = first + a.3 / 3;
            }
        }
    }
    neighbours
}

impl Renderer {
    /// Appends `lod`'s triangles after `mesh`'s source triangles and makes its hierarchy
    /// the mesh's visibility-buffer clusters. Hit records may then name primitives at or
    /// beyond the source count; forward raster draws, acceleration structures and shadow
    /// casters keep drawing the source triangles. Busy returns `None` and changes nothing.
    pub fn try_attach_lod(&mut self, mesh: &mut Mesh, lod: &MeshLod) -> Result<Option<()>, Error> {
        self.check_fault()?;
        if mesh.clusters.is_none() {
            return Err(Error::InvalidInput(
                "LOD needs a visibility-buffer renderer (VISIBILITY_RASTER_FEATURES)",
            ));
        }
        let source = u64::from(lod.source_triangles) * 12;
        if mesh.skin.is_some()
            || mesh.vertex.size() != lod.vertices as u64 * 48
            || mesh.index_count != lod.source_triangles * 3
        {
            return Err(Error::InvalidInput("LOD was built for a different mesh"));
        }
        let appended = lod.indices.len() as u64 * 4;
        let adjacency = mesh.index.size() == source * 2 + 4;
        let size = if adjacency {
            (source + appended) * 2 + 4
        } else {
            source + appended
        };
        let limits = self.device.limits();
        if size > limits.max_storage_buffer_binding_size || size > limits.max_buffer_size {
            return Err(Error::InvalidInput(
                "LOD index buffer exceeds device binding limits",
            ));
        }
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let scopes = OperationScopes::new(&self.device);
        let blas = if self.hardware_shadows.is_some() {
            wgpu::BufferUsages::BLAS_INPUT
        } else {
            wgpu::BufferUsages::empty()
        };
        let index = Arc::new(self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh indices with appended LOD triangles"),
                size,
                usage: wgpu::BufferUsages::INDEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::STORAGE
                    | blas,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        ));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&mesh.index, 0, &index, 0, source);
        let bytes = |v: &[u32]| v.iter().flat_map(|w| w.to_ne_bytes()).collect::<Vec<_>>();
        if !lod.indices.is_empty() {
            self.queue
                .write_buffer(&index, source, &bytes(&lod.indices));
        }
        if adjacency {
            let base = source + appended;
            encoder.copy_buffer_to_buffer(&mesh.index, source, &index, base, source);
            if !lod.neighbours.is_empty() {
                self.queue
                    .write_buffer(&index, base + source, &bytes(&lod.neighbours));
            }
        }
        let clusters = self.upload_clusters(lod.clusters.clone(), &lod.words, lod.groups.clone());
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        let submitted_at = Instant::now();
        self.queue.submit([encoder.finish()]);
        self.queue.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        self.submissions += 1;
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                view: None,
                revision: self.revision,
                submitted_at,
            },
        );
        self.uploads.geometry +=
            appended * if adjacency { 2 } else { 1 } + lod.words.len() as u64 * 4;
        self.uploads.write_calls += 3;
        mesh.index = index;
        mesh.clusters = Some(clusters);
        self.check_fault()?;
        Ok(Some(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_rotation_keeps_winding() {
        assert_eq!(canonical([5, 2, 9]), [2, 9, 5]);
        assert_eq!(canonical([9, 5, 2]), [2, 9, 5]);
        assert_ne!(canonical([2, 5, 9]), canonical([2, 9, 5]));
    }
    #[test]
    fn neighbours_link_only_shared_edges_inside_one_cluster() {
        let v = |x: f32, y: f32| Vertex {
            position: [x, y, 0.],
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        };
        // Quad 0-1-2-3 split twice; vertex 4 duplicates vertex 2's position (a seam).
        let vertices = [v(0., 0.), v(1., 0.), v(1., 1.), v(0., 1.), v(1., 1.)];
        let source = MeshData {
            vertices: &vertices,
            indices: &[0, 1, 2],
        };
        let indices = [0, 1, 2, 0, 4, 3, 0, 1, 2, 0, 2, 3];
        let n = cluster_neighbours(&source, &indices, &[0, 0, 1, 2], 10);
        // Triangles 0 and 1 share welded edge 0-2 (opposite vertices 1 and 0, 3 respectively).
        assert_eq!(n[1], 11);
        assert_eq!(n[5], 10);
        // Triangles 2 and 3 share edge 0-2 too, but sit in different clusters.
        assert!(n[6..].iter().all(|&x| x == MISSING));
        assert_eq!(n.iter().filter(|&&x| x != MISSING).count(), 2);
    }
}
