//! Owned compile recipes let the render thread keep submitting the previous revision.
//! Only hot sets are captured. Their four bind-group layouts are the existing explicit
//! program ABI, with no immediate data; shader bodies and dispatches are unchanged.
use super::*;
use std::thread::JoinHandle;

struct Recipe {
    variant: Variant,
    layouts: Vec<Owned<wgpu::PipelineLayout>>,
}

// P-SHC: mixed-program lists must finish compiling before a reload is published.
// Otherwise the worker validates modules, but the next frame blocks in ensure_lists.
struct ListRecipe {
    streamed: usize,
    shading: usize,
    layout: Owned<wgpu::PipelineLayout>,
}

struct Prepared {
    compiled: Compiled,
    variants: Vec<Variant>,
    built: Vec<Vec<Built>>,
    lists: Vec<(usize, usize, Owned<wgpu::ComputePipeline>)>,
    glass: Option<([u8; 32], Owned<wgpu::ComputePipeline>)>,
    compile: Duration,
}

pub(crate) struct ProgramReload {
    program: crate::Program,
    source: String,
    revision: Revision,
    primary_hash: [u8; 32],
    variants: Vec<Variant>,
    lists: u8,
    glass: Option<wgpu::ComputePipeline>,
    started: Instant,
    policy: crate::shader_compiler::CompilePolicy,
    worker: JoinHandle<(Result<Prepared, Error>, Duration)>,
}
impl ProgramReload {
    pub(crate) fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }
    pub(crate) fn discard(self) {
        let _ = self.worker.join();
    }

    /// A prelude/program transaction or visibility change invalidates this snapshot.
    /// A frame that switched to a previously cold set also needs a new recipe.
    pub(crate) fn current(&self, renderer: &Renderer) -> bool {
        self.revision == renderer.revision
            && self.primary_hash == renderer.primary_hash
            && renderer
                .hot
                .iter()
                .filter(|(id, _)| *id == self.program.id())
                .all(|(_, variant)| self.variants.contains(variant))
            && renderer
                .hot_lists
                .iter()
                .filter(|(id, _, _)| *id == self.program.id())
                .all(|&(_, streamed, shading)| {
                    self.lists & (1 << (usize::from(streamed) * 2 + shading)) != 0
                })
            && renderer.tp_reload_pipeline(self.program.id()) == self.glass.as_ref()
    }
}

impl Renderer {
    #[cfg(test)]
    pub(crate) fn start_program_reload(
        &mut self,
        program: &crate::Program,
        source: &str,
    ) -> Result<ProgramReload, ReloadError> {
        self.start_program_reload_with_policy(
            program,
            source,
            crate::shader_compiler::CompilePolicy::Optimized,
        )
    }

    pub(crate) fn can_preview_programs(&self) -> bool {
        crate::shader_compiler::capabilities(&self.device).is_some()
            && !self.hot.iter().any(|(_, v)| *v == Variant::Temporal)
    }

    pub(crate) fn program_source_matches(&self, program: &crate::Program, source: &str) -> bool {
        Arc::ptr_eq(&program.0.renderer, &self.identity)
            && self
                .programs
                .iter()
                .any(|slot| slot.id == program.id() && slot.source == source)
    }

    pub(crate) fn start_program_reload_with_policy(
        &mut self,
        program: &crate::Program,
        source: &str,
        policy: crate::shader_compiler::CompilePolicy,
    ) -> Result<ProgramReload, ReloadError> {
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        if !Arc::ptr_eq(&program.0.renderer, &self.identity) {
            return Err(ReloadError::Rejected {
                diagnostic: "program belongs to another renderer".into(),
                compile: Duration::ZERO,
            });
        }
        self.pipelines.attempts += 1;
        self.program_cache.refresh(&self.device);
        let slot = self.program_slot(program.id());
        let variants: Vec<_> = self.hot_sets(program.id(), &slot.modules).collect();
        let device = self.device.clone();
        let ledger = self.ledger.clone();
        let samples = self.options.samples;
        let lit = (*self.lighting.layout).clone();
        let cache = self.program_cache.get().cloned();
        let modules = self.program_cache.modules.clone();
        let raster = self.source.clone();
        let hardware = self.hardware_source.clone();
        let fresh = self.fresh_source.clone();
        let traced = self
            .primary_sources
            .as_ref()
            .map(|(_, heads)| heads.clone());
        let recipes: Vec<_> = variants
            .iter()
            .map(|&variant| {
                let layouts = (0..variant.len())
                    .map(|index| {
                        let groups: [_; 4] = std::array::from_fn(|group| match variant {
                            Variant::Lit => slot.pipelines.lit.as_ref().unwrap()[index]
                                .get_bind_group_layout(group as u32),
                            Variant::Hardware => slot.pipelines.hardware.as_ref().unwrap()[index]
                                .get_bind_group_layout(group as u32),
                            Variant::Fresh => slot.pipelines.fresh.as_ref().unwrap()[index]
                                .get_bind_group_layout(group as u32),
                            Variant::Traced(shading) => slot.pipelines.traced[shading as usize]
                                .as_ref()
                                .unwrap()
                                .get_bind_group_layout(group as u32),
                            Variant::Temporal => slot
                                .pipelines
                                .temporal
                                .as_ref()
                                .unwrap()
                                .get_bind_group_layout(group as u32),
                        });
                        ledger.own(
                            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                                label: Some("background program ABI"),
                                bind_group_layouts: &groups.each_ref().map(Some),
                                immediate_size: 0,
                            }),
                            ResourceKind::PipelineLayout,
                        )
                    })
                    .collect();
                Recipe { variant, layouts }
            })
            .collect();
        let mut list_recipes = Vec::new();
        let mut lists = 0;
        for &(_, streamed, shading) in self
            .hot_lists
            .iter()
            .filter(|(id, _, _)| *id == program.id())
        {
            let streamed = usize::from(streamed);
            let Some(pipeline) = slot.pipelines.list[streamed][shading].as_ref() else {
                continue;
            };
            let groups: [_; 4] =
                std::array::from_fn(|group| pipeline.get_bind_group_layout(group as u32));
            list_recipes.push(ListRecipe {
                streamed,
                shading,
                layout: ledger.own(
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("background program list ABI"),
                        bind_group_layouts: &groups.each_ref().map(Some),
                        immediate_size: 0,
                    }),
                    ResourceKind::PipelineLayout,
                ),
            });
            lists |= 1 << (streamed * 2 + shading);
        }
        let started = Instant::now();
        let glass = self.tp_reload_pipeline(program.id()).cloned();
        let glass_recipe =
            self.tp_reload_recipe(program.id(), source)
                .map_err(|e| ReloadError::Rejected {
                    diagnostic: e.to_string(),
                    compile: started.elapsed(),
                })?;
        let label = program.label().to_owned();
        let text = source.to_owned();
        let fault = self.fault.clone();
        let worker = std::thread::spawn(move || {
            let result = std::thread::scope(|scope| {
                let abi = ProgramAbi {
                    device: &device,
                    ledger: &ledger,
                    samples,
                    lit: &lit,
                    hardware: None,
                    fresh: None,
                    primary: None,
                    cache: cache.as_ref(),
                    modules: &modules,
                    vertex_identity: Sha256::digest(raster.as_bytes()).into(),
                };
                let compiled = compile_program(
                    &abi,
                    &Preludes {
                        raster: &raster,
                        hardware: hardware.as_deref(),
                        fresh: fresh.as_deref(),
                        traced: traced.as_ref(),
                    },
                    &label,
                    &text,
                )?;
                let glass_worker = glass_recipe.map(|recipe| {
                    let device = device.clone();
                    let ledger = ledger.clone();
                    let cache = cache.clone();
                    scope.spawn(move || {
                        let scopes = OperationScopes::new(&device);
                        let module = ledger.own(
                            {
                                let descriptor = wgpu::ShaderModuleDescriptor {
                                    label: Some("background glass reflection"),
                                    source: wgpu::ShaderSource::Wgsl(recipe.source.into()),
                                };
                                device.create_program_module(descriptor)
                            },
                            ResourceKind::ShaderModule,
                        );
                        let pipeline = crate::shader_compiler::with_policy(policy, || {
                            ledger.own(
                                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                                    label: Some("background glass reflection"),
                                    layout: Some(&recipe.layout),
                                    module: &module,
                                    entry_point: Some("tp_reflect"),
                                    compilation_options: Default::default(),
                                    cache: cache.as_ref(),
                                }),
                                ResourceKind::ComputePipeline,
                            )
                        });
                        match scopes.finish() {
                            Some(error) => Err(scope_error("glass reflection", error, &[])),
                            None => Ok((recipe.key, pipeline)),
                        }
                    })
                });
                let module_time = started.elapsed();
                let vertex_identity = Sha256::digest(raster.as_bytes()).into();
                let jobs = recipes.iter().flat_map(|recipe| {
                    let abi = &abi;
                    let compiled = &compiled;
                    recipe
                        .layouts
                        .iter()
                        .enumerate()
                        .map(move |(index, layout)| {
                            move || {
                                let m = &compiled.modules;
                                crate::shader_compiler::with_program_policy(
                                    policy,
                                    vertex_identity,
                                    || match recipe.variant {
                                        Variant::Lit | Variant::Hardware | Variant::Fresh => {
                                            let module = match recipe.variant {
                                                Variant::Lit => &m.lit,
                                                Variant::Hardware => m.hardware.as_ref().unwrap(),
                                                _ => m.fresh.as_ref().unwrap(),
                                            };
                                            Built::Render(mesh_pipeline(
                                                abi.device,
                                                abi.ledger,
                                                module,
                                                layout,
                                                wgpu::TextureFormat::Rgba16Float,
                                                "fs_lit",
                                                index == 1,
                                                if recipe.variant == Variant::Fresh {
                                                    SampleCount::One
                                                } else {
                                                    samples
                                                },
                                                abi.cache,
                                            ))
                                        }
                                        Variant::Traced(_) | Variant::Temporal => {
                                            let module = match recipe.variant {
                                                Variant::Traced(shading) => {
                                                    &m.traced.as_ref().unwrap()[shading as usize]
                                                }
                                                _ => m.temporal.as_ref().unwrap(),
                                            };
                                            Built::Compute(abi.ledger.own(
                                                abi.device.create_compute_pipeline(
                                                    &wgpu::ComputePipelineDescriptor {
                                                        label: Some("background program shading"),
                                                        layout: Some(layout),
                                                        module,
                                                        entry_point: Some("primary_shade_tiles"),
                                                        compilation_options: Default::default(),
                                                        cache: abi.cache,
                                                    },
                                                ),
                                                ResourceKind::ComputePipeline,
                                            ))
                                        }
                                    },
                                )
                            }
                        })
                });
                let mut results = in_parallel(&device, jobs).into_iter();
                let mut built = vec![];
                for recipe in &recipes {
                    let mut set = vec![];
                    for _ in 0..recipe.variant.len() {
                        let (pipeline, error) = results.next().unwrap();
                        if let Some(error) = error {
                            return Err(scope_error(&label, error, &[]));
                        }
                        set.push(pipeline);
                    }
                    built.push(set);
                }
                let mut list_modules = Vec::new();
                for recipe in &list_recipes {
                    let stage = if recipe.streamed == 0 {
                        Stage::Traced
                    } else {
                        Stage::TracedStreamed
                    };
                    let composed = program::compose_list(
                        &traced.as_ref().unwrap()[recipe.shading],
                        &text,
                        stage,
                        program::declares_parameters(&text),
                    );
                    let scopes = OperationScopes::new(&device);
                    let module = module(&abi, &label, &composed);
                    if let Some(error) = scopes.finish() {
                        return Err(scope_error(&label, error, &[composed]));
                    }
                    list_modules.push((composed, module));
                }
                let mut lists = Vec::new();
                for (recipe, (pipeline, error)) in list_recipes.iter().zip(in_parallel(
                    &device,
                    list_recipes
                        .iter()
                        .zip(&list_modules)
                        .map(|(recipe, (_, module))| {
                            let abi = &abi;
                            move || {
                                crate::shader_compiler::with_policy(policy, || {
                                    abi.ledger.own(
                                        abi.device.create_compute_pipeline(
                                            &wgpu::ComputePipelineDescriptor {
                                                label: Some("background program list shading"),
                                                layout: Some(&recipe.layout),
                                                module,
                                                entry_point: Some("primary_shade_list"),
                                                compilation_options: Default::default(),
                                                cache: abi.cache,
                                            },
                                        ),
                                        ResourceKind::ComputePipeline,
                                    )
                                })
                            }
                        }),
                )) {
                    if let Some(error) = error {
                        return Err(scope_error(&label, error, &[]));
                    }
                    lists.push((recipe.streamed, recipe.shading, pipeline));
                }
                for (composed, module) in list_modules {
                    abi.modules.insert(&composed.source, module);
                }
                let glass = glass_worker
                    .map(|worker| worker.join().unwrap())
                    .transpose()?;
                if std::env::var_os("RENDERLAB_RELOAD_PROFILE").is_some() {
                    eprintln!(
                        "program_reload_profile modules_ms={:.3} pipelines_ms={:.3} pipelines={} parallel=true",
                        module_time.as_secs_f64() * 1e3,
                        (started.elapsed() - module_time).as_secs_f64() * 1e3,
                        pipeline_count(&built) + lists.len() as u64 + u64::from(glass.is_some())
                    );
                }
                Ok(Prepared {
                    compiled,
                    variants: recipes.iter().map(|r| r.variant).collect(),
                    built,
                    lists,
                    glass,
                    compile: started.elapsed(),
                })
            });
            if let Err(Error::Fault(error)) = &result {
                let _ = fault.set(error.clone());
            }
            (result, started.elapsed())
        });
        Ok(ProgramReload {
            program: program.clone(),
            source: source.into(),
            revision: self.revision,
            primary_hash: self.primary_hash,
            variants,
            lists,
            glass,
            started,
            policy,
            worker,
        })
    }

    /// Called only for a finished, current job, between frames. No worker can mutate
    /// slots, parameter buffers, source identities or the active revision.
    pub(crate) fn finish_program_reload(
        &mut self,
        reload: ProgramReload,
    ) -> Result<ReloadReport, ReloadError> {
        let (prepared, elapsed) = reload.worker.join().unwrap_or_else(|_| {
            (
                Err(Error::Fault(Fault(
                    "program compile worker panicked".into(),
                ))),
                reload.started.elapsed(),
            )
        });
        let Prepared {
            compiled,
            variants,
            built,
            lists,
            glass,
            compile,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(Error::Fault(fault)) => {
                let _ = self.fault.set(fault.clone());
                return Err(ReloadError::Fault(fault));
            }
            Err(error) => {
                return Err(ReloadError::Rejected {
                    diagnostic: error.to_string(),
                    compile: elapsed,
                });
            }
        };
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        let report = self.install_program(
            &reload.program,
            &reload.source,
            compiled,
            variants,
            built,
            compile,
        );
        self.pipelines.created += lists.len() as u64;
        self.program_cache.count(lists.len() as u64);
        let slot = self
            .programs
            .iter_mut()
            .find(|s| s.id == reload.program.id())
            .unwrap();
        for (streamed, shading, pipeline) in lists {
            slot.pipelines.list[streamed][shading] = Some(pipeline);
        }
        if let Some((key, pipeline)) = glass {
            self.pipelines.created += 1;
            self.program_cache.count(1);
            self.tp_install_reload(reload.glass.as_ref().unwrap(), key, pipeline);
        }
        self.programs
            .iter_mut()
            .find(|s| s.id == reload.program.id())
            .unwrap()
            .preview = matches!(
            reload.policy,
            crate::shader_compiler::CompilePolicy::Preview
                | crate::shader_compiler::CompilePolicy::PreviewHelpers
                | crate::shader_compiler::CompilePolicy::PreviewNoInline
                | crate::shader_compiler::CompilePolicy::LibraryFast
        );
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "CPU profiling, writes RENDERLAB_FRONTEND_PROFILE; no GPU or window"]
    fn cpu_program_frontend_profile() {
        let mut rows = vec![];
        let salt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        for kind in ["standard", "user"] {
            for repeat in 0..5 {
                let source = STANDARD_PROGRAM.replace(
                    "return exposed;",
                    &format!(
                        "return exposed*vec3<f32>({:.9},0.9,1.0);",
                        0.5 + f64::from(salt.wrapping_add(repeat * 1_000_003))
                            / f64::from(u32::MAX)
                            * 0.1
                    ),
                );
                let source = if kind == "user" {
                    source.replace("0.9,1.0", "0.8,1.0")
                } else {
                    source
                };
                let start = Instant::now();
                let composed = program::compose(
                    include_str!("../shaders/scene.wgsl"),
                    &source,
                    Stage::Raster,
                    false,
                );
                let compose = start.elapsed();
                let start = Instant::now();
                let module = naga::front::wgsl::parse_str(&composed.source).unwrap();
                let parse = start.elapsed();
                let start = Instant::now();
                let info = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap();
                let validate = start.elapsed();
                let start = Instant::now();
                let words = naga::back::spv::write_vec(
                    &module,
                    &info,
                    &naga::back::spv::Options {
                        lang_version: (1, 6),
                        flags: naga::back::spv::WriterFlags::LABEL_VARYINGS
                            | naga::back::spv::WriterFlags::FORCE_POINT_SIZE,
                        ..Default::default()
                    },
                    None,
                )
                .unwrap();
                let emit = start.elapsed();
                rows.push(serde_json::json!({"kind":kind,"repeat":repeat,"compose_ms":compose.as_secs_f64()*1e3,
                    "parse_ms":parse.as_secs_f64()*1e3,"validate_ms":validate.as_secs_f64()*1e3,
                    "spirv_ms":emit.as_secs_f64()*1e3,"spirv_words":words.len(),"source_sha256":format!("{:x}",Sha256::digest(composed.source.as_bytes()))}));
            }
        }
        std::fs::write(
            std::env::var("RENDERLAB_FRONTEND_PROFILE").unwrap(),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
}
