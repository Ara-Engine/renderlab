//! A view's luminance histogram, binned on the GPU and read back without waiting
//! (`Engine::view_histogram`, `Engine::histograms`): what an automatic-exposure meter reads
//! in place of the whole displayed image.
use super::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    kernel::Kernel,
    shader::{self, Pass},
    transfer,
};
use anyhow::{Result, bail, ensure};
use ara_gpu::{Access, Buffer, Device, GpuDevice, Queue, Resource, TimelinePoint};

/// What a histogram bins: `bins` bins over the log of display luminance from `log_min`
/// (bin 0; luminance is floored at 1e-4) to 0 (white), over every `stride`-th pixel on
/// each axis of the view's displayed image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistogramDesc {
    /// Bin count, 1 to 4096.
    pub bins: u32,
    /// The log of the darkest luminance, below 0.
    pub log_min: f32,
    /// Pixels between samples on each axis, at least 1.
    pub stride: u32,
}

/// A finished histogram of one view's displayed image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistogramReading {
    /// What `Engine::view_histogram` returned for it.
    pub request: u64,
    /// The view it was taken of.
    pub view: crate::ViewId,
    /// The view's frame whose displayed image it binned.
    pub frame: crate::FrameId,
    /// The count in each bin.
    pub bins: Vec<u32>,
}

/// A histogram in flight: its readback and the point its submission signals.
pub(crate) struct Pending {
    pub request: u64,
    pub view: crate::ViewId,
    pub frame: crate::FrameId,
    readback: Buffer,
    point: TimelinePoint,
}

/// Records the histogram of `image` (sRGB-encoded RGBA8, in `CopySource`, where it is left)
/// and submits it without waiting; its counts are readable once `point` completes.
pub(crate) fn submit(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    image: &Texture,
    desc: HistogramDesc,
) -> Result<(Buffer, TimelinePoint)> {
    ensure!(
        (1..=4096).contains(&desc.bins) && desc.stride >= 1 && desc.log_min < 0.,
        "a histogram has 1 to 4096 bins, a stride of at least 1 and a darkest log below 0"
    );
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    graph.import(Resource::Image(image.image), Access::CopySource);
    let bins = StorageBuffer::new(gpu, &vec![0; desc.bins as usize * 4])?;
    let record = StorageBuffer::new(
        gpu,
        &[
            desc.bins.to_le_bytes(),
            desc.stride.to_le_bytes(),
            desc.log_min.to_le_bytes(),
            [0; 4],
        ]
        .concat(),
    )?;
    for buffer in [&bins, &record] {
        graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
    }
    let values = shader::inputs(Pass::ViewHistogram)?
        .iter()
        .map(|spec| {
            Ok((
                spec.key,
                match spec.key.binding {
                    0 => Input::Image(image),
                    1 => Input::Buffer(&bins),
                    2 => Input::Buffer(&record),
                    _ => bail!("the view histogram cannot supply {}", spec.name),
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let bindings = HeapBindings::new(gpu, Pass::ViewHistogram, &values)?;
    bindings.import(&mut graph);
    let samples = image.size.map(|n| n.div_ceil(desc.stride));
    Kernel::new(gpu, cache, Pass::ViewHistogram)?.record(
        &mut graph,
        &mut rec,
        &bindings,
        [samples[0], samples[1], 1],
    )?;
    let readback = transfer::readback_buffer(gpu, &mut graph, &mut rec, bins.buffer, 0, bins.len)?;
    graph.node(
        &mut rec,
        &[(Resource::Image(image.image), Access::CopySource)],
    )?;
    let packet = bindings.release();
    let point = gpu.submit(rec.finish(), &[])?;
    // Reclaimed once the submission completes.
    packet.retire(gpu)?;
    bins.retire(gpu)?;
    record.retire(gpu)?;
    Ok((readback, point))
}

impl Pending {
    pub(crate) fn new(
        request: u64,
        view: crate::ViewId,
        frame: crate::FrameId,
        (readback, point): (Buffer, TimelinePoint),
    ) -> Self {
        Self {
            request,
            view,
            frame,
            readback,
            point,
        }
    }
    /// The reading, once the GPU has finished it; `None` before.
    pub(crate) fn take(&self, gpu: &mut GpuDevice) -> Result<Option<HistogramReading>> {
        if gpu.completed()?[self.point.queue.index()] < self.point.value {
            return Ok(None);
        }
        let bytes = gpu.read(self.readback)?;
        gpu.retire(Resource::Buffer(self.readback))?;
        Ok(Some(HistogramReading {
            request: self.request,
            view: self.view,
            frame: self.frame,
            bins: bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect(),
        }))
    }
    /// Releases its readback unread (the engine is being retired).
    pub(crate) fn drop_unread(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire(Resource::Buffer(self.readback))?)
    }
}
