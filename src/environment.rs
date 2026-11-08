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
                if map.0.independent {
                    -(map.0.levels as f32)
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

pub(crate) fn pack_level(level: &CubeLevel) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(level.texels().len() * 16);
    for rgb in level.texels() {
        for value in rgb.iter().copied().chain([1.]) {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}
