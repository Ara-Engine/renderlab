//! Original camera traces, with portable hit-position fetches from retained meshes.
use super::shader::memo::Memo;
use super::shader::{Pass, Program};
use anyhow::{Context, Result, ensure};
/// Original camera trace output contract.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
        static KEPT: Memo<Entry, Program> = Memo::new();
        KEPT.get(self, || {
            Program::new(
                if matches!(self, Self::Trace) {
                    Pass::PrimaryTrace
                } else {
                    Pass::PrimaryTraceReceivers
                },
                source()?,
            )
        })
    }
}
pub(crate) fn source() -> Result<String> {
    let [hard, _] =
        crate::renderer::primary_ray_gpu::shader_sources(include_str!("../../shaders/scene.wgsl"))?;
    let head = hard
        .strip_suffix(TRACE)
        .context("original primary trace composition changed")?;
    let trace = format!("{head}{}", portable_trace()?);
    if crate::program::material_pass_on() {
        material_receivers_edits(trace, crate::program::material_trace_tail())
    } else {
        Ok(trace)
    }
}
/// RENDERLAB_MATERIAL_PASS=1, the non-interactive trace (`primary_trace_receivers`, whose hits
/// are repaired already): the miss sentinel on every pixel without a hit, and with `tail` the
/// receiver words on hits (the point is the repaired one).
fn material_receivers_edits(trace: String, tail: bool) -> Result<String> {
    let mut edits: Vec<(&str, &str)> = vec![(
        "    if !primary_tile_covered(trace.record.x!=0u,id.xy,group.xy) || any(id.xy>=size) {return;}\n    let p=vec2<i32>(id.xy);\n    textureStore(primary_hit_output,p,trace.record);",
        "    let rl_covered=primary_tile_covered(trace.record.x!=0u,id.xy,group.xy);\n    if all(id.xy<size) && trace.record.x==0u {rl_receiver_miss(vec2<i32>(id.xy));}\n    if !rl_covered || any(id.xy>=size) {return;}\n    let p=vec2<i32>(id.xy);\n    textureStore(primary_hit_output,p,trace.record);",
    )];
    if tail {
        edits.push((
            "    let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);\n    textureStore(primary_world_output,",
            "    let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);\n    rl_receiver_store(p,trace,receiver);\n    textureStore(primary_world_output,",
        ));
    }
    let mut trace = trace;
    for (seam, with) in edits {
        ensure!(trace.matches(seam).count() == 1, "receivers trace seam changed: {seam}");
        trace = trace.replacen(seam, with, 1);
    }
    trace.push_str(&crate::program::material_trace_writes());
    Ok(trace)
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
    Ok(without_vertex_return(trace))
}
/// `source` with the trace text's hit-vertex return replaced as `portable_trace` does: also
/// for the material pass's modules, which carry the trace's edge repair
/// (program.rs `material_trace_helpers`).
pub(crate) fn without_vertex_return(source: &str) -> String {
    source.replace("getCandidateHitVertexPositions(&query)",
        "primary_triangle_vertices(primary_placements[candidate.instance_custom_data]&0x1fffffffu,candidate.primitive_index)")
        .replace("getCommittedHitVertexPositions(&query)",
        "primary_triangle_vertices(primary_placements[hit.instance_custom_data]&0x1fffffffu,hit.primitive_index)")
        .replace("ray_query<vertex_return>","ray_query")
        .replace("acceleration_structure<vertex_return>","acceleration_structure")
}

/// The fused Interactive pass: each pixel's camera hit and its sun visibility in one
/// dispatch, with pixels near a triangle edge listed for a repair pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interactive {
    /// Trace and estimate every pixel; list near-edge pixels in the repair queue.
    Deferred,
    /// The same without deferral, for a repair queue above the device's limits.
    Full,
    /// Re-trace the listed pixels exactly; dispatched indirectly from the queue.
    Repair,
}
impl Interactive {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Deferred => "primary_trace_interactive_deferred",
            Self::Full => "primary_trace_interactive",
            Self::Repair => "primary_repair_interactive",
        }
    }
    /// The entry in the production soft module, or with `rotated` in temporal
    /// reconstruction's variant of it: disk directions rotated by the frame's sample
    /// and a ray count read from the primary frame record's sampling words.
    pub fn program(self, rotated: bool) -> Result<Program> {
        static KEPT: Memo<(Interactive, bool), Program> = Memo::new();
        KEPT.get((self, rotated), || {
            Program::new(Pass::PrimaryInteractive(self), interactive_source(rotated)?)
        })
    }
}
/// `shader_sources`' soft module with the portable trace, eight directions as the
/// renderer's pipeline override sets, and optionally `trace_source`'s two edits.
pub(crate) fn interactive_source(rotated: bool) -> Result<String> {
    let [_, soft] =
        crate::renderer::primary_ray_gpu::shader_sources(include_str!("../../shaders/scene.wgsl"))?;
    let marker = "override FRESH_DIRECTIONS:u32=256u;";
    ensure!(
        soft.matches(TRACE).count() == 1 && soft.matches(marker).count() == 1,
        "original soft kernel composition changed"
    );
    let soft = soft
        .replace(TRACE, &portable_trace()?)
        .replace(marker, "override FRESH_DIRECTIONS:u32=8u;");
    let soft = if rotated {
        crate::renderer::primary_ray_gpu::temporal_reconstruction::trace_source(&soft)?
    } else {
        soft
    };
    if crate::program::material_pass_on() {
        material_trace_edits(soft, crate::program::material_trace_tail())
    } else {
        Ok(soft)
    }
}
/// RENDERLAB_MATERIAL_PASS=1: the interactive kernels write the material record's miss sentinel
/// on every pixel without a camera hit (the dispatch covers the whole image, the region's
/// exterior included), and with `tail` (`program::material_trace_tail`) the receiver words on
/// hits, the repair kernel rewriting them where it moves a hit.
fn material_trace_edits(soft: String, tail: bool) -> Result<String> {
    let mut edits: Vec<(&str, String)> = vec![
        (
            "    if !inside || covered==0u {return;}\n",
            "    if inside && trace.record.x==0u {rl_receiver_miss(vec2<i32>(id.xy));}\n    if !inside || covered==0u {return;}\n".to_owned(),
        ),
        (
            "    if !in_bounds || covered==0u {return;}\n",
            "    if in_bounds && trace.record.x==0u {rl_receiver_miss(vec2<i32>(id.xy));}\n    if !in_bounds || covered==0u {return;}\n".to_owned(),
        ),
    ];
    if tail {
        edits.extend([
            (
                "        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);\n        visible=fresh_estimate(",
                "        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);\n        rl_receiver_store(p,trace,receiver);\n        visible=fresh_estimate(".to_owned(),
            ),
            (
                "trace.record.x,trace.record.y,trace.record.z,trace.record.w);\n        }\n",
                "trace.record.x,trace.record.y,trace.record.z,trace.record.w);\n        }\n        rl_receiver_store(p,trace,receiver);\n".to_owned(),
            ),
            (
                "    let receiver=primary_receiver(trace.point,trace.positions,slot,p);\n",
                "    let receiver=primary_receiver(trace.point,trace.positions,slot,p);\n    rl_receiver_store(p,trace,receiver);\n".to_owned(),
            ),
        ]);
    }
    let mut soft = soft;
    for (seam, with) in edits {
        ensure!(soft.matches(seam).count() == 1, "interactive kernel seam changed: {seam}");
        soft = soft.replacen(seam, &with, 1);
    }
    soft.push_str(&crate::program::material_trace_writes());
    Ok(soft)
}
