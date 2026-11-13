//! wgpu material-record seam. The shared composer owns packing and probe variants.
use super::*;
use crate::program::{self, MaterialRole, Stage};

pub(super) fn entries(write: bool) -> [wgpu::BindGroupLayoutEntry; 2] {
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
            let full = || {
                let parameters = program::declares_parameters(&slot.source);
                program::with_reflection_history(if mixed {
                    program::compose_list(head, &slot.source, stage, parameters)
                } else {
                    program::compose(head, &slot.source, stage, parameters)
                })
            };
            let Some(write) = program::material_variant(full(), &slot.source, MaterialRole::Write)
            else {
                continue;
            };
            let read = program::material_variant(full(), &slot.source, MaterialRole::Read).unwrap();
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
        let scene = include_str!("../shaders/scene.wgsl");
        let rt = shading_heads(scene).unwrap();
        let nonrt = non_rt_sources(scene).unwrap().1;
        for head in rt.iter().chain(&nonrt) {
            for list in [false, true] {
                for stage in [Stage::Traced, Stage::TracedStreamed] {
                    for (role, write) in [(MaterialRole::Write, true), (MaterialRole::Read, false)]
                    {
                        let source = if list {
                            program::compose_list(head, crate::STANDARD_PROGRAM, stage, false)
                        } else {
                            program::compose(head, crate::STANDARD_PROGRAM, stage, false)
                        };
                        let source =
                            program::material_variant(source, crate::STANDARD_PROGRAM, role)
                                .unwrap()
                                .source;
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

