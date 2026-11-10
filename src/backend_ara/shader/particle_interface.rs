//! Shared roots for particle sorting's frame, emitter and two storage arrays.
#[derive(Clone, Copy)]
#[repr(u32)]
pub(crate) enum ParticleResource {
    Frame,
    Emitter,
    Particles,
    Sorted,
}
impl ParticleResource {
    pub(crate) const ALL: [Self; 4] = [Self::Frame, Self::Emitter, Self::Particles, Self::Sorted];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Frame => "tp",
            Self::Emitter => "tp_emitter",
            Self::Particles => "tp_particles",
            Self::Sorted => "tp_sorted",
        }
    }
    pub(crate) fn ty(self) -> &'static str {
        match self {
            Self::Frame => "TpFrame",
            Self::Emitter => "TpEmitter",
            Self::Particles => "AraParticles",
            Self::Sorted => "AraWords",
        }
    }
    pub(crate) fn array(self) -> bool {
        matches!(self, Self::Particles | Self::Sorted)
    }
    pub(crate) fn writes(self) -> bool {
        matches!(self, Self::Sorted)
    }
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
}
