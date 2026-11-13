//! The material pass on the port (RENDERLAB_MATERIAL_PASS): cold compile times of the standard
//! program's single shading kernel against the material pass's record writer and the
//! record-reading shading, tiles and list. Measurement, ignored by default.
use super::*;
use renderlab::backend_ara::primary_shade;

#[test]
#[ignore]
fn material_pass_compile_seconds() -> Result<()> {
    let path = root("material-pass");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let standard = renderlab::STANDARD_PROGRAM;
    for list in [false, true] {
        let full = if list {
            primary_shade::hardware_soft_list_program(standard)?
        } else {
            primary_shade::hardware_soft_program(standard)?
        };
        let writer = primary_shade::hardware_soft_material_writer(standard, list)?
            .context("the standard program declares the record mapping")?;
        let reader = primary_shade::hardware_soft_material_reader(standard, list)?
            .context("the standard program declares the record mapping")?;
        for (name, program) in [
            ("single kernel", full),
            ("material writer", writer),
            ("record reader", reader),
        ] {
            let started = std::time::Instant::now();
            renderlab::backend_ara::kernel::Kernel::from_program(&mut gpu, &cache, &program)?;
            println!(
                "material pass compile: list={list} {name}: {:.2} s",
                started.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}

/// The record's receiver (words 1 and 2) against the sun path's receiver for a fresh camera
/// ray (its edge repair included), through `RENDERLAB_MATERIAL_DEBUG=receiver` (program.rs
/// `material_debug`): the trace point, slot, `thin`, both sides and the epsilon come back bit
/// for bit, at repaired edges too; the geometric normal within its octahedral 2 x 16 bits and
/// the interpolated vertex normal (against this pixel's decode) within 2 x 12. A floor, a wall
/// folded along a crease the middle column's rays land on, and a far wall whose epsilon is above the 1e-5 floor. Runs with
/// `RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_DEBUG=receiver` (read once per process);
/// without them it says so and passes.
#[test]
fn the_record_rebuilds_the_receiver() -> Result<()> {
    use renderlab::{
        Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
        backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
    };
    let set = |name: &str, value: &str| std::env::var(name).as_deref() == Ok(value);
    if !(set("RENDERLAB_MATERIAL_PASS", "1") && set("RENDERLAB_MATERIAL_DEBUG", "receiver")) {
        println!(
            "record receiver: RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_DEBUG=receiver not set, nothing checked"
        );
        return Ok(());
    }
    // Odd width: the middle column's camera ray lies in the plane x = 0, the crease's.
    const SIZE: [u32; 2] = [161, 121];
    let path = root("material-receiver");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let quad = |corners: [[f32; 3]; 4], normals: [[f32; 3]; 4]| -> Vec<Vertex> {
        (0..4)
            .map(|i| Vertex {
                position: corners[i],
                normal: normals[i],
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect()
    };
    let up = [0., 0., 1.];
    let floor = quad(
        [[-8., -8., 0.], [8., -8., 0.], [8., 8., 0.], [-8., 8., 0.]],
        [up; 4],
    );
    let toward = |x: f32, z: f32| Vec3::new(x, -1., z).normalize().to_array();
    // A wall folded along x = 0 (a concave crease, where the edge repair acts; both faces
    // share the edge's vertices), with tilted vertex normals.
    let crease: Vec<Vertex> = [
        ([0., 2.6, 0.], toward(0., -0.2)),
        ([0., 2.6, 2.], toward(0., 0.5)),
        ([-1.5, 2., 0.], toward(0.6, -0.2)),
        ([-1.5, 2., 2.], toward(0.5, 0.4)),
        ([1.5, 2., 0.], toward(-0.6, -0.2)),
        ([1.5, 2., 2.], toward(-0.5, 0.4)),
    ]
    .map(|(position, normal)| Vertex {
        position,
        normal,
        uv: [0.; 2],
        color: [1.; 4],
    })
    .to_vec();
    let far = quad(
        [
            [-40., 60., 0.],
            [40., 60., 0.],
            [40., 60., 30.],
            [-40., 60., 30.],
        ],
        [[0., -1., 0.]; 4],
    );
    let quad_indices: &[u32] = &[0, 1, 2, 0, 2, 3];
    let crease_indices: &[u32] = &[2, 0, 1, 2, 1, 3, 0, 4, 5, 0, 5, 1];
    let objects = [
        (&floor, quad_indices),
        (&crease, crease_indices),
        (&far, quad_indices),
    ]
    .map(|(vertices, indices)| ForwardObject {
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        material: MaterialDescriptor {
            base_color: [0.7, 0.7, 0.7, 1.],
            roughness: 0.5,
            double_sided: true,
            ..Default::default()
        },
    });
    let images = render(
        &mut gpu,
        &cache,
        &ForwardScene {
            meshes: None,
            materials: None,
            instances: None,
            objects: &objects,
            // Misses write black, so only hits carry errors.
            lighting: &Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(0.3, 0.4, -1.).normalize(),
                    lux: [1.; 3],
                }],
                environment: [0.; 3],
                exposure_stops: 0.,
            },
            camera: Camera::look_at(
                Vec3::new(0., -4., 1.6),
                Vec3::new(0., 1., 0.6),
                Vec3::Z,
                70f32.to_radians(),
                0.1,
                200.,
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
    // Hits write minus (1e-6 plus) the vertex normal's angle in w; misses write alpha 0 or 1.
    let (mut worst, mut hits, mut edges) = ([0f32; 4], 0, 0);
    for pixel in images.hdr.as_chunks::<8>().0 {
        let value = |c: usize| {
            f16::from_bits(u16::from_le_bytes([pixel[2 * c], pixel[2 * c + 1]])).to_f32()
        };
        if value(3) >= 0.0 {
            continue;
        }
        hits += 1;
        // z is negative (minus 1e-6) at repaired edges, where the record's point is the
        // repair's rather than shading's barycentric one.
        edges += usize::from(value(2) < 0.0);
        for (c, worst) in worst.iter_mut().enumerate() {
            *worst = worst.max(if c >= 2 && value(c) < 0.0 {
                -value(c) - 1e-6
            } else {
                value(c)
            });
        }
    }
    println!(
        "record receiver: {hits} hits, {edges} at repaired edges; trace point {:e}, epsilon {:e}, geometric normal {:.2e} rad, vertex normal {:.2e} rad ({:.3} deg)",
        worst[0],
        worst[1],
        worst[2],
        worst[3],
        worst[3].to_degrees()
    );
    ensure!(
        hits > (SIZE[0] * SIZE[1] / 2) as usize,
        "too few hits ({hits})"
    );
    ensure!(edges > 0, "no pixel reached a repaired edge");
    ensure!(
        worst[0] == 0.0,
        "the trace point, slot, thin or a side moved ({})",
        worst[0]
    );
    ensure!(worst[1] == 0.0, "the epsilon moved ({})", worst[1]);
    ensure!(
        worst[2] < 2e-4,
        "the geometric normal moved {} rad",
        worst[2]
    );
    ensure!(worst[3] < 2e-3, "the vertex normal moved {} rad", worst[3]);
    Ok(())
}
