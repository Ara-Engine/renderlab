//! Per-program/local-light ranking counters. No rejection gates and no HDR counters.
use super::super::program_gpu::{probe_block, probe_function_body as body};
use super::*;
pub(crate) const BINDING: u32 = 80;
const FIELDS: usize = 20;
const WORDS: usize = 4 + 4 * 32 * 2 * FIELDS;
const BYTES: u64 = WORDS as u64 * 4;
const START: u64 = 40;
const REPEATS: u64 = 8;
const API: &str = include_str!("../shaders/light_counter.wgsl");
pub(crate) fn on() -> bool {
    std::env::var_os("RENDERLAB_LIGHT_COUNTER").is_some()
}
fn lights() -> u32 {
    std::env::var("RENDERLAB_LIGHT_COUNTER_LIGHTS")
        .unwrap_or("3".into())
        .parse::<u32>()
        .unwrap()
        .clamp(1, 31)
}
pub(super) struct Targets {
    pub values: Buffer,
}
pub(super) fn entries(
    mut entries: Vec<wgpu::BindGroupLayoutEntry>,
) -> Vec<wgpu::BindGroupLayoutEntry> {
    if on() {
        entries.push(buffer(
            BINDING,
            wgpu::BufferBindingType::Storage { read_only: false },
            BYTES,
        ));
    }
    entries
}
pub(crate) fn compose(composed: program::Composed) -> program::Composed {
    variant(composed, on())
}
fn variant(mut composed: program::Composed, enabled: bool) -> program::Composed {
    if !enabled {
        return composed;
    }
    const BEGIN: &str = "// light-counter program begin\n";
    const END: &str = "\n// light-counter program end\n";
    let text = &composed.source[composed.program.clone()];
    let id = if text.contains("params.shadow_tint") {
        2
    } else if text.contains("params.glow") {
        1
    } else if text.contains("const SPECULAR_AA:") {
        0
    } else {
        3
    };
    composed.source.insert_str(composed.program.end, END);
    composed.source.insert_str(composed.program.start, BEGIN);
    let source = &mut composed.source;
    let shade = body(source, "shade").unwrap();
    let mut code = source[shade.clone()].to_owned();
    let seam = "for(var i=0u;i<light_count();i+=1u) {";
    let at = code.find(seam).expect("counter needs fixture light loop");
    let loop_start = at + seam.len();
    let loop_range = probe_block(&code, loop_start);
    let mut light_loop = code[loop_range.clone()].to_owned();
    for sample in [
        "let light=light_sample(i,input);",
        "let light=light_sample_untinted(i,input);",
    ] {
        light_loop =
            light_loop.replace(sample, &format!("{sample}\n lc_capture(input,s.n,light);"));
    }
    let assignment = light_loop
        .find("exposed=min(exposed+")
        .or_else(|| light_loop.find("exposed+="))
        .unwrap();
    let end = light_loop[assignment..].find(';').unwrap() + assignment + 1;
    let statement = light_loop[assignment..end].to_owned();
    let mirror =
        statement
            .replacen("exposed", "lc_without", 1)
            .replacen("exposed+", "lc_without+", 1);
    light_loop.insert_str(
        end,
        &format!("\nif lc_primary && light.index!=lc_counter.header.y {{{mirror}}}\n"),
    );
    code.replace_range(loop_range, &light_loop);
    code.insert_str(at, "var lc_without=exposed;\n");
    code = code.replace(
        "if film { exposed*=s.alpha; }",
        "if film { exposed*=s.alpha;lc_without*=s.alpha; }",
    );
    let ret = code.rfind("return ").unwrap();
    let end = code[ret..].find(';').unwrap() + ret;
    let expression = code[ret + 7..end].replace("exposed", "lc_without");
    code.insert_str(
        ret,
        &format!("if lc_primary {{lc_without_rgb={expression};}}\n"),
    );
    source.replace_range(shade, &code);
    let visibility = body(source, "light_visibility").unwrap();
    source.insert_str(
        visibility.start,
        "\n lc_current=lc_primary && s.index==lc_counter.header.y;if lc_current {lc_calls+=1u;}\n",
    );
    if let Some(ray) = body(source, "rl_local_rays_visibility") {
        let code = source[ray.clone()].replace(
            "rayQueryInitialize(",
            "if lc_current {lc_rays+=1u;}rayQueryInitialize(",
        );
        source.replace_range(ray, &code);
    }
    let entry = "rl_finite(shade(surface_output,surface_input))";
    assert_eq!(source.matches(entry).count(), 1);
    *source = source.replacen(
        entry,
        "rl_finite(lc_primary_shade(surface_output,surface_input))",
        1,
    );
    source.push_str(&format!("\nconst LC_COUNTER_PROGRAM:u32={id}u;\n"));
    source.push_str(API);
    composed.program = source.find(BEGIN).unwrap() + BEGIN.len()..source.find(END).unwrap();
    composed
}
impl Renderer {
    pub(super) fn light_counter_targets(&self) -> Targets {
        Targets {
            values: Buffer::new(
                self,
                "per-light counters",
                BYTES,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            ),
        }
    }
    pub(crate) fn prepare_light_counter(&self) {
        let Some(target) = self
            .view
            .primary
            .as_ref()
            .and_then(|s| s.targets.as_ref())
            .and_then(|t| t.light_counter.as_ref())
        else {
            return;
        };
        let n = lights();
        let active = self.frames >= START && self.frames < START + u64::from(n) * REPEATS;
        let selected = if active {
            ((self.frames - START) / REPEATS) as u32 + 1
        } else {
            0
        };
        let words = [u32::from(active), selected, self.frames as u32, n];
        let bytes: Vec<u8> = words.iter().flat_map(|v| v.to_ne_bytes()).collect();
        self.queue.write_buffer(&target.values.value, 0, &bytes);
    }
    pub(crate) fn encode_light_counter_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(path) = std::env::var_os("RENDERLAB_LIGHT_COUNTER") else {
            return;
        };
        let n = lights();
        if self.frames != START + u64::from(n) * REPEATS + 1 {
            return;
        }
        let Some(target) = self
            .view
            .primary
            .as_ref()
            .and_then(|s| s.targets.as_ref())
            .and_then(|t| t.light_counter.as_ref())
        else {
            return;
        };
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light counter readback"),
            size: BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&target.values.value, 0, &readback, 0, BYTES);
        let mapped = readback.clone();
        encoder.map_buffer_on_submit(&readback,wgpu::MapMode::Read,..,move|result|{
            if result.is_err(){return;}
            let Ok(data)=mapped.get_mapped_range(..)else{return};
            let words:Vec<u32>=data.chunks_exact(4).map(|b|u32::from_ne_bytes(b.try_into().unwrap())).collect();
            let names=["geometry_pixels","evaluated_pixels","range_cone_zero","source_nonfacing","view_nonfacing_after_source","facing_pass","thin_two_sided_disagreement","light_visibility_calls","ray_initializations","traced_under_1_code","traced_at_least_1_code","visibility_called_without_query_pixels","source_nonfacing_traced","view_nonfacing_traced","view_nonfacing_any","thin_two_sided_evaluated","reserved","evaluated_under_1_code","evaluated_at_least_1_code","max_codes_bits"];
            let mut rows=Vec::new();
            for (program,label) in ["standard","game","toon","other"].iter().enumerate(){for light in 1..=n as usize{
                let at=4+(program*32+light)*2*FIELDS;
                if words[at]==0{continue;}
                let fields=names.iter().enumerate().map(|(i,name)|format!("\"{name}\":{}",words[at+i])).collect::<Vec<_>>().join(",");
                let max=f32::from_bits(words[at+19]);
                let max=if max.is_finite(){max.to_string()}else{"null".into()};
                rows.push(format!("{{\"program\":\"{label}\",\"light_index\":{light},\"stage\":\"inline\",{fields},\"max_display_codes\":{max}}}"));
            }}
            let pose=std::env::var("RENDERLAB_LIGHT_COUNTER_POSE").ok().and_then(|s|s.parse::<u32>().ok()).map_or("null".into(),|v|v.to_string());
            let end=START+u64::from(n)*REPEATS-1;
            let json=format!("{{\"proxy\":\"pre-TR1 display-code proxy\",\"repeats_per_light\":{REPEATS},\"render_frames\":[{START},{end}],\"pose_frame\":{pose},\"source_order\":\"range/cone, unflipped geometric source, then camera-oriented shading view among source pass\",\"facing_is_diagnostic\":true,\"primary_only\":true,\"producer_supported\":false,\"rows\":[{}]}}",rows.join(","));
            let _=std::fs::write(path,json);drop(data);mapped.unmap();
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_counter_composes_primary_programs() {
        let scene = include_str!("../shaders/scene.wgsl");
        let mut count = 0;
        for heads in [
            shading_heads(scene).unwrap(),
            non_rt_sources(scene).unwrap().1,
        ] {
            for text in [
                program::STANDARD_PROGRAM,
                include_str!("../examples/game_scene.wgsl"),
                include_str!("../examples/game_toon.wgsl"),
            ] {
                for list in [false, true] {
                    let original = if list {
                        program::compose_list(
                            &heads[1],
                            text,
                            program::Stage::Traced,
                            program::declares_parameters(text),
                        )
                    } else {
                        program::compose(
                            &heads[1],
                            text,
                            program::Stage::Traced,
                            program::declares_parameters(text),
                        )
                    };
                    let off = variant(
                        program::Composed {
                            source: original.source.clone(),
                            program: original.program.clone(),
                        },
                        false,
                    );
                    assert_eq!(off.source, original.source);
                    let c = variant(original, true);
                    let m = naga::front::wgsl::parse_str(&c.source)
                        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&c.source)));
                    naga::valid::Validator::new(
                        naga::valid::ValidationFlags::all(),
                        naga::valid::Capabilities::all(),
                    )
                    .validate(&m)
                    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&c.source)));
                    count += 1;
                }
            }
        }
        assert_eq!(count, 12);
    }
}
