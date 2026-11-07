//! Owned compile recipes let the render thread keep submitting the previous revision.
//! Only hot sets are captured. Their four bind-group layouts are the existing explicit
//! program ABI, with no immediate data; shader bodies and dispatches are unchanged.
use super::*;
use std::thread::JoinHandle;

struct Recipe {
    variant: Variant,
    layouts: Vec<Owned<wgpu::PipelineLayout>>,
}

struct Prepared {
    compiled: Compiled,
    variants: Vec<Variant>,
    built: Vec<Vec<Built>>,
    compile: Duration,
}

pub(crate) struct ProgramReload {
    program: crate::Program,
    source: String,
    revision: Revision,
    primary_hash: [u8; 32],
    variants: Vec<Variant>,
    started: Instant,
    worker: JoinHandle<Result<Prepared, Error>>,
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
    }
}

impl Renderer {
    pub(crate) fn start_program_reload(
        &mut self,
        program: &crate::Program,
        source: &str,
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
                            Variant::Traced => slot.pipelines.traced.as_ref().unwrap()[index]
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
        let label = program.label().to_owned();
        let text = source.to_owned();
        let started = Instant::now();
        let fault = self.fault.clone();
        let worker = std::thread::spawn(move || {
            let result = (|| {
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
                let module_time = started.elapsed();
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
                                match recipe.variant {
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
                                    Variant::Traced | Variant::Temporal => {
                                        let module = if recipe.variant == Variant::Temporal {
                                            m.temporal.as_ref().unwrap()
                                        } else {
                                            &m.traced.as_ref().unwrap()[index]
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
                                }
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
                let compile = started.elapsed();
                if std::env::var_os("RENDERLAB_RELOAD_PROFILE").is_some() {
                    eprintln!(
                        "program_reload_profile modules_ms={:.3} pipelines_ms={:.3} pipelines={} parallel=true",
                        module_time.as_secs_f64() * 1e3,
                        (compile - module_time).as_secs_f64() * 1e3,
                        pipeline_count(&built)
                    );
                }
                Ok(Prepared {
                    compiled,
                    variants: recipes.iter().map(|r| r.variant).collect(),
                    built,
                    compile,
                })
            })();
            if let Err(Error::Fault(error)) = &result {
                let _ = fault.set(error.clone());
            }
            result
        });
        Ok(ProgramReload {
            program: program.clone(),
            source: source.into(),
            revision: self.revision,
            primary_hash: self.primary_hash,
            variants,
            started,
            worker,
        })
    }

    /// Called only for a finished, current job, between frames. No worker can mutate
    /// slots, parameter buffers, source identities or the active revision.
    pub(crate) fn finish_program_reload(
        &mut self,
        reload: ProgramReload,
    ) -> Result<ReloadReport, ReloadError> {
        let prepared = reload.worker.join().unwrap_or_else(|_| {
            Err(Error::Fault(Fault(
                "program compile worker panicked".into(),
            )))
        });
        let Prepared {
            compiled,
            variants,
            built,
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
                    compile: reload.started.elapsed(),
                });
            }
        };
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        Ok(self.install_program(
            &reload.program,
            &reload.source,
            compiled,
            variants,
            built,
            compile,
        ))
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
