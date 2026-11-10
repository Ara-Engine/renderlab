//! One reflected heap packet lowered to runtime descriptor arrays.
use super::spirv::rewrite_global_uses;
use anyhow::{Context, Result};
use std::collections::BTreeMap;

pub(super) fn adapt(body: &str, module: &naga::Module) -> Result<String> {
    use super::{InputKind, SampleType};
    use ara_gpu::texture::ViewDimension;
    let inputs = super::heap_interface::from_module(module)?;
    let mut names = BTreeMap::new();
    let mut declarations = String::new();
    let mut values = BTreeMap::new();
    for (i, input) in inputs.iter().enumerate() {
        let slot = format!("ara_packet[ara_root.w0].values[(ara_root.w1 >> 2u) + {i}u]");
        match input.kind {
            InputKind::Buffer { writable, .. } => {
                let (_, global) = module
                    .global_variables
                    .iter()
                    .find(|(_, g)| {
                        g.binding.is_some_and(|b| {
                            b.group == input.key.group && b.binding == input.key.binding
                        })
                    })
                    .context("packet source buffer missing")?;
                let unsized_struct = matches!(
                    module.types[global.ty].inner,
                    naga::TypeInner::Struct { .. }
                ) && module.types[global.ty]
                    .inner
                    .is_dynamically_sized(&module.types);
                let mut load = None;
                let (ty, array) = if let naga::TypeInner::Array { base, size, .. } =
                    module.types[global.ty].inner
                {
                    let element = super::heap_interface::wgsl_type(module, base)?;
                    let count = match size {
                        naga::ArraySize::Dynamic => String::new(),
                        naga::ArraySize::Constant(n) => format!(",{}", n.get()),
                        _ => anyhow::bail!("override-sized buffer input not supported"),
                    };
                    let ty = format!("AraBufferType{i}");
                    declarations.push_str(&format!(
                        "struct {ty} {{values:array<{element}{count}>,}};\n"
                    ));
                    (ty, true)
                } else if unsized_struct {
                    (super::heap_interface::wgsl_type(module, global.ty)?, false)
                } else if let naga::TypeInner::Struct { ref members, span } =
                    module.types[global.ty].inner
                    && !writable
                {
                    // Buffer layout and ordinary copied values need distinct SPIR-V
                    // struct types. Reconstruct the source value field by field;
                    // @size retains every original offset and trailing byte.
                    let ty = format!("AraBufferType{i}");
                    let payload = super::heap_interface::wgsl_type(module, global.ty)?;
                    let mut fields = String::new();
                    let mut values = Vec::new();
                    for (j, m) in members.iter().enumerate() {
                        let name = m.name.as_deref().context("buffer member missing name")?;
                        let next = members.get(j + 1).map_or(span, |n| n.offset);
                        let size = next
                            .checked_sub(m.offset)
                            .context("invalid buffer member offset")?;
                        let member_ty = super::heap_interface::wgsl_type(module, m.ty)?;
                        fields.push_str(&format!("@size({size}) {name}:{member_ty},"));
                        values.push(format!("ara_buffer_{i}[index].{name}"));
                    }
                    declarations.push_str(&format!("struct {ty} {{{fields}}};\n"));
                    load = Some(format!(
                        "fn ara_load_{i}(index:u32)->{payload} {{return {payload}({});}}\n",
                        values.join(",")
                    ));
                    (ty, false)
                } else {
                    let payload = super::heap_interface::wgsl_type(module, global.ty)?;
                    let ty = format!("AraBufferType{i}");
                    declarations.push_str(&format!("struct {ty} {{value:{payload},}};\n"));
                    (ty, false)
                };
                declarations.push_str(&format!(
                    "@group(0) @binding({}) var<storage,{}> ara_buffer_{i}:binding_array<{ty}>;\n",
                    400 + i,
                    if writable { "read_write" } else { "read" }
                ));
                let replacement = if let Some(load) = load {
                    declarations.push_str(&load);
                    values.insert(input.name.as_str(), format!("ara_load_{i}({slot})"));
                    format!("ara_buffer_{i}[{slot}]")
                } else {
                    format!(
                        "ara_buffer_{i}[{slot}]{}",
                        if array {
                            ".values"
                        } else if unsized_struct {
                            ""
                        } else {
                            ".value"
                        }
                    )
                };
                names.insert(input.name.as_str(), replacement);
            }
            InputKind::Sampler { comparison } => {
                let name = if comparison {
                    format!("ara_comparison_{i}")
                } else {
                    format!("ara_sampler_{i}")
                };
                declarations.push_str(&format!(
                    "@group(0) @binding({}) var {name}:binding_array<{}>;\n",
                    if comparison { 1300 + i } else { 1000 + i },
                    if comparison {
                        "sampler_comparison"
                    } else {
                        "sampler"
                    }
                ));
                names.insert(input.name.as_str(), format!("{name}[{slot}]"));
            }
            InputKind::Image { dimension, sample } => {
                let dim = match dimension {
                    ViewDimension::D2 => "2d",
                    ViewDimension::D2Array => "2d_array",
                    ViewDimension::D3 => "3d",
                };
                let ty = if sample == SampleType::Depth {
                    format!("texture_depth_{dim}")
                } else {
                    format!(
                        "texture_{dim}<{}>",
                        if sample == SampleType::Uint {
                            "u32"
                        } else {
                            "f32"
                        }
                    )
                };
                declarations.push_str(&format!(
                    "@group(0) @binding({}) var ara_image_{i}:binding_array<{ty}>;\n",
                    700 + i
                ));
                names.insert(input.name.as_str(), format!("ara_image_{i}[{slot}]"));
            }
        }
    }
    let out = rewrite_global_uses(body, &names, &values, module)?;
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
struct AraPacketWords {{values:array<u32>,}};
@group(0) @binding(1) var<storage,read> ara_packet:binding_array<AraPacketWords>;
{declarations}
{out}
"#
    ))
}
