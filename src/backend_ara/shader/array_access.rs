//! Retargets indexed uses of fixed `binding_array` globals onto caller-built heap
//! lookups by editing the WGSL text at the spans of a module parsed from that text.
//!
//! The helper owns only the text surgery: which heap, packet word or bounds policy
//! an index goes through is entirely the caller's `prefix`/`suffix`.
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::collections::BTreeSet;
use std::ops::Range;

/// One binding-array global to retarget. Every `global[index]` (the first
/// `Access`/`AccessIndex` whose base is this global) becomes
/// `prefix + index + suffix`. The index text is inserted verbatim, without its
/// brackets and without added parentheses, so a prefix that combines it
/// arithmetically must parenthesise it, e.g. `heap[base + (` … `)]`.
pub(super) struct ArrayRewrite {
    pub global: naga::Handle<naga::GlobalVariable>,
    pub prefix: String,
    pub suffix: String,
}

/// Rewrite `source`, which `module` must have been parsed from unmodified (its
/// spans are byte offsets into it). Everything outside each `global[index]` span,
/// including the declarations and downstream `.data`, `[j]` or `arrayLength(&…)`,
/// is copied unchanged. Indices that themselves use listed globals are rewritten
/// recursively. Fails, never panics, on a span shape it cannot prove, or on a
/// listed global used other than as an indexed root.
pub(super) fn rewrite(
    source: &str,
    module: &naga::Module,
    rewrites: &[ArrayRewrite],
) -> Result<String> {
    let mut names = Vec::with_capacity(rewrites.len());
    for (at, rewrite) in rewrites.iter().enumerate() {
        let global = module
            .global_variables
            .try_get(rewrite.global)
            .map_err(|_| anyhow!("array rewrite {at} names a global outside this module"))?;
        let name = global.name.as_deref().unwrap_or("<unnamed>");
        let ty = module
            .types
            .get_handle(global.ty)
            .map_err(|_| anyhow!("global `{name}` has a type outside this module"))?;
        ensure!(
            matches!(ty.inner, naga::TypeInner::BindingArray { .. }),
            "global `{name}` is not a binding_array, so it has no array index to retarget"
        );
        ensure!(
            !rewrites[..at].iter().any(|r| r.global == rewrite.global),
            "global `{name}` is listed twice"
        );
        names.push(name);
    }
    let mut sites = Vec::new();
    for (name, function) in module
        .functions
        .iter()
        .map(|(_, f)| (f.name.as_deref(), f))
        .chain(
            module
                .entry_points
                .iter()
                .map(|e| (Some(e.name.as_str()), &e.function)),
        )
    {
        collect(source, function, rewrites, &names, &mut sites)
            .with_context(|| format!("in function `{}`", name.unwrap_or("<unnamed>")))?;
    }
    sites.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then(b.span.end.cmp(&a.span.end))
    });
    // One source access always lowers to one IR access today; tolerate an exact
    // duplicate (same text, same replacement) rather than double-rewriting it.
    sites.dedup_by(|a, b| a.span == b.span && a.rewrite.global == b.rewrite.global);
    let mut out = String::with_capacity(source.len());
    render(source, 0..source.len(), &sites, &mut out)?;
    Ok(out)
}

/// One `global[index]` edit: `span` covers the whole access (including any
/// parentheses naga folded into it), `index` the trimmed text between its brackets.
struct Site<'r> {
    span: Range<usize>,
    index: Range<usize>,
    rewrite: &'r ArrayRewrite,
}

/// Record every access rooted at a listed global's own `GlobalVariable`
/// expression. Locals, arguments and struct members that merely share the name
/// never produce that expression, so they are untouched by construction.
fn collect<'r>(
    source: &str,
    function: &naga::Function,
    rewrites: &'r [ArrayRewrite],
    names: &[&str],
    sites: &mut Vec<Site<'r>>,
) -> Result<()> {
    let expressions = &function.expressions;
    let listed = |global: naga::Handle<naga::GlobalVariable>| {
        rewrites.iter().position(|r| r.global == global)
    };
    let mut rooted = BTreeSet::new();
    for (handle, expression) in expressions.iter() {
        let (naga::Expression::Access { base, .. } | naga::Expression::AccessIndex { base, .. }) =
            *expression
        else {
            continue;
        };
        let Ok(&naga::Expression::GlobalVariable(global)) = expressions.try_get(base) else {
            continue;
        };
        let Some(at) = listed(global) else {
            continue;
        };
        rooted.insert(base);
        sites.push(site(
            source,
            expressions.get_span(handle),
            expressions.get_span(base),
            names[at],
            &rewrites[at],
        )?);
    }
    for (handle, expression) in expressions.iter() {
        if let naga::Expression::GlobalVariable(global) = *expression
            && let Some(at) = listed(global)
            && !rooted.contains(&handle)
        {
            bail!(
                "`{}` at {} is used without an index; only `{}[index]` can be retargeted",
                names[at],
                span_position(source, expressions.get_span(handle)),
                names[at],
            );
        }
    }
    Ok(())
}

/// Derive the edit from the access and global spans alone. naga's WGSL parser
/// spans an index access from the start of its base's primary expression to just
/// after its `]`, and a global identifier by the identifier itself. The index
/// expression's own span is deliberately not used: a `let`-bound index points at
/// the `let` initializer, possibly far away.
fn site<'r>(
    source: &str,
    access: naga::Span,
    global: naga::Span,
    name: &str,
    rewrite: &'r ArrayRewrite,
) -> Result<Site<'r>> {
    let (Some(span), Some(ident)) = (access.to_range(), global.to_range()) else {
        bail!("an access of `{name}` has no source span");
    };
    let at = span_position(source, access);
    ensure!(
        span.start <= ident.start && ident.end <= span.end,
        "access of `{name}` at {at} does not contain its global's span"
    );
    let text = |range: Range<usize>| {
        source.get(range.clone()).with_context(|| {
            format!("span {range:?} of `{name}` is outside the source or splits a UTF-8 character")
        })
    };
    ensure!(
        text(ident.clone())? == name,
        "span at {at} does not name `{name}`; was the module parsed from this exact source?"
    );
    let before = text(span.start..ident.start)?;
    let after = text(ident.end..span.end)?;
    let open = after
        .find('[')
        .with_context(|| format!("access of `{name}` at {at} has no `[`"))?;
    let (closing, bracketed) = after.split_at(open);
    let opened = before.matches('(').count();
    ensure!(
        before.chars().all(|c| c == '(' || c.is_whitespace())
            && closing.chars().all(|c| c == ')' || c.is_whitespace())
            && closing.matches(')').count() == opened,
        "access of `{name}` at {at} is not `{name}[…]` up to balanced parentheses: `{}`",
        text(span.clone())?
    );
    let inner = bracketed
        .strip_prefix('[')
        .and_then(|b| b.strip_suffix(']'))
        .with_context(|| format!("access of `{name}` at {at} does not end at its `]`"))?;
    let trimmed = inner.trim();
    ensure!(
        !trimmed.is_empty(),
        "access of `{name}` at {at} has an empty index"
    );
    let start = ident.end + open + 1 + (inner.len() - inner.trim_start().len());
    Ok(Site {
        span,
        index: start..start + trimmed.len(),
        rewrite,
    })
}

/// Copy `range` of `source` into `out`, replacing each outermost site in it and
/// recursing into that site's index for sites nested there. `sites` is sorted by
/// start (outer before inner) and lies inside `range`; partial overlaps fail.
fn render(source: &str, range: Range<usize>, sites: &[Site<'_>], out: &mut String) -> Result<()> {
    let text = |range: Range<usize>| {
        source
            .get(range.clone())
            .with_context(|| format!("rewrite span {range:?} is not a valid source slice"))
    };
    let mut cursor = range.start;
    let mut rest = sites;
    while let Some((site, tail)) = rest.split_first() {
        let nested = tail
            .iter()
            .take_while(|s| s.span.start < site.span.end)
            .count();
        let (children, after) = tail.split_at(nested);
        ensure!(
            cursor <= site.span.start && site.span.end <= range.end,
            "binding-array accesses overlap at {}",
            position(source, site.span.start)
        );
        if let Some(child) = children
            .iter()
            .find(|c| c.span.start < site.index.start || site.index.end < c.span.end)
        {
            bail!(
                "binding-array access at {} overlaps the access at {} outside its index",
                position(source, child.span.start),
                position(source, site.span.start),
            );
        }
        out.push_str(text(cursor..site.span.start)?);
        out.push_str(&site.rewrite.prefix);
        render(source, site.index.clone(), children, out)?;
        out.push_str(&site.rewrite.suffix);
        cursor = site.span.end;
        rest = after;
    }
    out.push_str(text(cursor..range.end)?);
    Ok(())
}

/// `line:column` (1-based, columns in characters) of a naga span for errors.
fn span_position(source: &str, span: naga::Span) -> String {
    span.to_range()
        .map_or_else(|| "<no span>".into(), |range| position(source, range.start))
}

/// `line:column` of a byte offset, without naga's panicking `Span::location`.
fn position(source: &str, offset: usize) -> String {
    match source.get(..offset) {
        Some(prefix) => {
            let line = prefix.matches('\n').count() + 1;
            let column = prefix.rsplit('\n').next().unwrap_or(prefix).chars().count() + 1;
            format!("{line}:{column}")
        }
        None => format!("byte {offset}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{ArrayRewrite, rewrite};

    fn parse(source: &str) -> naga::Module {
        naga::front::wgsl::parse_str(source).expect("test WGSL parses")
    }

    fn global(module: &naga::Module, name: &str) -> naga::Handle<naga::GlobalVariable> {
        module
            .global_variables
            .iter()
            .find(|(_, g)| g.name.as_deref() == Some(name))
            .map(|(h, _)| h)
            .expect("test global exists")
    }

    fn heap(module: &naga::Module, name: &str, base: &str) -> ArrayRewrite {
        ArrayRewrite {
            global: global(module, name),
            prefix: format!("ara_buffers[{base} + ("),
            suffix: ")]".into(),
        }
    }

    const STORAGE: &str = r"enable wgpu_binding_array;
struct Words { data: array<u32> };
struct Pick { verts: u32 };
const V: u32 = 0u;
const I: u32 = 4u;
@group(0) @binding(0) var<storage, read_write> verts: binding_array<Words, 4>;
@group(0) @binding(1) var<storage, read> idx: binding_array<Words, 4>;
@group(0) @binding(2) var<storage, read_write> ara_buffers: binding_array<Words>;
fn shadow(verts: u32, p: Pick) -> u32 { return verts + p.verts; }
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let k = i * 2u;
    verts[k].data[0] = arrayLength(&verts[ (i + 1u) ].data);
    verts[0].data[i] = idx[2u].data[k];
    verts[idx[i].data[0]].data[1] = shadow(i, Pick(i));
}
";

    #[test]
    fn storage_roots_named_parenthesised_constant_and_nested_indices() {
        let module = parse(STORAGE);
        let rewrites = [heap(&module, "verts", "V"), heap(&module, "idx", "I")];
        let out = rewrite(STORAGE, &module, &rewrites).expect("rewrite");
        for line in [
            // A `let` index keeps its name; downstream member/index/arrayLength stay.
            "ara_buffers[V + (k)].data[0] = arrayLength(&ara_buffers[V + ((i + 1u))].data);",
            // Constant indices lower to AccessIndex and keep their source text.
            "ara_buffers[V + (0)].data[i] = ara_buffers[I + (2u)].data[k];",
            "ara_buffers[V + (ara_buffers[I + (i)].data[0])].data[1] = shadow(i, Pick(i));",
            // Same-named argument and member are not the global.
            "fn shadow(verts: u32, p: Pick) -> u32 { return verts + p.verts; }",
            "var<storage, read_write> verts: binding_array<Words, 4>;",
        ] {
            assert!(out.contains(line), "missing `{line}` in:\n{out}");
        }
        parse(&out);
    }

    #[test]
    fn handle_arrays_and_parenthesised_base() {
        let source = r"enable wgpu_binding_array;
const T: u32 = 1u;
const S: u32 = 2u;
@group(0) @binding(0) var tex: binding_array<texture_2d<f32>, 4>;
@group(0) @binding(1) var samp: binding_array<sampler, 4>;
@group(0) @binding(2) var ara_textures: binding_array<texture_2d<f32>>;
@group(0) @binding(3) var ara_samplers: binding_array<sampler>;
@fragment
fn main(@location(0) uv: vec2<f32>, @location(1) @interpolate(flat) m: u32) -> @location(0) vec4<f32> {
    return textureSampleLevel((tex)[m], samp[m], uv, 0.0);
}
";
        let module = parse(source);
        let rewrites = [
            ArrayRewrite {
                global: global(&module, "tex"),
                prefix: "ara_textures[T + (".into(),
                suffix: ")]".into(),
            },
            ArrayRewrite {
                global: global(&module, "samp"),
                prefix: "ara_samplers[S + (".into(),
                suffix: ")]".into(),
            },
        ];
        let out = rewrite(source, &module, &rewrites).expect("rewrite");
        assert!(
            out.contains(
                "textureSampleLevel(ara_textures[T + (m)], ara_samplers[S + (m)], uv, 0.0)"
            ),
            "{out}"
        );
        parse(&out);
    }

    #[test]
    fn rejects_non_arrays_and_mismatched_source() {
        let module = parse(STORAGE);
        let unused = ArrayRewrite {
            global: global(&module, "ara_buffers"),
            prefix: String::new(),
            suffix: String::new(),
        };
        // `ara_buffers` is a binding array with no indexed use: nothing changes.
        assert_eq!(
            rewrite(STORAGE, &module, &[unused]).expect("unused"),
            STORAGE
        );
        let shifted = format!(" {STORAGE}");
        let error = rewrite(&shifted, &module, &[heap(&module, "verts", "V")])
            .expect_err("spans no longer match");
        assert!(format!("{error:#}").contains("does not name"), "{error:#}");

        let source = "var<private> plain: array<u32, 2>;\n@compute @workgroup_size(1)\nfn main() { plain[0] = 1u; }\n";
        let module = parse(source);
        let error = rewrite(source, &module, &[heap(&module, "plain", "V")])
            .expect_err("not a binding array");
        assert!(
            format!("{error:#}").contains("not a binding_array"),
            "{error:#}"
        );
    }
}
