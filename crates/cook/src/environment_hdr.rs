//! Thin Radiance RGBE input boundary. Decoding never reaches the runtime/asset crate.
use crate::environment::{CookLimits, LinearEnvironment, source_texels};
use crate::{Error, Result};
use image::{ImageDecoder, codecs::hdr::HdrDecoder};
use std::io::{Cursor, Read};
use std::path::Path;

/// Read a bounded file and decode through the same production byte path.
pub fn read_rec709_hdr(path: &Path, limits: CookLimits) -> Result<LinearEnvironment> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > limits.max_source_file_bytes
        || len > limits.max_working_bytes
        || len > isize::MAX as u64
    {
        return Err(Error("HDR source file byte limit".into()));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len as usize)
        .map_err(|_| Error("HDR file allocation".into()))?;
    bytes.resize(len as usize, 0);
    file.read_exact(&mut bytes)?;
    // Check growth with a fixed-size buffer; never grow the admitted file allocation.
    if file.read(&mut [0; 1])? != 0 {
        return Err(Error("HDR file changed while reading".into()));
    }
    decode_rec709_hdr(&bytes, limits)
}

/// Caller asserts headerless RGB primaries are linear Rec.709; this does not convert colour spaces.
/// Explicit primaries must be Rec.709/D65. Only identity EXPOSURE/COLORCORR are supported.
/// Requires -Y +X RGBE with standard scanline RLE or uncompressed pixels. Legacy chained
/// repeat markers are unsupported (image 0.25.10's legacy run multiplier can overflow).
/// Stored RGB values are retained, with no tone mapping.
pub fn decode_rec709_hdr(bytes: &[u8], limits: CookLimits) -> Result<LinearEnvironment> {
    if bytes.len() as u64 > limits.max_source_file_bytes {
        return Err(Error("HDR source file byte limit".into()));
    }
    let body = validate_header(bytes, limits)?;
    let mut cursor = Cursor::new(bytes);
    let decoder = HdrDecoder::new(&mut cursor).map_err(|e| Error(e.to_string()))?;
    let (width, height) = decoder.dimensions();
    let n = source_texels(width, height, limits)?;
    let decoded_bytes = n * 12;
    // Decode bytes + owned pixels + RGBE scanline + caller's encoded bytes coexist.
    let working = decoded_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(u64::from(width) * 4))
        .and_then(|n| n.checked_add(bytes.len() as u64))
        .ok_or_else(|| Error("HDR working byte overflow".into()))?;
    if working > limits.max_working_bytes {
        return Err(Error("HDR decode working byte limit".into()));
    }
    validate_scanlines(&bytes[body..], width, height)?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(decoded_bytes as usize)
        .map_err(|_| Error("HDR decode allocation".into()))?;
    raw.resize(decoded_bytes as usize, 0);
    decoder
        .read_image(&mut raw)
        .map_err(|e| Error(e.to_string()))?;
    if cursor.position() != bytes.len() as u64 {
        return Err(Error("HDR trailing bytes".into()));
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(n as usize)
        .map_err(|_| Error("HDR pixel allocation".into()))?;
    for p in raw.chunks_exact(12) {
        pixels.push(std::array::from_fn(|i| {
            f32::from_ne_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
        }));
    }
    LinearEnvironment::new(width, height, pixels, limits)
}

fn validate_header(bytes: &[u8], limits: CookLimits) -> Result<usize> {
    let prefix = &bytes[..bytes.len().min(limits.max_header_bytes)];
    let mut pos = 0;
    let mut format = false;
    let mut ended = false;
    while pos < prefix.len() {
        let end = prefix[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|n| pos + n)
            .ok_or_else(|| Error("HDR truncated/oversized header".into()))?;
        let line = std::str::from_utf8(&prefix[pos..end])
            .map_err(|_| Error("HDR header encoding".into()))?
            .trim_end_matches('\r');
        pos = end + 1;
        if ended {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 4 || fields[0] != "-Y" || fields[2] != "+X" {
                return Err(Error("HDR requires -Y +X orientation".into()));
            }
            return if format {
                Ok(pos)
            } else {
                Err(Error("HDR missing RGBE format".into()))
            };
        }
        if line.is_empty() {
            ended = true;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            let numbers = |expected: &[f32]| -> bool {
                let values: std::result::Result<Vec<f32>, _> =
                    value.split_whitespace().map(str::parse).collect();
                values.is_ok_and(|v| v == expected)
            };
            match key {
                "FORMAT" if value == "32-bit_rle_rgbe" => format = true,
                "EXPOSURE" if numbers(&[1.0]) => {}
                "COLORCORR" if numbers(&[1.0; 3]) => {}
                "PRIMARIES" if numbers(&[0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290]) => {}
                "FORMAT" | "EXPOSURE" | "COLORCORR" | "PRIMARIES" | "GAMMA" => {
                    return Err(Error("unsupported HDR format/colour calibration".into()));
                }
                _ => {}
            }
        }
    }
    Err(Error("HDR truncated/oversized header".into()))
}

// Bounded, allocation-free framing before the dependency sees packet lengths or legacy runs.
fn validate_scanlines(mut bytes: &[u8], width: u32, height: u32) -> Result<()> {
    fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
        let result = bytes
            .get(..n)
            .ok_or_else(|| Error("HDR truncated scanline".into()))?;
        *bytes = &bytes[n..];
        Ok(result)
    }
    for _ in 0..height {
        let first = take(&mut bytes, 4)?;
        if first[..2] == [2, 2] && first[2] < 128 {
            if u32::from(first[2]) * 256 + u32::from(first[3]) != width {
                return Err(Error("HDR scanline width mismatch".into()));
            }
            for _ in 0..4 {
                let mut remaining = u64::from(width);
                while remaining > 0 {
                    let code = take(&mut bytes, 1)?[0];
                    let count = u64::from(if code > 128 { code - 128 } else { code });
                    if count == 0 || count > remaining {
                        return Err(Error("HDR invalid RLE packet".into()));
                    }
                    take(&mut bytes, if code > 128 { 1 } else { count as usize })?;
                    remaining -= count;
                }
            }
        } else {
            if first[..3] == [1, 1, 1] {
                return Err(Error("HDR legacy repeat markers unsupported".into()));
            }
            for _ in 1..width {
                if take(&mut bytes, 4)?[..3] == [1, 1, 1] {
                    return Err(Error("HDR legacy repeat markers unsupported".into()));
                }
            }
        }
    }
    if !bytes.is_empty() {
        return Err(Error("HDR trailing bytes".into()));
    }
    Ok(())
}
