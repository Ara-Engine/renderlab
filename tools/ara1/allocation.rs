//! Measurement only: named allocator inventory and this process's DXGI usage.
use renderlab::engine::Engine;
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

pub fn usage() -> serde_json::Value {
    dxgi::local_usage()
}

pub fn write(engine: &Engine, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let report = engine
        .device()
        .generate_allocator_report()
        .ok_or("allocator report unavailable")?;
    let mut groups: BTreeMap<&str, (u64, usize)> = BTreeMap::new();
    for a in &report.allocations {
        let g = groups.entry(&a.name).or_default();
        g.0 += a.size;
        g.1 += 1;
    }
    let stats = engine.renderer().stats();
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "allocated_bytes": report.total_allocated_bytes,
            "reserved_bytes": report.total_reserved_bytes,
            "memory_hints": format!("{:?}", renderlab::MEMORY_HINTS),
            "dxgi": dxgi::local_usage(),
            "hal_buffer_bytes": stats.hal.buffer_bytes,
            "hal_texture_bytes": stats.hal.texture_bytes,
            "ownership": format!("{:?}", stats.ownership),
            "by_label": groups.into_iter().map(|(name,(bytes,count))| json!({"name":name,"bytes":bytes,"count":count})).collect::<Vec<_>>(),
            "allocations": report.allocations.iter().map(|a| json!({"name":a.name,"bytes":a.size,"offset":a.offset})).collect::<Vec<_>>(),
            "blocks": report.blocks.iter().map(|b| json!({"bytes":b.size,"allocations":[b.allocations.start,b.allocations.end]})).collect::<Vec<_>>(),
        }))?,
    )?;
    Ok(())
}

#[cfg(not(windows))]
mod dxgi {
    pub fn local_usage() -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[cfg(windows)]
mod dxgi {
    use windows::{
        Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
            DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3,
            IDXGIFactory1,
        },
        core::Interface,
    };

    pub fn local_usage() -> serde_json::Value {
        // SAFETY: DXGI owns every returned COM reference. Queries write to initialized
        // SDK structs and never create a graphics device or change a memory reservation.
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
                return serde_json::Value::Null;
            };
            let mut rows = vec![];
            for index in 0..16 {
                let Ok(adapter) = factory.EnumAdapters1(index) else {
                    break;
                };
                let Ok(adapter) = adapter.cast::<IDXGIAdapter3>() else {
                    continue;
                };
                let mut local = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                let mut nonlocal = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                if adapter
                    .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut local)
                    .is_err()
                {
                    continue;
                }
                let nonlocal_ok = adapter
                    .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, &mut nonlocal)
                    .is_ok();
                rows.push(serde_json::json!({"adapter_index":index,"local_used_bytes":local.CurrentUsage,
                    "local_budget_bytes":local.Budget,"nonlocal_used_bytes":nonlocal_ok.then_some(nonlocal.CurrentUsage)}));
            }
            serde_json::json!(rows)
        }
    }
}
