//! Global reflection controls (`Renderer::set_reflections`), honoured by M7-R1's traced
//! reflections on RT frames and M7-R1n's software reflection rays on non-RT frames
//! (docs/m7-r1.md, docs/m7-r1n.md). Programs still opt in per program by declaring
//! `reflection_lobe` and reading `specular_incident`; these settings bound what any program
//! gets. Disabled or above the roughness bound, a program reads `specular_incident` with
//! w = 0 and keeps its own untraced estimate.
use crate::Error;

/// Whether reflection rays are traced, and up to which perceptual roughness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionSettings {
    pub enabled: bool,
    /// Perceptual roughness at which the traced fraction reaches 0, in [0, 1]. It starts
    /// to fall at 0.7 times this value. The default 0.5 is R1's band: traced fully to 0.35,
    /// fading to the untraced estimate by 0.5.
    pub max_roughness: f32,
}

/// The lone-stratum bound of glossy reflection rays (shaders/reflections.wgsl, docs/speck2.md):
/// the brightest stratum counts for at most this many times the luminance of all the others
/// together. 1 keeps any single stratum to half a pixel's estimate.
const LONE_STRATUM_LIMIT: f32 = 1.0;

/// The bound in force. Measurement switch: `RENDERLAB_REFLECTION_LONE` replaces it for a
/// process (0 switches the bound off), so one binary renders both sides of a comparison.
fn lone_stratum_limit() -> f32 {
    static LIMIT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("RENDERLAB_REFLECTION_LONE")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| v.is_finite() && *v >= 0.0)
            .unwrap_or(LONE_STRATUM_LIMIT)
    })
}

/// How many times its ordinary glossy strata a receiver traces on a frame the resolve restarts
/// it (a changed local light's restart sphere, a moving caster's swept shadow): it shows
/// that frame's strata alone (shaders/reflections.wgsl, docs/speck2.md).
const RESTART_STRATA: f32 = 2.0;

/// The multiple in force. Measurement switch: `RENDERLAB_REFLECTION_RESTART_STRATA` replaces
/// it for a process (1 switches it off).
fn restart_strata() -> f32 {
    static STRATA: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *STRATA.get_or_init(|| {
        std::env::var("RENDERLAB_REFLECTION_RESTART_STRATA")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| (1.0..=4.0).contains(v))
            .unwrap_or(RESTART_STRATA)
    })
}

impl Default for ReflectionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_roughness: 0.5,
        }
    }
}

impl ReflectionSettings {
    /// No reflection rays on either path: every program keeps its untraced estimate.
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !(0.0..=1.0).contains(&self.max_roughness) {
            return Err(Error::InvalidInput(
                "reflection max_roughness must be in [0, 1]",
            ));
        }
        Ok(())
    }
    /// WGSL `primary_frame.reflection`: x the roughness where the traced fraction reaches 0
    /// (0 when disabled, so nothing is traced), y where it starts to fall, z the lone-stratum
    /// bound (0: none), w the strata multiple of restarted receivers.
    pub(crate) fn frame_words(&self) -> [f32; 4] {
        // U-RTVIS measurement switch: `RENDERLAB_REFLECTION_ROUGH` replaces max_roughness.
        static ROUGH: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
        let rough = *ROUGH.get_or_init(|| {
            std::env::var("RENDERLAB_REFLECTION_ROUGH")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f32| (0.0..=1.0).contains(v))
        });
        let end = if self.enabled {
            rough.unwrap_or(self.max_roughness)
        } else {
            0.0
        };
        [end, 0.7 * end, lone_stratum_limit(), restart_strata()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default reproduces R1's bands bit for bit; off traces nothing; bounds are checked.
    #[test]
    fn reflection_settings_bands() {
        let wgsl = include_str!("../shaders/reflections.wgsl");
        assert!(
            wgsl.contains("const RL_TRACED:f32=0.35;")
                && wgsl.contains("const RL_UNTRACED:f32=0.5;")
        );
        let [end, start, ..] = ReflectionSettings::default().frame_words();
        assert_eq!(
            (end.to_bits(), start.to_bits()),
            (0.5f32.to_bits(), 0.35f32.to_bits())
        );
        assert_eq!(ReflectionSettings::off().frame_words()[0], 0.0);
        // The lone-stratum bound reaches the shader in word z, and the shader reads it there.
        if std::env::var_os("RENDERLAB_REFLECTION_LONE").is_none() {
            assert_eq!(
                ReflectionSettings::default().frame_words()[2],
                LONE_STRATUM_LIMIT
            );
        }
        assert!(wgsl.contains("let lone=primary_frame.reflection.z;"));
        assert!(wgsl.contains("let restart_strata=primary_frame.reflection.w;"));
        for bad in [-0.1, 1.5, f32::NAN] {
            let settings = ReflectionSettings {
                max_roughness: bad,
                ..Default::default()
            };
            assert!(settings.validate().is_err());
        }
    }
}
