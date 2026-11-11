//! Original camera traces, with portable hit-position fetches from retained meshes.
use super::shader::{Pass, Program};
use anyhow::{Context, Result, ensure};
/// Original camera trace output contract.
#[derive(Clone, Copy)]
pub enum Entry {
    /// Hit records, tile coverage and hit-only geometric precision.
    Trace,
    /// Hit records, tile coverage and hit-only world/normal receiver records.
    Receivers,
}
impl Entry {
    /// Compose the unchanged hardware traversal/culling/repair algorithms, reading
    /// hit positions from the same vertex/index version used to build the BLAS.
    pub fn program(self) -> Result<Program> {
        Program::new(
            if matches!(self, Self::Trace) {
                Pass::PrimaryTrace
            } else {
                Pass::PrimaryTraceReceivers
            },
            source()?,
        )
    }
}
pub(crate) fn source() -> Result<String> {
    let [hard, _] =
        crate::renderer::primary_ray_gpu::shader_sources(include_str!("../../shaders/scene.wgsl"))?;
    let head = hard
        .strip_suffix(TRACE)
        .context("original primary trace composition changed")?;
    Ok(format!("{head}{}", portable_trace()?))
}
const TRACE: &str = include_str!("../../shaders/primary_trace.wgsl");

/// `primary_trace.wgsl` without hit-vertex return: its three position intrinsics read
/// the retained vertex and index arrays through the hit's placement, and its query and
/// scene types lose the qualifier. Every module that contains the trace text needs
/// this, reachable or not, or its SPIR-V declares `SPV_KHR_ray_tracing_position_fetch`.
pub(crate) fn portable_trace() -> Result<String> {
    let trace = TRACE;
    ensure!(
        trace
            .matches("getCandidateHitVertexPositions(&query)")
            .count()
            == 2
            && trace
                .matches("getCommittedHitVertexPositions(&query)")
                .count()
                == 1
            && trace.matches("ray_query<vertex_return>").count() == 2
            && trace
                .matches("acceleration_structure<vertex_return>")
                .count()
                == 1,
        "original primary hit-position seams changed"
    );
    Ok(trace.replace("getCandidateHitVertexPositions(&query)",
        "primary_triangle_vertices(primary_placements[candidate.instance_custom_data]&0x1fffffffu,candidate.primitive_index)")
        .replace("getCommittedHitVertexPositions(&query)",
        "primary_triangle_vertices(primary_placements[hit.instance_custom_data]&0x1fffffffu,hit.primitive_index)")
        .replace("ray_query<vertex_return>","ray_query")
        .replace("acceleration_structure<vertex_return>","acceleration_structure"))
}
