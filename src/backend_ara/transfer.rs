//! Ordered staging/readback helpers for replay and the standalone example.
use super::graph::Graph;
use anyhow::Result;
use ara_gpu::{
    Access, Buffer, GpuDevice, Image, Resource,
    commands::{Command, Recorder},
    vk,
};
/// Stage complete image bytes; retire the returned buffer after submitting its list.
pub fn upload(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    image: Image,
    bytes: &[u8],
) -> Result<Buffer> {
    let buffer = gpu.create_buffer(bytes.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
    gpu.upload(buffer, bytes)?;
    graph.import(Resource::Buffer(buffer), Access::HostWrite);
    graph.node(
        rec,
        &[
            (Resource::Buffer(buffer), Access::CopySource),
            (Resource::Image(image), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::UploadImage {
        source: buffer,
        destination: image,
    });
    Ok(buffer)
}
/// Stage one mip level of a single-layer 2D image; retire the returned buffer after
/// submitting its list. Upload every level before the image is read.
pub fn upload_mip(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    image: Image,
    mip: u32,
    extent: [u32; 2],
    bytes: &[u8],
) -> Result<Buffer> {
    let buffer = gpu.create_buffer(bytes.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
    gpu.upload(buffer, bytes)?;
    graph.import(Resource::Buffer(buffer), Access::HostWrite);
    graph.node(
        rec,
        &[
            (Resource::Buffer(buffer), Access::CopySource),
            (Resource::Image(image), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::UploadImageRegion {
        source: buffer,
        destination: image,
        buffer_offset: 0,
        region: ara_gpu::texture::ImageRegion {
            mip,
            origin: [0; 3],
            extent: [extent[0], extent[1], 1],
            base_layer: 0,
            layer_count: 1,
        },
    });
    Ok(buffer)
}
/// Copy an entire image to a correctly sized host buffer and declare its host read.
pub fn readback(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    image: Image,
    byte_len: u64,
) -> Result<Buffer> {
    let buffer = gpu.create_readback_buffer(byte_len)?;
    graph.node(
        rec,
        &[
            (Resource::Image(image), Access::CopySource),
            (Resource::Buffer(buffer), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::ReadImage {
        source: image,
        destination: buffer,
    });
    graph.node(rec, &[(Resource::Buffer(buffer), Access::HostRead)])?;
    Ok(buffer)
}

/// Snapshot a buffer range on the GPU before a later submission overwrites it.
/// The returned host buffer is readable after its submission completes.
pub fn readback_buffer(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    source: Buffer,
    offset: u64,
    len: u64,
) -> Result<Buffer> {
    let destination = gpu.create_readback_buffer(len)?;
    graph.node(
        rec,
        &[
            (Resource::Buffer(source), Access::CopySource),
            (Resource::Buffer(destination), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::CopyBufferRange {
        source,
        source_offset: offset,
        destination,
        destination_offset: 0,
        size: len,
    });
    graph.node(rec, &[(Resource::Buffer(destination), Access::HostRead)])?;
    Ok(destination)
}
