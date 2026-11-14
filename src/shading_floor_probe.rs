//! Diagnostic shading-kernel ablations. Never applied to visibility, GI updates,
//! light-vis, raster/transparent shading or temporal reconstruction. Probe runs
//! explicitly disable lean shading so its background compiler cannot replace these.
use crate::program::Composed;

const MODES: [&str; 10] = [
    "material",
    "sun",
    "locals",
    "gi",
    "reflections",
    "emission",
    "fog",
    "floor",
    "no-lights",
    "loop-only",
];

pub(super) fn compose(composed: Composed) -> Composed {
    let mode = std::env::var("RENDERLAB_SHADING_FLOOR_PROBE").unwrap_or_default();
    let composed = variant(composed, &mode);
    crate::renderer::primary_ray_gpu::light_counter_gpu::compose(composed)
}

pub(crate) fn body(source: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let at = source
        .match_indices(&format!("fn {name}("))
        .find(|(at, _)| source[..*at].rsplit('\n').next().unwrap().trim().is_empty())?
        .0;
    let start = source[at..].find('{')? + at + 1;
    Some(block(source, start))
}

pub(crate) fn block(source: &str, start: usize) -> std::ops::Range<usize> {
    let mut depth = 1;
    for (i, b) in source.as_bytes()[start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return start..start + i;
        }
    }
    panic!("unterminated probe block")
}

fn stub(source: &mut String, name: &str, replacement: &str) {
    if let Some(range) = body(source, name) {
        source.replace_range(range, replacement);
    }
}

fn variant(mut composed: Composed, mode: &str) -> Composed {
    if !MODES.contains(&mode) {
        return composed;
    }
    // Keep the diagnostic source map accurate even when a stub changes API text
    // before the user program, or changes the program's own surface/shade body.
    const BEGIN: &str = "// shading-floor program begin\n";
    const END: &str = "\n// shading-floor program end\n";
    composed.source.insert_str(composed.program.end, END);
    composed.source.insert_str(composed.program.start, BEGIN);
    let source = &mut composed.source;
    match mode {
        "material" => {
            let start = source.find("struct Surface {").unwrap();
            let end = source[start..].find('}').unwrap() + start;
            let fields = source[start..end].to_owned();
            let mut replacement="var s:Surface;s.n=unit_normal(input.normal);s.base=clamp(input.material.base_color.rgb*input.color.rgb,vec3<f32>(0.0),vec3<f32>(1.0));".to_owned();
            for (field, value) in [
                ("metal", "input.material.metallic"),
                ("rough", "input.material.roughness"),
                ("alpha", "input.material.base_color.a"),
            ] {
                if fields.contains(&format!("{field}:")) {
                    replacement.push_str(&format!("s.{field}={value};"));
                }
            }
            replacement.push_str("return s;");
            stub(source, "surface", &replacement);
            stub(source, "decal_prepare", "return input;");
        }
        "sun" | "locals" => {
            let seam = "for(var i=0u;i<light_count();i+=1u) {";
            assert!(source.contains(seam), "light loop probe seam");
            let compare = if mode == "sun" { "==" } else { "!=" };
            *source = source.replace(
                seam,
                &format!("{seam}\n if lights[rl_light_index(i)].kind.x{compare}0u {{continue;}}"),
            );
        }
        "no-lights" => {
            stub(source, "light_count", "return 0u;");
        }
        "loop-only" => {
            let shade = body(source, "shade").unwrap();
            let seam = "for(var i=0u;i<light_count();i+=1u) {";
            let start = source[shade.clone()].find(seam).unwrap() + shade.start + seam.len();
            let range = block(source, start);
            source.replace_range(range,"\nlet sample=light_sample_untinted(i,input);\nfloor_loop_witness=floor_mix(floor_loop_witness,sample.index);\nfloor_loop_witness=floor_mix(floor_loop_witness,floor_hash3(sample.direction));\nfloor_loop_witness=floor_mix(floor_loop_witness,bitcast<u32>(sample.attenuation));\nfloor_loop_witness=floor_mix(floor_loop_witness,floor_hash3(sample.intensity));\n");
            let hint = "tr1_view_hint(surface_output,surface_input,view_ray)";
            assert!(source.contains(hint), "loop witness output seam");
            *source = source.replace(
                hint,
                &format!("select({hint},0.0,floor_loop_witness==0xdeadbeefu)"),
            );
            source.push_str("\nvar<private> floor_loop_witness:u32;\n");
            source.push_str(HASH);
        }
        "gi" => {
            stub(
                source,
                "gi_prepare",
                "gi_rx_valid=false;for(var i=0u;i<9u;i+=1u){gi_rx_sky[i]=vec3<f32>(0.0);}for(var i=0u;i<4u;i+=1u){gi_rx_bounce[i]=vec3<f32>(0.0);}",
            );
        }
        "reflections" => {
            stub(
                source,
                "rl_reflection_prepare",
                "rl_specular=vec4<f32>(0.0);",
            );
            stub(source, "specular_incident", "return vec4<f32>(0.0);");
            // Sky specular and traced reflection are one term; toon has neither.
            if let Some(shade) = body(source, "shade") {
                if let Some(start) = source[shade.clone()].find("    let reflected=reflect(-v,n);")
                {
                    let start = start + shade.start;
                    let end = source[start..shade.end]
                        .find("    var exposed=min(diffuse+specular,")
                        .unwrap()
                        + start;
                    source.replace_range(start..end, "    let specular=vec3<f32>(0.0);\n");
                }
            }
        }
        "emission" => {
            *source = source.replace("expose(max(params.glow,vec3<f32>(0.0)))", "vec3<f32>(0.0)");
        }
        "fog" => {
            stub(source, "atmosphere_surface", "return c;");
        }
        "floor" => {
            stub(source, "decal_prepare", "return input;");
            stub(source, "primary_miss", "return clear;");
            let pixel = body(source, "primary_shade_pixel").unwrap();
            let end = source[pixel.clone()].find("decal_none()));").unwrap()
                + pixel.start
                + "decal_none()));".len();
            source.replace_range(end..pixel.end, FLOOR);
            source.push_str(HASH);
        }
        _ => unreachable!(),
    }
    composed.program = source.find(BEGIN).unwrap() + BEGIN.len()..source.find(END).unwrap();
    composed
}

const HASH: &str = "\nfn floor_hash3(v:vec3<f32>)->u32 {let b=bitcast<vec3<u32>>(v);return ((b.x*16777619u)^b.y)*16777619u^b.z;}\nfn floor_mix(a:u32,b:u32)->u32 {return a*16777619u^b;}\n";
// All reconstructed vertex attributes and footprints remain live. The witness
// is extra integer work, so the floor is a retained-decode diagnostic, not an
// empty dispatch or a guaranteed minimum attainable by a different kernel.
const FLOOR:&str="
    var h=floor_hash3(surface_input.world);
    h=floor_mix(h,floor_hash3(surface_input.world_dx));h=floor_mix(h,floor_hash3(surface_input.world_dy));
    h=floor_mix(h,floor_hash3(surface_input.geometric));h=floor_mix(h,floor_hash3(surface_input.normal));
    h=floor_mix(h,floor_hash3(surface_input.normal_dx));h=floor_mix(h,floor_hash3(surface_input.normal_dy));
    h=floor_mix(h,floor_hash3(surface_input.tangent));h=floor_mix(h,bitcast<u32>(surface_input.handedness));
    h=floor_mix(h,floor_hash3(vec3<f32>(surface_input.uv,0.0)));h=floor_mix(h,floor_hash3(vec3<f32>(surface_input.uv_dx,0.0)));h=floor_mix(h,floor_hash3(vec3<f32>(surface_input.uv_dy,0.0)));
    h=floor_mix(h,floor_hash3(surface_input.color.xyz));h=floor_mix(h,bitcast<u32>(surface_input.color.w));
    h=floor_mix(h,u32(surface_input.source_front));h=floor_mix(h,floor_hash3(surface_input.receiver.world));
    h=floor_mix(h,floor_hash3(surface_input.receiver.geometric));h=floor_mix(h,bitcast<u32>(surface_input.receiver.epsilon));h=floor_mix(h,surface_input.receiver.slot);h=floor_mix(h,u32(surface_input.receiver.thin));
    textureStore(primary_hdr,pixel,vec4<f32>(vec3<f32>(select(0.18,0.19,h==0xdeadbeefu)),1.0));
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{program, renderer::primary_ray_gpu as primary};
    #[test]
    fn shading_floor_variants_validate_both_paths() {
        let scene = include_str!("../shaders/scene.wgsl");
        let mut count = 0;
        for heads in [
            primary::shading_heads(scene).unwrap(),
            primary::non_rt_sources(scene).unwrap().1,
        ] {
            let head = &heads[1];
            for text in [
                program::STANDARD_PROGRAM,
                include_str!("../examples/game_scene.wgsl"),
                include_str!("../examples/game_toon.wgsl"),
            ] {
                for list in [false, true] {
                    let compose = || {
                        if list {
                            program::compose_list(
                                head,
                                text,
                                program::Stage::Traced,
                                program::declares_parameters(text),
                            )
                        } else {
                            program::compose(
                                head,
                                text,
                                program::Stage::Traced,
                                program::declares_parameters(text),
                            )
                        }
                    };
                    let original = compose();
                    let off = variant(compose(), "full");
                    assert_eq!(off.source, original.source);
                    assert_eq!(off.program, original.program);
                    for mode in MODES {
                        let composed = variant(compose(), mode);
                        let module =
                            naga::front::wgsl::parse_str(&composed.source).unwrap_or_else(|e| {
                                panic!("{mode}: {}", e.emit_to_string(&composed.source))
                            });
                        naga::valid::Validator::new(
                            naga::valid::ValidationFlags::all(),
                            naga::valid::Capabilities::all(),
                        )
                        .validate(&module)
                        .unwrap_or_else(|e| {
                            panic!("{mode}: {}", e.emit_to_string(&composed.source))
                        });
                        count += 1;
                    }
                }
            }
        }
        assert_eq!(count, MODES.len() * 12);
    }
}
