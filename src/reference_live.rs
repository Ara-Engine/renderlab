//! Live reference: path tracing the engine's own scene, so every material program runs as the
//! engine runs it (docs/design-reference-live.md). Measurement only; never in the frame loop.
//!
//! Each bounce's hits are queued, every program present stores the material records of its own
//! draws' hits through `Stage::Reference` (shaders/reference_live_entry.wgsl), and one
//! program-free kernel shades the records with src/reference.rs's estimator.

/// One queued path hit, as shaders/reference_live_entry.wgsl `RfHit`: the placement and triangle
/// the ray met, its barycentrics, the ray's origin and flags (`RF_HIT`).
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RfHit {
    pub placement: u32,
    pub primitive: u32,
    pub bary: [f32; 2],
    pub origin: [f32; 3],
    pub flags: u32,
}

/// A hit's material record, as shaders/reference_live_entry.wgsl `RfRecord`: shading normal and
/// coverage, base colour and metallic, emission and roughness, world point and lighting model,
/// geometric normal (toward the ray) and the receiver's offset epsilon.
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RfRecord {
    pub normal_alpha: [f32; 4],
    pub base_metal: [f32; 4],
    pub emission_rough: [f32; 4],
    pub world_model: [f32; 4],
    pub geometric: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{Stage, compose, declares_parameters};

    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");

    /// Ara's terrain shape: the standard program with its `surface` replaced, reading parameters
    /// and sampling its atlas with explicit gradients (ara-renderlab src/splat.rs).
    fn splat_like() -> String {
        let standard = crate::STANDARD_PROGRAM;
        let start = standard.find("fn surface(input:SurfaceInput) -> Surface {").unwrap();
        let end = start + standard[start..].find("\n}\n").unwrap() + 3;
        format!(
            "struct Parameters {{ uv:array<vec4<f32>,4>, tint:array<vec4<f32>,4> }};\n{}fn surface(input:SurfaceInput) -> Surface {{\n    let c=sample_base_color_grad(input,input.uv*params.uv[0].xy,input.uv_dx,input.uv_dy);\n    let w=sample_metallic_roughness(input);\n    return Surface(material_normal(input),c.rgb*params.tint[0].rgb*w.r,0.0,0.5,1.0);\n}}\n{}",
            &standard[..start],
            &standard[end..]
        )
    }

    /// The reference stage composes and validates for every kind of program on the ray-hardware
    /// shading head: the standard program and Ara's terrain shape store `material_out`, a
    /// program without the record mapping (toon) and one with parameters but no mapping store the
    /// engine's factors. Its entry reaches no group-0 binding but its own (90-93): the reference
    /// binds the engine's arrays, materials and lighting at groups 1-3, and only its queue at 0.
    #[test]
    fn reference_stage_composes_for_every_program_kind() {
        let heads = crate::renderer::primary_ray_gpu::shading_heads(PRELUDE).unwrap();
        let emissive = "struct Parameters { glow:vec3<f32> };\nstruct Surface { n:vec3<f32>, albedo:vec3<f32> };\nfn surface(input:SurfaceInput)->Surface { return Surface(input.geometric,sample_base_color(input).rgb); }\nfn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return expose(s.albedo+params.glow); }\nfn emission(s:Surface,input:SurfaceInput)->vec3<f32> { return params.glow; }\n";
        let splat = splat_like();
        let programs = [
            ("standard", crate::STANDARD_PROGRAM, true),
            ("toon", include_str!("../shaders/programs/toon.wgsl"), false),
            ("emissive", emissive, false),
            ("splat", splat.as_str(), true),
        ];
        for (name, program, mapped) in programs {
            let composed = compose(&heads[1], program, Stage::Reference, declares_parameters(program));
            let source = &composed.source;
            assert!(source.contains("fn rf_materials("), "{name}");
            assert_eq!(source.contains("return material_out(s,input);"), mapped, "{name}");
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            let entry = module.entry_points.iter().position(|e| e.name == "rf_materials").unwrap();
            let uses = info.get_entry_point(entry);
            for (handle, global) in module.global_variables.iter() {
                if let Some(b) = global.binding.as_ref().filter(|_| !uses[handle].is_empty()) {
                    assert!(
                        b.group != 0 || (90..=93).contains(&b.binding),
                        "{name}: the reference entry reaches group 0 binding {} ({:?})",
                        b.binding,
                        global.name
                    );
                }
            }
        }
        // The Rust records match the WGSL's sizes: 32 and 80 bytes.
        assert_eq!(std::mem::size_of::<RfHit>(), 32);
        assert_eq!(std::mem::size_of::<RfRecord>(), 80);
    }
}
