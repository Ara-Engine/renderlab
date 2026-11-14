//! Measurement only (U-RTVIS, 2026-11-14): `RENDERLAB_F16_NEAREST=fog|all` rounds chosen
//! binary16 storage stores to the nearest binary16 in the shader, so the device's store
//! conversion (truncating on NVIDIA and Metal) is exact. `fog`: the volumetric fog's
//! visibility history (`at_scatter_out`, a 0.9 exponential blend). `all`: that, plus the
//! one-shot image chain: the shaded sample (`primary_hdr`), TR1's output (`tr1_output`) and
//! the post passes' outputs (`output_colour`). Unset, every module's text is unchanged.
//! Applied to each WGSL module as it is created (pipeline_compilation.rs `shader_module`).
use std::borrow::Cow;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stores {
    Fog,
    All,
}

/// The switch, read once; an unknown value warns and leaves every store as it is.
pub(crate) fn mode() -> Option<Stores> {
    static MODE: std::sync::OnceLock<Option<Stores>> = std::sync::OnceLock::new();
    *MODE.get_or_init(
        || match std::env::var("RENDERLAB_F16_NEAREST").ok()?.as_str() {
            "fog" => Some(Stores::Fog),
            "all" => Some(Stores::All),
            other => {
                eprintln!("RENDERLAB_F16_NEAREST={other:?} is not fog or all; stores unchanged");
                None
            }
        },
    )
}

const FOG: &[&str] = &["at_scatter_out"];
const IMAGE: &[&str] = &["primary_hdr", "tr1_output", "output_colour"];
/// The nearest binary16 of each component (ties to even), as f32: exact over binary16's
/// normal range (reflections.wgsl `rl_f16_nearest`, under a name of its own).
const HELPER: &str = "\nfn f16n_nearest(v:vec4<f32>)->vec4<f32> {\n    let bits=bitcast<vec4<u32>>(v);\n    let even=(bits>>vec4<u32>(13u))&vec4<u32>(1u);\n    return bitcast<vec4<f32>>((bits+vec4<u32>(0xfffu)+even)&vec4<u32>(0xffffe000u));\n}\n";

/// `source` with each `textureStore` to the chosen textures storing `f16n_nearest(value)`,
/// and the helper appended when any changed; borrowed and unchanged for `None`.
pub(crate) fn rewrite(source: &str, stores: Option<Stores>) -> Cow<'_, str> {
    let Some(stores) = stores else {
        return Cow::Borrowed(source);
    };
    let names: Vec<&str> = match stores {
        Stores::Fog => FOG.to_vec(),
        Stores::All => FOG.iter().chain(IMAGE).copied().collect(),
    };
    let calls: Vec<String> = names.iter().map(|n| format!("textureStore({n},")).collect();
    let mut out = String::with_capacity(source.len() + HELPER.len());
    let mut rest = source;
    let mut changed = false;
    while let Some((at, call)) = calls
        .iter()
        .filter_map(|c| rest.find(c.as_str()).map(|at| (at, c)))
        .min_by_key(|(at, _)| *at)
    {
        let args = at + call.len();
        out.push_str(&rest[..args]);
        // The coordinate ends at the first top-level comma; the value at the call's `)`.
        let (Some(coord), Some(value)) = (
            top_level(&rest[args..], b','),
            top_level(&rest[args..], b')'),
        ) else {
            break;
        };
        let value_start = args + coord + 1;
        let value_end = args + value;
        out.push_str(&rest[args..value_start]);
        out.push_str("f16n_nearest(");
        out.push_str(&rest[value_start..value_end]);
        out.push(')');
        rest = &rest[value_end..];
        changed = true;
    }
    if !changed {
        return Cow::Borrowed(source);
    }
    out.push_str(rest);
    out.push_str(HELPER);
    Cow::Owned(out)
}

/// The offset of the first `byte` at nesting depth 0 of `text` (an argument list's inside).
fn top_level(text: &str, byte: u8) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in text.bytes().enumerate() {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' if depth > 0 => depth -= 1,
            _ if b == byte && depth == 0 => return Some(i),
            b')' => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_leaves_text_and_set_wraps_only_the_chosen_stores() {
        let source = "fn a(){textureStore(primary_hdr,pixel,vec4<f32>(f(g(x),y),1.0));\ntextureStore(at_scatter_out,vec3<i32>(id),value);\ntextureStore(other,p,v);}";
        assert!(matches!(rewrite(source, None), Cow::Borrowed(s) if s == source));
        let fog = rewrite(source, Some(Stores::Fog));
        assert!(fog.contains("textureStore(at_scatter_out,vec3<i32>(id),f16n_nearest(value));"));
        assert!(fog.contains("textureStore(primary_hdr,pixel,vec4<f32>(f(g(x),y),1.0));"));
        let all = rewrite(source, Some(Stores::All));
        assert!(
            all.contains("textureStore(primary_hdr,pixel,f16n_nearest(vec4<f32>(f(g(x),y),1.0)));")
        );
        assert!(all.contains("textureStore(other,p,v);"));
        assert_eq!(all.matches("fn f16n_nearest").count(), 1);
        // A module with none of the stores is unchanged even when set.
        assert!(matches!(
            rewrite("fn b(){}", Some(Stores::All)),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn rewritten_post_module_validates() {
        let post = include_str!("../shaders/post.wgsl");
        let all = rewrite(post, Some(Stores::All));
        assert_eq!(
            all.matches("f16n_nearest(").count(),
            post.matches("textureStore(output_colour,").count() + 1
        );
        let module = naga::front::wgsl::parse_str(&all).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
