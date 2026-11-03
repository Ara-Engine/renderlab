#![allow(dead_code)]
use crate::policy::{self, Policy};
use renderlab::{Limits, virtual_geometry::StreamOptions};
use serde_json::{Value, json};
use std::time::Duration;
pub fn stream(p: &Policy) -> Result<StreamOptions, String> {
    let mut o = StreamOptions {
        limits: p.virtual_limits(),
        ..Default::default()
    };
    o.geometry_bytes = p.size("runtime", "geometry_bytes", o.geometry_bytes);
    o.metadata_bytes = p.size("runtime", "metadata_bytes", o.metadata_bytes);
    o.texture_bytes = p.size("runtime", "texture_bytes", o.texture_bytes);
    o.bootstrap_bytes = p.size("runtime", "bootstrap_bytes", o.bootstrap_bytes);
    o.read_slots = p.size("runtime", "read_slots", o.read_slots);
    o.uploads_per_tick = p.size("runtime", "uploads_per_tick", o.uploads_per_tick);
    o.max_candidates = p.size("runtime", "max_candidates", o.max_candidates);
    o.max_clusters = p.size("runtime", "max_clusters", o.max_clusters);
    o.error_pixels = p.real("runtime", "error_pixels", o.error_pixels);
    o.io_delay = Duration::from_millis(p.size("runtime", "io_delay_ms", 0) as u64);
    o.validate().map_err(|e| e.to_string())?;
    Ok(o)
}
pub fn renderer(p: &Policy, mut out: Limits) -> Limits {
    out.max_instances = p.dimension("renderer", "max_instances", out.max_instances);
    out.max_mesh_bytes = p.size("renderer", "max_mesh_bytes", out.max_mesh_bytes as usize) as u64;
    out.max_texture_bytes = p.size(
        "renderer",
        "max_texture_bytes",
        out.max_texture_bytes as usize,
    ) as u64;
    out
}
pub fn json(p: &Policy, s: StreamOptions, r: Limits) -> Value {
    json!({"requested":p.requested(), "data":policy::virtual_json(s.limits),
        "runtime":{
            "geometry_bytes":s.geometry_bytes,
            "metadata_bytes":s.metadata_bytes,
            "texture_bytes":s.texture_bytes,
            "bootstrap_bytes":s.bootstrap_bytes,
            "read_slots":s.read_slots,
            "uploads_per_tick":s.uploads_per_tick,
            "max_candidates":s.max_candidates,
            "max_clusters":s.max_clusters,
            "error_pixels":s.error_pixels,
            "io_delay_ms":s.io_delay.as_millis()
        },
        "renderer":{"max_instances":r.max_instances,"max_mesh_bytes":r.max_mesh_bytes,"max_texture_bytes":r.max_texture_bytes},
        "viewer":{"max_placement_slots":policy::cap(p.size("viewer","max_placement_slots",1_000_000))}
    })
}
