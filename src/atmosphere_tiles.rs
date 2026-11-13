//! Fog injection's dispatch shape; samples remain indexed by the global cell ID.
const ORIGINAL: &str = "@compute @workgroup_size(4,4,4) fn at_fog_inject";
const COHERENT: &str = "@compute @workgroup_size(8,4,2) fn at_fog_inject";

fn enabled() -> bool {
    super::options::mode() == super::options::Mode::Coherent
}

pub(super) fn source_for(source: String, tiled: bool) -> String {
    if !tiled {
        return source;
    }
    assert_eq!(source.matches(ORIGINAL).count(), 1);
    // Each 32-lane warp visits adjacent columns in one depth slice. The original
    // 4x4x4 group splits a warp across two slices. Both shapes still have 64 lanes.
    source.replacen(ORIGINAL, COHERENT, 1)
}

pub(super) fn groups(grid: [u32; 2]) -> [u32; 3] {
    groups_for(grid, enabled())
}

fn groups_for(grid: [u32; 2], tiled: bool) -> [u32; 3] {
    let [x, y, z] = if tiled { [8, 4, 2] } else { [4, 4, 4] };
    [
        grid[0].div_ceil(x),
        grid[1].div_ceil(y),
        super::FROXEL_SLICES.div_ceil(z),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "backend-ara")]
    #[test]
    fn native_backend_reflects_the_injection_shape() {
        use crate::backend_ara::shader::Pass;
        let expected = if enabled() { [8, 4, 2] } else { [4, 4, 4] };
        assert_eq!(Pass::AtmosphereFogInject.threads().unwrap(), expected);
        assert_eq!(Pass::AtmosphereFogTemporal.threads().unwrap(), [4, 4, 4]);
    }

    #[test]
    fn off_is_identical_and_only_injection_shape_changes() {
        for active in [
            super::super::kernel_source(true),
            super::super::kernel_source(false),
            super::super::virtual_kernel_source(),
        ] {
            let original = active.replace(COHERENT, ORIGINAL);
            assert_eq!(source_for(original.clone(), false), original);
            let tiled = source_for(original.clone(), true);
            assert_eq!(tiled.replace(COHERENT, ORIGINAL), original);
            assert_eq!(tiled.matches(COHERENT).count(), 1);
        }
    }

    #[test]
    fn every_shadow_source_validates_and_dispatch_matches_shader() {
        for active in [
            super::super::kernel_source(true),
            super::super::kernel_source(false),
            super::super::virtual_kernel_source(),
        ] {
            for tiled in [false, true] {
                let source = source_for(active.replace(COHERENT, ORIGINAL), tiled);
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap();
                let inject = module
                    .entry_points
                    .iter()
                    .find(|e| e.name == "at_fog_inject")
                    .unwrap();
                let temporal = module
                    .entry_points
                    .iter()
                    .find(|e| e.name == "at_fog_temporal")
                    .unwrap();
                assert_eq!(temporal.workgroup_size, [4, 4, 4]);
                for grid in [[1, 1], [53, 47], [240, 135]] {
                    let groups = groups_for(grid, tiled);
                    let extent = [grid[0], grid[1], super::super::FROXEL_SLICES];
                    for axis in 0..3 {
                        let threads = inject.workgroup_size[axis];
                        assert!(groups[axis] * threads >= extent[axis]);
                        assert!((groups[axis] - 1) * threads < extent[axis]);
                    }
                }
            }
        }
    }
}
