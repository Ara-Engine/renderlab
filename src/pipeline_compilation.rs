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

struct Compilation {
    cache: Option<wgpu::PipelineCache>,
    path: Option<PathBuf>,
    /// Pipeline creation time through `cache` since the last save, in nanoseconds.
    unsaved: AtomicU64,
    /// Held while saving; holds the last save's duration, which sets the save interval.
    save: Mutex<Duration>,
    profile: bool,
    checked_shaders: std::sync::atomic::AtomicBool,
    timings: Mutex<Vec<CompilationTiming>>,
    log: Option<crate::pipeline_log::PipelineLog>,
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
        let bytes = path
            .as_ref()
            .and_then(|path| read_cache(path).ok().flatten());
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
                save: Mutex::new(Duration::ZERO),
                profile,
                checked_shaders: std::sync::atomic::AtomicBool::new(engine_shader_checks(checked)),
                timings: Mutex::new(timings),
                log: crate::pipeline_log::PipelineLog::from_env(),
            }),
        }
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
            log.frame(renderer, report);
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
struct CompileSlot;
static SLOTS: (Mutex<usize>, std::sync::Condvar) = (Mutex::new(0), std::sync::Condvar::new());
impl CompileSlot {
    fn limit() -> usize {
        static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *LIMIT.get_or_init(|| {
            std::env::var("RENDERLAB_PIPELINE_COMPILES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1usize)
                .max(1)
        })
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
    // Measurement only: RENDERLAB_F16_NEAREST rewrites chosen binary16 stores (src/f16_nearest.rs);
    // unset, the descriptor passes through untouched.
    let desc = match &desc.source {
        wgpu::ShaderSource::Wgsl(source) => {
            match crate::f16_nearest::rewrite(source, crate::f16_nearest::mode()) {
                std::borrow::Cow::Owned(rewritten) => wgpu::ShaderModuleDescriptor {
                    label: desc.label,
                    source: wgpu::ShaderSource::Wgsl(rewritten.into()),
                },
                std::borrow::Cow::Borrowed(_) => desc,
            }
        }
        _ => desc,
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
    Ok(cache_data(&bytes).map(<[u8]>::to_vec))
}
/// Replaces `path` with `cache`'s current data behind a checksum. Each call writes its
/// own same-directory temporary file and renames it over `path`, so engines in one
/// process and other processes can share a directory: readers see a whole old or a
/// whole new file, and the last rename wins. Data over the size bound is not written.
pub(crate) fn write_cache(path: &Path, cache: &wgpu::PipelineCache) -> std::io::Result<()> {
    let Some(data) = cache.get_data().filter(|data| data.len() <= CACHE_BYTES) else {
        return Ok(());
    };
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut bytes = Sha256::digest(&data).to_vec();
    bytes.extend(data);
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}-{sequence}.tmp", std::process::id()));
    let result = std::fs::write(&temporary, bytes).and_then(|_| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
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
        let Ok(mut last) = self.save.try_lock() else {
            return;
        };
        if Duration::from_nanos(unsaved) < (*last * 20).max(Duration::from_millis(500)) {
            return;
        }
        let started = Instant::now();
        // Pipelines that finish from here on count toward the next save.
        self.unsaved.fetch_sub(unsaved, Ordering::Relaxed);
        self.persist();
        *last = started.elapsed();
        if self.profile {
            eprintln!("pipeline_cache_saved ms={:.3}", last.as_secs_f64() * 1e3);
        }
    }
    fn persist(&self) {
        let (Some(cache), Some(path)) = (&self.cache, &self.path) else {
            return;
        };
        if let Err(error) = write_cache(path, cache) {
            // Cache I/O is optional acceleration, never a rendering failure.
            eprintln!("pipeline_cache_save_failed={error}");
        }
    }
}
impl Drop for Compilation {
    fn drop(&mut self) {
        if *self.unsaved.get_mut() > 0 {
            self.persist();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
