//! One semantic table used by the host packet and both shader interfaces.
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub(crate) enum SkinResource {
    Source,
    Influences,
    Palette,
    Vertices,
    Previous,
    Params,
    TangentSource,
    Tangents,
    Mappings,
    ClusterData,
}
impl SkinResource {
    pub(crate) const ALL: [Self; 10] = [
        Self::Source,
        Self::Influences,
        Self::Palette,
        Self::Vertices,
        Self::Previous,
        Self::Params,
        Self::TangentSource,
        Self::Tangents,
        Self::Mappings,
        Self::ClusterData,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Influences => "influences",
            Self::Palette => "palette",
            Self::Vertices => "vertices",
            Self::Previous => "previous",
            Self::Params => "params",
            Self::TangentSource => "tangent_source",
            Self::Tangents => "tangents",
            Self::Mappings => "mappings",
            Self::ClusterData => "cluster_data",
        }
    }
    pub(crate) fn ty(self) -> &'static str {
        match self {
            Self::Source | Self::Vertices | Self::Previous => "AraFloats",
            Self::Influences => "AraInfluences",
            Self::Palette => "AraMatrices",
            Self::Params => "Params",
            Self::TangentSource | Self::Tangents => "AraTangents",
            Self::Mappings => "AraMappings",
            Self::ClusterData => "AraWords",
        }
    }
    pub(crate) fn writes(self) -> bool {
        matches!(
            self,
            Self::Vertices | Self::Previous | Self::Tangents | Self::ClusterData
        )
    }
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
    pub(crate) fn metal_slot(self) -> String {
        format!(
            "top.buffers[top.root[0]].data[(top.root[1] >> 2) + {}]",
            self as usize
        )
    }
    pub(crate) fn spirv_slot(self) -> String {
        format!(
            "ara_skin_table[ara_root.w0].values[(ara_root.w1 >> 2u) + {}u]",
            self as usize
        )
    }
}
