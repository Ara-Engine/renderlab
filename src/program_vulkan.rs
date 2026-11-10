//! Development-only Vulkan compiler policies below wgpu's pipeline API. The normal
//! policy forwards the exact create info. Only scoped program compilation opts in;
//! rendering, synchronization and native pipeline retirement stay owned by wgpu.
use ash::vk;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[path = "program_vulkan_library.rs"]
mod library;

/// Preview is transient development output. Optimized is the ordinary shipping path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompilePolicy {
    #[default]
    Optimized,
    /// SPIRV-Tools preprocessing for a transient, faster-to-compile edit.
    SpirvPreview,
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

#[derive(Clone, Copy)]
struct Scope {
    policy: CompilePolicy,
    vertex_identity: Option<[u8; 32]>,
    boundaries: Option<Boundaries>,
    helper_libraries: Option<bool>,
    retain_helper_ir: Option<bool>,
}
#[derive(Clone, Copy)]
struct Boundaries {
    environment: bool,
    names: &'static [&'static str],
}
thread_local! { static SCOPE: Cell<Scope> = const { Cell::new(Scope { policy: CompilePolicy::Optimized, vertex_identity: None, boundaries: None, helper_libraries: None, retain_helper_ir: None }) }; }
const HELPER_LIBRARIES: bool = false;
const RETAIN_HELPER_IR: bool = true;
const PREVIEW_BOUNDARIES: Boundaries = Boundaries {
    environment: true,
    names: &["exposed_product", "brdf", "unit_normal"],
};

#[cfg(test)]
pub(crate) fn with_helper_libraries<T>(
    enabled: bool,
    retain: bool,
    compile: impl FnOnce() -> T,
) -> T {
    scoped(
        Scope {
            helper_libraries: Some(enabled),
            retain_helper_ir: Some(retain),
            ..SCOPE.get()
        },
        compile,
    )
}

/// Diagnostic scope: compare call-boundary choices without editing any helper body.
#[cfg(test)]
pub(crate) fn with_boundaries<T>(
    environment: bool,
    names: &'static [&'static str],
    compile: impl FnOnce() -> T,
) -> T {
    scoped(
        Scope {
            boundaries: Some(Boundaries { environment, names }),
            ..SCOPE.get()
        },
        compile,
    )
}

/// The scope is per creating thread, including when a reload builds a pair in parallel.
pub fn with_policy<T>(policy: CompilePolicy, compile: impl FnOnce() -> T) -> T {
    scoped(
        Scope {
            policy,
            ..SCOPE.get()
        },
        compile,
    )
}
/// Only the engine prelude and entry name determine program vertex code: user
/// programs define surface/shade for fragment/compute, and cannot replace engine vs.
pub(crate) fn with_program_policy<T>(
    policy: CompilePolicy,
    vertex_identity: [u8; 32],
    compile: impl FnOnce() -> T,
) -> T {
    scoped(
        Scope {
            policy,
            vertex_identity: Some(vertex_identity),
            ..SCOPE.get()
        },
        compile,
    )
}
fn scoped<T>(scope: Scope, compile: impl FnOnce() -> T) -> T {
    struct Restore(Scope);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPE.set(self.0);
        }
    }
    let _restore = Restore(SCOPE.replace(scope));
    compile()
}

#[derive(Clone, Debug)]
pub struct CompileTiming {
    pub policy: CompilePolicy,
    pub kind: &'static str,
    pub duration: Duration,
    pub application_cache_hit: bool,
    pub shader_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub graphics_pipeline_library: bool,
    pub fast_linking: bool,
    pub independent_interpolation: bool,
}

struct DeviceState {
    raw: ash::Device,
    capabilities: Capabilities,
    timings: Mutex<VecDeque<CompileTiming>>,
    shader_modules: Mutex<HashMap<vk::ShaderModule, ShaderRecord>>,
    descriptor_layouts: Mutex<HashMap<vk::DescriptorSetLayout, u64>>,
    layouts: Mutex<HashMap<vk::PipelineLayout, [u8; 32]>>,
    next_descriptor: AtomicU64,
    libraries: library::Libraries,
}
#[derive(Clone, Copy)]
struct ShaderRecord {
    fingerprint: [u8; 32],
    bytes: usize,
}
struct NativeOwner {
    _base: wgpu::hal::OpenDevice<wgpu::hal::api::Vulkan>,
    handle: vk::Device,
}
impl Drop for NativeOwner {
    fn drop(&mut self) {
        devices()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.handle);
    }
}
static DEVICES: OnceLock<Mutex<HashMap<vk::Device, Arc<DeviceState>>>> = OnceLock::new();
fn devices() -> &'static Mutex<HashMap<vk::Device, Arc<DeviceState>>> {
    DEVICES.get_or_init(Mutex::default)
}
fn state(device: vk::Device) -> Option<Arc<DeviceState>> {
    devices()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&device)
        .cloned()
}

/// Creates an opt-in Vulkan development device. All requested wgpu features/limits
/// are preserved; wgpu-hal selects its normal feature chain and queue. Non-Vulkan
/// adapters return None so callers can use the ordinary request_device path.
/// The optional graphics-pipeline-library feature is enabled only when exposed.
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
    // SAFETY: the guard is borrowed only; the adapter and all handles stay owned.
    let Some(hal) = (unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Ok(None);
    };
    let instance = hal.shared_instance().raw_instance();
    let physical = hal.raw_physical_device();
    let extensions = hal.physical_device_capabilities();
    // Pipeline-creation feedback is core in 1.3. Older devices keep their ordinary
    // optimized wgpu path; no rendering capability or image quality is removed.
    if extensions.properties().api_version < vk::API_VERSION_1_3
        || hal.shared_instance().instance_api_version() < vk::API_VERSION_1_3
    {
        return Ok(None);
    }
    let mut supported = vk::PhysicalDeviceGraphicsPipelineLibraryFeaturesEXT::default();
    let mut properties = vk::PhysicalDeviceGraphicsPipelineLibraryPropertiesEXT::default();
    let available = extensions.supports_extension(ash::ext::graphics_pipeline_library::NAME)
        && extensions.supports_extension(ash::khr::pipeline_library::NAME);
    if available {
        // SAFETY: this physical device belongs to the guarded Vulkan instance. Both
        // output chains stay alive for the calls; no device state is changed.
        unsafe {
            instance.get_physical_device_features2(
                physical,
                &mut vk::PhysicalDeviceFeatures2::default().push_next(&mut supported),
            );
            instance.get_physical_device_properties2(
                physical,
                &mut vk::PhysicalDeviceProperties2::default().push_next(&mut properties),
            );
        }
    }
    let capabilities = Capabilities {
        graphics_pipeline_library: available && supported.graphics_pipeline_library != 0,
        fast_linking: properties.graphics_pipeline_library_fast_linking != 0,
        independent_interpolation: properties
            .graphics_pipeline_library_independent_interpolation_decoration
            != 0,
    };
    let mut enable = vk::PhysicalDeviceGraphicsPipelineLibraryFeaturesEXT::default()
        .graphics_pipeline_library(true);
    // SAFETY: only supported GPL extensions/features are added; no wgpu feature,
    // limit, queue or memory policy is removed. enable outlives vkCreateDevice.
    let base = unsafe {
        hal.open_with_callback(
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            Some(Box::new(|args| {
                if capabilities.graphics_pipeline_library {
                    args.extensions
                        .push(ash::ext::graphics_pipeline_library::NAME);
                    args.extensions.push(ash::khr::pipeline_library::NAME);
                    *args.create_info = args.create_info.push_next(&mut enable);
                }
            })),
        )
    }
    .map_err(|e| crate::Error::Fault(crate::Fault(format!("Vulkan development device: {e}"))))?;
    let original = base.device.raw_device().clone();
    let handle = original.handle();
    let family = base.device.queue_family_index();
    let queue_index = base.device.queue_index();
    let enabled = base.device.enabled_device_extensions().to_vec();
    let mut table = original.fp_v1_0().clone();
    table.create_graphics_pipelines = create_graphics;
    table.create_compute_pipelines = create_compute;
    table.create_shader_module = create_shader;
    table.destroy_shader_module = destroy_shader;
    table.create_descriptor_set_layout = create_descriptor_layout;
    table.destroy_descriptor_set_layout = destroy_descriptor_layout;
    table.create_pipeline_layout = create_layout;
    table.destroy_pipeline_layout = destroy_layout;
    table.destroy_render_pass = destroy_render_pass;
    let dispatch = ash::Device::from_parts_1_3(
        handle,
        table,
        original.fp_v1_1().clone(),
        original.fp_v1_2().clone(),
        original.fp_v1_3().clone(),
    );
    devices().lock().unwrap().insert(
        handle,
        Arc::new(DeviceState {
            raw: original,
            capabilities,
            timings: Mutex::default(),
            shader_modules: Mutex::default(),
            descriptor_layouts: Mutex::default(),
            layouts: Mutex::default(),
            next_descriptor: AtomicU64::new(1),
            libraries: Default::default(),
        }),
    );
    // The base OpenDevice is the sole owner of the native device. The imported HAL
    // object uses borrowed dispatch and releases that owner only after its resources.
    let owner = NativeOwner {
        _base: base,
        handle,
    };
    // SAFETY: same adapter, device, extensions, features and queue as hal.open. The
    // callback retains the native owner, so no alias can destroy the device early.
    let open = unsafe {
        hal.device_from_raw(
            dispatch,
            Some(Box::new(move || drop(owner))),
            &enabled,
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            family,
            queue_index,
        )
    }
    .map_err(|e| crate::Error::Fault(crate::Fault(format!("Vulkan development import: {e}"))))?;
    // SAFETY: open was created from this adapter with exactly descriptor's features.
    unsafe { adapter.create_device_from_hal::<wgpu::hal::api::Vulkan>(open, descriptor) }
        .map(Some)
        .map_err(|e| crate::Error::Fault(crate::Fault(e.to_string())))
}

fn for_device(device: &wgpu::Device) -> Option<Arc<DeviceState>> {
    // SAFETY: borrow only, without changing or destroying any HAL resource.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    state(hal.raw_device().handle())
}
pub fn capabilities(device: &wgpu::Device) -> Option<Capabilities> {
    for_device(device).map(|s| s.capabilities)
}
pub fn take_timings(device: &wgpu::Device) -> Vec<CompileTiming> {
    for_device(device).map_or_else(Vec::new, |s| s.timings.lock().unwrap().drain(..).collect())
}
fn record(
    state: &DeviceState,
    kind: &'static str,
    policy: CompilePolicy,
    started: Instant,
    feedback: vk::PipelineCreationFeedback,
    shader_bytes: usize,
) {
    let timing = CompileTiming {
        policy,
        kind,
        duration: started.elapsed(),
        application_cache_hit: feedback
            .flags
            .contains(vk::PipelineCreationFeedbackFlags::APPLICATION_PIPELINE_CACHE_HIT),
        shader_bytes,
    };
    if std::env::var_os("RENDERLAB_RELOAD_PROFILE").is_some() {
        eprintln!(
            "vulkan_compile kind={kind} policy={policy:?} ms={:.3} app_cache_hit={} shader_bytes={shader_bytes}",
            timing.duration.as_secs_f64() * 1e3,
            timing.application_cache_hit
        );
    }
    let mut timings = state.timings.lock().unwrap_or_else(|e| e.into_inner());
    if timings.len() == 512 {
        timings.pop_front();
    }
    timings.push_back(timing);
}

unsafe extern "system" fn create_graphics(
    device: vk::Device,
    cache: vk::PipelineCache,
    count: u32,
    infos: *const vk::GraphicsPipelineCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Pipeline,
) -> vk::Result {
    let Some(state) = state(device) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let scope = SCOPE.get();
    let policy = scope.policy;
    // SAFETY: Vulkan's call contract supplies count readable create infos. Copies
    // retain the original pointer fields only for this synchronous native call.
    let mut copies = unsafe { std::slice::from_raw_parts(infos, count as usize) }.to_vec();
    if count == 1
        && (matches!(
            policy,
            CompilePolicy::LibraryFast | CompilePolicy::LibraryLto
        ) || (policy == CompilePolicy::PreviewHelpers
            && scope.helper_libraries.unwrap_or(HELPER_LIBRARIES)))
        && let Some(result) =
            unsafe { library::create(&state, cache, &copies[0], allocator, out, scope) }
    {
        return result;
    }
    if matches!(
        policy,
        CompilePolicy::Preview
            | CompilePolicy::PreviewNoInline
            | CompilePolicy::PreviewHelpers
            | CompilePolicy::LibraryFast
    ) {
        for info in &mut copies {
            info.flags |= vk::PipelineCreateFlags::DISABLE_OPTIMIZATION;
        }
    }
    let mut feedback = vk::PipelineCreationFeedback::default();
    let mut feedback_info =
        vk::PipelineCreationFeedbackCreateInfo::default().pipeline_creation_feedback(&mut feedback);
    if count == 1 {
        feedback_info.p_next = copies[0].p_next;
        copies[0].p_next =
            (&feedback_info as *const vk::PipelineCreationFeedbackCreateInfo<'_>).cast();
    }
    let started = Instant::now();
    // SAFETY: all pointers and allocation callbacks are forwarded unchanged, except
    // the supported flag and the live feedback chain; the driver owns out on success.
    let result = unsafe {
        (state.raw.fp_v1_0().create_graphics_pipelines)(
            device,
            cache,
            count,
            copies.as_ptr(),
            allocator,
            out,
        )
    };
    let shader_bytes = copies
        .iter()
        .map(|i| unsafe { graphics_bytes(&state, i) })
        .sum();
    record(&state, "graphics", policy, started, feedback, shader_bytes);
    if policy == CompilePolicy::WarmLibraries && result == vk::Result::SUCCESS && count == 1 {
        // Use the unmodified descriptor, without the local feedback pNext.
        if let Err(error) = unsafe { library::warm(&state, cache, &*infos, scope) } {
            eprintln!("vulkan_library_warm_failed={error}");
        }
    }
    result
}

unsafe extern "system" fn create_compute(
    device: vk::Device,
    cache: vk::PipelineCache,
    count: u32,
    infos: *const vk::ComputePipelineCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Pipeline,
) -> vk::Result {
    let Some(state) = state(device) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let policy = SCOPE.get().policy;
    // SAFETY: the caller's Vulkan create-info slice is live for this entire call.
    let mut copies = unsafe { std::slice::from_raw_parts(infos, count as usize) }.to_vec();
    if matches!(
        policy,
        CompilePolicy::Preview
            | CompilePolicy::PreviewNoInline
            | CompilePolicy::PreviewHelpers
            | CompilePolicy::LibraryFast
    ) {
        for info in &mut copies {
            info.flags |= vk::PipelineCreateFlags::DISABLE_OPTIMIZATION;
        }
    }
    let mut feedback = vk::PipelineCreationFeedback::default();
    let mut feedback_info =
        vk::PipelineCreationFeedbackCreateInfo::default().pipeline_creation_feedback(&mut feedback);
    if count == 1 {
        feedback_info.p_next = copies[0].p_next;
        copies[0].p_next =
            (&feedback_info as *const vk::PipelineCreationFeedbackCreateInfo<'_>).cast();
    }
    let started = Instant::now();
    // SAFETY: only a supported compile flag and a live feedback chain were added.
    let result = unsafe {
        (state.raw.fp_v1_0().create_compute_pipelines)(
            device,
            cache,
            count,
            copies.as_ptr(),
            allocator,
            out,
        )
    };
    let modules = state
        .shader_modules
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let bytes = copies
        .iter()
        .filter_map(|i| modules.get(&i.stage.module))
        .map(|m| m.bytes)
        .sum();
    drop(modules);
    record(&state, "compute", policy, started, feedback, bytes);
    result
}

unsafe fn graphics_bytes(state: &DeviceState, info: &vk::GraphicsPipelineCreateInfo<'_>) -> usize {
    if info.stage_count == 0 {
        return 0;
    }
    let modules = state
        .shader_modules
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    unsafe { std::slice::from_raw_parts(info.p_stages, info.stage_count as usize) }
        .iter()
        .filter_map(|s| modules.get(&s.module))
        .map(|m| m.bytes)
        .sum()
}

unsafe extern "system" fn create_shader(
    device: vk::Device,
    info: *const vk::ShaderModuleCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::ShaderModule,
) -> vk::Result {
    let Some(state) = state(device) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let source = unsafe { &*info };
    let mut copy = *source;
    let edited = if SCOPE.get().policy == CompilePolicy::SpirvPreview {
        let words = unsafe { std::slice::from_raw_parts(source.p_code, source.code_size / 4) };
        match preview_spirv(words) {
            Ok(words) => Some(words),
            Err(error) => {
                eprintln!("spirv_preview_failed={error}");
                None
            }
        }
    } else if matches!(
        SCOPE.get().policy,
        CompilePolicy::PreviewNoInline | CompilePolicy::PreviewHelpers
    ) {
        let words = unsafe { std::slice::from_raw_parts(source.p_code, source.code_size / 4) };
        no_inline(
            words,
            SCOPE.get().policy == CompilePolicy::PreviewHelpers,
            SCOPE.get().boundaries.unwrap_or(PREVIEW_BOUNDARIES),
        )
    } else {
        None
    };
    if let Some(words) = &edited {
        copy.p_code = words.as_ptr();
        copy.code_size = words.len() * 4;
    }
    let info = &copy as *const vk::ShaderModuleCreateInfo<'_>;
    let result =
        unsafe { (state.raw.fp_v1_0().create_shader_module)(device, info, allocator, out) };
    if result == vk::Result::SUCCESS {
        // SAFETY: Vulkan's successful create call leaves the input code readable and
        // writes one valid output handle. The fingerprint retains no borrowed memory.
        let info = unsafe { &*info };
        let bytes = unsafe { std::slice::from_raw_parts(info.p_code.cast::<u8>(), info.code_size) };
        state
            .shader_modules
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                unsafe { *out },
                ShaderRecord {
                    fingerprint: Sha256::digest(bytes).into(),
                    bytes: bytes.len(),
                },
            );
    }
    result
}
// External SDK tool: no optimizer library or extra native build in the renderer.
// Pipes keep temporary shader files out of the user's tree. Only this scoped
// preview policy changes SPIR-V; the upgrade compiles the original naga output.
fn preview_spirv(words: &[u32]) -> Result<Vec<u32>, String> {
    let started = Instant::now();
    use std::io::Write;
    use std::process::{Command, Stdio};
    let executable = std::env::var_os("RENDERLAB_SPIRV_OPT")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("VULKAN_SDK")
                .map(|p| std::path::PathBuf::from(p).join("Bin/spirv-opt.exe"))
        })
        .unwrap_or_else(|| "spirv-opt".into());
    let mut command = Command::new(executable);
    command.args(["--skip-validation", "--target-env=vulkan1.3"]);
    // Measurement scaffolding: select one pass family without rebuilding.
    let passes = std::env::var("RENDERLAB_PREVIEW_PASSES").unwrap_or_else(|_| "-O".into());
    command
        .args(passes.split_whitespace())
        .args(["-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut stdin = child.stdin.take().unwrap();
    // Drain output concurrently: SPIRV-Tools may fill stdout before consuming all
    // input, and a synchronous write + wait would deadlock on larger modules.
    let output = std::thread::scope(|scope| {
        let write = scope.spawn(move || stdin.write_all(&bytes));
        let output = child.wait_with_output().map_err(|e| e.to_string());
        write.join().unwrap().map_err(|e| e.to_string())?;
        output
    })?;
    if !output.status.success() || output.stdout.len() % 4 != 0 {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let result: Vec<_> = output
        .stdout
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    if result.first() != Some(&0x07230203) {
        return Err("optimizer returned invalid SPIR-V".into());
    }
    eprintln!(
        "spirv_preview ms={:.3} input_bytes={} output_bytes={} passes={passes:?}",
        started.elapsed().as_secs_f64() * 1e3,
        words.len() * 4,
        result.len() * 4
    );
    Ok(result)
}
fn no_inline(source: &[u32], helpers_only: bool, boundaries: Boundaries) -> Option<Vec<u32>> {
    if source.len() < 5 {
        return None;
    }
    let mut at = 5;
    let mut entries = vec![];
    let mut helpers = vec![];
    while at < source.len() {
        let count = (source[at] >> 16) as usize;
        if count == 0 || at.checked_add(count)? > source.len() {
            return None;
        }
        if source[at] & 0xffff == 15 && count >= 3 {
            entries.push(source[at + 2]);
        }
        if source[at] & 0xffff == 5 && count >= 3 {
            let name: Vec<_> = source[at + 2..at + count]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .take_while(|&b| b != 0)
                .collect();
            if (boundaries.environment && (name.starts_with(b"environment_") || name == b"ambient"))
                || boundaries.names.iter().any(|n| n.as_bytes() == name)
            {
                helpers.push(source[at + 1]);
            }
        }
        at += count;
    }
    let mut words = source.to_vec();
    at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        // OpFunction: result type, result id, Function Control, function type.
        // DontInline=0x2; it cannot be combined with Inline=0x1.
        if words[at] & 0xffff == 54
            && count >= 5
            && !entries.contains(&words[at + 2])
            && (!helpers_only || helpers.is_empty() || helpers.contains(&words[at + 2]))
        {
            words[at + 3] = (words[at + 3] & !1) | 2;
        }
        at += count;
    }
    if SCOPE.get().boundaries.is_some() && std::env::var_os("RENDERLAB_RELOAD_PROFILE").is_some() {
        eprintln!(
            "vulkan_boundary_selection environment={} names={:?} matched={}",
            boundaries.environment,
            boundaries.names,
            helpers.len()
        );
    }
    Some(words)
}

unsafe extern "system" fn destroy_render_pass(
    device: vk::Device,
    pass: vk::RenderPass,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    if let Some(state) = state(device) {
        // Render-pass handles may be recycled. Drop cached subsets before forwarding
        // destruction so a later, incompatible pass can never alias their cache keys.
        state.libraries.clear();
        unsafe {
            (state.raw.fp_v1_0().destroy_render_pass)(device, pass, allocator);
        }
    }
}

unsafe extern "system" fn destroy_shader(
    device: vk::Device,
    shader: vk::ShaderModule,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    if let Some(state) = state(device) {
        state
            .shader_modules
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&shader);
        unsafe {
            (state.raw.fp_v1_0().destroy_shader_module)(device, shader, allocator);
        }
    }
}
unsafe extern "system" fn create_descriptor_layout(
    device: vk::Device,
    info: *const vk::DescriptorSetLayoutCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::DescriptorSetLayout,
) -> vk::Result {
    let Some(state) = state(device) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let result =
        unsafe { (state.raw.fp_v1_0().create_descriptor_set_layout)(device, info, allocator, out) };
    if result == vk::Result::SUCCESS {
        state
            .descriptor_layouts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                unsafe { *out },
                state.next_descriptor.fetch_add(1, Ordering::Relaxed),
            );
    }
    result
}
unsafe extern "system" fn destroy_descriptor_layout(
    device: vk::Device,
    layout: vk::DescriptorSetLayout,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    if let Some(state) = state(device) {
        state
            .descriptor_layouts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&layout);
        unsafe {
            (state.raw.fp_v1_0().destroy_descriptor_set_layout)(device, layout, allocator);
        }
    }
}
unsafe extern "system" fn create_layout(
    device: vk::Device,
    info: *const vk::PipelineLayoutCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::PipelineLayout,
) -> vk::Result {
    let Some(state) = state(device) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let result =
        unsafe { (state.raw.fp_v1_0().create_pipeline_layout)(device, info, allocator, out) };
    if result == vk::Result::SUCCESS
        && let Some(key) = unsafe { library::layout_key(&state, &*info) }
    {
        state
            .layouts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(unsafe { *out }, key);
    }
    result
}
unsafe extern "system" fn destroy_layout(
    device: vk::Device,
    layout: vk::PipelineLayout,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    if let Some(state) = state(device) {
        state
            .layouts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&layout);
        unsafe {
            (state.raw.fp_v1_0().destroy_pipeline_layout)(device, layout, allocator);
        }
    }
}
