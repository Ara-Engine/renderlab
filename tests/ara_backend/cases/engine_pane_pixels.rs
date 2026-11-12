//! Per-pixel pane shading (`shaders/pane_pixels.wgsl`) against the glass raster it replaces,
//! through the backend's engine-level entry, in motion.
use super::*;
use renderlab::{
    Camera, Mat4, MeshData, Transparency, Vec3, Vertex,
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, Settings, Sky, Sun, ViewDesc},
};
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../../support/ara_game_scene.rs"]
mod scene;
use scene::{Options, animate, build, look};

const SIZE: [u32; 2] = [480, 270];

/// Runs `run` with the panes drawn by the glass raster (`raster`) or per pixel.
fn with_panes<T>(raster: bool, run: impl FnOnce() -> Result<T>) -> Result<T> {
    // SAFETY: the backend suite runs on one thread (`--test-threads=1`, docs/ara-gpu-backend.md),
    // so no other thread reads the environment while this one writes it.
    unsafe {
        if raster {
            std::env::set_var("RENDERLAB_ARA_PANES", "raster");
        } else {
            std::env::remove_var("RENDERLAB_ARA_PANES");
        }
    }
    let result = run();
    // SAFETY: as above.
    unsafe { std::env::remove_var("RENDERLAB_ARA_PANES") };
    result
}

/// How two displayed frames differ: pixels off by more than one code, and the largest
/// difference in any channel.
fn compare(a: &[u8], b: &[u8]) -> (usize, u8) {
    let mut over = 0;
    let mut worst = 0;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
        over += usize::from(d > 1);
        worst = worst.max(d);
    }
    (over, worst)
}

/// A sphere of `radius` at `centre`, `rings` by `2 * rings` quads, facing out.
fn sphere(centre: Vec3, radius: f32, rings: u32) -> (Vec<Vertex>, Vec<u32>) {
    let segments = rings * 2;
    let mut vertices = Vec::new();
    for r in 0..=rings {
        let theta = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let phi = std::f32::consts::TAU * s as f32 / segments as f32;
            let n = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            vertices.push(Vertex {
                position: (centre + n * radius).to_array(),
                normal: n.to_array(),
                uv: [s as f32 / segments as f32, r as f32 / rings as f32],
                color: [1.; 4],
            });
        }
    }
    let mut indices = Vec::new();
    let row = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let (a, b) = (r * row + s, (r + 1) * row + s);
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    (vertices, indices)
}

/// A quad of `half` metres a side at depth `z`, facing +Z.
fn quad(half: f32, z: f32) -> [Vertex; 4] {
    [[-half, -half], [half, -half], [half, half], [-half, half]].map(|[x, y]| Vertex {
        position: [x, y, z],
        normal: [0., 0., 1.],
        uv: [0., 0.],
        color: [1.; 4],
    })
}

/// The game scene's fly path for eight frames (its glass pane in view, with the cache,
/// reconstruction under its jitter, fog and post on), drawn with per-pixel panes and with the
/// glass raster: every pixel of every frame, the cold first one included, within a code of
/// the raster, beyond what the raster's own runs differ by (up to a code from one run to the
/// next: the scene is not bit-reproducible across processes' GPU work).
#[test]
fn per_pixel_panes_draw_the_game_scene_as_the_glass_raster_does_in_motion() -> Result<()> {
    let path = root("engine-pane-pixels");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let mut run = |raster: bool| -> Result<Vec<Vec<u8>>> {
        with_panes(raster, || {
            let mut engine =
                Engine::new(ShaderCache::new(path.join("shaders"))?, scene::settings())?;
            let view = engine.create_view(&ViewDesc {
                look: renderlab::engine::ViewLook {
                    exposure: Exposure::Manual {
                        stops: scene::STOPS,
                    },
                    ..look()
                },
                ..ViewDesc::new(SIZE[0], SIZE[1])
            })?;
            let built = build(&mut engine.on(&mut gpu), Options::default())?;
            let mut frames = Vec::new();
            for frame in 0..8 {
                let camera = animate(&mut engine.on(&mut gpu), &built, frame)?;
                engine.render_view(&mut gpu, view, &camera)?;
                frames.push(engine.capture_view(view)?.rgba8);
            }
            engine.retire(&mut gpu)?;
            Ok(frames)
        })
    };
    let raster = run(true)?;
    let again = run(true)?;
    let pixels = run(false)?;
    for (frame, ((a, b), c)) in raster.iter().zip(&pixels).zip(&again).enumerate() {
        let (over, worst) = compare(a, b);
        let noise = compare(a, c).1;
        println!(
            "game scene frame {frame}: {over} pixels over a code, at most {worst} (raster runs differ by {noise})"
        );
        ensure!(
            worst <= noise + 1,
            "frame {frame}: {over} pixels differ by more than a code, up to {worst}, where two raster runs differ by {noise}"
        );
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Glass on glass, in motion: a glass ball in front of a glass pane, a second ball behind the
/// pane seen through it, before a coloured backdrop, from a camera that moves each frame (with
/// reconstruction's jitter). Several panes on one pixel keep their order and blend, and the
/// light refracted through both balls and the pane is the glass raster's within a code but for
/// at most one pixel in 2000, within three (measured: 6 to 19 pixels a frame of 129,600, at
/// most 3 codes: interpolation at grazing angles, which the refraction lookup magnifies).
#[test]
fn per_pixel_panes_keep_glass_on_glass_in_motion() -> Result<()> {
    let path = root("engine-pane-overlap");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let mut run = |raster: bool| -> Result<Vec<Vec<u8>>> {
        with_panes(raster, || {
            let mut engine = Engine::new(
                ShaderCache::new(path.join("shaders"))?,
                Settings {
                    volumetric_fog: None,
                    ..Default::default()
                },
            )?;
            let view = engine.create_view(&ViewDesc {
                look: renderlab::engine::ViewLook {
                    exposure: Exposure::Manual { stops: -11. },
                    ..Default::default()
                },
                ..ViewDesc::new(SIZE[0], SIZE[1])
            })?;
            engine.set_sky(Sky::Uniform([800.; 3]))?;
            engine.set_sun(Some(Sun {
                travel: Vec3::new(0.3, -1., -0.4).normalize(),
                illuminance: [60_000.; 3],
                angular_radius: 0.02,
            }))?;
            let opaque = |engine: &mut Engine, colour: [f32; 4]| {
                engine.create_material(&MaterialDesc {
                    base_color: colour,
                    double_sided: true,
                    ..Default::default()
                })
            };
            let glass = |engine: &mut Engine, tint: [f32; 4], thickness: f32| {
                engine.create_material(&MaterialDesc {
                    base_color: tint,
                    transparency: Some(Transparency {
                        thickness,
                        ..Default::default()
                    }),
                    double_sided: true,
                    ..Default::default()
                })
            };
            // A striped backdrop, so refraction shows as a shift of stripes.
            for k in 0..8 {
                let colour = if k % 2 == 0 {
                    [0.8, 0.2, 0.1, 1.]
                } else {
                    [0.1, 0.3, 0.8, 1.]
                };
                let x = -4. + k as f32;
                let strip = [[x, -3.], [x + 1., -3.], [x + 1., 3.], [x, 3.]].map(|[x, y]| Vertex {
                    position: [x, y, -4.],
                    normal: [0., 0., 1.],
                    uv: [0., 0.],
                    color: [1.; 4],
                });
                let mesh = engine.create_mesh(&MeshData {
                    vertices: &strip,
                    indices: &[0, 1, 2, 0, 2, 3],
                })?;
                let material = opaque(&mut engine, colour)?;
                engine.spawn(mesh, material, Mat4::IDENTITY)?;
            }
            let pane = engine.create_mesh(&MeshData {
                vertices: &quad(1.6, -1.),
                indices: &[0, 1, 2, 0, 2, 3],
            })?;
            let tinted = glass(&mut engine, [0.7, 1., 0.8, 0.15], 0.02)?;
            engine.spawn(pane, tinted, Mat4::IDENTITY)?;
            for (centre, radius) in [
                (Vec3::new(-0.3, 0.1, 0.4), 0.6),
                (Vec3::new(0.5, -0.2, -2.2), 0.7),
            ] {
                let (vertices, indices) = sphere(centre, radius, 24);
                let mesh = engine.create_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })?;
                let clear = glass(&mut engine, [1., 1., 1., 0.1], 0.)?;
                engine.spawn(mesh, clear, Mat4::IDENTITY)?;
            }
            let mut frames = Vec::new();
            for frame in 0..6 {
                let t = frame as f32 * 0.04;
                let camera = Camera::look_at(
                    Vec3::new(0.4 * t, 0.2 - 0.3 * t, 4.),
                    Vec3::new(0., 0., -1.),
                    Vec3::Y,
                    50f32.to_radians(),
                    0.1,
                    100.,
                )?;
                engine.render_view(&mut gpu, view, &camera)?;
                frames.push(engine.capture_view(view)?.rgba8);
            }
            engine.retire(&mut gpu)?;
            Ok(frames)
        })
    };
    let raster = run(true)?;
    let pixels = run(false)?;
    for (frame, (a, b)) in raster.iter().zip(&pixels).enumerate() {
        let (over, worst) = compare(a, b);
        println!("glass on glass frame {frame}: {over} pixels over a code, at most {worst}");
        ensure!(
            over * 2000 <= a.len() / 4 && worst <= 3,
            "frame {frame}: {over} pixels differ by more than a code, up to {worst}"
        );
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
