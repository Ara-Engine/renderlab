//! `LocalShadows::unshadowed_hits_from` on the port: reflection hits from rough enough lobes
//! light local lights unshadowed, near-mirror ones keep their shadows, and camera receivers
//! keep every shadow.
use super::*;
use renderlab::{
    Camera, Light, Lighting, LocalShadows, Mat4, MaterialDescriptor, ReflectionSettings, Vec3,
    Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];

/// A spot lights a wall and a small card shadows part of it; a floor reflects the wall.
/// Dropping hit shadows everywhere (0) lifts the reflected shadow; at the default a
/// near-mirror floor (roughness 0.05) keeps it, as if never dropped, while a glossy floor
/// as rough as the default threshold drops it. A frame without reflections is byte-identical whatever the setting.
#[test]
fn reflection_hits_drop_local_shadows_from_rough_lobes_only() -> Result<()> {
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
    // (reflections, unshadowed_hits_from, floor roughness)
    let mut frame = |reflections: Option<ReflectionSettings>, from: f32, floor_rough: f32| {
        let material = |base: f32, metallic: f32, roughness: f32| MaterialDescriptor {
            base_color: [base, base, base, 1.],
            metallic,
            roughness,
            double_sided: true,
            ..Default::default()
        };
        let objects = [
            (&floor, material(0.9, 1., floor_rough)),
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
                    unshadowed_hits_from: from,
                    ..Default::default()
                }),
                transparent: &[],
                programs: None,
                sky_map: None,
                skins: None,
                casts: None,
                colour_lut: None,
                ids: None,
            },
        )
    };
    let changed = |a: &[u8], b: &[u8]| {
        a.as_chunks::<8>()
            .0
            .iter()
            .zip(b.as_chunks::<8>().0)
            .filter(|(a, b)| a != b)
            .count()
    };
    const NEVER: f32 = 2.0;
    let traced = Some(ReflectionSettings::default());
    let from = LocalShadows::default().unshadowed_hits_from;
    let mirror_never = frame(traced, NEVER, 0.05)?.hdr;
    let mirror_all = changed(&mirror_never, &frame(traced, 0.0, 0.05)?.hdr);
    let mirror_default = changed(&mirror_never, &frame(traced, from, 0.05)?.hdr);
    let glossy_default = changed(
        &frame(traced, NEVER, from)?.hdr,
        &frame(traced, from, from)?.hdr,
    );
    println!(
        "hit local shadows: mirror floor {mirror_all} px change dropped everywhere, \
         {mirror_default} at the default; glossy floor {glossy_default} at the default"
    );
    ensure!(
        mirror_all > 20,
        "dropping every hit's local shadows kept the reflected spot shadow ({mirror_all} px)"
    );
    ensure!(
        mirror_default == 0,
        "a near-mirror lobe lost its hit shadows at the default ({mirror_default} px)"
    );
    ensure!(
        glossy_default > 20,
        "a glossy lobe kept its hit shadows at the default ({glossy_default} px)"
    );
    ensure!(
        frame(None, NEVER, 0.05)?.hdr == frame(None, 0.0, 0.05)?.hdr,
        "camera receivers changed with `unshadowed_hits_from`"
    );
    Ok(())
}
