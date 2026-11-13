//! Measurement split of RT primary visibility and reconstructed sun visibility.
//! Reuses the exact receiver trace and the visibility-buffer sun kernel: no new
//! estimator, direction sequence, history policy or material ABI.
use super::*;
#[path = "sun_rays_probe.rs"]
mod probe;

pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_SUN_PASS").is_ok_and(|v| v == "1")) || zone_stop()
}

fn zone_stop() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_SUN_ZONE_STOP").is_ok_and(|v| v == "1"))
}

/// A local light can restart the RGB history without changing sun visibility.
/// The existing held test already rejects moved receivers, swept caster shadows,
/// sun turns and non-unanimous history. Keep its eight fresh probes in those zones.
pub(super) fn schedule_source(source: String) -> String {
    if zone_stop() {
        with_zone_stop(source)
    } else {
        source
    }
}

fn with_zone_stop(source: String) -> String {
    const VETO: &str = " && tr1_rays==primary_frame.sampling.x";
    assert_eq!(source.matches(VETO).count(), 1, "sun held-stop budget seam");
    source.replacen(VETO, "", 1)
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
        if !probe::encode(self, encoder, sun_stamps.clone()) {
            self.encode_vb_sun(encoder, sun_stamps);
        }
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

    #[test]
    fn zone_stop_keeps_history_and_caster_guards() {
        let kernels = shader_sources(include_str!("../shaders/scene.wgsl")).unwrap();
        let baseline = temporal_reconstruction::vb_sun_source(&kernels[1]).unwrap();
        let variant = with_zone_stop(baseline.clone());
        assert_eq!(
            variant,
            baseline.replacen(" && tr1_rays==primary_frame.sampling.x", "", 1)
        );
        for guard in [
            "rtvis_moved(slot)",
            "rtvis_swept(receiver.world)",
            "k<9",
            "rtvis_held.words.z",
        ] {
            assert!(variant.contains(guard));
        }
        let module = naga::front::wgsl::parse_str(&variant).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
