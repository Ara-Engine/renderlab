//! The local-light visibility pass on the port (`RENDERLAB_LIGHT_VIS_PASS=1`,
//! src/backend_ara/light_vis.rs): a floor, a box and three shadowed point lights, drawn with the
//! pass off, with it decoding the hit, and with it reading the material record's receiver
//! layers (`RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_PROBE=tracetail`). The pass changes no
//! more than its 8-bit visibility quantization (soft shadows: the lights have a radius), and
//! shading reads its output only when it is on. The switches are read once per process, so each setting is a child run.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [160, 96];

/// The frame's HDR and whether shading reads the light visibility pass's output.
fn frame() -> Result<(Vec<u8>, bool)> {
    let path = root("light-vis");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let quad = |corners: [[f32; 3]; 4], normal: [f32; 3]| -> Vec<Vertex> {
        corners
            .map(|position| Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            })
            .to_vec()
    };
    let floor = quad(
        [[-6., -6., 0.], [6., -6., 0.], [6., 6., 0.], [-6., 6., 0.]],
        [0., 0., 1.],
    );
    let wall = quad(
        [[-1., 0., 0.], [1., 0., 0.], [1., 0., 1.5], [-1., 0., 1.5]],
        [0., -1., 0.],
    );
    let objects = [&floor, &wall].map(|vertices| ForwardObject {
        vertices,
        indices: &[0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        material: MaterialDescriptor {
            base_color: [0.7, 0.7, 0.7, 1.],
            roughness: 0.6,
            double_sided: true,
            ..Default::default()
        },
    });
    let point = |x: f32, y: f32| Light::Point {
        position: Vec3::new(x, y, 1.2),
        candela: [300.; 3],
        range: None,
        source_radius: 0.3,
        cast_shadows: Some(true),
    };
    let images = render(
        &mut gpu,
        &cache,
        &ForwardScene {
            meshes: None,
            materials: None,
            instances: None,
            objects: &objects,
            lighting: &Lighting {
                lights: vec![
                    Light::Directional {
                        travel: Vec3::new(0.3, 0.4, -1.).normalize(),
                        lux: [1.; 3],
                    },
                    point(-1.5, 1.0),
                    point(1.5, 1.0),
                    point(0.0, 2.0),
                ],
                environment: [0.5; 3],
                exposure_stops: -4.,
            },
            camera: Camera::look_at(
                Vec3::new(0., -5., 4.),
                Vec3::new(0., 1., 0.),
                Vec3::Z,
                60f32.to_radians(),
                0.1,
                100.,
            )?,
            size: SIZE,
            atmosphere: renderlab::Atmosphere::off(),
            sun_shadows: Some(&SunShadows {
                split_ends: vec![32.],
                disk_angle: 0.02,
            }),
            primary: Primary::Traced,
            post: None,
            indirect: None,
            reconstruction: None,
            reflections: None,
            textures: None,
            local_shadows: None,
            transparent: &[],
            programs: None,
            sky_map: None,
            skins: None,
            casts: None,
            colour_lut: None,
            ids: None,
        },
    )?;
    // Shading reads the pass's binding 73 only when the pass is on; reading it unwritten would
    // show in the picture, so a matching picture with the read means the pass ran.
    let ran =
        renderlab::backend_ara::primary_shade::hardware_soft_program(renderlab::STANDARD_PROGRAM)?
            .inputs()
            .iter()
            .any(|i| i.key.group == 0 && i.key.binding == 73);
    Ok((images.hdr, ran))
}

#[test]
fn the_light_visibility_pass_matches_inline_shadows() -> Result<()> {
    const NAME: &str = "light_vis_cases::the_light_visibility_pass_matches_inline_shadows";
    // In a child run: write the HDR and whether the pass ran, then stop.
    if let Ok(dump) = std::env::var("RL_LIGHT_VIS_DUMP") {
        let (hdr, ran) = frame()?;
        std::fs::write(dump, [hdr, vec![u8::from(ran)]].concat())?;
        return Ok(());
    }
    let dir = root("light-vis-runs");
    std::fs::create_dir_all(&dir)?;
    let run = |name: &str, envs: &[(&str, &str)]| -> Result<(Vec<u8>, bool)> {
        let dump = dir.join(format!("{name}.bin"));
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .args([NAME, "--exact", "--test-threads=1"])
            .env("RL_LIGHT_VIS_DUMP", &dump)
            .env_remove("RENDERLAB_LIGHT_VIS_PASS")
            .env_remove("RENDERLAB_MATERIAL_PASS")
            .env_remove("RENDERLAB_MATERIAL_PROBE");
        for (key, value) in envs {
            command.env(key, value);
        }
        ensure!(command.status()?.success(), "the {name} run failed");
        let mut bytes = std::fs::read(&dump)?;
        let ran = bytes.pop() == Some(1);
        Ok((bytes, ran))
    };
    let (off, off_ran) = run("off", &[])?;
    let (decoding, decoding_ran) = run("decoding", &[("RENDERLAB_LIGHT_VIS_PASS", "1")])?;
    let (record, record_ran) = run(
        "record",
        &[
            ("RENDERLAB_LIGHT_VIS_PASS", "1"),
            ("RENDERLAB_MATERIAL_PASS", "1"),
            ("RENDERLAB_MATERIAL_PROBE", "tracetail"),
        ],
    )?;
    // The largest relative difference per pixel, against the frame without the pass.
    let worst = |frame: &[u8]| -> f32 {
        off.as_chunks::<2>()
            .0
            .iter()
            .zip(frame.as_chunks::<2>().0)
            .map(|(a, b)| {
                let (a, b) = (
                    f16::from_bits(u16::from_le_bytes(*a)).to_f32(),
                    f16::from_bits(u16::from_le_bytes(*b)).to_f32(),
                );
                (a - b).abs() / a.abs().max(b.abs()).max(1e-3)
            })
            .fold(0f32, f32::max)
    };
    let (worst_decoding, worst_record) = (worst(&decoding), worst(&record));
    println!(
        "light visibility pass: ran off {off_ran}, decoding {decoding_ran}, record {record_ran}; worst relative difference {worst_decoding:.4} decoding, {worst_record:.4} record"
    );
    ensure!(
        !off_ran && decoding_ran && record_ran,
        "shading reads the pass only when it is on"
    );
    // Visibility is stored as 8 bits: a light's share moves by at most half a step (1/510).
    ensure!(
        worst_decoding < 0.01 && worst_record < 0.01,
        "the pass changed the picture"
    );
    Ok(())
}
