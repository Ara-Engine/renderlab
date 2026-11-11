//! A procedural gradient sky as a prepared `Sky::Map`, for measuring the map's shading
//! cost without an embedder or an asset (docs/design-sky-map-cost.md). The gradient is Ara
//! Showcase's studio sky in Ara's sky-dome form (zenith blends into horizon by
//! `(1 - sin(elevation))^3`, the ground darkens over the first 15 degrees below), in cd/m²,
//! laid out as Ara's bridge prepares it: the independent-roughness field (65 levels, every
//! level at full face size) and a 16-texel irradiance cube.
//!
//! The sky depends on elevation only, so every level is a 1D table over the direction's
//! height, convolved once by quadrature over a Fibonacci sphere and then written into the
//! cube faces. `flat` keeps the layout and every texture fetch but stores one constant
//! value: the doc's same-layout constant control.
use renderlab::asset::environment::{
    CubeFace, CubeLevel, FieldGenerator, Limits, Metadata, PreparedEnvironment, RadianceUnits,
    roughness_knots,
};
use std::f64::consts::PI;

/// Showcase's `Environment`: sky, horizon and ground colours times its intensity (900).
const SKY: [f64; 3] = [0.5, 0.48, 0.45];
const HORIZON: [f64; 3] = [0.6, 0.55, 0.5];
const GROUND: [f64; 3] = [0.25, 0.22, 0.2];
const INTENSITY: f64 = 900.0;
/// Rows of the per-level elevation tables, and directions of the convolution quadrature.
const ROWS: usize = 512;
const QUADRATURE: usize = 4096;

/// Radiance toward a direction of height `y` (its elevation's sine).
fn radiance(y: f64) -> [f64; 3] {
    let lerp = |a: [f64; 3], b: [f64; 3], t: f64| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
    let c = if y >= 0.0 {
        lerp(SKY, HORIZON, (1.0 - y).powi(3))
    } else {
        lerp(HORIZON, GROUND, (-y * 4.0).min(1.0))
    };
    c.map(|v| v * INTENSITY)
}

/// The sky's mean radiance over the upper hemisphere, cosine weighted: the `Sky::Uniform`
/// that sends the same light down.
pub fn mean_radiance() -> [f32; 3] {
    let (mut sum, mut weight) = ([0.0; 3], 0.0);
    for i in 0..ROWS {
        let y = (i as f64 + 0.5) / ROWS as f64;
        let r = radiance(y);
        for c in 0..3 {
            sum[c] += r[c] * y;
        }
        weight += y;
    }
    sum.map(|s| (s / weight) as f32)
}

fn fibonacci(i: usize) -> [f64; 3] {
    let y = 1.0 - (2.0 * i as f64 + 1.0) / QUADRATURE as f64;
    let r = (1.0 - y * y).sqrt();
    let phi = i as f64 * PI * (3.0 - 5f64.sqrt());
    [r * phi.cos(), y, r * phi.sin()]
}

/// A table over height for a kernel `weight(cos)` about a direction of that height.
fn table(weight: impl Fn(f64) -> f64, normalize: bool) -> Vec<[f64; 3]> {
    let samples: Vec<_> = (0..QUADRATURE).map(fibonacci).collect();
    let values: Vec<_> = samples.iter().map(|d| radiance(d[1])).collect();
    (0..ROWS)
        .map(|row| {
            let y = -1.0 + 2.0 * (row as f64 + 0.5) / ROWS as f64;
            let n = [(1.0 - y * y).max(0.0).sqrt(), y, 0.0];
            let (mut sum, mut total) = ([0.0; 3], 0.0);
            for (d, l) in samples.iter().zip(&values) {
                let w = weight(n[0] * d[0] + n[1] * d[1]);
                if w > 0.0 {
                    for c in 0..3 {
                        sum[c] += l[c] * w;
                    }
                    total += w;
                }
            }
            if normalize {
                sum.map(|s| if total > 0.0 { s / total } else { 0.0 })
            } else {
                sum.map(|s| s * 4.0 * PI / QUADRATURE as f64)
            }
        })
        .collect()
}

fn cube(size: u32, at: impl Fn(f64) -> [f64; 3]) -> Result<CubeLevel, renderlab::asset::Error> {
    let mut texels = Vec::with_capacity((6 * size * size) as usize);
    for face in CubeFace::ALL {
        for y in 0..size {
            for x in 0..size {
                let uv = [
                    (x as f64 + 0.5) / size as f64,
                    (y as f64 + 0.5) / size as f64,
                ];
                let d = face.direction(uv);
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                texels.push(at(d[1] / len).map(|v| v as f32));
            }
        }
    }
    CubeLevel::new(size, texels, Limits::unbounded())
}

fn lookup(table: &[[f64; 3]], y: f64) -> [f64; 3] {
    let at = ((y + 1.0) * 0.5 * ROWS as f64 - 0.5).clamp(0.0, (ROWS - 1) as f64);
    let (i, t) = (at.floor() as usize, at.fract());
    let j = (i + 1).min(ROWS - 1);
    std::array::from_fn(|c| table[i][c] * (1.0 - t) + table[j][c] * t)
}

/// The prepared gradient sky with radiance-field faces of `face` texels, or the same layout
/// holding the sky's mean radiance everywhere when `flat`.
pub fn prepare(face: u32, flat: bool) -> Result<PreparedEnvironment, renderlab::asset::Error> {
    let mean = mean_radiance().map(f64::from);
    let irradiance = if flat {
        cube(16, |_| mean.map(|v| v * PI))?
    } else {
        let t = table(|cos| cos.max(0.0), false);
        cube(16, |y| lookup(&t, y))?
    };
    let mut levels = Vec::with_capacity(65);
    for r in roughness_knots() {
        levels.push(if flat {
            cube(face, |_| mean)?
        } else if r < 1e-3 {
            cube(face, radiance)?
        } else {
            // GGX about the direction (normal = view = reflection), in the half-angle cosine.
            let a2 = f64::from(r).powi(4).max(1e-8);
            let t = table(
                |cos| {
                    let h = ((1.0 + cos) * 0.5).max(0.0);
                    let d = h * (a2 - 1.0) + 1.0;
                    if cos > 0.0 { a2 / (d * d) } else { 0.0 }
                },
                true,
            );
            cube(face, |y| lookup(&t, y))?
        });
    }
    PreparedEnvironment::new_independent(
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            rotation_y_radians: 0.0,
            source_radiance_scale: 1.0,
            generator_hash: [u8::from(flat); 32],
        },
        irradiance,
        levels,
        roughness_knots(),
        FieldGenerator::F64HostF32GpuV1,
        Limits::unbounded(),
    )
}
