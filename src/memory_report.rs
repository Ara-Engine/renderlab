//! Measurement only: `RENDERLAB_MEMORY_REPORT=<file>[;<frame>]` writes the allocator's report once,
//! after renderer frame `<frame>` (default 240): a header line with the reserved and allocated
//! totals and the block count, then one line per live allocation, `<bytes>\t<label>`. Grouped by
//! owner later (main-thread/clipmap/memory/owners.py). Nothing is written without the variable.
use std::sync::atomic::{AtomicBool, Ordering};

static WRITTEN: AtomicBool = AtomicBool::new(false);

fn target() -> Option<&'static (std::path::PathBuf, u64)> {
    static TARGET: std::sync::OnceLock<Option<(std::path::PathBuf, u64)>> = std::sync::OnceLock::new();
    TARGET
        .get_or_init(|| {
            let value = std::env::var("RENDERLAB_MEMORY_REPORT").ok()?;
            let (path, frame) = value.split_once(';').unwrap_or((&value, "240"));
            Some((path.into(), frame.trim().parse().unwrap_or(240)))
        })
        .as_ref()
}

/// Called once a frame with the renderer's frame count. Returns true on the frame it writes, so
/// other one-shot measurements (`Renderer::measure_gi_occupancy`) run at the same moment.
pub(crate) fn frame(device: &wgpu::Device, frame: u64) -> bool {
    let Some((path, at)) = target() else { return false };
    if frame < *at || WRITTEN.swap(true, Ordering::Relaxed) {
        return false;
    }
    let Some(report) = device.generate_allocator_report() else { return true };
    let mut text = format!(
        "frame {frame} reserved {} allocated {} blocks {} block_bytes {}\n",
        report.total_reserved_bytes,
        report.total_allocated_bytes,
        report.blocks.len(),
        report.blocks.iter().map(|b| b.size).sum::<u64>()
    );
    for a in &report.allocations {
        text.push_str(&format!("{}\t{}\n", a.size, a.name));
    }
    if let Err(error) = std::fs::write(path, text) {
        eprintln!("renderlab: RENDERLAB_MEMORY_REPORT {}: {error}", path.display());
    }
    true
}
