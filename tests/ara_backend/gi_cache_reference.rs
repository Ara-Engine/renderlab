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
    /// The six-way normal bin, and the cell's integer lattice coordinates at its level.
    pub bin: u32,
    pub lattice: [i32; 3],
    /// The hot record's split flag.
    pub split: bool,
    pub normal: Vec3,
    pub point: Vec3,
    /// One bit per fixed direction: set where the sky is visible.
    pub bits: u64,
    /// The SH's f32 scale, and its 27 halves (coefficient i * 3 + channel) over it.
    pub scale: f32,
    pub sh: [f32; 27],
    /// Bounce: the L1 SH's f32 scale (0 before the cell's first gather, -0.0 after a
    /// gather that found no light) and its 12 halves (coefficient i * 3 + channel).
    pub bounce_scale: f32,
    pub bounce: [f32; 12],
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
                let mut bounce = [0f32; 12];
                for (j, value) in bounce.iter_mut().enumerate() {
                    let pair = self.words[hot + 17 + j / 2];
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
                    bin: self.words[cold + 3] >> 8,
                    lattice: std::array::from_fn(|i| self.words[cold + 4 + i] as i32),
                    split: self.words[hot + 15] & 0x1_0000 != 0,
                    normal: unpack_normal(self.words[cold + 7]),
                    point: Vec3::new(self.f32(cold + 8), self.f32(cold + 9), self.f32(cold + 10)),
                    bits: u64::from(self.words[cold + 12])
                        | (u64::from(self.words[cold + 13]) << 32),
                    scale: self.f32(hot + 1),
                    sh,
                    bounce_scale: self.f32(hot + 16),
                    bounce,
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
    /// The cell filter's bits (sky, bounce, contrast refine, fill) and its tent's edge
    /// weight.
    pub fn filter(&self) -> (u32, f32) {
        (self.words[24], self.f32(25))
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

/// `gi_cell_bounce_irradiance`: the bounce light arriving about the cell's own normal,
/// from its L1 SH through the clamped-cosine band weights.
pub fn bounce_irradiance(cell: &Cell) -> [f64; 3] {
    if cell.bounce_scale <= 0. {
        return [0.; 3];
    }
    let n = cell.normal.as_dvec3();
    let y = [0.282095, 0.488603 * n.y, 0.488603 * n.z, 0.488603 * n.x];
    let weight = [
        std::f64::consts::PI,
        2.0943951023931953,
        2.0943951023931953,
        2.0943951023931953,
    ];
    std::array::from_fn(|c| {
        (0..4)
            .map(|i| weight[i] * y[i] * f64::from(cell.bounce[i * 3 + c]))
            .sum::<f64>()
            .max(0.)
            * f64::from(cell.bounce_scale)
    })
}

/// The cell filter's sources (`gi_raw`): hot words 1 to 22 of every slot as they were
/// written before filtering.
pub struct Raw<'a>(pub &'a [u32]);
impl Raw<'_> {
    fn word(&self, slot: usize, hot: usize) -> u32 {
        self.0[slot * 22 + hot - 1]
    }
    fn halves(&self, slot: usize, first: usize, count: usize) -> Vec<f64> {
        (0..count)
            .map(|j| {
                let pair = self.word(slot, first + j / 2);
                let half = if j % 2 == 0 {
                    pair as u16
                } else {
                    (pair >> 16) as u16
                };
                f64::from(half::f16::from_bits(half).to_f32())
            })
            .collect()
    }
    /// The unfiltered sky SH: 27 coefficients in cached units.
    pub fn sky(&self, slot: usize) -> Vec<f64> {
        let scale = f64::from(f32::from_bits(self.word(slot, 1)));
        self.halves(slot, 2, 27).iter().map(|h| h * scale).collect()
    }
    /// The unfiltered bounce SH, or None for a cell not gathered yet. A gather that
    /// found no light (-0.0) is twelve zeros.
    pub fn bounce(&self, slot: usize) -> Option<Vec<f64>> {
        let word = self.word(slot, 16);
        let scale = f64::from(f32::from_bits(word).max(0.));
        (word != 0).then(|| {
            self.halves(slot, 17, 12)
                .iter()
                .map(|h| h * scale)
                .collect()
        })
    }
    /// Whether the unfiltered value carried the split flag.
    fn split(&self, slot: usize) -> bool {
        self.word(slot, 15) & 0x1_0000 != 0
    }
}

/// What `gi_filter_cell` leaves in a cell's hot record: the filtered sky SH, the
/// filtered bounce SH (None where the filter leaves the bounce words alone), and the
/// split flag.
pub struct Filtered {
    pub sky: Vec<f64>,
    pub bounce: Option<Vec<f64>>,
    pub split: bool,
    /// Neighbours that took part, the cell itself aside.
    pub neighbours: usize,
}

/// `gi_filter_cell` under a constant sky: the weighted mean of the unfiltered values of
/// the cell and the eight cells around it in its bin's tangent plane at its level.
/// `cells` are the table's; `find` maps (level, bin, lattice) to an index into them.
pub fn filter(
    table: &Table,
    raw: &Raw,
    cells: &[Cell],
    find: &std::collections::HashMap<(u32, u32, [i32; 3]), usize>,
    cell: &Cell,
) -> Filtered {
    let (bits, e) = table.filter();
    let (sky_on, bounce_on, contrast_on, fill) =
        (bits & 1 != 0, bits & 2 != 0, bits & 5 == 5, bits & 8 != 0);
    let edge = table.min_edge() * (cell.level as f32).exp2();
    let axis = (cell.bin >> 1) as usize;
    let (q, n) = (cell.point, cell.normal);
    // The share of the open sky's irradiance about n that a slot's unfiltered value holds.
    let y = [
        0.282095,
        0.488603 * n.y,
        0.488603 * n.z,
        0.488603 * n.x,
        1.092548 * n.x * n.y,
        1.092548 * n.y * n.z,
        0.315392 * (3. * n.z * n.z - 1.),
        1.092548 * n.x * n.z,
        0.546274 * (n.x * n.x - n.y * n.y),
    ]
    .map(f64::from);
    let band = |i: usize| match i {
        0 => std::f64::consts::PI,
        1..=3 => 2.0943951023931953,
        _ => std::f64::consts::FRAC_PI_4,
    };
    let openness = |slot: usize| {
        let sky = raw.sky(slot);
        (0..27)
            .map(|j| band(j / 3) * y[j / 3] * sky[j])
            .sum::<f64>()
            / (3. * std::f64::consts::PI)
    };
    let own = openness(cell.slot);
    let gathered = raw.bounce(cell.slot).is_some();
    let filter_bounce = bounce_on && (gathered || fill);
    let (mut sky, mut bounce) = (vec![0f64; 27], vec![0f64; 12]);
    let (mut ws, mut wb, mut contrast, mut neighbours) = (0f64, 0f64, false, 0);
    for j in 0..9 {
        let (du, dv) = ((j % 3) as f32 - 1., (j / 3) as f32 - 1.);
        let (slot, w) = if j == 4 {
            (cell.slot, 1.)
        } else {
            let mut p = q;
            p[(axis + 1) % 3] += du * edge;
            p[(axis + 2) % 3] += dv * edge;
            if n[axis].abs() > 0.3 {
                p[axis] -= ((p - q).dot(n) / n[axis]).clamp(-1.5 * edge, 1.5 * edge);
            }
            let mut u = p / edge;
            u[axis] -= if cell.bin & 1 == 0 { 0.25 } else { -0.25 };
            let lattice = u.floor().as_ivec3().to_array();
            let Some(other) = find
                .get(&(cell.level, cell.bin, lattice))
                .map(|i| &cells[*i])
            else {
                continue;
            };
            if !other.valid {
                continue;
            }
            let tent = if du != 0. && dv != 0. { e * e } else { e };
            let d = other.normal.dot(n);
            let t = ((d - 0.5) / 0.4).clamp(0., 1.);
            let agree = t * t * (3. - 2. * t);
            let plane = (1.5 - 3. * (other.point - q).dot(n).abs() / edge).clamp(0., 1.);
            let w = f64::from(tent * agree * plane);
            if w <= 0. {
                continue;
            }
            if contrast_on && agree * plane >= 0.5 {
                let o = openness(other.slot);
                contrast |= o > 0.25 && own < 0.25 * o;
            }
            neighbours += 1;
            (other.slot, w)
        };
        if sky_on {
            for (sum, v) in sky.iter_mut().zip(raw.sky(slot)) {
                *sum += w * v;
            }
            ws += w;
        }
        if filter_bounce && let Some(values) = raw.bounce(slot) {
            for (sum, v) in bounce.iter_mut().zip(values) {
                *sum += w * v;
            }
            wb += w;
        }
    }
    Filtered {
        sky: if sky_on && ws > 0. {
            sky.iter().map(|v| v / ws).collect()
        } else {
            raw.sky(cell.slot)
        },
        bounce: (filter_bounce && wb > 0.).then(|| bounce.iter().map(|v| v / wb).collect()),
        split: raw.split(cell.slot) || (sky_on && ws > 0. && contrast),
        neighbours,
    }
}
