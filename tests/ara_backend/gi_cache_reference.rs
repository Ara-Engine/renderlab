//! CPU reference for the radiance cache's table: the layout `shaders/gi_cells.wgsl`
//! documents, each cell's 64 visibility rays as `gi_ray` forms them, and the occluded
//! sky's SH as `gi_finish` sums and `gi_store_sh` stores it under a constant sky.
//!
//! The rays are built in f32 like the kernel's, so a reference ray is the kernel's ray
//! to rounding; what they hit is the caller's scene, in f64.
use anyhow::{Result, ensure};
use glam::{DVec3, Vec3};

const HEADER: usize = 64;
const HOT: usize = 24;
/// `GI_DIRECTIONS` at its default.
pub const DIRECTIONS: u32 = 64;
/// State bits: the value is current; no canonical query point; directions untraced.
const TRACED: u32 = 1;
const ORPHAN: u32 = 2;
const PARTIAL: u32 = 4;

/// A whole table as `Renderer::indirect_table` returns it.
pub struct Table<'a> {
    pub words: &'a [u32],
    pub slots: usize,
    cold: usize,
}
/// One occupied slot, decoded.
pub struct Cell {
    pub slot: usize,
    /// The SH is current and the cell has a query point.
    pub valid: bool,
    /// Traced on every direction, from its own query point.
    pub settled: bool,
    pub key_b: u32,
    pub level: u32,
    pub normal: Vec3,
    pub point: Vec3,
    /// One bit per fixed direction: set where the sky is visible.
    pub bits: u64,
    /// The SH's f32 scale, and its 27 halves (coefficient i * 3 + channel) over it.
    pub scale: f32,
    pub sh: [f32; 27],
}

impl<'a> Table<'a> {
    pub fn new(words: &'a [u32]) -> Result<Self> {
        ensure!(words.len() > HEADER, "table is shorter than its header");
        let slots = words[1] as usize + 1;
        let cold = (words[23] as usize).max(24);
        ensure!(
            slots.is_power_of_two() && words.len() == HEADER + slots * (HOT + cold),
            "table length does not match its header"
        );
        Ok(Self { words, slots, cold })
    }
    fn f32(&self, at: usize) -> f32 {
        f32::from_bits(self.words[at])
    }
    /// Every slot whose key word is set.
    pub fn cells(&self) -> Vec<Cell> {
        (0..self.slots)
            .filter_map(|slot| {
                let hot = HEADER + slot * HOT;
                let cold = HEADER + self.slots * HOT + slot * self.cold;
                if self.words[hot] == 0 {
                    return None;
                }
                let state = self.words[cold + 1];
                let mut sh = [0f32; 27];
                for (j, value) in sh.iter_mut().enumerate() {
                    let pair = self.words[hot + 2 + j / 2];
                    let half = if j % 2 == 0 {
                        pair as u16
                    } else {
                        (pair >> 16) as u16
                    };
                    *value = half::f16::from_bits(half).to_f32();
                }
                Some(Cell {
                    slot,
                    valid: self.words[hot] & 0x8000_0000 != 0,
                    settled: state & (TRACED | ORPHAN | PARTIAL) == TRACED
                        && self.words[cold + 14] & self.words[cold + 15] == u32::MAX,
                    key_b: self.words[cold],
                    level: self.words[cold + 3] & 0xff,
                    normal: unpack_normal(self.words[cold + 7]),
                    point: Vec3::new(self.f32(cold + 8), self.f32(cold + 9), self.f32(cold + 10)),
                    bits: u64::from(self.words[cold + 12])
                        | (u64::from(self.words[cold + 13]) << 32),
                    scale: self.f32(hot + 1),
                    sh,
                })
            })
            .collect()
    }
    /// The header's level-0 edge in metres, and whether directions rotate per cell.
    pub fn min_edge(&self) -> f32 {
        self.f32(5)
    }
    pub fn rotates(&self) -> bool {
        self.words[2] & 4 != 0
    }
}

/// `gi_unpack_normal`: the octahedral normal of a `pack2x16snorm` word.
fn unpack_normal(w: u32) -> Vec3 {
    let snorm = |v: u16| (f32::from(v as i16) / 32767.).max(-1.);
    let (x, y) = (snorm(w as u16), snorm((w >> 16) as u16));
    let z = 1. - x.abs() - y.abs();
    let t = (-z).max(0.);
    Vec3::new(
        x + if x >= 0. { -t } else { t },
        y + if y >= 0. { -t } else { t },
        z,
    )
    .normalize()
}

/// `gi_basis` (Duff et al. 2017) applied to a local direction.
fn to_world(n: Vec3, local: Vec3) -> Vec3 {
    let s = if n.z >= 0. { 1. } else { -1. };
    let a = -1. / (s + n.z);
    let b = n.x * n.y * a;
    Vec3::new(1. + s * n.x * n.x * a, s * b, -s * n.x) * local.x
        + Vec3::new(b, s + n.y * n.y * a, -n.y) * local.y
        + n * local.z
}

/// `gi_direction`: direction `k` of the cell's Fibonacci hemisphere about its normal.
pub fn direction(cell: &Cell, rotates: bool, k: u32) -> Vec3 {
    let z = 1. - (k as f32 + 0.5) / DIRECTIONS as f32;
    let r = (1. - z * z).max(0.).sqrt();
    let rotation = if rotates {
        (cell.key_b >> 8) as f32 * (std::f32::consts::TAU / 16_777_216.)
    } else {
        0.
    };
    let phi = k as f32 * 2.399_963_3 + rotation;
    to_world(cell.normal, Vec3::new(r * phi.cos(), r * phi.sin(), z))
}

/// `gi_ray`'s origin: the query point lifted off its surface by `gi_origin_offset`.
pub fn origin(cell: &Cell, min_edge: f32) -> DVec3 {
    let q = cell.point;
    let edge = min_edge * (cell.level as f32).exp2();
    let m = q.abs().max_element().max(1e-30);
    let ulp = f32::from_bits(m.to_bits() + 1) - m;
    let offset = ((1e-5f32).max(32. * ulp) + 1e-4 * edge).max(1e-4 + 4e-7 * m);
    (q + cell.normal * offset).as_dvec3()
}

/// The SH sums of unit radiance over the directions `bits` leaves open, as `gi_finish`
/// forms them: nine coefficients, equal in every channel under a constant sky.
pub fn sky_sh(cell: &Cell, rotates: bool, bits: u64) -> [f64; 9] {
    let mut sh = [0f64; 9];
    for k in (0..DIRECTIONS).filter(|k| bits >> k & 1 != 0) {
        let d = direction(cell, rotates, k).as_dvec3();
        let y = [
            0.282095,
            0.488603 * d.y,
            0.488603 * d.z,
            0.488603 * d.x,
            1.092548 * d.x * d.y,
            1.092548 * d.y * d.z,
            0.315392 * (3. * d.z * d.z - 1.),
            1.092548 * d.x * d.z,
            0.546274 * (d.x * d.x - d.y * d.y),
        ];
        for (sum, y) in sh.iter_mut().zip(y) {
            *sum += std::f64::consts::TAU / f64::from(DIRECTIONS) * y;
        }
    }
    sh
}
