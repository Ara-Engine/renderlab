//! Meshes kept on the GPU across the frames of a sequence.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows},
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
