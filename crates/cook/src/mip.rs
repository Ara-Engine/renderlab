use crate::{Result, ensure};
use renderlab_asset::{Mip, Texture, TextureRole};

fn linear(v: u8) -> f64 {
    let x = f64::from(v) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> u8 {
    let x = if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (x * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Area box filter. Fractional coverage includes every odd edge texel.
pub(crate) fn build(width: u32, height: u32, rgba8: Vec<u8>, role: TextureRole) -> Result<Texture> {
    if role.is_normal() {
        let prepared = if role == TextureRole::NormalVarianceLinear {
            crate::normal_map::prepare_rgba8(width, height, &rgba8)?
        } else {
            crate::normal_map::prepare_rgba8_legacy(width, height, &rgba8)?
        };
        return Ok(Texture {
            role,
            format: renderlab_asset::TextureFormat::Uncompressed,
            mips: prepared
                .mips()
                .iter()
                .map(|m| {
                    let mut rgba8 = m.rgba8();
                    if role == TextureRole::NormalLinear {
                        for p in rgba8.chunks_exact_mut(4) {
                            p[3] = 255;
                        }
                    }
                    Mip {
                        width: m.width(),
                        height: m.height(),
                        rgba8,
                    }
                })
                .collect(),
        });
    }
    ensure(
        width > 0
            && height > 0
            && u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                == Some(rgba8.len() as u64),
        "invalid mip source",
    )?;
    let mut texture = Texture {
        role,
        format: renderlab_asset::TextureFormat::Uncompressed,
        mips: vec![Mip {
            width,
            height,
            rgba8,
        }],
    };
    while texture
        .mips
        .last()
        .is_some_and(|m| m.width != 1 || m.height != 1)
    {
        let input = texture.mips.last().unwrap();
        let (w, h) = ((input.width / 2).max(1), (input.height / 2).max(1));
        let size = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| crate::Error("mip allocation overflow".into()))?;
        let mut output = vec![0; size];
        for y in 0..h {
            let y0 = f64::from(y) * f64::from(input.height) / f64::from(h);
            let y1 = f64::from(y + 1) * f64::from(input.height) / f64::from(h);
            for x in 0..w {
                let x0 = f64::from(x) * f64::from(input.width) / f64::from(w);
                let x1 = f64::from(x + 1) * f64::from(input.width) / f64::from(w);
                let mut sum = [0.0; 4];
                for sy in (y0.floor() as u32)..(y1.ceil() as u32).min(input.height) {
                    for sx in (x0.floor() as u32)..(x1.ceil() as u32).min(input.width) {
                        let weight = (x1.min(f64::from(sx + 1)) - x0.max(f64::from(sx)))
                            * (y1.min(f64::from(sy + 1)) - y0.max(f64::from(sy)));
                        let pixel = (sy as usize * input.width as usize + sx as usize) * 4;
                        for (c, value) in sum.iter_mut().enumerate() {
                            let byte = input.rgba8[pixel + c];
                            *value += weight
                                * if role == TextureRole::BaseColorSrgb && c < 3 {
                                    linear(byte)
                                } else {
                                    f64::from(byte) / 255.0
                                };
                        }
                    }
                }
                let area = (x1 - x0) * (y1 - y0);
                for (c, value) in sum.into_iter().enumerate() {
                    output[(y as usize * w as usize + x as usize) * 4 + c] =
                        if role == TextureRole::BaseColorSrgb && c < 3 {
                            srgb(value / area)
                        } else {
                            (value / area * 255.0).round().clamp(0.0, 255.0) as u8
                        };
                }
            }
        }
        texture.mips.push(Mip {
            width: w,
            height: h,
            rgba8: output,
        });
    }
    Ok(texture)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_and_data_swatches_have_independent_expected_values() {
        let bytes = vec![0, 0, 0, 0, 255, 255, 255, 255];
        let color = build(2, 1, bytes.clone(), TextureRole::BaseColorSrgb).unwrap();
        let data = build(2, 1, bytes, TextureRole::MetallicRoughnessLinear).unwrap();
        assert_eq!(color.mips[1].rgba8, [188, 188, 188, 128]);
        assert_eq!(data.mips[1].rgba8, [128, 128, 128, 128]);
        let gray = build(3, 1, vec![128; 12], TextureRole::BaseColorSrgb).unwrap();
        assert_eq!(gray.mips[1].rgba8, [128; 4]);
    }
    #[test]
    fn odd_edges_contribute_area_and_chain_finishes() {
        let mut bytes = vec![0; 5 * 3 * 4];
        for y in 0..3 {
            bytes[(y * 5 + 4) * 4..(y * 5 + 5) * 4].fill(255);
        }
        let t = build(5, 3, bytes, TextureRole::MetallicRoughnessLinear).unwrap();
        assert_eq!(
            t.mips
                .iter()
                .map(|m| (m.width, m.height))
                .collect::<Vec<_>>(),
            [(5, 3), (2, 1), (1, 1)]
        );
        assert_eq!(t.mips[1].rgba8, [0, 0, 0, 0, 102, 102, 102, 102]);
        assert_eq!(t.mips[2].rgba8, [51; 4]);
        let mut bytes = vec![0; 3 * 5 * 4];
        bytes[3 * 4 * 4..].fill(255);
        let t = build(3, 5, bytes, TextureRole::MetallicRoughnessLinear).unwrap();
        assert_eq!(t.mips[2].rgba8, [51; 4]);
    }
}
