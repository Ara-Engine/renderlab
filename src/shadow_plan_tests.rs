use super::*;

fn camera(eye: Vec3) -> Camera {
    Camera::look_at(
        eye,
        eye + Vec3::NEG_Z,
        Vec3::Y,
        std::f32::consts::FRAC_PI_2,
        1.0,
        100.0,
    )
    .unwrap()
}

fn box_at(center: Vec3, half: f32) -> Bounds {
    Bounds {
        min: center - Vec3::splat(half),
        max: center + Vec3::splat(half),
    }
}

fn clip(view: &ShadowView, point: Vec3) -> Vec3 {
    let h = view.world_to_clip * point.extend(1.0);
    assert!(h.is_finite() && h.w > 0.0, "{h:?}");
    h.truncate() / h.w
}

fn inside(view: &ShadowView, point: Vec3, tolerance: f32) -> bool {
    let h = view.world_to_clip * point.extend(1.0);
    h.is_finite()
        && h.w > 0.0
        && h.x.abs() <= h.w * (1.0 + tolerance)
        && h.y.abs() <= h.w * (1.0 + tolerance)
        && h.z >= -h.w * tolerance
        && h.z <= h.w * (1.0 + tolerance)
}

fn assert_inside(view: &ShadowView, point: Vec3) {
    assert!(
        inside(view, point, 3e-5),
        "{:?}: {point:?} -> {:?}",
        view.kind,
        clip(view, point)
    );
}

// Independent fixture: a 90-degree, 2:1 camera looking -Z. No fitting helper is used.
fn slice_corners(origin: Vec3, near: f32, far: f32) -> Vec<Vec3> {
    [near, far]
        .into_iter()
        .flat_map(|d| {
            [-1.0, 1.0].into_iter().flat_map(move |x| {
                [-1.0, 1.0]
                    .into_iter()
                    .map(move |y| origin + Vec3::new(x * 2.0 * d, y * d, -d))
            })
        })
        .collect()
}

#[test]
fn directional_includes_offscreen_lightward_blocker_and_its_full_depth() {
    let camera = camera(Vec3::ZERO);
    let travel = Vec3::new(1.0, -1.0, 0.0).normalize();
    let receiver = Vec3::new(0.0, 0.0, -8.0);
    let blocker = receiver - 80.0 * travel;
    let camera_clip = camera.matrix(200, 100) * blocker.extend(1.0);
    assert!(camera_clip.x.abs() > camera_clip.w || camera_clip.y.abs() > camera_clip.w);
    let bounds = [
        box_at(receiver, 0.5),
        box_at(blocker, 0.5),
        box_at(receiver + Vec3::Z * 300.0, 0.5),
        box_at(receiver + travel * 300.0, 0.5),
    ];
    let plans = directional(&camera, [200, 100], travel, &[12.0], 512, &bounds).unwrap();
    let view = &plans[0];
    assert_eq!(view.caster_indices, [0, 1]);
    for bounds in &bounds[..2] {
        for p in bounds.corners() {
            assert_inside(view, p.as_vec3());
        }
    }
    for p in slice_corners(Vec3::ZERO, 1.0, 12.0) {
        assert_inside(view, p);
    }
    assert!(clip(view, blocker).z < clip(view, receiver).z);
}

#[test]
fn every_cascade_covers_both_boundary_planes_and_metadata_is_contiguous() {
    let camera = camera(Vec3::ZERO);
    let ends = [2.0, 8.0, 30.0, 100.0];
    for travel in [Vec3::NEG_Z, Vec3::Y, Vec3::new(0.7, -0.3, 0.8)] {
        let plans = directional(&camera, [200, 100], travel, &ends, 1024, &[]).unwrap();
        let mut near = 1.0;
        for (&far, view) in ends.iter().zip(&plans) {
            assert_eq!(view.kind, ViewKind::Cascade { near, far });
            for p in slice_corners(Vec3::ZERO, near, far) {
                assert_inside(view, p);
            }
            assert_eq!(view.resolution, 1024);
            assert!(view.caster_indices.is_empty());
            near = far;
        }
    }
    // Split count and resolution have no arbitrary product cap; no textures are allocated.
    let ends: Vec<_> = (2..=66).map(|n| n as f32).collect();
    assert_eq!(
        directional(&camera, [1, 1], Vec3::Y, &ends, u32::MAX, &[])
            .unwrap()
            .len(),
        65
    );
}

#[test]
fn subtexel_motion_is_stable_and_a_grid_crossing_moves_one_texel() {
    let first = directional(
        &camera(Vec3::ZERO),
        [200, 100],
        Vec3::NEG_Z,
        &[16.0],
        512,
        &[],
    )
    .unwrap()
    .remove(0);
    let texel = first.world_units_per_texel.unwrap();
    let right = first.world_to_view.inverse().transform_vector3(Vec3::X);
    let small = directional(
        &camera(right * (0.2 * texel)),
        [200, 100],
        Vec3::NEG_Z,
        &[16.0],
        512,
        &[],
    )
    .unwrap()
    .remove(0);
    let crossed = directional(
        &camera(right * (1.1 * texel)),
        [200, 100],
        Vec3::NEG_Z,
        &[16.0],
        512,
        &[],
    )
    .unwrap()
    .remove(0);
    let p = Vec3::new(0.5, 0.25, -5.0);
    assert_eq!(clip(&first, p).truncate(), clip(&small, p).truncate());
    assert_eq!(first.world_units_per_texel, crossed.world_units_per_texel);
    assert!((clip(&crossed, p).x - clip(&first, p).x + 2.0 / 512.0).abs() < 1e-6);
    let rotated_camera = Camera::look_at(
        Vec3::ZERO,
        Vec3::new(0.7, 0.0, -0.7),
        Vec3::Y,
        std::f32::consts::FRAC_PI_2,
        1.0,
        100.0,
    )
    .unwrap();
    let rotated = directional(&rotated_camera, [200, 100], Vec3::NEG_Z, &[16.0], 512, &[]).unwrap();
    assert_eq!(
        first.world_units_per_texel,
        rotated[0].world_units_per_texel
    );
}

#[test]
fn parallel_up_and_extreme_nonzero_travel_have_valid_right_handed_views() {
    for travel in [
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
        Vec3::new(1e-20, 1.0, 0.0),
        Vec3::splat(f32::MAX),
        Vec3::new(f32::from_bits(1), 0.0, 0.0),
    ] {
        let views = directional(&camera(Vec3::ZERO), [200, 100], travel, &[8.0], 64, &[]).unwrap();
        let view = &views[0];
        assert!((view.world_to_view.determinant() - 1.0).abs() < 2e-6);
        let forward = view
            .world_to_view
            .transform_vector3(travel.as_dvec3().normalize().as_vec3());
        assert!((forward - Vec3::NEG_Z).length() < 2e-6);
        for p in slice_corners(Vec3::ZERO, 1.0, 8.0) {
            assert_inside(view, p);
        }
    }
}

#[test]
fn spot_cone_rim_and_radial_near_far_are_covered() {
    let position = Vec3::new(2.0, -3.0, 4.0);
    let forward = Vec3::new(1.0, -2.0, -3.0).normalize();
    let tangent = forward.cross(Vec3::X).normalize();
    let bitangent = forward.cross(tangent).normalize();
    let outer = 0.6_f32;
    let range = LocalRange {
        near: 0.5,
        far: 20.0,
    };
    let mut points = vec![];
    for radius in [range.near, 3.0, range.far] {
        points.push(position + forward * radius);
        for step in 0..64 {
            let phase = step as f32 * std::f32::consts::TAU / 64.0;
            let direction = outer.cos() * forward
                + outer.sin() * (phase.cos() * tangent + phase.sin() * bitangent);
            points.push(position + radius * direction);
        }
    }
    let mut bounds: Vec<_> = points.iter().map(|&p| box_at(p, 0.0)).collect();
    bounds.push(box_at(position - 2.0 * forward, 0.1));
    bounds.push(box_at(position + 40.0 * forward, 0.1));
    let view = spot(position, forward, outer, range, 256, &bounds).unwrap();
    for (i, p) in points.into_iter().enumerate() {
        assert_inside(&view, p);
        assert!(view.caster_indices.contains(&i), "missing rim caster {i}");
    }
    assert!(!view.caster_indices.contains(&(bounds.len() - 2)));
    assert!(!view.caster_indices.contains(&(bounds.len() - 1)));
}

#[test]
fn six_point_faces_cover_axes_seams_corners_and_radial_shell() {
    let position = Vec3::new(2.0, 3.0, 4.0);
    let range = LocalRange {
        near: 0.125,
        far: 25.0,
    };
    let mut points = Vec::new();
    for x in -3..=3 {
        for y in -3..=3 {
            for z in -3..=3 {
                if (x, y, z) == (0, 0, 0) {
                    continue;
                }
                let direction = Vec3::new(x as f32, y as f32, z as f32).normalize();
                for radius in [range.near, 2.0, range.far] {
                    points.push(position + direction * radius);
                }
            }
        }
    }
    let bounds: Vec<_> = points.iter().map(|&p| box_at(p, 0.0)).collect();
    let views = point(position, range, 64, &bounds).unwrap();
    for (i, p) in points.into_iter().enumerate() {
        assert!(
            views
                .iter()
                .any(|v| inside(v, p, 3e-5) && v.caster_indices.contains(&i)),
            "uncovered {p:?}"
        );
    }
    let axes = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    let faces = [
        CubeFace::PositiveX,
        CubeFace::NegativeX,
        CubeFace::PositiveY,
        CubeFace::NegativeY,
        CubeFace::PositiveZ,
        CubeFace::NegativeZ,
    ];
    for ((view, axis), face) in views.iter().zip(axes).zip(faces) {
        assert_eq!(view.kind, ViewKind::Point(face));
        let middle = clip(view, position + axis * 2.0);
        assert!(middle.truncate().length() < 1e-6);
        for other in &views {
            if view.kind != other.kind {
                assert_ne!(view.world_to_clip, other.world_to_clip);
            }
        }
    }
}

#[test]
fn conservative_box_assignment_keeps_crossing_and_enclosing_bounds() {
    let bounds = [
        Bounds {
            min: Vec3::new(-20.0, -0.01, -3.1),
            max: Vec3::new(20.0, 0.01, -2.9),
        },
        box_at(Vec3::ZERO, 20.0),
        box_at(Vec3::new(20.0, 20.0, -3.0), 0.1),
        box_at(Vec3::new(0.0, 0.0, 3.0), 0.1),
    ];
    let view = spot(
        Vec3::ZERO,
        Vec3::NEG_Z,
        0.4,
        LocalRange {
            near: 0.1,
            far: 10.0,
        },
        128,
        &bounds,
    )
    .unwrap();
    assert_eq!(view.caster_indices, [0, 1]);
    let faces = point(
        Vec3::ZERO,
        LocalRange {
            near: 0.1,
            far: 10.0,
        },
        128,
        &bounds[1..2],
    )
    .unwrap();
    assert!(faces.iter().all(|v| v.caster_indices == [0]));
}

#[test]
fn near_far_metadata_and_projection_have_zero_to_one_depth() {
    let range = LocalRange {
        near: 0.5,
        far: 30.0,
    };
    let mut views = directional(
        &camera(Vec3::ZERO),
        [2, 1],
        Vec3::new(-1.0, -2.0, -1.0),
        &[15.0],
        64,
        &[],
    )
    .unwrap();
    views.push(spot(Vec3::new(2.0, 3.0, 4.0), Vec3::Y, 0.7, range, 64, &[]).unwrap());
    views.extend(point(Vec3::new(2.0, 3.0, 4.0), range, 64, &[]).unwrap());
    for view in views {
        for (distance, expected) in view.depth.into_iter().zip([0.0_f32, 1.0]) {
            let h = view.view_to_clip * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            assert!((h.z / h.w - expected).abs() < 2e-6, "{:?}", view.kind);
        }
    }
}

#[test]
fn translated_large_directional_scene_keeps_receiver_and_offscreen_caster_corners() {
    let origin = Vec3::new(1_000_000.0, -2_000_000.0, 500_000.0);
    let travel = Vec3::new(1.0, -1.0, 0.5).normalize();
    let receiver = origin + Vec3::new(0.0, 0.0, -20.0);
    let bounds = [box_at(receiver - travel * 2000.0, 2.0)];
    let views = directional(&camera(origin), [200, 100], travel, &[40.0], 1024, &bounds).unwrap();
    assert_eq!(views[0].caster_indices, [0]);
    for p in slice_corners(origin, 1.0, 40.0) {
        assert_inside(&views[0], p);
    }
    for p in bounds[0].corners() {
        assert_inside(&views[0], p.as_vec3());
    }
    // A large local-light domain avoids pretending f32 preserves centimetres at 1e6 metres.
    let views = point(
        origin,
        LocalRange {
            near: 32.0,
            far: 4096.0,
        },
        128,
        &bounds,
    )
    .unwrap();
    for p in bounds[0].corners() {
        assert!(
            views
                .iter()
                .any(|v| inside(v, p.as_vec3(), 3e-5) && v.caster_indices == [0])
        );
    }
}

#[test]
fn empty_and_nohit_plans_have_no_casters_but_valid_owned_views() {
    let range = LocalRange {
        near: 0.1,
        far: 10.0,
    };
    for bounds in [vec![], vec![box_at(Vec3::splat(1000.0), 0.1)]] {
        let spot = spot(Vec3::ZERO, Vec3::NEG_Z, 0.3, range, 1, &bounds).unwrap();
        assert!(spot.caster_indices.is_empty());
        assert!(
            point(Vec3::ZERO, range, 1, &bounds)
                .unwrap()
                .iter()
                .all(|v| v.caster_indices.is_empty())
        );
        assert!(
            directional(
                &camera(Vec3::ZERO),
                [2, 1],
                Vec3::NEG_Y,
                &[10.0],
                2,
                &bounds
            )
            .unwrap()[0]
                .caster_indices
                .is_empty()
        );
    }
}

#[test]
fn rotated_camera_slices_cover_independently_unprojected_rays() {
    for fov in [0.6, 1.4, 2.6] {
        let camera = Camera::look_at(
            Vec3::new(2.0, 3.0, -4.0),
            Vec3::new(-5.0, 1.0, 3.0),
            Vec3::Y,
            fov,
            0.25,
            100.0,
        )
        .unwrap();
        for viewport in [[3, 7], [7, 3]] {
            let inverse_clip = camera.matrix(viewport[0], viewport[1]).as_dmat4().inverse();
            let camera_view = camera.view().as_dmat4();
            let eye = camera_view.inverse().transform_point3(DVec3::ZERO);
            let views = directional(
                &camera,
                viewport,
                Vec3::new(1.0, -2.0, 3.0),
                &[3.0, 25.0],
                2048,
                &[],
            )
            .unwrap();
            for view in views {
                let ViewKind::Cascade { near, far } = view.kind else {
                    unreachable!()
                };
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        let h = inverse_clip * glam::DVec4::new(x, y, 0.0, 1.0);
                        let p = h.truncate() / h.w;
                        let forward_depth = -camera_view.transform_point3(p).z;
                        for depth in [near, far] {
                            assert_inside(
                                &view,
                                (eye + (p - eye) * f64::from(depth) / forward_depth).as_vec3(),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn local_caster_culling_keeps_far_plane_roundoff_conservative() {
    let position = Vec3::new(1_000_000.0, -800_000.0, 600_000.0);
    let range = LocalRange {
        near: 1.0,
        far: 1000.0,
    };
    let mut bounds = Vec::new();
    for axis in [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ] {
        bounds.push(box_at(position + axis * range.far, 0.0));
    }
    let views = point(position, range, 128, &bounds).unwrap();
    for i in 0..bounds.len() {
        assert!(
            views.iter().any(|v| v.caster_indices.contains(&i)),
            "far-plane caster {i} was culled"
        );
    }
}

#[test]
fn invalid_domains_reject_without_partial_results_or_panics() {
    let camera = camera(Vec3::ZERO);
    let range = LocalRange {
        near: 0.1,
        far: 10.0,
    };
    let invalid_bounds = [
        Bounds {
            min: Vec3::ONE,
            max: Vec3::ZERO,
        },
        Bounds {
            min: Vec3::splat(f32::NAN),
            max: Vec3::ONE,
        },
        Bounds {
            min: Vec3::ZERO,
            max: Vec3::splat(f32::INFINITY),
        },
    ];
    for b in invalid_bounds {
        assert!(directional(&camera, [2, 1], Vec3::Y, &[10.0], 64, &[b]).is_err());
        assert!(spot(Vec3::ZERO, Vec3::Y, 0.5, range, 64, &[b]).is_err());
        assert!(point(Vec3::ZERO, range, 64, &[b]).is_err());
    }
    for splits in [
        vec![],
        vec![1.0],
        vec![0.0],
        vec![2.0, 2.0],
        vec![2.0, 1.5],
        vec![2.0, f32::NAN],
        vec![f32::INFINITY],
    ] {
        assert!(directional(&camera, [2, 1], Vec3::Y, &splits, 64, &[]).is_err());
    }
    for travel in [
        Vec3::ZERO,
        Vec3::splat(f32::NAN),
        Vec3::splat(f32::INFINITY),
    ] {
        assert!(directional(&camera, [2, 1], travel, &[10.0], 64, &[]).is_err());
        assert!(spot(Vec3::ZERO, travel, 0.5, range, 64, &[]).is_err());
    }
    for angle in [
        0.0,
        -1.0,
        std::f32::consts::FRAC_PI_2,
        2.0,
        f32::NAN,
        f32::INFINITY,
        f32::from_bits(1),
    ] {
        assert!(spot(Vec3::ZERO, Vec3::Y, angle, range, 64, &[]).is_err());
    }
    for range in [
        LocalRange {
            near: 0.0,
            far: 10.0,
        },
        LocalRange {
            near: -1.0,
            far: 10.0,
        },
        LocalRange {
            near: 10.0,
            far: 10.0,
        },
        LocalRange {
            near: 1.0,
            far: f32::NAN,
        },
        LocalRange {
            near: f32::INFINITY,
            far: f32::INFINITY,
        },
    ] {
        assert!(point(Vec3::ZERO, range, 64, &[]).is_err());
        assert!(spot(Vec3::ZERO, Vec3::Y, 0.5, range, 64, &[]).is_err());
    }
    for position in [Vec3::splat(f32::NAN), Vec3::splat(f32::INFINITY)] {
        assert!(point(position, range, 64, &[]).is_err());
        assert!(spot(position, Vec3::Y, 0.5, range, 64, &[]).is_err());
    }
    assert!(point(Vec3::ZERO, range, 0, &[]).is_err());
    assert!(directional(&camera, [0, 1], Vec3::Y, &[10.0], 64, &[]).is_err());
    assert!(directional(&camera, [1, 0], Vec3::Y, &[10.0], 64, &[]).is_err());
    for resolution in [0, 1] {
        assert!(directional(&camera, [2, 1], Vec3::Y, &[10.0], resolution, &[]).is_err());
    }
    assert!(spot(Vec3::splat(f32::MAX), Vec3::NEG_Z, 0.01, range, 64, &[]).is_err());
}

#[test]
fn shadow_light_basis_crosses_helper_axis_ties_without_roll_jump() {
    let mut previous = None;
    for i in 0..100 {
        let travel = Vec3::new(0.4 + i as f32 * 0.0001, -0.405, -0.82);
        let basis = transported_orientation(travel, previous).unwrap();
        if let Some(old) = previous {
            assert!(basis.transpose().x_axis.dot(old.transpose().x_axis) > 0.9999);
        }
        assert!(
            (basis.transform_vector3(travel.as_dvec3().normalize()) - glam::DVec3::NEG_Z).length()
                < 1e-10
        );
        previous = Some(basis);
    }
}
