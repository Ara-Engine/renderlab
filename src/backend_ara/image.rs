//! Typed image identities and publication; the caller owns device retirement.
use anyhow::{Result, ensure};
use ara_gpu::{
    Device, GpuDevice, Image,
    descriptors::{Format, ImageDesc},
    texture::{ImageShape, TextureDesc},
};

/// One whole image with immutable heap descriptors; retire its handle after last use.
#[derive(Clone, Copy)]
pub struct Texture {
    /// Native arena identity used for commands, synchronization and retirement.
    pub image: Image,
    /// Base width and height.
    pub size: [u32; 2],
    /// Texel representation.
    pub format: Format,
    pub(crate) sampled: u32,
    pub(crate) storage: Option<u32>,
}
impl Texture {
    /// Allocate one layered image with all mips, publishing one narrow storage view
    /// per level. Every view shares the parent identity for barriers and retirement.
    pub fn cube_chain(gpu: &mut GpuDevice, side: u32, levels: u32) -> Result<(Self, Vec<Self>)> {
        use ara_gpu::texture::{ImageViewDesc, ViewDimension};
        let format = Format::Rgba32Float;
        let image = gpu.create_texture(&TextureDesc {
            image: ImageDesc {
                size: [side; 2],
                format,
                sampled: true,
                storage: true,
                attachment: false,
                copy_source: true,
                copy_destination: true,
            },
            shape: ImageShape {
                layers: 6,
                mip_levels: levels,
                ..Default::default()
            },
        })?;
        let result = (|| -> Result<_> {
            let parent = Self {
                image,
                size: [side; 2],
                format,
                sampled: gpu.sampled_image(image, false)?.index,
                storage: None,
            };
            let mut views = Vec::new();
            for mip in 0..levels {
                let desc = ImageViewDesc {
                    dimension: ViewDimension::D2Array,
                    base_mip: mip,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 6,
                };
                views.push(Self {
                    image,
                    size: [(side >> mip).max(1); 2],
                    format,
                    sampled: gpu.sampled_view(image, desc)?.index,
                    storage: Some(gpu.storage_view(image, desc)?.index),
                });
            }
            Ok((parent, views))
        })();
        if result.is_err() {
            gpu.retire(ara_gpu::Resource::Image(image))?;
        }
        result
    }
    /// Allocate a sampled single-mip 3D colour LUT.
    pub fn volume(gpu: &mut GpuDevice, side: u32, format: Format) -> Result<Self> {
        Self::shaped(
            gpu,
            [side; 2],
            format,
            false,
            false,
            ImageShape {
                dimension: ara_gpu::texture::TextureDimension::D3,
                depth: side,
                ..Default::default()
            },
        )
    }
    /// Allocate a single-level post image with copy and sampled access.
    pub fn new(
        gpu: &mut GpuDevice,
        size: [u32; 2],
        format: Format,
        storage: bool,
        attachment: bool,
    ) -> Result<Self> {
        Self::shaped(
            gpu,
            size,
            format,
            storage,
            attachment,
            ImageShape::default(),
        )
    }
    pub(crate) fn shaped(
        gpu: &mut GpuDevice,
        size: [u32; 2],
        format: Format,
        storage: bool,
        attachment: bool,
        shape: ImageShape,
    ) -> Result<Self> {
        ensure!(
            size.into_iter().all(|n| n > 0),
            "image extent must be nonzero"
        );
        let image = gpu.create_texture(&TextureDesc {
            image: ImageDesc {
                size,
                format,
                sampled: true,
                storage,
                attachment,
                copy_source: true,
                copy_destination: true,
            },
            shape,
        })?;
        let published = (|| -> Result<_> {
            Ok((
                gpu.sampled_image(image, false)?.index,
                if storage {
                    Some(gpu.storage_image(image)?.index)
                } else {
                    None
                },
            ))
        })();
        match published {
            Ok((sampled, storage)) => Ok(Self {
                image,
                size,
                format,
                sampled,
                storage,
            }),
            Err(e) => {
                gpu.retire(ara_gpu::Resource::Image(image))?;
                Err(e)
            }
        }
    }
}
