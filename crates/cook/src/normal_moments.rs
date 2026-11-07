//! Linearly filterable slope moments. No BRDF or roughness is baked into the asset.
use crate::{Result, ensure};
use renderlab_asset::{Filter, Mip, Sampler, Texture, TextureRole, Wrap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconstruction {
    /// Discrete texel distribution (the conventional LEAN approximation).
    Point,
    /// Integrate the continuous bilinear slope field over each base texel's area.
    Bilinear,
}

fn wrap(i: i64, size: u32, mode: Wrap) -> usize {
    let n = i64::from(size);
    (match mode {
        Wrap::Repeat => i.rem_euclid(n),
        Wrap::Clamp => i.clamp(0, n - 1),
        Wrap::Mirror => {
            let p = i.rem_euclid(2 * n);
            if p < n { p } else { 2 * n - 1 - p }
        }
    }) as usize
}

/// Input is the canonical RGBA8 normal map sampled by the legacy material. Only
/// positive-Z maps with finite, half-representable moments are supported; reject
/// unsupported data rather than silently clamping its slopes or variance.
pub fn prepare_rgba8(
    width: u32,
    height: u32,
    rgba8: &[u8],
    sampler: Sampler,
    reconstruction: Reconstruction,
) -> Result<Texture> {
    ensure(
        width > 0
            && height > 0
            && u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                == Some(rgba8.len() as u64),
        "invalid normal moment dimensions",
    )?;
    let mut normals = Vec::with_capacity(rgba8.len() / 4);
    let mut slopes = Vec::with_capacity(rgba8.len() / 4);
    for p in rgba8.chunks_exact(4) {
        let n = [p[0], p[1], p[2]].map(|v| f64::from(v) * 2.0 / 255.0 - 1.0);
        ensure(n[2] > 0.0, "slope moments require positive-Z normals")?;
        let slope = [n[0] / n[2], n[1] / n[2]];
        ensure(
            slope.iter().all(|x| x * x <= 65504.0),
            "normal slope moments exceed half-float range",
        )?;
        normals.push(n);
        slopes.push(slope);
    }
    let bilinear = reconstruction == Reconstruction::Bilinear
        && sampler.min == Filter::Linear
        && sampler.mag == Filter::Linear;
    let mut values = Vec::with_capacity(slopes.len());
    // Exact integrals of the 1D linear basis, centred on a base texel. In 2D
    // the separable product gives E[XY], not an average of covariances.
    let mean = [0.125, 0.75, 0.125];
    let product = [
        [1.0 / 24.0, 1.0 / 12.0, 0.0],
        [1.0 / 12.0, 7.0 / 12.0, 1.0 / 12.0],
        [0.0, 1.0 / 12.0, 1.0 / 24.0],
    ];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let [sx, sy] = slopes[index];
            let mut v = [
                sx,
                sy,
                sx * sx,
                sy * sy,
                sx * sy,
                normals[index][0],
                normals[index][1],
                normals[index][2],
            ];
            if bilinear {
                v[..5].fill(0.0);
                let p: Vec<_> = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                    .map(|(dx, dy)| {
                        slopes[wrap(i64::from(y) + dy, height, sampler.wrap_v) * width as usize
                            + wrap(i64::from(x) + dx, width, sampler.wrap_u)]
                    })
                    .collect();
                for a in 0..9 {
                    let ax = a % 3;
                    let ay = a / 3;
                    let w = mean[ax] * mean[ay];
                    v[0] += p[a][0] * w;
                    v[1] += p[a][1] * w;
                    for b in 0..9 {
                        let w = product[ax][b % 3] * product[ay][b / 3];
                        v[2] += w * p[a][0] * p[b][0];
                        v[3] += w * p[a][1] * p[b][1];
                        v[4] += w * p[a][0] * p[b][1];
                    }
                }
            }
            values.push(v);
        }
    }
    let mut texture = Texture {
        role: TextureRole::NormalSlopeMoments,
        mips: Vec::new(),
    };
    let (mut w, mut h) = (width, height);
    loop {
        let mut bytes = Vec::with_capacity(values.len() * 16);
        for layer in 0..2 {
            for v in &values {
                for &x in &v[layer * 4..layer * 4 + 4] {
                    let half = half::f16::from_f64(x);
                    ensure(half.is_finite(), "nonfinite encoded normal moment")?;
                    bytes.extend(half.to_bits().to_le_bytes());
                }
            }
        }
        texture.mips.push(Mip {
            width: w,
            height: h,
            rgba8: bytes,
        });
        if w == 1 && h == 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh) as usize);
        for y in 0..nh {
            for x in 0..nw {
                let (x0, x1) = (
                    f64::from(x) * f64::from(w) / f64::from(nw),
                    f64::from(x + 1) * f64::from(w) / f64::from(nw),
                );
                let (y0, y1) = (
                    f64::from(y) * f64::from(h) / f64::from(nh),
                    f64::from(y + 1) * f64::from(h) / f64::from(nh),
                );
                let mut sum = [0.0; 8];
                for sy in y0.floor() as u32..(y1.ceil() as u32).min(h) {
                    for sx in x0.floor() as u32..(x1.ceil() as u32).min(w) {
                        let weight = (x1.min(f64::from(sx + 1)) - x0.max(f64::from(sx)))
                            * (y1.min(f64::from(sy + 1)) - y0.max(f64::from(sy)));
                        for (a, b) in sum.iter_mut().zip(values[(sy * w + sx) as usize]) {
                            *a += weight * b;
                        }
                    }
                }
                next.push(sum.map(|v| v / ((x1 - x0) * (y1 - y0))));
            }
        }
        values = next;
        w = nw;
        h = nh;
    }
    Ok(texture)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn last(t: &Texture) -> [f64; 5] {
        let bytes = &t.mips.last().unwrap().rgba8;
        std::array::from_fn(|i| {
            f64::from(half::f16::from_bits(u16::from_le_bytes(
                bytes[i * 2..i * 2 + 2].try_into().unwrap(),
            )))
        })
    }
    #[test]
    fn covariance_is_formed_after_filtering_and_bilinear_reconstruction_reduces_it() {
        let pixels = [
            140, 128, 255, 255, 116, 128, 255, 255, 116, 128, 255, 255, 140, 128, 255, 255,
        ];
        let a = prepare_rgba8(2, 2, &pixels, Sampler::default(), Reconstruction::Point).unwrap();
        let b = prepare_rgba8(2, 2, &pixels, Sampler::default(), Reconstruction::Bilinear).unwrap();
        let (a, b) = (last(&a), last(&b));
        assert!(a[2] - a[0] * a[0] > 0.005);
        assert!(
            ((b[2] - b[0] * b[0]) / (a[2] - a[0] * a[0]) - 1.0 / 9.0).abs() < 0.001,
            "{a:?} {b:?}"
        );
        assert_eq!(a[0], b[0]);
        assert!(
            prepare_rgba8(
                1,
                1,
                &[255, 128, 0, 255],
                Sampler::default(),
                Reconstruction::Point
            )
            .is_err()
        );
    }

    #[test]
    fn correlated_slopes_keep_the_off_diagonal_moment() {
        assert!(
            prepare_rgba8(
                u32::MAX,
                u32::MAX,
                &[],
                Sampler::default(),
                Reconstruction::Point
            )
            .is_err()
        );
        let pixels = [140, 140, 255, 255, 116, 116, 255, 255];
        let t = prepare_rgba8(2, 1, &pixels, Sampler::default(), Reconstruction::Point).unwrap();
        let moments = last(&t);
        let covariance = moments[4] - moments[0] * moments[1];
        assert!(covariance > 0.005);
        assert!((covariance - (moments[2] - moments[0] * moments[0])).abs() < 1e-5);
    }
}
