//! `LocalShadows::reflection_hits` on the port: off, reflection hits light local lights
//! unshadowed while camera receivers keep their shadows.
use super::*;
use renderlab::{
    Camera, Light, Lighting, LocalShadows, Mat4, MaterialDescriptor, ReflectionSettings, Vec3,
    Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];

/// A spot lights a wall and a small card shadows part of it; a near-mirror floor reflects
/// the wall. With `reflection_hits` off the reflected shadow lifts, so the traced frame
/// changes, while a frame without reflections (camera receivers only) is byte-identical.
#[test]
fn reflection_hits_can_light_local_lights_unshadowed() -> Result<()> {
    let path = root("hit-local-shadows");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let quad = |corners: [[f32; 3]; 4], normal: [f32; 3]| {
        corners.map(|position| Vertex {
            position,
            normal,
            uv: [0.; 2],
            color: [1.; 4],
        })
    };
    let floor = quad(
        [[-8., -8., 0.], [8., -8., 0.], [8., 8., 0.], [-8., 8., 0.]],
        [0., 0., 1.],
    );
    let wall = quad(
        [[-1.5, 2., 0.], [1.5, 2., 0.], [1.5, 2., 2.], [-1.5, 2., 2.]],
        [0., -1., 0.],
    );
    let card = quad(
        [
            [-0.2, 1.4, 0.8],
            [0.2, 1.4, 0.8],
            [0.2, 1.4, 1.2],
            [-0.2, 1.4, 1.2],
        ],
        [0., -1., 0.],
    );
    let lighting = Lighting {
        // A traced frame shades with a sun: a dim one, so the spot carries the picture.
        lights: vec![
            Light::Directional {
                travel: Vec3::new(0.3, 0.4, -1.).normalize(),
                lux: [1.; 3],
            },
            Light::Spot {
                position: Vec3::new(0., 0.4, 1.0),
                travel: Vec3::Y,
                candela: [3000.; 3],
                range: None,
                inner: 0.5,
                outer: 0.9,
                source_radius: 0.,
                cast_shadows: Some(true),
            },
        ],
        environment: [20.; 3],
        exposure_stops: -6.,
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
    let mut frame = |reflections: Option<ReflectionSettings>, hits: bool| {
        let material = |base: f32, metallic: f32, roughness: f32| MaterialDescriptor {
            base_color: [base, base, base, 1.],
            metallic,
            roughness,
            double_sided: true,
            ..Default::default()
        };
        let objects = [
            (&floor, material(0.9, 1., 0.05)),
            (&wall, material(0.8, 0., 0.6)),
            (&card, material(0.8, 0., 0.6)),
        ]
        .map(|(vertices, material)| ForwardObject {
            vertices,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material,
        });
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
                local_shadows: Some(LocalShadows {
                    reflection_hits: hits,
                    ..Default::default()
                }),
                transparent: &[],
                programs: None,
                sky_map: None,
                skins: None,
                colour_lut: None,
                ids: None,
            },
        )
    };
    let shadowed = frame(Some(ReflectionSettings::default()), true)?.hdr;
    let unshadowed = frame(Some(ReflectionSettings::default()), false)?.hdr;
    let changed = shadowed
        .as_chunks::<8>()
        .0
        .iter()
        .zip(unshadowed.as_chunks::<8>().0)
        .filter(|(a, b)| a != b)
        .count();
    println!("reflection hits unshadowed: {changed} pixels changed");
    ensure!(
        changed > 20,
        "reflection hits kept the reflected spot shadow ({changed} pixels changed)"
    );
    ensure!(
        frame(None, true)?.hdr == frame(None, false)?.hdr,
        "camera receivers changed with `reflection_hits`"
    );
    Ok(())
}
