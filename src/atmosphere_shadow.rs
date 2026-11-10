//! VF1's read-only view of the Virtual clipmap. Compiled as a child of its owner so
//! fog never exposes or changes the cache's resource layout or update policy.
use super::*;

impl VirtualState {
    pub(crate) fn atmosphere_buffers(&self) -> Option<[&wgpu::Buffer; 3]> {
        let r = self.resources.as_ref()?;
        Some([&r.frame, &r.top, &r.state])
    }
}

pub(crate) fn atmosphere_source() -> String {
    // Reuse the owner's frame ABI and coordinate helpers verbatim. No walks, page
    // requests, cache writes or triangle/material bindings enter the fog module.
    let definitions = include_str!("../shaders/virtual_shadow.wgsl")
        .split("// ---- Page requests")
        .next()
        .unwrap()
        .replace("@binding(0)", "@binding(21)");
    format!(
        "const PRIMARY_BATCHES:u32={}u;\n{definitions}{}",
        crate::TRACED_DRAWS,
        include_str!("../shaders/atmosphere_shadow_virtual.wgsl")
    )
}
