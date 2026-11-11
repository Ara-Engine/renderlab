//! Direct roots shared by the original diagnostic mesh vertex/fragment stages.
#[derive(Clone, Copy)]
#[repr(u32)]
pub(crate) enum MeshResource {
    Camera,
    Models,
    Normals,
    Material,
    Base,
    MetallicRoughness,
    BaseSampler,
    MrSampler,
}
impl MeshResource {
    pub(crate) const ALL: [Self; 8] = [
        Self::Camera,
        Self::Models,
        Self::Normals,
        Self::Material,
        Self::Base,
        Self::MetallicRoughness,
        Self::BaseSampler,
        Self::MrSampler,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Models => "models",
            Self::Normals => "normals",
            Self::Material => "material",
            Self::Base => "base_color",
            Self::MetallicRoughness => "metallic_roughness",
            Self::BaseSampler => "base_sampler",
            Self::MrSampler => "mr_sampler",
        }
    }
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
    pub(crate) fn buffer_type(self) -> Option<&'static str> {
        match self {
            Self::Camera => Some("Camera"),
            Self::Models => Some("AraMatrices"),
            Self::Normals => Some("AraNormals"),
            Self::Material => Some("Material"),
            _ => None,
        }
    }
    pub(crate) fn array(self) -> bool {
        matches!(self, Self::Models | Self::Normals)
    }
}
