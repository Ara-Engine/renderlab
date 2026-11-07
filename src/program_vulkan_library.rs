//! Cached Vulkan graphics-pipeline subsets. These objects are never bound: only the
//! linked executable is returned to wgpu. Cache cells serialize identical fragment
//! compilation while independent vertex/tangent requests may proceed in parallel.
use super::*;
use ash::vk::Handle;
use sha2::{Digest, Sha256};
use std::{
    ffi::CStr,
    hash::{Hash, Hasher},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Part {
    Input,
    Vertex,
    Fragment,
    Output,
}
impl Part {
    fn flags(self) -> vk::GraphicsPipelineLibraryFlagsEXT {
        match self {
            Self::Input => vk::GraphicsPipelineLibraryFlagsEXT::VERTEX_INPUT_INTERFACE,
            Self::Vertex => vk::GraphicsPipelineLibraryFlagsEXT::PRE_RASTERIZATION_SHADERS,
            Self::Fragment => vk::GraphicsPipelineLibraryFlagsEXT::FRAGMENT_SHADER,
            Self::Output => vk::GraphicsPipelineLibraryFlagsEXT::FRAGMENT_OUTPUT_INTERFACE,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Input => "library-input",
            Self::Vertex => "library-vertex",
            Self::Fragment => "library-fragment",
            Self::Output => "library-output",
        }
    }
}

struct Fingerprint(Sha256);
impl Default for Fingerprint {
    fn default() -> Self {
        Self(Sha256::new())
    }
}
impl Hasher for Fingerprint {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    fn finish(&self) -> u64 {
        u64::from_le_bytes(self.0.clone().finalize()[..8].try_into().unwrap())
    }
}
impl Fingerprint {
    fn digest(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

struct Library {
    raw: vk::Pipeline,
    device: ash::Device,
}
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: libraries are never submitted/bound. A link retains an Arc through
        // its synchronous creation call. NativeOwner drops this cache before VkDevice.
        unsafe {
            self.device.destroy_pipeline(self.raw, None);
        }
    }
}
type Cell = Arc<OnceLock<Result<Arc<Library>, vk::Result>>>;
#[derive(Default)]
pub(super) struct Libraries(Mutex<HashMap<[u8; 32], Cell>>);
impl Libraries {
    pub(super) fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    fn get(
        &self,
        key: [u8; 32],
        create: impl FnOnce() -> Result<Arc<Library>, vk::Result>,
    ) -> Result<Arc<Library>, vk::Result> {
        let cell = {
            let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if entries.len() >= 128 && !entries.contains_key(&key) {
                entries.clear();
            }
            entries.entry(key).or_default().clone()
        };
        cell.get_or_init(create).clone()
    }
}

/// The layout key uses bind-group *creation generations*, so recycled Vulkan handles
/// cannot alias an earlier ABI. Equivalent reload layouts on the same live groups hit.
pub(super) unsafe fn layout_key(
    state: &DeviceState,
    info: &vk::PipelineLayoutCreateInfo<'_>,
) -> Option<[u8; 32]> {
    if !info.p_next.is_null() {
        return None;
    }
    let layouts = state
        .descriptor_layouts
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut hash = Fingerprint::default();
    (
        info.flags.as_raw(),
        info.set_layout_count,
        info.push_constant_range_count,
    )
        .hash(&mut hash);
    if info.set_layout_count > 0 {
        // SAFETY: Vulkan create-info arrays are readable for their declared counts.
        for layout in unsafe {
            std::slice::from_raw_parts(info.p_set_layouts, info.set_layout_count as usize)
        } {
            layouts.get(layout)?.hash(&mut hash);
        }
    }
    if info.push_constant_range_count > 0 {
        for range in unsafe {
            std::slice::from_raw_parts(
                info.p_push_constant_ranges,
                info.push_constant_range_count as usize,
            )
        } {
            (range.stage_flags.as_raw(), range.offset, range.size).hash(&mut hash);
        }
    }
    Some(hash.digest())
}

struct Stages<'a> {
    vertex: vk::PipelineShaderStageCreateInfo<'a>,
    fragment: vk::PipelineShaderStageCreateInfo<'a>,
    layout: [u8; 32],
    vertex_key: [u8; 32],
    fragment_key: [u8; 32],
}
unsafe fn stages<'a>(
    state: &DeviceState,
    info: &vk::GraphicsPipelineCreateInfo<'a>,
    scope: Scope,
) -> Option<Stages<'a>> {
    // Only the existing vertex+fragment program ABI is admitted here. Other Vulkan
    // state/extensions take the ordinary monolithic path with no lost capability.
    if info.stage_count != 2
        || !info.p_next.is_null()
        || !info.flags.is_empty()
        || info.render_pass == vk::RenderPass::null()
        || !info.p_tessellation_state.is_null()
    {
        return None;
    }
    let stages = unsafe { std::slice::from_raw_parts(info.p_stages, info.stage_count as usize) };
    if stages
        .iter()
        .any(|s| !s.p_next.is_null() || !s.p_specialization_info.is_null())
    {
        return None;
    }
    let vertex = *stages
        .iter()
        .find(|s| s.stage == vk::ShaderStageFlags::VERTEX)?;
    let fragment = *stages
        .iter()
        .find(|s| s.stage == vk::ShaderStageFlags::FRAGMENT)?;
    let layout = *state
        .layouts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&info.layout)?;
    let modules = state
        .shader_modules
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let vertex_key = scope
        .vertex_identity
        .unwrap_or(modules.get(&vertex.module)?.fingerprint);
    let fragment_key = modules.get(&fragment.module)?.fingerprint;
    Some(Stages {
        vertex,
        fragment,
        layout,
        vertex_key,
        fragment_key,
    })
}

unsafe fn key(
    info: &vk::GraphicsPipelineCreateInfo<'_>,
    stages: &Stages<'_>,
    part: Part,
    retain: bool,
) -> Option<[u8; 32]> {
    let mut hash = Fingerprint::default();
    part.hash(&mut hash);
    retain.hash(&mut hash);
    if part != Part::Input {
        (info.render_pass.as_raw(), info.subpass).hash(&mut hash);
    }
    if matches!(part, Part::Vertex | Part::Fragment) {
        stages.layout.hash(&mut hash);
        let stage = if part == Part::Vertex {
            &stages.vertex
        } else {
            &stages.fragment
        };
        let source = if part == Part::Vertex {
            stages.vertex_key
        } else {
            stages.fragment_key
        };
        source.hash(&mut hash);
        stage.flags.as_raw().hash(&mut hash);
        unsafe { CStr::from_ptr(stage.p_name) }
            .to_bytes()
            .hash(&mut hash);
    }
    // Each field below is consumed by the corresponding GPL subset. Raw structure
    // bytes are deliberately not hashed: padding and pointer addresses are not state.
    match part {
        Part::Input => {
            let v = unsafe { info.p_vertex_input_state.as_ref() }?;
            let a = unsafe { info.p_input_assembly_state.as_ref() }?;
            if !v.p_next.is_null() || !a.p_next.is_null() {
                return None;
            }
            (
                v.flags.as_raw(),
                v.vertex_binding_description_count,
                v.vertex_attribute_description_count,
                a.flags.as_raw(),
                a.topology.as_raw(),
                a.primitive_restart_enable,
            )
                .hash(&mut hash);
            if v.vertex_binding_description_count > 0 {
                for b in unsafe {
                    std::slice::from_raw_parts(
                        v.p_vertex_binding_descriptions,
                        v.vertex_binding_description_count as usize,
                    )
                } {
                    (b.binding, b.stride, b.input_rate.as_raw()).hash(&mut hash);
                }
            }
            if v.vertex_attribute_description_count > 0 {
                for a in unsafe {
                    std::slice::from_raw_parts(
                        v.p_vertex_attribute_descriptions,
                        v.vertex_attribute_description_count as usize,
                    )
                } {
                    (a.location, a.binding, a.format.as_raw(), a.offset).hash(&mut hash);
                }
            }
        }
        Part::Vertex => {
            let r = unsafe { info.p_rasterization_state.as_ref() }?;
            let v = unsafe { info.p_viewport_state.as_ref() }?;
            if !r.p_next.is_null()
                || !v.p_next.is_null()
                || !v.p_viewports.is_null()
                || !v.p_scissors.is_null()
                || r.rasterizer_discard_enable != 0
            {
                return None;
            }
            (
                r.flags.as_raw(),
                r.depth_clamp_enable,
                r.rasterizer_discard_enable,
                r.polygon_mode.as_raw(),
                r.cull_mode.as_raw(),
                r.front_face.as_raw(),
                r.depth_bias_enable,
                r.depth_bias_constant_factor.to_bits(),
                r.depth_bias_clamp.to_bits(),
                r.depth_bias_slope_factor.to_bits(),
                r.line_width.to_bits(),
            )
                .hash(&mut hash);
            (v.flags.as_raw(), v.viewport_count, v.scissor_count).hash(&mut hash);
        }
        Part::Fragment => {
            let d = unsafe { info.p_depth_stencil_state.as_ref() }?;
            if !d.p_next.is_null() {
                return None;
            }
            (
                d.flags.as_raw(),
                d.depth_test_enable,
                d.depth_write_enable,
                d.depth_compare_op.as_raw(),
                d.depth_bounds_test_enable,
                d.stencil_test_enable,
                d.min_depth_bounds.to_bits(),
                d.max_depth_bounds.to_bits(),
            )
                .hash(&mut hash);
            for s in [d.front, d.back] {
                (
                    s.fail_op.as_raw(),
                    s.pass_op.as_raw(),
                    s.depth_fail_op.as_raw(),
                    s.compare_op.as_raw(),
                    s.compare_mask,
                    s.write_mask,
                    s.reference,
                )
                    .hash(&mut hash);
            }
        }
        Part::Output => {
            let b = unsafe { info.p_color_blend_state.as_ref() }?;
            if !b.p_next.is_null() {
                return None;
            }
            (
                b.flags.as_raw(),
                b.logic_op_enable,
                b.logic_op.as_raw(),
                b.attachment_count,
                b.blend_constants.map(f32::to_bits),
            )
                .hash(&mut hash);
            if b.attachment_count > 0 {
                for a in unsafe {
                    std::slice::from_raw_parts(b.p_attachments, b.attachment_count as usize)
                } {
                    (
                        a.blend_enable,
                        a.src_color_blend_factor.as_raw(),
                        a.dst_color_blend_factor.as_raw(),
                        a.color_blend_op.as_raw(),
                        a.src_alpha_blend_factor.as_raw(),
                        a.dst_alpha_blend_factor.as_raw(),
                        a.alpha_blend_op.as_raw(),
                        a.color_write_mask.as_raw(),
                    )
                        .hash(&mut hash);
                }
            }
        }
    }
    if matches!(part, Part::Fragment | Part::Output) {
        let m = unsafe { info.p_multisample_state.as_ref() }?;
        if !m.p_next.is_null() {
            return None;
        }
        (
            m.flags.as_raw(),
            m.rasterization_samples.as_raw(),
            m.sample_shading_enable,
            m.min_sample_shading.to_bits(),
            m.alpha_to_coverage_enable,
            m.alpha_to_one_enable,
        )
            .hash(&mut hash);
        m.p_sample_mask.is_null().hash(&mut hash);
        if !m.p_sample_mask.is_null() {
            unsafe {
                std::slice::from_raw_parts(
                    m.p_sample_mask,
                    m.rasterization_samples.as_raw().div_ceil(32) as usize,
                )
            }
            .hash(&mut hash);
        }
    }
    if let Some(dynamic) = unsafe { info.p_dynamic_state.as_ref() } {
        if !dynamic.p_next.is_null() {
            return None;
        }
        dynamic.flags.as_raw().hash(&mut hash);
        for state in unsafe {
            std::slice::from_raw_parts(
                dynamic.p_dynamic_states,
                dynamic.dynamic_state_count as usize,
            )
        } {
            // wgpu 30 program pipelines use these four states. Unknown additions
            // deliberately fall back rather than guessing which subset consumes them.
            let owner = match *state {
                vk::DynamicState::VIEWPORT | vk::DynamicState::SCISSOR => Part::Vertex,
                vk::DynamicState::STENCIL_REFERENCE => Part::Fragment,
                vk::DynamicState::BLEND_CONSTANTS => Part::Output,
                _ => return None,
            };
            if owner == part {
                state.as_raw().hash(&mut hash);
            }
        }
    }
    Some(hash.digest())
}

unsafe fn native(
    state: &DeviceState,
    cache: vk::PipelineCache,
    info: &vk::GraphicsPipelineCreateInfo<'_>,
    kind: &'static str,
    policy: CompilePolicy,
) -> Result<(vk::Pipeline, vk::PipelineCreationFeedback), vk::Result> {
    let mut feedback = vk::PipelineCreationFeedback::default();
    let mut feedback_info =
        vk::PipelineCreationFeedbackCreateInfo::default().pipeline_creation_feedback(&mut feedback);
    feedback_info.p_next = info.p_next;
    let mut copy = *info;
    copy.p_next = (&feedback_info as *const vk::PipelineCreationFeedbackCreateInfo<'_>).cast();
    let mut out = vk::Pipeline::null();
    let started = Instant::now();
    // SAFETY: all borrowed original state and local pNext storage outlive this call;
    // only library-owned pipelines use this helper, always with a null allocator.
    let result = unsafe {
        (state.raw.fp_v1_0().create_graphics_pipelines)(
            state.raw.handle(),
            cache,
            1,
            &copy,
            std::ptr::null(),
            &mut out,
        )
    };
    record(state, kind, policy, started, feedback, unsafe {
        graphics_bytes(state, info)
    });
    if result == vk::Result::SUCCESS {
        Ok((out, feedback))
    } else {
        if out != vk::Pipeline::null() {
            unsafe {
                state.raw.destroy_pipeline(out, None);
            }
        }
        Err(result)
    }
}

unsafe fn part(
    state: &DeviceState,
    cache: vk::PipelineCache,
    original: &vk::GraphicsPipelineCreateInfo<'_>,
    stages: &Stages<'_>,
    part: Part,
    key: [u8; 32],
    scope: Scope,
) -> Result<Arc<Library>, vk::Result> {
    state.libraries.get(key, || {
        let mut gpl = vk::GraphicsPipelineLibraryCreateInfoEXT::default().flags(part.flags());
        let mut info = vk::GraphicsPipelineCreateInfo::default()
            .flags(vk::PipelineCreateFlags::LIBRARY_KHR)
            .push_next(&mut gpl);
        if retain_ir(part, scope) {
            info.flags |= vk::PipelineCreateFlags::RETAIN_LINK_TIME_OPTIMIZATION_INFO_EXT;
        }
        info.p_dynamic_state = original.p_dynamic_state;
        match part {
            Part::Input => {
                info.p_vertex_input_state = original.p_vertex_input_state;
                info.p_input_assembly_state = original.p_input_assembly_state;
            }
            Part::Vertex => {
                info.stage_count = 1;
                info.p_stages = &stages.vertex;
                info.p_viewport_state = original.p_viewport_state;
                info.p_rasterization_state = original.p_rasterization_state;
                info.layout = original.layout;
                info.render_pass = original.render_pass;
                info.subpass = original.subpass;
            }
            Part::Fragment => {
                info.flags |= vk::PipelineCreateFlags::DISABLE_OPTIMIZATION;
                info.stage_count = 1;
                info.p_stages = &stages.fragment;
                info.p_depth_stencil_state = original.p_depth_stencil_state;
                info.p_multisample_state = original.p_multisample_state;
                info.layout = original.layout;
                info.render_pass = original.render_pass;
                info.subpass = original.subpass;
            }
            Part::Output => {
                info.p_color_blend_state = original.p_color_blend_state;
                info.p_multisample_state = original.p_multisample_state;
                info.render_pass = original.render_pass;
                info.subpass = original.subpass;
            }
        }
        let (raw, _) = unsafe { native(state, cache, &info, part.name(), scope.policy) }?;
        Ok(Arc::new(Library {
            raw,
            device: state.raw.clone(),
        }))
    })
}

fn retain_ir(part: Part, scope: Scope) -> bool {
    part != Part::Fragment
        || scope.policy != CompilePolicy::PreviewHelpers
        || scope.retain_helper_ir.unwrap_or(RETAIN_HELPER_IR)
}

/// Returns None for unhandled native state, preserving the monolithic fallback.
pub(super) unsafe fn create(
    state: &DeviceState,
    cache: vk::PipelineCache,
    original: &vk::GraphicsPipelineCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Pipeline,
    scope: Scope,
) -> Option<vk::Result> {
    if !state.capabilities.graphics_pipeline_library || !allocator.is_null() {
        return None;
    }
    let stages = unsafe { stages(state, original, scope) }?;
    let parts = [Part::Input, Part::Vertex, Part::Fragment, Part::Output];
    let keys: Vec<_> = parts
        .iter()
        .map(|&p| unsafe { key(original, &stages, p, retain_ir(p, scope)) })
        .collect::<Option<_>>()?;
    let started = Instant::now();
    let mut libraries = vec![];
    for (&p, key) in parts.iter().zip(keys) {
        match unsafe { part(state, cache, original, &stages, p, key, scope) } {
            Ok(library) => libraries.push(library),
            Err(error) => return Some(error),
        }
    }
    let handles: Vec<_> = libraries.iter().map(|l| l.raw).collect();
    let mut link = vk::PipelineLibraryCreateInfoKHR::default().libraries(&handles);
    let flags = if scope.policy == CompilePolicy::LibraryLto {
        vk::PipelineCreateFlags::LINK_TIME_OPTIMIZATION_EXT
    } else {
        vk::PipelineCreateFlags::empty()
    };
    let info = vk::GraphicsPipelineCreateInfo::default()
        .flags(flags)
        .layout(original.layout)
        .render_pass(original.render_pass)
        .subpass(original.subpass)
        .push_next(&mut link);
    let result = unsafe { native(state, cache, &info, "library-link", scope.policy) };
    match result {
        Ok((pipeline, feedback)) => {
            // SAFETY: out is the live caller's one-pipeline output slot.
            unsafe {
                *out = pipeline;
            }
            record(
                state,
                "library-total",
                scope.policy,
                started,
                feedback,
                unsafe { graphics_bytes(state, original) },
            );
            Some(vk::Result::SUCCESS)
        }
        Err(error) => Some(error),
    }
}

pub(super) unsafe fn warm(
    state: &DeviceState,
    cache: vk::PipelineCache,
    info: &vk::GraphicsPipelineCreateInfo<'_>,
    scope: Scope,
) -> Result<(), vk::Result> {
    if !state.capabilities.graphics_pipeline_library {
        return Ok(());
    }
    let Some(stages) = (unsafe { stages(state, info, scope) }) else {
        return Ok(());
    };
    for p in [Part::Input, Part::Vertex, Part::Output] {
        if let Some(key) = unsafe { key(info, &stages, p, retain_ir(p, scope)) } {
            unsafe { part(state, cache, info, &stages, p, key, scope) }?;
        }
    }
    Ok(())
}
