//! PC1: device-owned pipeline cache and opt-in startup profiling. Descriptors and
//! shader sources are unchanged; wgpu still validates and compiles every edit.
//! Engine-owned shaders share one runtime-check policy; user programs stay checked.
//! P-PIPE: the disk cache is saved during compilation, not only on drop, so a killed
//! process keeps what it compiled.
use sha2::{Digest, Sha256};
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// CPU compilation time, including driver work for pipelines. Module time is
/// WGSL parsing/validation; pipeline time also includes SPIR-V generation.
#[derive(Clone, Debug)]
pub struct CompilationTiming {
    pub kind: &'static str,
    pub label: String,
    pub duration: Duration,
}

pub(crate) const CACHE_BYTES: usize = 64 * 1024 * 1024;
/// A cache file whose payload is above this is near `CACHE_BYTES`: a driver cache only grows
/// (every shader edit adds kernels and nothing removes stale ones), so the next start begins a
/// fresh cache instead of loading it, and that cache keeps only the pipelines its run creates.
/// One Glint run's own set is about 45 MiB, leaving the fresh cache about 11 MiB to grow.
pub(crate) const FRESH_BYTES: usize = CACHE_BYTES - CACHE_BYTES / 8;

struct Compilation {
    cache: Option<wgpu::PipelineCache>,
    path: Option<PathBuf>,
    /// Pipeline creation time through `cache` since the last save, in nanoseconds.
    unsaved: AtomicU64,
    /// Held while saving; holds the last save's duration, which sets the save interval, and
    /// the checksum of the data on disk (loaded or last written): unchanged data is not rewritten.
    save: Mutex<(Duration, Option<[u8; 32]>)>,
    profile: bool,
    checked_shaders: std::sync::atomic::AtomicBool,
    timings: Mutex<Vec<CompilationTiming>>,
    log: Option<crate::pipeline_log::PipelineLog>,
    /// Set when `Engine::prepare_pipelines` returns: every pipeline created after that is a
    /// lazy first-use compile in a frame, counted in `late` and logged loudly.
    prepared: std::sync::atomic::AtomicBool,
    late: AtomicU64,
    /// Every pipeline created and the nanoseconds spent creating them, for the pass log's
    /// per-frame compile columns (`compile_totals`).
    compiled_count: AtomicU64,
    compiled_nanos: AtomicU64,
}

/// Internal device facade: all render paths share one driver cache, including
/// kernels built on first use and pipelines built by background reload workers.
#[derive(Clone)]
pub(crate) struct PipelineDevice {
    raw: wgpu::Device,
    compilation: Arc<Compilation>,
}
impl Deref for PipelineDevice {
    type Target = wgpu::Device;
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}
impl From<wgpu::Device> for PipelineDevice {
    fn from(raw: wgpu::Device) -> Self {
        Self::new(raw, None)
    }
}
impl PipelineDevice {
    pub fn new(raw: wgpu::Device, disk: Option<(&Path, &wgpu::AdapterInfo)>) -> Self {
        Self::with_shader_checks(raw, disk, false)
    }
    pub fn with_shader_checks(
        raw: wgpu::Device,
        disk: Option<(&Path, &wgpu::AdapterInfo)>,
        checked: bool,
    ) -> Self {
        let started = Instant::now();
        let path = disk.and_then(|(directory, adapter)| cache_path(directory, adapter));
        if let Some(path) = &path {
            remove_stale_temporaries(path);
        }
        let (bytes, on_disk) = path.as_deref().map_or((None, None), load_cache);
        let cache = (disk.is_some() && raw.features().contains(wgpu::Features::PIPELINE_CACHE))
            .then(|| {
                // SAFETY: only checksummed, size-bounded wgpu get_data output is read.
                // wgpu additionally validates the cache header and driver UUID; a stale
                // or corrupt cache falls back to empty. No shader validation is skipped.
                unsafe {
                    raw.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                        label: Some("Renderlab shared pipeline cache"),
                        data: bytes.as_deref(),
                        fallback: true,
                    })
                }
            });
        let profile = std::env::var_os("RENDERLAB_STARTUP_PROFILE").is_some();
        let timings = if profile {
            vec![CompilationTiming {
                kind: "cache",
                label: format!("loaded {} bytes", bytes.as_ref().map_or(0, Vec::len)),
                duration: started.elapsed(),
            }]
        } else {
            Vec::new()
        };
        Self {
            raw,
            compilation: Arc::new(Compilation {
                cache,
                path,
                unsaved: AtomicU64::new(0),
                save: Mutex::new((Duration::ZERO, on_disk)),
                profile,
                checked_shaders: std::sync::atomic::AtomicBool::new(engine_shader_checks(checked)),
                timings: Mutex::new(timings),
                log: crate::pipeline_log::PipelineLog::from_env(),
                prepared: std::sync::atomic::AtomicBool::new(false),
                late: AtomicU64::new(0),
                compiled_count: AtomicU64::new(0),
                compiled_nanos: AtomicU64::new(0),
            }),
        }
    }
    /// Marks the scene prepared: pipelines created from here on count as compiles after
    /// prepare. Each call restarts the count, so a later prepare (a new level, replaced
    /// programs) measures from itself.
    pub fn mark_prepared(&self) {
        self.compilation.late.store(0, Ordering::Relaxed);
        self.compilation.prepared.store(true, Ordering::Relaxed);
    }
    /// Saves the disk cache now if it holds pipelines not yet on disk, waiting for a save in
    /// progress. Prepare calls it when it ends: compiles save only once their unsaved time
    /// reaches 20x the last save's cost (about 6 s for Ara's 6 MB cache), so a process
    /// stopped after a load lost the load's pipelines (19 Nov: Outpost's world prepare
    /// recompiled 6 s with the Renderlab cache alone).
    pub fn save_cache(&self) {
        let compilation = &self.compilation;
        if compilation.path.is_none() || compilation.unsaved.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut save = compilation.save.lock().unwrap();
        let unsaved = compilation.unsaved.load(Ordering::Relaxed);
        compilation.unsaved.fetch_sub(unsaved, Ordering::Relaxed);
        let started = Instant::now();
        let wrote = compilation.persist(&mut save.1);
        save.0 = started.elapsed();
        if compilation.profile {
            eprintln!("pipeline_cache_saved_at_prepare ms={:.3} wrote={wrote}", save.0.as_secs_f64() * 1e3);
        }
    }
    /// Pipelines created since the last `mark_prepared`; 0 when the engine was never prepared.
    pub fn compiles_after_prepare(&self) -> u64 {
        self.compilation.late.load(Ordering::Relaxed)
    }
    /// Pipelines created by this device so far, and the nanoseconds their creation took
    /// (waits for a compile slot excluded). The pass log books the difference between two
    /// frames to the later one.
    pub fn compile_totals(&self) -> (u64, u64) {
        (
            self.compilation.compiled_count.load(Ordering::Relaxed),
            self.compilation.compiled_nanos.load(Ordering::Relaxed),
        )
    }
    /// Custom preludes can feed any engine shading stage. Latch checks on for this
    /// device and its reload workers before compiling one; existing trusted modules
    /// still contain only the bundled source.
    pub fn check_custom_prelude(&self, source: &str) {
        if source != include_str!("../shaders/scene.wgsl") {
            self.compilation
                .checked_shaders
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    pub fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        self.compilation.cache.as_ref()
    }
    pub fn take_compilation_timings(&self) -> Vec<CompilationTiming> {
        std::mem::take(&mut *self.compilation.timings.lock().unwrap())
    }
    pub fn log_pipeline_frame(&self, renderer: &crate::Renderer, report: &crate::FrameReport) {
        if let Some(log) = &self.compilation.log {
            log.frame(renderer, report, self.compiles_after_prepare());
        }
    }
    fn timed<T>(&self, kind: &'static str, label: Option<&str>, compile: impl FnOnce() -> T) -> T {
        if !self.compilation.profile && self.compilation.log.is_none() {
            return compile();
        }
        let start = Instant::now();
        let result = compile();
        self.record(kind, label, start.elapsed());
        result
    }
    fn record(&self, kind: &'static str, label: Option<&str>, duration: Duration) {
        if let Some(log) = &self.compilation.log {
            log.compile(kind, label.unwrap_or(""), duration);
        }
        if !self.compilation.profile {
            return;
        }
        let timing = CompilationTiming {
            kind,
            label: label.unwrap_or("").into(),
            duration,
        };
        eprintln!(
            "pipeline_compile kind={kind} label={:?} ms={:.3}",
            timing.label,
            timing.duration.as_secs_f64() * 1e3
        );
        let mut timings = self.compilation.timings.lock().unwrap();
        // Profiling must stay bounded in a long-running editor session.
        if timings.len() < 4096 {
            timings.push(timing);
        }
    }
    /// Creates a pipeline within the process-wide compile limit, then lets the shared
    /// disk cache save if `cache` is it. Timing excludes the wait for a compile slot.
    fn pipeline<T>(
        &self,
        kind: &'static str,
        label: Option<&str>,
        cache: Option<&wgpu::PipelineCache>,
        compile: impl FnOnce() -> T,
    ) -> T {
        let slot = CompileSlot::acquire();
        let start = Instant::now();
        let result = compile();
        let duration = start.elapsed();
        drop(slot);
        self.compilation.compiled_count.fetch_add(1, Ordering::Relaxed);
        self.compilation
            .compiled_nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        if self.compilation.prepared.load(Ordering::Relaxed) {
            // Loud by design: prepare should have created it, and a frame waited for it.
            let late = self.compilation.late.fetch_add(1, Ordering::Relaxed) + 1;
            eprintln!(
                "warning: pipeline_compile_after_prepare n={late} kind={kind} label={:?} ms={:.3}",
                label.unwrap_or(""),
                duration.as_secs_f64() * 1e3
            );
        }
        if self.compilation.profile || self.compilation.log.is_some() {
            self.record(kind, label, duration);
        }
        if cache.is_some() && cache == self.pipeline_cache() {
            self.compilation.compiled(duration);
        }
        result
    }
    pub fn create_engine_shader_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor<'_>,
    ) -> wgpu::ShaderModule {
        self.timed("module", desc.label, || {
            shader_module(
                &self.raw,
                desc.clone(),
                self.compilation
                    .checked_shaders
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
        })
    }
    /// User-authored code always retains runtime checks, including reload workers.
    pub fn create_shader_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor<'_>,
    ) -> wgpu::ShaderModule {
        self.timed("module", desc.label, || {
            shader_module(&self.raw, desc.clone(), true)
        })
    }
    /// A module composed from program text: user, bundled standard and toon, TP1 glass, GI
    /// cell and glass reflected-hit shading. Always checked (P-SPLIT), so an edit recompiles
    /// only the kernels its text changes. Measurement only:
    /// `RENDERLAB_P_SPLIT_STANDARD_UNCHECKED` restores the earlier policy for the frame-time
    /// A/B, modules holding the bundled standard program on the engine API.
    pub fn create_program_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor<'_>,
    ) -> wgpu::ShaderModule {
        static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let legacy = *LEGACY.get_or_init(|| {
            std::env::var_os("RENDERLAB_P_SPLIT_STANDARD_UNCHECKED").is_some_and(|v| !v.is_empty())
        });
        if legacy
            && matches!(&desc.source, wgpu::ShaderSource::Wgsl(s) if s.contains(crate::STANDARD_PROGRAM))
        {
            return self.create_engine_shader_module(desc);
        }
        self.create_shader_module(desc)
    }
    pub fn create_compute_pipeline(
        &self,
        desc: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> wgpu::ComputePipeline {
        let desc = wgpu::ComputePipelineDescriptor {
            cache: desc.cache.or(self.pipeline_cache()),
            ..desc.clone()
        };
        self.pipeline("compute", desc.label, desc.cache, || {
            self.raw.create_compute_pipeline(&desc)
        })
    }
    pub fn create_render_pipeline(
        &self,
        desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> wgpu::RenderPipeline {
        let desc = wgpu::RenderPipelineDescriptor {
            cache: desc.cache.or(self.pipeline_cache()),
            ..desc.clone()
        };
        self.pipeline("render", desc.label, desc.cache, || {
            self.raw.create_render_pipeline(&desc)
        })
    }
}

/// One of the process's pipeline-compile slots, held while a driver compiles.
///
/// P-PIPE: every engine and reload worker in the process shares the limit, so callers
/// that fan out one thread per pipeline (`program_gpu::in_parallel`) cannot run
/// unbounded native compiles at once. On NVIDIA 617 each compile of a program shading
/// kernel (~230 KB SPIR-V) holds about 4.5-5 GB of driver memory while it runs; two at
/// once passed 8 GB and unbounded cold starts reached 18 GB private. The default of one
/// keeps a cold start near one kernel's peak (docs/p-pipe.md) at the cost of compiling
/// serially. `RENDERLAB_PIPELINE_COMPILES` raises it (at least 1).
///
/// 19 Nov (NVIDIA, current driver): a cold start compiling 52 pipelines, the program shading
/// kernels included, never moved the player's private bytes more than about 150 MB within
/// half a second, so a compile now holds far less. Prepare (`with_compile_slots`) holds four
/// slots while it runs (`RENDERLAB_PREPARE_COMPILES`), so a loading screen compiles four at a
/// time; frames keep the base limit.
struct CompileSlot;
static SLOTS: (Mutex<usize>, std::sync::Condvar) = (Mutex::new(0), std::sync::Condvar::new());
/// Slots added to the base limit while a prepare runs (`with_compile_slots`).
static EXTRA_SLOTS: AtomicU64 = AtomicU64::new(0);
impl CompileSlot {
    fn base() -> usize {
        static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *LIMIT.get_or_init(|| {
            std::env::var("RENDERLAB_PIPELINE_COMPILES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1usize)
                .max(1)
        })
    }
    fn limit() -> usize {
        Self::base() + EXTRA_SLOTS.load(Ordering::Relaxed) as usize
    }
    fn acquire() -> Self {
        let (busy, freed) = &SLOTS;
        let mut busy = freed
            .wait_while(busy.lock().unwrap(), |busy| *busy >= Self::limit())
            .unwrap();
        *busy += 1;
        Self
    }
}
impl Drop for CompileSlot {
    fn drop(&mut self) {
        let (busy, freed) = &SLOTS;
        *busy.lock().unwrap() -= 1;
        freed.notify_one();
    }
}

/// Runs `f` with at least `slots` compile slots (`CompileSlot`): prepare's fan-out
/// (`program_gpu::in_parallel`) then compiles that many pipelines at once. Nested and
/// concurrent calls add up their extra slots; each returns its own when `f` ends.
pub(crate) fn with_compile_slots<R>(slots: usize, f: impl FnOnce() -> R) -> R {
    let extra = slots.saturating_sub(CompileSlot::base()) as u64;
    EXTRA_SLOTS.fetch_add(extra, Ordering::Relaxed);
    SLOTS.1.notify_all();
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            EXTRA_SLOTS.fetch_sub(self.0, Ordering::Relaxed);
        }
    }
    let _restore = Restore(extra);
    f()
}
/// Compile slots prepare holds (`RENDERLAB_PREPARE_COMPILES`, default 4, at least 1).
pub(crate) fn prepare_compile_slots() -> usize {
    static SLOTS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SLOTS.get_or_init(|| {
        std::env::var("RENDERLAB_PREPARE_COMPILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4usize)
            .max(1)
    })
}
/// Resolves once when the device facade is created, so reloads use the same policy.
pub(crate) fn engine_shader_checks(default: bool) -> bool {
    match std::env::var("RENDERLAB_ENGINE_SHADER_CHECKS").as_deref() {
        Ok("checked" | "1") => true,
        Ok("unchecked" | "0") => false,
        _ => default,
    }
}
/// The only runtime-check switch. WGSL parsing and static validation remain enabled.
fn shader_module(
    device: &wgpu::Device,
    desc: wgpu::ShaderModuleDescriptor<'_>,
    checked: bool,
) -> wgpu::ShaderModule {
    let checks = if checked {
        wgpu::ShaderRuntimeChecks::checked()
    } else {
        wgpu::ShaderRuntimeChecks::unchecked()
    };
    // SAFETY: unchecked is used only for engine-owned shaders: bounded dispatches,
    // explicit buffer bounds and terminating loops are the engine's responsibility.
    // User-authored compositions always select checked, even in unchecked mode.
    unsafe { device.create_shader_module_trusted(desc, checks) }
}
pub(crate) fn create_engine_shader_module(
    device: &wgpu::Device,
    desc: wgpu::ShaderModuleDescriptor<'_>,
) -> wgpu::ShaderModule {
    shader_module(device, desc, engine_shader_checks(false))
}

fn cache_path(directory: &Path, adapter: &wgpu::AdapterInfo) -> Option<PathBuf> {
    let key = wgpu::util::pipeline_cache_key(adapter)?;
    let driver = Sha256::digest(format!("{adapter:?}").as_bytes());
    Some(directory.join(format!("pc1-wgpu30-{key}-{driver:x}.bin")))
}
fn cache_data(bytes: &[u8]) -> Option<&[u8]> {
    let (hash, data) = bytes.split_at_checked(32)?;
    (data.len() <= CACHE_BYTES && Sha256::digest(data).as_slice() == hash).then_some(data)
}
/// Reads a file `write_cache` wrote: its checksummed payload, or `None` when the file
/// is missing, oversize, truncated or corrupt.
pub(crate) fn read_cache(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    Ok(read_cache_checked(path)?.map(|(data, _)| data))
}
/// `read_cache`, with the payload's checksum (what `write_cache` compares to skip a rewrite).
pub(crate) fn read_cache_checked(path: &Path) -> std::io::Result<Option<(Vec<u8>, [u8; 32])>> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        file => file?,
    };
    if file.metadata()?.len() > CACHE_BYTES as u64 + 32 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    // Bound the read too, in case another process replaces or grows the file.
    file.take(CACHE_BYTES as u64 + 33).read_to_end(&mut bytes)?;
    Ok(cache_data(&bytes).map(|data| (data.to_vec(), bytes[..32].try_into().unwrap())))
}
/// The start-up read of a cache file: the data to create the cache from and the checksum of
/// what the file holds (`write_cache`'s `on_disk`). Unreadable files start empty, with a
/// warning. A file near the bound (`FRESH_BYTES`) also starts empty, logged, with no checksum,
/// so the first save replaces it with this run's pipelines only.
pub(crate) fn load_cache(path: &Path) -> (Option<Vec<u8>>, Option<[u8; 32]>) {
    match read_cache_checked(path) {
        Ok(Some((data, _))) if data.len() > FRESH_BYTES => {
            eprintln!(
                "pipeline_cache_fresh: {} holds {:.1} MiB, near the {} MiB bound; starting a fresh cache \
                 that keeps only the pipelines this run creates",
                path.display(),
                mib(data.len()),
                CACHE_BYTES >> 20
            );
            (None, None)
        }
        Ok(loaded) => loaded.map_or((None, None), |(data, hash)| (Some(data), Some(hash))),
        Err(error) => {
            eprintln!("pipeline_cache_load_failed: {}: {error}; starting empty", path.display());
            (None, None)
        }
    }
}
fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1 << 20) as f64
}
/// Replaces `path` with `cache`'s current data behind a checksum, unless the data is the
/// same as `on_disk` (the checksum of what `path` holds: loaded or last written), which is
/// then left alone. Returns whether it wrote. Data over the size bound cannot be written: that
/// is an error, and `path` is removed so the next start begins a fresh cache.
pub(crate) fn write_cache(
    path: &Path,
    cache: &wgpu::PipelineCache,
    on_disk: &mut Option<[u8; 32]>,
) -> std::io::Result<bool> {
    cache.get_data().map_or(Ok(false), |data| write_cache_bytes(path, &data, on_disk))
}
/// `write_cache`'s file side. Each write goes to its own same-directory temporary file
/// (`<name>.<pid>-<n>.tmp`) and is renamed over `path`, so engines in one process and other
/// processes can share a directory: readers see a whole old or a whole new file, and the
/// last rename wins. The writer holds an exclusive lock on the temporary file until the
/// rename, and `Temporary` removes it on every error (and on a panic); a killed writer's
/// file is removed by the next start-up (`remove_stale_temporaries`). Data over `CACHE_BYTES`
/// is an error (see `write_cache`).
pub(crate) fn write_cache_bytes(path: &Path, data: &[u8], on_disk: &mut Option<[u8; 32]>) -> std::io::Result<bool> {
    use std::io::Write;
    if data.len() > CACHE_BYTES {
        // The file this run loaded (if `path` still holds it) can no longer take the run's
        // pipelines: removing it makes the next start a fresh cache. Another writer's newer
        // file is left alone.
        let loaded = on_disk.take();
        let removed = loaded.is_some()
            && read_cache_checked(path).is_ok_and(|file| file.map(|(_, hash)| hash) == loaded)
            && std::fs::remove_file(path).is_ok();
        return Err(std::io::Error::other(format!(
            "{:.1} MiB of pipelines is over the {} MiB bound and cannot be saved; {}",
            mib(data.len()),
            CACHE_BYTES >> 20,
            if removed { "removed the file, so the next start begins a fresh cache" } else { "the file is left as it is" }
        )));
    }
    let hash: [u8; 32] = Sha256::digest(data).into();
    if *on_disk == Some(hash) {
        return Ok(false);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path_tmp = path.with_extension(format!("{}-{sequence}.tmp", std::process::id()));
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path_tmp)?;
    let mut temporary = Temporary { path: path_tmp, file: Some(file), renamed: false };
    let file = temporary.file.as_mut().unwrap();
    file.lock()?;
    file.write_all(&hash)?;
    file.write_all(data)?;
    std::fs::rename(&temporary.path, path)?;
    temporary.renamed = true;
    *on_disk = Some(hash);
    Ok(true)
}
/// A cache's temporary file while `write_cache_bytes` writes it (open and locked):
/// removed when dropped unless it was renamed into place.
struct Temporary {
    path: PathBuf,
    file: Option<std::fs::File>,
    renamed: bool,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.renamed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
/// Removes the temporary files of `path`'s cache (`<name>.<pid>-<n>.tmp`) whose writer is gone:
/// a writer holds its file locked until the rename, and the system releases a process's locks
/// when it exits or is killed, so a file whose lock can be taken is a dead writer's. A killed
/// process left one 47 MB file per run beside Ara's cache (275 files, 12.9 GB, on 16 Nov).
pub(crate) fn remove_stale_temporaries(path: &Path) {
    let (Some(directory), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let prefix = format!("{stem}.");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.starts_with(&prefix) && name.ends_with(".tmp")) {
            continue;
        }
        let stale = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(entry.path())
            .is_ok_and(|file| file.try_lock().is_ok());
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
impl Compilation {
    /// Counts `duration` of pipeline creation through the disk cache and saves once the
    /// unsaved time reaches 20x the last save's cost (at least 0.5 s). A killed process
    /// then loses at most that much compile work, and saving adds at most about 5% to
    /// compile time. A save already running on another thread is not waited for; that
    /// save or the next one includes this pipeline.
    fn compiled(&self, duration: Duration) {
        if self.path.is_none() {
            return;
        }
        let nanos = duration.as_nanos() as u64;
        let unsaved = self.unsaved.fetch_add(nanos, Ordering::Relaxed) + nanos;
        let Ok(mut save) = self.save.try_lock() else {
            return;
        };
        let (last, on_disk) = &mut *save;
        if Duration::from_nanos(unsaved) < (*last * 20).max(Duration::from_millis(500)) {
            return;
        }
        let started = Instant::now();
        // Pipelines that finish from here on count toward the next save.
        self.unsaved.fetch_sub(unsaved, Ordering::Relaxed);
        let wrote = self.persist(on_disk);
        *last = started.elapsed();
        if self.profile {
            eprintln!("pipeline_cache_saved ms={:.3} wrote={wrote}", last.as_secs_f64() * 1e3);
        }
    }
    /// Saves the cache when its data changed since the file was loaded or last written.
    fn persist(&self, on_disk: &mut Option<[u8; 32]>) -> bool {
        let (Some(cache), Some(path)) = (&self.cache, &self.path) else {
            return false;
        };
        write_cache(path, cache, on_disk).unwrap_or_else(|error| {
            // Cache I/O is optional acceleration, never a rendering failure, but a cache that
            // stops saving makes every start compile again: say so.
            eprintln!("warning: pipeline_cache_save_failed: {}: {error}", path.display());
            false
        })
    }
}
impl Drop for Compilation {
    fn drop(&mut self) {
        if *self.unsaved.get_mut() > 0 {
            let mut save = std::mem::take(self.save.get_mut().unwrap());
            let wrote = self.persist(&mut save.1);
            if self.profile {
                eprintln!("pipeline_cache_saved_at_exit wrote={wrote}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A scratch directory under the system temporary folder, removed by the caller.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("renderlab-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }
    fn temporaries(directory: &Path) -> Vec<String> {
        std::fs::read_dir(directory)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }
    #[test]
    fn cache_writes_once_per_change_and_leaves_no_temporary() {
        let directory = scratch("write");
        let path = directory.join("pc1-test.bin");
        let mut on_disk = None;
        assert!(write_cache_bytes(&path, b"first", &mut on_disk).unwrap());
        assert_eq!(read_cache_checked(&path).unwrap().unwrap(), (b"first".to_vec(), on_disk.unwrap()));
        // The same data again is not rewritten; new data is.
        assert!(!write_cache_bytes(&path, b"first", &mut on_disk).unwrap());
        assert!(write_cache_bytes(&path, b"second", &mut on_disk).unwrap());
        assert_eq!(read_cache(&path).unwrap().unwrap(), b"second");
        // A failed rename (the target is a directory) removes its temporary file.
        let blocked = directory.join("blocked.bin");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_cache_bytes(&blocked, b"data", &mut None).is_err());
        assert!(temporaries(&directory).is_empty(), "{:?}", temporaries(&directory));
        std::fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn a_cache_near_the_bound_starts_fresh_and_one_over_it_cannot_save_silently() {
        let directory = scratch("bound");
        let path = directory.join("pc1-test.bin");
        let near = vec![7u8; FRESH_BYTES + 1];
        assert!(write_cache_bytes(&path, &near, &mut None).unwrap());
        // The next start does not load it, and its first save replaces it with its own data.
        let (data, mut loaded) = load_cache(&path);
        assert!(data.is_none() && loaded.is_none());
        assert!(write_cache_bytes(&path, b"this run", &mut loaded).unwrap());
        assert_eq!(load_cache(&path), (Some(b"this run".to_vec()), loaded));
        // A run that outgrows the bound gets an error, and the file it loaded is removed.
        let over = vec![0u8; CACHE_BYTES + 1];
        let error = write_cache_bytes(&path, &over, &mut loaded).unwrap_err();
        assert!(error.to_string().contains("next start begins a fresh cache"), "{error}");
        assert!(!path.exists() && loaded.is_none());
        // Another writer's newer file is left alone.
        write_cache_bytes(&path, b"other", &mut None).unwrap();
        assert!(write_cache_bytes(&path, &over, &mut Some([0; 32])).is_err());
        assert_eq!(read_cache(&path).unwrap().unwrap(), b"other");
        std::fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn start_up_removes_dead_writers_temporaries_only() {
        let directory = scratch("stale");
        let path = directory.join("pc1-test.bin");
        std::fs::write(&path, b"cache").unwrap();
        // A killed writer's file (unlocked), a live writer's (locked by an open handle), and
        // another cache's temporary file.
        let dead = directory.join("pc1-test.999-0.tmp");
        let live = directory.join("pc1-test.1000-0.tmp");
        let other = directory.join("pc1-other.999-0.tmp");
        for f in [&dead, &live, &other] {
            std::fs::write(f, b"partial").unwrap();
        }
        let writer = std::fs::OpenOptions::new().write(true).open(&live).unwrap();
        writer.lock().unwrap();
        remove_stale_temporaries(&path);
        assert!(!dead.exists() && live.exists() && other.exists() && path.exists());
        drop(writer);
        remove_stale_temporaries(&path);
        assert!(!live.exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn cache_key_separates_driver_and_adapter() {
        let directory = Path::new("measurements/pipeline-cache");
        let mut info = wgpu::AdapterInfo::new(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
        let original = cache_path(directory, &info).unwrap();
        info.driver_info = "another driver".into();
        assert_ne!(cache_path(directory, &info).unwrap(), original);
        info.driver_info.clear();
        info.device = 1;
        assert_ne!(cache_path(directory, &info).unwrap(), original);
        info.backend = wgpu::Backend::Gl;
        assert!(cache_path(directory, &info).is_none());
    }
    #[test]
    fn compile_slots_bound_concurrent_compiles() {
        use std::sync::atomic::AtomicUsize;
        let (running, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..4 * CompileSlot::limit() {
                s.spawn(|| {
                    let _slot = CompileSlot::acquire();
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(5));
                    running.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert!(peak.into_inner() <= CompileSlot::limit());
    }
    #[test]
    fn rejects_corrupt_truncated_and_oversize_cache() {
        let mut bytes = Sha256::digest(b"wgpu payload").to_vec();
        bytes.extend(b"wgpu payload");
        assert_eq!(cache_data(&bytes), Some(b"wgpu payload".as_slice()));
        assert!(cache_data(&bytes[..31]).is_none());
        bytes[33] ^= 1;
        assert!(cache_data(&bytes).is_none());
        let data = vec![0; CACHE_BYTES + 1];
        let mut bytes = Sha256::digest(&data).to_vec();
        bytes.extend(data);
        assert!(cache_data(&bytes).is_none());
    }
}
