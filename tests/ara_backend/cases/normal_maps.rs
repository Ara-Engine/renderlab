//! Normal maps on the traced frame, against the same floor tilted for real.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    asset::{Mip, Sampler, TangentBasis, TangentStream, Texture, TextureRole},
    backend_ara::forward::{
        ForwardObject, ForwardScene, NormalMap, ObjectTexture, ObjectTextures, Primary,
        SceneTextures, SunShadows, render,
    },
};

const SIZE: [u32; 2] = [96, 64];

/// A one-texel normal map: `normal` in tangent space, as an importer would encode it.
fn constant_map(normal: [f32; 3]) -> Texture {
    Texture {
        role: TextureRole::NormalLinear,
        mips: vec![Mip {
            width: 1,
            height: 1,
            rgba8: normal
                .map(|v| ((v * 0.5 + 0.5) * 255.).round() as u8)
                .into_iter()
                .chain([255])
                .collect(),
        }],
    }
}
/// What the shader decodes from that texel at `scale`, before it is turned into the
/// surface's frame.
fn decoded(map: &Texture, scale: f32) -> Vec3 {
    let texel = &map.mips[0].rgba8;
    let v = Vec3::from_array(std::array::from_fn(|i| {
        f32::from(texel[i]) / 255. * 2. - 1.
    }));
    Vec3::new(v.x * scale, v.y * scale, v.z).normalize()
}

/// A flat floor under a slanted sun and a black sky, shaded with a normal map of one
/// texel: every pixel's shading normal is the same world vector, the texel's normal
/// carried through the vertex tangent, its bitangent `handedness * cross(n, t)` and
/// the surface normal. The same floor without a map, turned about the origin until
/// its real normal is that vector, has the same normal, the same view ray at every
/// pixel and the same sun, so it must be the same image.
///
/// Held for a tilt along the tangent, a tilt along the bitangent at either handedness
/// (which mirror each other), a tangent stream along y instead of x, and a normal
/// scale of one half. Each mapped image is within half a percent of its tilted floor
/// at every pixel and at least two percent from the flat floor on average, and the
/// two handednesses differ from each other. The mesh pass refuses a normal map, and a
/// tangent that is not perpendicular to its normal is refused.
#[test]
fn a_normal_mapped_floor_is_the_floor_tilted_to_that_normal() -> Result<()> {
    let path = root("normal-maps");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor: Vec<Vertex> = [[-20., -20.], [20., -20.], [20., 20.], [-20., 20.]]
        .into_iter()
        .map(|[x, y]: [f32; 2]| Vertex {
            position: [x, y, 0.],
            normal: [0., 0., 1.],
            uv: [x, y],
            color: [1.; 4],
        })
        .collect();
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: -Vec3::new(0.3, -0.2, 1.).normalize(),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![40.],
        disk_angle: 0.02,
    };
    let camera = Camera::look_at(
        Vec3::new(0., -2., 4.),
        Vec3::ZERO,
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let mut frame = |transform: Mat4,
                     scale: f32,
                     primary: Primary,
                     normal: Option<NormalMap<'_>>|
     -> Result<Vec<f32>> {
        let objects = [ForwardObject {
            vertices: &floor,
            indices: &[0, 1, 2, 0, 2, 3],
            transform,
            material: MaterialDescriptor {
                base_color: [0.5, 0.5, 0.5, 1.],
                metallic: 0.,
                roughness: 0.6,
                double_sided: true,
                normal_scale: scale,
                ..Default::default()
            },
        }];
        let slots = [ObjectTextures {
            normal,
            ..Default::default()
        }];
        let images = render(
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
                primary,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: normal.is_some().then_some(SceneTextures {
                    objects: &slots,
                    anisotropy_clamp: 16,
                }),
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
        Ok(images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32())
            .collect())
    };
    let flat = frame(Mat4::IDENTITY, 1., Primary::Traced, None)?;
    let stream = |tangent: [f32; 3], handedness: f32| TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[tangent[0], tangent[1], tangent[2], handedness]; 4],
    };
    let (along_x, along_y) = ([1., 0., 0.], [0., 1., 0.]);
    let mut images = Vec::new();
    let mut report = String::new();
    for (name, texel, tangent, handedness, scale) in [
        ("along the tangent", [0.28, 0., 0.96], along_x, 1., 1.),
        ("along the bitangent", [0., 0.28, 0.96], along_x, 1., 1.),
        (
            "along the mirrored bitangent",
            [0., 0.28, 0.96],
            along_x,
            -1.,
            1.,
        ),
        (
            "with the tangent along y",
            [0.28, 0., 0.96],
            along_y,
            1.,
            1.,
        ),
        ("at half scale", [0.6, 0., 0.8], along_x, 1., 0.5),
    ] {
        let map = constant_map(texel);
        let tangents = stream(tangent, handedness);
        let mapped = frame(
            Mat4::IDENTITY,
            scale,
            Primary::Traced,
            Some(NormalMap {
                texture: ObjectTexture {
                    data: (&map).into(),
                    sampler: Sampler::default(),
                    key: None,
                },
                tangents: &tangents,
            }),
        )?;
        // `material_normal`: t x + b y + n z, with b = handedness * cross(n, t).
        let (n, t) = (Vec3::Z, Vec3::from_array(tangent));
        let d = decoded(&map, scale);
        let world = (t * d.x + handedness * n.cross(t) * d.y + n * d.z).normalize();
        let tilted = frame(
            Mat4::from_quat(glam::Quat::from_rotation_arc(Vec3::Z, world)),
            1.,
            Primary::Traced,
            None,
        )?;
        let mut worst = 0f32;
        for (i, (a, b)) in mapped.iter().zip(&tilted).enumerate() {
            ensure!(
                *b > 0.01,
                "{name}: the tilted floor is dark or missing at pixel {i}"
            );
            worst = worst.max((a - b).abs() / b);
        }
        let apart = mapped
            .iter()
            .zip(&flat)
            .map(|(a, b)| (a - b).abs() / b)
            .sum::<f32>()
            / mapped.len() as f32;
        ensure!(
            worst < 5e-3 && apart > 0.02,
            "tilt {name}: {worst:.4} from the tilted floor at worst, {apart:.4} from the flat floor on average"
        );
        report += &format!("{name}: {worst:.5} worst, {apart:.3} from flat; ");
        images.push(mapped);
    }
    let mirrored = images[1]
        .iter()
        .zip(&images[2])
        .map(|(a, b)| (a - b).abs() / b)
        .sum::<f32>()
        / images[1].len() as f32;
    ensure!(
        mirrored > 0.02,
        "the two handednesses give the same image ({mirrored:.4} apart)"
    );
    let map = constant_map([0.28, 0., 0.96]);
    let tangents = stream(along_x, 1.);
    let normal = |tangents| {
        Some(NormalMap {
            texture: ObjectTexture {
                data: (&map).into(),
                sampler: Sampler::default(),
                key: None,
            },
            tangents,
        })
    };
    ensure!(
        frame(Mat4::IDENTITY, 1., Primary::Raster, normal(&tangents)).is_err(),
        "the mesh pass accepted a normal map"
    );
    let skewed = stream([0.6, 0., 0.8], 1.);
    ensure!(
        frame(Mat4::IDENTITY, 1., Primary::Traced, normal(&skewed)).is_err(),
        "a tangent along its own normal was accepted"
    );
    eprintln!("normal maps: {report}handedness {mirrored:.3} apart");
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
