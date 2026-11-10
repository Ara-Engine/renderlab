//! Logical WGSL inputs become one persistent heap-index packet, without native sets.
use super::{
    buffer::StorageBuffer,
    graph::Graph,
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
        let mut words = vec![layout.len() as u32, 0, 0, 0];
        let mut retained = Vec::new();
        for required in &layout {
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
                (InputKind::Image { dimension, sample }, Input::Image(t)) => {
                    ensure!(
                        t.view_dimension == dimension,
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
            words.push(slot);
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
    /// Retained buffer at an original source key, for encoder range validation.
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
    pub fn uses(&self) -> Vec<(Resource, Access)> {
        let mut uses = vec![(Resource::Buffer(self.record.buffer), Access::ShaderRead)];
        for (value, spec) in self.values.iter().zip(&self.layout) {
            match value {
                Input::Buffer(b) => uses.push((
                    Resource::Buffer(b.buffer),
                    if matches!(spec.kind, InputKind::Buffer { writable: true, .. }) {
                        Access::Storage
                    } else {
                        Access::ShaderRead
                    },
                )),
                Input::Image(t) => uses.push((Resource::Image(t.image), Access::ShaderRead)),
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
