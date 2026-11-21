//! Per-owner GPU allocator reporting and baseline delta tables for `lightbench`.
use std::collections::BTreeMap;
use std::path::Path;

/// Classifies an allocation label into the owner buckets used by `docs/memory-m2.md`.
fn owner_of(label: &str) -> &'static str {
    let l = label;
    if l.starts_with("N8")
        || l.starts_with("vsm")
        || l.contains("Disk lookup")
        || l.contains("clipmap")
        || l.contains("Sparse clipmap")
        || l.contains("virtual shadow")
    {
        return "vsm";
    }
    if l.starts_with("LC1 local shadow maps") {
        return "local_shadows";
    }
    if l.starts_with("GI ")
        || l.starts_with("G2")
        || l.contains("indirect cache")
        || l.contains("gi_")
        || l.contains("lighting cache")
    {
        return "gi_cache";
    }
    if l.starts_with("RL") || l.contains("reflection") || l.contains("glossy") {
        return "reflections";
    }
    if l.starts_with("N1c")
        || l.contains("software reflection geometry")
        || l.contains("software geometry")
        || l.contains("BVH")
        || l.contains("bvh")
        || l.contains("instance tree")
    {
        return "sw_trace";
    }
    if l.starts_with("mesh ")
        || l.starts_with("VB")
        || l.contains("skinn")
        || l.contains("cluster")
        || l.contains("visibility hardware")
    {
        return "geometry";
    }
    if l.contains("immutable material texture") || l.contains("environment cube") || l.contains("immutable") {
        return "textures";
    }
    if l.starts_with("TP") || l.contains("transmission") || l.contains("pane") {
        return "transparency";
    }
    if l.contains("TR1")
        || l.contains("history")
        || l.contains("fresh")
        || l.contains("primary")
        || l.contains("HDR")
        || l.contains("post ")
        || l.contains("bloom")
        || l.contains("depth")
        || l.contains("view output")
        || l.contains("receiver")
        || l.contains("deferred")
        || l.contains("samples")
        || l.contains("visibility")
        || l.contains("exposed")
        || l.contains("AT1")
        || l.contains("VF2")
        || l.contains("fog")
        || l.contains("atmosphere")
        || l.contains("sky")
    {
        return "targets";
    }
    "other"
}

/// Returns live GPU allocator usage grouped by owner in MiB, plus `allocated`, `reserved`, `slack`.
pub(super) fn memory_by_owner(device: &wgpu::Device) -> Option<BTreeMap<String, f64>> {
    let report = device.generate_allocator_report()?;
    let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
    let mut by_owner: BTreeMap<&'static str, u64> = BTreeMap::new();
    for a in &report.allocations {
        *by_owner.entry(owner_of(&a.name)).or_default() += a.size;
    }
    let mut out = BTreeMap::new();
    for (k, v) in by_owner {
        out.insert(k.to_owned(), mib(v));
    }
    out.insert("allocated".to_owned(), mib(report.total_allocated_bytes));
    out.insert("reserved".to_owned(), mib(report.total_reserved_bytes));
    out.insert("slack".to_owned(), mib(report.total_reserved_bytes.saturating_sub(report.total_allocated_bytes)));
    Some(out)
}

/// Formats a compact one-line summary of allocator usage by owner (sorted largest first).
pub(super) fn format_memory(mem: &BTreeMap<String, f64>) -> String {
    let reserved = mem.get("reserved").copied().unwrap_or(0.0);
    let allocated = mem.get("allocated").copied().unwrap_or(0.0);
    let slack = mem.get("slack").copied().unwrap_or(0.0);
    let mut owners: Vec<(&String, f64)> = mem
        .iter()
        .filter(|(k, v)| !matches!(k.as_str(), "reserved" | "allocated" | "slack") && **v >= 1.0)
        .map(|(k, &v)| (k, v))
        .collect();
    owners.sort_by(|a, b| b.1.total_cmp(&a.1));
    let parts: Vec<String> = owners.into_iter().map(|(k, v)| format!("{k}={v:.0}")).collect();
    format!("reserved={reserved:.0} MiB (alloc={allocated:.0} slack={slack:.0}) | {}", parts.join(" "))
}

/// One fixture+path record parsed from or emitted as a `lightbench` JSON line.
#[derive(Clone, Default)]
pub(super) struct Entry {
    pub metrics: BTreeMap<String, f64>,
    pub pass_ms: BTreeMap<String, f64>,
    pub memory_mib: BTreeMap<String, f64>,
}

/// Loads `lightbench` JSON lines from `path` (ignoring non-JSON lines), keyed by `(fixture, path)`.
pub(super) fn load_baseline(path: &Path) -> Result<BTreeMap<(String, String), Entry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') || !trimmed.contains("\"bench\":\"lightbench\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else { continue };
        let (Some(fixture), Some(p)) = (v["fixture"].as_str(), v["path"].as_str()) else { continue };
        let map = |key: &str| -> BTreeMap<String, f64> {
            v[key]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, val)| val.as_f64().map(|f| (k.clone(), f))).collect())
                .unwrap_or_default()
        };
        out.insert(
            (fixture.to_owned(), p.to_owned()),
            Entry { metrics: map("metrics"), pass_ms: map("pass_ms"), memory_mib: map("memory_mib") },
        );
    }
    Ok(out)
}

/// Prints a compact delta summary comparing `current` against `baseline`.
pub(super) fn print_comparison(
    baseline: &BTreeMap<(String, String), Entry>,
    current: &BTreeMap<(String, String), Entry>,
) {
    println!("--- lightbench delta vs baseline ---");
    for ((fixture, path), cur) in current {
        let Some(base) = baseline.get(&(fixture.clone(), path.clone())) else {
            println!("  {fixture:<13} {path:<4} (new; not in baseline)");
            continue;
        };
        let mut parts = Vec::new();
        if let (Some(&b), Some(&c)) = (base.pass_ms.get("span"), cur.pass_ms.get("span")) {
            let d = c - b;
            let pct = if b > 1e-6 { 100.0 * d / b } else { 0.0 };
            parts.push(format!("span {b:.2}->{c:.2} ms ({d:+.2}, {pct:+.1}%)"));
        }
        if let (Some(&b), Some(&c)) = (base.pass_ms.get("still_span"), cur.pass_ms.get("still_span")) {
            parts.push(format!("still {b:.2}->{c:.2} ({:+.2})", c - b));
        }
        if let (Some(&b), Some(&c)) = (base.pass_ms.get("cache"), cur.pass_ms.get("cache")) {
            parts.push(format!("cache {b:.2}->{c:.2} ({:+.2})", c - b));
        }
        if let (Some(&b), Some(&c)) = (base.memory_mib.get("reserved"), cur.memory_mib.get("reserved")) {
            parts.push(format!("mem {b:.0}->{c:.0} MiB ({:+.0})", c - b));
        }
        // Highlight key quality metrics when they change materially.
        let mut qual = Vec::new();
        for key in ["energy", "err", "arrive_energy", "hold_energy", "lit_in_shadow", "rail_wobble_px", "pass_ghost_px"] {
            if let (Some(&b), Some(&c)) = (base.metrics.get(key), cur.metrics.get(key)) {
                if (c - b).abs() > 1e-4 {
                    qual.push(format!("{key} {b:.4}->{c:.4}"));
                }
            }
        }
        // Top changed individual passes (|delta| >= 0.03 ms).
        let mut diffs: Vec<(&str, f64, f64)> = cur
            .pass_ms
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "span" | "cache" | "still_span" | "still_cache"))
            .filter_map(|(k, &c)| {
                let b = base.pass_ms.get(k).copied().unwrap_or(0.0);
                let d = c - b;
                (d.abs() >= 0.03).then_some((k.as_str(), d, c))
            })
            .collect();
        diffs.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        let top_passes: Vec<String> = diffs.into_iter().take(5).map(|(k, d, c)| format!("{k}{d:+.2}({c:.2})")).collect();
        let mut line = format!("  {fixture:<13} {path:<4} {}", parts.join(" | "));
        if !qual.is_empty() {
            line.push_str(&format!(" | Q: {}", qual.join(", ")));
        }
        if !top_passes.is_empty() {
            line.push_str(&format!(" | passes: {}", top_passes.join(" ")));
        }
        println!("{line}");
    }
}
