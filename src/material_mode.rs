//! Native material-record producer selection. The shared shader composer stays backend neutral.
use crate::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    Tail,
    Full,
}

impl Mode {
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "" | "0" | "off" => Ok(Self::Off),
            "tail" => Ok(Self::Tail),
            "1" | "full" => Ok(Self::Full),
            _ => Err(Error::InvalidInput(
                "RENDERLAB_MATERIAL_PASS must be off, tail, or full (1)",
            )),
        }
    }

    pub(crate) fn current() -> Self {
        // Initialization validates before creating pipelines or resources.
        static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| {
            Self::parse(&std::env::var("RENDERLAB_MATERIAL_PASS").unwrap_or_default())
                .unwrap_or(Self::Off)
        })
    }

    pub(crate) fn validate(limits: &wgpu::Limits) -> Result<(), Error> {
        let mode = Self::parse(&std::env::var("RENDERLAB_MATERIAL_PASS").unwrap_or_default())?;
        Self::check_limits(mode, limits)
    }

    fn check_limits(mode: Self, limits: &wgpu::Limits) -> Result<(), Error> {
        if mode != Self::Off && limits.max_storage_textures_per_shader_stage < 6 {
            return Err(Error::InvalidInput(
                "material record requires 6 storage textures per shader stage",
            ));
        }
        Ok(())
    }

    pub(crate) fn tail() -> bool {
        Self::current() == Self::Tail
    }
    pub(crate) fn full() -> bool {
        Self::current() == Self::Full
    }
    pub(crate) fn layers(self) -> Option<u32> {
        match self {
            Self::Off => None,
            Self::Tail => Some(crate::program::MATERIAL_RECEIVER_LAYERS),
            Self::Full => Some(crate::program::MATERIAL_RECORD_LAYERS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_mode_boundary() {
        for (value, mode) in [
            ("", Mode::Off),
            ("0", Mode::Off),
            ("off", Mode::Off),
            ("tail", Mode::Tail),
            ("1", Mode::Full),
            ("full", Mode::Full),
        ] {
            assert_eq!(Mode::parse(value).unwrap(), mode);
            let mut limits = wgpu::Limits::default();
            limits.max_storage_textures_per_shader_stage = 5;
            assert_eq!(Mode::check_limits(mode, &limits).is_ok(), mode == Mode::Off);
            limits.max_storage_textures_per_shader_stage = 6;
            assert!(Mode::check_limits(mode, &limits).is_ok());
        }
        assert!(Mode::parse("typo").is_err());
        assert_eq!(Mode::Tail.layers(), Some(2));
        assert_eq!(Mode::Full.layers(), Some(3));
    }
}
