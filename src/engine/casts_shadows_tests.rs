//! `Engine::set_casts_shadows` on the GPU: a light inside its own closed opaque fitting, which
//! blocks all of its light: a point light in a shade, and a downward spot in a lamp head with
//! walls 15-25 cm away (Outpost's floodlights sit 5-23 cm inside theirs). GPU tests: run in a
//! lease (the RT ones with --ignored).
use super::view_tests::cube;
use super::*;

const SIZE: [u32; 2] = [160, 96];

/// The fitting about a light 1 m up over a 12 m floor, no sun, after eight frames: a point
/// light in a 1 m shade, or (`spot`) a wide spot aimed down in a 0.5 x 0.3 x 0.5 m head. `None`
/// leaves the fitting out, `Some(casts)` places it with that flag; `lit` false leaves the light
/// out (the sky's share of the floor). Returns the frame and the fitting's screen rectangle.
fn lamp(path: RenderPath, shade: Option<bool>, spot: bool, lit: bool) -> (Image, [usize; 4]) {
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
    let mut spawn = |engine: &mut Engine, c, h, colour| {
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
                double_sided: true,
                ..Default::default()
            })
            .unwrap();
        engine.spawn(mesh, material, Mat4::IDENTITY).unwrap()
    };
    spawn(
        &mut engine,
        [0.0, -0.05, 0.0],
        [6.0, 0.05, 6.0],
        [0.7, 0.7, 0.7, 1.0],
    );
    let centre = [0.0, 1.0, 0.0];
    let half = if spot {
        [0.25, 0.15, 0.25]
    } else {
        [0.5, 0.5, 0.5]
    };
    if let Some(casts) = shade {
        let shade = spawn(&mut engine, centre, half, [0.8, 0.8, 0.8, 1.0]);
        engine.set_casts_shadows(shade, casts).unwrap();
    }
    engine.set_sun(None).unwrap();
    engine
        .set_lights(
            &[if !lit {
                None
            } else if spot {
                Some(Light::Spot {
                    position: Vec3::from(centre),
                    travel: Vec3::NEG_Y,
                    candela: [1000.0; 3],
                    range: None,
                    inner: 0.9,
                    outer: 1.2,
                    source_radius: 0.0,
                    cast_shadows: Some(true),
                })
            } else {
                Some(Light::Point {
                    position: Vec3::from(centre),
                    candela: [1000.0; 3],
                    range: None,
                    source_radius: 0.0,
                    cast_shadows: Some(true),
                })
            }]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        )
        .unwrap();
    let view = engine
        .create_view(&ViewDesc {
            look: ViewLook {
                exposure: Exposure::Manual { stops: -8.0 },
                ..Default::default()
            },
            ..ViewDesc::new(SIZE[0], SIZE[1])
        })
        .unwrap();
    let camera = Camera::look_at(
        Vec3::new(0.0, 7.0, 5.0),
        Vec3::ZERO,
        Vec3::Y,
        0.9,
        0.05,
        100.0,
    )
    .unwrap();
    for _ in 0..8 {
        engine.tick(1.0 / 60.0).unwrap();
        engine.render_view(view, &camera).unwrap();
    }
    // The box's corners on screen, padded by six pixels.
    let clip = camera.matrix(SIZE[0], SIZE[1]);
    let (mut lo, mut hi) = (
        glam::Vec2::splat(f32::INFINITY),
        glam::Vec2::splat(f32::NEG_INFINITY),
    );
    for i in 0..8 {
        let corner = Vec3::from(std::array::from_fn(|a| {
            centre[a] + if i & (1 << a) == 0 { -half[a] } else { half[a] }
        }));
        let p = clip * corner.extend(1.0);
        let pixel = glam::Vec2::new(
            (p.x / p.w * 0.5 + 0.5) * SIZE[0] as f32,
            (0.5 - p.y / p.w * 0.5) * SIZE[1] as f32,
        );
        (lo, hi) = (lo.min(pixel), hi.max(pixel));
    }
    let rect = [
        (lo.x - 6.0).max(0.0) as usize,
        (lo.y - 6.0).max(0.0) as usize,
        ((hi.x + 6.0) as usize).min(SIZE[0] as usize),
        ((hi.y + 6.0) as usize).min(SIZE[1] as usize),
    ];
    (engine.capture_view(view).unwrap(), rect)
}

/// Mean of the RGB bytes inside (`inside`) or outside the rectangle.
fn mean(image: &Image, [x0, y0, x1, y1]: [usize; 4], inside: bool) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for (i, p) in image.rgba8.chunks(4).enumerate() {
        let (x, y) = (i % SIZE[0] as usize, i / SIZE[0] as usize);
        if (x0..x1).contains(&x) && (y0..y1).contains(&y) == inside {
            sum += p[..3].iter().map(|&v| u64::from(v)).sum::<u64>();
            n += 3;
        }
    }
    sum as f64 / n.max(1) as f64
}

/// Outside the box, the lamp's own share of the floor (over the sky's): a casting box blocks
/// it, a non-casting one lets it out as if the box were absent; inside it, both draw the box.
fn check(path: RenderPath, spot: bool) {
    let (open, rect) = lamp(path, None, spot, true);
    let (lit, _) = lamp(path, Some(false), spot, true);
    let (shut, _) = lamp(path, Some(true), spot, true);
    let (sky, _) = lamp(path, None, spot, false);
    let sky = mean(&sky, rect, false);
    let floor = [
        mean(&open, rect, false) - sky,
        mean(&lit, rect, false) - sky,
        mean(&shut, rect, false) - sky,
    ];
    assert!(
        floor[0] > 10.0,
        "{path:?}: the open lamp lights the floor ({floor:?} over sky {sky})"
    );
    // Without RT a casting fitting blocks only about a quarter of its light today (cs2: point
    // 49.4 -> 37.0, spot 29.1 -> 22.9; RT blocks it all), a gap older than this flag
    // (docs/m7-gaps.md), so that path only has to darken.
    let blocked = if path == RenderPath::HardwareRt {
        0.2
    } else {
        0.9
    };
    assert!(
        floor[2] < blocked * floor[0],
        "{path:?}: a casting fitting blocks its light ({floor:?} over sky {sky})"
    );
    let difference = {
        let [x0, y0, x1, y1] = rect;
        let (mut sum, mut n) = (0u64, 0u64);
        for (i, (a, b)) in open.rgba8.chunks(4).zip(lit.rgba8.chunks(4)).enumerate() {
            let (x, y) = (i % SIZE[0] as usize, i / SIZE[0] as usize);
            if !((x0..x1).contains(&x) && (y0..y1).contains(&y)) {
                sum += (0..3).map(|c| u64::from(a[c].abs_diff(b[c]))).sum::<u64>();
                n += 3;
            }
        }
        sum as f64 / n as f64
    };
    assert!(
        difference < 2.0,
        "{path:?}: a non-casting shade lights the floor as no shade does (mean difference {difference})"
    );
    let shade = [mean(&open, rect, true), mean(&lit, rect, true)];
    assert!(
        (shade[0] - shade[1]).abs() > 5.0,
        "{path:?}: the non-casting shade is still drawn ({shade:?})"
    );
}

#[test]
fn a_non_casting_lamp_shade_lets_its_light_out_without_rt() {
    check(RenderPath::NonRt, false);
}

#[test]
fn a_non_casting_spot_head_lets_its_light_out_without_rt() {
    check(RenderPath::NonRt, true);
}

#[test]
#[ignore]
fn a_non_casting_lamp_shade_lets_its_light_out_with_rt() {
    check(RenderPath::HardwareRt, false);
}

#[test]
#[ignore]
fn a_non_casting_spot_head_lets_its_light_out_with_rt() {
    check(RenderPath::HardwareRt, true);
}
