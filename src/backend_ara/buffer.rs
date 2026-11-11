//! Immutable storage records shared by leaf encoders.
use anyhow::{Result, ensure};
use ara_gpu::{Buffer, Device, GpuDevice, Resource, vk};
/// One published storage allocation; explicit retirement belongs to its owner.
#[derive(Debug)]
pub struct StorageBuffer {
    /// Arena identity, for graph edges and readback.
    pub buffer: Buffer,
    /// Capacity in bytes.
    pub len: u64,
    pub(crate) slot: u32,
}
impl StorageBuffer {
    /// Upload a nonempty, four-byte-aligned record outside frame encoding.
    pub fn new(gpu: &mut GpuDevice, bytes: &[u8]) -> Result<Self> {
        Self::with_usage(gpu, bytes, vk::BufferUsageFlags::empty())
    }
    /// Add vertex/AS roles to a storage record; AS roles require an RT-enabled device.
    pub fn with_usage(
        gpu: &mut GpuDevice,
        bytes: &[u8],
        extra: vk::BufferUsageFlags,
    ) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len().is_multiple_of(4),
            "storage records need aligned bytes"
        );
        let buffer = gpu.create_buffer(
            bytes.len() as u64,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST
                | extra,
            true,
        )?;
        let result = (|| -> Result<u32> {
            gpu.upload(buffer, bytes)?;
            Ok(gpu.storage_buffer(buffer)?.index)
        })();
        match result {
            Ok(slot) => Ok(Self {
                buffer,
                len: bytes.len() as u64,
                slot,
            }),
            Err(error) => {
                gpu.retire(Resource::Buffer(buffer))?;
                Err(error)
            }
        }
    }
    /// Device-local storage of `len` bytes with undefined contents: fill or update it in
    /// a command list before anything reads it. For buffers the GPU keeps across frames.
    pub fn device(gpu: &mut GpuDevice, len: u64) -> Result<Self> {
        Self::device_with_usage(gpu, len, vk::BufferUsageFlags::empty())
    }
    /// [`device`](Self::device) with further roles, such as indirect arguments.
    pub fn device_with_usage(
        gpu: &mut GpuDevice,
        len: u64,
        extra: vk::BufferUsageFlags,
    ) -> Result<Self> {
        ensure!(
            len > 0 && len.is_multiple_of(4),
            "storage records need aligned bytes"
        );
        let buffer = gpu.create_buffer(
            len,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST
                | extra,
            false,
        )?;
        match gpu.storage_buffer(buffer) {
            Ok(slot) => Ok(Self {
                buffer,
                len,
                slot: slot.index,
            }),
            Err(error) => {
                gpu.retire(Resource::Buffer(buffer))?;
                Err(error.into())
            }
        }
    }
    /// Retire after all lists referencing this record have been submitted.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire(Resource::Buffer(self.buffer))?)
    }
}
