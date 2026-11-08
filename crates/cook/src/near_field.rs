//! M7 G1c: cook-time near-field sky occlusion (renderlab_asset::near_field) from an
//! already-cooked virtual package, for the world radiance cache's non-RT path.
//!
//! Each mesh's far field is a legal cluster-DAG cut (R0c's selector), rebuilt as a BVH4 with
//! eight triangles per leaf. With masks (`bins` > 0), the cut's vertices, which are source
//! vertices on the full-detail surface, each trace one exact ray per world-space octahedral
//! bin, on each side of their normal, `reach` long, against the full-detail triangles of every
//! placement (an f32 BVH2 in world space, built once): one blocked bit per bin and side.
//! Every vertex is traced, so nothing ranks where rays go (docs/m7-p0-claims.md 4.1). Results
//! do not depend on thread count or order.
use crate::{
    Error, Result,
    cache::{atomic_publish, hash, hex, immutable_entry, read_bounded},
    cluster_bvh,
    cluster_bvh_cache::GeometrySource,
    ensure,
};
use renderlab_asset::{
    near_field::{self as format, NearField, NearPlacement, Settings},
    virtual_mesh::{ClusterId, Package, PageGeometry, VirtualLimits},
};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

pub const ALGORITHM: &[u8] = b"renderlab-near-field/1/legal-cut-far-field/bvh4-median-8/world-octahedral-bins-two-sided/exact-any-hit-bvh2-median-8";

/// What one cook did, for the record.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub far_triangles: usize,
    pub far_positions: usize,
    pub full_triangles: usize,
    pub full_positions: usize,
    pub rays: u64,
    pub blocked: u64,
    pub gather_us: u128,
    pub tracer_us: u128,
    pub trace_us: u128,
    pub tracer_bytes: usize,
}

/// Triangles of a cut: positions, a normal per position and a triangle list.
#[derive(Default)]
struct Triangles {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

/// The cut's triangles, each page loaded once and its vertices deduplicated per page.
fn gather(
    package: &Package,
    mesh: usize,
    clusters: &[ClusterId],
    load: &mut impl FnMut(usize) -> Result<PageGeometry>,
) -> Result<Triangles> {
    let m = &package.meshes[mesh];
    let mut order = clusters.to_vec();
    order.sort_unstable_by_key(|id| (m.clusters[id.index()].page, *id));
    let mut out = Triangles::default();
    let mut current = None;
    let mut geometry = PageGeometry {
        vertices: vec![],
        indices: vec![],
        tangents: None,
    };
    let mut mapped = vec![];
    for id in order {
        let c = &m.clusters[id.index()];
        if current != Some(c.page) {
            geometry = load(c.page.index())?;
            mapped = vec![u32::MAX; geometry.vertices.len()];
            current = Some(c.page);
        }
        for &source in &geometry.indices[c.indices.range(geometry.indices.len())?] {
            let slot = mapped
                .get_mut(source as usize)
                .ok_or_else(|| Error("near-field page vertex index".into()))?;
            if *slot == u32::MAX {
                *slot = out.positions.len() as u32;
                let v = &geometry.vertices[source as usize];
                out.positions.push(v.position);
                out.normals.push(v.normal);
            }
            out.indices.push(*slot);
        }
    }
    ensure(
        out.positions.len() < u32::MAX as usize,
        "near-field position count",
    )?;
    Ok(out)
}

// ---- placement transforms (column-major, as renderlab_asset::Placement) ----
fn point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| {
        (m[0][r] as f64 * p[0] as f64
            + m[1][r] as f64 * p[1] as f64
            + m[2][r] as f64 * p[2] as f64
            + m[3][r] as f64) as f32
    })
}
/// The inverse transpose of the linear part, up to a positive scale: the cofactor matrix,
/// times the determinant's sign (a mirroring transform flips normals as winding flips).
fn normal_matrix(m: &[[f32; 4]; 4]) -> [[f64; 3]; 3] {
    let a = |r: usize, c: usize| m[c][r] as f64;
    let cof = |r: usize, c: usize| {
        let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
        let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
        a(r1, c1) * a(r2, c2) - a(r1, c2) * a(r2, c1)
    };
    let det = (0..3).map(|c| a(0, c) * cof(0, c)).sum::<f64>();
    let s = if det < 0. { -1. } else { 1. };
    std::array::from_fn(|r| std::array::from_fn(|c| cof(r, c) * s))
}
fn transform_normal(n: &[[f64; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let w: [f64; 3] = std::array::from_fn(|r| (0..3).map(|c| n[r][c] * v[c] as f64).sum());
    let l = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
    if l > 0. && l.is_finite() {
        w.map(|x| (x / l) as f32)
    } else {
        [0.; 3]
    }
}

// ---- the exact tracer: an f32 BVH2 over every placement's full-detail world triangles ----
const LEAF: u32 = 1 << 31;
const LEAF_SIZE: usize = 8;
#[derive(Clone, Copy)]
struct Node {
    lo: [f32; 3],
    hi: [f32; 3],
    /// Interior: left child. Leaf: first triangle in `order`.
    a: u32,
    /// Interior: right child. Leaf: LEAF | triangle count.
    b: u32,
}
struct Tracer {
    nodes: Vec<Node>,
    positions: Vec<[f32; 3]>,
    /// Triangles in leaf order.
    triangles: Vec<[u32; 3]>,
}
impl Tracer {
    fn new(positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]>) -> Self {
        let centroids: Vec<[f32; 3]> = triangles
            .iter()
            .map(|t| {
                std::array::from_fn(|a| {
                    t.iter().map(|&i| positions[i as usize][a]).sum::<f32>() / 3.
                })
            })
            .collect();
        let mut order: Vec<u32> = (0..triangles.len() as u32).collect();
        let mut tracer = Tracer {
            nodes: Vec::with_capacity(triangles.len() / 4 + 1),
            positions,
            triangles: vec![],
        };
        tracer.build(&mut order, 0, &centroids, &triangles);
        tracer.triangles = order.iter().map(|&t| triangles[t as usize]).collect();
        tracer
    }
    fn bytes(&self) -> usize {
        self.nodes.len() * std::mem::size_of::<Node>()
            + self.positions.len() * 12
            + self.triangles.len() * 12
    }
    /// Builds the subtree over `order` (which starts at `first` in the whole order) by
    /// median splits on the longest centroid axis; returns its node index.
    fn build(
        &mut self,
        order: &mut [u32],
        first: usize,
        centroids: &[[f32; 3]],
        triangles: &[[u32; 3]],
    ) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            lo: [0.; 3],
            hi: [0.; 3],
            a: 0,
            b: 0,
        });
        let (lo, hi) = order.iter().fold(
            ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |(mut lo, mut hi), &t| {
                for &v in &triangles[t as usize] {
                    let p = self.positions[v as usize];
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
                (lo, hi)
            },
        );
        if order.len() <= LEAF_SIZE {
            self.nodes[id as usize] = Node {
                lo,
                hi,
                a: first as u32,
                b: LEAF | order.len() as u32,
            };
            return id;
        }
        let (clo, chi) = order.iter().fold(
            ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |(mut lo, mut hi), &t| {
                for a in 0..3 {
                    lo[a] = lo[a].min(centroids[t as usize][a]);
                    hi[a] = hi[a].max(centroids[t as usize][a]);
                }
                (lo, hi)
            },
        );
        let axis = (0..3)
            .max_by(|&a, &b| (chi[a] - clo[a]).total_cmp(&(chi[b] - clo[b])))
            .unwrap();
        let mid = order.len() / 2;
        order.select_nth_unstable_by(mid, |&x, &y| {
            centroids[x as usize][axis]
                .total_cmp(&centroids[y as usize][axis])
                .then(x.cmp(&y))
        });
        let (left, right) = order.split_at_mut(mid);
        let a = self.build(left, first, centroids, triangles);
        let b = self.build(right, first + mid, centroids, triangles);
        self.nodes[id as usize] = Node { lo, hi, a, b };
        id
    }
    /// Whether the segment o + t d, t in (tmin, tmax), meets any triangle (either face).
    fn occluded(
        &self,
        o: [f32; 3],
        d: [f32; 3],
        tmin: f32,
        tmax: f32,
        stack: &mut Vec<u32>,
    ) -> bool {
        let inv: [f32; 3] = std::array::from_fn(|a| {
            if d[a].abs() < 1e-30 {
                1e30f32.copysign(d[a])
            } else {
                1. / d[a]
            }
        });
        stack.clear();
        stack.push(0);
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            let mut near = 0f32;
            let mut far = tmax;
            for a in 0..3 {
                let t0 = (n.lo[a] - o[a]) * inv[a];
                let t1 = (n.hi[a] - o[a]) * inv[a];
                near = near.max(t0.min(t1));
                far = far.min(t0.max(t1));
            }
            if near > far {
                continue;
            }
            if n.b & LEAF != 0 {
                let first = n.a as usize;
                for t in &self.triangles[first..first + (n.b & !LEAF) as usize] {
                    let [a, b, c] = t.map(|v| self.positions[v as usize]);
                    if hit(o, d, a, b, c, tmin, tmax) {
                        return true;
                    }
                }
            } else {
                stack.push(n.a);
                stack.push(n.b);
            }
        }
        false
    }
}
/// Every placement's full-detail world triangles behind the cook's exact tracer, for CPU
/// diagnostics of the near field (which surfaces a probe line or a camera ray meets).
pub struct WorldTracer(Tracer);
impl WorldTracer {
    pub fn build(
        package: &Package,
        limits: VirtualLimits,
        mut load_page: impl FnMut(usize) -> Result<PageGeometry>,
    ) -> Result<Self> {
        let (positions, triangles) = world_triangles(package, limits, &mut load_page)?;
        Ok(Self(Tracer::new(positions, triangles)))
    }
    /// Every hit of the ray within (0, tmax), nearest first: (t, unit source-winding
    /// normal).
    pub fn hits(&self, o: [f32; 3], d: [f32; 3], tmax: f32) -> Vec<(f32, [f32; 3])> {
        let mut out = vec![];
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = &self.0.nodes[i as usize];
            let (mut near, mut far) = (0f32, tmax);
            for a in 0..3 {
                let inv = 1. / d[a];
                let t0 = (n.lo[a] - o[a]) * inv;
                let t1 = (n.hi[a] - o[a]) * inv;
                near = near.max(t0.min(t1));
                far = far.min(t0.max(t1));
            }
            if near > far {
                continue;
            }
            if n.b & LEAF != 0 {
                let first = n.a as usize;
                for t in &self.0.triangles[first..first + (n.b & !LEAF) as usize] {
                    let [a, b, c] = t.map(|v| self.0.positions[v as usize]);
                    if let Some(t) = intersect(o, d, a, b, c, 0., tmax) {
                        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                        let m = [
                            e1[1] * e2[2] - e1[2] * e2[1],
                            e1[2] * e2[0] - e1[0] * e2[2],
                            e1[0] * e2[1] - e1[1] * e2[0],
                        ];
                        let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
                        out.push((t, m.map(|v| v / l)));
                    }
                }
            } else {
                stack.push(n.a);
                stack.push(n.b);
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }
}
/// World-space positions and the triangles indexing them.
type Soup = (Vec<[f32; 3]>, Vec<[u32; 3]>);
/// Every placement's full-detail triangles in world space.
fn world_triangles(
    package: &Package,
    limits: VirtualLimits,
    load_page: &mut impl FnMut(usize) -> Result<PageGeometry>,
) -> Result<Soup> {
    let mut positions = vec![];
    let mut triangles = vec![];
    let mut full: Vec<Option<Triangles>> = (0..package.meshes.len()).map(|_| None).collect();
    for p in &package.placements {
        let mesh = p.mesh as usize;
        ensure(mesh < package.meshes.len(), "near-field placement mesh")?;
        if full[mesh].is_none() {
            let cut = cluster_bvh::cut(package, mesh, 0., limits)?;
            full[mesh] = Some(gather(package, mesh, &cut, load_page)?);
        }
        let t = full[mesh].as_ref().unwrap();
        let base = positions.len() as u32;
        positions.extend(t.positions.iter().map(|&v| point(&p.transform, v)));
        triangles.extend(
            t.indices
                .chunks_exact(3)
                .map(|c| [c[0] + base, c[1] + base, c[2] + base]),
        );
        ensure(
            positions.len() < LEAF as usize,
            "near-field world position count",
        )?;
    }
    Ok((positions, triangles))
}
/// Moller-Trumbore, both faces, t in (tmin, tmax).
fn hit(
    o: [f32; 3],
    d: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    tmin: f32,
    tmax: f32,
) -> bool {
    intersect(o, d, a, b, c, tmin, tmax).is_some()
}
fn intersect(
    o: [f32; 3],
    d: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    tmin: f32,
    tmax: f32,
) -> Option<f32> {
    let sub = |x: [f32; 3], y: [f32; 3]| [x[0] - y[0], x[1] - y[1], x[2] - y[2]];
    let cross = |x: [f32; 3], y: [f32; 3]| {
        [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ]
    };
    let dot = |x: [f32; 3], y: [f32; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-30 {
        return None;
    }
    let inv = 1. / det;
    let s = sub(o, a);
    let u = dot(s, p) * inv;
    if !(0. ..=1.).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) * inv;
    if v < 0. || u + v > 1. {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > tmin && t < tmax).then_some(t)
}

/// Builds the near field of every placement in `package` (all treated as static) with
/// `threads` workers. The returned field's `source` is zero; `cook` fills it.
pub fn prepare(
    package: &Package,
    settings: Settings,
    limits: VirtualLimits,
    threads: usize,
    mut load_page: impl FnMut(usize) -> Result<PageGeometry>,
) -> Result<(NearField, Report)> {
    settings.validate()?;
    package.validate(limits)?;
    ensure(
        !package.placements.is_empty(),
        "near field needs placements",
    )?;
    let mut report = Report::default();
    let start = Instant::now();
    // Far-field cuts and their BVHs, plus each cut's object-space normals for the cook.
    let mut far = vec![];
    let mut far_normals = vec![];
    for mesh in 0..package.meshes.len() {
        let cut = cluster_bvh::cut(package, mesh, settings.lod_error, limits)?;
        let t = gather(package, mesh, &cut, &mut load_page)?;
        report.far_triangles += t.indices.len() / 3;
        report.far_positions += t.positions.len();
        let mut bvh = format::build_bvh(&t.positions, &t.indices)?;
        bvh.levels[0].requested_error = settings.lod_error;
        far.push(bvh);
        far_normals.push(t.normals);
    }
    if settings.bins == 0 {
        // The cut alone: no masks, so no full-detail tracer.
        report.gather_us = start.elapsed().as_micros();
        let placements = package
            .placements
            .iter()
            .map(|p| NearPlacement {
                mesh: p.mesh,
                transform: p.transform,
                masks: vec![],
            })
            .collect();
        let field = NearField {
            settings,
            source: [0; 32],
            meshes: far,
            placements,
        };
        return Ok((field, report));
    }
    // Full-detail world triangles of every placement.
    let (positions, triangles) = world_triangles(package, limits, &mut load_page)?;
    report.full_triangles = triangles.len();
    report.full_positions = positions.len();
    report.gather_us = start.elapsed().as_micros();
    let built = Instant::now();
    let tracer = Tracer::new(positions, triangles);
    report.tracer_bytes = tracer.bytes();
    report.tracer_us = built.elapsed().as_micros();
    // Masks: per far-field vertex of each placement, front then back, `words` each.
    let traced = Instant::now();
    let words = settings.words();
    let bins = settings.bins * settings.bins;
    let directions: Vec<[f32; 3]> = (0..bins)
        .map(|b| format::bin_direction(b, settings.bins))
        .collect();
    let mut placements = vec![];
    for p in &package.placements {
        let mesh = p.mesh as usize;
        let bvh = &far[mesh];
        let normals = normal_matrix(&p.transform);
        let vertices: Vec<([f32; 3], [f32; 3])> = bvh
            .positions
            .iter()
            .zip(&far_normals[mesh])
            .map(|(v, &n)| {
                (
                    point(&p.transform, [v[0], v[1], v[2]]),
                    transform_normal(&normals, n),
                )
            })
            .collect();
        let mut masks = vec![0u32; vertices.len() * 2 * words];
        const CHUNK: usize = 256;
        let next = AtomicUsize::new(0);
        let blocked = AtomicUsize::new(0);
        let chunks: Vec<&mut [u32]> = masks.chunks_mut(CHUNK * 2 * words).collect();
        let chunks: Vec<std::sync::Mutex<&mut [u32]>> =
            chunks.into_iter().map(std::sync::Mutex::new).collect();
        std::thread::scope(|s| {
            for _ in 0..threads.max(1) {
                s.spawn(|| {
                    let mut stack = Vec::with_capacity(128);
                    let mut count = 0usize;
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(chunk) = chunks.get(k) else { break };
                        let mut out = chunk.lock().unwrap();
                        for (j, (x, n)) in vertices[k * CHUNK..].iter().take(CHUNK).enumerate() {
                            // A small offset off each side of the surface, relative to the
                            // reach and the position's magnitude.
                            let m = x.iter().fold(0f32, |m, v| m.max(v.abs()));
                            let eps = 1e-4 * settings.reach + 64. * f32::EPSILON * m;
                            for side in 0..2 {
                                let s = if side == 0 { eps } else { -eps };
                                let o = [x[0] + n[0] * s, x[1] + n[1] * s, x[2] + n[2] * s];
                                let mask =
                                    &mut out[(j * 2 + side) * words..(j * 2 + side + 1) * words];
                                for (b, d) in directions.iter().enumerate() {
                                    if tracer.occluded(
                                        o,
                                        *d,
                                        settings.skip,
                                        settings.reach,
                                        &mut stack,
                                    ) {
                                        mask[b / 32] |= 1 << (b % 32);
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                    blocked.fetch_add(count, Ordering::Relaxed);
                });
            }
        });
        drop(chunks);
        report.rays += (vertices.len() * 2 * bins as usize) as u64;
        report.blocked += blocked.into_inner() as u64;
        placements.push(NearPlacement {
            mesh: p.mesh,
            transform: p.transform,
            masks,
        });
    }
    report.trace_us = traced.elapsed().as_micros();
    let field = NearField {
        settings,
        source: [0; 32],
        meshes: far,
        placements,
    };
    Ok((field, report))
}

// ---- content-addressed publication: near-field-v1/rla<S>-rvg<S>/{keys,packages} ----
fn namespace(cache: &Path, source: &GeometrySource) -> PathBuf {
    cache.join(format!(
        "near-field-v{}/rla{}-rvg{}",
        format::VERSION,
        source.rla_version,
        source.rvg_version
    ))
}
fn key(source: &GeometrySource, settings: &Settings) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(ALGORITHM);
    h.update(source.digest);
    h.update(source.rla_version.to_le_bytes());
    h.update(source.rvg_version.to_le_bytes());
    h.update(format::VERSION.to_le_bytes());
    for v in [settings.lod_error, settings.reach, settings.skip] {
        h.update(v.to_le_bytes());
    }
    h.update(settings.bins.to_le_bytes());
    h.finalize().into()
}
pub struct Cooked {
    pub field: NearField,
    pub digest: [u8; 32],
    pub key: [u8; 32],
    pub bytes: usize,
    pub cache_hit: bool,
    pub report: Report,
    pub total_us: u128,
}
const MANIFEST: &[u8; 4] = b"RNM\0";
const MANIFEST_BYTES: usize = 112;
/// Reuses a published near field for the same source and settings, or prepares, encodes,
/// round-trips and publishes one; `current` always names the result afterwards.
pub fn cook(
    source: &GeometrySource,
    source_cache: &Path,
    cache: &Path,
    current: &Path,
    settings: Settings,
    limits: VirtualLimits,
    threads: usize,
) -> Result<Cooked> {
    let start = Instant::now();
    let field_limits = format::Limits::default();
    settings.validate()?;
    let key = key(source, &settings);
    let directory = namespace(cache, source);
    let key_path = directory.join("keys").join(hex(&key));
    let cached = (|| -> Result<_> {
        let m = read_bounded(&key_path, MANIFEST_BYTES)?;
        ensure(
            m.len() == MANIFEST_BYTES
                && &m[..4] == MANIFEST
                && m[4..8] == format::VERSION.to_le_bytes()
                && m[8..40] == key
                && m[72..104] == source.digest
                && m[104..108] == source.rla_version.to_le_bytes()
                && m[108..112] == source.rvg_version.to_le_bytes(),
            "near-field manifest identity",
        )?;
        let digest: [u8; 32] = m[40..72].try_into().unwrap();
        let bytes = read_bounded(
            &directory.join("packages").join(hex(&digest)),
            field_limits.max_bytes + (1 << 24),
        )?;
        ensure(hash(&bytes) == digest, "near-field payload digest")?;
        let field = format::decode(&bytes, field_limits)?;
        ensure(
            field.settings == settings && field.source == source.digest,
            "near-field cached settings",
        )?;
        Ok((m, digest, field, bytes.len()))
    })();
    if let Ok((m, digest, field, bytes)) = cached {
        atomic_publish(current, |f| {
            f.write_all(&m)?;
            Ok(())
        })?;
        return Ok(Cooked {
            field,
            digest,
            key,
            bytes,
            cache_hit: true,
            report: Report::default(),
            total_us: start.elapsed().as_micros(),
        });
    }
    let (mut field, report) = prepare(&source.package, settings, limits, threads, |page| {
        crate::virtual_cache::load_page(source_cache, &source.package.pages[page])
    })?;
    field.source = source.digest;
    let bytes = format::encode(&field, field_limits)?;
    let digest = hash(&bytes);
    ensure(
        format::decode(&bytes, field_limits)? == field,
        "near-field round trip",
    )?;
    immutable_entry(&directory.join("packages").join(hex(&digest)), &bytes)?;
    let mut manifest = MANIFEST.to_vec();
    manifest.extend(format::VERSION.to_le_bytes());
    manifest.extend(key);
    manifest.extend(digest);
    manifest.extend(source.digest);
    manifest.extend(source.rla_version.to_le_bytes());
    manifest.extend(source.rvg_version.to_le_bytes());
    immutable_entry(&key_path, &manifest)?;
    atomic_publish(current, |f| {
        f.write_all(&manifest)?;
        Ok(())
    })?;
    Ok(Cooked {
        field,
        digest,
        key,
        bytes: bytes.len(),
        cache_hit: false,
        report,
        total_us: start.elapsed().as_micros(),
    })
}
/// The near field a manifest names, verified against its digest.
pub fn load(cache: &Path, current: &Path, field_limits: format::Limits) -> Result<NearField> {
    let m = read_bounded(current, MANIFEST_BYTES)?;
    ensure(
        m.len() == MANIFEST_BYTES
            && &m[..4] == MANIFEST
            && m[4..8] == format::VERSION.to_le_bytes(),
        "near-field manifest",
    )?;
    let digest: [u8; 32] = m[40..72].try_into().unwrap();
    let rla = u32::from_le_bytes(m[104..108].try_into().unwrap());
    let rvg = u32::from_le_bytes(m[108..112].try_into().unwrap());
    let path = cache
        .join(format!("near-field-v{}/rla{rla}-rvg{rvg}", format::VERSION))
        .join("packages")
        .join(hex(&digest));
    let bytes = read_bounded(&path, field_limits.max_bytes + (1 << 24))?;
    ensure(hash(&bytes) == digest, "near-field payload digest")?;
    Ok(format::decode(&bytes, field_limits)?)
}
