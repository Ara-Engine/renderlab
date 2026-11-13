//! Measurement: `RENDERLAB_MEMORY_REPORT=PATH[,FRAME]` writes the device allocator's report at
//! rendered frame FRAME (default 40), once per process: every live allocation grouped by its
//! resource label (largest first), the allocated and reserved totals, and the memory blocks, so
//! block slack (reserved minus allocated) shows beside the owners. Unset, nothing runs.
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};

static WRITTEN: AtomicBool = AtomicBool::new(false);

/// Called once per rendered frame with the renderer's frame count.
pub(crate) fn maybe_write(device: &wgpu::Device, frame: u64) {
    let Some(value) = std::env::var_os("RENDERLAB_MEMORY_REPORT") else {
        return;
    };
    let value = value.to_string_lossy().into_owned();
    let (path, at) = match value.rsplit_once(',').map(|(p, n)| (p, n.parse::<u64>())) {
        Some((path, Ok(n))) => (path.to_owned(), n),
        _ => (value.clone(), 40),
    };
    if frame != at || WRITTEN.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::fs::write(&path, report_json(device, frame));
}

fn report_json(device: &wgpu::Device, frame: u64) -> String {
    let Some(report) = device.generate_allocator_report() else {
        return "{\"error\":\"no allocator report on this backend\"}".to_owned();
    };
    let mut groups: std::collections::BTreeMap<&str, (u64, u64)> = Default::default();
    for a in &report.allocations {
        let e = groups.entry(a.name.as_str()).or_default();
        e.0 += a.size;
        e.1 += 1;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));
    let (allocated, reserved) = (report.total_allocated_bytes, report.total_reserved_bytes);
    let mut out = format!(
        "{{\n\"frame\": {frame},\n\"allocated_bytes\": {allocated},\n\"reserved_bytes\": {reserved},\n\"slack_bytes\": {},\n\"blocks\": [{}],\n\"by_label\": [\n",
        reserved.saturating_sub(allocated),
        report
            .blocks
            .iter()
            .map(|b| b.size.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
    for (i, (name, (bytes, count))) in groups.iter().enumerate() {
        let label: String = name.chars().flat_map(char::escape_default).collect();
        let comma = if i + 1 < groups.len() { "," } else { "" };
        let _ = writeln!(
            out,
            "{{\"label\": \"{label}\", \"bytes\": {bytes}, \"count\": {count}}}{comma}"
        );
    }
    out.push_str("]\n}\n");
    out
}
