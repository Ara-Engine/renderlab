//! Shading programs on the GPU: per program, a module for every active path, composed
//! against the active prelude compositions, and pipeline sets created from them on first
//! use. A reload transaction per program (or over everything, for the prelude) creates
//! every path's module, so naga validates the edit against each, then only the sets the
//! last frame drew with; the rest compile when a frame first needs them. Pipelines of
//! one transaction or frame are created concurrently, one thread each, since the driver
//! compiles on the creating thread. Engine kernels stay in `Pipelines`. Per-material
//! parameter blocks live here too.
// PC1: only the device type changes; pipeline creation uses the shared cache.
use super::*;
use crate::pipeline_compilation::PipelineDevice;
use crate::program::{
    self, Composed, MaterialParameters, ParameterLayout, ProgramShared, STANDARD_PROGRAM, Stage,
};
use primary_ray_gpu::{Shading, film_accumulation};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Mutex, Weak},
};

type Module = Arc<Owned<wgpu::ShaderModule>>;

/// Retains wgpu's validated naga IR and emitted module for recent compositions. The
/// source hash includes the prelude, program, stage and parameter declaration.
#[derive(Default)]
pub(super) struct ModuleCache(Mutex<VecDeque<([u8; 32], Module)>>);
impl ModuleCache {
    fn get(&self, source: &str) -> Option<Module> {
        let key: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, m)| m.clone())
    }
    fn insert(&self, source: &str, module: Module) {
        let mut entries = self.0.lock().unwrap();
        let key: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        entries.retain(|(k, _)| *k != key);
        if entries.len() == 32 {
            entries.pop_front();
        }
        entries.push_back((Sha256::digest(source.as_bytes()).into(), module));
    }
}

#[cfg(test)]
#[path = "program_api_tests.rs"]
mod api_tests;
#[path = "program_reload.rs"]
mod program_reload;
#[cfg(test)]
#[path = "toon_tests.rs"]
mod toon_tests;
pub(crate) use program_reload::ProgramReload;

/// A program pipeline set a frame shades with. Raster sets are pairs indexed by tangent
/// stream. Traced sets are one pipeline per `Shading`: a frame shades with one of them, so
/// each compiles only when a frame first needs it (P-PIPE: compiling the unused one cost a
/// cold start about 70 s and 5 GB of driver memory on NVIDIA).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// Lit raster frames without shadows, or with light-space shadow maps.
    Lit,
    /// Raster frames with hardware hard shadows.
    Hardware,
    /// Raster frames with fresh soft visibility counts.
    Fresh,
    /// Traced primary visibility, shaded Hard or Soft.
    Traced(Shading),
    /// Traced temporal film accumulation (`SoftShadows::Temporal`): one shading pipeline.
    Temporal,
}
impl Variant {
    /// Pipelines in the set: raster sets by tangent stream; traced and temporal sets one.
    pub fn len(self) -> usize {
        match self {
            Variant::Traced(_) | Variant::Temporal => 1,
            _ => 2,
        }
    }
}

/// One program's pipeline sets, each present once a frame needed it or a reload
/// recompiled it because the last frame drew with it.
#[derive(Default)]
pub(crate) struct ProgramPipelines {
    pub lit: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    pub hardware: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    pub fresh: Option<[Owned<wgpu::RenderPipeline>; 2]>,
    /// By `Shading`.
    pub traced: [Option<Owned<wgpu::ComputePipeline>>; 2],
    pub temporal: Option<Owned<wgpu::ComputePipeline>>,
    /// Traced shading whose sampling helpers record texture residency requests
    /// (`Renderer::ensure_streamed`), by `Shading`: built only when a frame streams
    /// textures with that shading.
    pub streamed: [Option<Owned<wgpu::ComputePipeline>>; 2],
    /// MP1: tile-list shading of mixed-program frames, without and with texture
    /// streaming, then by `Shading` (`Renderer::ensure_lists`). Built by the first mixed
    /// frame that needs one.
    pub list: [[Option<Owned<wgpu::ComputePipeline>>; 2]; 2],
}
impl ProgramPipelines {
    /// Tests: swaps probe shading pipelines in for both traced sets (both built).
    #[cfg(test)]
    pub(crate) fn swap_traced(&mut self, probe: &mut [Owned<wgpu::ComputePipeline>; 2]) {
        for (set, probe) in self.traced.iter_mut().zip(probe) {
            std::mem::swap(set.as_mut().unwrap(), probe);
        }
    }
    pub fn has(&self, variant: Variant) -> bool {
        match variant {
            Variant::Lit => self.lit.is_some(),
            Variant::Hardware => self.hardware.is_some(),
            Variant::Fresh => self.fresh.is_some(),
            Variant::Traced(shading) => self.traced[shading as usize].is_some(),
            Variant::Temporal => self.temporal.is_some(),
        }
    }
    /// Installs a set `create_sets` built: `variant.len()` pipelines.
    pub(super) fn install(&mut self, variant: Variant, set: Vec<Built>) {
        let render = |b: Built| match b {
            Built::Render(p) => p,
            Built::Compute(_) => unreachable!("raster sets hold render pipelines"),
        };
        let compute = |b: Built| match b {
            Built::Compute(p) => p,
            Built::Render(_) => unreachable!("traced sets hold compute pipelines"),
        };
        let mut set = set.into_iter();
        let mut next = || set.next().unwrap();
        match variant {
            Variant::Lit => self.lit = Some([render(next()), render(next())]),
            Variant::Hardware => self.hardware = Some([render(next()), render(next())]),
            Variant::Fresh => self.fresh = Some([render(next()), render(next())]),
            Variant::Traced(shading) => self.traced[shading as usize] = Some(compute(next())),
            Variant::Temporal => self.temporal = Some(compute(next())),
        }
    }
}

/// The driver cache program pipelines go through, present when the device enables
/// `Features::PIPELINE_CACHE`: recompiling a source seen before in this process (a revert,
/// an A/B toggle) is then a cache hit instead of a driver compile. Once `LIMIT` pipelines
/// have gone through it, the next transaction replaces it, dropping that history, which
/// bounds its memory over a long editing session.
/// Applications can persist it with `Renderer::set_program_cache_directory`, which saves
/// it after each compiled batch; recent composed modules retain wgpu's validated naga IR
/// in a separate bounded cache.
pub(super) struct ProgramCache {
    cache: Option<wgpu::PipelineCache>,
    pipelines: u64,
    path: Option<PathBuf>,
    pub modules: Arc<ModuleCache>,
}
impl ProgramCache {
    /// About 93 KB of driver data per lit pipeline was measured (fast-shader-reload.md).
    const LIMIT: u64 = 512;
    pub fn new(device: &PipelineDevice) -> Self {
        Self {
            cache: device
                .features()
                .contains(wgpu::Features::PIPELINE_CACHE)
                .then(|| {
                    device
                        .pipeline_cache()
                        .cloned()
                        .unwrap_or_else(|| Self::create(device))
                }),
            pipelines: 0,
            path: None,
            modules: Arc::default(),
        }
    }
    fn create(device: &PipelineDevice) -> wgpu::PipelineCache {
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
    pub fn refresh(&mut self, device: &PipelineDevice) {
        if self.cache.is_some() && self.pipelines >= Self::LIMIT {
            self.cache = Some(Self::create(device));
            self.pipelines = 0;
        }
    }
    /// Records a compiled batch of `pipelines` and saves a persistent cache, so a killed
    /// process keeps every finished batch.
    pub fn count(&mut self, pipelines: u64) {
        if self.cache.is_none() {
            return;
        }
        self.pipelines += pipelines;
        if let (Some(path), Some(cache)) = (&self.path, &self.cache)
            && pipelines > 0
            && let Err(error) = crate::pipeline_compilation::write_cache(path, cache)
        {
            eprintln!("program_pipeline_cache_save_failed={error}");
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
    lit: Module,
    hardware: Option<Module>,
    fresh: Option<Module>,
    /// Hard and Soft shading, present exactly while traced primary visibility is selected.
    pub traced: Option<[Module; 2]>,
    /// Temporal film shading, present with `traced`; its pipeline compiles on first use.
    pub temporal: Option<Module>,
    /// `Stage::TracedStreamed` Hard and Soft shading, built with `ProgramPipelines::streamed`.
    pub streamed: Option<[Module; 2]>,
}
impl ProgramModules {
    pub fn has(&self, variant: Variant) -> bool {
        match variant {
            Variant::Lit => true,
            Variant::Hardware => self.hardware.is_some(),
            Variant::Fresh => self.fresh.is_some(),
            Variant::Traced(_) => self.traced.is_some(),
            Variant::Temporal => self.temporal.is_some(),
        }
    }
    /// Pipeline `index` of `variant`: the tangent stream for raster sets, else 0.
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
            Variant::Traced(shading) => Built::Compute(abi.primary.unwrap().shade_pipeline(
                abi.device,
                abi.ledger,
                &self.traced.as_ref().unwrap()[shading as usize],
                shading as usize,
                abi.cache,
            )),
            Variant::Temporal => Built::Compute(abi.primary.unwrap().film_shade_pipeline(
                abi.device,
                abi.ledger,
                self.temporal.as_ref().unwrap(),
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
    /// Development preview pipelines must never seed temporal accumulation.
    pub preview: bool,
    /// Live materials' parameter blocks, repacked when a reload changes the layout.
    pub materials: Vec<Weak<MaterialParameters>>,
}
impl ProgramSlot {
    /// Install reflected layout/defaults only after the whole reload has succeeded.
    /// Explicit compatible overrides survive; omitted or retyped fields use defaults.
    pub(super) fn install_parameters(
        &mut self,
        next: Option<ParameterLayout>,
        queue: &wgpu::Queue,
        uploads: &mut UploadBytes,
    ) {
        if self.layout == next {
            return;
        }
        self.layout = next;
        self.materials.retain(|m| m.strong_count() > 0);
        for parameters in self.materials.iter().filter_map(Weak::upgrade) {
            let mut values = parameters.values.lock().unwrap();
            values.retain(|(name, value)| {
                ParameterLayout::check(self.layout.as_ref(), [(name.as_str(), value.clone())])
                    .is_ok()
            });
            let block = ParameterLayout::pack(self.layout.as_ref(), &values);
            queue.write_buffer(&parameters.buffer, 0, &block);
            uploads.materials += block.len() as u64;
            uploads.write_calls += 1;
        }
    }
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
    pub device: &'a PipelineDevice,
    pub ledger: &'a Ledger,
    pub samples: SampleCount,
    pub lit: &'a wgpu::PipelineLayout,
    pub hardware: Option<&'a wgpu::PipelineLayout>,
    pub fresh: Option<&'a FreshState>,
    pub primary: Option<&'a PrimaryState>,
    pub cache: Option<&'a wgpu::PipelineCache>,
    pub modules: &'a ModuleCache,
    pub vertex_identity: [u8; 32],
}

pub(super) struct Compiled {
    pub modules: ProgramModules,
    pub layout: Option<ParameterLayout>,
}

/// A program module, from the cache or newly created. Program modules always compile checked,
/// the bundled standard program included (P-SPLIT): one check mode for every program means an
/// edit recompiles only the kernels its text changes, not every kernel the program has.
fn module(abi: &ProgramAbi<'_>, label: &str, composed: &Composed) -> Module {
    if let Some(module) = abi.modules.get(&composed.source) {
        return module;
    }
    let module = Arc::new(abi.ledger.own(
        {
            let descriptor = wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(composed.source.as_str().into()),
            };
            abi.device.create_program_module(descriptor)
        },
        ResourceKind::ShaderModule,
    ));
    // Invalid modules must not enter the cache: their scoped validation error would
    // otherwise disappear on a later hit. Cache only after this module's scopes pass.
    module
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
    let started = Instant::now();
    let parameters = program::declares_parameters(source);
    let raster = program::compose(preludes.raster, source, Stage::Raster, parameters);
    let compose_raster = started.elapsed();
    let layout = program::analyse(label, source, &raster)?.parameters;
    let analysis = started.elapsed() - compose_raster;
    if layout.is_some() != parameters {
        return Err(Error::Shader(format!(
            "program `{label}`: struct Parameters must be declared at module scope"
        )));
    }
    let mut traced_sources: Vec<Composed> = preludes
        .traced
        .into_iter()
        .flatten()
        .map(|head| program::compose(head, source, Stage::Traced, parameters))
        .collect();
    // The temporal module follows the Soft one; naga checks it in the same transaction.
    if let Some(heads) = preludes.traced {
        let head = film_accumulation::shading_head(&heads[1])?;
        traced_sources.push(film_accumulation::compose_shading(
            &head, source, parameters,
        ));
    }
    let compose = started.elapsed() - analysis;
    let modules_started = Instant::now();
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
        hardware: preludes.hardware.map(raster_variant),
        fresh: preludes.fresh.map(raster_variant),
        traced: (!traced_sources.is_empty())
            .then(|| [0, 1].map(|i| module(abi, label, &traced_sources[i]))),
        temporal: traced_sources.get(2).map(|c| module(abi, label, c)),
        streamed: None,
    };
    if let Some(error) = scopes.finish() {
        return Err(scope_error(label, error, &traced_sources));
    }
    abi.modules.insert(&raster.source, modules.lit.clone());
    for (head, module) in [
        (preludes.hardware, &modules.hardware),
        (preludes.fresh, &modules.fresh),
    ] {
        if let (Some(head), Some(module)) = (head, module) {
            abi.modules.insert(
                &program::compose(head, source, Stage::Raster, parameters).source,
                module.clone(),
            );
        }
    }
    for (composed, module) in traced_sources.iter().zip(
        modules
            .traced
            .iter()
            .flatten()
            .chain(modules.temporal.iter()),
    ) {
        abi.modules.insert(&composed.source, module.clone());
    }
    if std::env::var_os("RENDERLAB_STARTUP_PROFILE").is_some() {
        eprintln!(
            "program_compile label={label:?} compose_ms={:.3} analyse_ms={:.3} modules_and_cache_ms={:.3} total_ms={:.3} raster_bytes={} traced_bytes={:?}",
            compose.as_secs_f64() * 1e3,
            analysis.as_secs_f64() * 1e3,
            modules_started.elapsed().as_secs_f64() * 1e3,
            started.elapsed().as_secs_f64() * 1e3,
            raster.source.len(),
            traced_sources
                .iter()
                .map(|c| c.source.len())
                .collect::<Vec<_>>()
        );
    }
    Ok(Compiled { modules, layout })
}

/// Runs `jobs` concurrently, one scoped thread each, and returns their values in order,
/// each with the first error its own scopes captured (wgpu error scopes are per thread).
pub(super) fn in_parallel<T: Send, F: FnOnce() -> T + Send>(
    device: &PipelineDevice,
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
) -> Result<Vec<Vec<Built>>, Error> {
    let jobs = sets.iter().flat_map(|&(modules, _, variant)| {
        (0..variant.len()).map(move |index| {
            move || {
                crate::shader_compiler::with_program_policy(
                    crate::shader_compiler::CompilePolicy::WarmLibraries,
                    abi.vertex_identity,
                    || modules.pipeline(abi, variant, index),
                )
            }
        })
    });
    let mut built = in_parallel(abi.device, jobs).into_iter();
    let mut out = Vec::with_capacity(sets.len());
    let mut failure = None;
    for &(_, label, variant) in sets {
        let mut set = Vec::with_capacity(variant.len());
        for _ in 0..variant.len() {
            let (pipeline, error) = built.next().unwrap();
            if let Some(error) = error.filter(|_| failure.is_none()) {
                failure = Some(match error {
                    wgpu::Error::Validation { .. } => {
                        Error::Shader(format!("program `{label}`, {variant:?} pipelines: {error}"))
                    }
                    _ => Error::Fault(Fault(error.to_string())),
                });
            }
            set.push(pipeline);
        }
        out.push(set);
    }
    failure.map_or(Ok(out), Err)
}

/// Pipelines in `built` sets, for `PipelineStats::created`.
pub(super) fn pipeline_count(built: &[Vec<Built>]) -> u64 {
    built.iter().map(|set| set.len() as u64).sum()
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
        preview: false,
        materials: vec![],
    };
    (handle, slot)
}

impl Renderer {
    /// Enables a persistent cache in an application-owned directory. Configure before
    /// the first lit frame. Files include a checksum, wgpu version and adapter/driver
    /// identity; damaged or obsolete files start empty. Saving follows each compiled batch.
    pub fn set_program_cache_directory(
        &mut self,
        directory: &Path,
        adapter: &wgpu::AdapterInfo,
    ) -> std::io::Result<()> {
        if self.program_cache.cache.is_none() {
            return Ok(());
        }
        let Some(key) = wgpu::util::pipeline_cache_key(adapter) else {
            return Ok(());
        };
        let driver = Sha256::digest(format!("{adapter:?}").as_bytes());
        let path = directory.join(format!("wgpu30-{key}-{driver:x}.bin"));
        let data = crate::pipeline_compilation::read_cache(&path)?;
        let data = data.as_deref();
        // SAFETY: the application-owned file contains only wgpu 30 get_data() output.
        // Its checksum rejects partial/corrupt writes; wgpu validates its own header
        // and driver compatibility, falling back to an empty cache on stale data.
        self.program_cache.cache = Some(unsafe {
            self.device
                .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("persistent program pipelines"),
                    data,
                    fallback: true,
                })
        });
        self.program_cache.path = Some(path);
        self.program_cache.pipelines = 0;
        Ok(())
    }

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
            preview: false,
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
    /// use program defaults (zero when absent). Zeroed members are listed in the report.
    pub fn reload_program(
        &mut self,
        p: &crate::Program,
        source: &str,
    ) -> Result<ReloadReport, ReloadError> {
        let reload = self.start_program_reload_with_policy(
            p,
            source,
            crate::shader_compiler::CompilePolicy::WarmLibraries,
        )?;
        self.finish_program_reload(reload)
    }

    fn install_program(
        &mut self,
        p: &crate::Program,
        source: &str,
        compiled: Compiled,
        variants: Vec<Variant>,
        built: Vec<Vec<Built>>,
        compile: Duration,
    ) -> ReloadReport {
        self.pipelines.successes += 1;
        self.pipelines.installations += 1;
        self.pipelines.created += pipeline_count(&built);
        self.program_cache.count(pipeline_count(&built));
        let slot = self.programs.iter_mut().find(|s| s.id == p.id()).unwrap();
        let zeroed = ParameterLayout::changed(slot.layout.as_ref(), compiled.layout.as_ref());
        slot.modules = compiled.modules;
        slot.preview = false;
        slot.pipelines = ProgramPipelines::default();
        for (variant, pair) in variants.into_iter().zip(built) {
            slot.pipelines.install(variant, pair);
        }
        slot.install_parameters(compiled.layout, &self.queue, &mut self.uploads);
        source.clone_into(&mut slot.source);
        self.update_hashes();
        self.revision.0 += 1;
        ReloadReport {
            revision: self.revision,
            compile,
            zeroed_parameters: zeroed,
        }
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
    /// builds its first draw's program's set; mixed traced frames (MP1) shade with list
    /// pipelines built in `submit_frame` (`ensure_lists`) and need no set here. A temporal
    /// frame (one program, by admission) also needs that program's traced set, which its
    /// reset frames shade with; raster frames also bind the standard program first.
    pub(super) fn frame_sets<'a>(
        &self,
        draws: impl IntoIterator<Item = Option<&'a Material>>,
        variant: Option<Variant>,
    ) -> Vec<(u64, Variant)> {
        let Some(variant) = variant else {
            return vec![];
        };
        let standard = self.standard.id();
        let programs = draws
            .into_iter()
            .map(|m| m.map_or(standard, |m| m.program.id()));
        let mut programs: Vec<u64> = if matches!(variant, Variant::Traced(_) | Variant::Temporal) {
            let (distinct, _) = primary_ray_gpu::program_lists::frame_slots(programs);
            if matches!(variant, Variant::Traced(_)) && distinct.len() > 1 {
                return vec![];
            }
            vec![distinct.first().copied().unwrap_or(standard)]
        } else {
            std::iter::once(standard).chain(programs).collect()
        };
        programs.sort_unstable();
        programs.dedup();
        // Temporal frames are soft; their reset frames shade Soft.
        let reset = (variant == Variant::Temporal).then_some(Variant::Traced(Shading::Soft));
        programs
            .into_iter()
            .flat_map(|id| std::iter::once((id, variant)).chain(reset.map(|v| (id, v))))
            .collect()
    }

    /// Creates the sets in `sets` that their programs lack, every pipeline concurrently,
    /// after the frame's admission checks and before its uploads, and records `sets` as
    /// the hot sets reloads recreate. A failure rejects the frame with `Error::Shader`
    /// (or faults).
    pub(super) fn ensure_pipelines(&mut self, sets: Vec<(u64, Variant)>) -> Result<(), Error> {
        let temporal: Vec<_> = sets
            .iter()
            .filter(|&&(id, variant)| variant == Variant::Temporal && self.program_slot(id).preview)
            .map(|&(id, _)| id)
            .collect();
        let missing: Vec<_> = sets
            .iter()
            .copied()
            .filter(|&(id, variant)| {
                temporal.contains(&id) || !self.program_slot(id).pipelines.has(variant)
            })
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
            self.pipelines.created += pipeline_count(&built);
            self.program_cache.count(pipeline_count(&built));
            // Only after successful native compilation: discard other preview sets
            // before admitting a temporal frame, so reset/history never sees them.
            for id in temporal {
                let slot = self.programs.iter_mut().find(|s| s.id == id).unwrap();
                slot.pipelines = ProgramPipelines::default();
                slot.preview = false;
            }
            for (&(id, variant), pair) in missing.iter().zip(built) {
                let slot = self.programs.iter_mut().find(|s| s.id == id).unwrap();
                slot.pipelines.install(variant, pair);
            }
            self.check_fault()?;
        }
        self.hot = sets;
        self.hot_lists.clear();
        Ok(())
    }

    /// Every set the standard program's modules can build, for tests that swap its
    /// pipelines for probes and references.
    #[cfg(test)]
    pub(super) fn ensure_standard_pipelines(&mut self) {
        let hot = std::mem::take(&mut self.hot);
        let hot_lists = std::mem::take(&mut self.hot_lists);
        let sets = [
            Variant::Lit,
            Variant::Hardware,
            Variant::Fresh,
            Variant::Traced(Shading::Hard),
            Variant::Traced(Shading::Soft),
        ]
        .into_iter()
        .filter(|&v| self.programs[0].modules.has(v))
        .map(|v| (self.standard.id(), v))
        .collect();
        self.ensure_pipelines(sets).unwrap();
        self.hot = hot;
        self.hot_lists = hot_lists;
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
        ParameterLayout::check(layout, values.iter().cloned())?;
        let mut stored = m.parameters.values.lock().unwrap();
        for (name, value) in values.iter().cloned() {
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
    /// Includes the program's defaults even when no material overrides are supplied.
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
                    | if self.view.primary.is_some() {
                        wgpu::BufferUsages::STORAGE
                    } else {
                        wgpu::BufferUsages::empty()
                    },
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let values: Vec<_> = values
            .iter()
            .map(|(n, v)| ((*n).to_owned(), v.clone()))
            .collect();
        let slot = self
            .programs
            .iter_mut()
            .find(|s| s.id == program.id())
            .unwrap();
        let block = ParameterLayout::pack(slot.layout.as_ref(), &values);
        if block.iter().any(|&b| b != 0) {
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
            data.parameters.iter().cloned(),
        )
    }

    pub(super) fn program_slot(&self, id: u64) -> &ProgramSlot {
        self.programs.iter().find(|s| s.id == id).unwrap()
    }

    /// Program `id`'s streamed traced shading (`Stage::TracedStreamed`), built the first
    /// time a frame streams textures with it. Reloads and visibility changes drop it, so
    /// default frames never compile or bind it.
    pub(super) fn ensure_streamed(&mut self, id: u64, shading: usize) -> Result<(), Error> {
        if self.program_slot(id).pipelines.streamed[shading].is_some() {
            return Ok(());
        }
        let heads = &self.primary_sources.as_ref().unwrap().1;
        let slot = self.program_slot(id);
        let parameters = program::declares_parameters(&slot.source);
        let sources = [0, 1]
            .map(|i| program::compose(&heads[i], &slot.source, Stage::TracedStreamed, parameters));
        let abi = self.program_abi();
        let scopes = OperationScopes::new(&self.device);
        // Modules are cheap (naga); only the pipeline this frame shades with compiles.
        let modules = slot
            .modules
            .streamed
            .clone()
            .unwrap_or_else(|| sources.each_ref().map(|c| module(&abi, &slot.label, c)));
        let pipeline = abi.primary.unwrap().streamed_pipeline(
            abi.device,
            abi.ledger,
            &modules[shading],
            shading,
            abi.cache,
        );
        if let Some(error) = scopes.finish() {
            return Err(match scope_error(&slot.label, error, &sources) {
                Error::Fault(Fault(message)) => self.fail(message),
                other => other,
            });
        }
        self.pipelines.created += 1;
        let slot = self.programs.iter_mut().find(|s| s.id == id).unwrap();
        slot.modules.streamed = Some(modules);
        slot.pipelines.streamed[shading] = Some(pipeline);
        Ok(())
    }

    /// MP1: the tile-list shading pipeline (`program::compose_list`) of `Shading` index
    /// `shading`, streamed or not, of each program in `ids` that lacks it, every pipeline
    /// concurrently: built by the first mixed-program frame that needs it. Reloads and
    /// visibility changes drop them, like `streamed`; single-program frames never build them.
    pub(super) fn ensure_lists(
        &mut self,
        ids: &[u64],
        streamed: bool,
        shading: usize,
    ) -> Result<(), Error> {
        let missing: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|&id| {
                self.program_slot(id).pipelines.list[usize::from(streamed)][shading].is_none()
            })
            .collect();
        if missing.is_empty() {
            self.hot_lists.clear();
            self.hot_lists
                .extend(ids.iter().map(|&id| (id, streamed, shading)));
            return Ok(());
        }
        let stage = if streamed {
            Stage::TracedStreamed
        } else {
            Stage::Traced
        };
        let head = &self.primary_sources.as_ref().unwrap().1[shading];
        let sources: Vec<Composed> = missing
            .iter()
            .map(|&id| {
                let source = &self.program_slot(id).source;
                program::compose_list(head, source, stage, program::declares_parameters(source))
            })
            .collect();
        let abi = self.program_abi();
        let rejected = |i: usize, error| match scope_error(
            &self.program_slot(missing[i]).label,
            error,
            &sources[i..=i],
        ) {
            Error::Fault(Fault(message)) => self.fail(message),
            other => other,
        };
        let mut modules = vec![];
        for (i, composed) in sources.iter().enumerate() {
            let scopes = OperationScopes::new(&self.device);
            modules.push(module(&abi, &self.program_slot(missing[i]).label, composed));
            if let Some(error) = scopes.finish() {
                return Err(rejected(i, error));
            }
        }
        let primary = abi.primary.unwrap();
        let built = in_parallel(
            abi.device,
            modules.iter().map(|module| {
                let abi = &abi;
                move || {
                    primary
                        .list_pipeline(abi.device, abi.ledger, module, shading, streamed, abi.cache)
                }
            }),
        );
        let mut pipelines = vec![];
        for (i, (pipeline, error)) in built.into_iter().enumerate() {
            if let Some(error) = error {
                return Err(rejected(i, error));
            }
            pipelines.push(pipeline);
        }
        for (composed, module) in sources.iter().zip(&modules) {
            abi.modules.insert(&composed.source, module.clone());
        }
        let count = pipelines.len() as u64;
        self.pipelines.created += count;
        self.program_cache.count(count);
        for (id, pipeline) in missing.into_iter().zip(pipelines) {
            let slot = self.programs.iter_mut().find(|s| s.id == id).unwrap();
            slot.pipelines.list[usize::from(streamed)][shading] = Some(pipeline);
        }
        self.hot_lists.clear();
        self.hot_lists
            .extend(ids.iter().map(|&id| (id, streamed, shading)));
        Ok(())
    }

    /// Every program's traced Hard and Soft shading modules on `heads`, and its temporal
    /// module, in slot order; their pipelines compile when a traced frame first draws with
    /// the program.
    #[allow(clippy::type_complexity)]
    pub(super) fn compile_traced_modules(
        &self,
        heads: &[String; 2],
    ) -> Result<Vec<([Module; 2], Module)>, Error> {
        let abi = self.program_abi();
        let temporal = film_accumulation::shading_head(&heads[1])?;
        let mut out = vec![];
        for slot in &self.programs {
            let parameters = program::declares_parameters(&slot.source);
            let sources = [
                program::compose(&heads[0], &slot.source, Stage::Traced, parameters),
                program::compose(&heads[1], &slot.source, Stage::Traced, parameters),
                film_accumulation::compose_shading(&temporal, &slot.source, parameters),
            ];
            let scopes = OperationScopes::new(&self.device);
            let [hard, soft, film] = sources.each_ref().map(|c| module(&abi, &slot.label, c));
            if let Some(error) = scopes.finish() {
                return Err(match scope_error(&slot.label, error, &sources) {
                    Error::Fault(Fault(message)) => self.fail(message),
                    other => other,
                });
            }
            out.push(([hard, soft], film));
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
            fresh: self.view.fresh.as_ref(),
            primary: self
                .view
                .primary
                .as_ref()
                .filter(|_| self.pipeline.primary.is_some()),
            cache: self.program_cache.get(),
            modules: &self.program_cache.modules,
            vertex_identity: Sha256::digest(self.source.as_bytes()).into(),
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
                Variant::Traced(Shading::Hard),
                Variant::Traced(Shading::Soft),
            ] {
                for index in 0..variant.len() {
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
