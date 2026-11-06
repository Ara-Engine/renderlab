//! Test-only light-space caster grid (LSG) for non-RT sun shadows: unit N4 of
//! docs/non-rt-path-2.md.
//!
//! Light space is (u, v, l), with `l` the grid's axis toward the light. Each triangle's
//! light-space box, widened by its placement's rounding margin (`margin`), is binned into
//! every fine cell it overlaps. A cell lists `(id, widened top, box)` sorted by top
//! descending; the box is the widened (u, v) box rounded outward to 8 bits within the
//! slightly enlarged cell (`quantize`). A max-top pyramid sits above the cells.
//!
//! A shadow ray is a line in light space that rises as it advances. `Grid::trace` walks the
//! cells its projection crosses, skips every pyramid cell whose top lies below the ray
//! where it enters, and scans a fine cell's list until the tops fall below the ray, testing
//! only triangles whose box the ray crosses below their top. The caller tests those with
//! N0's watertight test in placement object space, so blocked answers equal the software
//! BVH's while the widening covers the rounding. `shaders/light_grid.wgsl` mirrors `trace`
//! and builds the same layout on the GPU.
use crate::software_bvh::{self as bvh, Aabb, Triangles};
use glam::{DVec2, DVec3, Mat4, Vec2, Vec3};
use std::time::Instant;

/// Entry ids: placement in the top bits, primitive below.
pub const PLACEMENT_SHIFT: u32 = 28;
pub const PRIMITIVE_MASK: u32 = (1 << PLACEMENT_SHIFT) - 1;
/// Top of an empty cell.
pub const EMPTY: f32 = -f32::MAX;
/// Iterations after which a ray falls back to the BVH (never reached on the quarry).
pub const MAX_STEPS: u32 = 4096;
/// Words per entry: id, top bits, quantized box.
pub const ENTRY_WORDS: usize = 3;
/// Query WGSL, appended to N0's traced module; build WGSL, a module of its own.
pub const QUERY_WGSL: &str = include_str!("../shaders/light_grid.wgsl");
const BUILD: &str = "// ---- build module ----";
/// N0's shadow-ray seam: its software any-hit call, replaced by the grid query.
const OCCLUDED: &str = "                blocked=software_occluded(ray_origin,direction,tmin,tmax);\n";
const GRID_OCCLUDED: &str =
    "                blocked=light_grid_occluded(ray_origin,direction,tmin,tmax);\n";

/// The grid's basis and fine-cell geometry, shared by the CPU and GPU builds.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub u: Vec3,
    pub v: Vec3,
    /// Toward the light (the sun's, or tilted from it for a moving sun).
    pub l: Vec3,
    /// Light-space (u, v) of cell (0, 0)'s lower corner.
    pub origin: Vec2,
    pub cell: f32,
    pub dims: [u32; 2],
}
impl Frame {
    /// A frame whose fine cells cover the light-space box of every `world` box, widened by
    /// `margin` and one cell.
    pub fn new(l: Vec3, cell: f32, world: &[Aabb], margin: f32) -> Self {
        let l64 = l.as_dvec3().normalize();
        let helper = if l64.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        let u64 = l64.cross(helper).normalize();
        let v64 = l64.cross(u64);
        let (u, v, l) = (u64.as_vec3(), v64.as_vec3(), l64.as_vec3());
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for b in world {
            for corner in 0..8 {
                let pick = |bit: usize, a: usize| if corner & bit == 0 { b.lo[a] } else { b.hi[a] };
                let p = DVec3::new(pick(1, 0).into(), pick(2, 1).into(), pick(4, 2).into());
                let q = DVec2::new(p.dot(u.as_dvec3()), p.dot(v.as_dvec3()));
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        let pad = f64::from(margin + cell);
        let (lo, hi) = (lo - pad, hi + pad);
        let dims = ((hi - lo) / f64::from(cell)).ceil().as_uvec2().max(glam::UVec2::ONE);
        Self { u, v, l, origin: lo.as_vec2(), cell, dims: dims.to_array() }
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"u": self.u.to_array(), "v": self.v.to_array(), "l": self.l.to_array(),
            "origin": self.origin.to_array(), "cell": self.cell, "dims": self.dims})
    }
    /// Pyramid level dims, finest first, up to a single cell.
    pub fn levels(&self) -> Vec<[u32; 2]> {
        let mut levels = vec![self.dims];
        while levels.last().unwrap().iter().any(|&n| n > 1) {
            let [x, y] = *levels.last().unwrap();
            levels.push([x.div_ceil(2), y.div_ceil(2)]);
        }
        levels
    }
    /// A box quantization step: the cell enlarged by 1/16 on each side, in 255 steps.
    pub fn quantum(&self) -> f32 {
        self.cell * 1.125 / 255.0
    }
}

/// The world rounding margin of one placement's triangles: 8 ulps of its largest
/// object-space magnitude (mesh or any scene point moved into its object space, so ray
/// origins too) stretched to world units, 16 ulps of the scene's world magnitude, and 1e-6
/// of the scene diagonal for direction rounding. Watertight hits of the f32 object-space
/// ray lie within about 2 object ulps of the exact world ray, so this covers them with room.
pub fn margin(model: Mat4, object: Aabb, scene: Aabb) -> f32 {
    let m = model.as_dmat4();
    let inverse = m.inverse();
    let mut magnitude = f64::from(object.lo.abs().max(object.hi.abs()).max_element());
    for corner in 0..8 {
        let pick = |bit: usize, a: usize| if corner & bit == 0 { scene.lo[a] } else { scene.hi[a] };
        let p = DVec3::new(pick(1, 0).into(), pick(2, 1).into(), pick(4, 2).into());
        magnitude = magnitude.max(inverse.transform_point3(p).abs().max_element());
    }
    let stretch = (m.x_axis.truncate().length_squared()
        + m.y_axis.truncate().length_squared()
        + m.z_axis.truncate().length_squared())
    .sqrt();
    let world = scene.lo.abs().max(scene.hi.abs()).max_element();
    let diagonal = f64::from((scene.hi - scene.lo).length());
    (8.0 * f64::from(bvh::ulp(magnitude as f32)) * stretch
        + 16.0 * f64::from(bvh::ulp(world))
        + 1e-6 * diagonal) as f32
}

/// One placement's triangles as the grid bins them.
#[derive(Clone, Copy)]
pub struct Placement<'a> {
    pub tris: Triangles<'a>,
    pub model: Mat4,
    pub margin: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub id: u32,
    pub top: f32,
    /// `quantize`d (u, v) box: lo.x, lo.y, hi.x, hi.y bytes, low first.
    pub bbox: u32,
}

/// The widened light-space box `b` of a triangle in fine cell (x, y), rounded outward to
/// 8-bit steps of the enlarged cell and clamped to it. Clamping is safe: the ray is only
/// tested over this cell, and the enlargement keeps rounding at faces inside.
pub fn quantize(frame: &Frame, x: u32, y: u32, b: [f64; 4]) -> u32 {
    let (c, q) = (f64::from(frame.cell), f64::from(frame.quantum()));
    let base = [f64::from(x) * c - c / 16.0, f64::from(y) * c - c / 16.0];
    let o = [f64::from(frame.origin.x), f64::from(frame.origin.y)];
    let step = |v: f64, a: usize, up: bool| {
        let s = (v - o[a] - base[a]) / q;
        (if up { s.ceil() } else { s.floor() }).clamp(0.0, 255.0) as u32
    };
    step(b[0], 0, false) | step(b[1], 1, false) << 8 | step(b[2], 0, true) << 16 | step(b[3], 1, true) << 24
}
/// The grid-relative (lo, hi) of a `quantize`d box, in f32 as the WGSL computes it.
pub fn dequantize(frame: &Frame, x: i64, y: i64, bbox: u32) -> (Vec2, Vec2) {
    let (c, q) = (frame.cell, frame.quantum());
    let base = Vec2::new(x as f32, y as f32) * c - Vec2::splat(c * 0.0625);
    let at = |shift: u32| ((bbox >> shift) & 255) as f32;
    (
        Vec2::new(at(0).mul_add(q, base.x), at(8).mul_add(q, base.y)),
        Vec2::new(at(16).mul_add(q, base.x), at(24).mul_add(q, base.y)),
    )
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct BuildStats {
    pub triangles: usize,
    pub cells: usize,
    pub entries: usize,
    pub nonempty_cells: usize,
    pub max_list: usize,
    pub mean_cells_per_triangle: f64,
    pub levels: usize,
    pub build_ms: f64,
    /// Bytes of the GPU layout: list ends, entries and pyramid.
    pub bytes: usize,
}

pub struct Grid {
    pub frame: Frame,
    /// Pyramid level dims, finest first; `tops[k]` is row-major (x + y * width).
    pub levels: Vec<[u32; 2]>,
    pub tops: Vec<Vec<f32>>,
    /// Exclusive end of each fine cell's list in `entries` (begin is the previous end).
    pub ends: Vec<u32>,
    pub entries: Vec<Entry>,
    pub stats: BuildStats,
}

/// Light-space box (u, v lo, u, v hi) and top of a world triangle, widened by `margin`.
fn light_box(frame: &Frame, w: [DVec3; 3], margin: f64) -> ([f64; 4], f64) {
    let (u, v, l) = (frame.u.as_dvec3(), frame.v.as_dvec3(), frame.l.as_dvec3());
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut top = f64::NEG_INFINITY;
    for p in w {
        let (x, y) = (p.dot(u), p.dot(v));
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        top = top.max(p.dot(l));
    }
    ([b[0] - margin, b[1] - margin, b[2] + margin, b[3] + margin], top + margin)
}

/// Fine-cell range [x0, x1] x [y0, y1] of a widened box, clamped to the grid.
fn cell_range(frame: &Frame, b: [f64; 4]) -> [u32; 4] {
    let cell = f64::from(frame.cell);
    let o = frame.origin.as_dvec2();
    let clamp = |x: f64, n: u32| (x.floor().max(0.0) as u64).min(u64::from(n - 1)) as u32;
    [
        clamp((b[0] - o.x) / cell, frame.dims[0]),
        clamp((b[1] - o.y) / cell, frame.dims[1]),
        clamp((b[2] - o.x) / cell, frame.dims[0]),
        clamp((b[3] - o.y) / cell, frame.dims[1]),
    ]
}

impl Grid {
    /// CPU reference build in f64: count, prefix sum, fill, sort each list by (top
    /// descending, id ascending), then the pyramid.
    pub fn build(frame: Frame, placements: &[Placement<'_>]) -> Self {
        let started = Instant::now();
        let cells = (frame.dims[0] as usize) * (frame.dims[1] as usize);
        let visit = |fill: &mut dyn FnMut(u32, [u32; 4], f32, [f64; 4])| {
            for (k, p) in placements.iter().enumerate() {
                let m = p.model.as_dmat4();
                for t in 0..p.tris.len() as u32 {
                    let w = p.tris.vertices(t).map(|v| m.transform_point3(v.as_dvec3()));
                    let (b, top) = light_box(&frame, w, f64::from(p.margin));
                    fill((k as u32) << PLACEMENT_SHIFT | t, cell_range(&frame, b), top as f32, b);
                }
            }
        };
        let mut counts = vec![0u32; cells];
        let mut triangles = 0;
        visit(&mut |_, r, _, _| {
            triangles += 1;
            for y in r[1]..=r[3] {
                for x in r[0]..=r[2] {
                    counts[(x + y * frame.dims[0]) as usize] += 1;
                }
            }
        });
        let mut ends = Vec::with_capacity(cells);
        let mut total = 0u32;
        for c in &counts {
            total += c;
            ends.push(total);
        }
        drop(counts);
        let mut cursor: Vec<u32> = std::iter::once(0).chain(ends[..cells - 1].iter().copied()).collect();
        let mut entries = vec![Entry { id: 0, top: 0.0, bbox: 0 }; total as usize];
        visit(&mut |id, r, top, b| {
            for y in r[1]..=r[3] {
                for x in r[0]..=r[2] {
                    let c = (x + y * frame.dims[0]) as usize;
                    entries[cursor[c] as usize] = Entry { id, top, bbox: quantize(&frame, x, y, b) };
                    cursor[c] += 1;
                }
            }
        });
        drop(cursor);
        let (mut begin, mut max_list, mut nonempty) = (0, 0, 0);
        let levels = frame.levels();
        let mut tops = vec![vec![EMPTY; cells]];
        for (c, &end) in ends.iter().enumerate() {
            let list = &mut entries[begin as usize..end as usize];
            list.sort_unstable_by(|a, b| b.top.total_cmp(&a.top).then(a.id.cmp(&b.id)));
            if let Some(first) = list.first() {
                tops[0][c] = first.top;
                nonempty += 1;
            }
            max_list = max_list.max(list.len());
            begin = end;
        }
        for k in 1..levels.len() {
            let ([w, h], [cw, ch]) = (levels[k], levels[k - 1]);
            let mut level = vec![EMPTY; (w * h) as usize];
            for y in 0..h {
                for x in 0..w {
                    let mut m = EMPTY;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (cx, cy) = (2 * x + dx, 2 * y + dy);
                        if cx < cw && cy < ch {
                            m = m.max(tops[k - 1][(cx + cy * cw) as usize]);
                        }
                    }
                    level[(x + y * w) as usize] = m;
                }
            }
            tops.push(level);
        }
        let pyramid: usize = tops.iter().map(Vec::len).sum();
        let stats = BuildStats {
            triangles,
            cells,
            entries: entries.len(),
            nonempty_cells: nonempty,
            max_list,
            mean_cells_per_triangle: entries.len() as f64 / triangles as f64,
            levels: levels.len(),
            build_ms: started.elapsed().as_secs_f64() * 1e3,
            bytes: 4 * cells + 4 * ENTRY_WORDS * entries.len() + 4 * pyramid,
        };
        Self { frame, levels, tops, ends, entries, stats }
    }

    fn list(&self, cell: usize) -> &[Entry] {
        let begin = if cell == 0 { 0 } else { self.ends[cell - 1] };
        &self.entries[begin as usize..self.ends[cell] as usize]
    }
}

/// Per-ray work of one grid query: pyramid cells read, fine lists opened, list entries
/// read, and triangles tested.
#[derive(Clone, Copy, Debug, Default)]
pub struct TraceStats {
    pub lookups: u32,
    pub lists: u32,
    pub entries: u32,
    pub tests: u32,
}

/// 1 / d with the watertight test's tiny-component rule, so a zero component gives a
/// signed huge inverse instead of a NaN slab.
fn inverse(d: Vec2) -> Vec2 {
    let tiny = Vec2::select(d.cmpge(Vec2::ZERO), Vec2::splat(1e-30), Vec2::splat(-1e-30));
    Vec2::ONE / Vec2::select(d.abs().cmplt(Vec2::splat(1e-30)), tiny, d)
}

impl Grid {
    /// Whether some listed triangle that `test` accepts can block the ray, walking the
    /// pyramid from level `start` (clamped to the coarsest). None when the ray cannot be
    /// walked (not rising toward the grid's light, or MAX_STEPS): the caller falls back to
    /// the BVH. The arithmetic is the WGSL's, in f32.
    pub fn trace(
        &self,
        origin: Vec3,
        direction: Vec3,
        tmin: f32,
        tmax: f32,
        start: usize,
        stats: &mut TraceStats,
        mut test: impl FnMut(u32) -> bool,
    ) -> Option<bool> {
        let f = &self.frame;
        let p = Vec2::new(origin.dot(f.u), origin.dot(f.v)) - f.origin;
        let z0 = origin.dot(f.l);
        let d = Vec2::new(direction.dot(f.u), direction.dot(f.v));
        let dz = direction.dot(f.l);
        if !(dz > 0.0) {
            return None;
        }
        let inv = inverse(d);
        let top = self.levels.len() - 1;
        let size = Vec2::new(f.dims[0] as f32, f.dims[1] as f32) * f.cell;
        let (a, c) = (-p * inv, (size - p) * inv);
        let mut t = a.min(c).max_element().max(tmin);
        let end = a.max(c).min_element().min(tmax).min((self.tops[top][0] - z0) / dz);
        if !(t <= end) {
            return Some(false);
        }
        let n = [f.dims[0] as i64, f.dims[1] as i64];
        let s = [if d.x >= 0.0 { 1i64 } else { -1 }, if d.y >= 0.0 { 1 } else { -1 }];
        let inv_cell = 1.0 / f.cell;
        let fine = |t: f32, a: usize| -> i64 {
            let x = (d[a].mul_add(t, p[a]) * inv_cell).floor();
            x.clamp(0.0, (n[a] - 1) as f32) as i64
        };
        // Never step back along the ray: positions only move with the ray's signs.
        let ahead = |x: i64, at: i64, a: usize| if s[a] > 0 { x.max(at) } else { x.min(at) };
        let mut at = [fine(t, 0), fine(t, 1)];
        let mut level = start.min(top);
        for _ in 0..MAX_STEPS {
            let [w, _] = self.levels[level];
            let cell = [at[0] >> level, at[1] >> level];
            stats.lookups += 1;
            let h = self.tops[level][(cell[0] + cell[1] * i64::from(w)) as usize];
            let rise = t.mul_add(dz, z0);
            if h >= rise && level > 0 {
                for a in 0..2 {
                    let first = cell[a] << level;
                    let last = (((cell[a] + 1) << level) - 1).min(n[a] - 1);
                    at[a] = ahead(fine(t, a), at[a], a).clamp(first, last);
                }
                level -= 1;
                continue;
            }
            if h >= rise {
                stats.lists += 1;
                for e in self.list((at[0] + at[1] * n[0]) as usize) {
                    stats.entries += 1;
                    if e.top < rise {
                        break;
                    }
                    // The ray's part over the entry's box must pass below its top.
                    let (lo, hi) = dequantize(f, at[0], at[1], e.bbox);
                    let (a, c) = ((lo - p) * inv, (hi - p) * inv);
                    let near = a.min(c).max_element().max(t);
                    let far = a.max(c).min_element().min(end);
                    if near > far || e.top < near.mul_add(dz, z0) {
                        continue;
                    }
                    stats.tests += 1;
                    if test(e.id) {
                        return Some(true);
                    }
                }
            }
            // Leave this level's cell through its nearer exit face.
            let side = f.cell * (1u32 << level) as f32;
            let exit = |a: usize| ((cell[a] + i64::from(s[a] > 0)) as f32 * side - p[a]) * inv[a];
            let (ex, ey) = (exit(0), exit(1));
            let leave = ex.min(ey);
            if leave >= end {
                return Some(false);
            }
            t = t.max(leave);
            let axis = usize::from(ey < ex);
            let next = cell[axis] + s[axis];
            if next < 0 || next >= i64::from(self.levels[level][axis]) {
                return Some(false);
            }
            at[axis] = if s[axis] > 0 { next << level } else { (((next + 1) << level) - 1).min(n[axis] - 1) };
            let o = 1 - axis;
            let first = cell[o] << level;
            let last = (((cell[o] + 1) << level) - 1).min(n[o] - 1);
            at[o] = ahead(fine(t, o), at[o], o).clamp(first, last);
            level = (level + 1).min(top);
        }
        None
    }
}

/// N0's traced soft module with the shadow rays' software any-hit replaced by the grid
/// query, through N0's own seam (the same receivers, ε, directions and light-box tmax).
/// `constants` are N0's, then the grid's.
pub fn traced_source(soft: &str, constants: &str, grid: &str) -> String {
    let source = bvh::traced_source(soft, constants);
    assert_eq!(source.matches(OCCLUDED).count(), 1, "N0 shadow-ray seam");
    let source = source.replacen(OCCLUDED, GRID_OCCLUDED, 1);
    format!("{source}\n{grid}{}", QUERY_WGSL.split_once(BUILD).unwrap().0)
}
/// The GPU build module (count, scan, scatter, sort, pyramid) after the grid constants.
pub fn build_source(grid: &str) -> String {
    let (query, build) = QUERY_WGSL.split_once(BUILD).unwrap();
    // The build shares the parameter block at the head of the query half.
    let head = query.split_once("// ---- query ----").unwrap().0;
    format!("enable wgpu_binding_array;\n{grid}{head}{build}")
}
/// Module constants of the grid WGSL: heap chunks (binding-array lengths) and meshes.
pub fn constants(front_chunks: usize, entry_chunks: usize, meshes: usize) -> String {
    format!(
        "const LIGHT_GRID_FRONT_CHUNKS:u32={front_chunks}u;\nconst LIGHT_GRID_ENTRY_CHUNKS:u32={entry_chunks}u;\nconst LIGHT_GRID_MESHES:u32={meshes}u;\n"
    )
}

/// Words per heap chunk: 64 MiB bindings, under the 128 MiB storage-binding limit.
pub const CHUNK_WORDS: usize = 1 << 24;

/// Word offsets of the front heap's sections (list ends, pyramid, scan scratch) and the
/// entry capacity. Entries are ENTRY_WORDS words each in their own heap.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Layout {
    pub ends: u32,
    pub tops: u32,
    pub sums: u32,
    pub front_words: u32,
    pub capacity: u32,
}
impl Layout {
    pub fn new(frame: &Frame, capacity: u32) -> Self {
        let cells = frame.dims[0] * frame.dims[1];
        let pyramid: u32 = frame.levels().iter().map(|[x, y]| x * y).sum();
        let (ends, tops) = (0, cells);
        let sums = tops + pyramid;
        // Block totals, the grand total, then the overflow flag.
        let front_words = sums + cells.div_ceil(1024) + 2;
        Self { ends, tops, sums, front_words, capacity }
    }
    pub fn front_chunks(&self) -> usize {
        (self.front_words as usize).div_ceil(CHUNK_WORDS)
    }
    pub fn entry_chunks(&self) -> usize {
        (ENTRY_WORDS * self.capacity as usize).div_ceil(CHUNK_WORDS).max(1)
    }
    /// Word of the scan's grand total; the overflow flag follows it.
    pub fn total(&self, frame: &Frame) -> u32 {
        self.sums + (frame.dims[0] * frame.dims[1]).div_ceil(1024)
    }
}

/// One placement as the GPU build reads it: first global triangle, count, margin, mesh.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct GpuPlacement {
    pub first: u32,
    pub count: u32,
    pub margin: f32,
    pub mesh: u32,
}

/// The parameter block (`LightGrid` in the WGSL), little-endian.
pub fn params(frame: &Frame, layout: &Layout, start: u32, placements: &[GpuPlacement]) -> Vec<u8> {
    let levels = frame.levels();
    assert!(levels.len() <= 32 && placements.len() <= 16 && !placements.is_empty());
    let mut words: Vec<u32> = vec![];
    for (axis, w) in [(frame.u, frame.cell), (frame.v, 1.0 / frame.cell), (frame.l, frame.quantum())] {
        words.extend(axis.extend(w).to_array().map(f32::to_bits));
    }
    words.extend([frame.origin.x, frame.origin.y, 0.0, 0.0].map(f32::to_bits));
    words.extend([frame.dims[0], frame.dims[1], levels.len() as u32 - 1, start]);
    words.extend([layout.ends, layout.tops, layout.sums, 0]);
    let triangles = placements.last().map_or(0, |p| p.first + p.count);
    words.extend([layout.capacity, 0, placements.len() as u32, triangles]);
    let mut offset = 0;
    for k in 0..32 {
        words.push(offset);
        if let Some([x, y]) = levels.get(k) {
            offset += x * y;
        }
    }
    for k in 0..16 {
        let p = placements.get(k).copied().unwrap_or(GpuPlacement { first: u32::MAX, count: 0, margin: 0.0, mesh: 0 });
        words.extend([p.first, p.count, p.margin.to_bits(), p.mesh]);
    }
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
