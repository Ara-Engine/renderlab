//! Owned analytic fixtures, including an explicit lossy RGBE writer. No external assets.
use crate::{Error, Result};
use std::f64::consts::{PI, TAU};
use std::io::Write;

pub fn direction(x: u32, y: u32, width: u32, height: u32) -> [f64; 3] {
    let phi = TAU * ((x as f64 + 0.5) / width as f64 - 0.5);
    let theta = PI * (y as f64 + 0.5) / height as f64;
    [
        theta.sin() * phi.cos(),
        theta.cos(),
        theta.sin() * phi.sin(),
    ]
}

/// 2e6 peak on a dark coloured sky; cosine-power 128 sun has a known spherical integral.
pub fn sun_sky(d: [f64; 3]) -> [f64; 3] {
    let sun = [0.36, 0.48, 0.8]; // exactly unit length
    let dot = d.iter().zip(sun).map(|(a, b)| a * b).sum::<f64>().max(0.0);
    let lobe = 2e6 * dot.powi(128);
    [0.02 + lobe, 0.04 + lobe * 0.8, 0.1 + lobe * 0.5]
}

/// Radiance RGBE truncation: C=floor(L*256/2^e), shared e=1+floor(log2(max(L))).
/// Absolute per-channel error is <2^e/256; bright pixels can quantize weak colour channels to zero.
/// Values outside RGBE's exponent range are rejected. This loss is source-format loss, not cook loss.
pub fn rgbe(rgb: [f32; 3]) -> Result<[u8; 4]> {
    if rgb.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(Error("invalid RGBE fixture input".into()));
    }
    let max = rgb.into_iter().fold(0.0_f32, f32::max) as f64;
    if max == 0.0 {
        return Ok([0; 4]);
    }
    let e = max.log2().floor() as i32 + 1;
    if !(-127..=127).contains(&e) {
        return Err(Error("fixture outside RGBE range".into()));
    }
    let scale = 256.0 / 2.0_f64.powi(e);
    Ok([
        (rgb[0] as f64 * scale).floor() as u8,
        (rgb[1] as f64 * scale).floor() as u8,
        (rgb[2] as f64 * scale).floor() as u8,
        (e + 128) as u8,
    ])
}

/// Write actual standard scanline-RLE HDR with literal packets (or legacy pixels for narrow rows).
pub fn write_hdr(
    mut out: impl Write,
    width: u32,
    height: u32,
    mut radiance: impl FnMut([f64; 3]) -> [f32; 3],
) -> Result<()> {
    if width == 0 || height == 0 || width > 32767 {
        return Err(Error("fixture HDR dimensions".into()));
    }
    write!(
        out,
        "#?RADIANCE\n# Renderlab owned analytic fixture\nFORMAT=32-bit_rle_rgbe\nPRIMARIES=0.64 0.33 0.30 0.60 0.15 0.06 0.3127 0.3290\n\n-Y {height} +X {width}\n"
    )?;
    let mut row = Vec::new();
    row.try_reserve_exact(width as usize)
        .map_err(|_| Error("fixture scanline allocation".into()))?;
    for y in 0..height {
        row.clear();
        for x in 0..width {
            row.push(rgbe(radiance(direction(x, y, width, height)))?);
        }
        if width < 8 {
            for p in &row {
                out.write_all(p)?;
            }
        } else {
            out.write_all(&[2, 2, (width >> 8) as u8, width as u8])?;
            for c in 0..4 {
                for chunk in row.chunks(128) {
                    out.write_all(&[chunk.len() as u8])?;
                    for p in chunk {
                        out.write_all(&[p[c]])?;
                    }
                }
            }
        }
    }
    Ok(())
}
