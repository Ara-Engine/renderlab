//! Per-program/local-light ranking counters. No rejection gates and no HDR counters.
use super::super::program_gpu::{probe_block, probe_function_body as body};
use super::*;
pub(crate) const BINDING: u32 = 80;
pub(crate) const RECORD_BINDING: u32 = 81;
const FIELDS: usize = 96;
const STAGES: usize = 4;
const WORDS: usize = 4 + 4 * 32 * STAGES * FIELDS;
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
    pub records: Buffer,
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
        entries.push(buffer(
            RECORD_BINDING,
            wgpu::BufferBindingType::Storage { read_only: false },
            16,
        ));
    }
    entries
}
pub(crate) fn compose(composed: program::Composed) -> program::Composed {
    variant(composed, on(), super::light_vis_pass::on())
}
fn variant(mut composed: program::Composed, enabled: bool, lvis: bool) -> program::Composed {
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
    let unshadowed = if id == 2 {
        "if lc_primary && light.index==lc_counter.header.y {let lc_tint=mix(clamp(params.shadow_tint,vec3<f32>(0.0),vec3<f32>(1.0)),vec3<f32>(1.0),band);lc_unshadowed_sum+=exposed_product(light.intensity,s.base*lc_tint*(light.attenuation/PI));}\n".to_owned()
    } else {
        let filtered = if id == 0 {
            "if SPECULAR_AA {lc_response=specular_brdf(input,light.direction,base,metal,rough);}"
        } else {
            ""
        };
        format!(
            "if lc_primary && light.index==lc_counter.header.y {{var lc_response=brdf(n,v,light.direction,base,metal,rough);{filtered}lc_unshadowed_sum=min(lc_unshadowed_sum+exposed_product(light.intensity*light_transmission(light,input),lc_response*light.attenuation),vec3<f32>(65504.0));}}\n"
        )
    };
    if id == 2 {
        let at = light_loop
            .find("let visibility=light_visibility(input,light);")
            .unwrap();
        light_loop.insert_str(at, &unshadowed);
    } else {
        let seam = "lc_capture(input,s.n,light);";
        light_loop = light_loop.replacen(seam, &format!("{seam}\n{unshadowed}"), 1);
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
    let mirror_u = statement
        .replacen("exposed", "lc_unshadowed_sum", 1)
        .replacen("exposed+", "lc_unshadowed_sum+", 1);
    light_loop.insert_str(
        end,
        &format!("\nif lc_primary && light.index!=lc_counter.header.y {{{mirror}{mirror_u}}}\n"),
    );
    code.replace_range(loop_range, &light_loop);
    // Simulate the proposed receiver-wide budget only at the real caller seam,
    // after its existing range/facing gates. It never changes visibility.
    if id == 0 {
        code=code.replace("let visibility=light_visibility_within(input,light,", "lc_cutoff(light,exposed_product(light.intensity*light_transmission(light,input),unshadowed*light.attenuation));\nlet visibility=light_visibility_within(input,light,");
    } else if id == 1 {
        code=code.replace("let attenuation=light.attenuation*light_visibility(input,light);", "lc_cutoff(light,exposed_product(light.intensity,brdf(n,v,light.direction,base,metal,rough)*light.attenuation));\nlet attenuation=light.attenuation*light_visibility(input,light);");
    }
    code.insert_str(
        at,
        "var lc_without=exposed;var lc_unshadowed_sum=exposed;\n",
    );
    code = code.replace(
        "if film { exposed*=s.alpha; }",
        "if film { exposed*=s.alpha;lc_without*=s.alpha;lc_unshadowed_sum*=s.alpha; }",
    );
    let ret = code.rfind("return ").unwrap();
    let end = code[ret..].find(';').unwrap() + ret;
    let expression = code[ret + 7..end].replace("exposed", "lc_without");
    let expression_u = code[ret + 7..end].replace("exposed", "lc_unshadowed_sum");
    code.insert_str(
        ret,
        &format!(
            "if lc_primary {{lc_without_rgb={expression};lc_unshadowed_rgb={expression_u};}}\n"
        ),
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
    source.push_str(&format!(
        "\nconst LC_COUNTER_PROGRAM:u32={id}u;\nconst LC_COUNTER_LVIS:bool={lvis};\n"
    ));
    source.push_str(API);
    composed.program = source.find(BEGIN).unwrap() + BEGIN.len()..source.find(END).unwrap();
    composed
}
pub(super) fn producer(composed: program::Composed) -> program::Composed {
    producer_variant(composed, on())
}
fn producer_variant(mut composed: program::Composed, enabled: bool) -> program::Composed {
    if !enabled {
        return composed;
    }
    let marker = "// ---- program ----\n";
    let before = composed.source.find(marker).unwrap();
    let source = &mut composed.source;
    let store = body(source, "rl_lvis_store").unwrap();
    let code=source[store.clone()]
        .replace("let visibility=light_visibility(input,sample);", "lc_pass_begin(index);let visibility=light_visibility(input,sample);lc_pass_store(input,sample,slot);")
        .replace("rl_lvis[base+(slot>>2u)]=packed;", "lc_pass_word();rl_lvis[base+(slot>>2u)]=packed;");
    source.replace_range(store, &code);
    let visibility = body(source, "light_visibility").unwrap();
    source.insert_str(
        visibility.start,
        "\nif lc_pass_current {lc_pass_calls+=1u;}\n",
    );
    if let Some(ray) = body(source, "rl_local_rays_visibility") {
        let code = source[ray.clone()].replace(
            "rayQueryInitialize(",
            "if lc_pass_current {lc_pass_rays+=1u;}rayQueryInitialize(",
        );
        source.replace_range(ray, &code);
    }
    let tiles = body(source, "light_vis_tiles").unwrap();
    let code=source[tiles.clone()].replace(
        "if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u { return; }",
        "lc_pass_add(90u,1u);if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {lc_pass_add(91u,1u);return;}");
    source.replace_range(tiles, &code);
    let pixel = body(source, "rl_lvis_pixel").unwrap();
    let code = source[pixel.clone()].replace(
        "if record.x==0u {return;}",
        "if record.x==0u {lc_pass_add(92u,1u);return;}lc_pass_add(93u,1u);",
    );
    source.replace_range(pixel, &code);
    let delta = source.find(marker).unwrap() - before;
    composed.program = (composed.program.start + delta)..(composed.program.end + delta);
    source.push_str(include_str!("../shaders/light_counter_producer.wgsl"));
    composed
}
impl Renderer {
    pub(super) fn light_counter_targets(&self, size: wgpu::Extent3d) -> Targets {
        Targets {
            values: Buffer::new(
                self,
                "per-light counters",
                BYTES,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            ),
            records: Buffer::new(
                self,
                "per-light producer join",
                u64::from(size.width) * u64::from(size.height) * 16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
        }
    }
    pub(crate) fn prepare_light_counter(&self, encoder: &mut wgpu::CommandEncoder) {
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
        if active && super::light_vis_pass::on() {
            encoder.clear_buffer(&target.records.value, 0, None);
        }
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
            let names=["geometry_pixels","evaluated_pixels","range_cone_zero","source_nonfacing","view_nonfacing_after_source","facing_pass","thin_two_sided_disagreement","light_visibility_calls","ray_initializations","traced_under_1_code","traced_at_least_1_code","visibility_called_without_query_pixels","source_nonfacing_traced","view_nonfacing_traced","view_nonfacing_any","thin_two_sided_evaluated","ray_initializations_under_1_code","evaluated_under_1_code","evaluated_at_least_1_code","max_codes_bits","source_nonfacing_ray_initializations","view_nonfacing_ray_initializations","thin_two_sided_disagreement_ray_initializations","thin_two_sided_disagreement_ray_initializations_at_least_1_code","evaluated_unshadowed_under_1_code","ray_initializations_unshadowed_under_1_code","traced_unshadowed_under_1_code","max_unshadowed_codes_bits","evaluated_unshadowed_under_half_code","ray_initializations_unshadowed_under_half_code","traced_unshadowed_under_half_code","evaluated_unshadowed_under_quarter_code","ray_initializations_unshadowed_under_quarter_code","traced_unshadowed_under_quarter_code"];
            let mut rows=Vec::new();
            for (program,label) in ["standard","game","toon","other"].iter().enumerate(){for light in 1..=n as usize{for (stage,stage_label) in ["inline","inline_cached_population","lvis_producer"].iter().enumerate(){
                let at=4+((program*32+light)*STAGES+stage)*FIELDS;
                if words[at]==0{continue;}
                let fields=names.iter().enumerate().map(|(i,name)|format!("\"{name}\":{}",words[at+i])).collect::<Vec<_>>().join(",");
                let max=f32::from_bits(words[at+19]);
                let max=if max.is_finite(){max.to_string()}else{"null".into()};
                let upper=f32::from_bits(words[at+27]);let upper=if upper.is_finite(){upper.to_string()}else{"null".into()};
                let histogram=|offset:usize,bins:usize|{(0..bins).map(|i|{let a=at+offset+i*4;format!("[{},{},{},{}]",words[a],words[a+1],words[a+2],words[a+3])}).collect::<Vec<_>>().join(",")};
                let distance=histogram(34,7);let normal=histogram(62,6);let violations=words[at+86];
                let extra=[(87,"eligibility_mismatch_pixels"),(88,"unjoined_producer_ray_initializations"),(89,"packed_values"),(90,"packing_word_owner_pixels"),(91,"range_mismatch_pixels"),(92,"smooth_budget_skip_pixels"),(93,"smooth_budget_skip_ray_initializations"),(94,"actual_shadowed_under_half_code_ray_initializations"),(95,"smooth_budget_invalid_endpoint_pixels")].iter().map(|(i,name)|format!("\"{name}\":{}",words[at+i])).collect::<Vec<_>>().join(",");
                rows.push(format!("{{\"program\":\"{label}\",\"light_index\":{light},\"stage\":\"{stage_label}\",{fields},{extra},\"max_display_codes\":{max},\"max_unshadowed_codes\":{upper},\"distance_range_histogram\":[{distance}],\"ndotl_histogram\":[{normal}],\"bound_violations\":{violations}}}"));
            }}}
            let globals=(1..=n as usize).map(|light|{
                let at=4+((3*32+light)*STAGES+3)*FIELDS;
                let fields=[(89,"light_visibility_calls"),(90,"scheduled_pixels"),(91,"inactive_tile_pixels"),(92,"hit_miss_pixels"),(93,"valid_receiver_pixels"),(94,"packed_word_stores"),(95,"ray_initializations")].iter().map(|(i,name)|format!("\"{name}\":{}",words[at+i])).collect::<Vec<_>>().join(",");
                format!("{{\"light_index\":{light},{fields}}}")
            }).collect::<Vec<_>>().join(",");
            let pose=std::env::var("RENDERLAB_LIGHT_COUNTER_POSE").ok().and_then(|s|s.parse::<u32>().ok()).map_or("null".into(),|v|v.to_string());
            let end=START+u64::from(n)*REPEATS-1;
            let json=format!("{{\"proxy\":\"pre-TR1 display-code proxy\",\"unshadowed_definition\":\"visibility 1 versus light absent, all other lit terms retained; an endpoint bound over any shadow result\",\"histogram_columns\":[\"actual_rays\",\"unshadowed_lt_1\",\"unshadowed_lt_half\",\"unshadowed_lt_quarter\"],\"distance_range_bins\":[\"0..0.25\",\"0.25..0.5\",\"0.5..0.75\",\"0.75..0.9\",\"0.9..1\",\"1+\",\"unbounded\"],\"ndotl_bins\":[\"negative\",\"0..0.1\",\"0.1..0.25\",\"0.25..0.5\",\"0.5..0.75\",\"0.75..1\"],\"repeats_per_light\":{REPEATS},\"render_frames\":[{START},{end}],\"pose_frame\":{pose},\"source_order\":\"range/cone, unflipped geometric source, then camera-oriented shading view among source pass\",\"facing_is_diagnostic\":true,\"primary_only\":true,\"producer_supported\":true,\"producer_normals\":\"joined to inline resolved normals for the same pixel and first-16 shadow-capable slot\",\"producer_globals\":[{globals}],\"rows\":[{}]}}",rows.join(","));
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
                        false,
                    );
                    assert_eq!(off.source, original.source);
                    for lvis in [false, true] {
                        let original = program::Composed {
                            source: original.source.clone(),
                            program: original.program.clone(),
                        };
                        let original = super::super::light_vis_pass::apply_enabled(original, lvis);
                        let c = variant(original, true, lvis);
                        validate(&c.source);
                        count += 1;
                    }
                }
            }
            let pass = super::super::light_vis_pass::apply_enabled(
                program::compose(
                    &heads[1],
                    super::super::light_vis_pass::PASS_PROGRAM,
                    program::Stage::Traced,
                    false,
                ),
                true,
            );
            let off = producer_variant(
                program::Composed {
                    source: pass.source.clone(),
                    program: pass.program.clone(),
                },
                false,
            );
            assert_eq!(off.source, pass.source);
            let c = producer_variant(pass, true);
            assert!(c.source.contains("lc_pass_begin(index);"));
            assert!(c.source.contains("lc_pass_add(93u,1u)"));
            validate(&c.source);
        }
        assert_eq!(count, 24);
    }
    fn validate(source: &str) {
        let m = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    }
}
