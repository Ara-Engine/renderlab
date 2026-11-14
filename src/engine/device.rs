//! Render path selection, adapter capability queries and device negotiation.
use crate::Error;

/// Which of Renderlab's two first-class paths renders the scene. Both render the same
/// features to the same quality bar; neither is a fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenderPath {
    /// Hardware ray queries: traced primary visibility, traced sun visibility and
    /// hardware GI rays. Needs [`ExperimentalRayQuery`] at engine creation.
    HardwareRt,
    /// No ray-tracing hardware: a visibility buffer with software rasterization, the
    /// cached sun clipmap (`SoftShadows::Virtual`) and software GI rays. Needs 64-bit
    /// storage atomics and descriptor indexing, but no RT.
    NonRt,
}

/// The caller's acknowledgment of wgpu's experimental ray-query safety contract, which
/// [`RenderPath::HardwareRt`] needs. Renderlab never enables experimental features
/// implicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExperimentalRayQuery(());
impl ExperimentalRayQuery {
    /// # Safety
    /// wgpu's ray-query support is experimental: its validation is incomplete, so invalid
    /// use can be undefined behaviour rather than an error. The caller accepts that risk
    /// for the device this engine creates (`wgpu::ExperimentalFeatures::enabled`).
    pub unsafe fn acknowledge() -> Self {
        Self(())
    }
}

/// What an adapter can run, from its features and limits alone (no device is created).
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub adapter: wgpu::AdapterInfo,
    pub hardware_rt: bool,
    pub non_rt: bool,
}
impl Capabilities {
    /// What `adapter` supports. Both paths need `RECEIVER_FEATURES`, including
    /// `FLOAT32_FILTERABLE` for the filtered environment lookups (P-SKY): an adapter
    /// without it supports neither path, and `Engine::new` refuses it at start.
    pub fn query(adapter: &wgpu::Adapter) -> Self {
        let features = adapter.features();
        let limits = adapter.limits();
        let fits = |wanted: &wgpu::Limits| wanted.check_limits(&limits);
        let non_rt = features.contains(non_rt_features()) && fits(&non_rt_limits(&limits));
        let hardware_rt = features.contains(rt_features()) && fits(&rt_limits(&limits));
        Self {
            adapter: adapter.get_info(),
            hardware_rt,
            non_rt,
        }
    }
    pub fn supports(&self, path: RenderPath) -> bool {
        match path {
            RenderPath::HardwareRt => self.hardware_rt,
            RenderPath::NonRt => self.non_rt,
        }
    }
    /// Hardware RT where available, otherwise the non-RT path, otherwise None.
    pub fn preferred(&self) -> Option<RenderPath> {
        [RenderPath::HardwareRt, RenderPath::NonRt]
            .into_iter()
            .find(|&p| self.supports(p))
    }
}

fn rt_features() -> wgpu::Features {
    crate::RECEIVER_FEATURES | wgpu::Features::EXPERIMENTAL_RAY_QUERY | crate::TRACED_FEATURES
}
fn non_rt_features() -> wgpu::Features {
    crate::RECEIVER_FEATURES
        | crate::VISIBILITY_FEATURES
        | crate::VISIBILITY_RASTER_FEATURES
        | crate::VIRTUAL_FEATURES
}
/// Buffer limits follow the adapter: large scenes' software-ray geometry and clusters
/// live in single buffers (the quarry's is about 401 MB).
fn large_buffers(adapter: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_buffer_size: adapter.max_buffer_size,
        max_storage_buffer_binding_size: adapter.max_storage_buffer_binding_size,
        ..Default::default()
    }
}
fn non_rt_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    // U-RTVIS: the compacted reflection pass and its deferred composite need raised limits where
    // they run: with RENDERLAB_RL_PASS=1, or by default once reflection_pass.rs `NON_RT_DEFAULT`
    // is on (`default_on` reads the device's limits). Otherwise the visibility path's own limits.
    let limits = crate::visibility_limits(large_buffers(adapter));
    let wanted = crate::program::rl_pass_env()
        .unwrap_or(crate::renderer::primary_ray_gpu::reflection_pass::NON_RT_DEFAULT);
    if !wanted {
        return limits;
    }
    pass_limits(limits, adapter)
}
/// `limits` raised for the reflection pass and composite where `adapter` holds them (otherwise
/// unchanged, and the renderer keeps the inline path).
fn pass_limits(limits: wgpu::Limits, adapter: &wgpu::Limits) -> wgpu::Limits {
    use crate::renderer::primary_ray_gpu::reflection_pass::{STORAGE_BUFFERS, STORAGE_TEXTURES};
    let buffers = crate::visibility_limits(wgpu::Limits::default()).max_storage_buffers_per_shader_stage
        + STORAGE_BUFFERS;
    let fits = adapter.max_storage_buffers_per_shader_stage >= buffers
        && adapter.max_storage_textures_per_shader_stage >= STORAGE_TEXTURES;
    if !fits {
        return limits;
    }
    // What the adapter allows, capped (the RTX allows far more): the composite's shading stage
    // writes the HDR target, the reflection history, its lobes and RL#79's outputs, beside H, R
    // and the counters.
    wgpu::Limits {
        max_storage_buffers_per_shader_stage: limits
            .max_storage_buffers_per_shader_stage
            .max(adapter.max_storage_buffers_per_shader_stage.min(24)),
        max_storage_textures_per_shader_stage: limits
            .max_storage_textures_per_shader_stage
            .max(adapter.max_storage_textures_per_shader_stage.min(16)),
        ..limits
    }
}
fn rt_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    // The pass's bindings where it runs: by default (reflection_pass.rs `RT_DEFAULT`) or with
    // RENDERLAB_RL_PASS=1; RENDERLAB_RL_PASS=0 keeps the inline loop and the path's own limits.
    let limits = rt_path_limits(adapter);
    let wanted = crate::program::rl_pass_env()
        .unwrap_or(crate::renderer::primary_ray_gpu::reflection_pass::RT_DEFAULT);
    if wanted {
        return pass_limits(limits, adapter);
    }
    limits
}
fn rt_path_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    crate::traced_limits(wgpu::Limits {
        max_blas_primitive_count: adapter.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        // Real levels place tens of thousands of instances (foliage); the hardware
        // shadow path rejects more than 2^24 itself.
        max_tlas_instance_count: adapter.max_tlas_instance_count.min(1 << 24),
        max_acceleration_structures_per_shader_stage: 1,
        ..large_buffers(adapter)
    })
}

/// A device for `path` on `adapter`: the path's features and limits, plus timestamp
/// queries and the pipeline cache where the adapter has them.
pub(crate) fn request_device(
    adapter: &wgpu::Adapter,
    path: RenderPath,
) -> Result<(wgpu::Device, wgpu::Queue), Error> {
    let limits = adapter.limits();
    // PRIMITIVE_INDEX: per-pixel pane shading's id raster (src/transparency_panes.rs).
    // TEXTURE_COMPRESSION_BC: block-compressed material textures where the adapter samples
    // them (`Engine::compressed_textures`); hosts upload RGBA8 elsewhere.
    let optional = adapter.features()
        & (wgpu::Features::TIMESTAMP_QUERY
            | wgpu::Features::PIPELINE_CACHE
            | wgpu::Features::PRIMITIVE_INDEX
            | wgpu::Features::TEXTURE_COMPRESSION_BC);
    let (features, required_limits, experimental_features) = match path {
        RenderPath::HardwareRt => (
            // RENDERLAB_RT_PRIMARY (measurement): the visibility buffer's 64-bit raster atomics.
            if crate::renderer::primary_ray_gpu::rt_primary_switch().is_some() {
                rt_features() | crate::VISIBILITY_RASTER_FEATURES | crate::VIRTUAL_FEATURES
            } else {
                rt_features()
            },
            rt_limits(&limits),
            // SAFETY: only reached with the caller's `ExperimentalRayQuery` acknowledgment
            // (`Engine::new` checks the descriptor's path before calling this).
            unsafe { wgpu::ExperimentalFeatures::enabled() },
        ),
        RenderPath::NonRt => (
            non_rt_features(),
            non_rt_limits(&limits),
            wgpu::ExperimentalFeatures::disabled(),
        ),
    };
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("renderlab engine"),
        required_features: features | optional,
        required_limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    };
    // PC1: optional native driver timings use the existing compiler instrumentation.
    if std::env::var_os("RENDERLAB_STARTUP_PROFILE_DRIVER").is_some()
        && let Some(device) = crate::shader_compiler::request_device(adapter, &descriptor)?
    {
        return Ok(device);
    }
    pollster::block_on(adapter.request_device(&descriptor))
        .map_err(|_| Error::InvalidInput("adapter cannot create a device for this render path"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P-SKY samples the RGBA32F environment with hardware filtering: an adapter that
    /// cannot filter it must be refused by `Capabilities` on both paths, never drawn.
    #[test]
    fn both_paths_require_filterable_f32_environments() {
        for features in [rt_features(), non_rt_features()] {
            assert!(features.contains(wgpu::Features::FLOAT32_FILTERABLE));
        }
    }
}
