//! Views on the GPU (docs/design-views.md): two views of one engine against single-view
//! engines, and the view calls' bookkeeping. Non-RT path, default settings (GI, TR1, Virtual
//! sun shadows, atmosphere and fog, post) on a small scene.
use super::*;

const SIZE: [u32; 2] = [160, 96];

pub(super) fn cube(c: [f32; 3], h: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
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

/// Memory cut 1 through the engine (opt-in `auto_slots`): its settings reach the renderer
/// through `set_indirect`, which must leave an `auto_slots` table to the first frame, sized to
/// the scene (three boxes: the 2^16 floor), not allocate the 2^20 cap.
#[test]
fn the_engine_sizes_the_gi_table_to_its_scene() {
    let (mut engine, _) = engine(true);
    let mut settings = engine.renderer().indirect();
    settings.auto_slots = true;
    engine.renderer_mut().set_indirect(settings).unwrap();
    let view = engine.create_view(&desc()).unwrap();
    for _ in 0..3 {
        engine.tick(1.0 / 60.0).unwrap();
        engine.render_view(view, &cameras(0.0)[0]).unwrap();
    }
    assert_eq!(engine.renderer().indirect_stats().slots, 1 << 16);
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

/// Walking backward quickly past an obliquely viewed shadowed wall reveals fresh geometry on the
/// left edge of the screen. Final-gather probes must preserve oblique history and filter newly
/// revealed tiles cleanly without large sky/bounce blotches on either RT or non-RT.
#[test]
#[ignore]
fn backward_motion_reveals_shadowed_wall_without_gi_blotches() {
    const W: u32 = 1280;
    const H: u32 = 720;
    let dump_dir = std::env::var_os("RENDERLAB_WALL_DUMP").map(std::path::PathBuf::from);
    for path in [RenderPath::HardwareRt, RenderPath::NonRt] {
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
        let spawn = |engine: &mut Engine, c, h, colour, roughness| {
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
                    roughness,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap();
            engine.spawn(mesh, material, Mat4::IDENTITY).unwrap()
        };
        // Matte grey floor and a tall oblique vertical wall (roughness 0.5 matches Ara's default
        // material so both GI final gather and rough reflections are exercised). The sun sits
        // behind the wall (azimuth 0 -> -Z) so both the +Z wall face and the near floor are in
        // shadow, creating the strong sky-vs-dark-floor hemisphere contrast that exposes blotches.
        spawn(&mut engine, [0.0, -0.05, 0.0], [40.0, 0.05, 40.0], [0.28, 0.30, 0.36, 1.0], 0.85);
        spawn(&mut engine, [0.0, 3.0, -0.25], [40.0, 3.0, 0.25], [0.52, 0.54, 0.58, 1.0], 0.50);
        // Far dark enclosure behind the camera so backward-facing probe rays hit real geometry.
        spawn(&mut engine, [0.0, 2.5, 16.0], [40.0, 2.5, 0.25], [0.35, 0.36, 0.40, 1.0], 0.85);
        engine.set_sun(Some(Sun::at(0.45, 0.0))).unwrap();
        let view = engine
            .create_view(&ViewDesc {
                look: ViewLook {
                    exposure: Exposure::Manual { stops: -12.5 },
                    ..Default::default()
                },
                ..ViewDesc::new(W, H)
            })
            .unwrap();
        let cam = |x: f32, yaw: f32| {
            let eye = Vec3::new(x, 1.65, 2.6);
            let target = eye + Vec3::new(6.0 * yaw.cos() + 2.5 * yaw.sin(), -0.1, -2.5 * yaw.cos() + 6.0 * yaw.sin());
            Camera::look_at(eye, target, Vec3::Y, 0.95, 0.05, 120.0).unwrap()
        };
        let step = |engine: &mut Engine, camera: &Camera| {
            engine.tick(1.0 / 60.0).unwrap();
            engine.render_view(view, camera).unwrap();
        };
        // Settle at start pose, then walk rapidly backward + pan left so a wide fresh strip of the
        // shadowed wall enters from the left edge every frame.
        let start = cam(6.0, 0.0);
        for _ in 0..24 {
            step(&mut engine, &start);
        }
        let mut x = 6.0f32;
        let mut yaw = 0.0f32;
        for _ in 0..8 {
            x -= 0.9;
            yaw -= 0.025;
            step(&mut engine, &cam(x, yaw));
        }
        let moving = engine.capture_view(view).unwrap();
        let final_cam = cam(x, yaw);
        for _ in 0..48 {
            step(&mut engine, &final_cam);
        }
        let settled = engine.capture_view(view).unwrap();

        if let Some(dir) = &dump_dir {
            let _ = std::fs::create_dir_all(dir);
            let tag = if path == RenderPath::HardwareRt { "rt" } else { "nonrt" };
            for (kind, img) in [("moving", &moving), ("settled", &settled)] {
                let path = dir.join(format!("wall_{tag}_{kind}.png"));
                let file = std::fs::File::create(&path).unwrap();
                let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().unwrap().write_image_data(&img.rgba8).unwrap();
            }
        }

        // Measure 32x32 block-mean RMS and max deviation on the newly-revealed left half of the wall.
        let (mut sq, mut max_diff, mut n) = (0.0f64, 0.0f64, 0u32);
        let (mut sum_m, mut sum_s) = (0.0f64, 0.0f64);
        for ty in (224..480).step_by(32) {
            for tx in (32..576).step_by(32) {
                let (mut m, mut s) = (0.0f64, 0.0f64);
                for dy in 0..32 {
                    for dx in 0..32 {
                        let i = ((ty + dy) * W as usize + (tx + dx)) * 4;
                        let lum = |b: &[u8]| 0.2126 * f64::from(b[i]) + 0.7152 * f64::from(b[i + 1]) + 0.0722 * f64::from(b[i + 2]);
                        m += lum(&moving.rgba8);
                        s += lum(&settled.rgba8);
                    }
                }
                m /= 1024.0;
                s /= 1024.0;
                let d = (m - s).abs();
                sq += d * d;
                max_diff = max_diff.max(d);
                sum_m += m;
                sum_s += s;
                n += 1;
            }
        }
        let rms = (sq / f64::from(n)).sqrt();
        let mean_m = sum_m / f64::from(n);
        let mean_s = sum_s / f64::from(n);
        eprintln!(
            "wall_disocclusion path={path:?} block_rms={rms:.2} block_max={max_diff:.2} mean_moving={mean_m:.2} mean_settled={mean_s:.2}"
        );
        let max_limit = if path == RenderPath::HardwareRt { 2.0 } else { 3.0 };
        assert!(
            rms < 1.8 && max_diff < max_limit,
            "{path:?}: newly revealed wall shows blotches (block_rms={rms:.2}, block_max={max_diff:.2}, mean={mean_m:.2} vs {mean_s:.2})"
        );
    }
}

/// Reproduces the My project 5 arena corner (shadowed West/South walls, dark floor, sunlit white
/// Pillar 3, sunlit rust-orange Cover 5, Archway, and emissive Light panel) under still (24-frame
/// editor settle), 64-frame converged, and fast left-right camera pan on both RT and non-RT.
#[test]
#[ignore]
fn arena_corner_still_and_fast_pan_without_gi_spots_or_blotches() {
    const W: u32 = 1280;
    const H: u32 = 720;
    let dump_dir = std::env::var_os("RENDERLAB_ARENA_DUMP").map(std::path::PathBuf::from);
    for path in [RenderPath::HardwareRt, RenderPath::NonRt] {
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
        let spawn = |engine: &mut Engine, c, h, colour, roughness| {
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
                    roughness,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap();
            engine.spawn(mesh, material, Mat4::IDENTITY).unwrap()
        };
        // Floor & perimeter walls (exact My project 5 geometry & colours).
        spawn(&mut engine, [0.0, -0.25, 0.0], [20.0, 0.25, 20.0], [0.18, 0.18, 0.18, 1.0], 0.88);
        spawn(&mut engine, [0.0, 2.0, -19.75], [20.0, 2.0, 0.25], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [0.0, 2.0, 19.75], [20.0, 2.0, 0.25], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [-19.75, 2.0, 0.0], [0.25, 2.0, 19.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [19.75, 2.0, 0.0], [0.25, 2.0, 19.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        // Pillar 3 (sunlit bright pillar right beside the West wall shadow) + Pillar 1.
        spawn(&mut engine, [-14.0, 2.0, 14.0], [0.6, 2.0, 0.6], [0.70, 0.67, 0.62, 1.0], 0.90);
        spawn(&mut engine, [-14.0, 2.0, -14.0], [0.6, 2.0, 0.6], [0.70, 0.67, 0.62, 1.0], 0.90);
        // Cover 5 (rust-orange sunlit block across from the shadowed West wall) & Archway.
        spawn(&mut engine, [-14.0, 0.6, -5.0], [0.3, 0.6, 1.0], [0.52, 0.18, 0.04, 1.0], 0.78);
        spawn(&mut engine, [-6.0, 0.6, 9.0], [1.0, 0.6, 0.3], [0.52, 0.18, 0.04, 1.0], 0.78);
        spawn(&mut engine, [-10.0, 2.5, 4.0], [3.0, 2.5, 0.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        // Directional sun matching My project 5 rotation [-38, -32, 0] (both West and South inner
        // walls shadowed, left faces of Pillar 3 and orange Cover 5 directly sunlit), 90,000 lux.
        let el = 38.0f32.to_radians();
        let az = 32.0f32.to_radians();
        let toward = Vec3::new(-el.cos() * az.sin(), el.sin(), el.cos() * az.cos());
        engine
            .set_sun(Some(Sun {
                travel: -toward.normalize(),
                illuminance: [90_000.0, 85_500.0, 77_400.0],
                ..Sun::default()
            }))
            .unwrap();
        let view = engine
            .create_view(&ViewDesc {
                look: ViewLook {
                    exposure: Exposure::Manual { stops: -13.0 },
                    post: crate::PostSettings {
                        motion_blur: false,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..ViewDesc::new(W, H)
            })
            .unwrap();
        let base_eye = Vec3::new(-12.75, 6.66, 16.47);
        let cam_yaw = |delta_yaw: f32| {
            let yaw = 1.065001 + delta_yaw;
            let pitch = 0.725f32;
            let fwd = Vec3::new(-pitch.cos() * yaw.sin(), -pitch.sin(), -pitch.cos() * yaw.cos());
            Camera::look_at(base_eye, base_eye + fwd, Vec3::Y, 75.0f32.to_radians(), 0.05, 400.0).unwrap()
        };
        let step = |engine: &mut Engine, camera: &Camera| {
            engine.tick(1.0 / 60.0).unwrap();
            engine.render_view(view, camera).unwrap();
        };
        let home = cam_yaw(0.0);
        // Reference: deeply settled (96 frames).
        for _ in 0..96 {
            step(&mut engine, &home);
        }
        let ref_img = engine.capture_view(view).unwrap();
        // Fast left-right camera pan away and back to home, ending mid-pan right at home.
        let mut dyaw = 0.0f32;
        for _ in 0..5 {
            dyaw -= 0.16;
            step(&mut engine, &cam_yaw(dyaw));
        }
        for _ in 0..10 {
            dyaw += 0.16;
            step(&mut engine, &cam_yaw(dyaw));
        }
        for _ in 0..5 {
            dyaw -= 0.16;
            step(&mut engine, &cam_yaw(dyaw));
        }
        let pan_img = engine.capture_view(view).unwrap();
        // Editor-like 24-frame settle after stopping the pan.
        for _ in 0..24 {
            step(&mut engine, &home);
        }
        let still24 = engine.capture_view(view).unwrap();

        if let Some(dir) = &dump_dir {
            let _ = std::fs::create_dir_all(dir);
            let tag = if path == RenderPath::HardwareRt { "rt" } else { "nonrt" };
            for (kind, img) in [("ref", &ref_img), ("pan", &pan_img), ("still24", &still24)] {
                let p = dir.join(format!("arena_{tag}_{kind}.png"));
                let file = std::fs::File::create(&p).unwrap();
                let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().unwrap().write_image_data(&img.rgba8).unwrap();
            }
        }

        // Measure 24x24 block-mean RMS and max deviation on the shadowed West wall & corner floor.
        let measure = |a: &[u8], b: &[u8]| -> (f64, f64) {
            let (mut sq, mut mx, mut n) = (0.0f64, 0.0f64, 0u32);
            // West wall band + shadowed floor region (avoiding pillar & sunlit floor).
            for &(x0, y0, x1, y1) in &[(240usize, 192usize, 1080usize, 312usize), (288, 432, 600, 624)] {
                for ty in (y0..y1).step_by(24) {
                    for tx in (x0..x1).step_by(24) {
                        let (mut ra, mut ga, mut ba, mut rb, mut gb, mut bb) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
                        for dy in 0..24 {
                            for dx in 0..24 {
                                let i = ((ty + dy) * W as usize + (tx + dx)) * 4;
                                ra += f64::from(a[i]);
                                ga += f64::from(a[i + 1]);
                                ba += f64::from(a[i + 2]);
                                rb += f64::from(b[i]);
                                gb += f64::from(b[i + 1]);
                                bb += f64::from(b[i + 2]);
                            }
                        }
                        let inv = 1.0 / 576.0;
                        let dr = (ra - rb) * inv;
                        let dg = (ga - gb) * inv;
                        let db = (ba - bb) * inv;
                        let d = (dr * dr + dg * dg + db * db).sqrt();
                        sq += d * d;
                        mx = mx.max(d);
                        n += 1;
                    }
                }
            }
            ((sq / f64::from(n)).sqrt(), mx)
        };
        let (pan_rms, pan_max) = measure(&pan_img.rgba8, &ref_img.rgba8);
        let (s24_rms, s24_max) = measure(&still24.rgba8, &ref_img.rgba8);
        eprintln!(
            "arena_corner path={path:?} pan_rgb_rms={pan_rms:.2} pan_rgb_max={pan_max:.2} still24_rgb_rms={s24_rms:.2} still24_rgb_max={s24_max:.2}"
        );
        assert!(
            pan_rms < 4.0 && pan_max < 12.0 && s24_rms < 2.5 && s24_max < 8.0,
            "{path:?}: arena corner GI instability (pan rms={pan_rms:.2} max={pan_max:.2}, still24 rms={s24_rms:.2} max={s24_max:.2})"
        );
    }
}

/// Reproduces the My project 5 Night preset scene (4 lux moonlight, SkyAtmosphere, HeightFog,
/// 3 warm accent point lights, 8000 lm panel rect/spot light, 3000 cd/m² emissive Light panel,
/// EV100 = 4 exposure) from the saved Scene view camera under cold start, 36-frame Scene view
/// settle, 96-frame reference, and fast camera pans on both RT and Non-RT.
#[test]
#[ignore]
fn arena_night_preset_repro() {
    const W: u32 = 1280;
    const H: u32 = 720;
    let dump_dir = std::env::var_os("RENDERLAB_NIGHT_DUMP").map(std::path::PathBuf::from);
    if let Some(dir) = &dump_dir {
        let _ = std::fs::create_dir_all(dir);
    }

    // Exact Ara emissive program (ara-renderlab/src/scene.rs emissive_source).
    const FIELDS: &str = "alpha:f32 };";
    const BUILT: &str = "decal_mr.x,alpha);";
    const RETURN: &str = "    return exposed;";
    let emissive_wgsl = format!(
        "struct Parameters {{ glow:vec3<f32>, everywhere:f32, bounce:vec3<f32>, mapped:f32 }};\n\
         const DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.0),0.0,vec3<f32>(0.0),0.0);\n\
         fn ara_glows(input:SurfaceInput) -> bool {{ return params.everywhere>0.5 || abs(input.material.base_color.a-0.737)<0.002; }}\n\
         {}\n\
         fn emission(s:Surface, input:SurfaceInput) -> vec3<f32> {{ return select(vec3<f32>(0.0),max(params.bounce,vec3<f32>(0.0)),ara_glows(input)); }}\n",
        crate::STANDARD_PROGRAM
            .replace(FIELDS, "alpha:f32, glow:vec3<f32> };")
            .replace(
                BUILT,
                "decal_mr.x,alpha,select(vec3<f32>(0.0),max(params.glow,vec3<f32>(0.0)),ara_glows(input)));",
            )
            .replace(
                RETURN,
                "    return min(exposed+exposed_product(s.glow,vec3<f32>(1.0)),vec3<f32>(65504.0));",
            )
            .replace(
                "s.n,s.base,s.alpha,s.metal,s.rough,vec3<f32>(0.0),",
                "s.n,s.base,s.alpha,s.metal,s.rough,s.glow,",
            )
            .replace(
                "    s.alpha=m.alpha;\n    return s;",
                "    s.alpha=m.alpha;\n    s.glow=m.emission;\n    return s;",
            )
    );

    for path in [RenderPath::HardwareRt, RenderPath::NonRt] {
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

        let emissive_prog = engine.create_program("ara_emissive", &emissive_wgsl).unwrap();

        let spawn = |engine: &mut Engine, c, h, colour, roughness| {
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
                    roughness,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap();
            engine.spawn(mesh, material, Mat4::IDENTITY).unwrap()
        };

        // Exact My project 5 arena geometry & materials.
        spawn(&mut engine, [0.0, -0.25, 0.0], [20.0, 0.25, 20.0], [0.18, 0.18, 0.18, 1.0], 0.88);
        spawn(&mut engine, [0.0, 2.0, -19.75], [20.0, 2.0, 0.25], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [0.0, 2.0, 19.75], [20.0, 2.0, 0.25], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [-19.75, 2.0, 0.0], [0.25, 2.0, 19.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [19.75, 2.0, 0.0], [0.25, 2.0, 19.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        // Platform, Archway (jambs + lintel).
        spawn(&mut engine, [0.0, 1.0, -8.0], [4.0, 1.0, 3.0], [0.18, 0.18, 0.18, 1.0], 0.88);
        spawn(&mut engine, [-12.4, 2.5, 4.0], [0.6, 2.5, 0.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [-7.6, 2.5, 4.0], [0.6, 2.5, 0.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [-10.0, 4.5, 4.0], [1.8, 0.5, 0.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        // 4 Pillars.
        for [px, pz] in [[-14.0, -14.0], [14.0, -14.0], [-14.0, 14.0], [14.0, 14.0]] {
            spawn(&mut engine, [px, 2.0, pz], [0.6, 2.0, 0.6], [0.70, 0.67, 0.62, 1.0], 0.90);
        }
        // 5 Covers.
        spawn(&mut engine, [6.0, 0.6, 6.0], [1.0, 0.6, 0.3], [0.52, 0.18, 0.04, 1.0], 0.78);
        spawn(&mut engine, [-6.0, 0.6, 9.0], [1.0, 0.6, 0.3], [0.52, 0.18, 0.04, 1.0], 0.78);
        spawn(&mut engine, [9.0, 0.6, -2.0], [0.3, 0.6, 1.0], [0.52, 0.18, 0.04, 1.0], 0.78);
        spawn(&mut engine, [12.0, 0.6, 11.0], [1.0, 0.6, 0.3], [0.52, 0.18, 0.04, 1.0], 0.78);
        spawn(&mut engine, [-14.0, 0.6, -5.0], [0.3, 0.6, 1.0], [0.52, 0.18, 0.04, 1.0], 0.78);
        // Crates.
        for pos in [
            [5.0, 0.4, 12.0],
            [5.9, 0.4, 12.0],
            [5.45, 1.2, 12.0],
            [-7.0, 0.4, 13.0],
            [-1.2, 2.4, -9.0],
            [0.0, 2.4, -9.5],
        ] {
            spawn(&mut engine, pos, [0.4, 0.4, 0.4], [0.4, 0.24, 0.12, 1.0], 0.55);
        }
        // Emissive Light panel (`e_panel`): [0, 3, -19.45], size [4, 1, 0.1], emissive [3000, 2250, 1450], castShadows: false.
        {
            let (vertices, indices) = cube([0.0, 3.0, -19.45], [2.0, 0.5, 0.05]);
            let mesh = engine
                .create_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })
                .unwrap();
            let mat = engine
                .create_material(&MaterialDesc {
                    base_color: [0.8, 0.8, 0.8, 0.737],
                    roughness: 0.4,
                    double_sided: true,
                    program: Some(emissive_prog),
                    ..Default::default()
                })
                .unwrap();
            engine
                .set_material_parameters(
                    mat,
                    &[
                        ("glow", crate::Value::Vec3([3000.0, 2250.0, 1450.0])),
                        ("everywhere", crate::Value::F32(0.0)),
                        ("bounce", crate::Value::Vec3([3000.0, 2250.0, 1450.0])),
                        ("mapped", crate::Value::F32(0.0)),
                    ],
                )
                .unwrap();
            let obj = engine.spawn(mesh, mat, Mat4::IDENTITY).unwrap();
            engine.set_casts_shadows(obj, false).unwrap();
        }

        // Exact Night preset lighting: 4 lux cool moon + 3 warm point lamps + 8000 lm panel light.
        let el = 35.0f32.to_radians();
        let az = 32.0f32.to_radians();
        let toward = Vec3::new(-el.cos() * az.sin(), el.sin(), el.cos() * az.cos());
        engine
            .set_sun(Some(Sun {
                travel: -toward.normalize(),
                illuminance: [4.0 * 0.64161, 4.0 * 0.73686, 4.0],
                angular_radius: 0.5357f32.to_radians() * 0.5,
            }))
            .unwrap();
        let lm_to_cd = |lm: f32, c: [f32; 3]| {
            let s = lm / (4.0 * std::f32::consts::PI);
            [c[0] * s, c[1] * s, c[2] * s]
        };
        engine
            .set_lights(&[
                crate::engine::Light::Point {
                    position: Vec3::new(0.0, 3.6, -8.0),
                    candela: lm_to_cd(6000.0, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                },
                crate::engine::Light::Point {
                    position: Vec3::new(-10.0, 4.4, 5.5),
                    candela: lm_to_cd(5000.0, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                },
                crate::engine::Light::Point {
                    position: Vec3::new(12.0, 2.5, 12.0),
                    candela: lm_to_cd(4000.0, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                },
                crate::engine::Light::Spot {
                    position: Vec3::new(0.0, 3.0, -19.35),
                    travel: Vec3::Z,
                    candela: lm_to_cd(8000.0, [1.0, 0.75, 0.48]),
                    range: Some(16.0),
                    inner: 0.0,
                    outer: 1.55,
                    source_radius: 0.5,
                    cast_shadows: Some(true),
                },
            ])
            .unwrap();
        engine.set_sky(crate::engine::Sky::Atmosphere(crate::engine::AtmosphereMedium::default())).unwrap();
        engine
            .set_settings(crate::engine::Settings {
                aerial_perspective: true,
                height_fog: Some(crate::engine::HeightFog {
                    density: 0.004,
                    falloff: 1.0 / 30.0,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();

        // EV100 = 4 (auto-exposure minEv100 in My project 5): exposure = 1 / (1.2 * 2^4) => stops ≈ -4.263.
        let view = engine
            .create_view(&ViewDesc {
                look: ViewLook {
                    exposure: Exposure::Manual { stops: -4.263 },
                    post: crate::PostSettings {
                        bloom: 0.05,
                        motion_blur: false,
                        tonemapper: crate::Tonemapper::Filmic,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..ViewDesc::new(W, H)
            })
            .unwrap();

        let save = |name: &str, img: &Image| {
            let Some(dir) = &dump_dir else {
                return;
            };
            let p = dir.join(name);
            let file = std::fs::File::create(&p).unwrap();
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&img.rgba8).unwrap();
        };

        // Exact saved Scene View camera from scene-cameras.json (`My project 5` sc_main001).
        let target = Vec3::new(-1.9061034, 8.896457, 12.530297);
        let base_yaw = 1.33f32;
        let base_pitch = 0.70f32;
        let dist = 0.16160744f32;
        let cam_at = |dyaw: f32| {
            let yaw = base_yaw + dyaw;
            let back = Vec3::new(yaw.sin() * base_pitch.cos(), base_pitch.sin(), yaw.cos() * base_pitch.cos());
            let eye = target + back * dist;
            Camera::look_at(eye, target, Vec3::Y, 75.0f32.to_radians(), 0.05, 4000.0).unwrap()
        };
        let home = cam_at(0.0);
        let step = |engine: &mut Engine, camera: &Camera| {
            engine.tick(1.0 / 60.0).unwrap();
            engine.render_view(view, camera).unwrap();
        };
        let tag = if path == RenderPath::HardwareRt { "rt" } else { "nonrt" };

        // Cold start -> frame 1, frame 8, frame 36 (Scene settle), frame 96 (deep settle).
        step(&mut engine, &home);
        let f1 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_f1.png"), &f1);
        for _ in 1..8 {
            step(&mut engine, &home);
        }
        let f8 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_f8.png"), &f8);
        for _ in 8..36 {
            step(&mut engine, &home);
        }
        let f36 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_f36.png"), &f36);
        for _ in 36..96 {
            step(&mut engine, &home);
        }
        let f96 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_f96.png"), &f96);

        // Fast left-right pan returning to home, then 36-frame Scene view settle.
        let mut dyaw = 0.0f32;
        for _ in 0..5 {
            dyaw -= 0.15;
            step(&mut engine, &cam_at(dyaw));
        }
        for _ in 0..10 {
            dyaw += 0.15;
            step(&mut engine, &cam_at(dyaw));
        }
        for _ in 0..5 {
            dyaw -= 0.15;
            step(&mut engine, &cam_at(dyaw));
        }
        let pan = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_pan.png"), &pan);
        for _ in 0..36 {
            step(&mut engine, &home);
        }
        let pan36 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_pan36.png"), &pan36);

        // Second angle: wide overview facing North wall, Light panel, Accent light 1 & 2, and West wall.
        let overview = |dyaw: f32| {
            let eye = Vec3::new(0.0, 5.5, 14.0);
            let yaw = dyaw;
            let pitch = 0.22f32;
            let fwd = Vec3::new(yaw.sin() * pitch.cos(), -pitch.sin(), -yaw.cos() * pitch.cos());
            Camera::look_at(eye, eye + fwd, Vec3::Y, 75.0f32.to_radians(), 0.05, 4000.0).unwrap()
        };
        engine.cut_view(view);
        for _ in 0..36 {
            step(&mut engine, &overview(0.0));
        }
        let ov36 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_ov36.png"), &ov36);
        for _ in 36..96 {
            step(&mut engine, &overview(0.0));
        }
        let ov96 = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_ov96.png"), &ov96);
        // Pan across overview.
        let mut od = 0.0f32;
        for _ in 0..5 {
            od -= 0.14;
            step(&mut engine, &overview(od));
        }
        for _ in 0..10 {
            od += 0.14;
            step(&mut engine, &overview(od));
        }
        for _ in 0..5 {
            od -= 0.14;
            step(&mut engine, &overview(od));
        }
        let ov_pan = engine.capture_view(view).unwrap();
        save(&format!("night_{tag}_ov_pan.png"), &ov_pan);

        // Measure pixel-level and 24x24 block-level differences against 96-frame reference.
        let diff = |a: &[u8], b: &[u8]| -> (f64, f64, f64) {
            let (mut psq, mut bsq, mut bmax, mut bn) = (0.0f64, 0.0f64, 0.0f64, 0u32);
            for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
                let dr = f64::from(pa[0]) - f64::from(pb[0]);
                let dg = f64::from(pa[1]) - f64::from(pb[1]);
                let db = f64::from(pa[2]) - f64::from(pb[2]);
                psq += (dr * dr + dg * dg + db * db) / 3.0;
            }
            for ty in (0..H as usize).step_by(24) {
                for tx in (0..W as usize).step_by(24) {
                    if ty + 24 > H as usize || tx + 24 > W as usize {
                        continue;
                    }
                    let (mut dr, mut dg, mut db) = (0.0, 0.0, 0.0);
                    for dy in 0..24 {
                        for dx in 0..24 {
                            let i = ((ty + dy) * W as usize + (tx + dx)) * 4;
                            dr += f64::from(a[i]) - f64::from(b[i]);
                            dg += f64::from(a[i + 1]) - f64::from(b[i + 1]);
                            db += f64::from(a[i + 2]) - f64::from(b[i + 2]);
                        }
                    }
                    let d = (dr * dr + dg * dg + db * db).sqrt() / 576.0;
                    bsq += d * d;
                    bmax = bmax.max(d);
                    bn += 1;
                }
            }
            let pix_rms = (psq / f64::from(W * H)).sqrt();
            ((bsq / f64::from(bn)).sqrt(), bmax, pix_rms)
        };
        let (f1_b, f1_m, f1_p) = diff(&f1.rgba8, &f96.rgba8);
        let (f36_b, f36_m, f36_p) = diff(&f36.rgba8, &f96.rgba8);
        let (pan_b, pan_m, pan_p) = diff(&pan.rgba8, &f96.rgba8);
        let (p36_b, p36_m, p36_p) = diff(&pan36.rgba8, &f96.rgba8);
        let (ov36_b, ov36_m, ov36_p) = diff(&ov36.rgba8, &ov96.rgba8);
        let (ovp_b, ovp_m, ovp_p) = diff(&ov_pan.rgba8, &ov96.rgba8);
        eprintln!(
            "night_repro path={tag} f1(brms={f1_b:.2},bmax={f1_m:.2},prms={f1_p:.2}) f36(brms={f36_b:.2},bmax={f36_m:.2},prms={f36_p:.2}) pan(brms={pan_b:.2},bmax={pan_m:.2},prms={pan_p:.2}) pan36(brms={p36_b:.2},bmax={p36_m:.2},prms={p36_p:.2}) ov36(brms={ov36_b:.2},bmax={ov36_m:.2},prms={ov36_p:.2}) ov_pan(brms={ovp_b:.2},bmax={ovp_m:.2},prms={ovp_p:.2})"
        );
        assert!(
            f1_b < 14.0,
            "{tag} night frame 1 must include live emitter GI instead of flashing dark (brms={f1_b:.2})"
        );
        assert!(
            f36_b < 2.0 && f36_m < 10.0,
            "{tag} night 36-frame settle drifted from 96-frame reference (brms={f36_b:.2}, bmax={f36_m:.2})"
        );
        assert!(
            pan_b < 3.5 && pan_m < 24.0,
            "{tag} night fast pan smeared shadows/walls (brms={pan_b:.2}, bmax={pan_m:.2})"
        );
        assert!(
            p36_b < 2.0 && p36_m < 12.0,
            "{tag} night post-pan 36-frame settle did not recover cleanly (brms={p36_b:.2}, bmax={p36_m:.2})"
        );
        assert!(
            ov36_b < 2.0,
            "{tag} night overview 36-frame settle drifted from reference (brms={ov36_b:.2})"
        );
    }
}

/// Core dynamic GI / TR1 / local-light regression test reproducing:
/// 1) Dragging an emissive panel (`e_panel`) back and forth over the arena floor/platform in
///    Scene view (verifies zero concentric iso-contour rings and zero stale floor/TR1 trails);
/// 2) Jumping and strafing in first-person right beside `e_panel` + `e_panellight` looking down
///    the North wall toward Pillar 2 and the Far East wall (verifies zero darkened-wall LOD
///    flicker and zero wall-clipping local-light shadow flicker) on both `HardwareRt` and `NonRt`.
#[test]
#[ignore]
fn moving_emitter_and_jump_no_rings_or_flicker() {
    const W: u32 = 1280;
    const H: u32 = 720;
    let dump_dir = std::env::var_os("RENDERLAB_DYNAMIC_DUMP").map(std::path::PathBuf::from);
    if let Some(dir) = &dump_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut errors = Vec::new();

    const FIELDS: &str = "alpha:f32 };";
    const BUILT: &str = "decal_mr.x,alpha);";
    const RETURN: &str = "    return exposed;";
    let emissive_wgsl = format!(
        "struct Parameters {{ glow:vec3<f32>, everywhere:f32, bounce:vec3<f32>, mapped:f32 }};\n\
         const DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.0),0.0,vec3<f32>(0.0),0.0);\n\
         fn ara_glows(input:SurfaceInput) -> bool {{ return params.everywhere>0.5 || abs(input.material.base_color.a-0.737)<0.002; }}\n\
         {}\n\
         fn emission(s:Surface, input:SurfaceInput) -> vec3<f32> {{ return select(vec3<f32>(0.0),max(params.bounce,vec3<f32>(0.0)),ara_glows(input)); }}\n",
        crate::STANDARD_PROGRAM
            .replace(FIELDS, "alpha:f32, glow:vec3<f32> };")
            .replace(
                BUILT,
                "decal_mr.x,alpha,select(vec3<f32>(0.0),max(params.glow,vec3<f32>(0.0)),ara_glows(input)));",
            )
            .replace(
                RETURN,
                "    return min(exposed+exposed_product(s.glow,vec3<f32>(1.0)),vec3<f32>(65504.0));",
            )
            .replace(
                "s.n,s.base,s.alpha,s.metal,s.rough,vec3<f32>(0.0),",
                "s.n,s.base,s.alpha,s.metal,s.rough,s.glow,",
            )
            .replace(
                "    s.alpha=m.alpha;\n    return s;",
                "    s.alpha=m.alpha;\n    s.glow=m.emission;\n    return s;",
            )
    );

    for path in [RenderPath::HardwareRt, RenderPath::NonRt] {
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

        let emissive_prog = engine.create_program("ara_emissive", &emissive_wgsl).unwrap();
        let spawn = |engine: &mut Engine, c, h, colour, roughness| {
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
                    roughness,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap();
            engine.spawn(mesh, material, Mat4::IDENTITY).unwrap()
        };

        // Arena floor, walls, platform, pillars, crates.
        spawn(&mut engine, [0.0, -0.25, 0.0], [20.0, 0.25, 20.0], [0.18, 0.18, 0.18, 1.0], 0.88);
        spawn(&mut engine, [0.0, 2.0, -19.75], [20.0, 2.0, 0.25], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [0.0, 2.0, 19.75], [20.0, 2.0, 0.25], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [-19.75, 2.0, 0.0], [0.25, 2.0, 19.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [19.75, 2.0, 0.0], [0.25, 2.0, 19.5], [0.42, 0.41, 0.39, 1.0], 0.92);
        spawn(&mut engine, [0.0, 1.0, -8.0], [4.0, 1.0, 3.0], [0.18, 0.18, 0.18, 1.0], 0.88);
        for [px, pz] in [[-14.0, -14.0], [14.0, -14.0], [-14.0, 14.0], [14.0, 14.0]] {
            spawn(&mut engine, [px, 2.0, pz], [0.6, 2.0, 0.6], [0.70, 0.67, 0.62, 1.0], 0.90);
        }
        spawn(&mut engine, [-1.2, 2.4, -9.0], [0.4, 0.4, 0.4], [0.4, 0.24, 0.12, 1.0], 0.55);
        spawn(&mut engine, [0.0, 2.4, -9.5], [0.4, 0.4, 0.4], [0.4, 0.24, 0.12, 1.0], 0.55);

        // Movable Light panel (`e_panel`): unit-centered mesh moved via set_transform.
        let panel = {
            let (vertices, indices) = cube([0.0, 0.0, 0.0], [2.0, 0.5, 0.05]);
            let mesh = engine
                .create_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })
                .unwrap();
            let mat = engine
                .create_material(&MaterialDesc {
                    base_color: [0.8, 0.8, 0.8, 0.737],
                    roughness: 0.4,
                    double_sided: true,
                    program: Some(emissive_prog),
                    ..Default::default()
                })
                .unwrap();
            engine
                .set_material_parameters(
                    mat,
                    &[
                        ("glow", crate::Value::Vec3([3000.0, 2250.0, 1450.0])),
                        ("everywhere", crate::Value::F32(0.0)),
                        ("bounce", crate::Value::Vec3([3000.0, 2250.0, 1450.0])),
                        ("mapped", crate::Value::F32(0.0)),
                    ],
                )
                .unwrap();
            let obj = engine
                .spawn(mesh, mat, Mat4::from_translation(Vec3::new(0.0, 3.0, -19.30)))
                .unwrap();
            engine.set_casts_shadows(obj, false).unwrap();
            obj
        };

        let el = 35.0f32.to_radians();
        let az = 32.0f32.to_radians();
        let toward = Vec3::new(-el.cos() * az.sin(), el.sin(), el.cos() * az.cos());
        engine
            .set_sun(Some(Sun {
                travel: -toward.normalize(),
                illuminance: [4.0 * 0.64161, 4.0 * 0.73686, 4.0],
                angular_radius: 0.5357f32.to_radians() * 0.5,
            }))
            .unwrap();
        let lm_to_cd = |lm: f32, c: [f32; 3]| {
            let s = lm / (4.0 * std::f32::consts::PI);
            [c[0] * s, c[1] * s, c[2] * s]
        };
        engine
            .set_lights(&[
                crate::engine::Light::Point {
                    position: Vec3::new(0.0, 3.6, -8.0),
                    candela: lm_to_cd(6000.0, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                },
                crate::engine::Light::Point {
                    position: Vec3::new(-10.0, 4.4, 5.5),
                    candela: lm_to_cd(5000.0, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                },
                crate::engine::Light::Point {
                    position: Vec3::new(12.0, 2.5, 12.0),
                    candela: lm_to_cd(4000.0, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                },
                crate::engine::Light::Spot {
                    position: Vec3::new(0.0, 3.0, -19.35),
                    travel: Vec3::Z,
                    candela: lm_to_cd(8000.0, [1.0, 0.75, 0.48]),
                    range: Some(16.0),
                    inner: 0.0,
                    outer: 1.55,
                    source_radius: 0.5,
                    cast_shadows: Some(true),
                },
            ])
            .unwrap();
        engine.set_sky(crate::engine::Sky::Atmosphere(crate::engine::AtmosphereMedium::default())).unwrap();
        engine
            .set_settings(crate::engine::Settings {
                aerial_perspective: true,
                height_fog: Some(crate::engine::HeightFog {
                    density: 0.004,
                    falloff: 1.0 / 30.0,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();

        let view = engine
            .create_view(&ViewDesc {
                look: ViewLook {
                    exposure: Exposure::Manual { stops: -4.263 },
                    post: crate::PostSettings {
                        bloom: 0.05,
                        motion_blur: false,
                        tonemapper: crate::Tonemapper::Filmic,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..ViewDesc::new(W, H)
            })
            .unwrap();

        let save = |name: &str, img: &Image| {
            let Some(dir) = &dump_dir else { return };
            let p = dir.join(name);
            let file = std::fs::File::create(&p).unwrap();
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&img.rgba8).unwrap();
        };
        let step = |engine: &mut Engine, camera: &Camera| {
            engine.tick(1.0 / 60.0).unwrap();
            engine.render_view(view, camera).unwrap();
        };
        let diff = |a: &[u8], b: &[u8]| -> (f64, f64) {
            let (mut bsq, mut bmax, mut bn) = (0.0f64, 0.0f64, 0u32);
            for ty in (0..H as usize).step_by(24) {
                for tx in (0..W as usize).step_by(24) {
                    if ty + 24 > H as usize || tx + 24 > W as usize {
                        continue;
                    }
                    let (mut dr, mut dg, mut db) = (0.0, 0.0, 0.0);
                    for dy in 0..24 {
                        for dx in 0..24 {
                            let i = ((ty + dy) * W as usize + (tx + dx)) * 4;
                            dr += f64::from(a[i]) - f64::from(b[i]);
                            dg += f64::from(a[i + 1]) - f64::from(b[i + 1]);
                            db += f64::from(a[i + 2]) - f64::from(b[i + 2]);
                        }
                    }
                    let d = (dr * dr + dg * dg + db * db).sqrt() / 576.0;
                    bsq += d * d;
                    bmax = bmax.max(d);
                    bn += 1;
                }
            }
            ((bsq / f64::from(bn)).sqrt(), bmax)
        };
        let tag = if path == RenderPath::HardwareRt { "rt" } else { "nonrt" };

        // --- Part 1: Scene-view drag of emissive `e_panel` back and forth over the floor ---
        let scene_cam = Camera::look_at(
            Vec3::new(-12.0, 12.0, 6.0),
            Vec3::new(0.0, 1.0, -7.0),
            Vec3::Y,
            65.0f32.to_radians(),
            0.05,
            4000.0,
        )
        .unwrap();
        for _ in 0..36 {
            step(&mut engine, &scene_cam);
        }
        // Drag e_panel back and forth across the platform/floor for 24 frames.
        for f in 0..24 {
            let phase = (f as f32 / 23.0) * std::f32::consts::PI * 2.5;
            let z = -11.0 + phase.sin() * 7.5;
            let x = (phase * 0.5).cos() * 3.0;
            engine
                .set_transform(panel, Mat4::from_translation(Vec3::new(x, 2.5, z)))
                .unwrap();
            step(&mut engine, &scene_cam);
        }
        let final_pos = Vec3::new(2.0, 2.5, -3.5);
        engine
            .set_transform(panel, Mat4::from_translation(final_pos))
            .unwrap();
        step(&mut engine, &scene_cam);
        let drag_end = engine.capture_view(view).unwrap();
        save(&format!("dyn_{tag}_drag_end.png"), &drag_end);
        for _ in 0..8 {
            step(&mut engine, &scene_cam);
        }
        let drag_8 = engine.capture_view(view).unwrap();
        save(&format!("dyn_{tag}_drag_8.png"), &drag_8);
        // Clean reference at exact final_pos after cut + settle.
        engine.cut_view(view);
        for _ in 0..64 {
            step(&mut engine, &scene_cam);
        }
        let drag_ref = engine.capture_view(view).unwrap();
        save(&format!("dyn_{tag}_drag_ref.png"), &drag_ref);

        // --- Part 2: First-person jump & walk beside e_panel/e_panellight looking at Pillar 2 ---
        engine
            .set_transform(panel, Mat4::from_translation(Vec3::new(0.0, 3.0, -19.30)))
            .unwrap();
        let fp_cam = |x: f32, y: f32, z: f32| {
            Camera::look_at(
                Vec3::new(x, y, z),
                Vec3::new(14.0, 1.8, -14.5),
                Vec3::Y,
                75.0f32.to_radians(),
                0.05,
                4000.0,
            )
            .unwrap()
        };
        let base_cam = fp_cam(-5.5, 1.65, -14.5);
        let peak_cam = fp_cam(-3.0, 2.95, -15.2);
        engine.cut_view(view);
        for _ in 0..48 {
            step(&mut engine, &base_cam);
        }
        let fp_base_ref = engine.capture_view(view).unwrap();
        // Walk forward + jump to peak over 12 frames.
        for f in 1..=12 {
            let t = f as f32 / 12.0;
            let y = 1.65 + (t * std::f32::consts::FRAC_PI_2).sin() * 1.30;
            let cam = fp_cam(-5.5 + 2.5 * t, y, -14.5 - 0.7 * t);
            step(&mut engine, &cam);
        }
        let fp_peak = engine.capture_view(view).unwrap();
        save(&format!("dyn_{tag}_fp_peak.png"), &fp_peak);
        // Land back to base over 12 frames.
        for f in 1..=12 {
            let t = f as f32 / 12.0;
            let y = 2.95 - (t * std::f32::consts::FRAC_PI_2).sin() * 1.30;
            let cam = fp_cam(-3.0 - 2.5 * t, y, -15.2 + 0.7 * t);
            step(&mut engine, &cam);
        }
        let fp_land = engine.capture_view(view).unwrap();
        save(&format!("dyn_{tag}_fp_land.png"), &fp_land);
        // Clean reference at peak_cam.
        engine.cut_view(view);
        for _ in 0..48 {
            step(&mut engine, &peak_cam);
        }
        let fp_peak_ref = engine.capture_view(view).unwrap();
        save(&format!("dyn_{tag}_fp_peak_ref.png"), &fp_peak_ref);

        let (de_b, de_m) = diff(&drag_end.rgba8, &drag_ref.rgba8);
        let (d8_b, d8_m) = diff(&drag_8.rgba8, &drag_ref.rgba8);
        let (pk_b, pk_m) = diff(&fp_peak.rgba8, &fp_peak_ref.rgba8);
        let (ld_b, ld_m) = diff(&fp_land.rgba8, &fp_base_ref.rgba8);
        eprintln!(
            "dynamic_gi path={tag} drag_end(brms={de_b:.2},bmax={de_m:.2}) drag_8(brms={d8_b:.2},bmax={d8_m:.2}) fp_peak(brms={pk_b:.2},bmax={pk_m:.2}) fp_land(brms={ld_b:.2},bmax={ld_m:.2})"
        );
        if !(d8_b < 8.0 && d8_m < 42.0) {
            errors.push(format!(
                "{tag} moving emitter left stale floor trails/rings 8 frames after drag (brms={d8_b:.2}, bmax={d8_m:.2})"
            ));
        }
        if !(pk_b < 4.0 && ld_b < 3.0) {
            errors.push(format!(
                "{tag} first-person jump flickered walls/lights (peak_brms={pk_b:.2}, peak_bmax={pk_m:.2}, land_brms={ld_b:.2})"
            ));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
