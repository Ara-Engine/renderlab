//! The game scene (`examples/game_scene.rs`) through the backend's engine-level entry:
//! the example's own `Engine` calls, made on `backend_ara::engine::Engine`. The scene's
//! description is the fixture `support/ara_game_scene.rs`, a copy of the example's.
use super::*;
use renderlab::{
    Camera, LocalShadows, ReflectionSettings, Value, Vec3, ViewId,
    backend_ara::engine::{Engine, Refusal},
    engine::{Exposure, Settings, ViewDesc, ViewLook},
};
#[path = "../../../support/ara_game_scene.rs"]
mod scene;
use scene::{GLOW, Options, STOPS, animate, build, eye, glass_pane, hero, look, settings};

const SIZE: [u32; 2] = [480, 270];

/// Direct light only, so that one object's shading cannot reach another's pixels: no
/// radiance cache, reflections, local-light shadows, fog or history.
fn direct() -> Settings {
    Settings {
        global_illumination: false,
        temporal_reconstruction: false,
        local_shadows: LocalShadows {
            count: 0,
            ..Default::default()
        },
        // Its cells keep a history: a second frame would differ from the first.
        volumetric_fog: None,
        ..settings()
    }
}
/// The scene's view at the test's size, with `look`.
fn view(look: ViewLook) -> ViewDesc {
    ViewDesc {
        look,
        ..ViewDesc::new(SIZE[0], SIZE[1])
    }
}
/// The scene's look without reflections, for `direct`.
fn direct_look() -> ViewLook {
    ViewLook {
        reflections: ReflectionSettings::off(),
        ..look()
    }
}

fn rgb(hdr: &[u8]) -> Vec<[f32; 3]> {
    hdr.as_chunks::<8>()
        .0
        .iter()
        .map(|p| {
            std::array::from_fn(|c| {
                f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32()
            })
        })
        .collect()
}
/// The first frame's exposed HDR from an engine built with the hero on toon or on the
/// props' program, then the same frame with the bulbs' glow set to zero.
fn first_frames(
    engine: &mut Engine,
    gpu: &mut GpuDevice,
    toon_hero: bool,
) -> Result<[Vec<[f32; 3]>; 2]> {
    let view = engine.create_view(&view(direct_look()))?;
    let built = build(
        &mut engine.on(gpu),
        Options {
            toon_hero,
            ..Default::default()
        },
    )?;
    let camera = animate(&mut engine.on(gpu), &built, 0)?;
    engine.render_view(gpu, view, &camera)?;
    let lit = rgb(&engine.images(view).context("no frame")?.hdr);
    engine.set_material_parameters(built.bulb, &[("glow", Value::Vec3([0.; 3]))])?;
    engine.render_view(gpu, view, &camera)?;
    let dark = rgb(&engine.images(view).context("no frame")?.hdr);
    engine.set_material_parameters(built.bulb, &[("glow", Value::Vec3(GLOW))])?;
    Ok([lit, dark])
}

/// The example's scene, built by its own `Engine` calls on the backend's engine.
///
/// The engine names what it left out and nothing else: the scene's particle emitters
/// and its decals.
///
/// With direct light only, at the fly path's first pose:
/// - Every material on its own program, as the scene has them (the props on `game`, the
///   hero on `toon` with its style): the frame shades two programs by their lists.
///   Against an engine whose hero is given the `game` program, which shades the whole
///   region with one pipeline, only pixels near the hero differ, and most of the
///   hero's do. So do the sky's pixels above the horizon agree, which the first
///   program's list writes: they are lit, and bit for bit the one-program frame's.
/// - The bulbs' `glow`, set to zero through `set_material_parameters`: only pixels at
///   the two bulbs differ, and the arch bulb's centre loses the exposed glow less what
///   17 m of haze takes.
///
/// Then the scene's own settings (the radiance cache, traced reflections, local-light
/// shadows, temporal reconstruction, post with the scene's bloom) with the glass pane:
/// six frames of the example's animation run, the hero, spinner, lanterns and camera
/// moving, and the last is written to `target/` as a PNG.
#[test]
fn the_game_scene_builds_and_renders_through_the_engine_calls() -> Result<()> {
    let path = root("game-scene");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, direct())?;
    let [own, dark] = first_frames(&mut engine, &mut gpu, true)?;
    let refused = |call, what| Refusal { call, what };
    ensure!(
        engine.refusals()
            == [
                refused("create_emitter", "particle emitters"),
                refused("set_decals", "decals"),
            ],
        "unexpected refusals: {:?}",
        engine.refusals()
    );
    engine.retire(&mut gpu)?;
    let mut other = Engine::new(ShaderCache::new(path.join("shaders"))?, direct())?;
    let [one, _] = first_frames(&mut other, &mut gpu, false)?;
    other.retire(&mut gpu)?;
    ensure!(
        own.iter().flatten().all(|v| v.is_finite()),
        "the frame holds a value that is not finite"
    );

    let (eye, spheres) = (eye(0.), hero(0.));
    let camera = Camera::look_at(
        eye,
        spheres[0].0.lerp(Vec3::Y, 0.5),
        Vec3::Y,
        55f32.to_radians(),
        0.05,
        500.,
    )?;
    let matrix = camera.matrix(SIZE[0], SIZE[1]);
    // The pixel a world point projects to, and how many pixels one metre covers there.
    let project = |point: Vec3| {
        let ndc = matrix.project_point3(point);
        (
            [
                (ndc.x + 1.) * 0.5 * SIZE[0] as f32,
                (1. - ndc.y) * 0.5 * SIZE[1] as f32,
            ],
            SIZE[1] as f32 * 0.5 / ((point - eye).length() * 27.5f32.to_radians().tan()),
        )
    };
    // Whether pixel `i`'s centre lies within a sphere's outline grown by half its radius
    // and two pixels: every pixel that can show the sphere is inside.
    let near = |spheres: &[(Vec3, f32)], i: usize| {
        let centre = [
            (i as u32 % SIZE[0]) as f32 + 0.5,
            (i as u32 / SIZE[0]) as f32 + 0.5,
        ];
        spheres.iter().any(|&(c, r)| {
            let (at, scale) = project(c);
            (centre[0] - at[0]).hypot(centre[1] - at[1]) < 1.5 * r * scale + 2.
        })
    };
    // The glass pane reflects and refracts the hero (a grazing face's refraction lands far
    // across the frame), so a pixel on the pane, its projected box grown by two pixels, can
    // show the hero too.
    let pane = glass_pane();
    let (low, high) = (0..8).fold(
        ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
        |(low, high), k| {
            let corner = Vec3::new(
                if k & 1 == 0 { -0.5 } else { 0.5 },
                if k & 2 == 0 { -0.5 } else { 0.5 },
                if k & 4 == 0 { -0.5 } else { 0.5 },
            );
            let ([x, y], _) = project(pane.transform_point3(corner));
            (
                [low[0].min(x), low[1].min(y)],
                [high[0].max(x), high[1].max(y)],
            )
        },
    );
    let on_pane = |i: usize| {
        let x = (i as u32 % SIZE[0]) as f32 + 0.5;
        let y = (i as u32 / SIZE[0]) as f32 + 0.5;
        x >= low[0] - 2. && x <= high[0] + 2. && y >= low[1] - 2. && y <= high[1] + 2.
    };
    let bulb_spheres = [
        (Vec3::new(0., 3.55, -3.), 0.12),
        (Vec3::new(3.22, 3.2, 2.78), 0.1),
    ];
    // Rows whose centre rays all leave above the horizon: the 60 m ground ends below it.
    let inverse = matrix.inverse();
    let rises = |y: u32| {
        let far = inverse.project_point3(Vec3::new(
            0.,
            1. - (y as f32 + 0.5) / SIZE[1] as f32 * 2.,
            0.5,
        ));
        (far - eye).normalize().y > 0.02
    };
    let (mut sky, mut heroes, mut bulbs) = (0, 0, 0);
    for (i, ((own, one), dark)) in own.iter().zip(&one).zip(&dark).enumerate() {
        let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
        if rises(y) {
            sky += 1;
            ensure!(
                own == one && own.iter().all(|v| *v > 0.),
                "sky at ({x}, {y}) is {own:?} by lists and {one:?} by one program"
            );
        }
        if own != one {
            heroes += 1;
            ensure!(
                near(&spheres, i) || on_pane(i),
                "the hero's program changed ({x}, {y}), away from the hero: {own:?} against {one:?}"
            );
        }
        if own != dark {
            bulbs += 1;
            ensure!(
                near(&bulb_spheres, i),
                "the bulbs' glow changed ({x}, {y}), away from the bulbs: {own:?} against {dark:?}"
            );
        }
    }
    // The hero's outlines hold about 314 and 127 pixels at this size, and the gold
    // sphere stands in front of most of the body.
    ensure!(
        sky > 5000 && heroes > 120 && bulbs >= 4,
        "the fixture lost its regions: {sky} sky pixels, {heroes} changed by the hero's program, {bulbs} by the glow"
    );
    let (at, _) = project(bulb_spheres[0].0);
    let centre = at[1] as usize * SIZE[0] as usize + at[0] as usize;
    let exposure = 2f32.powf(STOPS);
    let mut kept = 1f32;
    for c in 0..3 {
        let (gained, emitted) = (own[centre][c] - dark[centre][c], GLOW[c] * exposure);
        kept = kept.min(gained / emitted);
        ensure!(
            gained <= emitted * 1.001 && gained >= emitted * 0.99,
            "the arch bulb's centre gained {gained} in channel {c}; its glow exposes to {emitted}"
        );
    }

    // The scene's own settings, and its animation.
    let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, settings())?;
    let view = engine.create_view(&view(look()))?;
    let built = build(&mut engine.on(&mut gpu), Options::default())?;
    let mut report = None;
    for frame in 0..6 {
        let camera = animate(&mut engine.on(&mut gpu), &built, frame)?;
        report = Some(engine.render_view(&mut gpu, view, &camera)?);
    }
    // The displayed frame stays on the device for a presenter, at the output's size.
    let output = engine
        .view_output(view)
        .context("no displayed image on the device")?;
    ensure!(
        output.size == SIZE,
        "the displayed image is {:?}, the output {SIZE:?}",
        output.size
    );
    let report = report.context("no frame")?;
    let images = engine.images(view).context("no frame")?;
    ensure!(
        rgb(&images.hdr).iter().flatten().all(|v| v.is_finite()),
        "the full frame holds a value that is not finite"
    );
    let image = engine.capture_view(view)?;
    let lit = image
        .rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] != [0; 3])
        .count();
    ensure!(
        lit * 10 > image.rgba8.len() / 4 * 9,
        "the displayed frame is mostly black: {lit} lit pixels"
    );
    let file = path.join("game-scene.png");
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(&file)?),
        image.width,
        image.height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.rgba8)?;
    eprintln!(
        "game scene: {} draws, {} triangles; {sky} sky pixels exact, {heroes} pixels changed by the hero's program and {bulbs} by the glow, all in place; the arch bulb keeps {kept:.4} of its glow; frame 6 at {}",
        report.scene.draws,
        report.scene.triangles,
        file.display()
    );
    engine.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// The views test's size: smaller, since it renders the scene sixteen times.
const VIEW_SIZE: [u32; 2] = [240, 135];

/// The second view's camera: across the scene from the first's, looking back.
fn across_camera() -> Result<Camera> {
    Ok(Camera::look_at(
        Vec3::new(4., 3., 12.),
        Vec3::new(0., 1., 2.),
        Vec3::Y,
        50f32.to_radians(),
        0.05,
        500.,
    )?)
}

/// The game scene seen by two views at once, from the fly path and from across the
/// scene, with the radiance cache, reconstruction and fog on, rendered in turn for four
/// frames: each view's last frame is the same bytes as a single-view engine that
/// renders that camera alone. Views share the scene and nothing of each other's frames.
/// Then a view resized by `set_view` draws at its new size, and a removed view is gone.
#[test]
fn two_views_each_draw_what_one_view_alone_draws() -> Result<()> {
    let path = root("engine-views");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let desc = ViewDesc {
        look: ViewLook {
            exposure: Exposure::Manual {
                stops: scene::STOPS,
            },
            ..look()
        },
        ..ViewDesc::new(VIEW_SIZE[0], VIEW_SIZE[1])
    };
    let second = across_camera()?;
    // Four frames of the scene with one view for each camera `which` names (0: the fly
    // path, 1: across), and each view's last exposed HDR.
    let mut run = |which: &[usize]| -> Result<Vec<Vec<u8>>> {
        let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, Settings::default())?;
        let views: Vec<ViewId> = which
            .iter()
            .map(|_| engine.create_view(&desc))
            .collect::<Result<_, _>>()?;
        let built = build(&mut engine.on(&mut gpu), Options::default())?;
        for frame in 0..4 {
            let path_camera = animate(&mut engine.on(&mut gpu), &built, frame)?;
            for (view, camera) in views.iter().zip(which) {
                let camera = if *camera == 0 { path_camera } else { second };
                engine.render_view(&mut gpu, *view, &camera)?;
            }
        }
        let images = views
            .iter()
            .map(|view| Ok(engine.images(*view).context("no frame")?.hdr.clone()))
            .collect::<Result<_>>()?;
        engine.retire(&mut gpu)?;
        Ok(images)
    };
    let both = run(&[0, 1])?;
    let [alone_path, alone_across] = [run(&[0])?, run(&[1])?];
    // Half-float values that differ, of all of them.
    let differing =
        |a: &[u8], b: &[u8]| a.chunks(2).zip(b.chunks(2)).filter(|(x, y)| x != y).count();
    ensure!(
        both[0] == alone_path[0],
        "the fly-path view differs from the same camera alone in {} of {} values",
        differing(&both[0], &alone_path[0]),
        both[0].len() / 2
    );
    ensure!(
        both[1] == alone_across[0],
        "the view across the scene differs from the same camera alone in {} of {} values",
        differing(&both[1], &alone_across[0]),
        both[1].len() / 2
    );
    ensure!(both[0] != both[1], "the two cameras drew the same image");

    // A resized view draws at its new size; a removed one is gone.
    let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, Settings::default())?;
    let view = engine.create_view(&desc)?;
    let built = build(&mut engine.on(&mut gpu), Options::default())?;
    let camera = animate(&mut engine.on(&mut gpu), &built, 0)?;
    engine.render_view(&mut gpu, view, &camera)?;
    engine.set_view(
        view,
        &ViewDesc {
            width: 160,
            height: 90,
            ..desc.clone()
        },
    )?;
    engine.render_view(&mut gpu, view, &camera)?;
    let output = engine.view_output(view).context("no output")?;
    ensure!(
        output.size == [160, 90],
        "resized view drew at {:?}",
        output.size
    );
    ensure!(
        engine.capture_view(view)?.rgba8.len() == 160 * 90 * 4,
        "the resized capture has the old size"
    );
    engine.remove_view(view)?;
    ensure!(
        engine.render_view(&mut gpu, view, &camera).is_err() && engine.views().count() == 0,
        "a removed view still renders"
    );
    engine.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
