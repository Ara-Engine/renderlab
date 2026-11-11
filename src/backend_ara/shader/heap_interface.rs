//! Reflection shared by shader adaptation and persistent logical-input packets.
use anyhow::{Context, Result, bail, ensure};
use ara_gpu::{descriptors::Format, texture::ViewDimension};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Top-level acceleration structure for ray queries.
    AccelerationStructure,
    /// Storage-backed data, retaining the source layout and minimum byte size.
    Buffer {
        /// Smallest valid source binding in bytes.
        minimum: u32,
        /// Whether shader stores are permitted.
        writable: bool,
    },
    /// Fixed logical array whose elements are arbitrary published buffers.
    BufferArray {
        /// Number of source binding-array elements.
        count: u32,
        /// Smallest valid element binding in bytes.
        minimum: u32,
        /// Whether stores through an element are permitted.
        writable: bool,
    },
    /// Sampled image with an explicit view dimension and sample interpretation.
    Image {
        /// Shader view dimension, including single-layer array views.
        dimension: ViewDimension,
        /// Scalar or depth interpretation.
        sample: SampleType,
    },
    /// Fixed logical array of sampled views, retaining each element owner.
    ImageArray {
        /// Number of source binding-array elements.
        count: u32,
        /// Required native view dimension.
        dimension: ViewDimension,
        /// Required texel interpretation.
        sample: SampleType,
    },
    /// Fixed logical array of cached immutable samplers.
    SamplerArray {
        /// Number of source binding-array elements.
        count: u32,
        /// Selects the comparison heap when true.
        comparison: bool,
    },
    /// Write-only storage image, with an exact portable format.
    StorageImage {
        /// Shader view dimension.
        dimension: ViewDimension,
        /// Declared storage texel format.
        format: Format,
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
    pass.reflected()?.inputs()
}
/// A caller-authored program's inputs, kept by the pass and its source's `digest`.
pub(super) fn from_digest(
    pass: super::Pass,
    source: &str,
    digest: [u8; 32],
) -> Result<Vec<ShaderInput>> {
    super::reflection::reflected(pass, source, digest)?.inputs()
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
        let kind = if let TypeInner::BindingArray { base, size } = module.types[g.ty].inner {
            let naga::ArraySize::Constant(count) = size else {
                bail!("source binding array needs a fixed element count");
            };
            match g.space {
                AddressSpace::Storage { access } => {
                    ensure!(
                        matches!(module.types[base].inner, TypeInner::Struct { .. }),
                        "binding-array elements must be storage structs"
                    );
                    InputKind::BufferArray {
                        count: count.get(),
                        minimum: layout[base].size,
                        writable: access.contains(naga::StorageAccess::STORE),
                    }
                }
                AddressSpace::Handle => match handle_kind(&module.types[base].inner)? {
                    InputKind::Image { dimension, sample } => {
                        image_entry(dimension, sample)?;
                        InputKind::ImageArray {
                            count: count.get(),
                            dimension,
                            sample,
                        }
                    }
                    InputKind::Sampler { comparison } => InputKind::SamplerArray {
                        count: count.get(),
                        comparison,
                    },
                    _ => bail!("only sampled image and sampler handle arrays are supported"),
                },
                _ => bail!("unsupported binding-array address space"),
            }
        } else {
            match g.space {
                AddressSpace::Uniform => InputKind::Buffer {
                    minimum: layout[g.ty].size,
                    writable: false,
                },
                AddressSpace::Storage { access } => InputKind::Buffer {
                    minimum: layout[g.ty].size,
                    writable: access.contains(naga::StorageAccess::STORE),
                },
                AddressSpace::Handle => match module.types[g.ty].inner {
                    TypeInner::AccelerationStructure {
                        vertex_return: false,
                    } => InputKind::AccelerationStructure,
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
                            (ImageDimension::Cube, true) => ViewDimension::CubeArray,
                            _ => bail!("unported image dimension for {:?}", g.name),
                        };
                        if let ImageClass::Storage { format, access } = class {
                            ensure!(
                                access == naga::StorageAccess::STORE,
                                "unported storage-image access"
                            );
                            inputs.push(ShaderInput {
                                key: InputKey {
                                    group: binding.group,
                                    binding: binding.binding,
                                },
                                name: g.name.clone().context("resource missing name")?,
                                kind: InputKind::StorageImage {
                                    dimension,
                                    format: storage_format(format)?,
                                },
                            });
                            continue;
                        }
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
            }
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
fn handle_kind(inner: &TypeInner) -> Result<InputKind> {
    Ok(match *inner {
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
                _ => bail!("unsupported handle-array image dimension"),
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
                _ => bail!("unsupported handle-array image class"),
            };
            InputKind::Image { dimension, sample }
        }
        _ => bail!("unsupported handle-array element"),
    })
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
        (ViewDimension::D2Array, SampleType::Uint) => "AraReadUintArrayEntry",
        (ViewDimension::D2Array, SampleType::Depth) => "AraDepthArrayEntry",
        (ViewDimension::D3, SampleType::Float) => "AraVolumeEntry",
        (ViewDimension::CubeArray, SampleType::Float) => "AraCubeArrayEntry",
        _ => bail!("unported sampled-image interpretation"),
    })
}
pub(super) fn wgsl_type(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Result<String> {
    if let Some(name) = &module.types[ty].name {
        return Ok(name.clone());
    }
    let scalar = |kind: ScalarKind, width: u8| -> Result<&'static str> {
        if width == 8 {
            return Ok(match kind {
                ScalarKind::Uint => "u64",
                ScalarKind::Sint => "i64",
                _ => bail!("unported 64-bit scalar kind"),
            });
        }
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
        TypeInner::Atomic(s) => {
            ensure!(
                matches!(s.kind, ScalarKind::Uint | ScalarKind::Sint),
                "unported atomic scalar"
            );
            format!("atomic<{}>", scalar(s.kind, s.width)?)
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

// Each backend reflects independently after transformations. Check the original
// sorted contract before emitting code, so a frontend change cannot reorder data.
pub(super) fn assert_source_contract(
    pass: super::Pass,
    source: &str,
    actual: &[ShaderInput],
) -> Result<()> {
    let expected = from_digest(pass, source, super::memo::digest(source))?;
    ensure!(
        expected.len() == actual.len()
            && expected
                .iter()
                .zip(actual)
                .all(|(a, b)| a.key == b.key && a.kind == b.kind),
        "selected shader input contract changed during lowering"
    );
    Ok(())
}

pub(super) fn storage_format(format: naga::StorageFormat) -> Result<Format> {
    Ok(match format {
        naga::StorageFormat::R32Float => Format::R32Float,
        naga::StorageFormat::Rgba16Float => Format::Rgba16Float,
        naga::StorageFormat::Rgba32Float => Format::Rgba32Float,
        naga::StorageFormat::R32Uint => Format::R32Uint,
        naga::StorageFormat::Rg32Uint => Format::Rg32Uint,
        naga::StorageFormat::Rgba32Uint => Format::Rgba32Uint,
        _ => bail!("unported storage-image format {format:?}"),
    })
}
pub(super) fn storage_wgsl(format: Format) -> Result<&'static str> {
    Ok(match format {
        Format::R32Float => "r32float",
        Format::Rgba16Float => "rgba16float",
        Format::Rgba32Float => "rgba32float",
        Format::R32Uint => "r32uint",
        Format::Rg32Uint => "rg32uint",
        Format::Rgba32Uint => "rgba32uint",
        Format::Rgba8 => "rgba8unorm",
        _ => bail!("unported storage-image format"),
    })
}
pub(super) fn storage_entry(dimension: ViewDimension, format: Format) -> Result<&'static str> {
    Ok(match (dimension, format) {
        (ViewDimension::D2, Format::R32Uint | Format::Rg32Uint | Format::Rgba32Uint) => {
            "AraWriteUintImageEntry"
        }
        (
            ViewDimension::D2,
            Format::R32Float | Format::Rgba16Float | Format::Rgba32Float | Format::Rgba8,
        ) => "AraWriteImageEntry",
        (ViewDimension::D2Array, Format::R32Float | Format::Rgba16Float | Format::Rgba32Float) => {
            "AraWriteCubeImageEntry"
        }
        (ViewDimension::D2Array, Format::R32Uint | Format::Rg32Uint | Format::Rgba32Uint) => {
            "AraWriteUintArrayEntry"
        }
        (ViewDimension::D3, Format::R32Float | Format::Rgba16Float | Format::Rgba32Float) => {
            "AraWriteVolumeEntry"
        }
        _ => bail!("unported storage-image interpretation"),
    })
}
