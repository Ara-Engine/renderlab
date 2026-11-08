//! Lossless RLA mip upload for Ara's native API. No BC conversion in a matched row.
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Barrier, Buffer, Device, GpuDevice, Image, Queue, Resource, commands::Command, vk,
};
use ara_render::instanced::{TextureBinding, TextureStats, Textures};
use renderlab_asset::{Texture, TextureRole};

pub struct RlaTextures {
    source: Vec<Texture>,
    digest: [u8; 32],
    images: Vec<Image>,
    table: Option<(Buffer, TextureBinding)>,
    bytes: u64,
}
impl RlaTextures {
    pub fn new(source: Vec<Texture>, digest: [u8; 32]) -> Self {
        Self {
            source,
            digest,
            images: Vec::new(),
            table: None,
            bytes: 0,
        }
    }
}
fn barrier(resource: Resource, before: Access, after: Access) -> Barrier {
    Barrier {
        resource,
        before,
        after,
        handoff: None,
    }
}
impl Textures for RlaTextures {
    fn content_digest(&self) -> [u8; 32] {
        self.digest
    }
    fn initialize(&mut self, gpu: &mut GpuDevice) -> Result<TextureStats> {
        if self.source.is_empty() {
            return Ok(TextureStats::default());
        }
        let mut rows = Vec::<[u32; 8]>::new();
        // Static capture scenes need the GPU copy only; release each CPU mip
        // chain as its upload completes instead of retaining a second resident set.
        for texture in std::mem::take(&mut self.source) {
            ensure!(
                matches!(
                    texture.role,
                    TextureRole::BaseColorSrgb
                        | TextureRole::MetallicRoughnessLinear
                        | TextureRole::NormalLinear
                ),
                "ARA1 does not silently reinterpret variance/moment normal textures"
            );
            let mip = &texture.mips[0];
            let format = if texture.role == TextureRole::BaseColorSrgb {
                vk::Format::R8G8B8A8_SRGB
            } else {
                vk::Format::R8G8B8A8_UNORM
            };
            let image = gpu.create_mip_image(
                [mip.width, mip.height],
                format,
                texture.mips.len() as u32,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            )?;
            let payload: Vec<_> = texture
                .mips
                .iter()
                .flat_map(|m| m.rgba8.iter().copied())
                .collect();
            self.bytes += payload.len() as u64;
            let stage = gpu.create_buffer(
                payload.len() as u64,
                vk::BufferUsageFlags::TRANSFER_SRC,
                true,
            )?;
            gpu.upload(stage, &payload)?;
            let mut commands = gpu.begin(Queue::Graphics);
            commands.barriers([
                barrier(
                    Resource::Buffer(stage),
                    Access::HostWrite,
                    Access::CopySource,
                ),
                barrier(
                    Resource::Image(image),
                    Access::Undefined,
                    Access::CopyDestination,
                ),
            ]);
            commands.push(Command::UploadImage {
                source: stage,
                destination: image,
            });
            commands.barriers([barrier(
                Resource::Image(image),
                Access::CopyDestination,
                Access::ShaderRead,
            )]);
            let point = gpu.submit(commands.finish(), &[])?;
            gpu.resolve(point)?;
            gpu.retire(Resource::Buffer(stage))?;
            // Native ABI: slot, first mip, original extent, encoding, levels, reserved.
            rows.push([
                gpu.sampled_image(image, false)?.index,
                0,
                mip.width,
                mip.height,
                0,
                texture.mips.len() as u32,
                0,
                0,
            ]);
            self.images.push(image);
        }
        let table = gpu.create_buffer(
            (rows.len() * 32) as u64,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            true,
        )?;
        gpu.upload(table, bytemuck::cast_slice(&rows))?;
        let mut commands = gpu.begin(Queue::Graphics);
        commands.barriers([barrier(
            Resource::Buffer(table),
            Access::HostWrite,
            Access::ShaderRead,
        )]);
        let point = gpu.submit(commands.finish(), &[])?;
        gpu.resolve(point)?;
        self.table = Some((
            table,
            TextureBinding {
                buffer_index: gpu.storage_buffer(table)?.index,
                textures: rows.len() as u32,
            },
        ));
        Ok(TextureStats {
            textures: rows.len(),
            resident_bytes: self.bytes,
            requested_bytes: self.bytes,
            uploaded_bytes: self.bytes,
            ..Default::default()
        })
    }
    fn refine(&mut self, _: &mut GpuDevice) -> Result<TextureStats> {
        Ok(TextureStats {
            textures: self.images.len(),
            resident_bytes: self.bytes,
            requested_bytes: self.bytes,
            ..Default::default()
        })
    }
    fn binding(&self) -> Option<TextureBinding> {
        self.table.map(|(_, binding)| binding)
    }
    fn visit_resources(&self, visit: &mut dyn FnMut(Resource, Access)) {
        if let Some((table, _)) = self.table {
            visit(Resource::Buffer(table), Access::ShaderRead);
        }
        for &image in &self.images {
            visit(Resource::Image(image), Access::ShaderRead);
        }
    }
    fn destroy(self: Box<Self>, gpu: &mut GpuDevice) -> Result<()> {
        if let Some((table, _)) = self.table {
            gpu.retire(Resource::Buffer(table))?;
        }
        for image in self.images {
            gpu.retire(Resource::Image(image))?;
        }
        Ok(())
    }
}
