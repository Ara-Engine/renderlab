//! Measurement-only receiver decomposition. No switch means byte-identical source.
//! Empty/decode variants retain coverage and equal stores, but intentionally omit
//! lighting. A data-dependent witness keeps only the receiver fields the pass uses
//! live. Their timing difference is an estimate, not an additive kernel breakdown.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Empty,
    Decode,
}

fn mode() -> Mode {
    match std::env::var("RENDERLAB_RECEIVER_DECODE_PROBE").as_deref() {
        Ok("empty") => Mode::Empty,
        Ok("decode") => Mode::Decode,
        _ => Mode::Off,
    }
}

const HASH: &str = "
fn rxprobe_vec(v:vec3<f32>)->u32 {let b=bitcast<vec3<u32>>(v);return b.x^(b.y*1664525u)^(b.z*1013904223u);}
fn rxprobe_receiver(r:Receiver)->u32 {return rxprobe_vec(r.world)^rxprobe_vec(r.geometric)^bitcast<u32>(r.epsilon)^r.slot^u32(r.thin);}
";

pub(super) fn gi(source: String) -> String {
    gi_mode(source, mode())
}
pub(super) fn lights(source: String) -> String {
    lights_mode(source, mode())
}
pub(super) fn sun(source: String) -> String {
    sun_mode(source, mode())
}

fn gi_mode(mut source: String, mode: Mode) -> String {
    if mode == Mode::Off {
        return source;
    }
    let start = source.rfind("    let hit=primary_decode(record);").unwrap();
    let prepare = source[start..].find("    gi_prepare(input);").unwrap() + start;
    let decode = source[start..prepare].to_owned();
    source.truncate(start);
    if mode == Mode::Decode {
        source.push_str(&decode);
    }
    source.push_str("    var witness=0u;\n");
    if mode == Mode::Decode {
        source.push_str("    witness=rxprobe_vec(input.receiver.world)^rxprobe_vec(input.receiver.geometric)^rxprobe_vec(input.normal);\n");
    }
    source.push_str("    let at=id.y*size.x+id.x;\n    gi_prepared[at].r0=vec4<u32>(bitcast<u32>(-1.0),0u,select(0u,1u,witness==0xdeadbeefu),0u);\n    gi_prepared[at].r1=vec4<u32>(0u);\n    gi_prepared[at].r2=vec4<u32>(0u);\n}\n");
    source.push_str(HASH);
    source
}

fn lights_mode(mut source: String, mode: Mode) -> String {
    if mode == Mode::Off {
        return source;
    }
    let pixel = source.find("fn rl_lvis_pixel(").unwrap();
    let start = source[pixel..]
        .find("    let hit=primary_decode(record);")
        .unwrap()
        + pixel;
    let store = source[start..]
        .find("    rl_lvis_store(surface_input);")
        .unwrap()
        + start;
    let decode = source[start..store].to_owned();
    source.truncate(start);
    if mode == Mode::Decode {
        source.push_str(&decode);
    }
    source.push_str("    var witness=0u;\n");
    if mode == Mode::Decode {
        source.push_str("    witness=rxprobe_vec(surface_input.world)^rxprobe_receiver(surface_input.receiver);\n");
    }
    source.push_str("    let base=(id.y*size.x+id.x)*4u;\n    if base+3u>=arrayLength(&rl_lvis) {return;}\n    rl_lvis[base]=select(0xffffffffu,0xfffffffeu,witness==0xdeadbeefu);\n    rl_lvis[base+1u]=0xffffffffu;rl_lvis[base+2u]=0xffffffffu;rl_lvis[base+3u]=0xffffffffu;\n}\n");
    source.push_str(HASH);
    source
}

fn sun_mode(mut source: String, mode: Mode) -> String {
    if mode == Mode::Off {
        return source;
    }
    let start = source
        .rfind("            let world=textureLoad(fresh_world,p,0);")
        .unwrap();
    let end = source[start..]
        .find("visible=primary_sun(receiver,slot,id.xy,size.x);")
        .unwrap()
        + start;
    let decode = source[start..end].to_owned();
    let replacement = if mode == Mode::Decode {
        format!("{decode}visible=select(256u,255u,rxprobe_receiver(receiver)==0xdeadbeefu);")
    } else {
        "            visible=256u;".to_owned()
    };
    source.replace_range(
        start..end + "visible=primary_sun(receiver,slot,id.xy,size.x);".len(),
        &replacement,
    );
    source.push_str(HASH);
    source
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{program, renderer::primary_ray_gpu as primary};
    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    }
    #[test]
    fn receiver_decode_probes_validate_both_paths_and_off_identity() {
        let scene = include_str!("../shaders/scene.wgsl");
        for heads in [
            primary::shading_heads(scene).unwrap(),
            primary::non_rt_sources(scene).unwrap().1,
        ] {
            for head in heads {
                let gi = primary::gi_receiver_pass::kernel_module(&head).source;
                let lights = primary::light_vis_pass::apply_enabled(
                    program::compose(
                        &head,
                        program::STANDARD_PROGRAM,
                        program::Stage::Traced,
                        false,
                    ),
                    true,
                )
                .source;
                assert_eq!(gi_mode(gi.clone(), Mode::Off), gi);
                assert_eq!(lights_mode(lights.clone(), Mode::Off), lights);
                for mode in [Mode::Empty, Mode::Decode] {
                    validate(&gi_mode(gi.clone(), mode));
                    validate(&lights_mode(lights.clone(), mode));
                }
            }
        }
        let kernels = primary::shader_sources(scene).unwrap();
        let sun = primary::temporal_reconstruction::vb_sun_source(&kernels[1]).unwrap();
        assert_eq!(sun_mode(sun.clone(), Mode::Off), sun);
        for mode in [Mode::Empty, Mode::Decode] {
            validate(&sun_mode(sun.clone(), mode));
        }
    }
}
