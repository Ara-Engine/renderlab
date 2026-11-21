//! Reflection hits' sun without ray hardware, from the sun-shadow map
//! (shaders/reflection_hit_sun.wgsl). The non-RT hit pass spent ~90% of its time on each hit's
//! software sun shadow ray; a resident level of the Virtual clipmap answers most hits with one
//! depth test, and the rest keep the ray. Reads the map as the fog does (atmosphere_shadow.rs:
//! its frame, top layer and page state), through `atmosphere_buffers`, in a fifth bind group of
//! the hit pipelines only: nothing of the map or its lookup changes. A child of
//! reflection_hits_gpu. `RENDERLAB_RL_HIT_SUN_MAP=0` keeps every hit's ray.
use super::*;

const ACCESS: &str = include_str!("../shaders/reflection_hit_sun.wgsl");
/// The hit pass's sun visibility seam (program.rs `REFLECTION_VISIBILITY`).
const SEAM: &str = "    if rl_reflection_hit {return rl_hit_shadow(lights[s.index],input.receiver,s.direction);}\n";
const REPLACEMENT: &str = "    if rl_reflection_hit {return rl_hit_sun(lights[s.index],input.receiver,s.direction);}\n";
/// The group the map binds at in the hit pipelines.
pub(super) const GROUP: u32 = 4;

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_HIT_SUN_MAP").map_or(true, |v| v != "0"))
}

/// `source` (a hit module) reading the sun from the map: the map's frame ABI and coordinate
/// helpers (shaders/virtual_shadow.wgsl before its page requests, with the frame at group 4
/// binding 0; its `Tr1Zones` is primary_rays.wgsl's, already declared) and the accessor. None
/// when the module has no sun seam to take (another switch already replaced it).
pub(super) fn compose(source: String) -> Option<String> {
    if source.matches(SEAM).count() != 1 {
        return None;
    }
    let zones = "struct Tr1Zones { shadow_lo:vec4<f32>, shadow_hi:vec4<f32>, shadow_light:vec4<f32>, relit:array<vec4<f32>,8>, counts:vec4<u32> };\n";
    let definitions = crate::renderer::virtual_shadow_gpu::with_pitch(
        include_str!("../shaders/virtual_shadow.wgsl")
            .split("// ---- Page requests")
            .next()?
            .replacen(zones, "", 1)
            .replacen(
                "@group(0) @binding(0) var<uniform> vsm:VsmFrame;",
                "@group(4) @binding(0) var<uniform> vsm:VsmFrame;",
                1,
            ),
    );
    if !source.contains(zones) || !definitions.contains("@group(4) @binding(0)") {
        return None;
    }
    Some(format!("{}\n{definitions}\n{ACCESS}", source.replacen(SEAM, REPLACEMENT, 1)))
}

/// Group 4's layout: the map's frame, top layer and page state.
pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("reflection hits' sun map"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(1),
            storage(2),
        ],
    })
}

impl Renderer {
    /// The map's buffers when the hit pass may read it: without ray hardware, with the map
    /// built, on a device that allows the fifth group.
    pub(super) fn hit_sun_buffers(&self) -> Option<[&wgpu::Buffer; 3]> {
        if !on()
            || self.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            || self.device.limits().max_bind_groups <= GROUP
        {
            return None;
        }
        self.view.virtual_shadows.as_ref()?.atmosphere_buffers()
    }
    /// Whether this view's frames will read the sun map in reflection hits: as
    /// `hit_sun_buffers`, but before the virtual shadows' resources exist (a frame allocates
    /// them before it builds its hit pipelines). Prepare builds the hit pipelines for it.
    pub(in crate::renderer) fn hit_sun_planned(&self) -> bool {
        on() && !self.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            && self.device.limits().max_bind_groups > GROUP
            && self.view.virtual_shadows.is_some()
    }
}
