//! PC1: device-owned pipeline cache and opt-in startup profiling. Descriptors and
//! shader sources are unchanged; wgpu still validates and compiles every edit.
use sha2::{Digest, Sha256};
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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

const CACHE_BYTES: usize = 64 * 1024 * 1024;

struct Compilation {
    cache: Option<wgpu::PipelineCache>,
    path: Option<PathBuf>,
    profile: bool,
    timings: Mutex<Vec<CompilationTiming>>,
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
                profile,
                timings: Mutex::new(timings),
            }),
        }
    }
    pub fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        self.compilation.cache.as_ref()
    }
    pub fn take_compilation_timings(&self) -> Vec<CompilationTiming> {
        std::mem::take(&mut *self.compilation.timings.lock().unwrap())
    }
    fn timed<T>(&self, kind: &'static str, label: Option<&str>, compile: impl FnOnce() -> T) -> T {
        if !self.compilation.profile {
            return compile();
        }
        let start = Instant::now();
        let result = compile();
        let timing = CompilationTiming {
            kind,
            label: label.unwrap_or("").into(),
            duration: start.elapsed(),
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
        result
    }
    pub fn create_shader_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor<'_>,
    ) -> wgpu::ShaderModule {
        self.timed("module", desc.label, || {
            self.raw.create_shader_module(desc.clone())
        })
    }
    pub fn create_compute_pipeline(
        &self,
        desc: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> wgpu::ComputePipeline {
        let desc = wgpu::ComputePipelineDescriptor {
            cache: desc.cache.or(self.pipeline_cache()),
            ..desc.clone()
        };
        self.timed("compute", desc.label, || {
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
        self.timed("render", desc.label, || {
            self.raw.create_render_pipeline(&desc)
        })
    }
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
fn read_cache(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > CACHE_BYTES as u64 + 32 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    // Bound the read too, in case another process replaces or grows the file.
    file.take(CACHE_BYTES as u64 + 33).read_to_end(&mut bytes)?;
    Ok(cache_data(&bytes).map(<[u8]>::to_vec))
}
impl Drop for Compilation {
    fn drop(&mut self) {
        let (Some(cache), Some(path)) = (&self.cache, &self.path) else {
            return;
        };
        let Some(data) = cache.get_data().filter(|data| data.len() <= CACHE_BYTES) else {
            return;
        };
        let save = || -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let mut bytes = Sha256::digest(&data).to_vec();
            bytes.extend(data);
            // A unique same-directory temporary file keeps concurrent engines and
            // processes from sharing partial data. Last successful writer wins.
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let temporary = path.with_extension(format!("{}-{sequence}.tmp", std::process::id()));
            let result =
                std::fs::write(&temporary, bytes).and_then(|_| std::fs::rename(&temporary, path));
            if result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            result
        };
        if let Err(error) = save() {
            // Cache I/O is optional acceleration, never a rendering failure.
            eprintln!("pipeline_cache_save_failed={error}");
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
