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
    // The sparse clipmap (vsm_sparse): the page state's epochs sit at the table index, and the
    // top word in the entry's physical page, which must hold this absolute page (its tag).
    let access = include_str!("../shaders/atmosphere_shadow_virtual.wgsl");
    let sparse = access
        .replace(
            "let slot=(level*pages+page.y)*pages+page.x;",
            "let slot=vsm_vpage(level,page);",
        )
        .replace(
            "let word=at_shadow_top[(level*side+cell.y)*side+cell.x];",
            "var index=(level*side+cell.y)*side+cell.x;\n        \
             if vsm_sparse() {\n            \
             let physical=at_shadow_pages[vsm_section(2u)+slot];\n            \
             if physical==VSM_NONE || at_shadow_pages[vsm_section(1u)+slot]!=vsm_tag(level,page) { continue; }\n            \
             index=physical*VSM_PAGE*VSM_PAGE+(cell.y%VSM_PAGE)*VSM_PAGE+cell.x%VSM_PAGE;\n        \
             }\n        \
             let word=at_shadow_top[index];",
        );
    assert!(
        sparse.contains("vsm_sparse()") && sparse.contains("vsm_vpage(level,page)"),
        "atmosphere_shadow_virtual.wgsl changed: update the sparse clipmap accessor"
    );
    format!(
        "const PRIMARY_BATCHES:u32={}u;\n{definitions}{sparse}",
        crate::TRACED_DRAWS,
    )
}
