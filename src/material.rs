use crate::{
    Error,
    observe::{Allocation, Owned},
};
use renderlab_asset::{Filter, MipmapFilter, Sampler, TextureRole, Wrap};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
/// Diagnostics show stored material data and world-space authored normals, without lighting.
pub enum ViewMode {
    #[default]
    BaseColor,
    Normal,
    Metallic,
    Roughness,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum SampleCount {
    #[default]
    One = 1,
    Four = 4,
}
#[derive(Clone, Copy, Debug, Default)]
/// Fixed for a renderer lifetime. Pipeline reload retains this sample configuration.
pub struct RendererOptions {
    pub samples: SampleCount,
}

/// Immutable shared upload. Cloning retains ownership without allocating or uploading.
#[derive(Clone)]
pub struct Texture(pub(crate) Arc<TextureInner>);
pub(crate) struct TextureInner {
    pub renderer: Arc<()>,
    pub role: TextureRole,
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
    pub double_sided: bool,
}
impl Default for MaterialDescriptor<'_> {
    fn default() -> Self {
        Self {
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            double_sided: false,
        }
    }
}
/// Caller-owned material retains its textures, including when the upload handles are dropped.
pub struct Material {
    pub(crate) renderer: Arc<()>,
    pub(crate) double_sided: bool,
    pub(crate) bind: Owned<wgpu::BindGroup>,
    pub(crate) _uniform: Owned<wgpu::Buffer>,
    pub(crate) _samplers: [Owned<wgpu::Sampler>; 2],
    pub(crate) _textures: [Texture; 2],
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
        for (binding, role) in [
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
                if binding.texture.0.role != role {
                    return Err(Error::InvalidInput("material texture role mismatch"));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn bytes(&self) -> [u8; 32] {
        let mut bytes = [0; 32];
        for (dst, value) in bytes.chunks_exact_mut(4).zip(
            self.base_color
                .into_iter()
                .chain([self.metallic, self.roughness]),
        ) {
            dst.copy_from_slice(&value.to_ne_bytes());
        }
        bytes[24..28].copy_from_slice(&u32::from(self.double_sided).to_ne_bytes());
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
                    .and_then(|x| x.checked_mul(4))
        {
            return Err(Error::InvalidInput("texture mip dimensions/bytes mismatch"));
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
pub(crate) fn sampler_descriptor(s: Sampler) -> wgpu::SamplerDescriptor<'static> {
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
            0.0
        } else {
            32.0
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderlab_asset::{Mip, Texture as Data};
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
