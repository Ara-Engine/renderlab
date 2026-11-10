//! Locals that shadow a resource global, renamed before the textual heap rewrite.
//!
//! The SPIR-V path of ray-query modules rewrites resource globals by name in the
//! original WGSL text. A `let camera = ...` inside a function would be rewritten along
//! with the `camera` global it hides, so such locals get a distinct name first. The
//! rename follows WGSL scoping: a parameter covers its function body, a `let`, `var`
//! or `const` covers the rest of its block from the end of its own statement.
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

const SUFFIX: &str = "_ara_local";

/// Returns `source` with every function parameter and block-scoped declaration named
/// like one of `globals` renamed, together with its uses, to that name plus
/// `_ara_local`. Module-scope declarations, struct members and member accesses are
/// untouched. A declaration inside parentheses (a `for` initialiser) is rejected: its
/// scope is the loop, which brace tracking alone cannot bound.
pub fn rename(source: &str, globals: &BTreeSet<&str>) -> Result<String> {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    // Active shadows with the brace depth they die below, and ones waiting for the
    // end of their declaring statement or the opening of their function body.
    let mut active: Vec<(&str, usize)> = Vec::new();
    let mut after_statement: Vec<(&str, usize)> = Vec::new();
    let mut parameters: Vec<&str> = Vec::new();
    let (mut depth, mut parens) = (0usize, 0usize);
    // 0: outside a signature; 1: saw `fn`, before `(`; 2: inside the parameter list;
    // 3: after it, before the body.
    let mut signature = 0u8;
    let mut signature_parens = 0usize;
    // Set between a declaring keyword and the name it declares; `template` counts the
    // angle brackets of `var<function>` in between.
    let mut declaring = false;
    let mut template = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        if source[at..].starts_with("//") {
            let end = source[at..].find('\n').map_or(bytes.len(), |n| at + n);
            out.push_str(&source[at..end]);
            at = end;
        } else if source[at..].starts_with("/*") {
            let end = source[at + 2..]
                .find("*/")
                .map_or(bytes.len(), |n| at + n + 4);
            out.push_str(&source[at..end]);
            at = end;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            let word = &source[start..at];
            let member = source[..start].trim_end().ends_with('.');
            let typed = source[at..].trim_start().starts_with(':');
            if declaring && template > 0 {
                out.push_str(word);
            } else if std::mem::take(&mut declaring) && globals.contains(word) {
                ensure!(
                    parens == 0,
                    "local `{word}` shadows a resource global inside parentheses"
                );
                after_statement.push((word, depth));
                out.push_str(word);
                out.push_str(SUFFIX);
            } else if signature == 2
                && parens == signature_parens
                && typed
                && !member
                && globals.contains(word)
            {
                parameters.push(word);
                out.push_str(word);
                out.push_str(SUFFIX);
            } else if !member && active.iter().any(|(name, _)| *name == word) {
                out.push_str(word);
                out.push_str(SUFFIX);
            } else {
                if word == "fn" && depth == 0 {
                    signature = 1;
                }
                declaring = depth > 0 && matches!(word, "let" | "var" | "const");
                template = 0;
                out.push_str(word);
            }
        } else {
            match c {
                b'(' => {
                    parens += 1;
                    if signature == 1 {
                        (signature, signature_parens) = (2, parens);
                    }
                }
                b')' => {
                    if signature == 2 && parens == signature_parens {
                        signature = 3;
                    }
                    parens = parens.saturating_sub(1);
                }
                b'{' => {
                    depth += 1;
                    if signature == 3 {
                        signature = 0;
                        active.extend(parameters.drain(..).map(|name| (name, depth)));
                    }
                }
                b'}' => {
                    active.retain(|(_, born)| *born < depth);
                    depth = depth.saturating_sub(1);
                }
                b';' if parens == 0 => {
                    if signature == 3 {
                        // A declaration without a body has no scope to rename in.
                        signature = 0;
                        parameters.clear();
                    }
                    active.append(&mut after_statement);
                }
                b'<' if declaring => template += 1,
                b'>' if declaring && template > 0 => template -= 1,
                _ => {}
            }
            let width = source[at..].chars().next().map_or(1, char::len_utf8);
            // Anything but space or a template list between the keyword and a name
            // means this was not a declaration after all.
            if declaring && template == 0 && !c.is_ascii_whitespace() && c != b'>' {
                declaring = false;
            }
            out.push_str(&source[at..at + width]);
            at += width;
        }
    }
    Ok(out)
}
