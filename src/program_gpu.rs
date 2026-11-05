//! Shading programs on the GPU: one pipeline set per program, compiled against the
//! active prelude compositions, with a reload transaction per program and per-material
//! parameter blocks. Engine kernels stay in `Pipelines`; a program edit recompiles only
//! its own shading pipelines.
use super::*;
use crate::program::{
    self, Composed, MaterialParameters, ParameterLayout, ProgramShared, STANDARD_PROGRAM, Stage,
};
use std::sync::{Mutex, Weak};

/// One program's pipelines for every active path. Raster pairs are indexed by tangent
/// stream, the traced pair by `Shading`.
pub(crate) struct ProgramPipelines {
    pub lit: [Owned<wgpu::RenderPipeline>; 2],
    pub hardware: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    pub fresh: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    /// Present exactly while traced primary visibility is selected.
    pub traced: Option<[Owned<wgpu::ComputePipeline>; 2]>,
}
impl ProgramPipelines {
    pub fn count(&self) -> u64 {
        2 + 2 * [
            self.hardware.is_some(),
            self.fresh.is_some(),
            self.traced.is_some(),
        ]
        .into_iter()
        .map(u64::from)
        .sum::<u64>()
    }
}

pub(super) struct ProgramSlot {
    pub id: u64,
    /// The renderer keeps the standard program's handle; other slots retire when theirs
    /// drop (materials hold theirs).
    pub handle: Weak<ProgramShared>,
    pub label: String,
    pub source: String,
    pub layout: Option<ParameterLayout>,
    pub pipelines: ProgramPipelines,
    /// Live materials' parameter blocks, repacked when a reload changes the layout.
    pub materials: Vec<Weak<MaterialParameters>>,
}

/// The prelude compositions programs compile against: the raster prelude, its
/// hardware-shadow and fresh variants, and the traced shading heads.
pub(super) struct Preludes<'a> {
    pub raster: &'a str,
    pub hardware: Option<&'a str>,
    pub fresh: Option<&'a str>,
    pub traced: Option<&'a [String; 2]>,
}

/// The pipeline layouts each program variant compiles on.
pub(super) struct ProgramAbi<'a> {
    pub device: &'a wgpu::Device,
    pub ledger: &'a Ledger,
    pub samples: SampleCount,
    pub lit: &'a wgpu::PipelineLayout,
    pub hardware: Option<&'a wgpu::PipelineLayout>,
    pub fresh: Option<&'a FreshState>,
    pub primary: Option<&'a PrimaryState>,
}

pub(super) struct Compiled {
    pub pipelines: ProgramPipelines,
    pub layout: Option<ParameterLayout>,
}

fn module(abi: &ProgramAbi<'_>, label: &str, composed: &Composed) -> Owned<wgpu::ShaderModule> {
    abi.ledger.own(
        abi.device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(composed.source.as_str().into()),
            }),
        ResourceKind::ShaderModule,
    )
}
fn scope_error(label: &str, error: wgpu::Error, traced: &[Composed]) -> Error {
    match error {
        wgpu::Error::Validation { .. } => traced
            .iter()
            .find_map(|c| program::traced_stage_error(label, c))
            .unwrap_or_else(|| Error::Shader(format!("program `{label}`: {error}"))),
        _ => Error::Fault(Fault(error.to_string())),
    }
}

/// Composes, checks and compiles one program for every path `abi` and `preludes` hold,
/// in its own error scope. Rejections are `Error::Shader` naming the program and, where
/// naga locates them, its lines; other device errors are `Error::Fault`.
pub(super) fn compile_program(
    abi: &ProgramAbi<'_>,
    preludes: &Preludes<'_>,
    label: &str,
    source: &str,
) -> Result<Compiled, Error> {
    let parameters = program::declares_parameters(source);
    let raster = program::compose(preludes.raster, source, Stage::Raster, parameters);
    let layout = program::analyse(label, source, &raster)?.parameters;
    if layout.is_some() != parameters {
        return Err(Error::Shader(format!(
            "program `{label}`: struct Parameters must be declared at module scope"
        )));
    }
    let traced_sources: Vec<Composed> = abi
        .primary
        .and(preludes.traced)
        .into_iter()
        .flatten()
        .map(|head| program::compose(head, source, Stage::Traced, parameters))
        .collect();
    let scopes = OperationScopes::new(abi.device);
    let pair = |m: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| {
        [false, true].map(|tangent| {
            mesh_pipeline(
                abi.device,
                abi.ledger,
                m,
                layout,
                wgpu::TextureFormat::Rgba16Float,
                "fs_lit",
                tangent,
                abi.samples,
            )
        })
    };
    let lit = pair(&module(abi, label, &raster), abi.lit);
    let raster_variant = |head: &str| {
        module(
            abi,
            label,
            &program::compose(head, source, Stage::Raster, parameters),
        )
    };
    let hardware = abi
        .hardware
        .zip(preludes.hardware)
        .map(|(layout, head)| pair(&raster_variant(head), layout));
    let fresh = abi
        .fresh
        .zip(preludes.fresh)
        .map(|(state, head)| state.shade_pipelines(abi.device, abi.ledger, &raster_variant(head)));
    let traced = abi
        .primary
        .filter(|_| !traced_sources.is_empty())
        .map(|state| {
            let [hard, soft] = [0, 1].map(|i| module(abi, label, &traced_sources[i]));
            state.shade_pipelines(abi.device, abi.ledger, [&hard, &soft])
        });
    if let Some(error) = scopes.finish() {
        return Err(scope_error(label, error, &traced_sources));
    }
    Ok(Compiled {
        pipelines: ProgramPipelines {
            lit,
            hardware,
            fresh,
            traced,
        },
        layout,
    })
}

/// The standard program's handle and slot for a new renderer, whose ids start at 0.
pub(super) fn standard_slot(
    identity: &Arc<()>,
    compiled: Compiled,
) -> (crate::Program, ProgramSlot) {
    let handle = crate::Program(Arc::new(ProgramShared {
        id: 0,
        renderer: identity.clone(),
        label: "standard".into(),
    }));
    let slot = ProgramSlot {
        id: 0,
        handle: Arc::downgrade(&handle.0),
        label: "standard".into(),
        source: STANDARD_PROGRAM.into(),
        layout: compiled.layout,
        pipelines: compiled.pipelines,
        materials: vec![],
    };
    (handle, slot)
}

impl Renderer {
    /// The built-in photoreal program, shaders/programs/standard.wgsl, which materials
    /// without a program use. It reloads like any program.
    pub fn standard_program(&self) -> &crate::Program {
        &self.standard
    }

    /// Composes and compiles `d` for every active path now (raster lit, hardware and
    /// fresh shading where this renderer has them, traced shading while traced is
    /// selected). Rejections are `Error::Shader` naming program lines; nothing else
    /// changes.
    pub fn try_create_program(
        &mut self,
        d: &crate::ProgramDescriptor<'_>,
    ) -> Result<crate::Program, Error> {
        self.check_fault()?;
        self.purge_programs();
        self.pipelines.attempts += 1;
        let compiled =
            match compile_program(&self.program_abi(), &self.preludes(), d.label, d.source) {
                Err(Error::Fault(Fault(message))) => return Err(self.fail(message)),
                other => other?,
            };
        self.check_fault()?;
        self.pipelines.successes += 1;
        self.pipelines.installations += 1;
        self.pipelines.created += compiled.pipelines.count();
        let handle = crate::Program(Arc::new(ProgramShared {
            id: self.next_program,
            renderer: self.identity.clone(),
            label: d.label.into(),
        }));
        self.next_program += 1;
        self.programs.push(ProgramSlot {
            id: handle.id(),
            handle: Arc::downgrade(&handle.0),
            label: d.label.into(),
            source: d.source.into(),
            layout: compiled.layout,
            pipelines: compiled.pipelines,
            materials: vec![],
        });
        self.update_hashes();
        Ok(handle)
    }

    /// One transaction over `p`'s pipelines: on rejection its last working pipelines keep
    /// rendering, and other programs are never recompiled. Existing materials are
    /// repacked by member name; new or retyped members become zero and are listed in
    /// the report.
    pub fn reload_program(
        &mut self,
        p: &crate::Program,
        source: &str,
    ) -> Result<ReloadReport, ReloadError> {
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        let start = Instant::now();
        let rejected = |error: Error| ReloadError::Rejected {
            diagnostic: error.to_string(),
            compile: start.elapsed(),
        };
        if !Arc::ptr_eq(&p.0.renderer, &self.identity) {
            return Err(rejected(Error::InvalidInput(
                "program belongs to another renderer",
            )));
        }
        self.pipelines.attempts += 1;
        let compiled =
            match compile_program(&self.program_abi(), &self.preludes(), &p.0.label, source) {
                Ok(compiled) => compiled,
                Err(Error::Fault(Fault(message))) => {
                    let _ = self.fault.set(Fault(message));
                    return Err(ReloadError::Fault(self.fault.get().unwrap().clone()));
                }
                Err(error) => return Err(rejected(error)),
            };
        let compile = start.elapsed();
        if let Some(fault) = self.fault.get() {
            return Err(ReloadError::Fault(fault.clone()));
        }
        self.pipelines.successes += 1;
        self.pipelines.installations += 1;
        self.pipelines.created += compiled.pipelines.count();
        let slot = self.programs.iter_mut().find(|s| s.id == p.id()).unwrap();
        let zeroed = ParameterLayout::changed(slot.layout.as_ref(), compiled.layout.as_ref());
        let repack = slot.layout != compiled.layout;
        slot.pipelines = compiled.pipelines;
        slot.layout = compiled.layout;
        source.clone_into(&mut slot.source);
        slot.materials.retain(|m| m.strong_count() > 0);
        if repack {
            for parameters in slot.materials.iter().filter_map(Weak::upgrade) {
                let mut values = parameters.values.lock().unwrap();
                values.retain(|(name, value)| {
                    ParameterLayout::check(slot.layout.as_ref(), [(name.as_str(), *value)]).is_ok()
                });
                let block = ParameterLayout::pack(slot.layout.as_ref(), &values);
                self.queue.write_buffer(&parameters.buffer, 0, &block);
                self.uploads.materials += block.len() as u64;
                self.uploads.write_calls += 1;
            }
        }
        self.update_hashes();
        self.revision.0 += 1;
        Ok(ReloadReport {
            revision: self.revision,
            compile,
            zeroed_parameters: zeroed,
        })
    }

    /// Sets named `Parameters` values of `m`, keeping its others: a buffer write, no
    /// compile. Unknown names and wrong types are rejected before any write.
    pub fn try_update_parameters(
        &mut self,
        m: &Material,
        values: &[(&str, crate::Value)],
    ) -> Result<(), Error> {
        self.check_fault()?;
        if !Arc::ptr_eq(&m.renderer, &self.identity) {
            return Err(Error::InvalidInput("material belongs to another renderer"));
        }
        let layout = self.program_slot(m.program.id()).layout.as_ref();
        ParameterLayout::check(layout, values.iter().copied())?;
        let mut stored = m.parameters.values.lock().unwrap();
        for &(name, value) in values {
            match stored.iter_mut().find(|(n, _)| n == name) {
                Some(entry) => entry.1 = value,
                None => stored.push((name.into(), value)),
            }
        }
        let block = ParameterLayout::pack(layout, &stored);
        self.queue.write_buffer(&m.parameters.buffer, 0, &block);
        self.uploads.materials += block.len() as u64;
        self.uploads.write_calls += 1;
        Ok(())
    }

    /// A new material's checked parameter block for `program`, registered for repacking.
    /// Blocks start zeroed, so only nonempty values are written.
    pub(super) fn material_parameters(
        &mut self,
        program: &crate::Program,
        values: &[(&str, crate::Value)],
    ) -> Arc<MaterialParameters> {
        let buffer = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("material program parameters"),
                size: program::PARAMETER_BYTES as u64,
                // Traced shading reads every draw's block from a storage binding array.
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_DST
                    | if self.primary.is_some() {
                        wgpu::BufferUsages::STORAGE
                    } else {
                        wgpu::BufferUsages::empty()
                    },
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let values: Vec<_> = values.iter().map(|&(n, v)| (n.to_owned(), v)).collect();
        let slot = self
            .programs
            .iter_mut()
            .find(|s| s.id == program.id())
            .unwrap();
        if !values.is_empty() {
            let block = ParameterLayout::pack(slot.layout.as_ref(), &values);
            self.queue.write_buffer(&buffer, 0, &block);
            self.uploads.materials += block.len() as u64;
            self.uploads.write_calls += 1;
        }
        let parameters = Arc::new(MaterialParameters {
            buffer,
            values: Mutex::new(values),
        });
        slot.materials.retain(|m| m.strong_count() > 0);
        slot.materials.push(Arc::downgrade(&parameters));
        parameters
    }

    /// Checks a material descriptor's program and parameters against this renderer.
    pub(super) fn validate_program(&self, data: &MaterialDescriptor<'_>) -> Result<(), Error> {
        let program = data.program.unwrap_or(&self.standard);
        if !Arc::ptr_eq(&program.0.renderer, &self.identity) {
            return Err(Error::InvalidInput("program belongs to another renderer"));
        }
        ParameterLayout::check(
            self.program_slot(program.id()).layout.as_ref(),
            data.parameters.iter().copied(),
        )
    }

    pub(super) fn program_slot(&self, id: u64) -> &ProgramSlot {
        self.programs.iter().find(|s| s.id == id).unwrap()
    }

    /// Every program's traced shading pipelines on `heads`, in slot order.
    pub(super) fn compile_traced_programs(
        &self,
        heads: &[String; 2],
    ) -> Result<Vec<[Owned<wgpu::ComputePipeline>; 2]>, Error> {
        let state = self.primary.as_ref().unwrap();
        let mut out = vec![];
        for slot in &self.programs {
            let parameters = program::declares_parameters(&slot.source);
            let sources = heads
                .each_ref()
                .map(|h| program::compose(h, &slot.source, Stage::Traced, parameters));
            let abi = self.program_abi();
            let scopes = OperationScopes::new(&self.device);
            let [hard, soft] = sources.each_ref().map(|c| module(&abi, &slot.label, c));
            let pipelines = state.shade_pipelines(&self.device, &self.ledger, [&hard, &soft]);
            if let Some(error) = scopes.finish() {
                return Err(match scope_error(&slot.label, error, &sources) {
                    Error::Fault(Fault(message)) => self.fail(message),
                    other => other,
                });
            }
            out.push(pipelines);
        }
        Ok(out)
    }

    pub(super) fn program_abi(&self) -> ProgramAbi<'_> {
        ProgramAbi {
            device: &self.device,
            ledger: &self.ledger,
            samples: self.options.samples,
            lit: &self.lighting.layout,
            hardware: self.hardware_shadows.as_ref().map(|h| &*h.layout),
            fresh: self.fresh.as_ref(),
            primary: self
                .primary
                .as_ref()
                .filter(|_| self.pipeline.primary.is_some()),
        }
    }
    pub(super) fn preludes(&self) -> Preludes<'_> {
        Preludes {
            raster: &self.source,
            hardware: self.hardware_source.as_deref(),
            fresh: self.fresh_source.as_deref(),
            traced: self.primary_sources.as_ref().map(|(_, heads)| heads),
        }
    }

    /// Releases the pipelines of programs whose every handle has dropped.
    pub(super) fn purge_programs(&mut self) {
        self.programs.retain(|s| s.handle.strong_count() > 0);
    }

    /// Identity of the modules writing HDR images: the prelude and every program's
    /// source, plus the traced kernels while traced is selected.
    pub(super) fn update_hashes(&mut self) {
        let mut hash = Sha256::new().chain_update(self.source.as_bytes());
        for slot in &self.programs {
            hash.update(slot.id.to_ne_bytes());
            hash.update(slot.source.as_bytes());
        }
        if let Some((kernels, _)) = &self.primary_sources {
            self.primary_hash = hash
                .clone()
                .chain_update(primary_ray_gpu::sources_hash(kernels))
                .finalize()
                .into();
        }
        self.shader_hash = hash.finalize().into();
    }
}
