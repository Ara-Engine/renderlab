//! What a sequence keeps on the GPU across its frames: named meshes and the
//! atmosphere's tables.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [96, 64];

fn quad(half: f32) -> [Vertex; 4] {
    [[-half, -half], [half, -half], [half, half], [-half, half]].map(|[x, y]| Vertex {
        position: [x, y, 0.],
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    })
}

/// A floor and up to two cards of one geometry, over five frames: the cards move, one
/// leaves, both leave, and one returns.
///
/// Naming each object's mesh (`ForwardScene::meshes`), with both cards under one
/// identity, gives the frames of the unnamed sequence byte for byte. So a mesh built in
/// an earlier frame is the same mesh, two objects can share one structure at different
/// transforms, and an identity no frame names is released and built again when it
/// returns. The device is clean after the sequence is retired. An identity whose
/// vertex or index count changed is refused, which is the one misuse a frame can see.
#[test]
fn named_meshes_stay_on_the_gpu_and_draw_the_same_frames() -> Result<()> {
    let path = root("resident-meshes");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let (floor, card) = (quad(8.), quad(0.4));
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.3, 0.2, -1.).normalize(),
            lux: [4.; 3],
        }],
        environment: [0.2; 3],
        exposure_stops: -2.,
    };
    let shadows = SunShadows {
        split_ends: vec![8.],
        disk_angle: 0.02,
    };
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    // Each frame's card positions; the floor is always there.
    let poses: [&[[f32; 2]]; 5] = [
        &[[-0.8, 0.], [0.8, 0.2]],
        &[[-0.6, 0.1], [0.9, 0.]],
        &[[-0.4, 0.2]],
        &[],
        &[[0.5, -0.3]],
    ];
    let sequence = |named: bool, gpu: &mut GpuDevice| -> Result<Vec<Vec<u8>>> {
        let mut frames = Frames::default();
        let mut images = Vec::new();
        for cards in poses {
            let object = |vertices, base: f32, transform| ForwardObject {
                vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform,
                material: MaterialDescriptor {
                    base_color: [base, base, base, 1.],
                    metallic: 0.,
                    roughness: 0.6,
                    double_sided: true,
                    ..Default::default()
                },
            };
            let mut objects = vec![object(&floor[..], 0.3, Mat4::IDENTITY)];
            objects.extend(cards.iter().map(|&[x, y]| {
                object(&card[..], 0.8, Mat4::from_translation(Vec3::new(x, y, 1.)))
            }));
            let mut keys = vec![1];
            keys.resize(objects.len(), 2);
            let frame = frames.render(
                gpu,
                &cache,
                &ForwardScene {
                    meshes: named.then_some(&keys),
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
                    reflections: None,
                    textures: None,
                    local_shadows: None,
                    transparent: &[],
                    programs: None,
                },
            );
            match frame {
                Ok(frame) => images.push(frame.hdr),
                Err(error) => {
                    frames.retire(gpu)?;
                    return Err(error);
                }
            }
        }
        frames.retire(gpu)?;
        Ok(images)
    };
    let plain = sequence(false, &mut gpu)?;
    let kept = sequence(true, &mut gpu)?;
    for (index, (plain, kept)) in plain.iter().zip(&kept).enumerate() {
        ensure!(
            plain == kept,
            "frame {index} differs with its meshes kept on the GPU"
        );
    }
    // The frames are not all alike: the cards moved and left.
    ensure!(
        plain.windows(2).all(|pair| pair[0] != pair[1]),
        "two consecutive frames are the same picture"
    );
    let big = quad(8.);
    let mut frames = Frames::default();
    let mut frame = |vertices: &[Vertex], indices: &[u32], gpu: &mut GpuDevice| {
        frames.render(
            gpu,
            &cache,
            &ForwardScene {
                meshes: Some(&[7]),
                objects: &[ForwardObject {
                    vertices,
                    indices,
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        double_sided: true,
                        ..Default::default()
                    },
                }],
                lighting: &lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&shadows),
                primary: Primary::Traced,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &[],
                programs: None,
            },
        )
    };
    frame(&big, &[0, 1, 2, 0, 2, 3], &mut gpu)?;
    ensure!(
        frame(&big, &[0, 1, 2], &mut gpu).is_err(),
        "an identity named a smaller geometry than before and was accepted"
    );
    frames.retire(&mut gpu)?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A floor and a wall under the physical sky with its procedural environment, over five
/// frames of one sequence: the same view twice, the camera 40 m higher, a thicker
/// medium, and the sun lower. The sun is behind the wall, so the side the camera sees
/// is lit by the sky's environment alone.
///
/// The sequence keeps the atmosphere's tables and remakes each only when the renderer
/// would: the transmittance and multiple-scattering tables when the medium changes, the
/// environment when its sun, medium or altitude does, the sky view every frame. Every
/// frame is byte for byte the frame a fresh sequence renders of the same scene, which
/// makes every table anew; after the sun turns, all of it but a few penumbra pixels,
/// which follow the shadow planner's carried orientation and not a table. So a kept
/// table is never stale, and the frames differ from one another where they should (all
/// but the repeated view). With the environment remade only once the test fails at the
/// camera's rise, and with the medium's tables remade only once at the thicker medium.
#[test]
fn kept_atmosphere_tables_give_every_frame_what_fresh_tables_give() -> Result<()> {
    let path = root("resident-sky");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor = quad(40.).map(|v| Vertex {
        position: [v.position[0], 0., v.position[1]],
        normal: [0., 1., 0.],
        ..v
    });
    let wall = [[-3., 0.], [3., 0.], [3., 4.], [-3., 4.]].map(|[x, y]| Vertex {
        position: [x, y, -3.],
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let shadows = SunShadows {
        split_ends: vec![40.],
        disk_angle: 0.02,
    };
    let medium = renderlab::AtmosphereMedium::default();
    let thick = renderlab::AtmosphereMedium {
        mie_scattering: medium.mie_scattering.map(|m| m * 4.),
        ..medium
    };
    // Each frame's eye height, medium and sun elevation.
    let poses = [
        (2f32, medium, 0.5f32),
        (2., medium, 0.5),
        (42., medium, 0.5),
        (42., thick, 0.5),
        (42., thick, 0.15),
    ];
    let mut frames = Frames::default();
    let mut images = Vec::new();
    for (index, (height, medium, elevation)) in poses.into_iter().enumerate() {
        let lighting = Lighting {
            lights: vec![Light::Directional {
                travel: -Vec3::new(0.3 * elevation.cos(), elevation.sin(), -elevation.cos())
                    .normalize(),
                lux: [100_000.; 3],
            }],
            environment: [0.; 3],
            exposure_stops: -14.,
        };
        let objects = [
            (&floor[..], &[0u32, 2, 1, 0, 3, 2]),
            (&wall[..], &[0, 1, 2, 0, 2, 3]),
        ]
        .map(|(vertices, indices)| ForwardObject {
            vertices,
            indices,
            transform: Mat4::IDENTITY,
            material: MaterialDescriptor {
                base_color: [0.5, 0.5, 0.5, 1.],
                metallic: 0.,
                roughness: 0.4,
                double_sided: true,
                ..Default::default()
            },
        });
        let scene = ForwardScene {
            meshes: None,
            objects: &objects,
            lighting: &lighting,
            camera: Camera::look_at(
                Vec3::new(0., height, 6.),
                Vec3::new(0., 2., -3.),
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                500.,
            )?,
            size: SIZE,
            atmosphere: renderlab::Atmosphere {
                medium,
                volumetric_fog: None,
                ..Default::default()
            },
            sun_shadows: Some(&shadows),
            primary: Primary::Traced,
            post: None,
            indirect: None,
            reconstruction: None,
            reflections: None,
            textures: None,
            local_shadows: None,
            transparent: &[],
            programs: None,
        };
        let kept = frames.render(&mut gpu, &cache, &scene)?.hdr;
        let fresh = render(&mut gpu, &cache, &scene)?.hdr;
        // The wall's middle, at the image's: sky light alone.
        let at = (SIZE[1] as usize / 2 * SIZE[0] as usize + SIZE[0] as usize / 2) * 8;
        let apart = kept
            .as_chunks::<8>()
            .0
            .iter()
            .zip(fresh.as_chunks::<8>().0)
            .filter(|(a, b)| a != b)
            .count();
        if index < 4 {
            ensure!(
                apart == 0,
                "frame {index} of the sequence differs from the frame fresh tables give at {apart} pixels"
            );
        } else {
            // The sun turned. The shadow planner carries the sun's shadow orientation
            // from frame to frame (`ShadowState::bases`), which a fresh sequence starts
            // anew, so the penumbra of the wall's shadow on the floor may differ: a few
            // pixels. The wall's own side, which only the environment lights, may not.
            ensure!(
                kept[at - 32..at + 40] == fresh[at - 32..at + 40] && apart * 100 < kept.len() / 8,
                "frame {index} differs from the frame fresh tables give at {apart} pixels, or on the wall"
            );
        }
        let lit = f16::from_bits(u16::from_le_bytes([kept[at + 4], kept[at + 5]])).to_f32();
        ensure!(
            lit > 1e-3,
            "frame {index}: the wall's shaded side holds {lit} of blue, with no sky light on it"
        );
        images.push(kept);
    }
    frames.retire(&mut gpu)?;
    ensure!(
        images[0] == images[1] && images[1..].windows(2).all(|pair| pair[0] != pair[1]),
        "the frames do not differ where the camera, the medium or the sun did"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
