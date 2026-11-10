//! Logical WGSL inputs become one persistent heap-index packet, without native sets.
use super::{
    buffer::StorageBuffer,
    graph::{Graph, ResourceUse},
    image::Texture,
    shader::{self, InputKey, InputKind, Pass, SampleType, ShaderInput},
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Resource, RootConstants, descriptors::Format, sampler::SamplerDesc,
};
/// Borrowed published input, retained by a packet until it is retired.
#[derive(Clone, Copy)]
pub enum Input<'a> {
    /// Storage-backed data with the source WGSL layout.
    Buffer(&'a StorageBuffer),
    /// Fixed logical array; elements may share owners or use scattered heap slots.
    Buffers(&'a [&'a StorageBuffer]),
    /// Immutable sampled view, with its native dimension retained.
    Image(&'a Texture),
    /// Cached regular or comparison sampler description.
    Sampler(SamplerDesc),
}
/// Persistent indices plus borrowed resource owners for one selected shader program.
pub struct HeapBindings<'a> {
    record: StorageBuffer,
    values: Vec<Input<'a>>,
    layout: Vec<ShaderInput>,
    pass: Pass,
}
impl<'a> HeapBindings<'a> {
    /// Array heads contain buffer-absolute tail word offsets. Packing records
    /// into an arena must relocate those offsets as well as the root byte offset.
    /// Validate the complete selected interface once; allocate only an ordinary
    /// storage record. Original group/binding numbers are logical source keys.
    pub fn new(gpu: &mut GpuDevice, pass: Pass, values: &[(InputKey, Input<'a>)]) -> Result<Self> {
        ensure!(
            pass.indirect(),
            "shader does not use an indirect input packet"
        );
        let layout = shader::inputs(pass)?;
        ensure!(
            values.len() == layout.len(),
            "wrong number of selected shader inputs"
        );
        let mut words = vec![0; 4 + layout.len()];
        words[0] = layout.len() as u32;
        let mut retained = Vec::new();
        for (ordinal, required) in layout.iter().enumerate() {
            let mut matches = values.iter().filter(|(key, _)| *key == required.key);
            let value = matches
                .next()
                .with_context(|| {
                    format!(
                        "missing shader input {} at {:?}",
                        required.name, required.key
                    )
                })?
                .1;
            ensure!(
                matches.next().is_none(),
                "duplicate shader input {:?}",
                required.key
            );
            let slot = match (required.kind, value) {
                (InputKind::Buffer { minimum, .. }, Input::Buffer(b)) => {
                    ensure!(
                        b.len >= u64::from(minimum),
                        "buffer {} is too short",
                        required.name
                    );
                    b.slot
                }
                (InputKind::BufferArray { count, minimum, .. }, Input::Buffers(buffers)) => {
                    ensure!(
                        buffers.len() == count as usize,
                        "buffer array {} has the wrong length",
                        required.name
                    );
                    ensure!(
                        buffers.iter().all(|b| b.len >= u64::from(minimum)),
                        "buffer array {} has a short element",
                        required.name
                    );
                    let start = u32::try_from(words.len())
                        .context("input packet exceeds 32-bit indexing")?;
                    words.extend(buffers.iter().map(|b| b.slot));
                    start
                }
                (InputKind::Image { dimension, sample }, Input::Image(t)) => {
                    ensure!(
                        t.view.dimension == dimension
                            && t.shape.samples == ara_gpu::texture::SampleCount::One,
                        "image {} has the wrong view dimension",
                        required.name
                    );
                    let actual = match t.format {
                        Format::Depth16Unorm | Format::Depth32Float => SampleType::Depth,
                        Format::R32Uint | Format::Rg32Uint | Format::Rgba32Uint => SampleType::Uint,
                        Format::Rgba8
                        | Format::Rgba8Srgb
                        | Format::Rgba32Float
                        | Format::Rgba16Float
                        | Format::Rgb10A2
                        | Format::R32Float
                        | Format::R16Float
                        | Format::Rg16Float => SampleType::Float,
                    };
                    ensure!(
                        actual == sample,
                        "image {} has the wrong sample type",
                        required.name
                    );
                    t.sampled
                }
                (InputKind::StorageImage { dimension, format }, Input::Image(t)) => {
                    ensure!(
                        t.view.dimension == dimension
                            && t.view.mip_count == 1
                            && t.shape.samples == ara_gpu::texture::SampleCount::One,
                        "storage image {} has the wrong view",
                        required.name
                    );
                    ensure!(
                        t.format == format,
                        "storage image {} has the wrong format",
                        required.name
                    );
                    t.storage.with_context(|| {
                        format!(
                            "storage image {} has no writable publication",
                            required.name
                        )
                    })?
                }
                (InputKind::Sampler { comparison }, Input::Sampler(desc)) => {
                    ensure!(
                        desc.compare.is_some() == comparison,
                        "sampler {} uses the wrong heap",
                        required.name
                    );
                    gpu.sampler(&desc)?.index
                }
                _ => anyhow::bail!("input {} has the wrong resource kind", required.name),
            };
            words[4 + ordinal] = slot;
            retained.push(value);
        }
        let record = StorageBuffer::new(gpu, bytemuck::cast_slice(&words))?;
        Ok(Self {
            record,
            values: retained,
            layout,
            pass,
        })
    }
    /// Selected shader family, to reject an incompatible pipeline packet.
    pub fn pass(&self) -> Pass {
        self.pass
    }
    /// Import the packet's initialization once; input producer states stay intact.
    pub fn import(&self, graph: &mut Graph) {
        graph.import(Resource::Buffer(self.record.buffer), Access::HostWrite);
    }
    /// Retained scalar buffer at a source key, for encoder range validation.
    /// Array keys return None; their elements remain retained in the packet.
    pub fn buffer(&self, key: InputKey) -> Option<&StorageBuffer> {
        self.layout
            .iter()
            .zip(&self.values)
            .find_map(|(spec, value)| {
                if spec.key == key
                    && let Input::Buffer(buffer) = value
                {
                    Some(*buffer)
                } else {
                    None
                }
            })
    }
    /// Every reachable input and packet use, for union with a whole raster batch.
    pub fn uses(&self) -> Vec<ResourceUse> {
        let mut uses = vec![ResourceUse::Whole(
            Resource::Buffer(self.record.buffer),
            Access::ShaderRead,
        )];
        for (value, spec) in self.values.iter().zip(&self.layout) {
            match value {
                Input::Buffer(b) => uses.push(ResourceUse::Whole(
                    Resource::Buffer(b.buffer),
                    if matches!(spec.kind, InputKind::Buffer { writable: true, .. }) {
                        Access::Storage
                    } else {
                        Access::ShaderRead
                    },
                )),
                Input::Buffers(buffers) => {
                    let access =
                        if matches!(spec.kind, InputKind::BufferArray { writable: true, .. }) {
                            Access::Storage
                        } else {
                            Access::ShaderRead
                        };
                    uses.extend(
                        buffers
                            .iter()
                            .map(|b| ResourceUse::Whole(Resource::Buffer(b.buffer), access)),
                    );
                }
                Input::Image(t) => uses.push(t.usage(
                    if matches!(spec.kind, InputKind::StorageImage { .. }) {
                        Access::Storage
                    } else {
                        Access::ShaderRead
                    },
                )),
                Input::Sampler(_) => {}
            }
        }
        uses
    }
    /// The packet starts after four reserved words, exercising the byte-offset ABI.
    pub fn roots(&self) -> RootConstants {
        RootConstants {
            words: [self.record.slot, 16, 0, 0, 0, 0, 0, 0],
        }
    }
    /// Retire the packet and release its resource borrows after its last submission.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Buffer(self.record.buffer));
        self.record.retire(gpu)
    }
}
