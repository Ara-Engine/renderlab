//! The bench's text override for every pass outside `gi` and `vsm` (docs/bench.md): a saved
//! shader file becomes hunks against the text the bench started with (each changed run of lines
//! with enough context to occur once in that file), and every shader module the process creates
//! (`pipeline_compilation::shader_module`) has them applied wherever their old text occurs. The
//! passes compose these files through constants, slices and seams; the hunks reach whatever
//! survives of the edited lines in each composed module, with no per-site plumbing.
//!
//! A module the hunks change is checked with naga first. When it fails, the module is created
//! from the last working hunks instead and the error is kept for the bench to report
//! (`take_error`), so the device never sees invalid text.
use std::collections::BTreeMap;
use std::sync::Mutex;

/// One edit: `old` occurs once in the file's starting text and becomes `new`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hunk {
    pub old: String,
    pub new: String,
}

#[derive(Default)]
struct State {
    /// Each edited file's hunks now, and the last set that reloaded without an error.
    current: BTreeMap<String, Vec<Hunk>>,
    good: BTreeMap<String, Vec<Hunk>>,
    /// Modules created since `reset_counts` that each file's current hunks changed.
    reached: BTreeMap<String, usize>,
    /// The first rejected module since the last `take_error`.
    error: Option<String>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(STATE.lock().unwrap().get_or_insert_with(State::default))
}

/// Sets `file`'s hunks from its starting text `base` and its text `now`. Returns whether it
/// has any (false: the file is back to its starting text).
pub(crate) fn set(file: &str, base: &str, now: &str) -> bool {
    let hunks = hunks(base, now);
    let edited = !hunks.is_empty();
    with(|s| {
        if hunks.is_empty() {
            s.current.remove(file);
        } else {
            s.current.insert(file.to_owned(), hunks);
        }
    });
    edited
}
/// After a reload: the current hunks become the working set (`ok`), or go back to it.
pub(crate) fn settle(ok: bool) {
    with(|s| {
        if ok {
            s.good = s.current.clone();
        } else {
            s.current = s.good.clone();
        }
    });
}
pub(crate) fn reset_counts() {
    with(|s| s.reached.clear());
}
/// Modules the current hunks of `file` changed since `reset_counts`.
pub(crate) fn reached(file: &str) -> usize {
    with(|s| s.reached.get(file).copied().unwrap_or(0))
}
pub(crate) fn take_error() -> Option<String> {
    with(|s| s.error.take())
}

/// The module text with the current hunks applied, or `None` when none applies. Called for
/// every module the process creates.
pub(crate) fn rewrite(source: &str) -> Option<String> {
    let (current, good) = {
        let guard = STATE.lock().unwrap();
        let s = guard.as_ref()?;
        if s.current.is_empty() {
            return None;
        }
        (s.current.clone(), s.good.clone())
    };
    let (text, files) = apply(source, &current);
    let text = text?;
    match check(&text) {
        Ok(()) => {
            with(|s| files.iter().for_each(|f| *s.reached.entry(f.clone()).or_default() += 1));
            Some(text)
        }
        Err(error) => {
            with(|s| {
                s.error.get_or_insert(error);
            });
            apply(source, &good).0
        }
    }
}

/// `source` with every hunk applied wherever its old text occurs, and the files whose hunks
/// changed it; `None` when nothing applied.
fn apply(source: &str, edits: &BTreeMap<String, Vec<Hunk>>) -> (Option<String>, Vec<String>) {
    let mut text = None::<String>;
    let mut files = vec![];
    for (file, hunks) in edits {
        for hunk in hunks {
            let now = text.as_deref().unwrap_or(source);
            if now.contains(&hunk.old) {
                text = Some(now.replace(&hunk.old, &hunk.new));
                if files.last() != Some(file) {
                    files.push(file.clone());
                }
            }
        }
    }
    (text, files)
}

fn check(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map(|_| ())
        .map_err(|e| e.emit_to_string(source))
}

/// The hunks turning `base` into `now`: each changed run of lines (a line diff of the part
/// between their common first and last lines), widened by whole lines of unchanged context
/// until its old text occurs once in `base`; runs whose widened text would overlap merge.
pub(crate) fn hunks(base: &str, now: &str) -> Vec<Hunk> {
    if base == now {
        return vec![];
    }
    let a: Vec<&str> = base.split_inclusive('\n').collect();
    let b: Vec<&str> = now.split_inclusive('\n').collect();
    let mut runs = changed_runs(&a, &b);
    loop {
        let widened: Vec<(usize, usize, usize, usize)> = runs.iter().map(|&r| widen(&a, r)).collect();
        // Merge runs whose widened old ranges touch, then widen again.
        let mut merged: Vec<(usize, usize, usize, usize)> = vec![];
        let mut changed = false;
        for (i, w) in widened.iter().enumerate() {
            if let Some(last) = merged.last_mut()
                && w.0 < widened[i - 1].1
            {
                let run = runs[i];
                *last = (last.0, run.1, last.2, run.3);
                changed = true;
                continue;
            }
            merged.push(runs[i]);
        }
        if !changed {
            return widened
                .into_iter()
                .zip(&runs)
                .map(|((a0, a1, _, _), &(r0, r1, s0, s1))| Hunk {
                    old: a[a0..a1].concat(),
                    new: [a[a0..r0].concat(), b[s0..s1].concat(), a[r1..a1].concat()].concat(),
                })
                .collect();
        }
        runs = merged;
    }
}

/// The changed runs of lines: (old start, old end, new start, new end).
fn changed_runs(a: &[&str], b: &[&str]) -> Vec<(usize, usize, usize, usize)> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    // A larger middle (a rewrite of most of a file) is one run.
    if ma.len() * mb.len() > 4_000_000 || ma.is_empty() || mb.is_empty() {
        return vec![(prefix, a.len() - suffix, prefix, b.len() - suffix)];
    }
    // Longest common subsequence of the middle's lines, then the runs between its matches.
    let (n, m) = (ma.len(), mb.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * (m + 1) + j] = if ma[i] == mb[j] {
                table[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
            };
        }
    }
    let (mut i, mut j, mut runs) = (0, 0, vec![]);
    let mut open: Option<(usize, usize)> = None;
    while i < n || j < m {
        if i < n && j < m && ma[i] == mb[j] {
            if let Some((i0, j0)) = open.take() {
                runs.push((prefix + i0, prefix + i, prefix + j0, prefix + j));
            }
            i += 1;
            j += 1;
            continue;
        }
        open.get_or_insert((i, j));
        if j < m && (i == n || table[i * (m + 1) + j + 1] >= table[(i + 1) * (m + 1) + j]) {
            j += 1;
        } else {
            i += 1;
        }
    }
    if let Some((i0, j0)) = open {
        runs.push((prefix + i0, prefix + n, prefix + j0, prefix + m));
    }
    runs
}

/// Run `r`'s old range widened by whole lines until its text is non-empty and occurs once in
/// the file (or covers it).
fn widen(a: &[&str], r: (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
    let base = a.concat();
    let (mut lo, mut hi) = (r.0, r.1);
    loop {
        let old = a[lo..hi].concat();
        if !old.trim().is_empty() && base.matches(old.as_str()).count() == 1 || lo == 0 && hi == a.len() {
            return (lo, hi, r.2, r.3);
        }
        lo = lo.saturating_sub(1);
        hi = (hi + 1).min(a.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(base: &str, now: &str) -> String {
        let mut text = base.to_owned();
        for h in hunks(base, now) {
            assert_eq!(text.matches(&h.old).count(), 1, "{h:?}");
            text = text.replace(&h.old, &h.new);
        }
        text
    }

    #[test]
    fn hunks_rebuild_the_new_text_from_the_old() {
        let base = "fn a() {\n  return 1.0;\n}\nfn b() {\n  return 1.0;\n}\nfn c() {}\n";
        for now in [
            "fn a() {\n  return 1.0;\n}\nfn b() {\n  return 2.0;\n}\nfn c() {}\n", // a repeated line
            "fn a() {\n  return 3.0;\n}\nfn b() {\n  return 2.0;\n}\nfn c() {}\n", // two runs
            "fn z() {}\nfn a() {\n  return 1.0;\n}\nfn b() {\n  return 1.0;\n}\nfn c() {}\n", // insert first
            "fn a() {\n  return 1.0;\n}\nfn b() {\n  return 1.0;\n}\nfn c() {}\nfn d() {}\n", // append
            "fn a() {\n  return 1.0;\n}\nfn c() {}\n",                                   // delete
        ] {
            assert_eq!(patch(base, now), now);
        }
        assert!(hunks(base, base).is_empty());
    }

    /// A hunk lands in a composed module holding only a slice of the file, as the passes compose.
    #[test]
    fn a_hunk_reaches_a_slice_of_its_file() {
        let base = "// head\nconst A:f32=0.5;\nfn f()->f32 { return A; }\n// tail\nfn g() {}\n";
        let now = base.replace("0.5", "0.75");
        let module = format!("enable x;\n{}// other\n", &base[base.find("const").unwrap()..base.find("// tail").unwrap()]);
        let edits = BTreeMap::from([("f.wgsl".to_owned(), hunks(base, &now))]);
        let (text, files) = apply(&module, &edits);
        assert_eq!(text.unwrap(), module.replace("0.5", "0.75"));
        assert_eq!(files, ["f.wgsl"]);
    }
}
