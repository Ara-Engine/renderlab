//! Selectable primary visibility and the CPU side of the traced hit-record layout.
use crate::Error;

/// How lit 1x regular-geometry frames resolve camera visibility. Raster is the
/// portable default. A traced-capable hardware-shadow renderer at 1x starts with
/// `Traced`, Interactive shadows, TR1 and its adaptive 256/64/32/8 shadow-ray budget.
/// A no-RT device requesting visibility-buffer and Virtual-shadow
/// capabilities selects `VisibilityBuffer` and `SoftShadows::Virtual` at creation.
/// Diagnostic views and virtual geometry still rasterize through the forward path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrimaryVisibility {
    #[default]
    Raster,
    /// One closest-hit camera ray per pixel centre against the hardware-shadow TLAS,
    /// then deferred compute shading of the hit record with each material's program.
    /// Requires a hardware-shadow renderer at 1x whose device enables
    /// [`TRACED_FEATURES`] and [`traced_limits`], and at most [`TRACED_DRAWS`]
    /// regular opaque double-sided draws. Diagnostic views and virtual geometry
    /// still rasterize.
    Traced,
    /// A per-frame cluster cut (original geometry, or an attached clusterLOD hierarchy at
    /// `Renderer::set_lod_error_pixels`) rasterized into a 64-bit depth/ID buffer,
    /// followed by the same exact receiver repair and programmable deferred shading as
    /// traced primaries. At 1x, requires [`VISIBILITY_FEATURES`],
    /// [`VISIBILITY_RASTER_FEATURES`] and [`visibility_limits`], but no RT.
    VisibilityBuffer,
}

/// Descriptor indexing for deferred materials; no acceleration-structure features.
pub const VISIBILITY_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_BINDING_ARRAY
    .union(wgpu::Features::BUFFER_BINDING_ARRAY)
    .union(wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY)
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING);
/// 64-bit storage max atomics for the visibility buffer's (depth, cluster, triangle)
/// words, written by its software and hardware rasters.
pub const VISIBILITY_RASTER_FEATURES: wgpu::Features =
    wgpu::Features::SHADER_INT64.union(wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX);

/// Deferred material arrays and five resolve storage targets (hits, world, normal,
/// coverage and receiver epsilon), thirteen storage buffers per stage for the Virtual
/// shadow lookup, including SK1 history, radiance-cache and DC1 decal tables, and 24
/// sampled textures per stage (LC1: the lighting group's AT1 sky and LC1 cluster/shadow
/// textures take the temporal film layout to 18). Cache-terminated reflection hits
/// (`RENDERLAB_RL_CACHE_HITS`, src/program.rs) bind one more read-only storage buffer to the
/// lighting group, so the switch asks for one more; so does the reflection pass
/// (`RENDERLAB_RL_PASS=1`, or the non-RT path's default), whose hit list H joins the traced
/// shading group. An adapter with
/// thirteen runs neither.
/// No acceleration-structure limits are raised. Check these against the adapter limits
/// before requesting a device; devices below them keep the portable raster path.
pub fn visibility_limits(limits: wgpu::Limits) -> wgpu::Limits {
    let storage = 13
        + u32::from(crate::program::cache_hits_rough().is_some())
        + crate::renderer::primary_ray_gpu::reflection_pass::STORAGE_BUFFERS * u32::from(crate::program::rl_pass_env() == Some(true));
    wgpu::Limits {
        max_storage_textures_per_shader_stage: limits.max_storage_textures_per_shader_stage.max(if crate::material_mode::Mode::current() == crate::material_mode::Mode::Off { 5 } else { 6 }),
        max_storage_buffers_per_shader_stage: limits.max_storage_buffers_per_shader_stage.max(storage),
        max_sampled_textures_per_shader_stage: limits.max_sampled_textures_per_shader_stage.max(24),
        max_binding_array_elements_per_shader_stage: limits
            .max_binding_array_elements_per_shader_stage
            .max(12 * TRACED_DRAWS),
        max_binding_array_sampler_elements_per_shader_stage: limits
            .max_binding_array_sampler_elements_per_shader_stage
            .max(3 * TRACED_DRAWS),
        ..limits
    }
}

/// Draws one traced frame can bind: descriptor-indexed geometry and material slots.
pub const TRACED_DRAWS: u32 = 256;
/// Device features traced visibility needs in addition to hardware ray queries.
/// Hit-vertex return also marks every acceleration structure such a renderer builds.
pub const TRACED_FEATURES: wgpu::Features =
    VISIBILITY_FEATURES.union(wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN);
/// Raises `limits` to the per-stage binding-array capacity traced visibility needs per
/// draw slot: three geometry buffers, the material factor and program parameter buffers,
/// three textures and three samplers (wgpu counts samplers in both limits). The Interactive trace also binds two
/// views of the TLAS: camera rays with hit-vertex return and shadow rays without.
pub fn traced_limits(limits: wgpu::Limits) -> wgpu::Limits {
    let limits = visibility_limits(limits);
    wgpu::Limits {
        max_acceleration_structures_per_shader_stage: limits
            .max_acceleration_structures_per_shader_stage
            .max(2),
        ..limits
    }
}

/// Per TLAS placement, in placement order: its draw (batch) index, with bit 31 set when
/// that draw has a tangent stream, bit 30 for single-sided, bit 29 for thin.
/// `draws` yields instance count, tangent presence, single-sidedness and thinness.
pub(crate) fn placement_table(
    draws: impl Iterator<Item = (usize, bool, bool, bool)>,
) -> Result<Vec<u32>, Error> {
    let mut table = vec![];
    for (batch, (instances, tangent, single_sided, thin)) in draws.enumerate() {
        if batch >= TRACED_DRAWS as usize {
            return Err(Error::InvalidInput(
                "traced primary visibility supports at most TRACED_DRAWS draws",
            ));
        }
        table.extend(std::iter::repeat_n(
            batch as u32
                | (u32::from(tangent) << 31)
                | (u32::from(single_sided) << 30)
                | (u32::from(thin) << 29),
            instances,
        ));
    }
    Ok(table)
}

/// Bit 31 of a draw's batch word: its previous-vertex stream is a skinned one.
pub(crate) const BATCH_SKINNED: u32 = 1 << 31;
/// Bit 30 of a draw's batch word: its index buffer carries the adjacency suffix.
pub(crate) const BATCH_ADJACENT: u32 = 1 << 30;
/// One draw's geometry facts for the shaders (`primary_batch_info`): the triangle count,
/// `BATCH_ADJACENT` for an index buffer of 6T + 1 words (receiver_adjacency.rs) rather than
/// 3T, and `BATCH_SKINNED`. The shaders once read these from `arrayLength` of the draw's
/// element of a buffer binding array, which a driver may answer for another element.
pub(crate) fn batch_word(index_words: u64, skinned: bool) -> u32 {
    let adjacent = index_words % 3 == 1;
    let triangles = if adjacent {
        (index_words - 1) / 6
    } else {
        index_words / 3
    };
    debug_assert!(triangles < u64::from(BATCH_ADJACENT));
    triangles as u32
        | if adjacent { BATCH_ADJACENT } else { 0 }
        | if skinned { BATCH_SKINNED } else { 0 }
}
/// The placement buffer's words: `placements` (one zero word for an empty frame), then
/// `TRACED_DRAWS` batch words counted back from the end, draw 0 last, so a shader finds
/// draw `b` at `arrayLength - 1 - b` whatever the placement count.
pub(crate) fn placement_words(placements: &[u32], batches: &[u32]) -> Vec<u32> {
    let mut words = placements.to_vec();
    words.resize(placements.len().max(1) + TRACED_DRAWS as usize, 0);
    let end = words.len();
    for (batch, &word) in batches.iter().enumerate() {
        words[end - 1 - batch] = word;
    }
    words
}
/// The shaders' reader of the batch words, cut from a shader file that carries it.
#[cfg(test)]
pub(crate) fn batch_info_source(shader: &'static str) -> &'static str {
    let (_, rest) = shader.split_once("// renderlab: batch info begin").unwrap();
    rest.split_once("// renderlab: batch info end").unwrap().0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_words_follow_the_buffers_and_sit_at_the_end() {
        // 4 triangles: 12 words without adjacency, 25 with.
        assert_eq!(batch_word(12, false), 4);
        assert_eq!(batch_word(25, false), 4 | BATCH_ADJACENT);
        assert_eq!(batch_word(12, true), 4 | BATCH_SKINNED);
        assert_eq!(batch_word(0, false), 0);
        let words = placement_words(&[7, 8, 9], &[11, 12]);
        assert_eq!(words.len(), 3 + TRACED_DRAWS as usize);
        assert_eq!(words[..3], [7, 8, 9]);
        assert_eq!(words[words.len() - 1], 11);
        assert_eq!(words[words.len() - 2], 12);
        assert_eq!(placement_words(&[], &[]).len(), 1 + TRACED_DRAWS as usize);
    }
    #[test]
    fn both_shader_copies_of_the_batch_reader_are_one_text() {
        assert_eq!(
            batch_info_source(include_str!("../shaders/primary_rays.wgsl")),
            batch_info_source(include_str!("../shaders/virtual_shadow.wgsl"))
        );
    }
    #[test]
    fn visibility_and_traced_limits_include_virtual_lighting() {
        let limits = visibility_limits(wgpu::Limits::default());
        // One more each while RENDERLAB_RL_CACHE_HITS or RENDERLAB_RL_PASS is set.
        let storage = 13
            + u32::from(crate::program::cache_hits_rough().is_some())
            + crate::renderer::primary_ray_gpu::reflection_pass::STORAGE_BUFFERS * u32::from(crate::program::rl_pass_env() == Some(true));
        assert_eq!(limits.max_storage_buffers_per_shader_stage, storage);
        assert_eq!(limits.max_sampled_textures_per_shader_stage, 24);
        assert_eq!(
            traced_limits(wgpu::Limits::default()).max_storage_buffers_per_shader_stage,
            storage
        );
        let higher = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 16,
            ..limits
        };
        assert_eq!(visibility_limits(higher.clone()), higher);
    }
    #[test]
    fn placements_name_their_draw_and_tangent_stream() {
        let draws = [
            (2, false, false, false),
            (0, true, true, false),
            (1, true, true, true),
        ];
        let table = placement_table(draws.into_iter()).unwrap();
        assert_eq!(table, [0, 0, 2 | 1 << 31 | 1 << 30 | 1 << 29]);
        let many = std::iter::repeat_n((1, false, false, false), TRACED_DRAWS as usize + 1);
        assert!(placement_table(many).is_err());
        let limits = traced_limits(wgpu::Limits::default());
        assert_eq!(
            limits.max_binding_array_elements_per_shader_stage,
            12 * TRACED_DRAWS
        );
        assert_eq!(limits.max_acceleration_structures_per_shader_stage, 2);
    }
}
