//! Lazy Metal views of packet slot lists, retaining naga's element expressions.
use anyhow::{Context, Result, ensure};

const PROXIES: &str = r#"
template<typename Ptr> struct AraBufferRef { Ptr inner; };
template<typename Ptr> struct AraBufferArray {
    device const BufferEntry* heap;
    device const uint* slots;
    AraBufferRef<Ptr> operator[](uint i) const {
        return {reinterpret_cast<Ptr>(heap[slots[i]].data)};
    }
};
struct AraBufferSizes {
    device const BufferEntry* heap;
    device const uint* slots;
    uint operator[](uint i) const { return uint(heap[slots[i]].metadata); }
};
"#;

/// Replace only naga's buffer-array parameter type and per-element byte sizes.
/// Each index resolves a heap entry lazily; no native array of 64 pointers is built.
pub(super) fn adapt(source: &mut String, module: &naga::Module) -> Result<()> {
    let arrays: Vec<_> = module
        .global_variables
        .iter()
        .filter_map(|(handle, global)| {
            if matches!(global.space, naga::AddressSpace::Storage { .. })
                && let naga::TypeInner::BindingArray {
                    size: naga::ArraySize::Constant(n),
                    ..
                } = module.types[global.ty].inner
            {
                Some((handle, global, n.get()))
            } else {
                None
            }
        })
        .collect();
    if arrays.is_empty() {
        return Ok(());
    }
    let needle = "device NagaArgumentBufferWrapper<";
    let mut cursor = 0;
    let mut out = String::new();
    while let Some(relative) = source[cursor..].find(needle) {
        let start = cursor + relative;
        let inner = start + needle.len();
        let mut depth = 1u32;
        let mut end = None;
        for (i, c) in source[inner..].char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(inner + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.context("unterminated naga binding-array wrapper")?;
        let ptr = &source[inner..end];
        ensure!(
            ptr.starts_with("device ") && ptr.ends_with('*'),
            "unsupported binding-array element type {ptr}"
        );
        ensure!(
            source[end + 1..].starts_with('*'),
            "naga binding-array wrapper must be a pointer"
        );
        out.push_str(&source[cursor..start]);
        out.push_str(&format!("AraBufferArray<{ptr}>"));
        cursor = end + 2;
    }
    ensure!(cursor > 0, "naga buffer-array parameter type missing");
    out.push_str(&source[cursor..]);
    for (handle, global, count) in arrays {
        if module.types[global.ty]
            .inner
            .needs_host_buffer_byte_size(&module.types)
        {
            let field = format!("uint size{}[{count}];", handle.index());
            ensure!(
                out.matches(&field).count() == 1,
                "naga array-size field missing: {field}"
            );
            out = out.replace(&field, &format!("AraBufferSizes size{};", handle.index()));
        }
    }
    *source = format!("{PROXIES}\n{out}");
    Ok(())
}

pub(super) fn slots(ordinal: usize) -> String {
    format!(
        "top.buffers[top.root[0]].data + {}",
        super::heap_interface::slot(ordinal)
    )
}
pub(super) fn argument(parameter: &str, ordinal: usize) -> Result<String> {
    let (ty, _) = parameter
        .rsplit_once(' ')
        .context("buffer-array argument missing name")?;
    let ty = ty.trim().strip_prefix("const ").unwrap_or(ty.trim());
    ensure!(
        ty.starts_with("AraBufferArray<") && ty.ends_with('>'),
        "unexpected buffer-array argument {parameter}"
    );
    Ok(format!("{ty}{{top.buffers,{}}}", slots(ordinal)))
}
