//! Shading programs on the GPU: per program, a module for every active path, composed
//! against the active prelude compositions, and pipeline sets created from them on first
//! use. A reload transaction per program (or over everything, for the prelude) creates
//! every path's module, so naga validates the edit against each, then only the sets the
//! last frame drew with; the rest compile when a frame first needs them. Pipelines of
//! one transaction or frame are created concurrently, one thread each, since the driver
//! compiles on the creating thread. Engine kernels stay in `Pipelines`. Per-material
//! parameter blocks live here too.
use super::*;
use crate::program::{
    self, Composed, MaterialParameters, ParameterLayout, ProgramShared, STANDARD_PROGRAM, Stage,
};
use std::sync::{Mutex, Weak};

/// A program pipeline set a frame shades with. Raster sets are pairs indexed by tangent
/// stream; the traced set is indexed by `Shading`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// Lit raster frames without shadows, or with light-space shadow maps.
    Lit,
    /// Raster frames with hardware hard shadows.
    Hardware,
    /// Raster frames with fresh soft visibility counts.
    Fresh,
    /// Traced primary visibility: Hard and Soft shading.
    Traced,
}

/// One program's pipeline sets, each present once a frame needed it or a reload
/// recompiled it because the last frame drew with it.
#[derive(Default)]
pub(crate) struct ProgramPipelines {
    pub lit: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    pub hardware: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    pub fresh: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    pub traced: Option<[Owned<wgpu::ComputePipeline>; 2]>,
}
impl ProgramPipelines {
    pub fn has(&self, variant: Variant) -> bool {
        match variant {
            Variant::Lit => self.lit.is_some(),
            Variant::Hardware => self.hardware.is_some(),
            Variant::Fresh => self.fresh.is_some(),
            Variant::Traced => self.traced.is_some(),
        }
    }
    pub(super) fn install(&mut self, variant: Variant, pair: [Built; 2]) {
        let render = |pair: [Built; 2]| {
            Some(pair.map(|b| match b {
                Built::Render(p) => p,
                Built::Compute(_) => unreachable!("raster sets hold render pipelines"),
            }))
        };
        match variant {
            Variant::Lit => self.lit = render(pair),
            Variant::Hardware => self.hardware = render(pair),
            Variant::Fresh => self.fresh = render(pair),
            Variant::Traced => {
                self.traced = Some(pair.map(|b| match b {
                    Built::Compute(p) => p,
                    Built::Render(_) => unreachable!("the traced set holds compute pipelines"),
                }))
            }
        }
    }
}

/// The driver cache program pipelines go through, present when the device enables
/// `Features::PIPELINE_CACHE`: recompiling a source seen before in this process (a revert,
/// an A/B toggle) is then a cache hit instead of a driver compile. Once `LIMIT` pipelines
/// have gone through it, the next transaction replaces it, dropping that history, which
/// bounds its memory over a long editing session.
pub(super) struct ProgramCache {
    cache: Option<wgpu::PipelineCache>,
    pipelines: u64,
}
impl ProgramCache {
    /// About 93 KB of driver data per lit pipeline was measured (fast-shader-reload.md).
    const LIMIT: u64 = 512;
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            cache: device
                .features()
                .contains(wgpu::Features::PIPELINE_CACHE)
                .then(|| Self::create(device)),
            pipelines: 0,
        }
    }
    fn create(device: &wgpu::Device) -> wgpu::PipelineCache {
        // SAFETY: no initial data is supplied, so nothing unvalidated reaches the driver.
        unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("program pipelines"),
                data: None,
                fallback: true,
            })
        }
    }
    /// Replaces a full cache before the next pipelines go through it.
    pub fn refresh(&mut self, device: &wgpu::Device) {
        if self.cache.is_some() && self.pipelines >= Self::LIMIT {
            self.cache = Some(Self::create(device));
            self.pipelines = 0;
        }
    }
    pub fn count(&mut self, pipelines: u64) {
        if self.cache.is_some() {
            self.pipelines += pipelines;
        }
    }
    pub fn get(&self) -> Option<&wgpu::PipelineCache> {
        self.cache.as_ref()
    }
}

/// One created pipeline of a set.
pub(super) enum Built {
    Render(Owned<wgpu::RenderPipeline>),
    Compute(Owned<wgpu::ComputePipeline>),
}

/// One program's modules for every path the renderer has, created (and so validated by
/// naga against each composition) inside its creation or reload transaction.
pub(super) struct ProgramModules {
    lit: Owned<wgpu::ShaderModule>,
    hardware: Option<Owned<wgpu::ShaderModule>>,
    fresh: Option<Owned<wgpu::ShaderModule>>,
    /// Hard and Soft shading, present exactly while traced primary visibility is selected.
    pub traced: Option<[Owned<wgpu::ShaderModule>; 2]>,
}
impl ProgramModules {
    pub fn has(&self, variant: Variant) -> bool {
        match variant {
            Variant::Lit => true,
            Variant::Hardware => self.hardware.is_some(),
            Variant::Fresh => self.fresh.is_some(),
            Variant::Traced => self.traced.is_some(),
        }
    }
    /// Pipeline `index` of `variant`: the tangent stream, or `Shading` for traced.
    fn pipeline(&self, abi: &ProgramAbi<'_>, variant: Variant, index: usize) -> Built {
        let lit = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| {
            Built::Render(mesh_pipeline(
                abi.device,
                abi.ledger,
                module,
                layout,
                wgpu::TextureFormat::Rgba16Float,
                "fs_lit",
                index == 1,
                abi.samples,
                abi.cache,
            ))
        };
        match variant {
            Variant::Lit => lit(&self.lit, abi.lit),
            Variant::Hardware => lit(self.hardware.as_ref().unwrap(), abi.hardware.unwrap()),
            Variant::Fresh => Built::Render(abi.fresh.unwrap().shade_pipeline(
                abi.device,
                abi.ledger,
                self.fresh.as_ref().unwrap(),
                index == 1,
                abi.cache,
            )),
            Variant::Traced => Built::Compute(abi.primary.unwrap().shade_pipeline(
                abi.device,
                abi.ledger,
                &self.traced.as_ref().unwrap()[index],
                index,
                abi.cache,
            )),
        }
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
    pub modules: ProgramModules,
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
    pub cache: Option<&'a wgpu::PipelineCache>,
}

pub(super) struct Compiled {
    pub modules: ProgramModules,
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

/// Composes, checks and creates one program's module for every path `abi` and `preludes`
/// hold, in one error scope; pipelines come later (`create_sets`). Rejections are
/// `Error::Shader` naming the program and, where naga locates them, its lines; other
/// device errors are `Error::Fault`.
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
    let raster_variant = |head: &str| {
        module(
            abi,
            label,
            &program::compose(head, source, Stage::Raster, parameters),
        )
    };
    let modules = ProgramModules {
        lit: module(abi, label, &raster),
        hardware: abi.hardware.and(preludes.hardware).map(raster_variant),
        fresh: abi.fresh.and(preludes.fresh).map(raster_variant),
        traced: (!traced_sources.is_empty())
            .then(|| [0, 1].map(|i| module(abi, label, &traced_sources[i]))),
    };
    if let Some(error) = scopes.finish() {
        return Err(scope_error(label, error, &traced_sources));
    }
    Ok(Compiled { modules, layout })
}

/// Runs `jobs` concurrently, one scoped thread each, and returns their values in order,
/// each with the first error its own scopes captured (wgpu error scopes are per thread).
pub(super) fn in_parallel<T: Send, F: FnOnce() -> T + Send>(
    device: &wgpu::Device,
    jobs: impl IntoIterator<Item = F>,
) -> Vec<(T, Option<wgpu::Error>)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .into_iter()
            .map(|job| {
                s.spawn(move || {
                    let scopes = OperationScopes::new(device);
                    let value = job();
                    (value, scopes.finish())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// Creates each requested `(modules, label, variant)` set, every pipeline concurrently.
/// A failure names the first failing program in request order: `Error::Shader` for
/// validation, `Error::Fault` otherwise.
pub(super) fn create_sets(
    abi: &ProgramAbi<'_>,
    sets: &[(&ProgramModules, &str, Variant)],
) -> Result<Vec<[Built; 2]>, Error> {
    let jobs = sets.iter().flat_map(|&(modules, _, variant)| {
        [0, 1].map(move |index| move || modules.pipeline(abi, variant, index))
    });
    let mut built = in_parallel(abi.device, jobs).into_iter();
    let mut out = Vec::with_capacity(sets.len());
    let mut failure = None;
    for &(_, label, variant) in sets {
        let [(a, ea), (b, eb)] = [built.next().unwrap(), built.next().unwrap()];
        if let Some(error) = ea.or(eb).filter(|_| failure.is_none()) {
            failure = Some(match error {
                wgpu::Error::Validation { .. } => {
                    Error::Shader(format!("program `{label}`, {variant:?} pipelines: {error}"))
                }
                _ => Error::Fault(Fault(error.to_string())),
            });
        }
        out.push([a, b]);
    }
    failure.map_or(Ok(out), Err)
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
        modules: compiled.modules,
        pipelines: ProgramPipelines::default(),
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

    /// Composes `d` and creates its module for every active path now (raster lit,
    /// hardware and fresh shading where this renderer has them, traced shading while
    /// traced is selected), so naga checks it against each; its pipelines compile when a
    /// frame first draws with it. Rejections are `Error::Shader` naming program lines;
    /// nothing else changes.
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
            modules: compiled.modules,
            pipelines: ProgramPipelines::default(),
            materials: vec![],
        });
        self.update_hashes();
        Ok(handle)
    }

    /// One transaction over `p`: every path's module is created (naga checks the edit
    /// against each), then the pipeline sets the last frame drew with `p` are created
    /// concurrently; on rejection its last working pipelines keep rendering. Its other
    /// sets compile when a frame first needs them, and other programs are never
    /// recompiled. Existing materials are repacked by member name; new or retyped members
    /// become zero and are listed in the report.
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
        self.program_cache.refresh(&self.device);
        let abi = self.program_abi();
        let built = compile_program(&abi, &self.preludes(), &p.0.label, source).and_then(|c| {
            let sets: Vec<_> = self
                .hot_sets(p.id(), &c.modules)
                .map(|v| (&c.modules, p.0.label.as_str(), v))
                .collect();
            let built = create_sets(&abi, &sets)?;
            let variants: Vec<_> = sets.iter().map(|s| s.2).collect();
            Ok((c, variants, built))
        });
        let (compiled, variants, built) = match built {
            Ok(built) => built,
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
        self.pipelines.created += 2 * built.len() as u64;
        self.program_cache.count(2 * built.len() as u64);
        let slot = self.programs.iter_mut().find(|s| s.id == p.id()).unwrap();
        let zeroed = ParameterLayout::changed(slot.layout.as_ref(), compiled.layout.as_ref());
        let repack = slot.layout != compiled.layout;
        slot.modules = compiled.modules;
        slot.pipelines = ProgramPipelines::default();
        for (variant, pair) in variants.into_iter().zip(built) {
            slot.pipelines.install(variant, pair);
        }
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

    /// The sets of program `id` the last frame drew with that `modules` can build: what a
    /// reload recreates eagerly.
    pub(super) fn hot_sets<'a>(
        &'a self,
        id: u64,
        modules: &'a ProgramModules,
    ) -> impl Iterator<Item = Variant> + 'a {
        self.hot
            .iter()
            .filter(move |&&(program, variant)| program == id && modules.has(variant))
            .map(|&(_, variant)| variant)
    }

    /// The sets a frame draws with: `variant` (None for unlit, diagnostic frames) of each
    /// program its draws' materials use (none: the standard program). A traced frame
    /// shades with its first draw's program (admission rejects mixed ones); raster frames
    /// also bind the standard program first.
    pub(super) fn frame_sets<'a>(
        &self,
        draws: impl IntoIterator<Item = Option<&'a Material>>,
        variant: Option<Variant>,
    ) -> Vec<(u64, Variant)> {
        let Some(variant) = variant else {
            return vec![];
        };
        let standard = self.standard.id();
        let mut programs = draws
            .into_iter()
            .map(|m| m.map_or(standard, |m| m.program.id()));
        let mut programs: Vec<u64> = if variant == Variant::Traced {
            vec![programs.next().unwrap_or(standard)]
        } else {
            std::iter::once(standard).chain(programs).collect()
        };
        programs.sort_unstable();
        programs.dedup();
        programs.into_iter().map(|id| (id, variant)).collect()
    }

    /// Creates the sets in `sets` that their programs lack, every pipeline concurrently,
    /// after the frame's admission checks and before its uploads, and records `sets` as
    /// the hot sets reloads recreate. A failure rejects the frame with `Error::Shader`
    /// (or faults).
    pub(super) fn ensure_pipelines(&mut self, sets: Vec<(u64, Variant)>) -> Result<(), Error> {
        let missing: Vec<_> = sets
            .iter()
            .copied()
            .filter(|&(id, variant)| !self.program_slot(id).pipelines.has(variant))
            .collect();
        if !missing.is_empty() {
            self.program_cache.refresh(&self.device);
            let abi = self.program_abi();
            let requests: Vec<_> = missing
                .iter()
                .map(|&(id, variant)| {
                    let slot = self.program_slot(id);
                    (&slot.modules, slot.label.as_str(), variant)
                })
                .collect();
            let built = match create_sets(&abi, &requests) {
                Err(Error::Fault(Fault(message))) => return Err(self.fail(message)),
                other => other?,
            };
            self.pipelines.created += 2 * built.len() as u64;
            self.program_cache.count(2 * built.len() as u64);
            for (&(id, variant), pair) in missing.iter().zip(built) {
                let slot = self.programs.iter_mut().find(|s| s.id == id).unwrap();
                slot.pipelines.install(variant, pair);
            }
            self.check_fault()?;
        }
        self.hot = sets;
        Ok(())
    }

    /// Every set the standard program's modules can build, for tests that swap its
    /// pipelines for probes and references.
    #[cfg(test)]
    pub(super) fn ensure_standard_pipelines(&mut self) {
        let hot = std::mem::take(&mut self.hot);
        let sets = [
            Variant::Lit,
            Variant::Hardware,
            Variant::Fresh,
            Variant::Traced,
        ]
        .into_iter()
        .filter(|&v| self.programs[0].modules.has(v))
        .map(|v| (self.standard.id(), v))
        .collect();
        self.ensure_pipelines(sets).unwrap();
        self.hot = hot;
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

    /// Every program's traced Hard and Soft shading modules on `heads`, in slot order;
    /// their pipelines compile when a traced frame first draws with the program.
    pub(super) fn compile_traced_modules(
        &self,
        heads: &[String; 2],
    ) -> Result<Vec<[Owned<wgpu::ShaderModule>; 2]>, Error> {
        let abi = self.program_abi();
        let mut out = vec![];
        for slot in &self.programs {
            let parameters = program::declares_parameters(&slot.source);
            let sources = heads
                .each_ref()
                .map(|h| program::compose(h, &slot.source, Stage::Traced, parameters));
            let scopes = OperationScopes::new(&self.device);
            let modules = sources.each_ref().map(|c| module(&abi, &slot.label, c));
            if let Some(error) = scopes.finish() {
                return Err(match scope_error(&slot.label, error, &sources) {
                    Error::Fault(Fault(message)) => self.fail(message),
                    other => other,
                });
            }
            out.push(modules);
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
            cache: self.program_cache.get(),
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

#[cfg(test)]
mod compile_profile {
    use super::*;
    use std::{fs, path::PathBuf, time::SystemTime};

    /// Time every production shading pipeline separately, including Fresh and traced
    /// pairs. Each pipeline gets a different live tint, salted per process, so neither
    /// an earlier process nor the other member of a pair can supply its driver compile.
    #[test]
    #[ignore = "offscreen; GPU slot, >=6 GiB available RAM, experimental acknowledgment and RENDERLAB_PRIMARY_MANIFEST"]
    fn gpu_program_pipeline_compile_profile() {
        assert_eq!(
            std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
            Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
        )
        .unwrap();
        let source = manifest["prelude"].as_str().map_or_else(
            || include_str!("../shaders/scene.wgsl").to_owned(),
            |path| fs::read_to_string(path).unwrap(),
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::empty(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | crate::TRACED_FEATURES
                | wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            required_limits: crate::traced_limits(wgpu::Limits {
                max_blas_primitive_count: 2_000_000,
                max_blas_geometry_count: 1,
                max_tlas_instance_count: 4096,
                max_acceleration_structures_per_shader_stage: 1,
                ..Default::default()
            }),
            // The test caller explicitly accepts wgpu's experimental device contract.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        }))
        .unwrap();
        let mut renderer = Renderer::new_with_hardware_shadows(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            Limits::default(),
            RendererOptions {
                samples: SampleCount::One,
                anisotropy_clamp: 16,
            },
            HardwareShadowOptions::default(),
            &source,
        )
        .unwrap();
        renderer
            .set_primary_visibility(PrimaryVisibility::Traced)
            .unwrap();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        let salt = now.subsec_nanos() ^ (now.as_secs() as u32) ^ std::process::id();
        assert_eq!(STANDARD_PROGRAM.matches("*sampled.rgb*TINT").count(), 1);
        let mut serial = 0u32;
        let mut rows = vec![];
        for rep in 0..manifest["reps"].as_u64().unwrap_or(3) {
            for variant in [
                Variant::Lit,
                Variant::Hardware,
                Variant::Fresh,
                Variant::Traced,
            ] {
                for index in 0..2 {
                    serial += 1;
                    let tint =
                        f32::from_bits(0x3f000000 | (salt.wrapping_add(serial) & 0x003fffff));
                    let program = STANDARD_PROGRAM
                        .replace("*sampled.rgb*TINT", &format!("*sampled.rgb*TINT*{tint:.9}"));
                    let abi = renderer.program_abi();
                    let start = Instant::now();
                    let compiled =
                        compile_program(&abi, &renderer.preludes(), "profile", &program).unwrap();
                    let module_ms = start.elapsed().as_secs_f64() * 1e3;
                    let scopes = OperationScopes::new(&renderer.device);
                    let start = Instant::now();
                    let pipeline = compiled.modules.pipeline(&abi, variant, index);
                    let pipeline_ms = start.elapsed().as_secs_f64() * 1e3;
                    assert!(scopes.finish().is_none(), "{variant:?} {index}");
                    drop(pipeline);
                    let row = serde_json::json!({"rep": rep, "variant": format!("{variant:?}"),
                        "index": index, "tint": tint, "module_ms": module_ms, "pipeline_ms": pipeline_ms});
                    eprintln!("program_pipeline_profile {row}");
                    rows.push(row);
                }
            }
        }
        let out = PathBuf::from(manifest["output"].as_str().unwrap());
        fs::create_dir_all(&out).unwrap();
        let report = serde_json::json!({
            "adapter": format!("{:?}", adapter.get_info()), "salt": salt,
            "prelude_sha256": format!("{:x}", Sha256::digest(source.as_bytes())), "samples": rows
        });
        fs::write(
            out.join("pipeline-profile.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
