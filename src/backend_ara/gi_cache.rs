//! The world radiance cache's kernels, composed as the wgpu renderer composes them.
//!
//! Sky visibility first (`IndirectSettings` with bounce and the filter off): visible
//! pixels request cells, claims insert them, a scan refreshes the rest on a rotation,
//! hardware rays resolve each cell's direction bits and an integration turns the bits
//! into the occluded sky's SH. Nothing here records or replays yet; this is the
//! interface the adapter reflects.
use super::shader::{Pass, Program};
use anyhow::{Result, ensure};

/// The sky-visibility entries, in `GiState::encode` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    /// Visible pixels look their cells up and request the missing ones.
    Probe,
    /// One round of `atomicMax` claims for requested cells.
    Claim,
    /// The winner of each claim writes its cell; losers retry next round.
    Resolve,
    /// The third round's resolve, which also closes the frame's requests.
    ResolveLast,
    /// Queues new and invalidated cells, and the rotation's share of the rest.
    Scan,
    /// Lays the queued cells' visibility rays out as ray items.
    Setup,
    /// One visibility ray per item, in parallel.
    Rays,
    /// Folds the ray results into each cell's direction bits.
    Finish,
    /// The occluded sky as fixed-weight L2 SH per cell.
    Integrate,
}
impl Entry {
    /// Every sky-visibility entry, in encode order.
    pub const ALL: [Self; 9] = [
        Self::Probe,
        Self::Claim,
        Self::Resolve,
        Self::ResolveLast,
        Self::Scan,
        Self::Setup,
        Self::Rays,
        Self::Finish,
        Self::Integrate,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Probe => "gi_probe",
            Self::Claim => "gi_claim",
            Self::Resolve => "gi_resolve",
            Self::ResolveLast => "gi_resolve_last",
            Self::Scan => "gi_scan",
            Self::Setup => "gi_setup",
            Self::Rays => "gi_rays",
            Self::Finish => "gi_finish",
            Self::Integrate => "gi_integrate",
        }
    }
    /// The hardware-ray kernel module with the default direction count, and this
    /// entry selected.
    pub fn program(self) -> Result<Program> {
        Program::new(Pass::GiCache(self), source()?)
    }
}

/// `gi_cache::kernel_source` for hardware rays, with its one hit-position intrinsic
/// replaced by a fetch from the module's own per-draw vertex and index arrays (the
/// hit's placement names its draw, as `gi_receiver` already resolves it), and the
/// vertex-return qualifiers dropped. Traversal and everything else are unchanged.
pub(crate) fn source() -> Result<String> {
    let original = crate::renderer::gi_cache::kernel_source(
        false,
        crate::IndirectSettings::default().directions,
    );
    ensure!(
        original
            .matches("getCommittedHitVertexPositions(&query)")
            .count()
            == 1
            && original.matches("HitVertexPositions").count() == 1
            && original.matches("ray_query<vertex_return>").count() == 2
            && original
                .matches("acceleration_structure<vertex_return>")
                .count()
                == 1,
        "original radiance cache hit-position seams changed"
    );
    Ok(format!(
        "{}\n{FETCH}",
        original
            .replace(
                "getCommittedHitVertexPositions(&query)",
                "gi_hit_vertices(primary_placements[hit.instance_custom_data]&0x1fffffffu,hit.primitive_index)",
            )
            .replace("ray_query<vertex_return>", "ray_query")
            .replace("acceleration_structure<vertex_return>", "acceleration_structure")
    ))
}
/// Object-space corners of one triangle from the retained 48-byte vertex records, three
/// vec4 per vertex with the position first, in the build's primitive order.
const FETCH: &str = "fn gi_hit_vertices(batch:u32,primitive:u32)->array<vec3<f32>,3> {
    let i=primitive*3u;
    return array<vec3<f32>,3>(gi_vertices[batch].data[gi_indices[batch].data[i]*3u].xyz,
        gi_vertices[batch].data[gi_indices[batch].data[i+1u]*3u].xyz,
        gi_vertices[batch].data[gi_indices[batch].data[i+2u]*3u].xyz);
}
";
