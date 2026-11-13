use crate::{
    Error,
    observe::{Allocation, Owned},
    program::{MaterialParameters, Program, Value},
};
use renderlab_asset::{Filter, MipmapFilter, Sampler, TextureRole, Wrap};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u32)]
/// What a frame shows (docs/design-id-output.md). `Lit` is the program's `shade`. The data
/// modes show one value of the program's surface per pixel (its optional `view_data` hook,
/// else its material inputs), exactly: no temporal reconstruction, fog or tone mapping.
/// `LightingOnly` lights a white Lambert surface at the program's normal and is lit (it is
/// reconstructed and tone-mapped). Traced and visibility-buffer frames shade every mode
/// through the program; the raster path shows modes 0-3 from material data.
pub enum ViewMode {
    #[default]
    BaseColor,
    /// The shading normal, world space, as `n * 0.5 + 0.5`.
    Normal,
    Metallic,
    Roughness,
    Lit = 4,
    /// The surface's own radiance (unexposed).
    Emission,
    /// The program's occlusion output (`ViewData::occlusion`), 1 where it declares none.
    Occlusion,
    /// White albedo lit at the program's normal by the frame's lights and indirect light.
    LightingOnly,
}
impl ViewMode {
    /// Every mode, in discriminant order.
    pub const ALL: [ViewMode; 8] = [
        ViewMode::BaseColor,
        ViewMode::Normal,
        ViewMode::Metallic,
        ViewMode::Roughness,
        ViewMode::Lit,
        ViewMode::Emission,
        ViewMode::Occlusion,
        ViewMode::LightingOnly,
    ];
    /// Shows exact values: no temporal reconstruction, transparency, fog or tone mapping.
    pub fn is_data(self) -> bool {
        !matches!(self, ViewMode::Lit | ViewMode::LightingOnly)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum SampleCount {
    #[default]
    One = 1,
    Four = 4,
}
#[derive(Clone, Copy, Debug)]
/// Fixed for a renderer lifetime, including across shader reloads.
pub struct RendererOptions {
    pub samples: SampleCount,
    /// 1 disables anisotropy; 2..=16 enhances all-linear mipmapped bindings only.
    /// Explicit nearest and disabled-mip samplers retain their authored behavior.
    pub anisotropy_clamp: u16,
}
impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            samples: SampleCount::One,
            anisotropy_clamp: 1,
        }
    }
}

/// Immutable shared upload. Cloning retains ownership without allocating or uploading.
#[derive(Clone)]
pub struct Texture(pub(crate) Arc<TextureInner>);
pub(crate) struct TextureInner {
    pub renderer: Arc<()>,
    pub role: TextureRole,
    pub storage: TextureStorage,
    /// The block format of a compressed upload (`Renderer::try_upload_compressed_texture`);
    /// None for texel uploads. BC5 normal maps hold X and Y only.
    pub block: Option<renderlab_asset::BlockFormat>,
}
pub(crate) enum TextureStorage {
    /// Every level, immutable after upload.
    Full(Box<FullTexture>),
    /// Resident levels live in the renderer's residency table under this id
    /// (`texture_residency.rs`); the table drops them with the last handle.
    Streamed(u64),
}
pub(crate) struct FullTexture {
    pub _texture: Owned<wgpu::Texture>,
    pub view: Owned<wgpu::TextureView>,
    pub _allocation: Allocation,
}
#[derive(Clone, Copy)]
pub struct TextureBinding<'a> {
    pub texture: &'a Texture,
    pub sampler: Sampler,
}
pub struct MaterialDescriptor<'a> {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub base_color_texture: Option<TextureBinding<'a>>,
    pub metallic_roughness_texture: Option<TextureBinding<'a>>,
    pub normal_texture: Option<TextureBinding<'a>>,
    pub normal_scale: f32,
    pub double_sided: bool,
    /// Shadow rays may leave either side toward the light. False keeps opaque
    /// receivers on the camera-viewed side, independently of the shading normal.
    pub thin: bool,
    /// The material's shading program; None is the renderer's standard program.
    pub program: Option<&'a Program>,
    /// Named values for the program's `struct Parameters`, checked by name and type.
    /// Omitted members use the program's DEFAULT_PARAMETERS (zero when absent).
    pub parameters: &'a [(&'a str, Value)],
    /// TP1: Some draws this material in the transparent pass instead of the opaque paths
    /// (`crate::Transparency`). None is opaque.
    pub transparency: Option<crate::Transparency>,
}
impl Default for MaterialDescriptor<'_> {
    fn default() -> Self {
        Self {
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.,
            double_sided: false,
            thin: false,
            program: None,
            parameters: &[],
            transparency: None,
        }
    }
}
/// Caller-owned material retains its textures, including when the upload handles are dropped.
pub struct Material {
    pub(crate) renderer: Arc<()>,
    pub(crate) normal_mapped: bool,
    pub(crate) double_sided: bool,
    pub(crate) thin: bool,
    /// TP1: transparent materials draw only in the transparent pass.
    pub(crate) transparency: Option<crate::Transparency>,
    /// Raster group 2. Streamed textures bind the renderer's 1x1 defaults here, so this
    /// group never retains their levels; passes that sample build a group of the current
    /// levels instead (`Renderer::sampling_group`).
    pub(crate) bind: Owned<wgpu::BindGroup>,
    /// Whether any of its textures streams (`texture_residency.rs`).
    pub(crate) streamed: bool,
    /// Per texture role, the levels a streamed texture keeps above the finest level its
    /// samples touch: 0, or `ANISOTROPIC_MARGIN` for anisotropic samplers.
    pub(crate) margins: [u32; 3],
    pub(crate) _uniform: Owned<wgpu::Buffer>,
    pub(crate) _samplers: [Owned<wgpu::Sampler>; 3],
    pub(crate) _textures: [Texture; 3],
    /// The resolved program, retained while the material lives.
    pub(crate) program: Program,
    pub(crate) parameters: Arc<MaterialParameters>,
    pub(crate) _allocation: Allocation,
}
impl MaterialDescriptor<'_> {
    pub(crate) fn validate(&self, renderer: &Arc<()>) -> Result<(), Error> {
        if !self
            .base_color
            .iter()
            .chain([&self.metallic, &self.roughness])
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        {
            return Err(Error::InvalidInput(
                "material factors must be finite in [0,1]",
            ));
        }
        if !self.normal_scale.is_finite() {
            return Err(Error::InvalidInput("normal scale must be finite"));
        }
        if self.transparency.is_some_and(|t| !t.valid()) {
            return Err(Error::InvalidInput(
                "transparency needs opacity in [0,1], IOR in [1,3] and thickness in [0,1] m",
            ));
        }
        for (binding, role) in [
            (self.normal_texture, TextureRole::NormalLinear),
            (self.base_color_texture, TextureRole::BaseColorSrgb),
            (
                self.metallic_roughness_texture,
                TextureRole::MetallicRoughnessLinear,
            ),
        ] {
            if let Some(binding) = binding {
                if !Arc::ptr_eq(renderer, &binding.texture.0.renderer) {
                    return Err(Error::InvalidInput("texture belongs to another renderer"));
                }
                if binding.texture.0.role != role
                    && !(role == TextureRole::NormalLinear && binding.texture.0.role.is_normal())
                {
                    return Err(Error::InvalidInput("material texture role mismatch"));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn bytes(&self) -> [u8; 48] {
        let mut bytes = [0; 48];
        for (dst, value) in bytes.chunks_exact_mut(4).zip(
            self.base_color
                .into_iter()
                .chain([self.metallic, self.roughness]),
        ) {
            dst.copy_from_slice(&value.to_ne_bytes());
        }
        bytes[24..28].copy_from_slice(&u32::from(self.double_sided).to_ne_bytes());
        bytes[28..32].copy_from_slice(&u32::from(self.thin).to_ne_bytes());
        bytes[32..36].copy_from_slice(&self.normal_scale.to_ne_bytes());
        // 1: a normal map; 2: a BC5 one, whose Z the shading programs rebuild
        // (program_api.wgsl rl_normal_texel).
        let normal_mapped = self.normal_texture.map_or(0u32, |t| {
            if t.texture.0.block == Some(renderlab_asset::BlockFormat::Bc5) {
                2
            } else {
                1
            }
        });
        bytes[36..40].copy_from_slice(&normal_mapped.to_ne_bytes());
        let variance = self
            .normal_texture
            .map_or(0u32, |t| match t.texture.0.role {
                TextureRole::NormalVarianceLinear => 1,
                TextureRole::NormalSlopeMoments => 2,
                _ => 0,
            });
        bytes[40..44].copy_from_slice(&variance.to_ne_bytes());
        // TP1: the last word, zero for opaque materials (WGSL `transparent_opacity`).
        bytes[44..48]
            .copy_from_slice(&crate::Transparency::word(self.transparency.as_ref()).to_ne_bytes());
        bytes
    }
}
pub(crate) fn validate_texture(
    data: &renderlab_asset::Texture,
    max_dimension: u32,
    max_bytes: u64,
) -> Result<u64, Error> {
    let first = data
        .mips
        .first()
        .ok_or(Error::InvalidInput("empty texture mip chain"))?;
    let (mut w, mut h) = (first.width, first.height);
    if w == 0 || h == 0 || w > max_dimension || h > max_dimension {
        return Err(Error::InvalidInput(
            "texture dimensions exceed device limit",
        ));
    }
    let mut bytes = 0u64;
    for (i, mip) in data.mips.iter().enumerate() {
        if mip.width != w
            || mip.height != h
            || Some(mip.rgba8.len() as u64)
                != u64::from(w)
                    .checked_mul(u64::from(h))
                    .and_then(|x| x.checked_mul(data.role.texel_bytes() as u64))
        {
            return Err(Error::InvalidInput("texture mip dimensions/bytes mismatch"));
        }
        if !data.role.texels_finite(&mip.rgba8) {
            return Err(Error::InvalidInput("nonfinite texture texel"));
        }
        bytes = bytes
            .checked_add(mip.rgba8.len() as u64)
            .filter(|v| *v <= max_bytes)
            .ok_or(Error::InvalidInput("texture exceeds max_texture_bytes"))?;
        if w == 1 && h == 1 {
            return if i + 1 == data.mips.len() {
                Ok(bytes)
            } else {
                Err(Error::InvalidInput("mips after final 1x1"))
            };
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    Err(Error::InvalidInput("incomplete texture mip chain"))
}
pub(crate) fn sampler_descriptor(
    s: Sampler,
    anisotropy_clamp: u16,
) -> wgpu::SamplerDescriptor<'static> {
    let wrap = |v| match v {
        Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
        Wrap::Repeat => wgpu::AddressMode::Repeat,
    };
    let filter = |v| match v {
        Filter::Nearest => wgpu::FilterMode::Nearest,
        Filter::Linear => wgpu::FilterMode::Linear,
    };
    wgpu::SamplerDescriptor {
        label: Some("material sampler"),
        address_mode_u: wrap(s.wrap_u),
        address_mode_v: wrap(s.wrap_v),
        mag_filter: filter(s.mag),
        min_filter: filter(s.min),
        mipmap_filter: match s.mipmap {
            MipmapFilter::Linear => wgpu::MipmapFilterMode::Linear,
            _ => wgpu::MipmapFilterMode::Nearest,
        },
        lod_max_clamp: if s.mipmap == MipmapFilter::None {
            // Vulkan chooses min/mag after LOD clamping; 0.25 still selects base mip.
            0.25
        } else {
            32.0
        },
        anisotropy_clamp: if s.mag == Filter::Linear
            && s.min == Filter::Linear
            && s.mipmap == MipmapFilter::Linear
        {
            anisotropy_clamp
        } else {
            1
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderlab_asset::{Mip, Texture as Data};
    #[test]
    fn thin_is_explicit_and_independent_of_double_sided_shading() {
        let mut descriptor = MaterialDescriptor::default();
        assert!(!descriptor.thin);
        assert_eq!(&descriptor.bytes()[28..32], &0u32.to_ne_bytes());
        descriptor.double_sided = true;
        assert_eq!(&descriptor.bytes()[28..32], &0u32.to_ne_bytes());
        descriptor.thin = true;
        assert_eq!(&descriptor.bytes()[28..32], &1u32.to_ne_bytes());
        assert_eq!(descriptor.bytes().len(), 48);
    }
    #[test]
    fn anisotropy_preserves_explicit_sampler_choices() {
        let linear = Sampler::default();
        assert_eq!(sampler_descriptor(linear, 16).anisotropy_clamp, 16);
        assert_eq!(sampler_descriptor(linear, 1).anisotropy_clamp, 1);
        for authored in [
            Sampler {
                mag: Filter::Nearest,
                ..linear
            },
            Sampler {
                min: Filter::Nearest,
                ..linear
            },
            Sampler {
                mipmap: MipmapFilter::Nearest,
                ..linear
            },
            Sampler {
                mipmap: MipmapFilter::None,
                ..linear
            },
        ] {
            assert_eq!(sampler_descriptor(authored, 16).anisotropy_clamp, 1);
        }
        assert_eq!(
            sampler_descriptor(
                Sampler {
                    mipmap: MipmapFilter::None,
                    ..linear
                },
                16
            )
            .lod_max_clamp,
            0.25
        );
    }
    #[test]
    fn texture_admission_checks_complete_odd_mips_before_allocation() {
        let mut data = Data {
            role: TextureRole::BaseColorSrgb,
            mips: vec![
                Mip {
                    width: 3,
                    height: 5,
                    rgba8: vec![0; 60],
                },
                Mip {
                    width: 1,
                    height: 2,
                    rgba8: vec![0; 8],
                },
                Mip {
                    width: 1,
                    height: 1,
                    rgba8: vec![0; 4],
                },
            ],
        };
        assert_eq!(validate_texture(&data, 5, 72).unwrap(), 72);
        assert!(validate_texture(&data, 4, 72).is_err());
        assert!(validate_texture(&data, 5, 71).is_err());
        data.mips.pop();
        assert!(validate_texture(&data, 5, 72).is_err());
        data.mips[1].rgba8.pop();
        assert!(validate_texture(&data, 5, 72).is_err());
    }
}
