//! Keep original ray-query WGSL while moving its soon-unused bindings out of the heap.
use anyhow::{Context, Result, ensure};
/// Naga's WGSL writer cannot emit ray-query IR. Preserve original declarations
/// until entry compaction; their old descriptors become unused after adaptation.
/// Move groups so they cannot collide with new group-zero heap declarations.
pub(super) fn relocate_groups(source: &str, module: &naga::Module) -> Result<String> {
    let mut globals: Vec<_> = module
        .global_variables
        .iter()
        .filter_map(|(h, g)| {
            g.binding
                .map(|b| (module.global_variables.get_span(h), b.group))
        })
        .collect();
    globals.sort_by_key(|(span, _)| span.to_range().map(|r| r.start));
    let mut edits = Vec::new();
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if source[at..].starts_with("//") {
            at = source[at..].find('\n').map_or(bytes.len(), |n| at + n);
            continue;
        }
        if source[at..].starts_with("/*") {
            let mut nesting = 1;
            at += 2;
            while at < bytes.len() && nesting > 0 {
                if source[at..].starts_with("/*") {
                    nesting += 1;
                    at += 2;
                } else if source[at..].starts_with("*/") {
                    nesting -= 1;
                    at += 2;
                } else {
                    at += source[at..]
                        .chars()
                        .next()
                        .context("comment character missing")?
                        .len_utf8();
                }
            }
            ensure!(nesting == 0, "unterminated source comment");
            continue;
        }
        if bytes[at] != b'@' {
            at += source[at..]
                .chars()
                .next()
                .context("source character missing")?
                .len_utf8();
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let name = at;
        while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
            at += 1;
        }
        if &source[name..at] != "group" {
            continue;
        }
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        ensure!(
            bytes.get(at) == Some(&b'('),
            "unsupported group attribute shape"
        );
        at += 1;
        let start = at;
        let mut depth = 1;
        while at < bytes.len() && depth > 0 {
            match bytes[at] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            };
            if depth > 0 {
                at += 1;
            }
        }
        ensure!(depth == 0, "unterminated group attribute");
        let (_, group) = globals
            .iter()
            .find(|(span, _)| span.to_range().is_some_and(|r| r.start >= at))
            .context("group attribute has no following global")?;
        let relocated = group.checked_add(64).context("source group overflow")?;
        edits.push((start..at, relocated.to_string()));
        at += 1;
    }
    ensure!(
        edits.len() == globals.len(),
        "source resource attributes did not match reflected globals"
    );
    let mut out = source.to_owned();
    for (range, value) in edits.into_iter().rev() {
        out.replace_range(range, &value);
    }
    Ok(out)
}
