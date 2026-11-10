//! One reflected heap packet lowered to runtime descriptor arrays.
use super::spirv::rewrite_globals;
use anyhow::{Context, Result};
use std::collections::BTreeMap;

pub(super) fn adapt(source: &str, module: &naga::Module) -> Result<String> {
    let inputs = super::heap_interface::from_module(module)?;
    lower(source, module, &inputs, false)
}
pub(super) fn adapt_ray_source(source: &str, inputs: &[super::ShaderInput]) -> Result<String> {
    // This path rewrites the original text, so a local named like a resource global
    // (`let camera = ...`) must get its own name before anything is rewritten.
    let resources: Vec<String> = super::parse(source)?
        .global_variables
        .iter()
        .filter(|(_, g)| g.binding.is_some())
        .filter_map(|(_, g)| g.name.clone())
        .collect();
    let source =
        &super::local_shadows::rename(source, &resources.iter().map(String::as_str).collect())?;
    let original = super::parse(source)?;
    let moved = super::ray_source::relocate_groups(source, &original)?;
    let module = super::parse(&moved)?;
    lower(&moved, &module, inputs, true)
}
fn lower(
    source: &str,
    module: &naga::Module,
    inputs: &[super::ShaderInput],
    preserve_declarations: bool,
) -> Result<String> {
    use super::{InputKind, SampleType};
    use ara_gpu::texture::ViewDimension;
    let mut names = BTreeMap::new();
    let mut declarations = String::new();
    let mut arrays = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let slot = format!("ara_packet[ara_root.w0].values[(ara_root.w1 >> 2u) + {i}u]");
        match input.kind {
            InputKind::AccelerationStructure => {
                declarations.push_str(&format!("@group(0) @binding({}) var ara_acceleration_{i}:binding_array<acceleration_structure>;\n",2200+i));
                names.insert(input.name.as_str(), format!("ara_acceleration_{i}[0u]"));
            }
            InputKind::Buffer { writable, .. } => {
                let (_, global) = module
                    .global_variables
                    .iter()
                    .find(|(_, g)| g.name.as_deref() == Some(input.name.as_str()))
                    .context("packet source buffer missing")?;
                let unsized_struct = matches!(
                    module.types[global.ty].inner,
                    naga::TypeInner::Struct { .. }
                ) && module.types[global.ty]
                    .inner
                    .is_dynamically_sized(&module.types);
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
                let replacement = format!(
                    "ara_buffer_{i}[{slot}]{}",
                    if array {
                        ".values"
                    } else if unsized_struct {
                        ""
                    } else {
                        ".value"
                    }
                );
                names.insert(input.name.as_str(), replacement);
            }
            InputKind::BufferArray { writable, .. } => {
                let (handle, global) = module
                    .global_variables
                    .iter()
                    .find(|(_, g)| g.name.as_deref() == Some(input.name.as_str()))
                    .context("packet source array missing")?;
                let naga::TypeInner::BindingArray { base, .. } = module.types[global.ty].inner
                else {
                    anyhow::bail!("reflected binding array changed type");
                };
                let ty = super::heap_interface::wgsl_type(module, base)?;
                declarations.push_str(&format!(
                    "@group(0) @binding({}) var<storage,{}> ara_array_{i}:binding_array<{ty}>;\n",
                    1900 + i,
                    if writable { "read_write" } else { "read" }
                ));
                arrays.push(super::array_access::ArrayRewrite {
                    global: handle,
                    prefix: format!("ara_array_{i}[ara_packet[ara_root.w0].values[{slot} + u32("),
                    suffix: ")]]".into(),
                });
            }
            InputKind::StorageImage { dimension, format } => {
                let dim = match dimension {
                    ViewDimension::D2 => "2d",
                    ViewDimension::D2Array => "2d_array",
                    ViewDimension::D3 => "3d",
                };
                declarations.push_str(&format!("@group(0) @binding({}) var ara_storage_{i}:binding_array<texture_storage_{dim}<{},write>>;\n",1600+i,super::heap_interface::storage_wgsl(format)?));
                names.insert(input.name.as_str(), format!("ara_storage_{i}[{slot}]"));
            }
            InputKind::Sampler { comparison } | InputKind::SamplerArray { comparison, .. } => {
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
                if matches!(input.kind, InputKind::SamplerArray { .. }) {
                    arrays.push(array_rewrite(module, input, &name, &slot)?);
                } else {
                    names.insert(input.name.as_str(), format!("{name}[{slot}]"));
                }
            }
            InputKind::Image { dimension, sample }
            | InputKind::ImageArray {
                dimension, sample, ..
            } => {
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
                if matches!(input.kind, InputKind::ImageArray { .. }) {
                    arrays.push(array_rewrite(
                        module,
                        input,
                        &format!("ara_image_{i}"),
                        &slot,
                    )?);
                } else {
                    names.insert(input.name.as_str(), format!("ara_image_{i}[{slot}]"));
                }
            }
        }
    }
    let rewritten = super::array_access::rewrite(source, module, &arrays)?;
    let body = if preserve_declarations {
        rewritten
    } else {
        super::spirv::strip_bindings(&rewritten)
    };
    let out = rewrite_globals(&body, &names, module)?;
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

fn array_rewrite(
    module: &naga::Module,
    input: &super::ShaderInput,
    name: &str,
    slot: &str,
) -> Result<super::array_access::ArrayRewrite> {
    let (global, _) = module
        .global_variables
        .iter()
        .find(|(_, g)| g.name.as_deref() == Some(input.name.as_str()))
        .context("packet handle array missing")?;
    Ok(super::array_access::ArrayRewrite {
        global,
        prefix: format!("{name}[ara_packet[ara_root.w0].values[{slot} + u32("),
        suffix: ")]]".into(),
    })
}
