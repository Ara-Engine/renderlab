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
    /// (0 when disabled, so nothing is traced), y where it starts to fall.
    pub(crate) fn frame_words(&self) -> [f32; 4] {
        let end = if self.enabled {
            self.max_roughness
        } else {
            0.0
        };
        [end, 0.7 * end, 0.0, 0.0]
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
        for bad in [-0.1, 1.5, f32::NAN] {
            let settings = ReflectionSettings {
                max_roughness: bad,
                ..Default::default()
            };
            assert!(settings.validate().is_err());
        }
    }
}
