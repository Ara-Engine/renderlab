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
    /// A legacy mip chain stored as equal-size banks (`bank_legacy`).
    pub banked: bool,
    pub yaw: f32,
    pub levels: u32,
    pub hash: [u8; 32],
}
pub(crate) struct EnvironmentCube {
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

/// Legacy mip chains with level-0 faces at most this wide are banked at load.
pub(crate) const BANK_MAX_SIZE: u32 = 64;
/// Banked levels are at least this wide.
const BANK_MIN_SIZE: u32 = 32;

/// P-SHOW2 (docs/p-show2.md): a small legacy mip chain as equal-size array banks, one per
/// level, each texel the chain's own exact cube-bilinear value (shaders/scene.wgsl
/// `environment_bilinear`, seams and corners included) at the texel's centre direction.
/// The chain's 8- to 1-texel levels sent almost every shading lookup through the
/// cross-face slow path (Showcase's Sky::Map: +38 ms of glossy shading at 1080p). In banks
/// at least BANK_MIN_SIZE wide, almost every lookup stays inside its face. The bank size
/// and the packed RGBA32F levels, or None for larger chains, which keep their mips.
pub(crate) fn bank_legacy(levels: &[CubeLevel]) -> Option<(u32, Vec<Vec<u8>>)> {
    let first = levels.first()?.size();
    if first > BANK_MAX_SIZE {
        return None;
    }
    let size = first.max(BANK_MIN_SIZE);
    let banks = levels
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
    Some((size, banks))
}

// CPU mirrors of shaders/scene.wgsl's face selection and exact cube-bilinear lookup.
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

    /// Banking reproduces a full-size level's texels (its centres are its own exact
    /// lookups), keeps a constant level constant, and leaves large chains as mips.
    #[test]
    fn small_legacy_chains_bank_their_own_lookups() {
        let full = level(32, |i| [i as f32, (i % 7) as f32, 1.0]);
        let constant = level(2, |_| [3.0, 4.0, 5.0]);
        let reference = full.texels().to_vec();
        let (size, banks) = bank_legacy(&[full, constant]).unwrap();
        assert_eq!((size, banks.len()), (32, 2));
        for (i, texel) in reference.iter().enumerate() {
            let got = read(&banks[0], i);
            assert!((0..3).all(|c| (got[c] - texel[c]).abs() <= 1e-3 * texel[c].max(1.0)), "{i}: {got:?} {texel:?}");
        }
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|c| (a[c] - b[c]).abs() <= 1e-5 * b[c]);
        assert!((0..6 * 32 * 32).all(|i| close(read(&banks[1], i), [3.0, 4.0, 5.0])));
        assert!(bank_legacy(&[level(128, |_| [1.0; 3])]).is_none());
    }
}
