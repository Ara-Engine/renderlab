//! Views on the GPU (docs/design-views.md): two views of one engine against single-view
//! engines, and the view calls' bookkeeping. Non-RT path, default settings (GI, TR1, Virtual
//! sun shadows, atmosphere and fog, post) on a small scene.
use super::*;

const SIZE: [u32; 2] = [160, 96];

fn cube(c: [f32; 3], h: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
    let (mut vertices, mut indices) = (vec![], vec![]);
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut normal = [0.0; 3];
            normal[axis] = sign;
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut position = c;
                position[axis] += sign * h[axis];
                position[u] += a * h[u];
                position[v] += b * h[v];
                vertices.push(Vertex {
                    position,
                    normal,
                    uv: [0.0; 2],
                    color: [1.0; 4],
                });
            }
            let quad = if sign > 0.0 {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            indices.extend(quad.map(|i| base + i));
        }
    }
    (vertices, indices)
}

/// An engine with a ground, a red wall (bounce light) and a crate under the default sun,
/// and the crate's object.
fn engine(gi: bool) -> (Engine, ObjectId) {
    let adapter = test_adapter();
    let settings = Settings {
        global_illumination: gi,
        ..Default::default()
    };
    let descriptor = EngineDescriptor {
        settings,
        ..Default::default()
    };
    let mut engine = Engine::new(&adapter, descriptor).unwrap();
    let mut last = None;
    for (c, h, colour) in [
        ([0.0, -0.05, 0.0], [6.0, 0.05, 6.0], [0.7, 0.7, 0.7, 1.0]),
        ([-1.5, 1.0, 0.0], [0.1, 1.0, 1.5], [0.8, 0.1, 0.1, 1.0]),
        ([0.5, 0.4, 0.3], [0.4; 3], [0.6, 0.5, 0.3, 1.0]),
    ] {
        let (vertices, indices) = cube(c, h);
        let mesh = engine
            .create_mesh(&MeshData {
                vertices: &vertices,
                indices: &indices,
            })
            .unwrap();
        let material = engine
            .create_material(&MaterialDesc {
                base_color: colour,
                ..Default::default()
            })
            .unwrap();
        last = Some(engine.spawn(mesh, material, Mat4::IDENTITY).unwrap());
    }
    engine.set_sun(Some(Sun::at(0.6, 2.2))).unwrap();
    (engine, last.unwrap())
}

/// View 0 orbits by `turn` radians; view 1 stays still. Both eyes are 2 m up: the
/// atmosphere's sky light is placed once per tick at the first view's altitude, so views at
/// other heights differ from an engine of their own by at most one level of sky light.
fn cameras(turn: f32) -> [Camera; 2] {
    [
        Camera::look_at(
            Vec3::new(5.0 * (0.64 + turn).sin(), 2.0, 5.0 * (0.64 + turn).cos()),
            Vec3::Y * 0.4,
            Vec3::Y,
            0.9,
            0.05,
            200.0,
        ),
        Camera::look_at(
            Vec3::new(-4.0, 2.0, -3.0),
            Vec3::ZERO,
            Vec3::Y,
            0.8,
            0.05,
            200.0,
        ),
    ]
    .map(Result::unwrap)
}

fn desc() -> ViewDesc {
    ViewDesc {
        look: ViewLook {
            exposure: Exposure::Manual { stops: -14.0 },
            ..Default::default()
        },
        ..ViewDesc::new(SIZE[0], SIZE[1])
    }
}

/// Renders `ticks` ticks of the cameras `which` (indices into `cameras`), each tick rendering
/// every view in order, and returns each view's last image. `moving`: the crate spins and
/// view 0 orbits, a little every tick.
fn render(which: &[usize], ticks: u32, gi: bool, moving: bool) -> Vec<Image> {
    let (mut engine, crate_object) = engine(gi);
    let views: Vec<_> = which
        .iter()
        .map(|_| engine.create_view(&desc()).unwrap())
        .collect();
    for tick in 0..ticks {
        let t = if moving { tick as f32 * 0.05 } else { 0.0 };
        let cameras = cameras(t);
        let spin = Mat4::from_translation(Vec3::new(0.5, 0.4, 0.3))
            * Mat4::from_rotation_y(t)
            * Mat4::from_translation(Vec3::new(-0.5, -0.4, -0.3));
        engine.set_transform(crate_object, spin).unwrap();
        engine.tick(1.0 / 60.0).unwrap();
        for (view, &camera) in views.iter().zip(which) {
            let report = engine.render_view(*view, &cameras[camera]).unwrap();
            assert_eq!(report.view, *view);
        }
    }
    views
        .iter()
        .map(|&view| engine.capture_view(view).unwrap())
        .collect()
}

/// Per view of `[0, 1]` rendered together, the mean absolute byte difference from the same
/// camera in an engine of its own, after `ticks` still ticks.
fn difference_from_alone(ticks: u32, gi: bool, moving: bool) -> [f64; 2] {
    let both = render(&[0, 1], ticks, gi, moving);
    let alone = [
        render(&[0], ticks, gi, moving),
        render(&[1], ticks, gi, moving),
    ];
    std::array::from_fn(|view| {
        let (a, b) = (&both[view].rgba8, &alone[view][0].rgba8);
        let sum: u64 = a
            .iter()
            .zip(b)
            .map(|(x, y)| u64::from(x.abs_diff(*y)))
            .sum();
        sum as f64 / a.len() as f64
    })
}

#[test]
fn two_views_match_single_view_engines_without_gi() {
    // Every per-view and per-tick state, with no shared cache filled by both views.
    assert_eq!(difference_from_alone(24, false, false), [0.0; 2]);
}

#[test]
fn a_moving_view_and_moving_objects_leave_each_view_as_if_alone() {
    // View 0 orbits and the crate spins every tick; view 1 is still. No history (TR1, fog,
    // motion vectors, particles) carries from one view into the other.
    assert_eq!(difference_from_alone(24, false, true), [0.0; 2]);
}

#[test]
fn two_views_differ_from_single_view_engines_only_by_the_shared_gi_cache() {
    // The GI cache holds both views' cells; a view's cells are traced and gathered on the
    // tick's schedule rather than its own, so its still frame is not byte-identical to an
    // engine of its own, by well under one level on average (docs/design-views.md).
    let [a, b] = difference_from_alone(96, true, false);
    assert!(a < 1.0 && b < 1.0, "mean byte difference {a} and {b}");
}

#[test]
fn views_are_created_resized_and_removed_independently() {
    let (mut engine, _) = engine(true);
    let [near, far] = cameras(0.0);
    let a = engine.create_view(&desc()).unwrap();
    let b = engine.create_view(&desc()).unwrap();
    assert_ne!(a, b);
    engine.tick(1.0 / 60.0).unwrap();
    engine.render_view(a, &near).unwrap();
    engine.render_view(b, &far).unwrap();
    // A resize changes only that view's output.
    let wide = ViewDesc {
        width: 2 * SIZE[0],
        ..desc()
    };
    engine.set_view(b, &wide).unwrap();
    assert_eq!(engine.view_output(b).unwrap().width(), 2 * SIZE[0]);
    assert_eq!(engine.view_output(a).unwrap().width(), SIZE[0]);
    engine.tick(1.0 / 60.0).unwrap();
    engine.render_view(b, &far).unwrap();
    assert_eq!(engine.capture_view(b).unwrap().width, 2 * SIZE[0]);
    // Removing the selected view keeps the other rendering; the removed handle is stale.
    engine.remove_view(b).unwrap();
    assert!(engine.render_view(b, &far).is_err());
    assert!(engine.view_output(b).is_none());
    engine.tick(1.0 / 60.0).unwrap();
    engine.render_view(a, &near).unwrap();
    // A view created after a removal works, and handles are never reused.
    let c = engine.create_view(&desc()).unwrap();
    assert!(c != a && c != b);
    engine.render_view(c, &far).unwrap();
    assert_eq!(engine.views().count(), 2);
    let ids = ViewDesc {
        outputs: Outputs { ids: true },
        ..desc()
    };
    // Ids per view (RL#41): a view with ids is created and renders like any other.
    let d = engine.create_view(&ids).unwrap();
    engine.render_view(d, &far).unwrap();
}

/// An engine with no objects at all (a new project's empty scene) renders sky on both paths,
/// through cuts, a spawn and a despawn back to empty. GPU test: run in a lease.
#[test]
#[ignore]
fn an_empty_scene_renders_on_both_paths() {
    for path in [RenderPath::NonRt, RenderPath::HardwareRt] {
        let adapter = test_adapter();
        // SAFETY: the test suite's documented experimental-feature acknowledgement.
        let experimental_ray_query =
            (path == RenderPath::HardwareRt).then(|| unsafe { ExperimentalRayQuery::acknowledge() });
        let mut engine = Engine::new(
            &adapter,
            EngineDescriptor {
                path,
                experimental_ray_query,
                ..Default::default()
            },
        )
        .unwrap();
        engine.set_sun(Some(Sun::at(0.6, 2.2))).unwrap();
        let view = engine.create_view(&desc()).unwrap();
        let [near, _] = cameras(0.0);
        let frame = |engine: &mut Engine| {
            engine.tick(1.0 / 60.0).unwrap();
            engine.render_view(view, &near).unwrap();
        };
        for _ in 0..3 {
            frame(&mut engine);
        }
        let sky = engine.capture_view(view).unwrap();
        assert!(sky.rgba8.chunks(4).any(|p| p[..3] != [0, 0, 0]), "{path:?}: an empty scene drew black");
        engine.cut_view(view);
        frame(&mut engine);
        let (vertices, indices) = cube([0.0, -0.05, 0.0], [6.0, 0.05, 6.0]);
        let mesh = engine.create_mesh(&MeshData { vertices: &vertices, indices: &indices }).unwrap();
        let material = engine.create_material(&MaterialDesc::default()).unwrap();
        let floor = engine.spawn(mesh, material, Mat4::IDENTITY).unwrap();
        frame(&mut engine);
        engine.despawn(floor).unwrap();
        for _ in 0..3 {
            frame(&mut engine);
        }
        engine.capture_view(view).unwrap();
    }
}
