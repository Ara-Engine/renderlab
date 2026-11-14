//! The lazy BC5 variance layer. A BC5 normal map uploads with one layer, and no variance work,
//! while no installed program reads normal variance (`program::reads_normal_variance`; the
//! standard and toon programs do not). Installing one that does gives every such map its
//! variance layer (`Renderer::ensure_variance_layers`), and the engine then refreshes its
//! materials (`Renderer::refresh_material`): the layer's bytes are the eager path's, so a frame
//! that reads variance sees what it always did. A child of renderer.
use super::*;
use crate::material::{TextureStorage, VarianceTexture};

impl Renderer {
    /// Whether an installed program reads normal variance.
    pub(super) fn variance_read(&self) -> bool {
        self.programs
            .iter()
            .any(|s| crate::program::reads_normal_variance(&s.source))
    }

    /// When an installed program reads normal variance, gives every one-layer BC5 normal map
    /// uploaded so far its variance layer. Each map's blocks are read back once (this waits for
    /// the device), the layers are computed on the CPU in parallel (`variance_layer`, the eager
    /// path's bytes), and a two-layer texture replaces the map for new bindings. True when any
    /// map changed: materials bound before need `refresh_material`.
    pub fn ensure_variance_layers(&mut self) -> Result<bool, Error> {
        self.check_fault()?;
        if self.variance_pending.is_empty() || !self.variance_read() {
            return Ok(false);
        }
        let pending: Vec<_> = std::mem::take(&mut self.variance_pending)
            .into_iter()
            .filter_map(|weak| weak.upgrade())
            .filter(|inner| !inner.variance_layer())
            .collect();
        let mut sources = Vec::with_capacity(pending.len());
        for inner in &pending {
            let TextureStorage::Full(full) = &inner.storage else {
                continue;
            };
            sources.push((inner.clone(), self.read_back_blocks(&full._texture, 0)?));
        }
        use rayon::prelude::*;
        let layers: Vec<_> = sources
            .par_iter()
            .map(|(_, data)| crate::normal_variance_bc::variance_layer(data))
            .collect();
        let scopes = OperationScopes::new(&self.device);
        for ((inner, data), layer) in sources.iter().zip(layers) {
            let TextureStorage::Full(full) = &inner.storage else {
                continue;
            };
            let (texture, view, bytes) = self.block_texture(data, &[&data.mips, &layer]);
            let _ = full.variance.set(VarianceTexture {
                _texture: texture,
                view,
                _allocation: self.ledger.allocate(AllocationKind::Texture, bytes),
            });
        }
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.queue.submit([]);
        Ok(!sources.is_empty())
    }

    /// A material bound before `ensure_variance_layers` changed one of its textures, bound
    /// again: its raster group over the current views, and its variance word (1 for a BC5
    /// normal map with its layer).
    pub fn refresh_material(&mut self, material: &mut Material) {
        let defaults = self.default_material.as_ref().unwrap()._textures.clone();
        let views = std::array::from_fn(|i| {
            let t = &material._textures[i];
            self.texture_view(if matches!(t.0.storage, TextureStorage::Streamed(_)) {
                &defaults[i]
            } else {
                t
            })
        });
        material.bind = self.material_group(
            &material._uniform,
            views,
            &material._samplers,
            &material.parameters.buffer,
        );
        if material.normal_mapped && material._textures[2].0.variance_layer() {
            // MaterialDescriptor::bytes' `normal_variance` word.
            self.queue
                .write_buffer(&material._uniform, 40, &1u32.to_ne_bytes());
            self.uploads.materials += 4;
            self.uploads.write_calls += 1;
        }
    }

    /// A BC5 texture's blocks, every level of one array layer, read back (waits for the device).
    pub(crate) fn read_back_blocks(
        &self,
        texture: &wgpu::Texture,
        layer: u32,
    ) -> Result<asset::CompressedTexture, Error> {
        let (width, height) = (texture.width(), texture.height());
        let block = asset::BlockFormat::BLOCK;
        let mut layout = vec![];
        let mut size = 0u64;
        for level in 0..texture.mip_level_count() {
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            let row = asset::BlockFormat::row_bytes(w);
            let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let rows = h.div_ceil(block);
            layout.push((w, h, row, padded, rows, size));
            size += u64::from(padded) * u64::from(rows);
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("BC5 blocks readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (level, &(w, h, _, padded, rows, offset)) in layout.iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    ..texture.as_image_copy()
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(rows),
                    },
                },
                wgpu::Extent3d {
                    width: w.div_ceil(block) * block,
                    height: h.div_ceil(block) * block,
                    depth_or_array_layers: 1,
                },
            );
        }
        let index = self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .map_err(Error::Poll)?;
        let mips = {
            let mapped = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|_| Error::InvalidInput("BC5 blocks readback did not map"))?;
            layout
                .iter()
                .map(|&(_, _, row, padded, rows, offset)| {
                    (0..rows)
                        .flat_map(|r| {
                            let at = (offset + u64::from(r * padded)) as usize;
                            mapped[at..at + row as usize].iter().copied()
                        })
                        .collect()
                })
                .collect()
        };
        buffer.unmap();
        Ok(asset::CompressedTexture {
            format: asset::BlockFormat::Bc5,
            width,
            height,
            mips,
        })
    }
}
