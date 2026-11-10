//! N1c: primary visibility from a clusterLOD cut. Each frame selects one cut of every
//! mesh's cluster hierarchy at a pixel error bound, culls it against the frustum and a
//! two-phase HZB, rasterizes small clusters in software and the rest in hardware into one
//! 64-bit (depth, cluster, triangle) buffer, then reconstructs exact pixel-centre
//! receivers into the primary hit ABI. Meshes without an attached hierarchy are one
//! level of original-geometry clusters. No lighting-model decisions, acceleration
//! structures or ray-query instructions.
// PC1: only the device type changes; pipeline creation uses the shared cache.
use super::*;
use crate::pipeline_compilation::PipelineDevice;
use primary_ray_gpu::Buffer;
use rayon::prelude::*;

/// Largest cluster: the software raster's workgroup and 7-bit triangle IDs.
pub(crate) const MAX_CLUSTER: usize = 128;
/// No refined group: the cluster is original geometry.
pub(crate) const NONE: u32 = u32::MAX;
/// `Cluster::counts` flag: per-triangle primitive IDs follow in the cluster data.
const TABLE: u32 = 1 << 16;
const CLUSTER_BYTES: u64 = 64;
const GROUP_BYTES: u64 = 32;
/// visibility_buffer_cull.wgsl's Args (16 working words, then the last frame's copy
/// kept by visibility_buffer_reset.wgsl) and Rejects header.
const ARGS_BYTES: u64 = 128;
const REJECT_HEADER: u64 = 400;
/// Default projected-error bound for the cut, in pixels (docs/non-rt-n1c.md).
pub(crate) const DEFAULT_LOD_PIXELS: f32 = 0.35;
/// Default lift of LOD receivers, in units of their simplification error.
pub(crate) const DEFAULT_LOD_LIFT: f32 = 1.;
/// Clusters whose projected box exceeds this many pixels use the hardware raster.
pub(crate) const DEFAULT_SOFTWARE_EXTENT: f32 = 32.;

/// One cluster of at most 128 triangles over at most 128 of its mesh's vertices.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cluster {
    pub bounds: Aabb,
    pub owner: u32,
    pub refined: u32,
    /// Word offsets into the mesh's cluster data: three position words per vertex.
    pub vertices: u32,
    pub triangles: u32,
    /// First primitive ID, or with `TABLE` the word offset of per-triangle IDs.
    pub primitives: u32,
    /// Triangles | vertices << 8 | TABLE.
    pub counts: u32,
}

/// A mesh's cluster records and LOD groups (sphere centre, radius, accumulated error;
/// `f32::MAX` error never coarsens), plus the GPU words the records index: local vertex
/// positions (exact object-space f32 copies), packed u8 local triangles and source
/// primitive tables.
pub(crate) struct Clusters {
    pub clusters: Vec<Cluster>,
    pub groups: Vec<[f32; 5]>,
    pub data: Arc<Owned<wgpu::Buffer>>,
}

/// Accumulates clusters and their data words. `add` fills the open cluster; `finish`
/// closes it with its LOD links.
pub(crate) struct ClusterBuilder {
    // SK1: exact source vertex for each duplicated cluster position.
    pub source_map: Vec<[u32; 2]>,
    pub clusters: Vec<Cluster>,
    pub words: Vec<u32>,
    mark: Vec<u32>,
    local: Vec<u8>,
    vertices: Vec<u32>,
    triangles: Vec<u32>,
    primitives: Vec<u32>,
}
impl ClusterBuilder {
    pub fn new(vertex_count: usize) -> Self {
        Self {
            source_map: vec![],
            clusters: vec![],
            words: vec![],
            mark: vec![0; vertex_count],
            local: vec![0; vertex_count],
            vertices: Vec::with_capacity(MAX_CLUSTER),
            triangles: Vec::with_capacity(MAX_CLUSTER),
            primitives: Vec::with_capacity(MAX_CLUSTER),
        }
    }
    /// Adds a triangle unless the open cluster would exceed 128 triangles or vertices.
    pub fn add(&mut self, triangle: [u32; 3], primitive: u32) -> bool {
        let serial = self.clusters.len() as u32 + 1;
        let new = (0..3)
            .filter(|&k| {
                let v = triangle[k];
                self.mark[v as usize] != serial && !triangle[..k].contains(&v)
            })
            .count();
        if self.triangles.len() == MAX_CLUSTER || self.vertices.len() + new > MAX_CLUSTER {
            return false;
        }
        let mut packed = 0;
        for (k, &v) in triangle.iter().enumerate() {
            let v = v as usize;
            if self.mark[v] != serial {
                self.mark[v] = serial;
                self.local[v] = self.vertices.len() as u8;
                self.vertices.push(v as u32);
            }
            packed |= u32::from(self.local[v]) << (8 * k);
        }
        self.triangles.push(packed);
        self.primitives.push(primitive);
        true
    }
    /// Closes the open cluster. Contiguous primitive IDs store only the first.
    pub fn finish(&mut self, vertices: &[Vertex], owner: u32, refined: u32) {
        if self.triangles.is_empty() {
            return;
        }
        let contiguous = self.primitives.windows(2).all(|w| w[1] == w[0] + 1);
        let vertex_offset = self.words.len() as u32;
        for &v in &self.vertices {
            self.source_map.push([self.words.len() as u32, v]);
            self.words
                .extend(vertices[v as usize].position.map(f32::to_bits));
        }
        let triangle_offset = self.words.len() as u32;
        self.words.extend_from_slice(&self.triangles);
        let (primitives, table) = if contiguous {
            (self.primitives[0], 0)
        } else {
            let offset = self.words.len() as u32;
            self.words.extend_from_slice(&self.primitives);
            (offset, TABLE)
        };
        self.clusters.push(Cluster {
            bounds: Aabb::from_positions(
                self.vertices.iter().map(|&v| vertices[v as usize].position),
            )
            .unwrap(),
            owner,
            refined,
            vertices: vertex_offset,
            triangles: triangle_offset,
            primitives,
            counts: self.triangles.len() as u32 | (self.vertices.len() as u32) << 8 | table,
        });
        self.vertices.clear();
        self.triangles.clear();
        self.primitives.clear();
    }
}

/// Original-geometry clusters from Morton-sorted triangle centroids: one terminal
/// group, so every cluster is always in the cut.
pub(crate) fn source_clusters(data: &MeshData<'_>) -> ClusterBuilder {
    let bounds = Aabb::from_positions(data.vertices.iter().map(|v| v.position)).unwrap();
    let lo = Vec3::from_array(bounds.min);
    let scale = Vec3::splat(1023.) / (Vec3::from_array(bounds.max) - lo).max(Vec3::splat(1e-20));
    let spread = |v: u32| {
        let mut x = v & 1023;
        x = (x | (x << 16)) & 0x030000ff;
        x = (x | (x << 8)) & 0x0300f00f;
        x = (x | (x << 4)) & 0x030c30c3;
        (x | (x << 2)) & 0x09249249
    };
    let mut order: Vec<u64> = data
        .indices
        .chunks_exact(3)
        .enumerate()
        .map(|(id, ix)| {
            let centre = ix
                .iter()
                .map(|&i| Vec3::from_array(data.vertices[i as usize].position))
                .sum::<Vec3>()
                / 3.;
            let q = ((centre - lo) * scale)
                .clamp(Vec3::ZERO, Vec3::splat(1023.))
                .as_uvec3();
            let morton = spread(q.x) | (spread(q.y) << 1) | (spread(q.z) << 2);
            (u64::from(morton) << 32) | id as u64
        })
        .collect();
    order.par_sort_unstable();
    let mut builder = ClusterBuilder::new(data.vertices.len());
    for primitive in order.into_iter().map(|v| v as u32) {
        let p = primitive as usize * 3;
        let triangle = [data.indices[p], data.indices[p + 1], data.indices[p + 2]];
        if !builder.add(triangle, primitive) {
            builder.finish(data.vertices, 0, NONE);
            assert!(builder.add(triangle, primitive));
        }
    }
    builder.finish(data.vertices, 0, NONE);
    builder
}

impl Renderer {
    /// Uploads cluster data words and wraps the records for a mesh.
    pub(crate) fn upload_clusters(
        &self,
        clusters: Vec<Cluster>,
        words: &[u32],
        groups: Vec<[f32; 5]>,
    ) -> Clusters {
        let bytes = (words.len().max(1) * 4) as u64;
        let data = Arc::new(self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("N1c cluster vertices, triangles and primitive tables"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        ));
        if !words.is_empty() {
            let mut staging = self
                .queue
                .write_buffer_with(&data, 0, std::num::NonZeroU64::new(bytes).unwrap())
                .unwrap();
            let (chunks, _) = staging.slice(..).into_chunks::<4>();
            chunks.write_iter(words.iter().map(|w| w.to_ne_bytes()));
        }
        Clusters {
            clusters,
            groups,
            data,
        }
    }
}

pub(super) fn supported(device: &PipelineDevice) -> bool {
    let limits = device.limits();
    device
        .features()
        .contains(crate::VISIBILITY_FEATURES | crate::VISIBILITY_RASTER_FEATURES)
        && crate::visibility_limits(limits.clone()) == limits
}

/// The same repair arithmetic as the hardware two-pass check. The slow path for
/// ambiguous/missing adjacency searches exact shared endpoints in the original mesh;
/// this also supports upload policies that cannot fit the optional adjacency suffix.
pub(super) fn repair_source() -> String {
    let s = include_str!("../shaders/primary_trace.wgsl");
    let s =
        &s[s.find("struct PrimaryTrace {").unwrap()..s.find("// The pixel's camera ray").unwrap()];
    let start = s.find("// The full near-to-far").unwrap();
    let end = s.find("fn primary_repair(").unwrap();
    let mut s = format!("{}{}", &s[..start], &s[end..]);
    s = s.replace("    var query:ray_query<vertex_return>;", "");
    let start = s.find("            if neighbour==0xffffffffu {").unwrap();
    let end = start
        + s[start..]
            .find("            var direct_pending=true;")
            .unwrap();
    s.replace_range(start..end, "            let range=primary_repair_range(batch,trace.record.y,edge);\n            if neighbour==0xffffffffu && range.x==range.y {continue;}\n            var search=range.x;\n");
    let start = s
        .find("                    if !rayQueryProceed(&query)")
        .unwrap();
    let end = start
        + s[start..]
            .find("                }\n                var common_vertices")
            .unwrap();
    s.replace_range(start..end, "                    let words=arrayLength(&primary_indices[batch].data);\n                    let end=select(select(words/3u,(words-1u)/6u,words%3u==1u),range.y,range.z!=0u);\n                    if search>=end {break;}candidate_primitive=select(search,primary_repair_scan[search],range.z!=0u);search+=1u;\n                    if candidate_primitive==trace.record.y {continue;}\n                    v=primary_triangle_vertices(batch,candidate_primitive);\n                    var shared_count=0u;for(var j=0u;j<3u;j+=1u) {shared_count+=u32(all(v[j]==e0)||all(v[j]==e1));}\n                    if shared_count!=2u || !primary_probe_intersects(v,slot,start,probe_direction,reach) {continue;}\n                    if !visible {visible=primary_edge_dot(primary_geometric(v[0],v[1],v[2],slot),direction)<0.0;}\n");
    assert!(!s.contains("rayQuery") && !s.contains("ray_query"));
    s
}
/// The resolve's fallback-scan policy (`VisibilityBuffer::new` writes it): a
/// missing-neighbour halfedge's scan range. With z = 1, x..y index this policy's list of the
/// halfedge's candidates: every triangle with two corners at the edge's endpoint positions,
/// ascending, a superset of what the scan accepts. Otherwise x..y are primitives: y past the
/// mesh for a whole-mesh scan (meshes without lists, LOD triangles), or x == y for none.
/// Visiting only listed candidates leaves the repair unchanged.
const REPAIR_POLICY: &str =
    "@group(0) @binding(30) var<storage,read> primary_repair_scan:array<u32>;
fn primary_repair_range(batch:u32,primitive:u32,edge:u32)->vec3<u32> {
    let s=primary_repair_scan[batch];
    if s==0u {return vec3<u32>(0u);}
    let whole=vec3<u32>(0u,0xffffffffu,0u);
    if s==0xffffffffu {return whole;}
    let h=primitive*3u+edge;
    if h>=primary_repair_scan[s-1u] {return whole;}
    return vec3<u32>(primary_repair_scan[s+h],primary_repair_scan[s+h+1u],1u);
}";
/// Share the original ray-free visibility composition across native backends.
pub(crate) fn resolve_source(wgsl: &str) -> Result<String, Error> {
    let head = primary_ray_gpu::non_rt_heads(wgsl)?;
    Ok(format!(
        "{}\n{REPAIR_POLICY}\n{}\n{}",
        head[0],
        repair_source(),
        include_str!("../shaders/visibility_buffer_resolve.wgsl")
    ))
}
/// The shared declarations, the per-draw array size, then one stage.
fn stage_source(stage: &str) -> String {
    format!(
        "{}\nconst PRIMARY_BATCHES:u32={}u;\n{stage}",
        include_str!("../shaders/visibility_buffer.wgsl"),
        crate::TRACED_DRAWS
    )
}

/// Counters a pass leaves in its arguments buffer.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PassCounts {
    pub software_clusters: u32,
    pub hardware_clusters: u32,
    pub cut_clusters: u32,
    pub cut_triangles: u32,
    pub software_triangles: u32,
    pub hardware_triangles: u32,
}

pub(super) struct VisibilityBuffer {
    size: wgpu::Extent3d,
    lift: f32,
    key: Vec<(wgpu::Buffer, usize)>,
    matrix: Mat4,
    frames: u64,
    count: u32,
    _clusters: Arc<Buffer>,
    _groups: Arc<Buffer>,
    configs: [Buffer; 2],
    _lists: [Buffer; 2],
    args: [Buffer; 2],
    rejects: Buffer,
    _repair_scan: Buffer,
    _image: Buffer,
    _absent: Buffer,
    _visibility: Buffer,
    depth: Depth,
    _hzb: Depth,
    _hzb_absent: Depth,
    cull: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
    reset: wgpu::ComputePipeline,
    software: wgpu::ComputePipeline,
    hardware: wgpu::RenderPipeline,
    base: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
    resolve: [wgpu::ComputePipeline; 2],
    cull_groups: [wgpu::BindGroup; 2],
    software_groups: [wgpu::BindGroup; 2],
    hardware_groups: [wgpu::BindGroup; 2],
    hzb_groups: Vec<wgpu::BindGroup>,
    resolve_group: wgpu::BindGroup,
    data_group: wgpu::BindGroup,
    // Binding resources are retained by wgpu command buffers across replacements.
    hit: wgpu::Texture,
    world: wgpu::Texture,
}
fn layout(
    device: &PipelineDevice,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("N1c ABI"),
        entries,
    })
}
fn buffer_entry(
    binding: u32,
    uniform: bool,
    write: bool,
    stages: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stages,
        count: None,
        ty: wgpu::BindingType::Buffer {
            ty: if uniform {
                wgpu::BufferBindingType::Uniform
            } else {
                wgpu::BufferBindingType::Storage { read_only: !write }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    }
}
/// One read-only storage buffer per draw slot: the meshes' cluster data.
pub(super) fn data_entry(binding: u32, stages: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        count: std::num::NonZeroU32::new(crate::TRACED_DRAWS),
        ..buffer_entry(binding, false, false, stages)
    }
}
fn tex_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        count: None,
        ty,
    }
}
fn sampled(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    tex_entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}
fn image(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    tex_entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}
fn target(
    r: &Renderer,
    label: &str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
    mips: u32,
    usage: wgpu::TextureUsages,
) -> Depth {
    let texture = r.ledger.own(
        r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = r.ledger.own(
        texture.create_view(&Default::default()),
        ResourceKind::TextureView,
    );
    let bytes = u64::from(size.width)
        * u64::from(size.height)
        * u64::from(format.block_copy_size(None).unwrap_or(4))
        * if mips > 1 { 2 } else { 1 };
    Depth {
        texture,
        view,
        _allocation: r.ledger.allocate(AllocationKind::Target, bytes),
    }
}
fn bind(
    device: &PipelineDevice,
    layout: &wgpu::BindGroupLayout,
    entries: &[(u32, wgpu::BindingResource<'_>)],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("N1c resources"),
        layout,
        entries: &entries
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: resource.clone(),
            })
            .collect::<Vec<_>>(),
    })
}
fn words(values: impl IntoIterator<Item = u32>) -> Vec<u8> {
    values.into_iter().flat_map(u32::to_ne_bytes).collect()
}
/// Every draw's clusters once per placement, in traced placement order (16 words each,
/// visibility_buffer.wgsl's `Cluster`), and the draws' LOD groups (8 words each), uploaded.
/// N1c's primaries and N8c's shadow page raster both cull this table.
pub(super) struct ClusterInstances {
    pub count: u32,
    pub clusters: Arc<Buffer>,
    pub groups: Arc<Buffer>,
}
/// Only weak references: primary and shadow consumers own their shared immutable
/// tables. Dropping both consumers releases the generation without a cache purge.
#[derive(Default)]
pub(super) struct ClusterInstanceCache(std::sync::Mutex<Option<CachedClusterInstances>>);
struct CachedClusterInstances {
    key: Vec<(std::sync::Weak<Owned<wgpu::Buffer>>, usize)>,
    count: u32,
    clusters: std::sync::Weak<Buffer>,
    groups: std::sync::Weak<Buffer>,
}
impl ClusterInstances {
    pub fn new(r: &Renderer, draws: &[PreparedDraw<'_>]) -> Result<Self, Error> {
        let mut cache = r.cluster_instances.0.lock().unwrap();
        if let Some(cached) = cache.as_ref()
            && cached.key.len() == draws.len()
            && cached.key.iter().zip(draws).all(|((data, count), d)| {
                d.mesh.clusters.as_ref().is_some_and(|c| {
                    std::ptr::eq(data.as_ptr(), Arc::as_ptr(&c.data)) && *count == d.instances.len()
                })
            })
            && let (Some(clusters), Some(groups)) =
                (cached.clusters.upgrade(), cached.groups.upgrade())
        {
            return Ok(Self {
                count: cached.count,
                clusters,
                groups,
            });
        }
        let mut records = vec![];
        let mut groups = vec![];
        let mut placement = 0u32;
        for d in draws {
            let mesh = d.mesh.clusters.as_ref().ok_or(Error::InvalidInput(
                "visibility buffer mesh uploaded without cluster data",
            ))?;
            let base = (groups.len() / 8) as u32;
            for g in &mesh.groups {
                groups.extend(g.map(f32::to_bits));
                groups.extend([0; 3]);
            }
            for _ in 0..d.instances.len() {
                for c in &mesh.clusters {
                    let bounds = if d.mesh.skin.is_some() {
                        d.mesh.bounds
                    } else {
                        c.bounds
                    };
                    let refined = if c.refined == NONE {
                        NONE
                    } else {
                        c.refined + base
                    };
                    records.extend(bounds.min.map(f32::to_bits));
                    records.push(c.vertices);
                    records.extend(bounds.max.map(f32::to_bits));
                    records.push(c.triangles);
                    records.extend([
                        c.owner + base,
                        refined,
                        c.primitives,
                        c.counts,
                        placement,
                        0,
                        0,
                        0,
                    ]);
                }
                placement += 1;
            }
        }
        let count = records.len() / 16;
        if count >= 1 << 25 {
            return Err(Error::InvalidInput(
                "visibility buffer supports at most 2^25 cluster instances",
            ));
        }
        let limit = r.device.limits().max_storage_buffer_binding_size;
        let bytes = count.max(1) as u64 * CLUSTER_BYTES;
        if bytes > limit {
            return Err(Error::InvalidInput(
                "visibility clusters exceed storage binding limit",
            ));
        }
        let storage = wgpu::BufferUsages::STORAGE;
        let clusters = Buffer::new(r, "N1c cluster instances", bytes, storage);
        if !records.is_empty() {
            r.queue
                .write_buffer(&clusters.value, 0, &words(records.iter().copied()));
        }
        let group_buffer = Buffer::new(
            r,
            "N1c LOD groups",
            (groups.len().max(8) / 8) as u64 * GROUP_BYTES,
            storage,
        );
        if !groups.is_empty() {
            r.queue
                .write_buffer(&group_buffer.value, 0, &words(groups.iter().copied()));
        }
        let clusters = Arc::new(clusters);
        let groups = Arc::new(group_buffer);
        *cache = Some(CachedClusterInstances {
            key: draws
                .iter()
                .map(|d| {
                    (
                        Arc::downgrade(&d.mesh.clusters.as_ref().unwrap().data),
                        d.instances.len(),
                    )
                })
                .collect(),
            count: count as u32,
            clusters: Arc::downgrade(&clusters),
            groups: Arc::downgrade(&groups),
        });
        Ok(Self {
            count: count as u32,
            clusters,
            groups,
        })
    }
}

// SK1: update only the deforming draws' records in the table shared by primary
// visibility and shadow casters. Rebuilding consumers/pipelines for each pose would
// both be expensive and leave already-cached consumers holding stale bounds.
pub(super) fn refresh_skinned_bounds(r: &Renderer, draws: &[PreparedDraw<'_>]) -> UploadBytes {
    let mut uploads = UploadBytes::default();
    if !draws.iter().any(|d| d.mesh.skin.is_some()) {
        return uploads;
    }
    let cache = r.cluster_instances.0.lock().unwrap();
    let Some(cache) = cache.as_ref() else {
        return uploads;
    };
    if cache.key.len() != draws.len()
        || !cache.key.iter().zip(draws).all(|((data, count), d)| {
            d.mesh.clusters.as_ref().is_some_and(|c| {
                std::ptr::eq(data.as_ptr(), Arc::as_ptr(&c.data)) && *count == d.instances.len()
            })
        })
    {
        return uploads;
    }
    let Some(buffer) = cache.clusters.upgrade() else {
        return uploads;
    };
    let (mut record, mut group, mut placement) = (0u64, 0u32, 0u32);
    for d in draws {
        let mesh = d.mesh.clusters.as_ref().unwrap();
        if d.mesh.skin.is_some() && !d.instances.is_empty() {
            let b = d.mesh.bounds;
            let mut data = Vec::with_capacity(mesh.clusters.len() * d.instances.len() * 16);
            for instance in 0..d.instances.len() {
                for c in &mesh.clusters {
                    data.extend(b.min.map(f32::to_bits));
                    data.push(c.vertices);
                    data.extend(b.max.map(f32::to_bits));
                    data.push(c.triangles);
                    data.extend([
                        c.owner + group,
                        if c.refined == NONE {
                            NONE
                        } else {
                            c.refined + group
                        },
                        c.primitives,
                        c.counts,
                        placement + instance as u32,
                        0,
                        0,
                        0,
                    ]);
                }
            }
            let bytes = words(data.into_iter());
            r.queue
                .write_buffer(&buffer.value, record * CLUSTER_BYTES, &bytes);
            uploads.skinning += bytes.len() as u64;
            uploads.write_calls += 1;
        }
        record += mesh.clusters.len() as u64 * d.instances.len() as u64;
        group += mesh.groups.len() as u32;
        placement += d.instances.len() as u32;
    }
    uploads
}
/// One read-only storage binding per draw slot: each draw's cluster data words, padded
/// with `absent` (`data_entry`'s layout at binding 8).
pub(super) fn cluster_data_group(
    r: &Renderer,
    layout: &wgpu::BindGroupLayout,
    draws: &[PreparedDraw<'_>],
    absent: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let data: Vec<_> = draws
        .iter()
        .map(|d| {
            d.mesh
                .clusters
                .as_ref()
                .unwrap()
                .data
                .as_entire_buffer_binding()
        })
        .chain(std::iter::repeat(absent.as_entire_buffer_binding()))
        .take(crate::TRACED_DRAWS as usize)
        .collect();
    bind(
        &r.device,
        layout,
        &[(8, wgpu::BindingResource::BufferArray(&data))],
    )
}
impl VisibilityBuffer {
    fn new(r: &Renderer, draws: &[PreparedDraw<'_>], size: wgpu::Extent3d) -> Result<Self, Error> {
        use wgpu::{
            BindingResource as R, ShaderStages as S, TextureFormat as F, TextureUsages as U,
        };
        let state = r.primary.as_ref().unwrap();
        let t = state.targets.as_ref().unwrap();
        let fresh = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let instances = ClusterInstances::new(r, draws)?;
        let count = instances.count as usize;
        let (clusters, group_buffer) = (instances.clusters, instances.groups);
        let storage = wgpu::BufferUsages::STORAGE;
        let limit = r.device.limits().max_storage_buffer_binding_size;
        let configs = std::array::from_fn(|_| {
            Buffer::new(r, "N1c view and pass", 176, wgpu::BufferUsages::UNIFORM)
        });
        let list = |label, n: u64| Buffer::new(r, label, count.max(1) as u64 * 4 * n, storage);
        let lists = std::array::from_fn(|_| list("N1c software then hardware clusters", 2));
        let args = std::array::from_fn(|_| {
            Buffer::new(
                r,
                "N1c indirect software dispatch, hardware draw and counts",
                ARGS_BYTES,
                storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
            )
        });
        let rejects = Buffer::new(
            r,
            "N1c pass-1 HZB rejects and pyramid dispatches",
            REJECT_HEADER + count.max(1) as u64 * 4,
            storage | wgpu::BufferUsages::INDIRECT,
        );
        // Per draw (resolve_source's primary_repair_range): 0 never scans, u32::MAX scans the
        // whole mesh, else 1 + the word offset of the mesh's halfedge count H, then H + 1
        // absolute offsets of its candidate lists, then the lists
        // (receiver_adjacency::repair_candidates), once per mesh.
        let mut policy = vec![0u32; crate::TRACED_DRAWS as usize];
        let mut placed: Vec<(*const u32, u32)> = vec![];
        for (i, d) in draws.iter().take(policy.len()).enumerate() {
            policy[i] = match (d.mesh.repair_scan, &d.mesh.repair_edges) {
                (false, _) => 0,
                (true, None) => u32::MAX,
                (true, Some(lists)) => match placed.iter().find(|(p, _)| *p == lists.as_ptr()) {
                    Some(&(_, at)) => at + 1,
                    None => {
                        let at = policy.len() as u32;
                        let halfedges = lists[0] as usize;
                        let base = at + 2 + halfedges as u32;
                        policy.push(lists[0]);
                        policy.extend(lists[1..halfedges + 2].iter().map(|o| base + o));
                        policy.extend_from_slice(&lists[halfedges + 2..]);
                        placed.push((lists.as_ptr(), at));
                        at + 1
                    }
                },
            };
        }
        let repair_scan = Buffer::new(
            r,
            "N1c ambiguous-edge fallback policy",
            policy.len() as u64 * 4,
            storage,
        );
        r.queue
            .write_buffer(&repair_scan.value, 0, &words(policy.iter().copied()));
        let image_size = Buffer::new(r, "N1c image size", 16, wgpu::BufferUsages::UNIFORM);
        r.queue.write_buffer(
            &image_size.value,
            0,
            &words([size.width, size.height, 0, 0]),
        );
        let pixels = u64::from(size.width) * u64::from(size.height) * 8;
        if pixels > limit {
            return Err(Error::InvalidInput(
                "visibility buffer exceeds storage binding limit",
            ));
        }
        let visibility = Buffer::new(r, "N1c 64-bit depth and cluster IDs", pixels, storage);
        let depth = target(
            r,
            "N1c hardware-cluster depth",
            size,
            F::Depth32Float,
            1,
            U::RENDER_ATTACHMENT,
        );
        // Level 0 texels cover 2x2 pixels. Power-of-two sizes halve exactly; texels
        // beyond the image stay 0.
        let extent = wgpu::Extent3d {
            width: size.width.div_ceil(2).next_power_of_two(),
            height: size.height.div_ceil(2).next_power_of_two(),
            depth_or_array_layers: 1,
        };
        let levels = 32 - extent.width.max(extent.height).leading_zeros();
        if levels > 16 {
            return Err(Error::InvalidInput(
                "visibility buffer HZB exceeds 16 levels",
            ));
        }
        let hzb = target(
            r,
            "N1c conservative min depth",
            extent,
            F::R32Float,
            levels,
            U::STORAGE_BINDING | U::TEXTURE_BINDING,
        );
        let hzb_absent = target(
            r,
            "N1c unused HZB base input",
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            F::R32Float,
            1,
            U::TEXTURE_BINDING,
        );
        let raster_stages = S::VERTEX | S::FRAGMENT;
        let cull_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(1, false, false, S::COMPUTE),
                buffer_entry(2, false, false, S::COMPUTE),
                buffer_entry(3, false, true, S::COMPUTE),
                buffer_entry(5, false, true, S::COMPUTE),
                buffer_entry(6, false, true, S::COMPUTE),
                sampled(7, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        );
        let software_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(1, false, false, S::COMPUTE),
                buffer_entry(3, false, false, S::COMPUTE),
                buffer_entry(5, false, false, S::COMPUTE),
                buffer_entry(8, false, true, S::COMPUTE),
            ],
        );
        let hardware_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, raster_stages),
                buffer_entry(1, false, false, S::VERTEX),
                buffer_entry(3, false, false, S::VERTEX),
                buffer_entry(8, false, true, S::FRAGMENT),
            ],
        );
        let data_layout = layout(&r.device, &[data_entry(8, S::COMPUTE | S::VERTEX)]);
        let hzb_layout = layout(
            &r.device,
            &[
                buffer_entry(0, false, false, S::COMPUTE),
                image(1, F::R32Float),
                sampled(2, wgpu::TextureSampleType::Float { filterable: false }),
                buffer_entry(3, true, false, S::COMPUTE),
            ],
        );
        let resolve_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(10, true, false, S::COMPUTE),
                buffer_entry(30, false, false, S::COMPUTE),
                buffer_entry(29, false, true, S::COMPUTE),
                buffer_entry(31, false, false, S::COMPUTE),
                buffer_entry(33, false, false, S::COMPUTE),
                image(13, F::Rgba32Uint),
                image(16, F::Rgba32Float),
                image(17, F::Rgba32Float),
                image(20, F::R32Uint),
                image(23, F::R32Float),
            ],
        );
        let pipeline_layout = |groups: &[Option<&wgpu::BindGroupLayout>]| {
            r.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("N1c pipeline"),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                })
        };
        let compute = |source: &str,
                       entry: &str,
                       groups: &[Option<&wgpu::BindGroupLayout>],
                       constants: &[(&str, f64)]| {
            let module = r.device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(entry),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            r.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout(groups)),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    cache: None,
                })
        };
        let cull_source = stage_source(include_str!("../shaders/visibility_buffer_cull.wgsl"));
        let groups_cull = [Some(&cull_layout), Some(state.array_layout())];
        let cull = compute(&cull_source, "cull", &groups_cull, &[]);
        let finish = compute(&cull_source, "finish", &groups_cull, &[]);
        let reset = compute(
            &stage_source(include_str!("../shaders/visibility_buffer_reset.wgsl")),
            "reset",
            &groups_cull,
            &[],
        );
        let software_pipeline = compute(
            &stage_source(include_str!("../shaders/visibility_buffer_sw.wgsl")),
            "raster",
            &[
                Some(&software_layout),
                Some(state.array_layout()),
                Some(&data_layout),
            ],
            &[],
        );
        let shader = r.device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("N1c hardware ID raster"),
            source: wgpu::ShaderSource::Wgsl(
                stage_source(include_str!("../shaders/visibility_buffer_raster.wgsl")).into(),
            ),
        });
        let hardware_pipeline = r
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("N1c hardware ID raster"),
                layout: Some(&pipeline_layout(&[
                    Some(&hardware_layout),
                    Some(state.array_layout()),
                    Some(&data_layout),
                ])),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                primitive: Default::default(),
                // GreaterEqual: equal-depth fragments still reach the 64-bit max, so
                // ties never depend on the culler's nondeterministic draw order.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: F::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let hzb_source = include_str!("../shaders/visibility_buffer_hzb.wgsl");
        let base = compute(hzb_source, "base", &[Some(&hzb_layout)], &[]);
        let reduce = compute(hzb_source, "reduce", &[Some(&hzb_layout)], &[]);
        let resolve_source = resolve_source(&r.source)?;
        let resolve = [0.0, 1.0].map(|v| {
            compute(
                &resolve_source,
                "resolve",
                &[
                    Some(&resolve_layout),
                    Some(state.array_layout()),
                    Some(&data_layout),
                ],
                &[("WRITE_RECEIVERS", v), ("LOD_LIFT", f64::from(r.lod_lift))],
            )
        });
        let state = r.primary.as_ref().unwrap();
        let absent = Buffer::new(r, "N1c absent cluster data", 16, storage);
        let cull_groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &cull_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (2, group_buffer.value.as_entire_binding()),
                    (3, lists[i].value.as_entire_binding()),
                    (5, args[i].value.as_entire_binding()),
                    (6, rejects.value.as_entire_binding()),
                    (7, R::TextureView(&hzb.view)),
                ],
            )
        });
        let software_groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &software_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (3, lists[i].value.as_entire_binding()),
                    (5, args[i].value.as_entire_binding()),
                    (8, visibility.value.as_entire_binding()),
                ],
            )
        });
        let hardware_groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &hardware_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (3, lists[i].value.as_entire_binding()),
                    (8, visibility.value.as_entire_binding()),
                ],
            )
        });
        let data_group = cluster_data_group(r, &data_layout, draws, &absent.value);
        let views: Vec<_> = (0..levels)
            .map(|i| {
                hzb.texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        // The unused source of the base pass must not alias its storage output.
        let hzb_groups = (0..levels)
            .map(|i| {
                bind(
                    &r.device,
                    &hzb_layout,
                    &[
                        (0, visibility.value.as_entire_binding()),
                        (1, R::TextureView(&views[i as usize])),
                        (
                            2,
                            R::TextureView(if i == 0 {
                                &hzb_absent.view
                            } else {
                                &views[i as usize - 1]
                            }),
                        ),
                        (3, image_size.value.as_entire_binding()),
                    ],
                )
            })
            .collect();
        let resolve_group = bind(
            &r.device,
            &resolve_layout,
            &[
                (0, r.uniform.as_entire_binding()),
                (10, state.frame_buffer().as_entire_binding()),
                (30, repair_scan.value.as_entire_binding()),
                (29, visibility.value.as_entire_binding()),
                (31, clusters.value.as_entire_binding()),
                (33, group_buffer.value.as_entire_binding()),
                (13, R::TextureView(&t.hit.view)),
                (16, R::TextureView(&fresh.world.view)),
                (17, R::TextureView(&fresh.normal.view)),
                (20, R::TextureView(&t.tiles.view)),
                (23, R::TextureView(&t.epsilon.view)),
            ],
        );
        Ok(Self {
            size,
            lift: r.lod_lift,
            key: draws
                .iter()
                .map(|d| ((**d.mesh.index).clone(), d.instances.len()))
                .collect(),
            matrix: Mat4::ZERO,
            frames: 0,
            count: count as u32,
            _clusters: clusters,
            _groups: group_buffer,
            configs,
            _lists: lists,
            args,
            rejects,
            _repair_scan: repair_scan,
            _image: image_size,
            _absent: absent,
            _visibility: visibility,
            depth,
            _hzb: hzb,
            _hzb_absent: hzb_absent,
            cull,
            finish,
            reset,
            software: software_pipeline,
            hardware: hardware_pipeline,
            base,
            reduce,
            resolve,
            cull_groups,
            software_groups,
            hardware_groups,
            hzb_groups,
            resolve_group,
            data_group,
            hit: (*t.hit.texture).clone(),
            world: (*fresh.world.texture).clone(),
        })
    }
    /// Pass `i`'s pyramid; its dispatches are zero when nothing reads it.
    fn pyramid(&self, encoder: &mut wgpu::CommandEncoder, i: usize, stamp: Stamp<'_>) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N1c HZB"),
            timestamp_writes: stamp.compute(),
        });
        for (level, group) in self.hzb_groups.iter().enumerate() {
            pass.set_pipeline(if level == 0 { &self.base } else { &self.reduce });
            pass.set_bind_group(0, group, &[]);
            let word = 4 + i * 48 + level * 3;
            pass.dispatch_workgroups_indirect(&self.rejects.value, word as u64 * 4);
        }
    }
    /// Both passes' counters. Test and diagnostic use only: waits for the GPU.
    #[cfg(test)]
    pub(super) fn test_counts(&self, r: &Renderer) -> [PassCounts; 2] {
        let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("N1c culling counters"),
            size: ARGS_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut e = r.device.create_command_encoder(&Default::default());
        for i in 0..2 {
            // The copy the last frame's reset kept.
            let half = ARGS_BYTES / 2;
            e.copy_buffer_to_buffer(&self.args[i].value, half, &buffer, i as u64 * half, half);
        }
        let done = Arc::new(OnceLock::new());
        let signal = done.clone();
        e.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            signal.set(result).unwrap();
        });
        r.queue.submit([e.finish()]);
        r.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        done.get().unwrap().as_ref().unwrap();
        let bytes = buffer.get_mapped_range(..).unwrap();
        let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        std::array::from_fn(|pass| {
            let w = |i| word(pass * 16 + i);
            PassCounts {
                software_clusters: w(3),
                hardware_clusters: w(5),
                cut_clusters: w(8),
                cut_triangles: w(9),
                software_triangles: w(10),
                hardware_triangles: w(11),
            }
        })
    }
    #[cfg(test)]
    pub(super) fn cluster_instances(&self) -> u32 {
        self.count
    }
}

/// Optional timestamp pair indices for one encoded stage.
#[derive(Clone, Copy)]
pub(crate) struct Stamp<'a>(Option<(&'a wgpu::QuerySet, u32)>);
impl<'a> Stamp<'a> {
    fn compute(self) -> Option<wgpu::ComputePassTimestampWrites<'a>> {
        self.0
            .map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            })
    }
    fn render(self) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
        self.0
            .map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            })
    }
}
/// Diagnostic stage stamps: per pass cull, software, hardware, HZB; then resolve.
#[cfg(test)]
pub(crate) const STAGE_QUERIES: u32 = 18;

#[cfg(test)]
mod tests {
    use super::*;
    fn vertex(position: [f32; 3]) -> Vertex {
        Vertex {
            position,
            normal: [0., 1., 0.],
            uv: [0.; 2],
            color: [1.; 4],
        }
    }
    #[test]
    fn source_clusters_preserve_each_triangle_within_vertex_and_triangle_caps() {
        // A strip with shared vertices and a fan that forces the vertex cap.
        let mut vertices: Vec<_> = (0..300u32)
            .map(|i| vertex([i as f32 * 0.01, (i % 2) as f32, 0.]))
            .collect();
        let mut indices: Vec<u32> = (0..298).flat_map(|i| [i, i + 1, i + 2]).collect();
        let hub = vertices.len() as u32;
        vertices.push(vertex([0., 0., 1.]));
        for i in 0..299 {
            indices.extend([hub, i, i + 1]);
        }
        let data = MeshData {
            vertices: &vertices,
            indices: &indices,
        };
        let b = source_clusters(&data);
        let mut seen = vec![false; indices.len() / 3];
        for c in &b.clusters {
            let triangles = (c.counts & 255) as usize;
            let count = ((c.counts >> 8) & 255) as usize;
            assert!(triangles <= MAX_CLUSTER && count <= MAX_CLUSTER && triangles > 0);
            let local = &b.words[c.vertices as usize..c.vertices as usize + 3 * count];
            for k in 0..triangles {
                let packed = b.words[c.triangles as usize + k];
                let primitive = if c.counts & TABLE != 0 {
                    b.words[c.primitives as usize + k]
                } else {
                    c.primitives + k as u32
                } as usize;
                assert!(!std::mem::replace(&mut seen[primitive], true));
                for corner in 0..3 {
                    let l = (packed >> (8 * corner) & 255) as usize;
                    let p = vertices[indices[primitive * 3 + corner] as usize].position;
                    assert_eq!(&local[3 * l..3 * l + 3], &p.map(f32::to_bits));
                    assert!((0..3).all(|a| p[a] >= c.bounds.min[a] && p[a] <= c.bounds.max[a]));
                }
            }
        }
        assert!(seen.into_iter().all(|s| s));
    }
    #[test]
    fn visibility_shaders_have_no_ray_queries_and_validate() {
        let mut sources = vec![
            stage_source(include_str!("../shaders/visibility_buffer_cull.wgsl")),
            stage_source(include_str!("../shaders/visibility_buffer_reset.wgsl")),
            stage_source(include_str!("../shaders/visibility_buffer_sw.wgsl")),
            stage_source(include_str!("../shaders/visibility_buffer_raster.wgsl")),
            include_str!("../shaders/visibility_buffer_hzb.wgsl").to_owned(),
            resolve_source(include_str!("../shaders/scene.wgsl")).unwrap(),
        ];
        let (kernels, heads) =
            primary_ray_gpu::non_rt_sources(include_str!("../shaders/scene.wgsl")).unwrap();
        let temporal = primary_ray_gpu::film_accumulation::shading_head(&heads[1]).unwrap();
        sources.push(
            primary_ray_gpu::film_accumulation::compose_shading(
                &temporal,
                crate::STANDARD_PROGRAM,
                false,
            )
            .source,
        );
        sources.extend(kernels);
        for head in heads {
            sources.push(
                crate::program::compose(
                    &head,
                    crate::STANDARD_PROGRAM,
                    crate::program::Stage::Traced,
                    false,
                )
                .source,
            );
        }
        for source in sources {
            assert!(
                !source.contains("ray_query")
                    && !source.contains("rayQuery")
                    && !source.contains("acceleration_structure")
            );
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
    }
}
impl Renderer {
    /// Projected-error bound of the visibility buffer's cluster cut, in pixels.
    /// Negative selects original geometry only. Meshes without an attached hierarchy
    /// always draw their original geometry.
    pub fn set_lod_error_pixels(&mut self, pixels: f32) {
        self.lod_pixels = pixels;
    }
    pub fn lod_error_pixels(&self) -> f32 {
        self.lod_pixels
    }
    #[cfg(test)]
    pub(super) fn set_software_raster_extent(&mut self, pixels: f32) {
        self.software_extent = pixels;
    }
    /// Scale of a coarse triangle's simplification error by which its shadow receiver
    /// moves to the viewed side; 0 keeps receivers on the drawn LOD triangle.
    pub fn set_lod_receiver_lift(&mut self, scale: f32) {
        self.lod_lift = scale;
    }
    pub(super) fn prepare_visibility_buffer(
        &mut self,
        draws: &[PreparedDraw<'_>],
        camera: &Camera,
        size: wgpu::Extent3d,
    ) -> Result<(), Error> {
        // Unshadowed raster-primary frames still need receiver targets bound to resolve.
        if self
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .is_none_or(|t| t.world.texture.size() != size)
        {
            self.ensure_fresh_targets(size, false);
        }
        let primary = self.primary.as_ref().unwrap().targets.as_ref().unwrap();
        let world = &self
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .world
            .texture;
        let key: Vec<_> = draws
            .iter()
            .map(|d| ((**d.mesh.index).clone(), d.instances.len()))
            .collect();
        if self.visibility_buffer.as_ref().is_none_or(|v| {
            v.size != size
                || v.key != key
                || v.lift != self.lod_lift
                || v.hit != *primary.hit.texture
                || v.world != **world
        }) {
            let scopes = OperationScopes::new(&self.device);
            let next = VisibilityBuffer::new(self, draws, size)?;
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
            self.visibility_buffer = Some(next);
        }
        let (lod, extent) = (self.lod_pixels, self.software_extent);
        let v = self.visibility_buffer.as_mut().unwrap();
        let matrix = camera.matrix(size.width, size.height);
        let eye = camera.view().inverse().w_axis.truncate();
        let scale = size.height as f32 * 0.5 / (camera.fov_y() * 0.5).tan();
        let previous = if v.frames == 0 { matrix } else { v.matrix };
        for i in 0..2 {
            let mut bytes = matrix_bytes(matrix).to_vec();
            bytes.extend_from_slice(&matrix_bytes(previous));
            for value in [eye.x, eye.y, eye.z, scale, lod, camera.near(), extent, 0.] {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            for word in [
                v.count,
                u32::from(v.frames != 0),
                i as u32,
                size.width | (size.height << 16),
            ] {
                bytes.extend_from_slice(&word.to_ne_bytes());
            }
            self.queue.write_buffer(&v.configs[i].value, 0, &bytes);
        }
        v.matrix = matrix;
        v.frames += 1;
        Ok(())
    }
    pub(super) fn encode_visibility_buffer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        soft: bool,
        stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        self.encode_visibility_stages(encoder, soft, stamps, None);
    }
    /// `stages` optionally receives STAGE_QUERIES diagnostic timestamps.
    pub(super) fn encode_visibility_stages(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        soft: bool,
        stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        stages: Option<&wgpu::QuerySet>,
    ) {
        let v = self.visibility_buffer.as_ref().unwrap();
        let arrays = self.primary.as_ref().unwrap().virtual_geometry().unwrap().1;
        let stage = |i: u32| Stamp(stages.map(|q| (q, i * 2)));
        // The first cull records the primary span's start and the resolve pass its end.
        // No clears: the previous frame's resolve zeroed its hits and its reset zeroed
        // the counters; new buffers start zeroed.
        let begin = stamps.as_ref().map(|s| wgpu::ComputePassTimestampWrites {
            query_set: s.query_set,
            beginning_of_pass_write_index: s.beginning_of_pass_write_index,
            end_of_pass_write_index: None,
        });
        for i in 0..2 {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("N1c LOD cut and cluster cull"),
                    timestamp_writes: if i == 0 && begin.is_some() {
                        begin.clone()
                    } else {
                        stage(i as u32 * 4).compute()
                    },
                });
                pass.set_pipeline(&v.cull);
                pass.set_bind_group(0, &v.cull_groups[i], &[]);
                pass.set_bind_group(1, arrays, &[]);
                if i == 0 {
                    let groups = v.count.div_ceil(64);
                    pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535).max(1), 1);
                } else {
                    // Only pass 1's rejects, dispatched by pass 1's finish.
                    pass.dispatch_workgroups_indirect(&v.args[0].value, 48);
                }
                pass.set_pipeline(&v.finish);
                pass.dispatch_workgroups(1, 1, 1);
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("N1c software raster"),
                    timestamp_writes: stage(i as u32 * 4 + 1).compute(),
                });
                pass.set_pipeline(&v.software);
                pass.set_bind_group(0, &v.software_groups[i], &[]);
                pass.set_bind_group(1, arrays, &[]);
                pass.set_bind_group(2, &v.data_group, &[]);
                pass.dispatch_workgroups_indirect(&v.args[i].value, 0);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("N1c hardware raster"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &v.depth.view,
                        depth_ops: Some(wgpu::Operations {
                            load: if i == 0 {
                                wgpu::LoadOp::Clear(0.0)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: stage(i as u32 * 4 + 2).render(),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&v.hardware);
                pass.set_bind_group(0, &v.hardware_groups[i], &[]);
                pass.set_bind_group(1, arrays, &[]);
                pass.set_bind_group(2, &v.data_group, &[]);
                pass.draw_indirect(&v.args[i].value, 16);
            }
            v.pyramid(encoder, i, stage(i as u32 * 4 + 3));
        }
        let resolve_stamps = stamps
            .map(|s| wgpu::ComputePassTimestampWrites {
                query_set: s.query_set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: s.end_of_pass_write_index,
            })
            .or(stage(8).compute());
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N1c exact hit resolve"),
            timestamp_writes: resolve_stamps,
        });
        pass.set_pipeline(&v.resolve[usize::from(soft)]);
        pass.set_bind_group(0, &v.resolve_group, &[]);
        pass.set_bind_group(1, arrays, &[]);
        pass.set_bind_group(2, &v.data_group, &[]);
        pass.dispatch_workgroups(v.size.width.div_ceil(8), v.size.height.div_ceil(4), 1);
        pass.set_pipeline(&v.reset);
        for group in &v.cull_groups {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}
