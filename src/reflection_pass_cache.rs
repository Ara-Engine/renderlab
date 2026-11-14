//! Optional software-pass hit termination using the existing outgoing GI cache.
//! `RENDERLAB_SW_REFLECTION_CACHE=0.35` enables it above that roughness; unset or
//! zero disables this local cache path. Cells contain view-independent outgoing radiance,
//! so custom programs that require view-dependent hit shading should leave it off.
use super::*;

fn roughness(device: &wgpu::Device) -> Option<f32> {
    if device
        .features()
        .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        || device.limits().max_storage_buffers_per_shader_stage
            < crate::visibility_limits(wgpu::Limits::default()).max_storage_buffers_per_shader_stage
                + STORAGE_BUFFERS
                + 1
    {
        return None;
    }
    static ROUGHNESS: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *ROUGHNESS.get_or_init(|| {
        std::env::var("RENDERLAB_SW_REFLECTION_CACHE")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|&value| value > 0.0 && value <= 1.0)
    })
}

pub(super) fn layout(
    device: &PipelineDevice,
    ledger: &Ledger,
) -> Option<Owned<wgpu::BindGroupLayout>> {
    roughness(device)?;
    Some(ledger.own(
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("software reflection outgoing cache"),
            entries: &[read(1, 16)],
        }),
        ResourceKind::BindGroupLayout,
    ))
}

/// The trace reads the cache directly, so successful lookups avoid hit records
/// and the hit-shading dispatch. Misses keep their full material evaluation.
pub(super) fn source(source: String, device: &wgpu::Device) -> String {
    let Some(roughness) = roughness(device) else {
        return source;
    };
    let seam = "let cached=rl_cache_hit(traced.record";
    assert_eq!(
        source.matches(seam).count(),
        1,
        "software reflection cache seam"
    );
    let source = source.replacen(seam, "let cached=rl_pass_cache_hit(traced.record", 1);
    let helper = crate::program::CACHE_HITS
        .replace("@group(3) @binding(28)", "@group(2) @binding(1)")
        .replace("rl_cache_outgoing", "rl_pass_cache_outgoing")
        .replace("rl_cache_hit", "rl_pass_cache_hit")
        .replace("RL_CACHE_ROUGH_MIN", &format!("{roughness:?}"))
        .replace("RL_CACHE_ROUGH", "RL_PASS_CACHE_ROUGH");
    format!("{source}\n{helper}")
}

impl Renderer {
    pub(super) fn rl_outgoing_group(
        &self,
        layout: &wgpu::BindGroupLayout,
    ) -> Owned<wgpu::BindGroup> {
        self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("software reflection outgoing cache"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.gi.shading_outgoing().as_entire_binding(),
                }],
            }),
            ResourceKind::BindGroup,
        )
    }
}
