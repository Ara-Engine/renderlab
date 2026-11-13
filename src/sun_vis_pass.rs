//! Measurement split of RT primary visibility and reconstructed sun visibility.
//! Reuses the exact receiver trace and the visibility-buffer sun kernel: no new
//! estimator, direction sequence, history policy or material ABI.
use super::*;

pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_SUN_PASS").is_ok_and(|v| v == "1"))
}

impl Renderer {
    /// The camera pass publishes corrected float32 receivers. The following sun
    /// pass consumes those records with TR1's unchanged rotation and stopping rules.
    pub(in crate::renderer) fn encode_sun_split(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        camera_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        sun_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        self.encode_primary_visibility(encoder, true, camera_stamps);
        mark(encoder, "sun_shadow");
        self.encode_vb_sun(encoder, sun_stamps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sun_keeps_the_reconstructed_estimator() {
        let kernels = shader_sources(include_str!("../shaders/scene.wgsl")).unwrap();
        let source = temporal_reconstruction::vb_sun_source(&kernels[1]).unwrap();
        assert!(source.contains("visible=primary_sun(receiver,slot,id.xy,size.x);"));
        assert!(source.contains("primary_sun_store(p,visible);"));
        assert!(source.contains("tr1_held_probes"));
        assert!(source.contains("j+primary_frame.sided.y"));
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
