//! Linear +Y tangent-space normal preparation, independent of RLA texture roles.
use crate::{Error, Result, ensure};

/// A cancelled vector has no preferred direction. Use the unperturbed surface normal.
pub const ZERO_VECTOR_FALLBACK: [f32; 3] = [0.0, 0.0, 1.0];
const MIN_VECTOR_LENGTH: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq)]
pub struct NormalMip {
    width: u32,
    height: u32,
    vectors: Vec<[f32; 3]>,
}
impl NormalMip {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn vectors(&self) -> &[[f32; 3]] {
        &self.vectors
    }
    /// Linear UNORM encoding. Alpha is unused and set to 255.
    pub fn rgba8(&self) -> Vec<u8> {
        self.vectors
            .iter()
            .flat_map(|v| [encode(v[0]), encode(v[1]), encode(v[2]), 255])
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedNormalMap {
    mips: Vec<NormalMip>,
}
impl PreparedNormalMap {
    pub fn mips(&self) -> &[NormalMip] {
        &self.mips
    }
}

fn encode(v: f32) -> u8 {
    ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8
}
fn normalized(v: [f64; 3]) -> [f32; 3] {
    let length = v[0].hypot(v[1]).hypot(v[2]);
    if length <= MIN_VECTOR_LENGTH {
        ZERO_VECTOR_FALLBACK
    } else {
        v.map(|x| (x / length) as f32)
    }
}

/// Reference math for sampled, decoded XYZ. Scale is a material parameter, not baked into mips.
/// glTF permits finite negative scale. X and Y change together; Z is preserved.
pub fn apply_scale(vector: [f32; 3], scale: f32) -> Result<[f32; 3]> {
    ensure(
        scale.is_finite() && vector.iter().all(|v| v.is_finite()),
        "normal scale/vector must be finite",
    )?;
    Ok(normalized([
        f64::from(vector[0]) * f64::from(scale),
        f64::from(vector[1]) * f64::from(scale),
        f64::from(vector[2]),
    ]))
}

/// Decode RGB as linear XYZ in [-1,1]. Alpha has no meaning for a normal texture.
pub fn prepare_rgba8(width: u32, height: u32, rgba8: &[u8]) -> Result<PreparedNormalMap> {
    ensure(
        u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4))
            == Some(rgba8.len() as u64),
        "normal texture byte count differs from dimensions",
    )?;
    let vectors = rgba8
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]].map(|v| (f64::from(v) * (2.0 / 255.0) - 1.0) as f32))
        .collect();
    prepare_vectors(width, height, vectors)
}

/// Area box filtering includes odd edges. Each level averages unit decoded vectors,
/// then renormalizes. Mean length/variance is discarded; there is no roughness compensation.
pub fn prepare_vectors(
    width: u32,
    height: u32,
    vectors: Vec<[f32; 3]>,
) -> Result<PreparedNormalMap> {
    ensure(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) == vectors.len() as u64,
        "normal texture requires nonzero dimensions and one vector per texel",
    )?;
    ensure(
        vectors.iter().flatten().all(|v| v.is_finite()),
        "nonfinite normal texture vector",
    )?;
    let mut map = PreparedNormalMap {
        mips: vec![NormalMip {
            width,
            height,
            vectors: vectors
                .into_iter()
                .map(|v| normalized(v.map(f64::from)))
                .collect(),
        }],
    };
    while map
        .mips
        .last()
        .is_some_and(|m| m.width != 1 || m.height != 1)
    {
        let input = map.mips.last().unwrap();
        let (w, h) = ((input.width / 2).max(1), (input.height / 2).max(1));
        let count = (w as usize)
            .checked_mul(h as usize)
            .ok_or_else(|| Error("normal mip size overflow".into()))?;
        let mut output = Vec::with_capacity(count);
        for y in 0..h {
            let y0 = f64::from(y) * f64::from(input.height) / f64::from(h);
            let y1 = f64::from(y + 1) * f64::from(input.height) / f64::from(h);
            for x in 0..w {
                let x0 = f64::from(x) * f64::from(input.width) / f64::from(w);
                let x1 = f64::from(x + 1) * f64::from(input.width) / f64::from(w);
                let mut sum = [0.0; 3];
                for sy in (y0.floor() as u32)..(y1.ceil() as u32).min(input.height) {
                    for sx in (x0.floor() as u32)..(x1.ceil() as u32).min(input.width) {
                        let weight = (x1.min(f64::from(sx + 1)) - x0.max(f64::from(sx)))
                            * (y1.min(f64::from(sy + 1)) - y0.max(f64::from(sy)));
                        for (c, value) in sum.iter_mut().enumerate() {
                            *value += weight
                                * f64::from(
                                    input.vectors[sy as usize * input.width as usize + sx as usize]
                                        [c],
                                );
                        }
                    }
                }
                let area = (x1 - x0) * (y1 - y0);
                output.push(normalized(sum.map(|v| v / area)));
            }
        }
        map.mips.push(NormalMip {
            width: w,
            height: h,
            vectors: output,
        });
    }
    Ok(map)
}
