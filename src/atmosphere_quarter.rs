//! Rotating-quarter ray updates; no extra volume or per-cell record.
const LOOP: &str = "    for(var sample=0u;sample<samples;sample+=1u) {";

pub(super) fn source(mut source: String) -> String {
    assert_eq!(source.matches(LOOP).count(), 1);
    source = source.replacen(
        LOOP,
        &format!(
            "    let recycled=at_quarter_history(id,j,samples);\n    if recycled.x>=0.0 {{\n        textureStore(at_scatter_out,vec3<i32>(id),vec4<f32>(recycled.x,0.0,0.0,recycled.y));\n        return;\n    }}\n{LOOP}"
        ),
        1,
    );
    source.push_str(include_str!("../shaders/atmosphere_quarter.wgsl"));
    source
}

#[cfg(test)]
mod tests {
    #[test]
    fn quarter_injection_reads_existing_history_and_validates() {
        let source = super::super::kernel_source_for(true, super::super::options::Mode::Quarter);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let entry = module
            .entry_points
            .iter()
            .position(|e| e.name == "at_fog_inject")
            .unwrap();
        let uses = info.get_entry_point(entry);
        let bindings: Vec<_> = module
            .global_variables
            .iter()
            .filter_map(|(handle, global)| {
                global
                    .binding
                    .as_ref()
                    .filter(|_| !uses[handle].is_empty())
                    .map(|b| (b.group, b.binding))
            })
            .collect();
        assert!(bindings.contains(&(0, 3)));
        assert!(bindings.contains(&(0, 5)));
        assert!(bindings.contains(&(0, 20)));
    }
}
