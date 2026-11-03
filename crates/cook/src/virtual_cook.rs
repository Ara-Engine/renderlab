//! Cooker-only meshoptimizer clusterLOD and immutable page packing.
use crate::{Error, Result, cache::hash, ensure};
use renderlab_asset::{Asset, Vertex, virtual_mesh::*};
use std::collections::{BTreeMap, BTreeSet};

pub const NORMAL_ALGORITHM: &[u8] = b"renderlab-virtual/2/meshopt-0.7.0/meshoptimizer-1.3/13-attributes/locked-tangent-seams/subset-original-frames/pages64k/rla2/mikktspace-0.3.0";
pub const ALGORITHM: &[u8]=b"renderlab-virtual/1/meshopt-0.7.0/meshoptimizer-1.3/regular-attributes/domain1e6/locked-subset-seams/pages64k/rla1";
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub max_triangles: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self { max_triangles: 128 }
    }
}
impl Settings {
    pub fn validate(self) -> Result<()> {
        ensure(
            matches!(self.max_triangles, 64 | 128 | 256),
            "cluster triangles must be 64, 128 or 256",
        )
    }
    pub fn bytes(self) -> [u8; 4] {
        self.max_triangles.to_le_bytes()
    }
}
#[derive(Clone, Debug, Default)]
pub struct Statistics {
    pub source_vertices: usize,
    pub source_indices: usize,
    pub packed_vertices: usize,
    pub packed_indices: usize,
    pub unique_referenced_source_vertices: usize,
    pub source_boundary_vertices_locked: usize,
    pub copied_callback_index_bytes: usize,
    pub peak_single_primitive_attribute_bytes: usize,
}
pub struct Cooked {
    pub package: Package,
    pub pages: Vec<Vec<u8>>,
    pub statistics: Statistics,
}
struct RawCluster {
    owner: GroupId,
    refined: Option<GroupId>,
    bounds: Bounds,
    primitive: usize,
    indices: Vec<u32>,
}
fn bounds(b: meshopt::ClodBounds) -> Bounds {
    Bounds {
        center: b.center,
        radius: b.radius,
        error: b.error,
    }
}
fn position_key(v: &Vertex) -> [u32; 3] {
    v.position.map(|f| if f == 0. { 0 } else { f.to_bits() })
}

struct PageBuilder {
    tangents: Option<renderlab_asset::TangentStream>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    remap: BTreeMap<(usize, u32), u32>,
}
impl PageBuilder {
    fn new(basis: Option<renderlab_asset::TangentBasis>) -> Self {
        Self {
            tangents: basis.map(|basis| renderlab_asset::TangentStream {
                basis,
                values: vec![],
            }),
            vertices: vec![],
            indices: vec![],
            remap: BTreeMap::new(),
        }
    }
    fn size_after(&self, c: &RawCluster) -> usize {
        let missing: BTreeSet<_> = c
            .indices
            .iter()
            .filter(|&&v| !self.remap.contains_key(&(c.primitive, v)))
            .collect();
        (self.vertices.len() + missing.len()) * (if self.tangents.is_some() { 64 } else { 48 })
            + (self.indices.len() + c.indices.len()) * 4
    }
    fn append(&mut self, c: &RawCluster, source: &renderlab_asset::Mesh) -> Span {
        let span = Span {
            offset: self.indices.len() as u32,
            count: c.indices.len() as u32,
        };
        for &i in &c.indices {
            let next = self.vertices.len() as u32;
            let local = *self.remap.entry((c.primitive, i)).or_insert_with(|| {
                self.vertices
                    .push(source.primitives[c.primitive].vertices[i as usize]);
                if let Some(t) = &mut self.tangents {
                    t.values.push(
                        source.primitives[c.primitive]
                            .tangents
                            .as_ref()
                            .unwrap()
                            .values[i as usize],
                    );
                }
                next
            });
            self.indices.push(local);
        }
        span
    }
    fn finish(self) -> (Page, Vec<u8>) {
        let mut bytes = Vec::with_capacity(PAGE_BYTES);
        for v in &self.vertices {
            for f in v
                .position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .chain(&v.color)
            {
                bytes.extend(f.to_bits().to_le_bytes());
            }
        }
        if let Some(t) = &self.tangents {
            for v in &t.values {
                for f in v {
                    bytes.extend(f.to_le_bytes());
                }
            }
        }
        for i in &self.indices {
            bytes.extend(i.to_le_bytes());
        }
        let length = bytes.len();
        bytes.resize(PAGE_BYTES, 0);
        (
            Page {
                tangents: self.tangents.as_ref().map(|t| t.basis),
                vertices: self.vertices.len() as u32,
                indices: self.indices.len() as u32,
                payload_bytes: length as u32,
                sha256: hash(&bytes),
            },
            bytes,
        )
    }
}

pub fn prepare(asset: &Asset, settings: Settings, limits: VirtualLimits) -> Result<Cooked> {
    settings.validate()?;
    asset.validate(limits.prepared)?;
    // Bound upstream f32 intermediate arithmetic independently of M3's wider finite domain.
    ensure(
        asset
            .meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .flat_map(|p| &p.vertices)
            .all(|v| {
                v.position
                    .iter()
                    .chain(&v.normal)
                    .chain(&v.uv)
                    .all(|f| f.abs() <= 1_000_000.)
            }),
        "virtual cooker position/normal/UV magnitude exceeds 1e6",
    )?;
    let mut package = Package {
        meshes: vec![],
        pages: vec![],
        placements: asset.placements.clone(),
        resources: Asset {
            materials: asset.materials.clone(),
            textures: asset.textures.clone(),
            ..Asset::default()
        },
    };
    let mut page_bytes = vec![];
    let mut statistics = Statistics::default();
    let mut aggregate_groups = 0usize;
    let mut aggregate_clusters = 0usize;
    for source in &asset.meshes {
        // Shared-position boundaries between independently cooked material/primitive subsets are locked.
        // The same primitive's UV/normal seams remain under regular simplification and protect mask.
        let mut first_primitive = BTreeMap::new();
        let mut shared = BTreeSet::new();
        for (pi, p) in source.primitives.iter().enumerate() {
            for v in &p.vertices {
                let k = position_key(v);
                if let Some(previous) = first_primitive.insert(k, pi)
                    && previous != pi
                {
                    shared.insert(k);
                }
            }
        }
        let mut groups = vec![];
        let mut raw = vec![];
        for (pi, p) in source.primitives.iter().enumerate() {
            statistics.source_vertices += p.vertices.len();
            statistics.source_indices += p.indices.len();
            let positions: Vec<_> = p.vertices.iter().map(|v| v.position).collect();
            let attribute_count = if p.tangents.is_some() { 13 } else { 9 };
            let attribute_scalars = p
                .vertices
                .len()
                .checked_mul(attribute_count)
                .ok_or_else(|| Error("virtual attribute count overflow".into()))?;
            let preparation_bytes = attribute_scalars
                .checked_mul(4)
                .and_then(|n| n.checked_add(p.vertices.len().checked_mul(12)?))
                .ok_or_else(|| Error("virtual attribute byte overflow".into()))?;
            ensure(
                preparation_bytes <= limits.prepared.max_decoded_bytes,
                "virtual attribute preparation byte limit",
            )?;
            let mut attributes = Vec::with_capacity(attribute_scalars);
            for (i, v) in p.vertices.iter().enumerate() {
                attributes.extend(v.normal.into_iter().chain(v.uv).chain(v.color));
                if let Some(t) = &p.tangents {
                    attributes.extend(t.values[i]);
                }
            }
            // Subset simplification retains source frames. Lock coincident frame seams explicitly.
            let mut frames = BTreeMap::new();
            let mut tangent_seams = BTreeSet::new();
            if let Some(t) = &p.tangents {
                for (v, t) in p.vertices.iter().zip(&t.values) {
                    let key = position_key(v);
                    let frame = v
                        .normal
                        .into_iter()
                        .chain(v.uv)
                        .chain(*t)
                        .map(|x| if x == 0. { 0 } else { x.to_bits() })
                        .collect::<Vec<_>>();
                    if frames
                        .insert(key, frame.clone())
                        .is_some_and(|old| old != frame)
                    {
                        tangent_seams.insert(key);
                    }
                }
            }
            statistics.peak_single_primitive_attribute_bytes = statistics
                .peak_single_primitive_attribute_bytes
                .max(attributes.len() * 4 + positions.len() * 12);
            let locks: Vec<_> = p
                .vertices
                .iter()
                .map(|v| {
                    if shared.contains(&position_key(v)) || tangent_seams.contains(&position_key(v))
                    {
                        statistics.source_boundary_vertices_locked += 1;
                        meshopt::SimplifyVertex::Lock
                    } else {
                        meshopt::SimplifyVertex::empty()
                    }
                })
                .collect();
            let adapter =
                meshopt::VertexDataAdapter::new(meshopt::typed_to_bytes(&positions), 12, 0)
                    .map_err(|e| Error(e.to_string()))?;
            let weights = vec![1.; attribute_count];
            let mesh = meshopt::ClodMesh {
                indices: &p.indices,
                vertices: &adapter,
                vertex_attributes: &attributes,
                vertex_attributes_stride: attribute_count * 4,
                vertex_lock: Some(&locks),
                attribute_weights: &weights,
                attribute_protect_mask: (1 << attribute_count) - 1,
            };
            let mut config = meshopt::clod_default_config(settings.max_triangles as usize);
            config.simplify_permissive = false;
            config.simplify_fallback_permissive = false;
            config.simplify_fallback_sloppy = false;
            config.simplify_error_clamped = false;
            config.simplify_error_edge_limit = 0.;
            config.simplify_dilate_borders = false;
            let mut overflow = false;
            meshopt::clod_build(config, &mesh, |group, clusters| {
                let id = groups.len();
                let index_bytes = clusters.iter().map(|c| c.indices.len() * 4).sum::<usize>();
                if id >= i32::MAX as usize
                    || group.depth < 0
                    || raw
                        .len()
                        .checked_add(clusters.len())
                        .is_none_or(|n| n > u32::MAX as usize)
                    || aggregate_groups
                        .checked_add(groups.len())
                        .and_then(|n| n.checked_add(1))
                        .is_none_or(|n| n > limits.max_groups)
                    || aggregate_clusters
                        .checked_add(raw.len())
                        .and_then(|n| n.checked_add(clusters.len()))
                        .is_none_or(|n| n > limits.max_clusters)
                    || statistics
                        .copied_callback_index_bytes
                        .checked_add(index_bytes)
                        .is_none_or(|n| n > limits.max_geometry_bytes)
                {
                    overflow = true;
                    return id as i32;
                }
                groups.push(Group {
                    depth: group.depth as u32,
                    simplified: bounds(group.simplified),
                    clusters: Span {
                        offset: raw.len() as u32,
                        count: clusters.len() as u32,
                    },
                });
                for c in clusters {
                    // The wrapper borrows callback arrays. Retain owned indices and scalar metadata only.
                    raw.push(RawCluster {
                        owner: GroupId(id as u32),
                        refined: (c.refined >= 0).then_some(GroupId(c.refined as u32)),
                        bounds: bounds(c.bounds),
                        primitive: pi,
                        indices: c.indices.to_vec(),
                    });
                }
                statistics.copied_callback_index_bytes += index_bytes;
                id as i32
            });
            ensure(!overflow, "clusterLOD output exceeds bounded cooker limits")?;
        }
        aggregate_groups += groups.len();
        aggregate_clusters += raw.len();
        let mut clusters: Vec<_> = raw
            .iter()
            .map(|c| Cluster {
                owner: c.owner,
                refined: c.refined,
                bounds: c.bounds,
                material: source.primitives[c.primitive].material,
                page: PageId(0),
                indices: Span {
                    offset: 0,
                    count: 0,
                },
            })
            .collect();
        let unique: BTreeSet<_> = raw
            .iter()
            .flat_map(|c| c.indices.iter().map(|&i| (c.primitive, i)))
            .collect();
        statistics.unique_referenced_source_vertices += unique.len();
        let mut order: Vec<_> = (0..raw.len()).collect();
        order.sort_by_key(|&i| {
            (
                groups[raw[i].owner.index()].simplified.error != f32::MAX,
                std::cmp::Reverse(raw[i].owner.0),
                i,
            )
        });
        let mut builder = PageBuilder::new(None);
        for i in order {
            let c = &raw[i];
            let basis = source.primitives[c.primitive]
                .tangents
                .as_ref()
                .map(|t| t.basis);
            if !builder.indices.is_empty()
                && (builder.size_after(c) > PAGE_BYTES
                    || basis != builder.tangents.as_ref().map(|t| t.basis))
            {
                ensure(
                    !builder.indices.is_empty(),
                    "single cluster exceeds page capacity",
                )?;
                let (page, bytes) = builder.finish();
                package.pages.push(page);
                page_bytes.push(bytes);
                builder = PageBuilder::new(basis);
            }
            if builder.indices.is_empty() {
                builder = PageBuilder::new(basis);
            }
            ensure(
                builder.size_after(c) <= PAGE_BYTES,
                "single tangent cluster exceeds page capacity",
            )?;
            ensure(
                package.pages.len() < limits.max_pages
                    && package.pages.len() < u32::MAX as usize
                    && package
                        .pages
                        .len()
                        .checked_add(1)
                        .and_then(|n| n.checked_mul(PAGE_BYTES))
                        .is_some_and(|n| n <= limits.max_geometry_bytes),
                "virtual page budget",
            )?;
            clusters[i].page = PageId(package.pages.len() as u32);
            clusters[i].indices = builder.append(c, source);
        }
        if !builder.indices.is_empty() {
            let (page, bytes) = builder.finish();
            package.pages.push(page);
            page_bytes.push(bytes);
        }
        package.meshes.push(VirtualMesh { groups, clusters });
    }
    package.validate(limits)?;
    statistics.packed_vertices = package.pages.iter().map(|p| p.vertices as usize).sum();
    statistics.packed_indices = package.pages.iter().map(|p| p.indices as usize).sum();
    Ok(Cooked {
        package,
        pages: page_bytes,
        statistics,
    })
}
