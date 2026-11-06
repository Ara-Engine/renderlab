//! Test-only world-space sun-visibility cache for non-RT shadows: unit N7a of
//! docs/non-rt-n7.md.
//!
//! Cells are the sub-triangles of each receiver triangle's 2^k barycentric subdivision,
//! sampled at their centroids, so a cell's value does not depend on the camera. A pixel
//! chooses k from its footprint and reads the cell under its hit barycentrics. Each cell
//! owns a jittered, rotated square direction lattice in a gnomonic chart fixed in the world;
//! the lattice points inside the current sun cone are its rays (8 on average). A cell keeps
//! a 4x4 window of lattice points (blocked and valid bits), which slides as the sun moves,
//! so only points entering the cone are traced.
//!
//! `shaders/visibility_cache.wgsl` holds the GPU table, lookup, fill, resolve and validation
//! kernels, appended to N0's traced module. This file is their CPU mirror (keys, cells,
//! lattice, window and shift, in the same f32 arithmetic where it decides a key or a bit),
//! the parameter block, and the composition.
use crate::software_bvh as bvh;
use glam::{IVec2, Vec2, Vec3};

pub const WGSL: &str = include_str!("../shaders/visibility_cache.wgsl");
/// Finest subdivision level: 14 bits of i and j in key b.
pub const MAX_LEVEL: u32 = 14;
/// Words per table slot: key a, key b, bits (blocked | valid << 16), meta.
pub const SLOT_WORDS: usize = 4;
/// Key a of a claim in flight.
pub const LOCK: u32 = 1;
/// Mean lattice points inside the sun disk: Interactive's ray count.
pub const RAYS: f32 = 8.0;

/// Word offsets of the work buffer (`cache_work`): counters, indirect arguments, the
/// mismatch dump, then the primary queue, the prefetch queue and the pending (pixel, slot)
/// list.
pub mod work {
    pub const MISMATCH: usize = 14;
    pub const CHECKED: usize = 15;
    pub const FILL_ARGS: usize = 24;
    /// Resolve arguments follow the fill arguments: the probe copies both in one 32-byte copy.
    pub const RESOLVE_ARGS: usize = 28;
    const _: () = assert!(RESOLVE_ARGS == FILL_ARGS + 4);
    pub const DUMP: usize = 32;
    pub const QUEUES: usize = 40;
    /// Counter names in word order, for summaries.
    pub const NAMES: [&str; 21] = [
        "pending", "queued", "queued_prefetch", "probe_limit", "busy", "filled", "rays",
        "fallback", "mover_rays", "warm", "incomplete", "inserted", "fill_primary",
        "fill_prefetch", "mismatch", "checked", "substituted", "ring_claimed", "partial_used",
        "resolve_found", "prefetch_busy",
    ];
    /// Words for `queue` entries per region and `pending` pixels.
    pub fn words(queue: u32, pending: u32) -> usize {
        QUEUES + 2 * queue as usize + 2 * pending as usize
    }
}
/// `CacheParams.frame.w` bits (CACHE_FLAG_* in the WGSL).
pub mod flags {
    /// Prefetch the sub-triangles sharing an edge with an unusable cell.
    pub const SUBDIVISION: u32 = 1;
    /// Prefetch the whole-triangle cells across an unusable whole-triangle cell's edges.
    pub const RING: u32 = 2;
    /// A pixel whose own cell stays unusable shows a usable whole-triangle neighbour's value.
    pub const SUBSTITUTE: u32 = 4;
    /// Cells missing at most one ray in the cone stay usable.
    pub const PARTIAL: u32 = 8;
    pub const ALL: u32 = SUBDIVISION | RING | SUBSTITUTE | PARTIAL;
}

pub fn fresh_hash(input: u32) -> u32 {
    let mut x = input;
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb_352d);
    x = (x ^ (x >> 15)).wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// A cell key, unpacked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub placement: u32,
    pub primitive: u32,
    pub level: u32,
    pub i: u32,
    pub j: u32,
    pub upper: u32,
    pub side: u32,
}
impl Key {
    pub fn a(&self) -> u32 {
        assert!(self.primitive < 1 << 22 && self.placement < 63 && self.level <= MAX_LEVEL);
        self.primitive | (self.placement + 1) << 22 | self.level << 28
    }
    pub fn b(&self) -> u32 {
        1 + (self.j | self.i << 14 | self.upper << 28 | self.side << 29)
    }
    pub fn unpack(a: u32, b: u32) -> Self {
        let k = b - 1;
        Self {
            placement: ((a >> 22) & 63) - 1,
            primitive: a & 0x3f_ffff,
            level: a >> 28,
            i: (k >> 14) & 0x3fff,
            j: k & 0x3fff,
            upper: (k >> 28) & 1,
            side: (k >> 29) & 1,
        }
    }
    /// Barycentrics (b1, b2) of the cell's centroid, as cache_receiver computes them.
    pub fn centroid(&self) -> Vec2 {
        let n = (1u32 << self.level) as f32;
        let corner = Vec2::splat(if self.upper != 0 { 2.0 / 3.0 } else { 1.0 / 3.0 });
        (Vec2::new(self.i as f32, self.j as f32) + corner) / n
    }
}

/// The cell (i, j, upper) of hit barycentrics (b1, b2) at `level` (cache_cell).
pub fn cell(bary: Vec2, level: u32) -> [u32; 3] {
    let n = (1u32 << level) as f32;
    let last = (1u32 << level) - 1;
    let x = bary * n;
    let i = (x.x.floor().max(0.0) as u32).min(last);
    let mut j = (x.y.floor().max(0.0) as u32).min(last);
    let mut upper = u32::from((x.x - i as f32) + (x.y - j as f32) > 1.0);
    if i + j > last {
        j = last - i;
        upper = 0;
    }
    if i + j == last {
        upper = 0;
    }
    [i, j, upper]
}
/// The level whose cells have about one footprint of area (cache_level).
pub fn level(area: f32, footprint: f32) -> u32 {
    let k = (area.sqrt().max(1e-30) / footprint.max(1e-30)).log2().round();
    k.clamp(0.0, MAX_LEVEL as f32) as u32
}
/// How close `level`'s decision is to its rounding boundary, in log2 units: GPU and CPU
/// may round differently within a few ulps of it.
pub fn level_margin(area: f32, footprint: f32) -> f32 {
    let x = (area.sqrt().max(1e-30) / footprint.max(1e-30)).log2();
    ((x - x.floor()) - 0.5).abs()
}

/// A cell's direction lattice: offset in [0, 1)^2 and rotation (cos, sin), from its key.
#[derive(Clone, Copy, Debug)]
pub struct Lattice {
    pub offset: Vec2,
    pub c: f32,
    pub s: f32,
}
pub fn lattice(a: u32, b: u32) -> Lattice {
    let h1 = fresh_hash(a ^ fresh_hash(b));
    let h2 = fresh_hash(h1 ^ 0xa511_e9b3);
    let h3 = fresh_hash(h2 ^ 0x63d8_3595);
    let offset = Vec2::new((h1 >> 8) as f32, (h2 >> 8) as f32) * (1.0 / 16_777_216.0);
    let angle = 6.283_185_5 * (h3 >> 8) as f32 * (1.0 / 16_777_216.0);
    Lattice { offset, c: angle.cos(), s: angle.sin() }
}

/// The world direction chart: a gnomonic plane at the anchor sun `l`, its axes, and the
/// lattice spacing that puts RAYS points inside a cone of half-angle `alpha` on average.
#[derive(Clone, Copy, Debug)]
pub struct Chart {
    pub l: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub spacing: f32,
    pub alpha: f32,
}
impl Chart {
    pub fn new(l: Vec3, alpha: f32) -> Self {
        let l64 = l.as_dvec3().normalize();
        let helper = if l64.x.abs() < 0.9 { glam::DVec3::X } else { glam::DVec3::Y };
        let u = l64.cross(helper).normalize();
        let v = l64.cross(u);
        let tan = f64::from(alpha).tan();
        let spacing = (std::f64::consts::PI * tan * tan / f64::from(RAYS)).sqrt();
        Self { l: l64.as_vec3(), u: u.as_vec3(), v: v.as_vec3(), spacing: spacing as f32, alpha }
    }
    /// The sun's chart point, in chart units (cache_centre before the cell's transform).
    pub fn chart(&self, sun: Vec3) -> Vec2 {
        Vec2::new(sun.dot(self.u), sun.dot(self.v)) / sun.dot(self.l)
    }
    /// Window radius in chart units: tan(alpha) widened by the gnomonic stretch at the
    /// sun's chart point, with 2% margin. The window must stay within four spacings.
    pub fn window_radius(&self, sun: Vec3) -> f32 {
        let c = self.chart(sun);
        let r = self.alpha.tan() * (1.0 + c.length_squared()) * 1.02;
        assert!(2.0 * r / self.spacing < 4.0, "the sun moved too far from the chart anchor");
        r
    }
    /// The sun in a cell's lattice coordinates (cache_centre).
    pub fn centre(&self, t: &Lattice, sun: Vec3) -> Vec2 {
        let p = self.u.dot(sun);
        let q = self.v.dot(sun);
        let c = Vec2::new(p, q) / (sun.dot(self.l) * self.spacing);
        Vec2::new(t.c * c.x + t.s * c.y, -t.s * c.x + t.c * c.y) - t.offset
    }
    pub fn window(&self, t: &Lattice, sun: Vec3) -> IVec2 {
        let r = self.window_radius(sun) / self.spacing;
        (self.centre(t, sun) - Vec2::splat(r)).ceil().as_ivec2()
    }
    /// World direction of a cell's lattice point at absolute indices q (cache_direction).
    pub fn direction(&self, t: &Lattice, q: IVec2) -> Vec3 {
        let x = (q.as_vec2() + t.offset) * self.spacing;
        let chart = Vec2::new(t.c * x.x - t.s * x.y, t.s * x.x + t.c * x.y);
        (self.l + chart.x * self.u + chart.y * self.v).normalize()
    }
    /// Window bits whose directions lie in the sun's cone (cache_disk).
    pub fn disk(&self, t: &Lattice, window: IVec2, sun: Vec3) -> u32 {
        let cos = self.alpha.cos();
        (0..16)
            .filter(|&b| self.direction(t, window + IVec2::new(b & 3, b >> 2)).dot(sun) > cos)
            .fold(0u32, |m, b| m | 1u32 << b)
    }
}

/// Window bits moved from window `from` to `to`; points leaving are dropped (cache_shift).
pub fn shift(bits: u32, from: IVec2, to: IVec2) -> u32 {
    let d = to - from;
    (0..16)
        .filter(|&b| bits & (1u32 << b) != 0)
        .filter_map(|b| {
            let (x, y) = ((b & 3) - d.x, (b >> 2) - d.y);
            ((0..4).contains(&x) && (0..4).contains(&y)).then(|| 1u32 << (x + 4 * y))
        })
        .fold(0u32, |m, b| m | b)
}
/// The window origin a meta word holds (cache_meta_window).
pub fn meta_window(meta: u32) -> IVec2 {
    IVec2::new(((meta << 8) as i32) >> 24, (meta as i32) >> 24)
}

/// One frame's parameter block (`CacheParams` in the WGSL), little-endian.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub chart: Chart,
    /// Unit direction toward the sun this frame.
    pub sun: Vec3,
    pub scene: bvh::Aabb,
    /// Radians per pixel: 2 tan(fov / 2) / height.
    pub pixel_angle: f32,
    /// Footprints per cell edge (1: about one pixel of area per cell).
    pub cell_scale: f32,
    /// Frame stamp, 1..=65535.
    pub stamp: u32,
    pub slots: u32,
    pub budget: u32,
    /// `flags` bits.
    pub flags: u32,
    pub queue: u32,
    pub pending: u32,
    pub probes: u32,
    pub movers: bool,
}
impl Params {
    pub fn bytes(&self) -> Vec<u8> {
        assert!(self.slots.is_power_of_two() && (1..=0xffff).contains(&self.stamp));
        let c = &self.chart;
        let floats = [
            c.u.extend(c.spacing),
            c.v.extend(c.window_radius(self.sun)),
            c.l.extend(0.0),
            self.sun.normalize().extend(c.alpha.cos()),
            self.scene.lo.extend(0.0),
            self.scene.hi.extend(0.0),
            glam::Vec4::new(self.pixel_angle, self.cell_scale, 0.0, 0.0),
        ];
        let mut words: Vec<u32> = floats.iter().flat_map(|v| v.to_array().map(f32::to_bits)).collect();
        words.extend([self.stamp, self.slots - 1, self.budget, self.flags]);
        words.extend([self.queue, self.pending, self.probes, u32::from(self.movers)]);
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// N0's traced module (every ray query replaced by software traversal) with the cache
/// kernels appended; `constants` are N0's.
pub fn traced_source(soft: &str, constants: &str) -> String {
    format!("{}\n{WGSL}", bvh::traced_source(soft, constants))
}
