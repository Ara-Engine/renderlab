//! The variance layer of a BC5 normal map (`Renderer::try_upload_compressed_texture`).
//!
//! BC5 holds a normal's X and Y; the shading API rebuilds a unit Z, so a filtered sample is
//! unit length and the map's Toksvig spread (`material_normal_variance`, from the mean
//! normal's shortening) would vanish. This computes, per texel of every level, the spread its
//! footprint of top-level unit normals has: `A = sqrt(clamp(2 (1 - L) / L, 0, 1))` with L the
//! length of their mean, the quantity NormalVarianceLinear's alpha carries, and stores it as a
//! second array layer of the same texture (BC4 in the red channel of BC5 blocks, green zero).
//! The material word then reads it as variance code 1, so specular anti-aliasing keeps the
//! spread an RGBA8 chain of the same normals implied, and no stage needs another binding.
use renderlab_asset::{BlockFormat, CompressedTexture};

/// Each level's variance from its parent's footprint (twice as wide) instead of its own: the
/// conservative fallback should the gate show more sparkle than the RGBA8 chain.
const WIDER: bool = false;

/// The 16 values of a BC4 block, 0-1, row-major (the format's interpolation).
fn decode_bc4(block: &[u8]) -> [f32; 16] {
    let (a, b) = (f32::from(block[0]), f32::from(block[1]));
    let bits = u64::from_le_bytes(block[..8].try_into().unwrap()) >> 16;
    std::array::from_fn(|t| {
        let k = ((bits >> (3 * t)) & 7) as u32;
        let v = match k {
            0 => a,
            1 => b,
            k if block[0] > block[1] => ((8 - k) as f32 * a + (k - 1) as f32 * b) / 7.0,
            6 => 0.0,
            7 => 255.0,
            k => ((6 - k) as f32 * a + (k - 1) as f32 * b) / 5.0,
        };
        v / 255.0
    })
}

/// A BC4 block for 16 bytes: the eight-value palette between their extremes, each texel its
/// nearest entry (a constant block is exact).
fn encode_bc4(values: &[u8; 16]) -> [u8; 8] {
    let (hi, lo) = (*values.iter().max().unwrap(), *values.iter().min().unwrap());
    if hi == lo {
        return [hi, lo, 0, 0, 0, 0, 0, 0];
    }
    let palette: [f32; 8] = std::array::from_fn(|k| match k {
        0 => f32::from(hi),
        1 => f32::from(lo),
        k => ((8 - k) as f32 * f32::from(hi) + (k - 1) as f32 * f32::from(lo)) / 7.0,
    });
    let mut bits = u64::from(hi) | u64::from(lo) << 8;
    for (t, &v) in values.iter().enumerate() {
        let k = (0..8)
            .min_by(|&i, &j| {
                (palette[i] - f32::from(v))
                    .abs()
                    .total_cmp(&(palette[j] - f32::from(v)).abs())
            })
            .unwrap();
        bits |= (k as u64) << (16 + 3 * t);
    }
    bits.to_le_bytes()
}

/// Unit normals of the top level, as running sums per texel of level 1 (the 2x2 texels each
/// covers) with their counts: the top level itself needs no spread (one texel, L = 1).
fn level_one_sums(t: &CompressedTexture) -> (u32, u32, Vec<[f32; 4]>) {
    let (w, h) = (t.width, t.height);
    let (w1, h1) = ((w / 2).max(1), (h / 2).max(1));
    let mut sums = vec![[0f32; 4]; (w1 * h1) as usize];
    let blocks_x = w / BlockFormat::BLOCK;
    for (b, block) in t.mips[0].chunks_exact(16).enumerate() {
        let (bx, by) = (b as u32 % blocks_x, b as u32 / blocks_x);
        let (xs, ys) = (decode_bc4(&block[..8]), decode_bc4(&block[8..]));
        for i in 0..16 {
            let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
            let (nx, ny) = (xs[i] * 2.0 - 1.0, ys[i] * 2.0 - 1.0);
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let s = &mut sums[((y / 2).min(h1 - 1) * w1 + (x / 2).min(w1 - 1)) as usize];
            s[0] += nx;
            s[1] += ny;
            s[2] += nz;
            s[3] += 1.0;
        }
    }
    (w1, h1, sums)
}

/// A from a footprint's summed unit normals.
fn spread(s: [f32; 4]) -> u8 {
    let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt() / s[3].max(1.0);
    let l = l.clamp(1e-6, 1.0);
    let a = (2.0 * (1.0 - l) / l).clamp(0.0, 1.0).sqrt();
    (a * 255.0).round() as u8
}

/// BC5 blocks of `values` (one byte a texel, `w` x `h`) in the red channel, green zero.
fn blocks(values: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(BlockFormat::level_bytes(w, h) as usize);
    for by in 0..h.div_ceil(4) {
        for bx in 0..w.div_ceil(4) {
            let texel: [u8; 16] = std::array::from_fn(|i| {
                let (x, y) = (
                    (bx * 4 + i as u32 % 4).min(w - 1),
                    (by * 4 + i as u32 / 4).min(h - 1),
                );
                values[(y * w + x) as usize]
            });
            out.extend(encode_bc4(&texel));
            out.extend([0u8; 8]);
        }
    }
    out
}

/// The variance layer's levels for a BC5 normal map, as BC5 block data of the map's sizes.
pub(crate) fn variance_layer(t: &CompressedTexture) -> Vec<Vec<u8>> {
    let levels = t.mips.len();
    // Footprint sums per level from 1 down the chain (each texel the sum of its 2x2 children).
    let mut sums = vec![level_one_sums(t)];
    while sums.len() + 1 < levels {
        let (w, h, prev) = sums.last().unwrap();
        let (w2, h2) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![[0f32; 4]; (w2 * h2) as usize];
        for y in 0..*h {
            for x in 0..*w {
                let s = prev[(y * w + x) as usize];
                let d = &mut next[((y / 2).min(h2 - 1) * w2 + (x / 2).min(w2 - 1)) as usize];
                for c in 0..4 {
                    d[c] += s[c];
                }
            }
        }
        sums.push((w2, h2, next));
    }
    (0..levels)
        .map(|level| {
            let (w, h) = t.level_size(level as u32);
            // Level `level` reads the sums of its own footprint (index level - 1), or with
            // WIDER its parent's; the top level's own footprint is one texel (A = 0).
            let source = if WIDER { level } else { level.wrapping_sub(1) };
            let values: Vec<u8> = match sums.get(source) {
                Some((sw, sh, s)) => (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        let scale = if WIDER { 2 } else { 1 };
                        spread(s[((y / scale).min(sh - 1) * sw + (x / scale).min(sw - 1)) as usize])
                    })
                    .collect(),
                None if level == 0 && !WIDER => vec![0; (w * h) as usize],
                None => vec![
                    spread(sums.last().map_or([0.0, 0.0, 1.0, 1.0], |s| s.2[0]));
                    (w * h) as usize
                ],
            };
            blocks(&values, w, h)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BC4 round trips: a constant block exactly, a two-value block exactly, others within a
    /// fourteenth of their range.
    #[test]
    fn bc4_round_trip() {
        let constant = [77u8; 16];
        assert!(
            decode_bc4(&encode_bc4(&constant))
                .iter()
                .all(|&v| (v * 255.0).round() == 77.0)
        );
        let two: [u8; 16] = std::array::from_fn(|i| if i % 3 == 0 { 10 } else { 200 });
        let back = decode_bc4(&encode_bc4(&two));
        assert!(
            two.iter()
                .zip(back)
                .all(|(&a, b)| (f32::from(a) - b * 255.0).abs() < 1e-3)
        );
        let ramp: [u8; 16] = std::array::from_fn(|i| (i * 13) as u8);
        let back = decode_bc4(&encode_bc4(&ramp));
        assert!(
            ramp.iter()
                .zip(back)
                .all(|(&a, b)| (f32::from(a) - b * 255.0).abs() <= 195.0 / 14.0 + 0.5)
        );
    }

    /// A flat map has no spread at any level; a map of two opposite slopes gains spread as
    /// the footprint covers both, and the top level keeps none.
    #[test]
    fn variance_from_the_footprint() {
        let flat_block = {
            let mut b = encode_bc4(&[128; 16]).to_vec();
            b.extend(encode_bc4(&[128; 16]));
            b
        };
        let side = 8u32;
        let chain = |block: &dyn Fn(u32, u32) -> Vec<u8>| {
            let mut mips = vec![];
            let mut w = side;
            loop {
                let mut level = vec![];
                for by in 0..w.div_ceil(4) {
                    for bx in 0..w.div_ceil(4) {
                        level.extend(block(bx, by));
                    }
                }
                mips.push(level);
                if w == 1 {
                    break;
                }
                w /= 2;
            }
            CompressedTexture {
                format: BlockFormat::Bc5,
                width: side,
                height: side,
                mips,
            }
        };
        let flat = chain(&|_, _| flat_block.clone());
        let layer = variance_layer(&flat);
        assert_eq!(layer.len(), 4);
        for level in &layer {
            assert!(level.chunks_exact(16).all(|b| b[0] <= 1 && b[1] <= 1));
        }
        // Columns alternating X = -0.6 and +0.6.
        let striped = chain(&|_, _| {
            let x: [u8; 16] = std::array::from_fn(|i| if i % 2 == 0 { 51 } else { 204 });
            let mut b = encode_bc4(&x).to_vec();
            b.extend(encode_bc4(&[128; 16]));
            b
        });
        let layer = variance_layer(&striped);
        assert_eq!(layer[0][0], 0, "the top level has no spread");
        assert!(
            layer[1][0] > 100,
            "level 1 spans both slopes: {}",
            layer[1][0]
        );
    }
}
