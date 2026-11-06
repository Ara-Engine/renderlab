//! Selectable primary visibility and the CPU side of the traced hit-record layout.
use crate::Error;

/// How lit 1x regular-geometry frames resolve camera visibility. Raster is the
/// portable default. A no-RT device requesting visibility-buffer and Virtual-shadow
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
    /// Full-detail cluster rasterization into IDs and depth, followed by the same
    /// exact receiver repair and programmable deferred shading as traced primaries.
    /// At 1x, requires [`VISIBILITY_FEATURES`] and [`visibility_limits`], but no RT.
    VisibilityBuffer,
}

/// Descriptor indexing for deferred materials; no acceleration-structure features.
pub const VISIBILITY_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_BINDING_ARRAY
    .union(wgpu::Features::BUFFER_BINDING_ARRAY)
    .union(wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY)
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING);

/// Deferred material arrays and five resolve storage targets (hits, world, normal,
/// coverage and receiver epsilon). No acceleration-structure limits are raised.
pub fn visibility_limits(limits: wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_storage_textures_per_shader_stage: limits.max_storage_textures_per_shader_stage.max(5),
        max_binding_array_elements_per_shader_stage: limits
            .max_binding_array_elements_per_shader_stage
            .max(11 * TRACED_DRAWS),
        max_binding_array_sampler_elements_per_shader_stage: limits
            .max_binding_array_sampler_elements_per_shader_stage
            .max(3 * TRACED_DRAWS),
        ..limits
    }
}

/// Draws one traced frame can bind: descriptor-indexed geometry and material slots.
pub const TRACED_DRAWS: u32 = 64;
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

#[cfg(test)]
mod tests {
    use super::*;
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
            11 * TRACED_DRAWS
        );
        assert_eq!(limits.max_acceleration_structures_per_shader_stage, 2);
    }
}
