//! Unit N7a of docs/non-rt-n7.md: the world-space visibility cache on N0's harness.
//!
//! CPU: cells partition each triangle and keys round-trip; the 4x4 window covers the sun
//! cone and its shift keeps the overlapping bits; the composed WGSL validates on the probe
//! layout.
//! GPU (`gpu_visibility_cache_probe`, one batched offscreen process on the quarry): a cold
//! view and warm frames at poses 0 and 1, the Q-walk with lookup, fill, resolve and display
//! timed separately, a CPU replay of the cache state over its first 16 frames, validation
//! replay of every stored ray, a rotating and jumping sun, and a budgeted (progressive)
//! cold start. Counts are written for scripts/n7a-score.py.
//! `gpu_visibility_cache_bench` runs the S2/S3 benches (visibility_cache_bench.rs).
//! Nested under software_bvh_tests for its quarry, BVH and binding helpers.
use super::*;
use crate::visibility_cache::{self as vc, Chart, Key, Params};
use glam::{IVec2, Vec2, Vec4};
use std::collections::{HashMap, HashSet};

#[path = "visibility_cache_bench.rs"]
mod visibility_cache_bench;

#[test]
fn visibility_cache_cells_partition_triangles() {
    let mut rng = Rng(0x7a11);
    for level in 0..=vc::MAX_LEVEL {
        let n = (1u32 << level) as f32;
        for _ in 0..2000 {
            let (mut b1, mut b2) = (rng.unit(), rng.unit());
            if b1 + b2 > 1.0 {
                (b1, b2) = (1.0 - b1, 1.0 - b2);
            }
            let [i, j, upper] = vc::cell(Vec2::new(b1, b2), level);
            assert!(i + j + upper < 1 << level, "cell outside the triangle");
            // The point lies in its sub-triangle, up to f32 rounding of b * n.
            let (x, y) = (b1 * n - i as f32, b2 * n - j as f32);
            let slack = 1e-3;
            if upper == 0 {
                assert!(x >= -slack && y >= -slack && x + y <= 1.0 + slack, "{level} {b1} {b2}");
            } else {
                assert!(x <= 1.0 + slack && y <= 1.0 + slack && x + y >= 1.0 - slack, "{level} {b1} {b2}");
            }
            let key = Key {
                placement: rng.below(63) as u32,
                primitive: rng.below(1 << 22) as u32,
                level,
                i,
                j,
                upper,
                side: rng.below(2) as u32,
            };
            assert_eq!(Key::unpack(key.a(), key.b()), key);
            assert!(key.a() != 0 && key.a() != vc::LOCK && key.b() != 0);
            assert_eq!(vc::cell(key.centroid(), level), [i, j, upper], "centroid leaves its cell");
        }
    }
}

/// `l` tilted by `angle` toward azimuth `phi` around it.
fn tilt(l: Vec3, angle: f32, phi: f32) -> Vec3 {
    let u = l.cross(Vec3::Y).normalize();
    let v = l.cross(u);
    (l * angle.cos() + (u * phi.cos() + v * phi.sin()) * angle.sin()).normalize()
}

#[test]
fn visibility_cache_window_covers_the_sun_cone() {
    let anchor = Vec3::new(0.4, 0.8, 0.3).normalize();
    let chart = Chart::new(anchor, 0.04);
    let cos = chart.alpha.cos();
    let mut rng = Rng(0x5c0e);
    let mut counts = vec![];
    for trial in 0..3000 {
        let a = (rng.next() as u32 & 0x003f_ffff) | 3 << 22;
        let b = rng.next() as u32 | 1;
        let t = vc::lattice(a, b);
        // Suns within 20 degrees of the anchor, the most the window radius allows with room.
        let sun = if trial < 200 { tilt(anchor, 0.05 * rng.unit(), 6.283 * rng.unit()) } else { tilt(anchor, 0.35 * rng.unit().sqrt(), 6.283 * rng.unit()) };
        let window = chart.window(&t, sun);
        let disk = chart.disk(&t, window, sun);
        let mut inside = 0;
        for x in -8..12 {
            for y in -8..12 {
                if chart.direction(&t, window + IVec2::new(x, y)).dot(sun) > cos {
                    inside += 1;
                    assert!((0..4).contains(&x) && (0..4).contains(&y), "cone point outside the window");
                }
            }
        }
        assert_eq!(inside, disk.count_ones());
        counts.push((sun.dot(anchor).acos(), inside));
    }
    // The chart's density grows as 1 / cos^3 of the angle from the anchor: 8 rays on
    // average there, about 9.6 at 20 degrees.
    let near: Vec<u32> = counts.iter().filter(|c| c.0 < 0.05).map(|c| c.1).collect();
    let mean = near.iter().sum::<u32>() as f64 / near.len() as f64;
    assert!(near.len() > 10 && (7.6..8.4).contains(&mean), "mean rays {mean} near the anchor");
    let all = counts.iter().map(|c| c.1).sum::<u32>() as f64 / counts.len() as f64;
    assert!((8.0..9.6).contains(&all), "mean rays {all}");
    assert!(counts.iter().all(|c| c.1 >= 4));
    // Shifting to a window and back keeps exactly the bits both windows hold.
    for _ in 0..2000 {
        let bits = rng.next() as u32 & 0xffff;
        let (from, to) = (IVec2::new(rng.below(9) as i32 - 4, rng.below(9) as i32 - 4), IVec2::ZERO);
        let d = to - from;
        let overlap = (0..16)
            .filter(|&k: &i32| (0..4).contains(&((k & 3) - d.x)) && (0..4).contains(&((k >> 2) - d.y)))
            .fold(0u32, |m, k| m | 1u32 << k);
        assert_eq!(vc::shift(vc::shift(bits, from, to), to, from), bits & overlap);
    }
    for (x, y) in [(-3, 5), (0, 0), (-128, 127), (127, -128)] {
        let meta = 0x1234 | ((x as u32) & 0xff) << 16 | ((y as u32) & 0xff) << 24;
        assert_eq!(vc::meta_window(meta), IVec2::new(x, y));
    }
}

const CACHE_ENTRIES: [&str; 5] = ["cache_lookup", "cache_plan", "cache_fill", "cache_resolve", "cache_validate"];

#[test]
fn visibility_cache_wgsl_validates_on_the_probe_layout() {
    let soft = &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1];
    let source = vc::traced_source(soft, &bvh::constants(12, 12, 32));
    let (module, info) = validate_wgsl(&source);
    for entry in CACHE_ENTRIES {
        for (group, binding) in bindings(&module, &info, entry) {
            assert!(
                group == 3 || SOFTWARE_BINDINGS[group as usize].contains(&binding) || (group == 2 && (47..=49).contains(&binding)),
                "{entry} uses {group}/{binding}"
            );
        }
    }
}

/// Small sizes of the cache's GPU resources.
#[derive(Clone, Copy, Debug, serde::Serialize)]
struct Sizes {
    slots: u32,
    queue: u32,
    pending: u32,
}
impl Sizes {
    fn table_bytes(&self) -> u64 {
        (vc::SLOT_WORDS as u64) * 4 * u64::from(self.slots)
    }
    fn work_bytes(&self) -> u64 {
        4 * vc::work::words(self.queue, self.pending) as u64
    }
}

/// The cache's buffers (bound in N0's group 2 at 47-49) and pipelines.
struct Cache {
    params: wgpu::Buffer,
    table: wgpu::Buffer,
    work: wgpu::Buffer,
    /// The plan's fill and resolve dispatch arguments, copied out of `work`: a buffer bound
    /// read-write may not also be a dispatch's indirect buffer.
    args: wgpu::Buffer,
    sizes: Sizes,
    lookup: wgpu::ComputePipeline,
    plan: wgpu::ComputePipeline,
    fill: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    validate: wgpu::ComputePipeline,
}
/// GPU ms of one frame's passes and its counters.
#[derive(Clone, Debug, Default, serde::Serialize)]
struct FrameStats {
    lookup_ms: f64,
    plan_ms: f64,
    fill_ms: f64,
    resolve_ms: f64,
    /// First pass begin to last pass end.
    total_ms: f64,
    counters: BTreeMap<String, u32>,
}
const STAMPS: u32 = 8;
impl Cache {
    fn buffers(r: &Renderer, sizes: Sizes) -> [wgpu::Buffer; 3] {
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let make = |label, size, usage| r.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        [
            make("cache params", 144, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
            make("cache table", sizes.table_bytes(), usage),
            make("cache work", sizes.work_bytes(), usage),
        ]
    }
    fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
        [layout_entry(47, storage(true), Some(1)), layout_entry(48, storage(false), Some(1)), layout_entry(49, storage(false), Some(1))]
    }
    fn new(r: &Renderer, sw: &Software, [params, table, work]: [wgpu::Buffer; 3], sizes: Sizes, source: &str) -> Self {
        let scopes = OperationScopes::new(&r.device);
        let m = module(r, source);
        let c = |entry| compute(r, &sw.layout, &m, entry, &EIGHT);
        let args = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cache dispatch arguments"),
            size: 32,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cache = Self {
            params,
            table,
            work,
            args,
            sizes,
            lookup: c("cache_lookup"),
            plan: c("cache_plan"),
            fill: c("cache_fill"),
            resolve: c("cache_resolve"),
            validate: c("cache_validate"),
        };
        if let Some(e) = scopes.finish() {
            panic!("cache pipelines failed to compile: {e}");
        }
        cache
    }
    fn clear(&self, r: &Renderer) {
        submit(r, |e| {
            e.clear_buffer(&self.table, 0, None);
            e.clear_buffer(&self.work, 0, None);
        });
    }
    fn pass(&self, r: &Renderer, sw: &Software, e: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, dispatch: Result<[u32; 2], u64>, stamps: Option<(&wgpu::QuerySet, u32)>) {
        let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("visibility cache"),
            timestamp_writes: stamps.map(|(q, i)| wgpu::ComputePassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            }),
        });
        pass.set_pipeline(pipeline);
        for (i, g) in sw.groups.iter().enumerate() {
            pass.set_bind_group(i as u32, g, &[]);
        }
        pass.set_bind_group(3, r.lighting.bind(), &[]);
        match dispatch {
            Ok([x, y]) => pass.dispatch_workgroups(x, y, 1),
            Err(offset) => pass.dispatch_workgroups_indirect(&self.args, offset),
        }
    }
    /// One cache frame: lookup, plan, fill and resolve, each timed.
    fn frame(&self, r: &Renderer, sw: &Software, params: &Params) -> FrameStats {
        r.queue.write_buffer(&self.params, 0, &params.bytes());
        let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("cache frame"), ty: wgpu::QueryType::Timestamp, count: STAMPS });
        let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cache stamps"),
            size: 8 * u64::from(STAMPS),
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let size = r.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        submit(r, |e| {
            e.clear_buffer(&self.work, 0, Some(4 * vc::work::QUEUES as u64));
            self.pass(r, sw, e, &self.lookup, Ok([size.width.div_ceil(8), size.height.div_ceil(4)]), Some((&queries, 0)));
            self.pass(r, sw, e, &self.plan, Ok([1, 1]), Some((&queries, 2)));
            e.copy_buffer_to_buffer(&self.work, 4 * vc::work::FILL_ARGS as u64, &self.args, 0, 32);
            self.pass(r, sw, e, &self.fill, Err(0), Some((&queries, 4)));
            self.pass(r, sw, e, &self.resolve, Err(16), Some((&queries, 6)));
            e.resolve_query_set(&queries, 0..STAMPS, &resolve, 0);
        });
        let w = u32s(&read_buffer(r, &resolve));
        let stamp = |i: usize| u64::from(w[2 * i]) | u64::from(w[2 * i + 1]) << 32;
        let period = f64::from(r.queue.get_timestamp_period()) * 1e-6;
        let ms = |a: usize, b: usize| stamp(b).saturating_sub(stamp(a)) as f64 * period;
        let counters = self.counters(r);
        FrameStats { lookup_ms: ms(0, 1), plan_ms: ms(2, 3), fill_ms: ms(4, 5), resolve_ms: ms(6, 7), total_ms: ms(0, 7), counters }
    }
    fn counters(&self, r: &Renderer) -> BTreeMap<String, u32> {
        let words = read_words(r, &self.work, 0, vc::work::NAMES.len() as u64);
        vc::work::NAMES.iter().zip(words).map(|(n, w)| ((*n).to_owned(), w)).collect()
    }
    /// Validation replay of every stored ray: (rays checked, mismatches, first dumps).
    fn validate(&self, r: &Renderer, sw: &Software, params: &Params) -> serde_json::Value {
        r.queue.write_buffer(&self.params, 0, &params.bytes());
        let groups = self.sizes.slots.div_ceil(64);
        let x = groups.min(32768);
        let started = Instant::now();
        submit(r, |e| {
            e.clear_buffer(&self.work, 0, Some(4 * vc::work::QUEUES as u64));
            self.pass(r, sw, e, &self.validate, Ok([x, groups.div_ceil(x)]), None);
        });
        let w = read_words(r, &self.work, 0, vc::work::QUEUES as u64);
        serde_json::json!({"checked": w[vc::work::CHECKED], "mismatches": w[vc::work::MISMATCH],
            "dump": &w[vc::work::DUMP..vc::work::DUMP + 8], "wall_s": started.elapsed().as_secs_f64()})
    }
    /// Every claimed slot: (key a, key b, bits, meta).
    fn slots(&self, r: &Renderer) -> Vec<[u32; 4]> {
        u32s(&read_buffer(r, &self.table))
            .chunks_exact(4)
            .filter(|s| s[0] != 0 && s[0] != vc::LOCK)
            .map(|s| [s[0], s[1], s[2], s[3]])
            .collect()
    }
}
fn read_words(r: &Renderer, buffer: &wgpu::Buffer, word: u64, count: u64) -> Vec<u32> {
    let staging = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cache words"),
        size: 4 * count,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    submit(r, |e| e.copy_buffer_to_buffer(buffer, 4 * word, &staging, 0, 4 * count));
    u32s(&wait_map(r, &staging))
}
/// A count texture through a buffer copy.
fn read_counts(r: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    read_texture(r, texture)
}
fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
}
fn quantile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}
/// p50, p99 and max of one stat over frames.
fn spread_of(frames: &[FrameStats], f: impl Fn(&FrameStats) -> f64) -> serde_json::Value {
    let v: Vec<f64> = frames.iter().map(f).collect();
    serde_json::json!({"p50": median(&v), "p99": quantile(&v, 0.99), "max": v.iter().copied().fold(f64::NAN, f64::max), "mean": v.iter().sum::<f64>() / v.len().max(1) as f64})
}
fn summarize(frames: &[FrameStats]) -> serde_json::Value {
    let c = |name: &'static str| move |s: &FrameStats| f64::from(s.counters[name]);
    serde_json::json!({"frames": frames.len(),
        "lookup_ms": spread_of(frames, |s| s.lookup_ms), "fill_ms": spread_of(frames, |s| s.fill_ms),
        "resolve_ms": spread_of(frames, |s| s.resolve_ms), "plan_ms": spread_of(frames, |s| s.plan_ms),
        "shadow_ms": spread_of(frames, |s| s.lookup_ms + s.plan_ms + s.fill_ms + s.resolve_ms),
        "span_ms": spread_of(frames, |s| s.total_ms),
        "filled": spread_of(frames, c("filled")), "rays": spread_of(frames, c("rays")),
        "fallback": spread_of(frames, c("fallback")), "inserted": spread_of(frames, c("inserted")),
        "incomplete": spread_of(frames, c("incomplete")), "busy": spread_of(frames, c("busy")),
        "probe_limit": spread_of(frames, c("probe_limit")), "substituted": spread_of(frames, c("substituted")),
        "ring_claimed": spread_of(frames, c("ring_claimed")), "partial_used": spread_of(frames, c("partial_used")),
        "resolve_found": spread_of(frames, c("resolve_found")), "warm": spread_of(frames, c("warm")),
        "prefetch_busy": spread_of(frames, c("prefetch_busy")), "mover_rays": spread_of(frames, c("mover_rays"))})
}

/// The CPU mirror of cache_lookup's keys: placements as (mesh, model, normal rows), each
/// mesh's runtime adjacency (opposite-vertex edge order, as the GPU index buffers carry it),
/// and the frame's pixel angle.
struct Mirror<'a> {
    meshes: &'a [MeshData32],
    placements: Vec<(usize, Mat4, [[f32; 4]; 4])>,
    adjacency: Vec<Vec<u32>>,
    pixel_angle: f32,
    cell_scale: f32,
    flags: u32,
}
/// A pixel's key, its footprint, and how close its level and side decisions were to their
/// boundaries (GPU and CPU f32 may decide those differently).
struct Mirrored {
    key: Key,
    footprint: f32,
    level_margin: f32,
    side_margin: f32,
}
/// unit_normal in scene.wgsl.
fn unit(x: Vec3) -> Vec3 {
    let s = x.abs().max_element().max(1e-30);
    let x = x / s;
    x * x.dot(x).max(1e-30).sqrt().recip()
}
impl Mirror<'_> {
    fn vertices(&self, placement: usize, primitive: u32) -> [Vec3; 3] {
        let (mesh, _, _) = self.placements[placement];
        let (positions, indices) = &self.meshes[mesh];
        std::array::from_fn(|k| Vec3::from_array(positions[indices[3 * primitive as usize + k] as usize]))
    }
    fn world(&self, placement: usize, p: Vec3) -> Vec3 {
        (self.placements[placement].1 * p.extend(1.0)).truncate()
    }
    /// primary_geometric: the normal transform applied to the object winding normal.
    fn geometric(&self, placement: usize, v: [Vec3; 3]) -> Vec3 {
        let n = self.placements[placement].2;
        let source = unit(unit(v[1] - v[0]).cross(unit(v[2] - v[0])));
        unit(Vec3::from_slice(&n[0][..3]) * source.x + Vec3::from_slice(&n[1][..3]) * source.y + Vec3::from_slice(&n[2][..3]) * source.z)
    }
    fn key(&self, record: [u32; 4], eye: Vec3) -> Mirrored {
        let placement = record[0] as usize - 1;
        let v = self.vertices(placement, record[1]);
        let bary = Vec2::new(f32::from_bits(record[2]), f32::from_bits(record[3]));
        let [a, b, c] = v.map(|p| self.world(placement, p));
        let w = Vec3::new(1.0 - bary.x - bary.y, bary.x, bary.y);
        let point = self.world(placement, v[2].mul_add(Vec3::splat(w.z), v[1].mul_add(Vec3::splat(w.y), v[0] * w.x)));
        let footprint = (eye - point).length() * self.pixel_angle * self.cell_scale;
        let area = 0.5 * (b - a).cross(c - a).length();
        let level = vc::level(area, footprint);
        let toward = eye - point;
        let side_dot = self.geometric(placement, v).dot(toward);
        let [i, j, upper] = vc::cell(bary, level);
        Mirrored {
            key: Key { placement: placement as u32, primitive: record[1], level, i, j, upper, side: u32::from(side_dot < 0.0) },
            footprint,
            level_margin: vc::level_margin(area, footprint),
            side_margin: side_dot.abs() / toward.length(),
        }
    }
    /// The keys with the other side or level of a near-boundary decision (for classifying
    /// CPU/GPU key differences).
    fn alternatives(&self, record: [u32; 4], m: &Mirrored) -> Vec<Key> {
        let mut out = vec![];
        if m.side_margin < 1e-5 {
            out.push(Key { side: 1 - m.key.side, ..m.key });
        }
        if m.level_margin < 1e-4 {
            let bary = Vec2::new(f32::from_bits(record[2]), f32::from_bits(record[3]));
            for level in [m.key.level.saturating_sub(1), (m.key.level + 1).min(vc::MAX_LEVEL)] {
                if level != m.key.level {
                    let [i, j, upper] = vc::cell(bary, level);
                    out.push(Key { level, i, j, upper, ..m.key });
                }
            }
        }
        out
    }
    /// cache_prefetch's claims for key `k` at `footprint`, and whether any of their own
    /// decisions was near a boundary.
    fn neighbours(&self, k: &Key, footprint: f32, eye: Vec3) -> (Vec<Key>, bool) {
        if k.level > 0 {
            if self.flags & vc::flags::SUBDIVISION == 0 {
                return (vec![], false);
            }
            let last = (1u32 << k.level) - 1;
            let keys = (0..3)
                .filter_map(|e| {
                    let (mut i, mut j) = (k.i as i32, k.j as i32);
                    match (k.upper, e) {
                        (0, 1) => i -= 1,
                        (0, 2) => j -= 1,
                        (1, 1) => i += 1,
                        (1, 2) => j += 1,
                        _ => {}
                    }
                    let up = 1 - k.upper;
                    if i < 0 || j < 0 {
                        return None;
                    }
                    let n = (i + j) as u32;
                    (n <= last && !(up != 0 && n == last)).then_some(Key { i: i as u32, j: j as u32, upper: up, ..*k })
                })
                .collect();
            return (keys, false);
        }
        if self.flags & vc::flags::RING == 0 {
            return (vec![], false);
        }
        let placement = k.placement as usize;
        let mesh = self.placements[placement].0;
        let (mut keys, mut near) = (vec![], false);
        for e in 0..3 {
            let n = self.adjacency[mesh][3 * k.primitive as usize + e];
            if n == crate::receiver_adjacency::MISSING {
                continue;
            }
            let v = self.vertices(placement, n);
            let [a, b, c] = v.map(|p| self.world(placement, p));
            let area = 0.5 * (b - a).cross(c - a).length();
            near |= vc::level_margin(area, footprint) < 1e-4 && vc::level(area, footprint) <= 1;
            if vc::level(area, footprint) != 0 {
                continue;
            }
            let centre = (a + b + c) * (1.0 / 3.0);
            let toward = eye - centre;
            let side_dot = self.geometric(placement, v).dot(toward);
            near |= side_dot.abs() / toward.length() < 1e-5;
            keys.push(Key { placement: k.placement, primitive: n, level: 0, i: 0, j: 0, upper: 0, side: u32::from(side_dot < 0.0) });
        }
        (keys, near)
    }
}
/// Each mesh's runtime adjacency, built on the CPU as the upload builds it.
fn adjacency(meshes: &[MeshData32]) -> Vec<Vec<u32>> {
    meshes
        .iter()
        .map(|(positions, indices)| {
            let vertices: Vec<Vertex> = positions.iter().map(|&position| Vertex { position, normal: [0.; 3], uv: [0.; 2], color: [1.; 4] }).collect();
            crate::receiver_adjacency::build(&MeshData { vertices: &vertices, indices }).neighbours
        })
        .collect()
}

/// Unit vector toward the sun of a lighting file, as the renderer normalizes it.
fn sun(lighting: &crate::Lighting) -> Vec3 {
    match lighting.lights[0] {
        crate::Light::Directional { travel, .. } => -travel.as_dvec3().normalize().as_vec3(),
        _ => panic!("the first light is the sun"),
    }
}
/// GPU milliseconds of one encoded pass with begin and end timestamps.
fn time_once(r: &Renderer, f: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::ComputePassTimestampWrites<'_>)) -> f64 {
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("cache once"), ty: wgpu::QueryType::Timestamp, count: 2 });
    let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cache once"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    submit(r, |e| {
        f(e, wgpu::ComputePassTimestampWrites { query_set: &queries, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) });
        e.resolve_query_set(&queries, 0..2, &resolve, 0);
    });
    let w = u32s(&read_buffer(r, &resolve));
    let stamp = |i: usize| u64::from(w[2 * i]) | u64::from(w[2 * i + 1]) << 32;
    (stamp(1) - stamp(0)) as f64 * f64::from(r.queue.get_timestamp_period()) * 1e-6
}

/// The registered sun rotated about +Y by `degrees`.
fn rotated(l: Vec3, degrees: f64) -> Vec3 {
    glam::DQuat::from_rotation_y(degrees.to_radians()).mul_vec3(l.as_dvec3()).normalize().as_vec3()
}
fn lighting_with(lighting: &crate::Lighting, l: Vec3) -> crate::Lighting {
    let mut lighting = lighting.clone();
    if let crate::Light::Directional { travel, .. } = &mut lighting.lights[0] {
        *travel = -l;
    }
    lighting
}
fn quarry_shadows() -> Shadows {
    Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional { split_ends: vec![4., 12., 40.] }],
        ..Default::default()
    }
}

#[test]
#[ignore = "GPU lock, 4 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_visibility_cache_probe() {
    let manifest = manifest();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let started = Instant::now();
    let mut summary = serde_json::Map::new();
    let write = |summary: &serde_json::Map<String, serde_json::Value>| {
        fs::write(out.join("summary.json"), serde_json::to_vec_pretty(summary).unwrap()).unwrap();
    };
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let mut r = renderer(validate);
    summary.insert("validation_layer".into(), validate.into());
    let mut quarry = decode_quarry(&manifest);
    let source = quarry.source.take().unwrap();
    let (mut asset, cameras) = upload_quarry(&manifest, &mut r, source);
    let (bvhs, bvh_build) = build_all(&quarry.meshes, 4, 16.0);
    summary.insert("bvh_build".into(), bvh_build);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    traced_frame(&mut r, &color, &cameras.frames[0].camera().unwrap(), &mut asset, true);
    r.test_drain("cache first frame");
    let lighting = lighting_settings::parse(&fs::read_to_string(manifest_path(&manifest, "lighting")).unwrap()).unwrap();
    let l = sun(&lighting);
    let sizes = Sizes {
        slots: manifest["slots"].as_u64().unwrap_or(1 << 22) as u32,
        queue: manifest["queue"].as_u64().unwrap_or(1 << 21) as u32,
        pending: 1920 * 1080,
    };
    let buffers = Cache::buffers(&r, sizes);
    let refs: Vec<&Bvh> = bvhs.iter().collect();
    let packed = bvh::pack(&refs);
    let stack = packed.stack;
    let (sw, placements) = {
        let ones: Vec<_> = buffers.iter().map(|b| [b.as_entire_buffer_binding()]).collect();
        let extra: Vec<_> = Cache::layout_entries().into_iter().zip(&ones).map(|(e, b)| (e, wgpu::BindingResource::BufferArray(b))).collect();
        Software::new(&r, &asset.draws(Visibility::Frustum), &bvhs, &packed, &extra)
    };
    drop(refs);
    drop(packed);
    let (meshes, instances) = (quarry.meshes.len(), placements.len());
    let soft = primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone();
    let cache = Cache::new(&r, &sw, buffers, sizes, &vc::traced_source(&soft, &bvh::constants(meshes, instances, stack)));
    let p = Pipelines::new(&r, &sw, meshes, instances, stack);
    let sided = asset.draws(Visibility::Frustum).iter().any(|d| !d.material.unwrap().double_sided);
    let scene = placements.iter().fold(bvh::Aabb::EMPTY, |s, &(slot, model, _)| {
        let b = bvh::world_bounds(model, &bvhs[slot]);
        bvh::Aabb { lo: s.lo.min(b.lo), hi: s.hi.max(b.hi) }
    });
    let mirror = {
        let draws = asset.draws(Visibility::Frustum);
        let mut list = vec![];
        for (slot, d) in draws.iter().enumerate() {
            for (m, n) in d.instances.state.models.iter().zip(&d.instances.state.normals) {
                list.push((slot, *m, n.0));
            }
        }
        let built = Instant::now();
        let adjacency = adjacency(&quarry.meshes);
        eprintln!("cache_adjacency ms={:.0}", built.elapsed().as_secs_f64() * 1e3);
        Mirror { meshes: &quarry.meshes, placements: list, adjacency, pixel_angle: 0.0, cell_scale: 1.0, flags: 0 }
    };
    summary.insert("memory".into(), serde_json::json!({"table_bytes": sizes.table_bytes(), "work_bytes": sizes.work_bytes(),
        "params_bytes": 144, "total_bytes": sizes.table_bytes() + sizes.work_bytes() + 144, "sizes": sizes,
        "n0_bvh_bytes": bvhs.iter().map(|b| b.stats.bytes).sum::<usize>()}));
    let chart = Chart::new(l, 0.04);
    let cell_scale = manifest["cell_scale"].as_f64().unwrap_or(1.0) as f32;
    let budget = manifest["budget"].as_u64().unwrap_or(65_536) as u32;
    let flags = manifest["flags"].as_u64().map_or(vc::flags::ALL, |f| f as u32);
    let base = |camera: &Camera, sun: Vec3, stamp: u32, budget: u32| Params {
        chart,
        sun,
        scene,
        pixel_angle: 2.0 * (camera.fov_y() * 0.5).tan() / 1080.0,
        cell_scale,
        stamp,
        slots: sizes.slots,
        budget,
        flags,
        queue: sizes.queue,
        pending: sizes.pending,
        probes: 64,
        movers: false,
    };
    let receivers_at = |r: &mut Renderer, asset: &mut prepared::Prepared, camera: &Camera| {
        traced_frame(r, &color, camera, asset, true);
        r.test_drain("cache pose");
        r.queue.write_buffer(&sw.frame, 0, &frame_bytes(r, camera, [1920, 1080], sided));
        let receivers = &r.pipeline.primary.as_ref().unwrap().trace[1];
        submit(r, |e| r.encode_primary_trace(e, receivers, primary_ray_gpu::TRACE_TILE, true, None));
    };
    let save = |name: &str, bytes: &[u8]| fs::write(out.join(name), bytes).unwrap();
    let mut stamp = 0u32;
    let mut next = || {
        stamp = stamp % 0xffff + 1;
        stamp
    };
    let unbounded = sizes.queue;

    // 1. Cold views and warm frames at poses 0 and 1, N0's Interactive beside them.
    let pose = |k: usize| cameras.frames[k].camera().unwrap();
    let mut poses = serde_json::Map::new();
    let mut cold = vec![];
    for rep in 0..3 {
        receivers_at(&mut r, &mut asset, &pose(0));
        cache.clear(&r);
        let s = cache.frame(&r, &sw, &base(&pose(0), l, next(), unbounded));
        eprintln!("cache_cold rep={rep} {}", serde_json::to_string(&s).unwrap());
        cold.push(s);
    }
    let pose0_cold = read_counts(&r, &sw.counts);
    save("pose0-cold.u32", &pose0_cold);
    let mut warm0 = vec![];
    for _ in 0..8 {
        warm0.push(cache.frame(&r, &sw, &base(&pose(0), l, next(), budget)));
    }
    let pose0_warm = read_counts(&r, &sw.counts);
    save("pose0-warm.u32", &pose0_warm);
    submit(&r, |e| encode(&r, &sw, &p, Pass::SwShadows, e, None));
    let n0_0 = read_counts(&r, &sw.counts);
    save("pose0-n0-interactive.u32", &n0_0);
    // The display pass: production soft shading of this frame (cost is independent of the
    // visibility values it reads).
    let display: Vec<f64> = (0..8).map(|_| time_once(&r, |e, s| r.encode_primary_shading(e, primary_ray_gpu::Shading::Soft, Some(s)))).collect();
    let n0_ms: Vec<f64> = (0..8).map(|_| time_once(&r, |e, s| sw.pass(&r, e, &p.sw_shadows, [240, 270], Some(s)))).collect();
    let hw_ms: Vec<f64> = (0..8).map(|_| time_once(&r, |e, s| encode(&r, &sw, &p, Pass::HwShadows, e, Some(s)))).collect();
    poses.insert("pose0".into(), serde_json::json!({"cold": cold, "cold_summary": summarize(&cold), "warm": summarize(&warm0),
        "warm_equals_cold_counts": pose0_warm == pose0_cold, "display_ms": median(&display), "display_samples_ms": display,
        "n0_interactive_ms": median(&n0_ms), "n0_samples_ms": n0_ms, "hardware_interactive_ms": median(&hw_ms)}));
    eprintln!("cache_pose0 {}", serde_json::Value::Object(poses.clone()));
    // Pose 1 read from pose 0's cache, then warm, then the return to pose 0.
    receivers_at(&mut r, &mut asset, &pose(1));
    let after = cache.frame(&r, &sw, &base(&pose(1), l, next(), unbounded));
    let pose1_after = read_counts(&r, &sw.counts);
    save("pose1-after0.u32", &pose1_after);
    let mut warm1 = vec![];
    for _ in 0..4 {
        warm1.push(cache.frame(&r, &sw, &base(&pose(1), l, next(), budget)));
    }
    let pose1_warm = read_counts(&r, &sw.counts);
    save("pose1-warm.u32", &pose1_warm);
    submit(&r, |e| encode(&r, &sw, &p, Pass::SwShadows, e, None));
    save("pose1-n0-interactive.u32", &read_counts(&r, &sw.counts));
    receivers_at(&mut r, &mut asset, &pose(0));
    cache.frame(&r, &sw, &base(&pose(0), l, next(), budget));
    let returned = read_counts(&r, &sw.counts);
    poses.insert("pose1".into(), serde_json::json!({"after_pose0": after, "warm": summarize(&warm1),
        "warm_equals_after": pose1_warm == pose1_after, "return_equals_pose0": returned == pose0_warm}));
    // Pose 1 from a cold cache: the cell values do not depend on the history.
    receivers_at(&mut r, &mut asset, &pose(1));
    cache.clear(&r);
    let cold1 = cache.frame(&r, &sw, &base(&pose(1), l, next(), unbounded));
    poses.insert("pose1_cold".into(), serde_json::json!({"stats": cold1, "equals_after_pose0": read_counts(&r, &sw.counts) == pose1_after}));
    summary.insert("poses".into(), serde_json::Value::Object(poses));
    let check = cache.validate(&r, &sw, &base(&pose(1), l, next(), budget));
    eprintln!("cache_validate poses {check}");
    summary.insert("validation_poses".into(), check);
    write(&summary);

    // 2. CPU replay of the Q-walk's first frames with unbounded fills: every frame's claimed
    // key set is then a function of its hit records (centre keys, and the neighbours of
    // centres not complete at the frame's start), so the GPU table must hold exactly the
    // CPU's keys, each complete, in the CPU's window, with N0's any-hit for a sample of rays.
    let walk = cameras::Cameras::load(&manifest_path(&manifest, "walk")).unwrap();
    let hit = r.primary.as_ref().unwrap().targets.as_ref().unwrap().hit.texture.clone();
    let replay_frames = manifest["replay_frames"].as_u64().unwrap_or(16) as usize;
    let mut mirror = mirror;
    mirror.flags = flags;
    let replay = {
        let t = Instant::now();
        cache.clear(&r);
        let mut present: HashSet<Key> = HashSet::new();
        let mut centres: HashSet<Key> = HashSet::new();
        let mut ambiguous: HashSet<Key> = HashSet::new();
        let mut frames = vec![];
        let mut cpu_ms = 0.0;
        for pose in walk.frames.iter().take(replay_frames) {
            let camera = pose.camera().unwrap();
            receivers_at(&mut r, &mut asset, &camera);
            let params = base(&camera, l, next(), unbounded);
            frames.push(cache.frame(&r, &sw, &params));
            let started = Instant::now();
            mirror.pixel_angle = params.pixel_angle;
            let eye = Vec3::from_array(pose.eye);
            let records = u32s(&read_tex(&r, &hit));
            let mut added: HashSet<Key> = HashSet::new();
            for record in records.chunks_exact(4) {
                if record[0] == 0 {
                    continue;
                }
                let record: [u32; 4] = record.try_into().unwrap();
                let m = mirror.key(record, eye);
                for alt in mirror.alternatives(record, &m) {
                    ambiguous.insert(alt);
                    ambiguous.extend(mirror.neighbours(&alt, m.footprint, eye).0);
                }
                centres.insert(m.key);
                if present.contains(&m.key) {
                    continue;
                }
                added.insert(m.key);
                let (keys, near) = mirror.neighbours(&m.key, m.footprint, eye);
                if near {
                    ambiguous.extend(keys.iter().copied());
                }
                added.extend(keys);
            }
            present.extend(added);
            cpu_ms += started.elapsed().as_secs_f64() * 1e3;
        }
        let gpu: HashMap<Key, [u32; 2]> = cache.slots(&r).iter().map(|s| (Key::unpack(s[0], s[1]), [s[2], s[3]])).collect();
        let only_gpu: Vec<&Key> = gpu.keys().filter(|k| !present.contains(k)).collect();
        let only_cpu: Vec<&Key> = present.iter().filter(|k| !gpu.contains_key(k)).collect();
        // Each one-sided key: another key of the same triangle on the other side (a level or
        // side decision differs), a prefetch-only key, or a centre.
        let triangles = |set: &mut dyn Iterator<Item = &Key>| -> HashSet<(u32, u32)> { set.map(|k| (k.placement, k.primitive)).collect() };
        let gpu_triangles = triangles(&mut gpu.keys());
        let cpu_triangles = triangles(&mut present.iter());
        let classify = |keys: &[&Key], other: &HashSet<(u32, u32)>| {
            let mut c = BTreeMap::<&str, usize>::new();
            for k in keys {
                let class = if ambiguous.contains(*k) {
                    "near_boundary"
                } else if other.contains(&(k.placement, k.primitive)) {
                    "same_triangle_other_key"
                } else if centres.contains(*k) {
                    "centre"
                } else {
                    "prefetch_only"
                };
                *c.entry(class).or_default() += 1;
            }
            c
        };
        let gpu_classes = classify(&only_gpu, &cpu_triangles);
        let cpu_classes = classify(&only_cpu, &gpu_triangles);
        let mut window_boundary = 0;
        let mut window_differences = 0;
        let mut disk_differences = 0;
        let mut incomplete = 0;
        let mut rng = Rng(0x16f);
        let mut sample = vec![];
        let mut levels = BTreeMap::<u32, usize>::new();
        for (k, [bits, meta]) in &gpu {
            *levels.entry(k.level).or_default() += 1;
            let lattice = vc::lattice(k.a(), k.b());
            let window = chart.window(&lattice, l);
            let disk = chart.disk(&lattice, window, l);
            if vc::meta_window(*meta) != window {
                window_differences += 1;
                let x = chart.centre(&lattice, l) - Vec2::splat(chart.window_radius(l) / chart.spacing);
                window_boundary += usize::from((x - x.round()).abs().min_element() < 1e-4);
            }
            incomplete += usize::from((bits >> 16) & disk != disk);
            disk_differences += usize::from((bits >> 16) != disk);
            if rng.below(64) == 0 && sample.len() < 20_000 {
                sample.push((*k, *bits, vc::meta_window(*meta)));
            }
        }
        let world = CpuRays::new(&quarry.meshes, &bvhs, &mirror.placements);
        let (mut rays, mut differ, mut differ_near) = (0, 0, 0);
        let mut differ_examples = vec![];
        for (k, bits, window) in &sample {
            let lattice = vc::lattice(k.a(), k.b());
            let receiver = world.receiver(&mirror, k);
            for bit in 0..16 {
                if (bits >> 16) & (1 << bit) == 0 {
                    continue;
                }
                let d = chart.direction(&lattice, *window + IVec2::new(bit & 3, bit >> 2));
                let blocked = world.blocked(&receiver, d, &scene, false).0;
                rays += 1;
                if blocked != (bits & (1 << bit) != 0) {
                    differ += 1;
                    let margin = world.blocked(&receiver, d, &scene, true).1;
                    differ_near += usize::from(margin < 1e-6);
                    if differ_examples.len() < 8 {
                        differ_examples.push(serde_json::json!({"key": format!("{k:?}"), "bit": bit, "cpu_blocked": blocked, "margin_m": margin,
                            "origin": receiver.world.to_array(), "direction": d.to_array(), "epsilon": receiver.epsilon}));
                    }
                }
            }
        }
        serde_json::json!({"frames": replay_frames, "frame_stats": summarize(&frames), "cpu_keys": present.len(), "gpu_keys": gpu.len(),
            "gpu_keys_by_level": levels,
            "only_gpu": only_gpu.len(), "only_cpu": only_cpu.len(),
            "only_gpu_near_boundary": only_gpu.iter().filter(|k| ambiguous.contains(k)).count(),
            "only_cpu_near_boundary": only_cpu.iter().filter(|k| ambiguous.contains(k)).count(),
            "only_gpu_examples": only_gpu.iter().filter(|k| !ambiguous.contains(k)).take(8).map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "only_cpu_examples": only_cpu.iter().filter(|k| !ambiguous.contains(k)).take(8).map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "only_gpu_classes": gpu_classes, "only_cpu_classes": cpu_classes, "cpu_centres": centres.len(),
            "window_differences": window_differences, "window_differences_within_1e-4_of_a_lattice_line": window_boundary,
            "cone_mask_differences": disk_differences, "incomplete_cells": incomplete, "ray_difference_examples": differ_examples,
            "sampled_cells": sample.len(), "sampled_rays": rays, "ray_differences": differ, "ray_differences_within_1um_of_an_edge": differ_near,
            "cpu_key_ms": cpu_ms, "replay_s": t.elapsed().as_secs_f64()})
    };
    eprintln!("cache_replay {replay}");
    summary.insert("cpu_replay".into(), replay);
    write(&summary);

    // 3. The Q-walk: frame 0 is a cold view (unbounded fill), then the budgeted walk.
    let keep: Vec<u64> = manifest["walk_keep"].as_array().map_or(vec![0, 1, 16, 48, 64, 96, 128, 160], |a| a.iter().map(|v| v.as_u64().unwrap()).collect());
    cache.clear(&r);
    let mut frames = vec![];
    for (index, pose) in walk.frames.iter().enumerate() {
        let camera = pose.camera().unwrap();
        receivers_at(&mut r, &mut asset, &camera);
        let params = base(&camera, l, next(), if index == 0 { unbounded } else { budget });
        let stats = cache.frame(&r, &sw, &params);
        if keep.contains(&(index as u64)) {
            save(&format!("walk-{index:03}.u32"), &read_counts(&r, &sw.counts));
        }
        if index % 16 == 0 {
            eprintln!("cache_walk {index} {}", serde_json::to_string(&stats).unwrap());
        }
        frames.push(stats);
    }
    let walk_end = read_counts(&r, &sw.counts);
    let moving: Vec<FrameStats> = walk.frames.iter().zip(&frames).skip(1).filter(|(p, _)| p.phase != "hold").map(|(_, s)| s.clone()).collect();
    summary.insert("walk".into(), serde_json::json!({"frames": frames, "cold_frame_0": frames[0], "after_frame_0": summarize(&frames[1..]),
        "moving": summarize(&moving), "end_equals_pose0_warm": walk_end == pose0_warm,
        "phases": walk.frames.iter().map(|p| p.phase.clone()).collect::<Vec<_>>()}));
    eprintln!("cache_walk_summary {}", summary["walk"]["moving"]);
    write(&summary);
    let check = cache.validate(&r, &sw, &base(&pose(0), l, next(), budget));
    eprintln!("cache_validate walk {check}");
    summary.insert("validation_walk".into(), check);

    // 4. Sun motion at pose 0 on a warm cache: 1 and 10 degrees per second at 60 Hz, then a
    // 5 degree jump. Production lighting follows the sun so the fallback traces it too.
    receivers_at(&mut r, &mut asset, &pose(0));
    cache.clear(&r);
    cache.frame(&r, &sw, &base(&pose(0), l, next(), unbounded));
    let mut sun = serde_json::Map::new();
    let mut angle = 0.0f64;
    let shadows = quarry_shadows();
    for (name, step, count) in [("1deg_s", 1.0 / 60.0, 64), ("10deg_s", 10.0 / 60.0, 64), ("jump_5deg", 5.0, 1)] {
        let mut stats = vec![];
        for _ in 0..count {
            angle += step;
            let s = rotated(l, angle);
            r.set_lighting_and_shadows(&lighting_with(&lighting, s), &shadows).unwrap();
            receivers_at(&mut r, &mut asset, &pose(0));
            stats.push(cache.frame(&r, &sw, &base(&pose(0), s, next(), budget)));
        }
        // The frame after: whatever the budget left behind.
        let s = rotated(l, angle);
        let settle = cache.frame(&r, &sw, &base(&pose(0), s, next(), budget));
        save(&format!("sun-{name}.u32"), &read_counts(&r, &sw.counts));
        eprintln!("cache_sun {name} {}", summarize(&stats));
        sun.insert(name.into(), serde_json::json!({"degrees_per_frame": step, "frames": count, "summary": summarize(&stats), "stats": stats, "next_frame": settle}));
    }
    let check = cache.validate(&r, &sw, &base(&pose(0), rotated(l, angle), next(), budget));
    sun.insert("validation".into(), check);
    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    summary.insert("sun".into(), serde_json::Value::Object(sun));
    write(&summary);

    // 5. Progressive cold start at pose 0: the fill budget amortizes the cold view, and the
    // rest of the frame falls back to Interactive at each deferred pixel.
    receivers_at(&mut r, &mut asset, &pose(0));
    let mut progressive = serde_json::Map::new();
    for &b in manifest["progressive_budgets"].as_array().map_or(vec![65_536u64, 131_072], |a| a.iter().map(|v| v.as_u64().unwrap()).collect()).iter() {
        cache.clear(&r);
        let mut stats = vec![];
        for _ in 0..64 {
            let s = cache.frame(&r, &sw, &base(&pose(0), l, next(), b as u32));
            let done = s.counters["fallback"] == 0 && s.counters["filled"] == 0;
            stats.push(s);
            if done {
                break;
            }
        }
        let equal = read_counts(&r, &sw.counts) == pose0_warm;
        eprintln!("cache_progressive budget={b} frames={} {}", stats.len(), summarize(&stats));
        progressive.insert(b.to_string(), serde_json::json!({"frames_until_warm": stats.len(), "summary": summarize(&stats), "stats": stats, "final_equals_warm": equal}));
    }
    summary.insert("progressive".into(), serde_json::Value::Object(progressive));
    summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
    write(&summary);
    let failures: Vec<&str> = [
        (summary["validation_poses"]["mismatches"] != 0, "pose validation"),
        (summary["validation_walk"]["mismatches"] != 0, "walk validation"),
        (summary["sun"]["validation"]["mismatches"] != 0, "sun validation"),
    ]
    .into_iter()
    .filter_map(|(bad, name)| bad.then_some(name))
    .collect();
    eprintln!("cache_done failures={failures:?} wall_s={:.1}", started.elapsed().as_secs_f64());
    assert!(failures.is_empty(), "stored outcomes differ from fresh traces: {failures:?}");
}

/// N0's CPU any-hit for replayed cache rays: placements' world-to-object rows (f64 inverse
/// rounded to f32, as the instance records hold them) and padded world boxes.
struct CpuRays<'a> {
    meshes: &'a [MeshData32],
    bvhs: &'a [Bvh],
    placed: Vec<(usize, [Vec4; 3], bvh::Aabb)>,
}
/// A cell's lattice-point receiver on the CPU (cache_receiver).
struct CpuReceiver {
    world: Vec3,
    geometric: Vec3,
    epsilon: f32,
    side: u32,
}
impl<'a> CpuRays<'a> {
    fn new(meshes: &'a [MeshData32], bvhs: &'a [Bvh], placements: &[(usize, Mat4, [[f32; 4]; 4])]) -> Self {
        let placed = placements
            .iter()
            .map(|&(slot, model, _)| {
                let rows = model.as_dmat4().inverse().transpose();
                (slot, [0, 1, 2].map(|r| rows.col(r).as_vec4()), bvh::world_bounds(model, &bvhs[slot]))
            })
            .collect();
        Self { meshes, bvhs, placed }
    }
    fn receiver(&self, mirror: &Mirror<'_>, k: &Key) -> CpuReceiver {
        let (_, model, n) = mirror.placements[k.placement as usize];
        let v = mirror.vertices(k.placement as usize, k.primitive);
        let bary = k.centroid();
        let w = Vec3::new(1.0 - bary.x - bary.y, bary.x, bary.y);
        let object = v[2].mul_add(Vec3::splat(w.z), v[1].mul_add(Vec3::splat(w.y), v[0] * w.x));
        let world = |p: Vec3| (model * p.extend(1.0)).truncate();
        let [a, b, c] = v.map(world);
        let unit = |x: Vec3| {
            let s = x.abs().max_element().max(1e-30);
            let x = x / s;
            x * x.dot(x).max(1e-30).sqrt().recip()
        };
        let source = unit(unit(v[1] - v[0]).cross(unit(v[2] - v[0])));
        let geometric = unit(Vec3::from_slice(&n[0][..3]) * source.x + Vec3::from_slice(&n[1][..3]) * source.y + Vec3::from_slice(&n[2][..3]) * source.z);
        let m = a.abs().max(b.abs()).max(c.abs()).max_element();
        let epsilon = (16.0 * (f32::from_bits(m.to_bits() + 1) - m)).max(1e-5);
        CpuReceiver { world: world(object), geometric, epsilon, side: k.side }
    }
    /// Whether the opaque receiver's ray toward `d` is blocked within the scene box and,
    /// with `margins`, the smallest f64 margin of any candidate triangle (edge or endpoint
    /// distance).
    fn blocked(&self, r: &CpuReceiver, d: Vec3, scene: &bvh::Aabb, margins: bool) -> (bool, f64) {
        let normal = if r.side != 0 { -r.geometric } else { r.geometric };
        let origin = normal.mul_add(Vec3::splat(r.epsilon), r.world);
        let inv = Vec3::select(d.abs().cmplt(Vec3::splat(1e-30)), Vec3::splat(1e30), d.recip());
        let (a, b) = ((scene.lo - origin) * inv, (scene.hi - origin) * inv);
        let tmax = a.max(b).min_element();
        if !(tmax > r.epsilon) {
            return (false, f64::INFINITY);
        }
        let mut margin = f64::INFINITY;
        let mut blocked = false;
        let mut candidates = vec![];
        for &(slot, rows, _) in &self.placed {
            let q = origin.extend(1.0);
            let o = Vec3::new(rows[0].dot(q), rows[1].dot(q), rows[2].dot(q));
            let dir = Vec3::new(rows[0].truncate().dot(d), rows[1].truncate().dot(d), rows[2].truncate().dot(d));
            let ray = Ray { origin: o, direction: dir, tmin: r.epsilon, tmax };
            let tris = tris(&self.meshes[slot]);
            blocked |= self.bvhs[slot].trace(tris, &ray, bvh::Mode::Any).is_some();
            if margins {
                candidates.clear();
                self.bvhs[slot].candidates(o.as_dvec3(), dir.as_dvec3(), f64::from(r.epsilon), f64::from(tmax), 1e-6, &mut candidates);
                for &primitive in &candidates {
                    let v = tris.vertices(primitive).map(|x| x.as_dvec3());
                    margin = margin.min(bvh::replay(o.as_dvec3(), dir.as_dvec3(), f64::from(r.epsilon), f64::from(tmax), v).margin);
                }
            }
            if blocked && !margins {
                break;
            }
        }
        (blocked, margin)
    }
}
