use crate::{
    Error,
    observe::{Allocation, Owned},
};
use renderlab_asset::environment::{CubeLevel, PreparedEnvironment};
use std::sync::Arc;

/// Renderer-affine f32 cubes. Uploaded maps are immutable; the renderer updates its
/// internal procedural sky in place. Cloning retains ownership, without an upload.
#[derive(Clone)]
pub struct EnvironmentMap(pub(crate) Arc<EnvironmentInner>);
pub(crate) struct EnvironmentInner {
    pub renderer: Arc<()>,
    pub diffuse: EnvironmentCube,
    pub specular: EnvironmentCube,
    pub specular_second: Option<EnvironmentCube>,
    /// The fitted lobe table (src/environment_lobes.rs) of an RLE2 field with dominant
    /// lobes; None binds the renderer's empty table.
    pub lobes: Option<EnvironmentCube>,
    pub independent: bool,
    /// A legacy mip chain whose narrow levels are banked in `specular_second` (`bank_legacy`).
    pub banked: bool,
    pub yaw: f32,
    pub levels: u32,
    pub hash: [u8; 32],
}
pub(crate) struct EnvironmentCube {
    /// A cube texture's cube-array view (cube i is layers 6i..6i+5), as lighting group 3
    /// binds it; the lobe table's plain 2D view.
    pub view: Owned<wgpu::TextureView>,
    pub _texture: Owned<wgpu::Texture>,
    pub _allocation: Allocation,
}

/// Policy limits, additionally bounded by device dimensions and checked host addressing.
#[derive(Clone, Copy, Debug)]
pub struct EnvironmentLimits {
    pub max_dimension: u32,
    pub max_map_bytes: u64,
    /// All live map generations, including caller handles and submitted uses.
    pub max_owned_bytes: u64,
}
impl EnvironmentLimits {
    pub const fn unbounded() -> Self {
        Self {
            max_dimension: u32::MAX,
            max_map_bytes: u64::MAX,
            max_owned_bytes: u64::MAX,
        }
    }
}
impl Default for EnvironmentLimits {
    fn default() -> Self {
        Self {
            max_dimension: 2048,
            max_map_bytes: 128 * 1024 * 1024,
            max_owned_bytes: 384 * 1024 * 1024,
        }
    }
}

/// Additional source-to-world +Y yaw and linear illumination multiplier.
/// Calibration in the prepared metadata is already baked and is never multiplied again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentParameters {
    pub rotation_y_radians: f32,
    pub intensity: f32,
}
impl Default for EnvironmentParameters {
    fn default() -> Self {
        Self {
            rotation_y_radians: 0.,
            intensity: 1.,
        }
    }
}
impl EnvironmentParameters {
    pub(crate) fn validate(self) -> Result<(), Error> {
        if !self.rotation_y_radians.is_finite()
            || !self.intensity.is_finite()
            || self.intensity < 0.
        {
            return Err(Error::InvalidInput(
                "environment yaw must be finite and intensity finite/nonnegative",
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct EnvironmentState {
    pub selected: Option<EnvironmentMap>,
    pub parameters: EnvironmentParameters,
    /// Primary misses show the selected map (Renderer::set_visible_environment).
    pub visible: bool,
    pub generation: u64,
    /// In-place procedural pixels changed; bindings and geometric visibility stay valid.
    pub content_revision: u64,
    pub limits: EnvironmentLimits,
    pub uploading: Vec<(u64, EnvironmentMap)>,
}
impl EnvironmentState {
    pub fn uniform(&self) -> [u8; 16] {
        let mut bytes = [0; 16];
        if let Some(map) = &self.selected {
            // Sum in f64 so two finite source angles cannot overflow before reduction.
            let angle = (f64::from(map.0.yaw) + f64::from(self.parameters.rotation_y_radians))
                .rem_euclid(std::f64::consts::TAU);
            let (sin, cos) = angle.sin_cos();
            for (dst, value) in bytes.chunks_exact_mut(4).zip([
                cos as f32,
                sin as f32,
                self.parameters.intensity,
                // WGSL `lighting.map.w`: -levels for an independent field, levels for a
                // legacy chain, levels + 0.5 for a banked legacy chain.
                if map.0.independent {
                    -(map.0.levels as f32)
                } else if map.0.banked {
                    map.0.levels as f32 + 0.5
                } else {
                    map.0.levels as f32
                },
            ]) {
                dst.copy_from_slice(&value.to_ne_bytes());
            }
        }
        bytes
    }
}

pub(crate) fn upload_bytes(
    data: &PreparedEnvironment,
    limits: EnvironmentLimits,
    device_dimension: u32,
) -> Result<u64, Error> {
    let dimension = limits.max_dimension.min(device_dimension);
    if data.diffuse_irradiance().size() > dimension
        || data.specular_radiance()[0].size() > dimension
    {
        return Err(Error::InvalidInput(
            "environment exceeds policy/device cube dimensions",
        ));
    }
    data.texel_count()
        .checked_mul(16)
        .filter(|n| *n <= limits.max_map_bytes && *n <= isize::MAX as u64)
        .ok_or(Error::InvalidInput(
            "environment exceeds GPU map byte/address limit",
        ))
}

/// The face size of a legacy chain's banked levels (`bank_legacy`). Every level narrower
/// than this is banked; shaders/scene.wgsl `environment_bank_start` uses the same rule.
pub(crate) const BANK_SIZE: u32 = 32;

/// P-SHOW2 (docs/p-show2.md): a legacy mip chain's levels narrower than BANK_SIZE, as
/// equal-size array banks of BANK_SIZE faces. Each texel is the level's own exact
/// cube-bilinear value (seams and corners included, as the shading's filtered cube
/// lookup reads them) at the texel's centre direction. On 8- to 1-texel levels almost every shading
/// lookup crossed a face edge into the slow cross-face path: Showcase's 16-texel Sky::Map
/// cost +38 ms of glossy shading at 1080p. In the banks almost every lookup stays inside
/// its face. Levels at least BANK_SIZE wide keep their exact mips. Returns the first banked
/// level and the packed RGBA32F banks, or None when no level is narrower.
pub(crate) fn bank_legacy(levels: &[CubeLevel]) -> Option<(usize, Vec<Vec<u8>>)> {
    let start = levels.iter().take_while(|l| l.size() >= BANK_SIZE).count();
    if start == levels.len() {
        return None;
    }
    let size = BANK_SIZE;
    let banks = levels[start..]
        .iter()
        .map(|level| {
            let mut bytes = Vec::with_capacity((6 * size * size * 16) as usize);
            for face in 0..6 {
                for y in 0..size {
                    for x in 0..size {
                        let uv = [x, y].map(|i| 2.0 * (i as f32 + 0.5) / size as f32 - 1.0);
                        let rgb = cube_bilinear(level, face_direction(face, uv));
                        for value in rgb.into_iter().chain([1.0]) {
                            bytes.extend_from_slice(&value.to_ne_bytes());
                        }
                    }
                }
            }
            bytes
        })
        .collect();
    Some((start, banks))
}

// CPU cube-bilinear lookup for the banks: WebGPU face selection, and three-face corners as
// seamless cube filtering reads them.
fn face_uv(d: [f32; 3]) -> (i32, [f32; 2]) {
    let a = d.map(f32::abs);
    if a[0] >= a[1] && a[0] >= a[2] {
        if d[0] > 0.0 {
            return (0, [-d[2] / a[0], -d[1] / a[0]]);
        }
        return (1, [d[2] / a[0], -d[1] / a[0]]);
    }
    if a[1] >= a[2] {
        if d[1] > 0.0 {
            return (2, [d[0] / a[1], d[2] / a[1]]);
        }
        return (3, [d[0] / a[1], -d[2] / a[1]]);
    }
    if d[2] > 0.0 {
        return (4, [d[0] / a[2], -d[1] / a[2]]);
    }
    (5, [-d[0] / a[2], -d[1] / a[2]])
}
fn face_direction(face: i32, uv: [f32; 2]) -> [f32; 3] {
    match face {
        0 => [1.0, -uv[1], -uv[0]],
        1 => [-1.0, -uv[1], uv[0]],
        2 => [uv[0], 1.0, uv[1]],
        3 => [uv[0], -1.0, -uv[1]],
        4 => [uv[0], -uv[1], 1.0],
        _ => [-uv[0], -uv[1], -1.0],
    }
}
fn load(level: &CubeLevel, face: i32, p: [i32; 2]) -> [f32; 3] {
    let n = level.size() as usize;
    level.texels()[face as usize * n * n + p[1] as usize * n + p[0] as usize]
}
fn remap(level: &CubeLevel, face: i32, p: [i32; 2]) -> [f32; 3] {
    let size = level.size() as i32;
    if p.iter().all(|&v| (0..size).contains(&v)) {
        return load(level, face, p);
    }
    let uv = p.map(|v| 2.0 * (v as f32 + 0.5) / size as f32 - 1.0);
    let (next, uv) = face_uv(face_direction(face, uv));
    let q = uv.map(|v| (((v + 1.0) * size as f32 * 0.5) as i32).clamp(0, size - 1));
    load(level, next, q)
}
fn tap(level: &CubeLevel, face: i32, p: [i32; 2]) -> [f32; 3] {
    let size = level.size() as i32;
    let outside = p.map(|v| v < 0 || v >= size);
    if !(outside[0] && outside[1]) {
        return remap(level, face, p);
    }
    let q = p.map(|v| v.clamp(0, size - 1));
    let points = [q, [p[0], q[1]], [q[0], p[1]]];
    let sum = points
        .iter()
        .map(|&point| remap(level, face, point))
        .fold([0.0; 3], |s, v| std::array::from_fn(|i| s[i] + v[i]));
    sum.map(|v| v / 3.0)
}
fn cube_bilinear(level: &CubeLevel, d: [f32; 3]) -> [f32; 3] {
    let size = level.size() as f32;
    let (face, uv) = face_uv(d);
    let xy = uv.map(|v| (v + 1.0) * size * 0.5 - 0.5);
    let p = xy.map(|v| v.floor() as i32);
    let t = [xy[0] - xy[0].floor(), xy[1] - xy[1].floor()];
    let [a, b, c, e] = [[0, 0], [1, 0], [0, 1], [1, 1]].map(|o| tap(level, face, [p[0] + o[0], p[1] + o[1]]));
    std::array::from_fn(|i| {
        (a[i] * (1.0 - t[0]) + b[i] * t[0]) * (1.0 - t[1]) + (c[i] * (1.0 - t[0]) + e[i] * t[0]) * t[1]
    })
}

pub(crate) fn pack_level(level: &CubeLevel) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(level.texels().len() * 16);
    for rgb in level.texels() {
        for value in rgb.iter().copied().chain([1.]) {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

#[cfg(test)]
mod bank_tests {
    use super::*;
    use renderlab_asset::environment::Limits;

    fn level(size: u32, texel: impl Fn(usize) -> [f32; 3]) -> CubeLevel {
        let texels = (0..6 * (size * size) as usize).map(texel).collect();
        CubeLevel::new(size, texels, Limits::default()).unwrap()
    }
    fn read(bank: &[u8], i: usize) -> [f32; 3] {
        std::array::from_fn(|c| f32::from_ne_bytes(bank[i * 16 + c * 4..i * 16 + c * 4 + 4].try_into().unwrap()))
    }

    /// Levels at least BANK_SIZE wide keep their mips; a narrower level's bank holds its own
    /// lookups at the bank's texel centres: a level linear in its texel coordinates reads
    /// back exactly inside each face, on every face (row-major, face-major), and a
    /// constant stays constant. Chains with no narrow level are not banked.
    #[test]
    fn legacy_chains_bank_their_narrow_levels() {
        let n = 16usize;
        let linear = level(16, |i| [(i % n) as f32, ((i % (n * n)) / n) as f32, (i / (n * n)) as f32]);
        let chain = [level(64, |_| [1.0; 3]), level(32, |_| [1.0; 3]), linear, level(8, |_| [3.0, 4.0, 5.0])];
        let (start, banks) = bank_legacy(&chain).unwrap();
        assert_eq!((start, banks.len()), (2, 2));
        let size = BANK_SIZE as usize;
        for face in 0..6 {
            for y in 1..size - 2 {
                for x in 1..size - 2 {
                    let got = read(&banks[0], face * size * size + y * size + x);
                    let want = [x, y].map(|v| (v as f32 + 0.5) / 2.0 - 0.5);
                    assert!((got[0] - want[0]).abs() < 1e-4 && (got[1] - want[1]).abs() < 1e-4, "{face} {x} {y}: {got:?}");
                    assert_eq!(got[2], face as f32);
                }
            }
        }
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|c| (a[c] - b[c]).abs() <= 1e-5 * b[c]);
        assert!((0..6 * size * size).all(|i| close(read(&banks[1], i), [3.0, 4.0, 5.0])));
        assert!(bank_legacy(&chain[..2]).is_none());
    }
}
