//! Dominant environment lobes (M7-ES1). A prepared field's concentrated sources, such as a
//! sun and its aureole, are fitted at upload as spherical Gaussians
//! `G(l) = A exp(lambda (l.s - 1))`, and each lobe's share of the N=V prefiltered field is
//! tabulated per roughness knot. Shading programs evaluate the lobes against their own
//! BRDF and subtract that share (shaders/environment_lobes.wgsl), so the split-sum prefilter
//! carries only the smooth remainder of the sky, where its N=V assumption holds. See
//! docs/m7-es1.md for the measured accuracy.
use renderlab_asset::environment::{CubeFace, CubeLevel, FIELD_LEVELS, GGX_ALPHA_FLOOR};
use std::f64::consts::{PI, TAU};

/// Lobes per environment: one RGBA texel of the share table holds all of them.
pub(crate) const MAX_LOBES: usize = 4;
/// Share-table angle bins, uniform in sqrt(angle / pi), so they are finest at the axis.
pub(crate) const SHARE_BINS: usize = 128;
/// Table rows: lobe parameters, then one row per roughness knot.
pub(crate) const TABLE_ROWS: usize = 1 + FIELD_LEVELS;
/// Broader structure stays in the prefiltered field. Below this sharpness (a half width of
/// about 12 degrees) a point evaluation loses to the split sum on ES1's lobe grid.
const MIN_SHARPNESS: f64 = 32.0;
/// A lobe is kept only while it carries this fraction of the field's residual luminous flux.
const MIN_FLUX_FRACTION: f64 = 0.02;
/// The moment fit reads a cap where the lobe is above exp(-CAP) of its peak.
const CAP: f64 = 12.0;
/// Stratified quadrature points per axis for each share-table entry.
const STRATA: usize = 48;

/// One fitted lobe in source space (before the map's yaw).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lobe {
    /// Unit direction of the axis.
    pub direction: [f64; 3],
    pub sharpness: f64,
    /// Peak radiance above the field's per-channel minimum.
    pub amplitude: [f64; 3],
}
impl Lobe {
    /// The integral of `exp(lambda (l.s - 1))` over the sphere.
    pub fn solid_angle(&self) -> f64 {
        TAU / self.sharpness * -(-2.0 * self.sharpness).exp_m1()
    }
    fn shape(&self, d: [f64; 3]) -> f64 {
        (self.sharpness * (dot(d, self.direction) - 1.0)).exp()
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    a.map(|x| x / l)
}
fn luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
/// An orthonormal frame about `n`.
fn frame(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let up = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let t = normalize([
        up[1] * n[2] - up[2] * n[1],
        up[2] * n[0] - up[0] * n[2],
        up[0] * n[1] - up[1] * n[0],
    ]);
    let b = [
        n[1] * t[2] - n[2] * t[1],
        n[2] * t[0] - n[0] * t[2],
        n[0] * t[1] - n[1] * t[0],
    ];
    (t, b)
}
fn compose(t: [f64; 3], b: [f64; 3], n: [f64; 3], x: f64, y: f64, z: f64) -> [f64; 3] {
    std::array::from_fn(|i| t[i] * x + b[i] * y + n[i] * z)
}

/// Fits up to `MAX_LOBES` lobes to a delta-convention source level (RLE2 knot 0), greedily
/// and deterministically: the brightest residual texel seeds each lobe, von Mises-Fisher
/// moments within its cap give the axis and sharpness, and the amplitude matches the cap's
/// flux. Fitting stops at the first lobe that is too broad or too faint; a constant or
/// smooth field gets none.
pub(crate) fn fit(source: &CubeLevel) -> Vec<Lobe> {
    let size = source.size() as usize;
    let mut directions = Vec::with_capacity(source.texels().len());
    let mut omega = Vec::with_capacity(source.texels().len());
    let step = 2.0 / size as f64;
    for face in CubeFace::ALL {
        for y in 0..size {
            for x in 0..size {
                let u = (x as f64 + 0.5) * step - 1.0;
                let v = (y as f64 + 0.5) * step - 1.0;
                directions.push(face.direction([u, v]));
                omega.push(step * step / (1.0 + u * u + v * v).powf(1.5));
            }
        }
    }
    let mut floor = [f64::INFINITY; 3];
    for t in source.texels() {
        for c in 0..3 {
            floor[c] = floor[c].min(f64::from(t[c]));
        }
    }
    let mut residual: Vec<[f64; 3]> = source
        .texels()
        .iter()
        .map(|t| std::array::from_fn(|c| f64::from(t[c]) - floor[c]))
        .collect();
    let flux = |residual: &[[f64; 3]]| -> f64 {
        residual
            .iter()
            .zip(&omega)
            .map(|(r, w)| luminance(*r).max(0.0) * w)
            .sum()
    };
    let total = flux(&residual);
    let mut lobes = Vec::new();
    if total.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return lobes;
    }
    while lobes.len() < MAX_LOBES {
        let mut peak = 0;
        for i in 1..residual.len() {
            if luminance(residual[i]) > luminance(residual[peak]) {
                peak = i;
            }
        }
        let peak_luminance = luminance(residual[peak]);
        if peak_luminance <= 0.0 {
            break;
        }
        // Start from the half-maximum solid angle: 2 pi ln 2 / lambda for a vMF lobe.
        let half: f64 = (0..residual.len())
            .filter(|&i| luminance(residual[i]) >= 0.5 * peak_luminance)
            .filter(|&i| dot(directions[i], directions[peak]) > 0.0)
            .map(|i| omega[i])
            .sum();
        // A source below a texel's resolution cannot be sharper than the texel.
        let sharpest = 8.0 / omega[peak];
        let mut lobe = Lobe {
            direction: directions[peak],
            sharpness: (TAU * std::f64::consts::LN_2 / half).min(sharpest),
            amplitude: [0.0; 3],
        };
        for _ in 0..4 {
            let cap = 1.0 - CAP / lobe.sharpness;
            let mut mean = [0.0; 3];
            let mut weight = 0.0;
            for i in 0..residual.len() {
                let w = luminance(residual[i]).max(0.0) * omega[i];
                if w > 0.0 && dot(directions[i], lobe.direction) >= cap {
                    for k in 0..3 {
                        mean[k] += w * directions[i][k];
                    }
                    weight += w;
                }
            }
            if weight <= 0.0 {
                break;
            }
            lobe.direction = normalize(mean);
            let length = (dot(mean, mean)).sqrt() / weight;
            // The mean resultant length of a vMF lobe is coth(lambda) - 1/lambda.
            lobe.sharpness = (1.0 / (1.0 - length).max(1e-12)).min(sharpest);
        }
        // Flux within the cap, matched by the fitted shape's.
        let cap = 1.0 - CAP / lobe.sharpness;
        let mut energy = [0.0; 3];
        let mut shape = 0.0;
        for i in 0..residual.len() {
            if dot(directions[i], lobe.direction) >= cap {
                for c in 0..3 {
                    energy[c] += residual[i][c] * omega[i];
                }
                shape += lobe.shape(directions[i]) * omega[i];
            }
        }
        lobe.amplitude = energy.map(|e| (e / shape).max(0.0));
        let lobe_flux = luminance(lobe.amplitude) * lobe.solid_angle();
        if lobe.sharpness < MIN_SHARPNESS || lobe_flux < MIN_FLUX_FRACTION * total {
            break;
        }
        for (r, d) in residual.iter_mut().zip(&directions) {
            let g = lobe.shape(*d);
            for (value, amplitude) in r.iter_mut().zip(lobe.amplitude) {
                *value -= amplitude * g;
            }
        }
        lobes.push(lobe);
    }
    lobes
}

/// The normalization of the prefilter's N=V kernel `D(h) max(N.L,0) / 4`, as the cooker
/// computes it (crates/cook/src/environment/specular.rs).
fn kernel_integral(a: f64) -> f64 {
    if a > 0.9 {
        let x = (1.0 - a) / (1.0 + a);
        let mut power = 1.0;
        let mut sum = 0.0;
        for k in 0..16 {
            sum += f64::from(k + 1) / f64::from(k + 2) * power;
            power *= x;
        }
        2.0 * a / (1.0 + a).powi(2) * sum
    } else {
        (1.0 - a + 2.0 * a * (2.0 * a / (1.0 + a)).ln()) / (1.0 - a).powi(2)
    }
}

/// The angle of share-table bin `j` from the lobe's axis.
pub(crate) fn bin_angle(j: usize) -> f64 {
    let u = j as f64 / (SHARE_BINS - 1) as f64;
    PI * u * u
}

/// A unit-amplitude lobe's share of the N=V prefiltered field at reflection direction `r`
/// and perceptual roughness `roughness`: the normalized kernel's average of its shape. It
/// samples whichever of the kernel and the lobe is narrower, on a fixed stratified grid.
pub(crate) fn share(lobe: &Lobe, r: [f64; 3], roughness: f64) -> f64 {
    let alpha = (roughness * roughness).max(GGX_ALPHA_FLOOR);
    let a = alpha * alpha;
    if roughness == 0.0 {
        return lobe.shape(r);
    }
    let n = STRATA as f64;
    if a * lobe.sharpness < 0.5 {
        // Kernel samples: the half vector from the NDF's inverse CDF, then the reflection;
        // their density is D(h) / 4, so each weighs max(R.L, 0) (self-normalized).
        let (t, b) = frame(r);
        let (mut num, mut den) = (0.0, 0.0);
        for i in 0..STRATA {
            let v = (i as f64 + 0.5) / n;
            let ch = ((1.0 - v) / (1.0 + (a - 1.0) * v)).sqrt();
            let sh = (1.0 - ch * ch).max(0.0).sqrt();
            for k in 0..STRATA {
                let phi = TAU * (k as f64 + 0.5) / n;
                let h = compose(t, b, r, sh * phi.cos(), sh * phi.sin(), ch);
                let rh = dot(r, h);
                let l: [f64; 3] = std::array::from_fn(|c| 2.0 * rh * h[c] - r[c]);
                let w = dot(r, l).max(0.0);
                num += lobe.shape(l) * w;
                den += w;
            }
        }
        return if den > 0.0 { num / den } else { 0.0 };
    }
    // Lobe samples: stratified in the vMF's inverse CDF about its axis.
    let (t, b) = frame(lobe.direction);
    let lambda = lobe.sharpness;
    let mut sum = 0.0;
    for i in 0..STRATA {
        let u = (i as f64 + 0.5) / n;
        let w = 1.0 + (u + (1.0 - u) * (-2.0 * lambda).exp()).ln() / lambda;
        let sw = (1.0 - w * w).max(0.0).sqrt();
        for k in 0..STRATA {
            let phi = TAU * (k as f64 + 0.5) / n;
            let l = compose(t, b, lobe.direction, sw * phi.cos(), sw * phi.sin(), w);
            let c = dot(r, l);
            if c > 0.0 {
                let q = a + (1.0 - a) * (1.0 - c) * 0.5;
                sum += a / (4.0 * PI * q * q) * c;
            }
        }
    }
    sum * lobe.solid_angle() / (n * n) / kernel_integral(a)
}

/// The lobe table texture (Rgba32Float, `SHARE_BINS` x `TABLE_ROWS`), as tightly packed
/// texels. Row 0: texel k is lobe k's axis and sharpness, texel 4 + k its amplitude and
/// solid angle, texel 8 the lobe count. Row 1 + i: knot i's shares, one lobe per channel,
/// at `bin_angle(j)` for texel j.
pub(crate) fn table(lobes: &[Lobe], knots: &[f32; FIELD_LEVELS]) -> Vec<[f32; 4]> {
    let mut texels = vec![[0.0f32; 4]; SHARE_BINS * TABLE_ROWS];
    for (k, lobe) in lobes.iter().enumerate().take(MAX_LOBES) {
        let s = lobe.direction;
        texels[k] = [s[0] as f32, s[1] as f32, s[2] as f32, lobe.sharpness as f32];
        let a = lobe.amplitude;
        texels[4 + k] = [
            a[0] as f32,
            a[1] as f32,
            a[2] as f32,
            lobe.solid_angle() as f32,
        ];
        let (t, _) = frame(s);
        for (i, knot) in knots.iter().enumerate() {
            for j in 0..SHARE_BINS {
                let (sin, cos) = bin_angle(j).sin_cos();
                let r = compose(t, [0.0; 3], s, sin, 0.0, cos);
                texels[(1 + i) * SHARE_BINS + j][k] = share(lobe, r, f64::from(*knot)) as f32;
            }
        }
    }
    texels[8][0] = lobes.len().min(MAX_LOBES) as f32;
    texels
}
