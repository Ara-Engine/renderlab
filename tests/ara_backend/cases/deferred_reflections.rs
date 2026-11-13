//! Deferred reflections, the program side (`program::deferred_variant`): what shading writes
//! for the composite reproduces the inline traced colour.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, ReflectionSettings, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];

/// The composite's identity on the standard program: with reflections traced, every pixel is
/// the untraced frame plus `w x (reflectance x traced - untraced)`, from the reflectance,
/// traced fraction and untraced specular shading wrote (bindings 62 and 63) and the inline
/// traced estimate (63's alpha). Grey materials under white light keep the traced estimate
/// grey, so its luminance stands for it in every channel. A glossy grey metal floor reflects
/// a near-black wall, where the untraced estimate sees the bright sky.
///
/// Runs with `RENDERLAB_RL_DEFERRED=1` (read once per process); without it the outputs are
/// not composed and the test says so and passes.
#[test]
fn deferred_outputs_reproduce_the_inline_traced_colour() -> Result<()> {
    if std::env::var("RENDERLAB_RL_DEFERRED").as_deref() != Ok("1") {
        println!("deferred reflections: RENDERLAB_RL_DEFERRED=1 not set, nothing checked");
        return Ok(());
    }
    let path = root("deferred-reflections");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor = [[-8., -8.], [8., -8.], [8., 8.], [-8., 8.]].map(|[x, y]| Vertex {
        position: [x, y, 0.],
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let wall = [[-1.5, 0.], [1.5, 0.], [1.5, 2.], [-1.5, 2.]].map(|[x, z]| Vertex {
        position: [x, 2., z],
        normal: [0., -1., 0.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.3, 0.4, -1.).normalize(),
            lux: [1000.; 3],
        }],
        environment: [200.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![16.],
        disk_angle: 0.02,
    };
    let camera = Camera::look_at(
        Vec3::new(0., -4., 1.6),
        Vec3::new(0., 1., 0.3),
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let half = |bytes: &[u8]| -> Vec<[f32; 4]> {
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32()
                })
            })
            .collect()
    };
    let mut frame = |reflections: Option<ReflectionSettings>| {
        let material = |base: f32, metallic: f32, roughness: f32| MaterialDescriptor {
            base_color: [base, base, base, 1.],
            metallic,
            roughness,
            double_sided: true,
            ..Default::default()
        };
        let objects = [
            ForwardObject {
                vertices: &floor,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: material(0.7, 1., 0.2),
            },
            ForwardObject {
                vertices: &wall,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: material(0.02, 0., 0.6),
            },
        ];
        render(
            &mut gpu,
            &cache,
            &ForwardScene {
                meshes: None,
                materials: None,
                instances: None,
                objects: &objects,
                lighting: &lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&shadows),
                primary: Primary::Traced,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections,
                textures: None,
                local_shadows: None,
                transparent: &[],
                programs: None,
                sky_map: None,
                skins: None,
                colour_lut: None,
                ids: None,
            },
        )
    };
    let traced = frame(Some(ReflectionSettings::default()))?;
    let untraced = frame(None)?;
    let [reflectance, specular] = traced
        .deferred_reflections
        .as_ref()
        .context("deferred outputs were not read back")?;
    let (lit, plain) = (half(&traced.hdr), half(&untraced.hdr));
    let (reflectance, specular) = (half(reflectance), half(specular));
    let (mut checked, mut worst, mut moved) = (0, 0f32, 0f32);
    for i in 0..lit.len() {
        let w = reflectance[i][3];
        if w <= 0.01 {
            continue;
        }
        checked += 1;
        for c in 0..3 {
            let expected = plain[i][c] + w * (reflectance[i][c] * specular[i][3] - specular[i][c]);
            let error = (lit[i][c] - expected).abs() / lit[i][c].abs().max(0.05);
            worst = worst.max(error);
            moved = moved.max((lit[i][c] - plain[i][c]).abs() / plain[i][c].abs().max(0.05));
        }
    }
    println!(
        "deferred reflections: {checked} traced pixels, worst relative error {worst:.4}, \
         largest traced change {moved:.3}"
    );
    ensure!(
        checked > 1000 && moved > 0.05,
        "the fixture traces too little to test: {checked} pixels, largest change {moved}"
    );
    // Five f16-stored values meet in each pixel (two colours, the reflectance, the untraced
    // specular and the traced luminance): about 1% at the darkest pixels.
    ensure!(
        worst < 0.02,
        "the composite misses the inline colour by up to {worst:.4}"
    );
    Ok(())
}
