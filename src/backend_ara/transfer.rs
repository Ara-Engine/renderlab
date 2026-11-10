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
