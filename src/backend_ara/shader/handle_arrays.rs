//! Lazy Metal views of packet slot lists for fixed sampled-texture and sampler
//! binding arrays, retaining naga's `[i].inner` element expressions.
use super::heap_interface::{SampleType, image_entry};
use anyhow::{Context, Result, bail, ensure};
use ara_gpu::texture::ViewDimension;
use naga::{AddressSpace, ImageClass, ImageDimension, ScalarKind, TypeInner};

// Templated on the entry type, so this header may precede the auxiliary entry
// structs; `SamplerEntry` comes from the shared ABI prepended by the cache.
const PROXIES: &str = r#"
template<typename Value> struct AraHandleRef { Value inner; };
template<typename Value, typename Entry> struct AraImageArray {
    device const Entry* heap;
    device const uint* slots;
    AraHandleRef<Value> operator[](uint i) const { return {heap[slots[i]].texture}; }
};
template<typename Value> struct AraSamplerArray {
    device const SamplerEntry* heap;
    device const uint* slots;
    AraHandleRef<Value> operator[](uint i) const { return {heap[slots[i]].state}; }
};
"#;

/// Naga's handle binding-array parameter prefix. Storage buffer arrays use
/// `device NagaArgumentBufferWrapper<` and are left to `buffer_arrays`.
const NEEDLE: &str = "constant NagaArgumentBufferWrapper<";

/// One supported element form: naga's exact native spelling and its heap entry
/// (`None` for samplers).
struct Form {
    value: String,
    entry: Option<&'static str>,
}

impl Form {
    fn proxy(&self) -> String {
        match self.entry {
            Some(entry) => format!("AraImageArray<{}, {entry}>", self.value),
            None => format!("AraSamplerArray<{}>", self.value),
        }
    }
}

/// Replace naga's handle-array parameter types (entry and helper functions) with
/// lazy heap views. Each index resolves one heap entry; no native array of handles
/// is built and shader bodies are untouched. Call before `buffer_arrays::adapt`.
pub(super) fn adapt(source: &mut String, module: &naga::Module) -> Result<()> {
    let mut forms: Vec<(Form, bool)> = Vec::new();
    for (_, global) in module.global_variables.iter() {
        if let Some(form) = form(module, global)?
            && !forms.iter().any(|(f, _)| f.value == form.value)
        {
            forms.push((form, false));
        }
    }
    if forms.is_empty() {
        return Ok(());
    }
    let mut out = String::with_capacity(source.len() + PROXIES.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find(NEEDLE) {
        let start = cursor + relative;
        let open = start + NEEDLE.len();
        let close = closing_angle(source, open)?;
        ensure!(
            source[close + 1..].starts_with('*'),
            "naga handle-array wrapper must be a pointer"
        );
        let value = &source[open..close];
        let (form, seen) = forms
            .iter_mut()
            .find(|(f, _)| f.value == value)
            .with_context(|| format!("unexpected naga handle-array element {value}"))?;
        *seen = true;
        out.push_str(&source[cursor..start]);
        out.push_str(&form.proxy());
        cursor = close + 2;
    }
    out.push_str(&source[cursor..]);
    // Compaction removed unused globals, so every reflected form is a parameter.
    if let Some((form, _)) = forms.iter().find(|(_, seen)| !seen) {
        bail!("naga handle-array parameter missing for {}", form.value);
    }
    *source = format!("{PROXIES}\n{out}");
    Ok(())
}

/// Entry-call expression for a proxy parameter. `table` is `textures` with the
/// native image entry, or `samplers`/`comparison_samplers` with `None`.
pub(super) fn argument(
    parameter: &str,
    ordinal: usize,
    table: &str,
    entry: Option<&str>,
) -> Result<String> {
    let (ty, _) = parameter
        .rsplit_once(' ')
        .context("handle-array argument missing name")?;
    let ty = ty.trim();
    let slots = super::buffer_arrays::slots(ordinal);
    match (table, entry) {
        ("textures", Some(entry)) => {
            let args = template_args(ty, "AraImageArray")?;
            ensure!(
                args.len() == 2 && args[1] == entry,
                "image-array argument {parameter} does not use heap entry {entry}"
            );
            Ok(format!(
                "{ty}{{(device const {entry}*)top.textures,{slots}}}"
            ))
        }
        ("samplers" | "comparison_samplers", None) => {
            let args = template_args(ty, "AraSamplerArray")?;
            ensure!(
                args == ["metal::sampler"],
                "unexpected sampler-array argument {parameter}"
            );
            Ok(format!("{ty}{{top.{table},{slots}}}"))
        }
        _ => bail!("unsupported handle-array table {table} with entry {entry:?}"),
    }
}

/// Classify a fixed handle binding array; other globals yield `None`.
fn form(module: &naga::Module, global: &naga::GlobalVariable) -> Result<Option<Form>> {
    let TypeInner::BindingArray { base, size } = module.types[global.ty].inner else {
        return Ok(None);
    };
    if global.space != AddressSpace::Handle {
        return Ok(None);
    }
    let name = global.name.as_deref().unwrap_or("<unnamed>");
    ensure!(
        matches!(size, naga::ArraySize::Constant(_)),
        "handle binding array {name} needs a fixed element count"
    );
    match module.types[base].inner {
        TypeInner::Sampler { .. } => Ok(Some(Form {
            value: "metal::sampler".into(),
            entry: None,
        })),
        TypeInner::Image {
            dim,
            arrayed,
            class,
        } => {
            let (dimension, dim_str) = match (dim, arrayed) {
                (ImageDimension::D2, false) => (ViewDimension::D2, "2d"),
                (ImageDimension::D2, true) => (ViewDimension::D2Array, "2d_array"),
                (ImageDimension::D3, false) => (ViewDimension::D3, "3d"),
                _ => bail!("unsupported image dimension {dim:?} in handle binding array {name}"),
            };
            let (sample, texture, scalar) = match class {
                ImageClass::Sampled {
                    kind: ScalarKind::Float,
                    multi: false,
                } => (SampleType::Float, "texture", "float"),
                ImageClass::Sampled {
                    kind: ScalarKind::Uint,
                    multi: false,
                } => (SampleType::Uint, "texture", "uint"),
                ImageClass::Depth { multi: false } => (SampleType::Depth, "depth", "float"),
                ImageClass::Storage { .. } => {
                    bail!("storage-image binding arrays are unsupported ({name})")
                }
                _ => bail!("unsupported image class {class:?} in handle binding array {name}"),
            };
            let entry = image_entry(dimension, sample)
                .with_context(|| format!("handle binding array {name}"))?;
            Ok(Some(Form {
                value: format!("metal::{texture}{dim_str}<{scalar}, metal::access::sample>"),
                entry: Some(entry),
            }))
        }
        ref other => bail!("unsupported handle binding-array element {other:?} in {name}"),
    }
}

/// Index of the `>` closing a template whose `<` ends just before `from`.
fn closing_angle(text: &str, from: usize) -> Result<usize> {
    let mut depth = 1u32;
    for (i, c) in text[from..].char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(from + i);
                }
            }
            ';' | '{' | '}' | '(' | ')' => bail!("unbalanced handle-array template"),
            _ => {}
        }
    }
    bail!("unterminated handle-array template")
}

/// Top-level template arguments of exactly `name<...>`.
fn template_args<'a>(ty: &'a str, name: &str) -> Result<Vec<&'a str>> {
    let body = ty
        .strip_prefix(name)
        .and_then(|t| t.strip_prefix('<'))
        .with_context(|| format!("expected {name}<...>, found {ty}"))?;
    let open = ty.len() - body.len();
    let close = closing_angle(ty, open)?;
    ensure!(
        close + 1 == ty.len(),
        "trailing text after {name} arguments: {ty}"
    );
    let mut args = Vec::new();
    let mut depth = 0u32;
    let mut start = open;
    for (i, c) in ty[open..close].char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                args.push(ty[start..open + i].trim());
                start = open + i + 1;
            }
            _ => {}
        }
    }
    args.push(ty[start..close].trim());
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use naga::back::msl;

    const ARRAYS: &str = r#"
enable wgpu_binding_array;
@group(0) @binding(0) var colours: binding_array<texture_2d<f32>, 4>;
@group(0) @binding(1) var depths: binding_array<texture_depth_2d, 4>;
@group(0) @binding(2) var filters: binding_array<sampler, 4>;
@group(0) @binding(3) var shadows: binding_array<sampler_comparison, 4>;
@group(0) @binding(4) var<storage, read_write> results: array<vec4<f32>, 4>;
fn fetch(i: u32) -> vec4<f32> {
    return textureSampleLevel(colours[i], filters[i], vec2(0.5), 0.0);
}
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let d = textureSampleCompareLevel(depths[id.x], shadows[id.x], vec2(0.5), 0.5);
    results[id.x] = fetch(id.x) + vec4(d);
}
"#;

    /// Emit with the production bind targets: binding arrays stay on `buffer`.
    fn emit(source: &str) -> (naga::Module, String) {
        let module = super::super::parse(source).expect("test WGSL parses");
        let info = super::super::validate(&module).expect("test WGSL validates");
        let mut resources = msl::BindingMap::default();
        for (slot, (_, global)) in module.global_variables.iter().enumerate() {
            let binding = global.binding.expect("test globals are bound");
            let target = msl::BindTarget {
                buffer: Some(slot as u8),
                ..Default::default()
            };
            resources.insert(binding, target);
        }
        let mut options = msl::Options {
            lang_version: (3, 0),
            bounds_check_policies: super::super::bounds_checks(),
            fake_missing_bindings: false,
            ..Default::default()
        };
        options.per_entry_point_map.insert(
            "main".into(),
            msl::EntryPointResources {
                resources,
                sizes_buffer: Some(30),
                ..Default::default()
            },
        );
        let (text, _) =
            msl::write_string(&module, &info, &options, &msl::PipelineOptions::default())
                .expect("naga emits MSL");
        (module, text)
    }

    #[test]
    fn rewrites_every_handle_wrapper_and_keeps_inner_accesses() {
        let (module, mut source) = emit(ARRAYS);
        adapt(&mut source, &module).expect("handle arrays adapt");
        assert!(!source.contains(NEEDLE), "{source}");
        let colours = "AraImageArray<metal::texture2d<float, metal::access::sample>, TextureEntry>";
        // Entry signature and the `fetch` helper both carry the proxy.
        assert!(source.matches(colours).count() >= 2, "{source}");
        assert!(source.contains(
            "AraImageArray<metal::depth2d<float, metal::access::sample>, AraDepthEntry>"
        ));
        assert!(source.contains("AraSamplerArray<metal::sampler>"));
        assert!(source.contains("].inner.sample("), "{source}");
    }

    #[test]
    fn rejects_storage_image_arrays() {
        let module = super::super::parse(
            r#"
enable wgpu_binding_array;
@group(0) @binding(0) var outs: binding_array<texture_storage_2d<rgba16float, write>, 4>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    textureStore(outs[id.x], vec2(0), vec4(1.0));
}
"#,
        )
        .expect("test WGSL parses");
        let error = adapt(&mut String::new(), &module).expect_err("storage arrays rejected");
        assert!(error.to_string().contains("storage-image"), "{error}");
    }

    #[test]
    fn rejects_unexpected_emitted_shape() {
        let (module, source) = emit(ARRAYS);
        let mut broken = source.replace(
            "constant NagaArgumentBufferWrapper<metal::sampler>*",
            "constant NagaArgumentBufferWrapper<metal::sampler>&",
        );
        assert!(adapt(&mut broken, &module).is_err());
    }

    #[test]
    fn arguments_bind_heap_tables_and_check_entries() {
        let image =
            "AraImageArray<metal::depth2d<float, metal::access::sample>, AraDepthEntry> depths";
        let slots = super::super::buffer_arrays::slots(1);
        assert_eq!(
            argument(image, 1, "textures", Some("AraDepthEntry")).expect("image argument"),
            format!(
                "AraImageArray<metal::depth2d<float, metal::access::sample>, AraDepthEntry>{{(device const AraDepthEntry*)top.textures,{slots}}}"
            )
        );
        assert!(argument(image, 1, "textures", Some("TextureEntry")).is_err());
        let sampler = "AraSamplerArray<metal::sampler> shadows";
        assert_eq!(
            argument(sampler, 1, "comparison_samplers", None).expect("sampler argument"),
            format!("AraSamplerArray<metal::sampler>{{top.comparison_samplers,{slots}}}")
        );
        assert!(argument(sampler, 1, "samplers", Some("TextureEntry")).is_err());
        assert!(argument(image, 1, "samplers", None).is_err());
    }
}
