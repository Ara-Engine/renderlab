//! Volumetric fog through the backend's engine-level entry.
use super::*;
use renderlab::{
    Camera, LocalShadows, Mat4, MeshData, ReflectionSettings, Vec3, Vertex, VolumetricFog,
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, Settings, Sky, Sun, ViewDesc, ViewLook},
};

const SIZE: [u32; 2] = [96, 64];

/// A square of `half` metres a side at height `y`, facing +Y.
fn square(half: f32, y: f32) -> Vec<Vertex> {
    [[-half, -half], [half, -half], [half, half], [-half, half]]
        .into_iter()
        .map(|[x, z]| Vertex {
            position: [x, y, z],
            normal: [0., 1., 0.],
            uv: [0., 0.],
            color: [1.; 4],
        })
        .collect()
}

/// A camera 2 m above a black floor a kilometre wide, looking along it toward a low sun,
/// in fog that scatters forward; the fog's light is the frame with fog less the same
/// frame without.
/// - Fog adds light everywhere on the floor, and more toward the horizon.
/// - A roof 40 m up, as wide as the floor, shades every froxel from the sun: the light
///   the fog adds falls to under half of the open frame's, since only the sky's ambient
///   share remains. That is the fog's sun visibility, one ray a sample against the
///   frame's scene, as the wgpu renderer's hardware path traces it.
/// - For a still camera the fog's history holds its jittered samples together: with a
///   history weight of 0.9 its sixth frame changes from the fifth by under half what
///   it does without history.
///
/// Nothing is refused.
#[test]
fn volumetric_fog_lights_the_air_and_the_scene_shades_it() -> Result<()> {
    let path = root("engine-fog");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let floor = square(1000., 0.);
    let roof = square(1000., 40.);
    let camera = Camera::look_at(
        Vec3::new(0., 2., 0.),
        Vec3::new(0., 1., 30.),
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        2000.,
    )?;
    // The exposed HDR luminance of each pixel, for the last two frames.
    let mut frames = |fogged: bool, roofed: bool, history: f32| -> Result<[Vec<f32>; 2]> {
        let settings = Settings {
            global_illumination: false,
            temporal_reconstruction: false,
            local_shadows: LocalShadows {
                count: 0,
                ..Default::default()
            },
            height_fog: None,
            volumetric_fog: fogged.then_some(VolumetricFog {
                distance: 128.,
                extra_density: 0.01,
                history_weight: history,
            }),
            ..Default::default()
        };
        let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, settings)?;
        let view = engine.create_view(&ViewDesc {
            look: ViewLook {
                exposure: Exposure::Manual { stops: -11. },
                reflections: ReflectionSettings::off(),
                ..Default::default()
            },
            ..ViewDesc::new(SIZE[0], SIZE[1])
        })?;
        engine.set_sky(Sky::Uniform([1500., 1700., 2000.]))?;
        engine.set_sun(Some(Sun {
            travel: Vec3::new(0.1, -0.3, -1.).normalize(),
            illuminance: [50_000.; 3],
            angular_radius: 0.02,
        }))?;
        let material = engine.create_material(&MaterialDesc {
            // Black and rough, so what the frames show is the fog's own light.
            base_color: [0., 0., 0., 1.],
            roughness: 1.,
            double_sided: true,
            ..Default::default()
        })?;
        for (vertices, wanted) in [(&floor, true), (&roof, roofed)] {
            if wanted {
                let mesh = engine.create_mesh(&MeshData {
                    vertices,
                    indices: &[0, 2, 1, 0, 3, 2],
                })?;
                engine.spawn(mesh, material, Mat4::IDENTITY)?;
            }
        }
        let mut last = Vec::new();
        for _ in 0..6 {
            engine.render_view(&mut gpu, view, &camera)?;
            let luminance: Vec<f32> = engine
                .images(view)
                .context("no frame")?
                .hdr
                .as_chunks::<8>()
                .0
                .iter()
                .map(|p| {
                    let c = |i: usize| {
                        f16::from_bits(u16::from_le_bytes([p[2 * i], p[2 * i + 1]])).to_f32()
                    };
                    0.2126 * c(0) + 0.7152 * c(1) + 0.0722 * c(2)
                })
                .collect();
            last.push(luminance);
        }
        let refusals = engine.refusals().to_vec();
        engine.retire(&mut gpu)?;
        ensure!(refusals.is_empty(), "refused: {refusals:?}");
        ensure!(
            last.iter().flatten().all(|v| v.is_finite()),
            "a nonfinite pixel"
        );
        let second = last.pop().context("frames")?;
        let first = last.pop().context("frames")?;
        Ok([first, second])
    };
    let added = |fog: &[f32], clear: &[f32]| -> Vec<f32> {
        fog.iter().zip(clear).map(|(f, c)| f - c).collect()
    };
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let [settling, open_fog] = frames(true, false, 0.9)?;
    let [_, open] = frames(false, false, 0.9)?;
    let [_, roofed_fog] = frames(true, true, 0.9)?;
    let [_, roofed] = frames(false, true, 0.9)?;
    let [fresh_before, fresh] = frames(true, false, 0.)?;
    let open_added = added(&open_fog, &open);
    let roofed_added = added(&roofed_fog, &roofed);
    // The floor's rows: dark without fog (its sheen only), where the sky's misses read
    // about 0.8.
    let width = SIZE[0] as usize;
    let floor_rows: Vec<usize> = (0..SIZE[1] as usize)
        .filter(|row| {
            open[row * width..(row + 1) * width]
                .iter()
                .all(|v| *v < 0.3)
        })
        .collect();
    ensure!(floor_rows.len() > 8, "{} floor rows", floor_rows.len());
    let rows = |image: &[f32], rows: &[usize]| -> Vec<f32> {
        rows.iter()
            .flat_map(|row| image[row * width..(row + 1) * width].iter().copied())
            .collect()
    };
    let quarter = floor_rows.len() / 4;
    // The floor's first rows are nearest the horizon: they look furthest through the fog.
    let far = mean(&rows(&open_added, &floor_rows[..quarter]));
    let near = mean(&rows(
        &open_added,
        &floor_rows[floor_rows.len() - quarter..],
    ));
    let open_floor = rows(&open_added, &floor_rows);
    ensure!(
        open_floor.iter().all(|v| *v > 0.) && far > near,
        "fog's added light on the floor: far {far:.4}, near {near:.4}, least {:.4}",
        open_floor.iter().copied().fold(f32::MAX, f32::min)
    );
    let (lit, shaded) = (mean(&open_floor), mean(&rows(&roofed_added, &floor_rows)));
    ensure!(
        shaded < 0.5 * lit,
        "the roof left the fog {shaded:.4} of light against {lit:.4} in the open"
    );
    // Mean relative change between two frames.
    let change = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs() / b.max(1e-3))
            .sum::<f32>()
            / a.len() as f32
    };
    let (drift, fresh_drift) = (change(&settling, &open_fog), change(&fresh_before, &fresh));
    ensure!(
        drift < 0.5 * fresh_drift,
        "with history the fog moved {drift:.5} a frame, without it {fresh_drift:.5}"
    );
    eprintln!(
        "engine fog: added {lit:.4} open (far {far:.4}, near {near:.4}), {shaded:.4} under the roof; a frame's change {drift:.5} with history, {fresh_drift:.5} without"
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
