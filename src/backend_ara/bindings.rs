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
    /// Published scene retained until every referencing list is submitted.
    Scene(&'a super::acceleration::Scene),
    /// Storage-backed data with the source WGSL layout.
    Buffer(&'a StorageBuffer),
    /// Fixed logical array; elements may share owners or use scattered heap slots.
    Buffers(&'a [&'a StorageBuffer]),
    /// Immutable sampled view, with its native dimension retained.
    Image(&'a Texture),
    /// Fixed logical array of sampled views.
    Images(&'a [&'a Texture]),
    /// Fixed logical array of immutable cached sampler descriptions.
    Samplers(&'a [SamplerDesc]),
    /// Cached regular or comparison sampler description.
    Sampler(SamplerDesc),
}
/// Persistent indices plus borrowed resource owners for one selected shader program.
pub struct HeapBindings<'a> {
    record: StorageBuffer,
    values: Vec<Input<'a>>,
    layout: Vec<ShaderInput>,
    pass: Pass,
    contract: [u8; 32],
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
        ensure!(
            !pass.program_only(),
            "shading requires caller material WGSL via Entry::program"
        );
        Self::with_layout(gpu, pass, shader::inputs(pass)?, values)
    }
    /// Build a packet for caller-authored WGSL. The reflected contract is retained
    /// by value, so source code can be dropped after pipeline/packet construction.
    pub fn for_program(
        gpu: &mut GpuDevice,
        program: &shader::Program,
        values: &[(InputKey, Input<'a>)],
    ) -> Result<Self> {
        Self::with_layout(gpu, program.pass(), program.inputs().to_vec(), values)
    }
    fn with_layout(
        gpu: &mut GpuDevice,
        pass: Pass,
        layout: Vec<ShaderInput>,
        values: &[(InputKey, Input<'a>)],
    ) -> Result<Self> {
        let contract = shader::packet_contract(pass, &layout);
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
                (InputKind::AccelerationStructure, Input::Scene(scene)) => scene.slot,
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
                    validate_image(t, dimension, sample, &required.name)?;
                    t.sampled
                }
                (
                    InputKind::ImageArray {
                        count,
                        dimension,
                        sample,
                    },
                    Input::Images(images),
                ) => {
                    ensure!(
                        images.len() == count as usize,
                        "image array {} has the wrong length",
                        required.name
                    );
                    for image in images {
                        validate_image(image, dimension, sample, &required.name)?;
                    }
                    let start = u32::try_from(words.len())
                        .context("input packet exceeds 32-bit indexing")?;
                    words.extend(images.iter().map(|image| image.sampled));
                    start
                }
                (InputKind::SamplerArray { count, comparison }, Input::Samplers(samplers)) => {
                    ensure!(
                        samplers.len() == count as usize,
                        "sampler array {} has the wrong length",
                        required.name
                    );
                    ensure!(
                        samplers
                            .iter()
                            .all(|desc| desc.compare.is_some() == comparison),
                        "sampler array {} uses the wrong heap",
                        required.name
                    );
                    let start = u32::try_from(words.len())
                        .context("input packet exceeds 32-bit indexing")?;
                    for desc in samplers {
                        words.push(gpu.sampler(desc)?.index);
                    }
                    start
                }
                (
                    InputKind::StorageImage {
                        dimension, format, ..
                    },
                    Input::Image(t),
                ) => {
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
            contract,
        })
    }
    pub(crate) fn contract(&self) -> [u8; 32] {
        self.contract
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
    /// Retained scalar image at its original source key.
    pub fn image(&self, key: InputKey) -> Option<&Texture> {
        self.layout
            .iter()
            .zip(&self.values)
            .find_map(|(spec, value)| {
                if spec.key == key
                    && let Input::Image(image) = value
                {
                    Some(*image)
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
                Input::Scene(scene) => uses.push(ResourceUse::Whole(
                    Resource::AccelerationStructure(scene.acceleration),
                    Access::AsRead,
                )),
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
                Input::Images(images) => {
                    uses.extend(images.iter().map(|t| t.usage(Access::ShaderRead)))
                }
                Input::Sampler(_) | Input::Samplers(_) => {}
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
    /// Give up the resource borrows once every pass that uses the packet is recorded,
    /// keeping only its own record. Retire that record after the list is submitted; the
    /// resources it named must still outlive that submission.
    pub fn release(self) -> StorageBuffer {
        self.record
    }
    /// Retire the packet and release its resource borrows after its last submission.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Buffer(self.record.buffer));
        self.record.retire(gpu)
    }
}

fn validate_image(
    t: &Texture,
    dimension: ara_gpu::texture::ViewDimension,
    sample: SampleType,
    name: &str,
) -> Result<()> {
    let samples = if sample == SampleType::UintMultisampled {
        ara_gpu::texture::SampleCount::Four
    } else {
        ara_gpu::texture::SampleCount::One
    };
    ensure!(
        t.view.dimension == dimension && t.shape.samples == samples,
        "image {name} has the wrong view dimension or sample count"
    );
    let actual = match t.format {
        Format::Depth16Unorm | Format::Depth32Float => SampleType::Depth,
        Format::R32Uint | Format::Rg32Uint | Format::Rgba32Uint
            if t.shape.samples == ara_gpu::texture::SampleCount::Four =>
        {
            SampleType::UintMultisampled
        }
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
    ensure!(actual == sample, "image {name} has the wrong sample type");
    Ok(())
}
