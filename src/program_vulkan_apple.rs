//! `shader_compiler` on Apple targets, where wgpu has no Vulkan backend. The same public API as
//! `program_vulkan.rs`, with that module's behaviour on a non-Vulkan adapter: no development
//! device (`request_device` returns `None`, so callers take wgpu's ordinary path), no
//! capabilities and no native compile timings. The policy scopes only select how the Vulkan
//! device compiles, so here they run their closure as it is.

/// Preview is transient development output. Optimized is the ordinary shipping path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompilePolicy {
    #[default]
    Optimized,
    Preview,
    /// Retains helper call boundaries as a compiler hint. Original IR is always
    /// used by Optimized; no WGSL helper body is changed.
    PreviewNoInline,
    /// Retains measured helper boundaries for transient development compilation.
    PreviewHelpers,
    /// Fast-linked graphics libraries; compute uses the preview flag.
    LibraryFast,
    /// Optimize the same retained library IR when linking. Compute stays monolithic.
    LibraryLto,
    /// Normal optimized pipeline plus cached unchanged graphics subsets.
    WarmLibraries,
}

/// One native pipeline compile on the Vulkan development device (never recorded here).
#[derive(Clone, Debug)]
pub struct CompileTiming {
    pub policy: CompilePolicy,
    pub kind: &'static str,
    pub duration: std::time::Duration,
    pub application_cache_hit: bool,
    pub shader_bytes: usize,
}

/// The Vulkan development device's optional pipeline features (never available here).
#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub graphics_pipeline_library: bool,
    pub fast_linking: bool,
    pub independent_interpolation: bool,
}

/// Diagnostic scope of the Vulkan device's helper libraries; runs `compile` as it is.
#[cfg(test)]
pub(crate) fn with_helper_libraries<T>(
    _enabled: bool,
    _retain: bool,
    compile: impl FnOnce() -> T,
) -> T {
    compile()
}

/// Diagnostic scope of the Vulkan device's call boundaries; runs `compile` as it is.
#[cfg(test)]
pub(crate) fn with_boundaries<T>(
    _environment: bool,
    _names: &'static [&'static str],
    compile: impl FnOnce() -> T,
) -> T {
    compile()
}

/// The Vulkan device's compile policy for `compile`; runs it as it is.
pub fn with_policy<T>(_policy: CompilePolicy, compile: impl FnOnce() -> T) -> T {
    compile()
}

/// The Vulkan device's program compile policy for `compile`; runs it as it is.
pub(crate) fn with_program_policy<T>(
    _policy: CompilePolicy,
    _vertex_identity: [u8; 32],
    compile: impl FnOnce() -> T,
) -> T {
    compile()
}

/// No Vulkan development device on Apple targets: `None`, so callers use wgpu's ordinary
/// `request_device` path, as `program_vulkan.rs` does for a non-Vulkan adapter (after the
/// same check of the requested features and limits, which fails first there too).
pub fn request_device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<Option<(wgpu::Device, wgpu::Queue)>, crate::Error> {
    if !adapter.features().contains(descriptor.required_features)
        || !descriptor.required_limits.check_limits(&adapter.limits())
    {
        return Err(crate::Error::InvalidInput(
            "unsupported development-device features or limits",
        ));
    }
    Ok(None)
}

/// No Vulkan development device, so no capabilities.
pub fn capabilities(_device: &wgpu::Device) -> Option<Capabilities> {
    None
}

/// No Vulkan development device, so no native compile timings.
pub fn take_timings(_device: &wgpu::Device) -> Vec<CompileTiming> {
    Vec::new()
}
