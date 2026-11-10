//! P-S4B (docs/p-s4b.md, docs/design-nonrt-sunvis-s4.md): S4, occluder-centric sun visibility for
//! the Surface lookup's queued pixels on frames of 8 or more direction batches (cuts and history
//! restarts). shaders/sunvis_s4.wgsl describes the pass; this module owns its geometry (every
//! caster triangle's plane record behind a light-space BVH2), its module and pipeline, and the
//! per-frame decision. Moving and still frames keep the march, so their passes are unchanged.
//!
//! Geometry comes from the non-RT GI's full-detail triangles (`gi_bvh::Blas::exact`: built from
//! the drawn mesh's own positions and indices), placed by the draws' instance transforms in f64.
//! Scenes qualify ("game-class") when every draw has such geometry over its whole index range,
//! none is skinned, and the total is at most `MAX_TRIANGLES`; otherwise the march serves every
//! frame. Nothing here depends on the frame's direction budget, so the same records can serve
//! every frame once S4 replaces the march (docs/p-s4b.md "Toward the full replacement").
use super::*;
use crate::gi_bvh::Blas;

// The shader's CPU twin: `pack` builds records with it; tests and tools/p_s4b_oracle.rs run
// its test and bound.
#[path = "sunvis_s4_math.rs"]
#[allow(dead_code)]
pub(crate) mod math;

/// The largest scene S4 serves (records are 96 bytes a triangle).
pub(super) const MAX_TRIANGLES: usize = 1 << 21;
/// Frames with at least this many direction batches run S4 (cuts: 32, restarts: 8).
pub(super) const MIN_BATCHES: u32 = 8;
const HEADER_WORDS: usize = 16;
const NODE_WORDS: usize = 16;
const RECORD_WORDS: usize = 24;
const LEAF: u32 = 0x8000_0000;
const EMPTY: u32 = u32::MAX;
const LEAF_TRIANGLES: usize = 4;
/// WGSL `S4_DEPTH`: a lane's own traversal stack.
const MAX_DEPTH: usize = 31;

/// The module (the surface cache's with shaders/sunvis_s4.wgsl appended), group 2's layout,
/// the pipeline, this scene's geometry and whether this frame runs S4.
pub(super) struct S4 {
    module: Owned<wgpu::ShaderModule>,
    layout: Owned<wgpu::BindGroupLayout>,
    pipeline_layout: Owned<wgpu::PipelineLayout>,
    pipeline: Option<(Vec<(&'static str, f64)>, Owned<wgpu::ComputePipeline>)>,
    scene: Option<Scene>,
    /// The scene's key when it does not qualify (checked again only when it changes).
    refused: Option<Vec<u64>>,
    pub frame: bool,
}
struct Scene {
    key: Vec<u64>,
    _buffer: Owned<wgpu::Buffer>,
    group: Owned<wgpu::BindGroup>,
    _allocation: Allocation,
}

pub(super) fn source() -> String {
    format!(
        "{}\n{}",
        surface_cache::surface_source(),
        include_str!("../shaders/sunvis_s4.wgsl")
    )
}

impl S4 {
    /// `lookup`: the exact lookup's group 0 layout (the queued walk's group).
    pub(super) fn new(
        device: &crate::pipeline_compilation::PipelineDevice,
        ledger: &Ledger,
        lookup: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("P-S4B occluder records"),
                entries: &[storage(60, true, wgpu::ShaderStages::COMPUTE)],
            }),
            ResourceKind::BindGroupLayout,
        );
        let pipeline_layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("P-S4B occluder gather ABI"),
                bind_group_layouts: &[Some(lookup), None, Some(&layout), Some(lighting)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let module = ledger.own(
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("P-S4B occluder gather"),
                source: wgpu::ShaderSource::Wgsl(source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        Self {
            module,
            layout,
            pipeline_layout,
            pipeline: None,
            scene: None,
            refused: None,
            frame: false,
        }
    }

    /// Builds the pass with the surface cache's constants (rebuilt when they change).
    pub(super) fn ensure_pipeline(
        &mut self,
        device: &crate::pipeline_compilation::PipelineDevice,
        ledger: &Ledger,
        constants: &[(&'static str, f64)],
    ) {
        let constants: Vec<_> = constants
            .iter()
            .copied()
            .filter(|(name, _)| !name.starts_with("VSM_QUEUED") && !name.starts_with("VMAP_"))
            .chain([("VSM_DEBUG", 0.0)])
            .collect();
        if self.pipeline.as_ref().is_some_and(|(c, _)| *c == constants) {
            return;
        }
        let pipeline = ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("s4_missed"),
                layout: Some(&self.pipeline_layout),
                module: &self.module,
                entry_point: Some("s4_missed"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        );
        self.pipeline = Some((constants, pipeline));
    }

    /// This frame's pipeline and group 2 when it runs S4.
    pub(super) fn dispatch(&self) -> Option<(&wgpu::ComputePipeline, &wgpu::BindGroup)> {
        let (_, pipeline) = self.pipeline.as_ref().filter(|_| self.frame)?;
        Some((pipeline, &*self.scene.as_ref()?.group))
    }

    /// Whether the scene qualifies, (re)building its records when the draws, their transforms or
    /// the sun changed. `exact`: a draw's full-detail GI triangles, if any.
    pub(super) fn prepare<'b>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Ledger,
        draws: &[PreparedDraw<'_>],
        exact: impl Fn(&crate::Mesh) -> Option<&'b Blas>,
        sun: Vec3,
    ) -> bool {
        let mut key = vec![];
        for d in draws {
            key.push(Arc::as_ptr(&d.mesh.vertex) as usize as u64);
            key.push(u64::from(d.indices.start) | u64::from(d.indices.end) << 32);
            key.push(u64::from(d.mesh.skin.is_some()));
            for m in &d.instances.state.models {
                key.extend(m.to_cols_array().map(|v| u64::from(v.to_bits())));
            }
        }
        key.extend(sun.to_array().map(|v| u64::from(v.to_bits())));
        if self.scene.as_ref().is_some_and(|s| s.key == key) {
            return true;
        }
        if self.refused.as_ref() == Some(&key) {
            return false;
        }
        self.scene = None;
        let Some(words) = triangles(draws, &exact).and_then(|t| pack(&t, sun.as_dvec3())) else {
            self.refused = Some(key);
            return false;
        };
        self.refused = None;
        let bytes = (words.len() * 4) as u64;
        let buffer = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("P-S4B occluder records"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        queue.write_buffer(&buffer, 0, &raw);
        let group = ledger.own(
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("P-S4B occluder records"),
                layout: &self.layout,
                entries: &[buffer_entry(60, &buffer)],
            }),
            ResourceKind::BindGroup,
        );
        self.scene = Some(Scene {
            key,
            _buffer: buffer,
            group,
            _allocation: ledger.allocate(AllocationKind::Shadow, bytes),
        });
        true
    }
}
fn buffer_entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

/// Every drawn triangle in world space (f64), or None when the scene does not qualify.
fn triangles<'b>(
    draws: &[PreparedDraw<'_>],
    exact: &impl Fn(&crate::Mesh) -> Option<&'b Blas>,
) -> Option<Vec<[[f64; 3]; 3]>> {
    let mut out = vec![];
    for d in draws {
        let blas = exact(d.mesh)?;
        if d.mesh.skin.is_some() || d.indices != (0..d.mesh.index_count) {
            return None;
        }
        let bvh = &blas.bvh;
        let local: Vec<[usize; 3]> = bvh
            .leaves
            .iter()
            .flat_map(|l| {
                let at = l.indices.offset as usize;
                bvh.indices[at..at + l.indices.count as usize]
                    .chunks_exact(3)
                    .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            })
            .collect();
        if local.len() * 3 != d.mesh.index_count as usize {
            return None;
        }
        for model in &d.instances.state.models {
            let m = model.as_dmat4();
            out.extend(local.iter().map(|t| {
                t.map(|i| {
                    let p = bvh.positions[i];
                    m.transform_point3(glam::DVec3::new(
                        f64::from(p[0]),
                        f64::from(p[1]),
                        f64::from(p[2]),
                    ))
                    .to_array()
                })
            }));
            if out.len() > MAX_TRIANGLES {
                return None;
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// An orthonormal sun frame (U, V, L) for unit `l`.
fn frame(l: glam::DVec3) -> [glam::DVec3; 3] {
    let up = if l.y.abs() < 0.9 {
        glam::DVec3::Y
    } else {
        glam::DVec3::X
    };
    let u = up.cross(l).normalize();
    [u, l.cross(u), l]
}

/// The buffer shaders/sunvis_s4.wgsl reads: header, BVH2 nodes over the triangles' light-space
/// boxes (median splits, `LEAF_TRIANGLES` a leaf; node 0 holds the root), plane records in leaf
/// order. None when the tree is deeper than a lane's stack.
fn pack(triangles: &[[[f64; 3]; 3]], l: glam::DVec3) -> Option<Vec<u32>> {
    let l = l.normalize();
    let basis = frame(l);
    let light = |p: [f64; 3]| {
        let p = glam::DVec3::from_array(p);
        [basis[0].dot(p), basis[1].dot(p), basis[2].dot(p)]
    };
    let records: Vec<(math::Planes, [f64; 3], [f64; 3])> = triangles
        .iter()
        .filter_map(|t| {
            let planes = math::planes(*t, l.to_array())?;
            let q = t.map(light);
            let lo = std::array::from_fn(|k| q.iter().map(|v| v[k]).fold(f64::INFINITY, f64::min));
            let hi =
                std::array::from_fn(|k| q.iter().map(|v| v[k]).fold(f64::NEG_INFINITY, f64::max));
            Some((planes, lo, hi))
        })
        .collect();
    if records.is_empty() {
        return None;
    }
    let mut order: Vec<usize> = (0..records.len()).collect();
    let mut nodes: Vec<[u32; NODE_WORDS]> = vec![[0; NODE_WORDS]];
    let mut leaves: Vec<usize> = Vec::with_capacity(records.len());
    let mut depth = 0;
    let (lo, hi, root) = build(&records, &mut order, &mut nodes, &mut leaves, 1, &mut depth);
    if depth > MAX_DEPTH {
        return None;
    }
    nodes[0] = node([(lo, hi, root), ([0.0; 3], [0.0; 3], EMPTY)]);
    let at = HEADER_WORDS + nodes.len() * NODE_WORDS;
    let mut words = vec![0u32; at + leaves.len() * RECORD_WORDS];
    words[0] = nodes.len() as u32;
    words[1] = leaves.len() as u32;
    words[2] = at as u32;
    words[3] = depth as u32;
    for (k, axis) in basis.iter().enumerate() {
        for c in 0..3 {
            words[4 + 4 * k + c] = (axis[c] as f32).to_bits();
        }
    }
    for (i, n) in nodes.iter().enumerate() {
        words[HEADER_WORDS + i * NODE_WORDS..][..NODE_WORDS].copy_from_slice(n);
    }
    for (i, &r) in leaves.iter().enumerate() {
        let p = &records[r].0;
        let f = [
            p.p,
            p.e[0],
            p.e[1],
            p.e[2],
            [p.h[0], p.h[1], p.pl, 0.0],
            [p.c[0], p.c[1], p.c[2], 0.0],
        ];
        let w = &mut words[at + i * RECORD_WORDS..][..RECORD_WORDS];
        for (k, v) in f.iter().flatten().enumerate() {
            w[k] = v.to_bits();
        }
        w[19] = u32::from(p.l_planes);
    }
    Some(words)
}

type Box3 = ([f64; 3], [f64; 3]);
/// A node of two children (box, child word); boxes rounded outward to f32.
fn node(children: [([f64; 3], [f64; 3], u32); 2]) -> [u32; NODE_WORDS] {
    let mut n = [0u32; NODE_WORDS];
    for (c, (lo, hi, child)) in children.iter().enumerate() {
        for k in 0..3 {
            let pad = |x: f64| 1e-6 * x.abs() + 1e-6;
            n[6 * c + k] = ((lo[k] - pad(lo[k])) as f32).to_bits();
            n[6 * c + 3 + k] = ((hi[k] + pad(hi[k])) as f32).to_bits();
        }
        n[12 + c] = *child;
    }
    n
}
fn union(a: Box3, b: Box3) -> Box3 {
    (
        std::array::from_fn(|k| a.0[k].min(b.0[k])),
        std::array::from_fn(|k| a.1[k].max(b.1[k])),
    )
}
/// The subtree over `items` (indices into `records`): its child word and box. Leaves append
/// their records to `leaves`, so a leaf's triangles are consecutive.
fn build(
    records: &[(math::Planes, [f64; 3], [f64; 3])],
    items: &mut [usize],
    nodes: &mut Vec<[u32; NODE_WORDS]>,
    leaves: &mut Vec<usize>,
    level: usize,
    depth: &mut usize,
) -> ([f64; 3], [f64; 3], u32) {
    *depth = (*depth).max(level);
    let bounds = items
        .iter()
        .map(|&i| (records[i].1, records[i].2))
        .reduce(union)
        .unwrap();
    if items.len() <= LEAF_TRIANGLES {
        let first = leaves.len() as u32;
        leaves.extend_from_slice(items);
        return (
            bounds.0,
            bounds.1,
            LEAF | first << 3 | (items.len() as u32 - 1),
        );
    }
    let centre = |i: usize| -> [f64; 3] {
        std::array::from_fn(|k| 0.5 * (records[i].1[k] + records[i].2[k]))
    };
    let (clo, chi) = items
        .iter()
        .map(|&i| (centre(i), centre(i)))
        .reduce(union)
        .unwrap();
    let axis = (0..3)
        .max_by(|&a, &b| (chi[a] - clo[a]).total_cmp(&(chi[b] - clo[b])))
        .unwrap();
    let mid = items.len() / 2;
    items.select_nth_unstable_by(mid, |&a, &b| centre(a)[axis].total_cmp(&centre(b)[axis]));
    let index = nodes.len();
    nodes.push([0; NODE_WORDS]);
    let (left, right) = items.split_at_mut(mid);
    let a = build(records, left, nodes, leaves, level + 1, depth);
    let b = build(records, right, nodes, leaves, level + 1, depth);
    nodes[index] = node([a, b]);
    (bounds.0, bounds.1, index as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packed tree holds every triangle once, inside every box on its path, within a lane's
    /// stack; records keep their planes' bits.
    #[test]
    fn pack_covers_every_triangle_once() {
        let mut t = vec![];
        for i in 0..300 {
            let x = f64::from(i % 17) * 0.7;
            let z = f64::from(i / 17) * 0.4;
            let y = f64::from(i % 5) * 0.3;
            t.push([[x, y, z], [x + 0.5, y + 0.1, z], [x, y + 0.2, z + 0.5]]);
        }
        let l = glam::DVec3::new(0.3, 0.9, 0.2).normalize();
        let words = pack(&t, l).unwrap();
        let f = |i: usize| f32::from_bits(words[i]);
        let at = words[2] as usize;
        assert_eq!(words[1] as usize, t.len());
        let basis = frame(l);
        let mut seen = vec![0u32; t.len()];
        let mut stack = vec![(0u32, vec![])];
        while let Some((n, boxes)) = stack.pop() {
            let base = HEADER_WORDS + n as usize * NODE_WORDS;
            for c in 0..2 {
                let child = words[base + 12 + c];
                if child == EMPTY {
                    continue;
                }
                let lo: [f32; 3] = std::array::from_fn(|k| f(base + 6 * c + k));
                let hi: [f32; 3] = std::array::from_fn(|k| f(base + 6 * c + 3 + k));
                let mut path = boxes.clone();
                path.push((lo, hi));
                if child & LEAF != 0 {
                    let first = ((child & !LEAF) >> 3) as usize;
                    for r in first..first + (child & 7) as usize + 1 {
                        seen[r] += 1;
                        // The record's triangle: find it by its four planes (translated copies
                        // share a supporting plane).
                        let rec: [f32; 16] = std::array::from_fn(|k| f(at + r * RECORD_WORDS + k));
                        let tri = t
                            .iter()
                            .find(|v| {
                                let q = math::planes(**v, l.to_array()).unwrap();
                                [q.p, q.e[0], q.e[1], q.e[2]].concat() == rec
                            })
                            .unwrap();
                        for v in tri {
                            let q = basis.map(|a| a.dot(glam::DVec3::from_array(*v)) as f32);
                            for (lo, hi) in &path {
                                assert!((0..3).all(|k| lo[k] <= q[k] && q[k] <= hi[k]));
                            }
                        }
                    }
                } else {
                    stack.push((child, path));
                }
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
        assert!(words[3] as usize <= MAX_DEPTH);
    }

    /// The gather binds only its layout: the exact lookup's group 0, its records, the lighting.
    #[test]
    fn gather_binds_only_its_layout() {
        let source = source();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let index = module
            .entry_points
            .iter()
            .position(|e| e.name == "s4_missed")
            .unwrap();
        let used = info.get_entry_point(index);
        let lookup = [0, 7, 8, 9, 10, 15, 16, 18, 19, 20, 24, 35];
        for (handle, global) in module.global_variables.iter() {
            let Some(b) = &global.binding else { continue };
            if used[handle].is_empty() {
                continue;
            }
            let allowed = match b.group {
                0 => lookup.contains(&b.binding),
                2 => b.binding == 60,
                3 => true,
                _ => false,
            };
            assert!(
                allowed,
                "s4_missed uses @group({}) @binding({})",
                b.group, b.binding
            );
        }
    }

    /// A sliver whose third vertex lies on its opposite edge (P-S4B's first GPU screen: the game's
    /// ground has one, two vertices 3e-18 m apart) gets no record: its edge planes' orientation
    /// would be noise and its prism a half-space.
    #[test]
    fn slivers_get_no_record() {
        let l = [0.45, 0.75, 0.5];
        let sliver = [
            [0.066_324_763, 0.004_705_313, 2.027_472_734],
            [6.222_844e-17, 0.0, 2.0],
            [6.506_049e-17, 0.0, 2.0],
        ];
        assert!(math::planes(sliver, l).is_none());
        assert!(math::planes([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], l).is_some());
    }

    /// Two triangles sharing an edge get exactly negated edge planes there (no crack), and the
    /// boolean test sees a ray through the shared edge's midpoint hit at least one of them.
    #[test]
    fn shared_edges_are_watertight() {
        let l = [0.2f64, 0.95, 0.1];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        let l = l.map(|x| x / n);
        let a = [0.31, 2.0, 0.17];
        let b = [1.73, 2.4, 0.91];
        let s = math::planes([a, b, [0.2, 2.1, 1.5]], l).unwrap();
        let t = math::planes([b, a, [1.9, 1.8, -0.7]], l).unwrap();
        let (es, et) = (s.e[0], t.e[0]);
        assert_eq!(es.map(|x| -x), et);
        let mid: [f32; 3] = std::array::from_fn(|k| (0.5 * (a[k] + b[k])) as f32);
        for k in 0..64 {
            let d = [
                l[0] as f32 + 1e-3 * (k as f32).cos(),
                l[1] as f32,
                l[2] as f32,
            ];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let d = d.map(|x| x / len);
            let o: [f32; 3] = std::array::from_fn(|i| mid[i] - 3.0 * d[i]);
            assert!(math::hits(&s, o, d, 1e-3) || math::hits(&t, o, d, 1e-3));
        }
    }
}
