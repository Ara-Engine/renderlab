//! Reflection shared by shader adaptation and persistent logical-input packets.
use anyhow::{Context, Result, bail, ensure};
use ara_gpu::texture::ViewDimension;
use naga::{AddressSpace, ImageClass, ImageDimension, ScalarKind, TypeInner};
/// A logical WGSL resource key; never a native descriptor set allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputKey {
    /// Original source group.
    pub group: u32,
    /// Original source binding.
    pub binding: u32,
}
/// Sample interpretation required by a selected shader input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    /// Floating point color.
    Float,
    /// Unsigned integer texels.
    Uint,
    /// Depth sampling.
    Depth,
}
/// Runtime validation of each heap publication's native interpretation.
#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    /// Storage-backed data, retaining the source layout and minimum byte size.
    Buffer {
        /// Smallest valid source binding in bytes.
        minimum: u32,
        /// Whether shader stores are permitted.
        writable: bool,
    },
    /// Sampled image with an explicit view dimension and sample interpretation.
    Image {
        /// Shader view dimension, including single-layer array views.
        dimension: ViewDimension,
        /// Scalar or depth interpretation.
        sample: SampleType,
    },
    /// Immutable regular or comparison sampler.
    Sampler {
        /// Selects the comparison table when true.
        comparison: bool,
    },
}
/// One selected input, ordered by its original group/binding for the packet ABI.
#[derive(Debug, Clone)]
pub struct ShaderInput {
    /// Source key retained across naga re-emission and native naming.
    pub key: InputKey,
    /// Original global name for diagnostics.
    pub name: String,
    /// Native layout expected at this packet word.
    pub kind: InputKind,
}
/// Reflect only the selected entry pair; unused program interfaces are omitted.
pub fn inputs(pass: super::Pass) -> Result<Vec<ShaderInput>> {
    let mut module = super::parse(pass.source().as_ref())?;
    super::select(&mut module, pass)?;
    from_module(&module)
}
pub(super) fn from_module(module: &naga::Module) -> Result<Vec<ShaderInput>> {
    let mut layout = naga::proc::Layouter::default();
    layout.update(naga::proc::GlobalCtx {
        types: &module.types,
        constants: &module.constants,
        overrides: &module.overrides,
        global_expressions: &module.global_expressions,
    })?;
    let mut inputs = Vec::new();
    for (_, g) in module.global_variables.iter() {
        let Some(binding) = g.binding else { continue };
        let kind = match g.space {
            AddressSpace::Uniform => InputKind::Buffer {
                minimum: layout[g.ty].size,
                writable: false,
            },
            AddressSpace::Storage { access } => InputKind::Buffer {
                minimum: layout[g.ty].size,
                writable: access.contains(naga::StorageAccess::STORE),
            },
            AddressSpace::Handle => match module.types[g.ty].inner {
                TypeInner::Sampler { comparison } => InputKind::Sampler { comparison },
                TypeInner::Image {
                    dim,
                    arrayed,
                    class,
                } => {
                    let dimension = match (dim, arrayed) {
                        (ImageDimension::D2, false) => ViewDimension::D2,
                        (ImageDimension::D2, true) => ViewDimension::D2Array,
                        (ImageDimension::D3, false) => ViewDimension::D3,
                        _ => bail!("unported image dimension for {:?}", g.name),
                    };
                    let sample = match class {
                        ImageClass::Sampled {
                            kind: ScalarKind::Float,
                            multi: false,
                        } => SampleType::Float,
                        ImageClass::Sampled {
                            kind: ScalarKind::Uint,
                            multi: false,
                        } => SampleType::Uint,
                        ImageClass::Depth { multi: false } => SampleType::Depth,
                        _ => bail!("unported image class for {:?}", g.name),
                    };
                    InputKind::Image { dimension, sample }
                }
                _ => bail!("unported handle input {:?}", g.name),
            },
            _ => bail!("unported resource address space {:?}", g.space),
        };
        inputs.push(ShaderInput {
            key: InputKey {
                group: binding.group,
                binding: binding.binding,
            },
            name: g.name.clone().context("resource missing name")?,
            kind,
        });
    }
    inputs.sort_by_key(|i| i.key);
    ensure!(
        inputs.len() < 256 && inputs.windows(2).all(|w| w[0].key != w[1].key),
        "invalid selected resource keys"
    );
    Ok(inputs)
}
pub(super) fn slot(ordinal: usize) -> String {
    format!("top.buffers[top.root[0]].data[(top.root[1] >> 2) + {ordinal}]")
}
pub(super) fn ordinal(inputs: &[ShaderInput], binding: naga::ResourceBinding) -> Result<usize> {
    inputs
        .iter()
        .position(|i| {
            i.key
                == InputKey {
                    group: binding.group,
                    binding: binding.binding,
                }
        })
        .context("selected resource absent from packet layout")
}
pub(super) fn image_entry(dimension: ViewDimension, sample: SampleType) -> Result<&'static str> {
    Ok(match (dimension, sample) {
        (ViewDimension::D2, SampleType::Float) => "TextureEntry",
        (ViewDimension::D2, SampleType::Uint) => "AraReadUintImageEntry",
        (ViewDimension::D2, SampleType::Depth) => "AraDepthEntry",
        (ViewDimension::D2Array, SampleType::Float) => "AraReadArrayEntry",
        (ViewDimension::D2Array, SampleType::Depth) => "AraDepthArrayEntry",
        (ViewDimension::D3, SampleType::Float) => "AraVolumeEntry",
        _ => bail!("unported sampled-image interpretation"),
    })
}
pub(super) fn wgsl_type(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Result<String> {
    if let Some(name) = &module.types[ty].name {
        return Ok(name.clone());
    }
    let scalar = |kind: ScalarKind, width: u8| -> Result<&'static str> {
        ensure!(width == 4, "unported scalar width");
        Ok(match kind {
            ScalarKind::Float => "f32",
            ScalarKind::Uint => "u32",
            ScalarKind::Sint => "i32",
            _ => bail!("unported scalar kind"),
        })
    };
    Ok(match module.types[ty].inner {
        TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(n),
            ..
        } => {
            format!("array<{},{}>", wgsl_type(module, base)?, n.get())
        }
        TypeInner::Scalar(s) => scalar(s.kind, s.width)?.into(),
        TypeInner::Vector { size, scalar: s } => {
            format!("vec{}<{}>", size as u32, scalar(s.kind, s.width)?)
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar: s,
        } => format!(
            "mat{}x{}<{}>",
            columns as u32,
            rows as u32,
            scalar(s.kind, s.width)?
        ),
        _ => bail!("unnamed input element type is not supported"),
    })
}
