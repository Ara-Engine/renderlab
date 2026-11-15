//! Native visibility-tail composition. Packing belongs to program::material_trace_writes.
use crate::{Error, material_mode::Mode};

fn edit(source: &mut String, name: &str, before: &str, after: &str) -> Result<(), Error> {
    let begin = source
        .find(&format!("fn {name}("))
        .ok_or(Error::InvalidInput(
            "material tail requires the visibility entry",
        ))?;
    let end = begin
        + source[begin..].find("\n}\n").ok_or(Error::InvalidInput(
            "material tail requires the visibility entry end",
        ))?
        + 3;
    if source[begin..end].matches(before).count() != 1 {
        return Err(Error::InvalidInput(
            "material tail requires the unchanged visibility seam",
        ));
    }
    let body = source[begin..end].replacen(before, after, 1);
    source.replace_range(begin..end, &body);
    Ok(())
}

fn helpers(normal_probe: bool) -> String {
    let source = crate::program::material_trace_writes();
    if normal_probe {
        // Same dynamic packing and stores; omit only the normal attribute gather.
        source.replace(
            "rl_mr_receiver_words(receiver,rl_mr_trace_vertex_normal(trace),",
            "rl_mr_receiver_words(receiver,receiver.geometric,",
        )
    } else {
        source
    }
}

pub(super) fn trace(mut source: String, mode: Mode, normal_probe: bool) -> Result<String, Error> {
    if mode != Mode::Tail {
        return Ok(source);
    }
    for (entry, record) in [
        ("primary_trace", "record"),
        ("primary_trace_receivers", "trace.record"),
    ] {
        let seam = format!(
            "    if !primary_tile_covered({record}.x!=0u,id.xy,group.xy) || any(id.xy>=size) {{return;}}"
        );
        let replacement = format!(
            "    if all(id.xy<size) && {record}.x==0u {{rl_receiver_miss(vec2<i32>(id.xy));}}\n{seam}"
        );
        edit(&mut source, entry, &seam, &replacement)?;
    }
    edit(
        &mut source,
        "primary_trace",
        "    if record.x!=0u {textureStore(primary_epsilon_output,",
        "    if record.x!=0u {\n        let p=vec2<i32>(id.xy);\n        rl_receiver_store(p,trace,primary_receiver(trace.point,trace.positions,record.x-1u,p));\n    }\n    if record.x!=0u {textureStore(primary_epsilon_output,",
    )?;
    let receiver =
        "    let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);";
    edit(
        &mut source,
        "primary_trace_receivers",
        receiver,
        &format!("{receiver}\n    rl_receiver_store(p,trace,receiver);"),
    )?;
    if source.contains("fn primary_trace_interactive(") {
        for (entry, bounds) in [
            ("primary_trace_interactive", "inside"),
            ("primary_trace_interactive_deferred", "in_bounds"),
        ] {
            let seam = format!("    if !{bounds} || covered==0u {{return;}}");
            edit(
                &mut source,
                entry,
                &seam,
                &format!(
                    "    if {bounds} && trace.record.x==0u {{rl_receiver_miss(vec2<i32>(id.xy));}}\n{seam}"
                ),
            )?;
        }
        edit(
            &mut source,
            "primary_trace_interactive",
            receiver,
            &format!("{receiver}\n        rl_receiver_store(p,trace,receiver);"),
        )?;
        // Raw near-edge hits are overwritten by repair before any reader dispatch.
        let store = "    let p=vec2<i32>(id.xy);textureStore(primary_hit_output,p,trace.record);";
        edit(
            &mut source,
            "primary_trace_interactive_deferred",
            store,
            &format!("{store}\n    if trace.record.x!=0u {{rl_receiver_store(p,trace,receiver);}}"),
        )?;
        let repaired = "    let receiver=primary_receiver(trace.point,trace.positions,slot,p);";
        edit(
            &mut source,
            "primary_repair_interactive",
            repaired,
            &format!("{repaired}\n    rl_receiver_store(p,trace,receiver);"),
        )?;
    }
    source.push_str(&helpers(normal_probe));
    Ok(source)
}

pub(in crate::renderer) fn resolve(mut source: String, mode: Mode) -> Result<String, Error> {
    if mode != Mode::Tail {
        return Ok(source);
    }
    let exit = "    if atomicLoad(&covered)==0u || any(id.xy>=size) {return;}";
    edit(
        &mut source,
        "resolve",
        exit,
        &format!(
            "    if all(id.xy<size) && trace.record.x==0u {{rl_receiver_miss(vec2<i32>(id.xy));}}\n{exit}"
        ),
    )?;
    let receiver =
        "    var receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);";
    // Store the repaired point before the sun's per-pass LOD lift.
    edit(
        &mut source,
        "resolve",
        receiver,
        &format!("{receiver}\n    rl_receiver_store(p,trace,receiver);"),
    )?;
    source.push_str(&helpers(false));
    Ok(source)
}

const CHECK: &str = r#"
@group(0) @binding(0) var hit:texture_2d<u32>;
@group(0) @binding(1) var record:texture_2d_array<u32>;
@group(0) @binding(2) var tiles:texture_2d<u32>;
@group(0) @binding(3) var<storage,read_write> counts:array<atomic<u32>,8>;
@group(0) @binding(4) var<storage,read> placements:array<u32>;
@compute @workgroup_size(8,4) fn check(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(hit)) {return;}
    let p=vec2<i32>(id.xy);let h=textureLoad(hit,p,0);
    let empty=textureLoad(tiles,vec2<i32>(id.xy/vec2<u32>(8u,4u)),0).x==0u;
    let a=textureLoad(record,p,0,0);let b=textureLoad(record,p,1,0);
    if empty || h.x==0u {
        atomicAdd(&counts[1],1u);
        if empty {atomicAdd(&counts[2],1u);}
        if any(a!=vec4<u32>(0u,0u,0u,0xffffffffu)) || any(b!=vec4<u32>(0u)) {atomicAdd(&counts[3],1u);}
    } else {
        atomicAdd(&counts[0],1u);
        if (a.w&0x0fffffffu)!=(placements[h.x-1u]&0x1fffffffu) {atomicAdd(&counts[4],1u);}
        if a.y!=0u || a.z!=0u {atomicAdd(&counts[5],1u);}
        if (a.w&0x10000000u)!=0u {atomicAdd(&counts[6],1u);}
        if any((b.xyz&vec3<u32>(0x7f800000u))==vec3<u32>(0x7f800000u)) {atomicAdd(&counts[7],1u);}
    }
}
"#;

impl crate::renderer::Renderer {
    /// Diagnostic scan after visibility. Every miss, including fast-clear tiles, must
    /// overwrite last frame's words. Keep the ordinary hit checks in every reader.
    pub(in crate::renderer) fn encode_material_tail_check(
        &self,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if !Mode::tail() || std::env::var("RENDERLAB_MATERIAL_TAIL_CHECK").as_deref() != Ok("1") {
            return;
        }
        let Some(targets) = self.view.primary.as_ref().and_then(|s| s.targets.as_ref()) else {
            return;
        };
        let words = targets.material.as_ref().unwrap();
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("material tail check"),
                source: wgpu::ShaderSource::Wgsl(CHECK.into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("material tail check"),
                layout: None,
                module: &module,
                entry_point: Some("check"),
                compilation_options: Default::default(),
                cache: None,
            });
        let counts = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material tail counters"),
            size: 32,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material tail readback"),
            size: 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material tail check"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&targets.hit.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&words.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&targets.tiles.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.view.primary.as_ref().unwrap().geometry.as_ref().unwrap()._placements.value.as_entire_binding(),
                },
            ],
        });
        encoder.clear_buffer(&counts, 0, None);
        let size = targets.hit.texture.size();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("material tail check"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(4), 1);
        }
        encoder.copy_buffer_to_buffer(&counts, 0, &readback, 0, 32);
        let mapped = readback.clone();
        let frame = self.frames;
        encoder.map_buffer_on_submit(&readback,wgpu::MapMode::Read,..,move |result| {
            if result.is_ok() {
                if let Ok(bytes) = mapped.slice(..).get_mapped_range() {
                    let n: Vec<_> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
                    eprintln!("material_tail_check frame={frame} hits={} misses={} fast_misses={} bad_miss={} bad_slot={} bad_yz={} bad_near={} bad_point={}",n[0],n[1],n[2],n[3],n[4],n[5],n[6],n[7]);
                }
            } else { eprintln!("material_tail_check map_failed"); }
            mapped.unmap();
        });
    }
}

pub(super) fn normal_probe() -> bool {
    std::env::var("RENDERLAB_MATERIAL_TAIL_PROBE").as_deref() == Ok("geometric")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::{primary_ray_gpu as primary, visibility_buffer};

    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let global = module
            .global_variables
            .iter()
            .find(|(_, v)| {
                v.binding
                    .as_ref()
                    .is_some_and(|b| b.group == 0 && b.binding == 75)
            })
            .unwrap()
            .1;
        assert!(matches!(
            module.types[global.ty].inner,
            naga::TypeInner::Image {
                arrayed: true,
                class: naga::ImageClass::Storage {
                    format: naga::StorageFormat::Rgba32Uint,
                    access: naga::StorageAccess::STORE
                },
                ..
            }
        ));
    }

    #[test]
    fn material_tail_composes_native_visibility_paths() {
        let check = naga::front::wgsl::parse_str(CHECK).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&check)
        .unwrap();
        let scene = include_str!("../shaders/scene.wgsl");
        let [hard, soft] = primary::shader_sources(scene).unwrap();
        for source in [hard, soft] {
            assert_eq!(trace(source.clone(), Mode::Off, false).unwrap(), source);
            assert_eq!(trace(source.clone(), Mode::Full, false).unwrap(), source);
            for probe in [false, true] {
                let tail = trace(source.clone(), Mode::Tail, probe).unwrap();
                validate(&tail);
                if tail.contains("fn primary_trace_interactive(") {
                    validate(&primary::temporal_reconstruction::trace_source_held(&tail).unwrap());
                    validate(&primary::film_accumulation::kernel_source(&tail).unwrap());
                }
            }
        }
        let raw = visibility_buffer::resolve_source(scene).unwrap();
        assert_eq!(resolve(raw.clone(), Mode::Off).unwrap(), raw);
        let tail = resolve(raw, Mode::Tail).unwrap();
        validate(&tail);
        assert!(
            tail.find("rl_receiver_store(p,trace,receiver);").unwrap()
                < tail.find("    if lift>0.0").unwrap()
        );
        assert_eq!(
            tail.matches("rl_receiver_miss(vec2<i32>(id.xy))").count(),
            1
        );
    }
}
