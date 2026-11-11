//! The colour LUT through the backend's engine-level entry.
use super::*;
use renderlab::{
    Camera, ColourLut, Mat4, MeshData, PostSettings, Vec3, Vertex,
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, Settings, Sky, Sun},
};

const SIZE: [u32; 2] = [64, 48];

/// sRGB's transfer, to an 8-bit display value.
fn encoded(linear: f32) -> f32 {
    let x = f64::from(linear);
    let v = if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1. / 2.4) - 0.055
    };
    (v * 255.) as f32
}

/// A lit floor under a uniform sky, graded with no vignette and no grain:
/// - A constant LUT at full strength makes every displayed pixel its value: the LUT's
///   RGBA8 texel, sRGB-encoded, within one display level.
/// - An explicit 16-cube identity LUT shows the frame without a LUT within one level at
///   every pixel (its RGBA8 nodes are rounded).
/// - Half strength lands between the two.
///
/// That holds the LUT's upload, its size and strength in the post record, and that the
/// engine keeps it.
#[test]
fn a_colour_lut_grades_the_displayed_frame() -> Result<()> {
    let path = root("engine-lut");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let floor: Vec<Vertex> = [[-20., -20.], [20., -20.], [20., 20.], [-20., 20.]]
        .into_iter()
        .map(|[x, z]: [f32; 2]| Vertex {
            position: [x, 0., z],
            normal: [0., 1., 0.],
            uv: [x, z],
            color: [1.; 4],
        })
        .collect();
    let camera = Camera::look_at(
        Vec3::new(0., 4., -2.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let constant = [0.5f32, 0.25, 0.75];
    let mut frame = |lut: Option<ColourLut>, strength: f32| -> Result<Vec<u8>> {
        let settings = Settings {
            exposure: Exposure::Manual { stops: -11. },
            volumetric_fog: None,
            post: PostSettings {
                vignette: 0.,
                grain: 0.,
                lut_strength: strength,
                ..PostSettings::default()
            },
            ..Default::default()
        };
        let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, settings)?;
        engine.resize(SIZE[0], SIZE[1])?;
        engine.set_sky(Sky::Uniform([2000.; 3]))?;
        engine.set_sun(Some(Sun {
            travel: Vec3::new(0.3, -1., 0.5).normalize(),
            illuminance: [50_000.; 3],
            angular_radius: 0.02,
        }))?;
        engine.set_colour_lut(lut)?;
        let mesh = engine.create_mesh(&MeshData {
            vertices: &floor,
            indices: &[0, 2, 1, 0, 3, 2],
        })?;
        let material = engine.create_material(&MaterialDesc {
            base_color: [0.6, 0.5, 0.4, 1.],
            double_sided: true,
            ..Default::default()
        })?;
        engine.spawn(mesh, material, Mat4::IDENTITY)?;
        engine.render(&mut gpu, &camera)?;
        let image = engine.capture()?;
        let refusals = engine.refusals().to_vec();
        engine.retire(&mut gpu)?;
        ensure!(refusals.is_empty(), "refused: {refusals:?}");
        Ok(image.rgba8)
    };
    let flat = ColourLut {
        size: 2,
        rgb: vec![constant; 8],
    };
    let graded = frame(Some(flat.clone()), 1.)?;
    // The texel the LUT holds, as RGBA8.
    let expected = constant.map(|c| encoded((c * 255.).round() / 255.));
    let off = graded
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| (0..3).map(move |c| (f32::from(p[c]) - expected[c]).abs()))
        .fold(0f32, f32::max);
    ensure!(
        off <= 1.,
        "a constant LUT left the display {off} levels from its value"
    );
    let plain = frame(None, 1.)?;
    let identity = frame(Some(ColourLut::identity(16)?), 1.)?;
    let apart = plain
        .iter()
        .zip(&identity)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    ensure!(
        apart <= 1,
        "an identity LUT moved the display by {apart} levels"
    );
    let half = frame(Some(flat), 0.5)?;
    // Within a level of the range between the frame without a grade and the full one.
    let between = half
        .iter()
        .zip(plain.iter().zip(&graded))
        .all(|(&h, (&p, &g))| h.saturating_add(1) >= p.min(g) && h <= p.max(g).saturating_add(1));
    ensure!(
        between,
        "half strength left the range between no grade and the full one"
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
