//! VF1's read-only view of the Virtual clipmap. Compiled as a child of its owner so
//! fog never exposes or changes the cache's resource layout or update policy.
use super::*;

impl VirtualState {
    /// Whether this state is the RT fog map (`FogMap`), the fog's only reader.
    pub(crate) fn fog_map(&self) -> bool {
        self.fog
    }
    pub(crate) fn atmosphere_buffers(&self) -> Option<[&wgpu::Buffer; 3]> {
        let r = self.resources.as_ref()?;
        Some([&r.frame, &r.top, &r.state])
    }
}

pub(crate) fn atmosphere_source() -> String {
    atmosphere_source_at(21, None)
}

/// The accessor with its three buffers at bindings `first..first + 3` (the RT fog module's
/// cull holds 21 and 22). `fallback` answers the points no drawn page covers (the RT fog map
/// traces them, e.g. pages a cut's budget has not drawn yet); without one they are lit.
pub(crate) fn atmosphere_source_at(first: u32, fallback: Option<&str>) -> String {
    // Reuse the owner's frame ABI and coordinate helpers verbatim. No walks, page
    // requests, cache writes or triangle/material bindings enter the fog module.
    let definitions = include_str!("../shaders/virtual_shadow.wgsl")
        .split("// ---- Page requests")
        .next()
        .unwrap()
        .replace("@binding(0)", &format!("@binding({first})"));
    // The sparse clipmap (vsm_sparse): the page state's epochs sit at the table index, and the
    // top word in the entry's physical page, which must hold this absolute page (its tag).
    let mut access = include_str!("../shaders/atmosphere_shadow_virtual.wgsl").to_owned();
    for (from, to) in [(22, first + 1), (23, first + 2)] {
        let pattern = format!("@binding({from})");
        assert_eq!(access.matches(&pattern).count(), 1, "accessor binding {from} moved");
        access = access.replacen(&pattern, &format!("@binding({to})"), 1);
    }
    if let Some(fallback) = fallback {
        // The RT fog map's texels are its 32-bit heights (fog_shader_source), 0 empty.
        for (from, to) in [
            ("at_shadow_top:array<vec2<u32>>", "at_shadow_top:array<u32>"),
            ("if word.x==0u { return 1.0; }", "if word==0u { return 1.0; }"),
            ("bitcast<f32>(word.y)", "bitcast<f32>(word)"),
        ] {
            assert_eq!(access.matches(from).count(), 1, "accessor changed: {from}");
            access = access.replacen(from, to, 1);
        }
        let tail = "    }
    return 1.0;
}";
        assert_eq!(access.matches(tail).count(), 1, "accessor tail changed");
        access = access.replacen(tail, &format!("    }}
    return {fallback};
}}"), 1);
    }
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

#[cfg(test)]
mod tests {
    /// Both fog injections that read the clipmap (without ray hardware, and the RT fog map
    /// with its cull and traced fallback) parse and validate.
    #[test]
    fn clipmap_fog_injections_validate() {
        // The RT fog map's own clipmap module.
        let source = super::fog_shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("fog clipmap: {e:?}"));
        for hardware in [false, true] {
            let sources = super::super::super::atmosphere_gpu::compile_structure_sources(hardware);
            let (_, source) = sources.iter().find(|(name, _)| *name == "virtual-injection").unwrap();
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{hardware}: {e:?}"));
        }
    }
}
