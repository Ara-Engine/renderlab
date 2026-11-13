//! Measurement: `RENDERLAB_MEMORY_REPORT=PATH[,FRAME,...]` writes the device allocator's report
//! before the work of each listed frame (FRAME frames already rendered; default 40; 0 is the
//! loaded scene before any frame), to PATH with `-FRAME` before its extension: every live
//! allocation grouped by its resource label (largest first), the allocated and reserved
//! totals and the memory blocks, so block slack (reserved minus allocated) shows beside the
//! owners. Unset, nothing runs.
use std::fmt::Write as _;
use std::sync::Mutex;

static WRITTEN: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// Called at the start of every frame with the frames rendered so far.
pub(crate) fn maybe_write(device: &wgpu::Device, frames: u64) {
    let Some(value) = std::env::var_os("RENDERLAB_MEMORY_REPORT") else {
        return;
    };
    let value = value.to_string_lossy().into_owned();
    let mut parts = value.split(',');
    let path = parts.next().unwrap_or_default().to_owned();
    let mut at: Vec<u64> = parts.filter_map(|n| n.trim().parse().ok()).collect();
    if at.is_empty() {
        at.push(40);
    }
    if !at.contains(&frames) {
        return;
    }
    {
        let mut written = WRITTEN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if written.contains(&frames) {
            return;
        }
        written.push(frames);
    }
    let file = std::path::Path::new(&path);
    let stem = file
        .file_stem()
        .map_or("memory".into(), |s| s.to_string_lossy());
    let ext = file
        .extension()
        .map_or("json".into(), |s| s.to_string_lossy());
    let _ = std::fs::write(
        file.with_file_name(format!("{stem}-{frames}.{ext}")),
        report_json(device, frames),
    );
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
