//! wgpu material-record seam. The shared composer owns packing and probe variants.
use super::*;
use crate::program::{self, MaterialRole, Stage};

/// The engine's composition boundary, also used by the naga/layout checks. Keep
/// reflection history/pass edits ahead of the shared material-record variant.
fn composition(
    head: &str,
    source: &str,
    stage: Stage,
    list: bool,
    role: MaterialRole,
) -> Option<program::Composed> {
    composition_control(head, source, stage, list, role, reader_control_on())
}

fn reader_control_on() -> bool {
    std::env::var("RENDERLAB_MATERIAL_READER_CONTROL").as_deref() == Ok("1")
}

// The record rounds roughness upward. Include the code containing 0.05 so
// the control cannot miss a low-roughness pixel at the quantized boundary.
const READER_SEAM: &str =
    "    let surface_output=material_in(rl_material_load(pixel),surface_input);\n";
const READER_CONTROL: &str = "    let rl_control_record=rl_material_load(pixel);\n\
    var surface_output:Surface;\n\
    if rl_control_record.roughness<=3277.0/65535.0 {\n\
        surface_output=surface(surface_input);\n\
        rl_material_n=material_out(surface_output,surface_input).normal;\n\
        rl_material_view_hint=tr1_view_hint(surface_output,surface_input,view_ray);\n\
    } else {\n\
        surface_output=material_in(rl_control_record,surface_input);\n\
    }\n";

const CONTROL_STATS: &str = r#"
@group(0) @binding(0) var hit:texture_2d<u32>;
@group(0) @binding(1) var record:texture_2d<u32>;
@group(0) @binding(2) var<storage,read_write> counts:array<atomic<u32>,4>;
@compute @workgroup_size(8,4) fn count_pixels(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(hit)) {return;}
    atomicAdd(&counts[3],1u);
    let p=vec2<i32>(id.xy);
    if textureLoad(hit,p,0).x==0u {return;}
    atomicAdd(&counts[2],1u);
    let word=textureLoad(record,p,0).y;
    if (word&255u)==255u {return;}
    atomicAdd(&counts[1],1u);
    if (word>>16u)<=3277u {atomicAdd(&counts[0],1u);}
}
"#;

fn composition_control(
    head: &str,
    source: &str,
    stage: Stage,
    list: bool,
    role: MaterialRole,
    control: bool,
) -> Option<program::Composed> {
    let parameters = program::declares_parameters(source);
    let full = if list {
        program::compose_list(head, source, stage, parameters)
    } else {
        program::compose(head, source, stage, parameters)
    };
    let mut composed =
        program::material_variant(program::with_reflection_history(full), source, role)?;
    if control && role == MaterialRole::Read {
        let entry = composed.source.find("fn primary_shade_pixel(")?;
        let at = entry + composed.source[entry..].find(READER_SEAM)?;
        composed
            .source
            .replace_range(at..at + READER_SEAM.len(), READER_CONTROL);
    }
    Some(composed)
}

pub(super) fn entries(
    write: bool,
) -> [wgpu::BindGroupLayoutEntry; program::MATERIAL_RECORD_BINDINGS.len()] {
    program::MATERIAL_RECORD_BINDINGS.map(|binding| {
        if write {
            image(binding, wgpu::TextureFormat::Rgba32Uint)
        } else {
            texture(binding, wgpu::TextureSampleType::Uint)
        }
    })
}

#[derive(Clone)]
pub(super) struct Layouts {
    pub groups: [Owned<wgpu::BindGroupLayout>; 2],
    layouts: [[Owned<wgpu::PipelineLayout>; 2]; 2],
    pub lists: ListLayouts,
}
impl Layouts {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        shade: &[Vec<wgpu::BindGroupLayoutEntry>; 2],
        arrays: &wgpu::BindGroupLayout,
        materials: [&wgpu::BindGroupLayout; 2],
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        let rt = device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let shade = shade.each_ref().map(|e| {
            e.iter()
                .copied()
                .chain(entries(true))
                .filter(|e| rt || !matches!(e.ty, wgpu::BindingType::AccelerationStructure { .. }))
                .collect::<Vec<_>>()
        });
        let groups = shade.each_ref().map(|entries| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("material writer"),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        });
        let layouts = materials.map(|m| {
            groups.each_ref().map(|g| {
                ledger.own(
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("material writer ABI"),
                        bind_group_layouts: &[Some(g), Some(arrays), Some(m), Some(lighting)],
                        immediate_size: 0,
                    }),
                    ResourceKind::PipelineLayout,
                )
            })
        });
        Self {
            groups,
            layouts,
            lists: ListLayouts::new(device, ledger, &shade, arrays, materials, lighting),
        }
    }
}
impl Renderer {
    /// Diagnostic only: count once at the requested renderer frame, then print
    /// after the ordinary device poll completes its asynchronous map. No wait.
    pub(in crate::renderer) fn encode_material_reader_stats(
        &self,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let requested = std::env::var("RENDERLAB_MATERIAL_READER_STATS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());
        if requested != Some(self.frames) {
            return;
        }
        let Some(targets) = self.view.primary.as_ref().and_then(|s| s.targets.as_ref()) else {
            return;
        };
        let Some(words) = targets.material.as_ref() else {
            return;
        };
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("material reader statistics"),
                source: wgpu::ShaderSource::Wgsl(CONTROL_STATS.into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("material reader statistics"),
                layout: None,
                module: &module,
                entry_point: Some("count_pixels"),
                compilation_options: Default::default(),
                cache: None,
            });
        let counts = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material reader counts"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material reader counts readback"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material reader statistics"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&targets.hit.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&words[1].view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: counts.as_entire_binding(),
                },
            ],
        });
        encoder.clear_buffer(&counts, 0, None);
        let size = targets.hit.texture.size();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("material reader statistics"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(4), 1);
        }
        encoder.copy_buffer_to_buffer(&counts, 0, &readback, 0, 16);
        let mapped = readback.clone();
        let frame = self.frames;
        let view = self.view_key.0;
        encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
            if result.is_ok() {
                if let Ok(bytes) = mapped.slice(..).get_mapped_range() {
                    let n: Vec<_> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
                    eprintln!("material_reader_stats frame={frame} view={view} low={} mapped={} hits={} total={} low_frame_share={:.8} low_mapped_share={:.8}", n[0], n[1], n[2], n[3], f64::from(n[0])/f64::from(n[3].max(1)), f64::from(n[0])/f64::from(n[1].max(1)));
                }
            } else { eprintln!("material_reader_stats frame={frame} map_failed"); }
            mapped.unmap();
        });
    }

    pub(crate) fn ensure_material_pass(&mut self, shading: Shading) -> Result<(), Error> {
        if !program::material_pass_on() || self.view.soft_mode == SoftShadows::Temporal {
            return Ok(());
        }
        let state = self.view.primary.as_ref().unwrap();
        let mixed = state.mixed();
        let streamed = state.requests;
        let ids = if mixed {
            state.programs.clone()
        } else {
            vec![state.program]
        };
        let si = shading as usize;
        for id in ids {
            let slot = self.program_slot(id);
            if slot.pipelines.material[mixed as usize][streamed as usize][si].is_some() {
                continue;
            }
            let head = &self.primary_sources.as_ref().unwrap().1[si];
            let stage = if streamed {
                Stage::TracedStreamed
            } else {
                Stage::Traced
            };
            let Some(write) = composition(head, &slot.source, stage, mixed, MaterialRole::Write)
            else {
                continue;
            };
            // Receiver-only writers (model 255 in the three-word schema) leave
            // custom shading unchanged when the shared composer supplies no reader.
            let read = composition(head, &slot.source, stage, mixed, MaterialRole::Read)
                .unwrap_or_else(|| {
                    let parameters = program::declares_parameters(&slot.source);
                    program::with_reflection_history(if mixed {
                        program::compose_list(head, &slot.source, stage, parameters)
                    } else {
                        program::compose(head, &slot.source, stage, parameters)
                    })
                });
            let abi = self.program_abi();
            let scopes = OperationScopes::new(&self.device);
            let state = self.view.primary.as_ref().unwrap();
            let l = state.material_layouts.as_ref().unwrap();
            let writer = if mixed {
                if streamed {
                    &l.lists.streamed_layouts[si]
                } else {
                    &l.lists.layouts[si]
                }
            } else {
                &l.layouts[streamed as usize][si]
            };
            let reader = if mixed {
                if streamed {
                    &state.list_layouts.streamed_layouts[si]
                } else {
                    &state.list_layouts.layouts[si]
                }
            } else if streamed {
                &state.streamed_layouts[si]
            } else {
                &state.shade_layouts[si]
            };
            let sources = [write, read];
            let pipelines =
                [(&sources[0], writer), (&sources[1], reader)].map(|(source, layout)| {
                    let module = super::super::program_gpu::module(&abi, &slot.label, source);
                    let start = Instant::now();
                    let pipeline = self.ledger.own(
                        self.device
                            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                                label: Some("material seam"),
                                layout: Some(layout),
                                module: &module,
                                entry_point: Some(if mixed {
                                    "primary_shade_list"
                                } else {
                                    "primary_shade_tiles"
                                }),
                                compilation_options: Default::default(),
                                cache: abi.cache,
                            }),
                        ResourceKind::ComputePipeline,
                    );
                    eprintln!(
                        "material_pipeline program={} role={} seconds={:.3} cache=warm",
                        slot.label,
                        if std::ptr::eq(source, &sources[0]) {
                            "writer"
                        } else {
                            "reader"
                        },
                        start.elapsed().as_secs_f64()
                    );
                    pipeline
                });
            if let Some(error) = scopes.finish() {
                return Err(super::super::program_gpu::scope_error(
                    &slot.label,
                    error,
                    &sources,
                ));
            }
            self.pipelines.created += 2;
            self.program_cache.count(2);
            self.programs
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .pipelines
                .material[mixed as usize][streamed as usize][si] = Some(pipelines);
        }
        Ok(())
    }
    /// Writer and reader are separate compute passes: storage writes must finish before sampling.
    pub(in crate::renderer) fn encode_material_shading(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
        stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        write: bool,
    ) -> bool {
        if !program::material_pass_on() || self.view.soft_mode == SoftShadows::Temporal {
            return false;
        }
        let state = self.view.primary.as_ref().unwrap();
        let si = shading as usize;
        let mixed = state.mixed();
        let pair = |id| {
            self.program_slot(id).pipelines.material[mixed as usize][state.requests as usize][si]
                .as_ref()
        };
        if mixed && !state.programs.iter().any(|&id| pair(id).is_some()) {
            return false;
        }
        if !mixed {
            let Some(p) = pair(state.program) else {
                return false;
            };
            let group = state.bindings.as_ref().unwrap().material_write[0][si]
                .as_ref()
                .unwrap();
            if write {
                self.encode_shade_group(encoder, &p[0], group, SHADE_TILE, None);
            } else {
                self.encode_primary_shade(encoder, &p[1], SHADE_TILE, shading, stamps);
            }
            return true;
        }
        let lists = state.lists.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        if write {
            encoder.clear_buffer(&lists.args.value, 0, None);
            for k in 0..state.programs.len() as u64 {
                encoder.clear_buffer(
                    &lists.entries.value,
                    k * u64::from(lists.capacity) * 4,
                    Some(4),
                );
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("material bins"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(state.bins.as_ref().unwrap());
                pass.set_bind_group(0, &*lists.bins.as_ref().unwrap().0, &[]);
                let [x, y] = state.region_groups([4, 4]);
                let [bx, by] = [0, 1].map(|i| [size.width, size.height][i].div_ceil(CLEAR_TILE[i]));
                pass.dispatch_workgroups(x.max(bx), y + by, 1);
            }
        }
        for role in [usize::from(!write)] {
            let group = if role == 0 {
                &state.bindings.as_ref().unwrap().material_write[1][si]
            } else {
                &state.bindings.as_ref().unwrap().list[si]
            };
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(if role == 0 {
                    "material writer"
                } else {
                    "material reader"
                }),
                timestamp_writes: if role == 1 { stamps.clone() } else { None },
            });
            pass.set_bind_group(1, &*g.arrays, &[]);
            pass.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            for (k, &id) in state.programs.iter().enumerate() {
                let pipeline = if let Some(p) = pair(id) {
                    &*p[role]
                } else if role == 0 {
                    continue;
                } else {
                    &**self.program_slot(id).pipelines.list[state.requests as usize][si]
                        .as_ref()
                        .unwrap()
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &**group.as_ref().unwrap(), &[k as u32 * lists.stride]);
                pass.dispatch_workgroups_indirect(&lists.args.value, k as u64 * 16);
            }
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_layout_matches_both_path_compositions() {
        let stats = naga::front::wgsl::parse_str(CONTROL_STATS).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&stats)
        .unwrap();
        let scene = include_str!("../shaders/scene.wgsl");
        let rt = shading_heads(scene).unwrap();
        let nonrt = non_rt_sources(scene).unwrap().1;
        for program_source in [
            crate::STANDARD_PROGRAM,
            include_str!("../examples/game_scene.wgsl"),
        ] {
            for head in rt.iter().chain(&nonrt) {
                for list in [false, true] {
                    for stage in [Stage::Traced, Stage::TracedStreamed] {
                        for (role, write, control) in [
                            (MaterialRole::Write, true, false),
                            (MaterialRole::Read, false, false),
                            (MaterialRole::Read, false, true),
                        ] {
                            let source = composition_control(
                                head,
                                program_source,
                                stage,
                                list,
                                role,
                                control,
                            )
                            .unwrap()
                            .source;
                            if control {
                                assert!(source.contains(READER_CONTROL));
                            }
                            let module = naga::front::wgsl::parse_str(&source)
                                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                            naga::valid::Validator::new(
                                naga::valid::ValidationFlags::all(),
                                naga::valid::Capabilities::all(),
                            )
                            .validate(&module)
                            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                            for layout in entries(write) {
                                let global = module
                                    .global_variables
                                    .iter()
                                    .find(|(_, g)| {
                                        g.binding.as_ref().is_some_and(|b| {
                                            b.group == 0 && b.binding == layout.binding
                                        })
                                    })
                                    .unwrap()
                                    .1;
                                let naga::TypeInner::Image { class, .. } =
                                    module.types[global.ty].inner
                                else {
                                    panic!("record is an image")
                                };
                                match (layout.ty, class) {
                                    (
                                        wgpu::BindingType::StorageTexture {
                                            access: wgpu::StorageTextureAccess::WriteOnly,
                                            format: wgpu::TextureFormat::Rgba32Uint,
                                            ..
                                        },
                                        naga::ImageClass::Storage {
                                            format: naga::StorageFormat::Rgba32Uint,
                                            access,
                                        },
                                    ) => assert_eq!(access, naga::StorageAccess::STORE),
                                    (
                                        wgpu::BindingType::Texture {
                                            sample_type: wgpu::TextureSampleType::Uint,
                                            ..
                                        },
                                        naga::ImageClass::Sampled {
                                            kind: naga::ScalarKind::Uint,
                                            ..
                                        },
                                    ) => {}
                                    other => panic!("record ABI mismatch: {other:?}"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
