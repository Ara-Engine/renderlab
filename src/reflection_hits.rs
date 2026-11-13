//! Reflection hit shading (docs/design-reflection-pass.md): the reduced-rate reflection
//! pass's hits, shaded as their own compacted pass instead of inside the camera shading
//! groups, so a SIMD group no longer pays for its pixel with the most strata.
//!
//! U-RTVIS's `rl_trace` appends one record per hit to the list H ([`HEADER_BYTES`], then
//! [`RECORD_BYTES`] each). This pass bins the records by the hit's program, then shades each
//! bin with that program's hit module (`program::compose_hits`: the traced module with the
//! `reflection_hits` entry of shaders/reflection_hits.wgsl) and writes the radiance into
//! the sample image R at each record's slot, which `rl_resolve` accumulates.

/// The hit-shading entry, appended to a program's traced module by `program::compose_hits`.
pub(crate) const ENTRY: &str = include_str!("../shaders/reflection_hits.wgsl");
/// H's header: the record count, then the dispatch arguments.
pub(crate) const HEADER_BYTES: u64 = 16;
/// One hit record: three `vec4<u32>` (the design's word table).
pub(crate) const RECORD_BYTES: u64 = 48;
/// Lanes per workgroup. The port's binning runner reads it (wgpu's mixed frames filter per
/// program instead, reflection_hits_gpu.rs).
#[cfg(any(test, feature = "backend-ara"))]
pub(crate) const LANES: u32 = 64;
/// Group-0 bindings of the hit entry: H (read) and R (written), in the traced shading group
/// while the reflection pass is on (U-RTVIS's L is at 56-58 there).
pub(crate) const HITS: u32 = 64;
pub(crate) const SAMPLES: u32 = 65;

/// The traced shading layout's hit entries: H read-only, R write-only.
pub(crate) fn shade_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        wgpu::BindGroupLayoutEntry {
            binding: HITS,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(HEADER_BYTES + RECORD_BYTES),
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: SAMPLES,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        },
    ]
}

/// The binning kernel (shaders/reflection_hit_bins.wgsl): each record's index into its
/// program's bin at `slot * stride` (the bin's length, then indices), with the bin's
/// indirect dispatch arguments. Program-agnostic, compiled once. The port's binning runner
/// only (wgpu's mixed frames filter per program instead).
#[cfg(any(test, feature = "backend-ara"))]
pub(crate) fn bin_source() -> String {
    format!(
        "const PRIMARY_BATCHES:u32={}u;\n{}",
        crate::TRACED_DRAWS,
        include_str!("../shaders/reflection_hit_bins.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{self, Stage};

    /// Every program's hit module composes and validates on both paths' heads, traced and
    /// streamed, with the `reflection_hits` entry, including a program that traces no lobe
    /// of its own (toon): any program's draw can be hit.
    #[test]
    fn hit_modules_validate_on_both_heads() {
        const SCENE: &str = include_str!("../shaders/scene.wgsl");
        let rt = super::super::primary_ray_gpu::shading_heads(SCENE).unwrap();
        let (_, non_rt) = super::super::primary_ray_gpu::non_rt_sources(SCENE).unwrap();
        for (label, source) in [
            ("standard", program::STANDARD_PROGRAM),
            ("toon", include_str!("../shaders/programs/toon.wgsl")),
        ] {
            let parameters = program::declares_parameters(source);
            for head in rt.iter().chain(&non_rt) {
                for (stage, lists) in [Stage::Traced, Stage::TracedStreamed]
                    .into_iter()
                    .flat_map(|stage| [(stage, false), (stage, true)])
                {
                    let composed = program::compose_hits(head, source, stage, parameters, lists);
                    // Mixed frames keep only their own slot's hits (MP1's RlList at 38).
                    assert_eq!(composed.source.contains("rl_list.slot"), lists);
                    assert_eq!(&composed.source[composed.program.clone()], source);
                    let module =
                        naga::front::wgsl::parse_str(&composed.source).unwrap_or_else(|e| {
                            panic!("{label}: {}", e.emit_to_string(&composed.source))
                        });
                    naga::valid::Validator::new(
                        naga::valid::ValidationFlags::all(),
                        naga::valid::Capabilities::all(),
                    )
                    .validate(&module)
                    .unwrap_or_else(|e| {
                        panic!("{label} {stage:?}: {}", e.emit_to_string(&composed.source))
                    });
                    assert!(
                        module
                            .entry_points
                            .iter()
                            .any(|e| e.name == "reflection_hits"),
                        "{label}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_binning_kernel_validates() {
        let source = bin_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    }
    /// The reflection pass's other modules validate on both paths' heads: each head's traced
    /// shading of the standard program (with the lobe output while `RENDERLAB_RL_PASS=1`) and
    /// U-RTVIS's `rl_trace` module (`rl_trace` and `rl_hit_args`). Run it with the variable
    /// set to cover the pass's composition.
    #[test]
    fn pass_modules_validate_on_both_heads() {
        const SCENE: &str = include_str!("../shaders/scene.wgsl");
        let rt = super::super::primary_ray_gpu::shading_heads(SCENE).unwrap();
        let (_, non_rt) = super::super::primary_ray_gpu::non_rt_sources(SCENE).unwrap();
        let parameters = program::declares_parameters(program::STANDARD_PROGRAM);
        for head in rt.iter().chain(&non_rt) {
            let shading =
                program::compose(head, program::STANDARD_PROGRAM, Stage::Traced, parameters).source;
            for (label, source) in [
                ("shading", shading),
                ("trace", program::rl_trace_source(head)),
            ] {
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
            }
        }
    }
}
