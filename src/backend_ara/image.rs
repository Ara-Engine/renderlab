//! Typed image identities and publication; the caller owns device retirement.
use super::graph::{ResourceUse, Subresources};
use anyhow::{Result, ensure};
use ara_gpu::{
    Device, GpuDevice, Image,
    descriptors::{Format, ImageDesc},
    texture::{ImageShape, ImageViewDesc, TextureDesc, ViewDimension},
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
    /// Published view shape; all views retain the parent resource identity.
    pub shape: ImageShape,
    /// Full allocation bounds for graph range accounting.
    pub allocation: Subresources,
    /// Published mip/layer interval, including one-layer array dimensionality.
    pub view: ImageViewDesc,
    pub(crate) sampled: u32,
    pub(crate) storage: Option<u32>,
}
impl Texture {
    /// Declare exactly the published view, retaining its parent allocation identity.
    pub fn usage(&self, access: ara_gpu::Access) -> ResourceUse {
        ResourceUse::Image {
            image: self.image,
            allocation: self.allocation,
            view: self.view,
            access,
        }
    }
    /// Allocate one 2D mip chain and publish separate sampled/storage views per level.
    /// Retire the returned parent exactly once; views share that allocation.
    pub fn mip_chain(
        gpu: &mut GpuDevice,
        size: [u32; 2],
        levels: u32,
        format: Format,
    ) -> Result<(Self, Vec<Self>)> {
        let shape = ImageShape {
            mip_levels: levels,
            ..Default::default()
        };
        let image = gpu.create_texture(&TextureDesc {
            image: ImageDesc {
                size,
                format,
                sampled: true,
                storage: true,
                attachment: false,
                copy_source: true,
                copy_destination: true,
            },
            shape,
        })?;
        let sampled = match gpu.sampled_image(image, false) {
            Ok(v) => v.index,
            Err(e) => {
                gpu.retire(ara_gpu::Resource::Image(image))?;
                return Err(e.into());
            }
        };
        let parent = Self {
            image,
            size,
            format,
            shape,
            allocation: Subresources {
                mips: levels,
                layers: 1,
            },
            view: ImageViewDesc {
                dimension: ViewDimension::D2,
                base_mip: 0,
                mip_count: levels,
                base_layer: 0,
                layer_count: 1,
            },
            sampled,
            storage: None,
        };
        let result: Result<Vec<Self>> = (0..levels)
            .map(|mip| {
                let view = ImageViewDesc {
                    dimension: ViewDimension::D2,
                    base_mip: mip,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                };
                Ok(Self {
                    image: parent.image,
                    size: size.map(|n| (n >> mip).max(1)),
                    format,
                    shape: ImageShape::default(),
                    allocation: parent.allocation,
                    view,
                    sampled: gpu.sampled_view(parent.image, view)?.index,
                    storage: Some(gpu.storage_view(parent.image, view)?.index),
                })
            })
            .collect();
        match result {
            Ok(views) => Ok((parent, views)),
            Err(e) => {
                gpu.retire(ara_gpu::Resource::Image(parent.image))?;
                Err(e)
            }
        }
    }
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
                shape: ImageShape {
                    layers: 6,
                    mip_levels: levels,
                    ..Default::default()
                },
                allocation: Subresources {
                    mips: levels,
                    layers: 6,
                },
                view: ImageViewDesc {
                    dimension: ViewDimension::D2Array,
                    base_mip: 0,
                    mip_count: levels,
                    base_layer: 0,
                    layer_count: 6,
                },
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
                    shape: ImageShape {
                        layers: 6,
                        mip_levels: 1,
                        ..Default::default()
                    },
                    allocation: Subresources {
                        mips: levels,
                        layers: 6,
                    },
                    view: desc,
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
    /// Publish an explicit 2D-array view, including a single-layer array.
    pub fn array(
        gpu: &mut GpuDevice,
        size: [u32; 2],
        layers: u32,
        format: Format,
        storage: bool,
        attachment: bool,
    ) -> Result<Self> {
        let shape = ImageShape {
            layers,
            ..Default::default()
        };
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
        let desc = ImageViewDesc {
            dimension: ViewDimension::D2Array,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: layers,
        };
        let result = (|| -> Result<Self> {
            Ok(Self {
                image,
                size,
                format,
                shape,
                allocation: Subresources { mips: 1, layers },
                view: desc,
                sampled: gpu.sampled_view(image, desc)?.index,
                storage: if storage {
                    Some(gpu.storage_view(image, desc)?.index)
                } else {
                    None
                },
            })
        })();
        if result.is_err() {
            gpu.retire(ara_gpu::Resource::Image(image))?;
        }
        result
    }
    /// Publish one sampled 2D mip/slice inside this view. The returned identity
    /// shares the parent's retirement and preserves full allocation bounds.
    pub fn view_2d(&self, gpu: &mut GpuDevice, mip: u32, layer: u32) -> Result<Self> {
        ensure!(
            self.shape.dimension == ara_gpu::texture::TextureDimension::D2
                && mip >= self.view.base_mip
                && mip - self.view.base_mip < self.view.mip_count
                && layer >= self.view.base_layer
                && layer - self.view.base_layer < self.view.layer_count,
            "2D view exceeds its parent mip/layer interval"
        );
        let view = ImageViewDesc {
            dimension: ViewDimension::D2,
            base_mip: mip,
            mip_count: 1,
            base_layer: layer,
            layer_count: 1,
        };
        let sampled = gpu.sampled_view(self.image, view)?.index;
        Ok(Self {
            image: self.image,
            size: self.size.map(|n| (n >> (mip - self.view.base_mip)).max(1)),
            format: self.format,
            shape: ImageShape {
                samples: self.shape.samples,
                ..Default::default()
            },
            allocation: self.allocation,
            view,
            sampled,
            storage: None,
        })
    }
    /// Allocate a reversed-depth atlas and one narrow 2D view per mip/layer.
    /// Views are ordered mip first, then layer; retire the returned parent once.
    pub fn depth_atlas(
        gpu: &mut GpuDevice,
        size: [u32; 2],
        layers: u32,
        levels: u32,
    ) -> Result<(Self, Vec<Self>)> {
        let parent = Self::shaped(
            gpu,
            size,
            Format::Depth32Float,
            false,
            true,
            ImageShape {
                layers,
                mip_levels: levels,
                ..Default::default()
            },
        )?;
        let result = (|| -> Result<Vec<Self>> {
            let mut views = Vec::new();
            for mip in 0..levels {
                for layer in 0..layers {
                    views.push(parent.view_2d(gpu, mip, layer)?);
                }
            }
            Ok(views)
        })();
        match result {
            Ok(views) => Ok((parent, views)),
            Err(error) => {
                gpu.retire(ara_gpu::Resource::Image(parent.image))?;
                Err(error)
            }
        }
    }
    /// Allocate a sampled single-mip 3D colour LUT.
    pub fn volume(gpu: &mut GpuDevice, side: u32, format: Format) -> Result<Self> {
        Self::volume_shape(gpu, [side; 3], format, false)
    }
    /// Allocate a sampled non-cubic volume, optionally publishing a writable view.
    pub fn volume_shape(
        gpu: &mut GpuDevice,
        size: [u32; 3],
        format: Format,
        storage: bool,
    ) -> Result<Self> {
        Self::shaped(
            gpu,
            [size[0], size[1]],
            format,
            storage,
            false,
            ImageShape {
                dimension: ara_gpu::texture::TextureDimension::D3,
                depth: size[2],
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
                shape,
                allocation: Subresources {
                    mips: shape.mip_levels,
                    layers: shape.layers,
                },
                view: ImageViewDesc {
                    dimension: if shape.dimension == ara_gpu::texture::TextureDimension::D3 {
                        ViewDimension::D3
                    } else if shape.layers > 1 {
                        ViewDimension::D2Array
                    } else {
                        ViewDimension::D2
                    },
                    base_mip: 0,
                    mip_count: shape.mip_levels,
                    base_layer: 0,
                    layer_count: shape.layers,
                },
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
