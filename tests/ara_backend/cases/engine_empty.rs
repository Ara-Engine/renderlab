//! A scene with nothing in it, through the backend's engine-level entry: what a new
//! project's empty scene asks for.
use super::*;
use renderlab::{
    Camera, ID_NONE, IdRect, Mat4, MeshData, Vec3, Vertex,
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, Outputs, Quality, Settings, Sky, Sun, ViewDesc, ViewLook},
};

const SIZE: [u32; 2] = [64, 48];

/// With no objects, a view draws the sky through post and the grade, with the cache,
/// reconstruction and fog on, on the exact and the reconstructed frame, and its ids
/// name nothing. The uniform sky reaches every pixel at a level between black and
/// white, and every id is `ID_NONE` in both layers. A floor spawned into the same view
/// then draws, and despawning it leaves an empty scene that draws again: an editor's
/// new scene, its first object and an undo.
#[test]
fn an_empty_scene_draws_its_sky_and_names_nothing() -> Result<()> {
    let path = root("engine-empty");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 2., 6.),
        Vec3::ZERO,
        Vec3::Y,
        60f32.to_radians(),
        0.1,
        100.,
    )?;
    for (quality, frames) in [(Quality::Reference, 1), (Quality::Realtime, 4)] {
        let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, Settings::default())?;
        let view = engine.create_view(&ViewDesc {
            look: ViewLook {
                exposure: Exposure::Manual { stops: -11. },
                quality,
                ..Default::default()
            },
            outputs: Outputs { ids: true },
            ..ViewDesc::new(SIZE[0], SIZE[1])
        })?;
        engine.set_sky(Sky::Uniform([1000.; 3]))?;
        engine.set_sun(Some(Sun {
            travel: Vec3::new(0.2, -1., 0.3).normalize(),
            illuminance: [50_000.; 3],
            angular_radius: 0.02,
        }))?;
        for _ in 0..frames {
            engine.render_view(&mut gpu, view, &camera)?;
        }
        let image = engine.capture_view(view)?;
        let (min, max) = image
            .rgba8
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| &p[..3])
            .fold((u8::MAX, 0), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        ensure!(
            min > 8 && max < 250,
            "{quality:?}: the sky reads {min}..{max}, not a level between black and white"
        );
        let request = engine.request_ids(
            &mut gpu,
            view,
            IdRect {
                x: 0,
                y: 0,
                width: SIZE[0],
                height: SIZE[1],
            },
        )?;
        let events = engine.poll()?;
        let read = events
            .ids
            .iter()
            .find(|r| r.request == request)
            .context("the request did not arrive at poll")?;
        let named = read
            .ids
            .iter()
            .filter(|p| p.opaque != ID_NONE || p.front != ID_NONE)
            .count();
        ensure!(named == 0, "{quality:?}: {named} pixels name something");

        let floor = [[-4., -4.], [4., -4.], [4., 4.], [-4., 4.]].map(|[x, z]| Vertex {
            position: [x, 0., z],
            normal: [0., 1., 0.],
            uv: [0., 0.],
            color: [1.; 4],
        });
        let mesh = engine.create_mesh(&MeshData {
            vertices: &floor,
            indices: &[0, 2, 1, 0, 3, 2],
        })?;
        let material = engine.create_material(&MaterialDesc::default())?;
        let object = engine.spawn(mesh, material, Mat4::IDENTITY)?;
        for _ in 0..frames {
            engine.render_view(&mut gpu, view, &camera)?;
        }
        let with_floor = engine.capture_view(view)?.rgba8;
        ensure!(
            with_floor != image.rgba8,
            "{quality:?}: the spawned floor does not show"
        );
        engine.despawn(object)?;
        for _ in 0..frames {
            engine.render_view(&mut gpu, view, &camera)?;
        }
        engine.retire(&mut gpu)?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
