//! Skinned meshes through the backend's engine-level entry.
use super::*;
use renderlab::{
    Camera, LocalShadows, Mat4, MeshData, ReflectionSettings, Vec3, Vertex,
    asset::{
        Mip, Primitive, Sampler, TangentBasis, TangentStream, Texture as TextureData, TextureRole,
        skin::Influence,
    },
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, Settings, Sky, Sun, TextureRef, ViewDesc, ViewLook},
};

const SIZE: [u32; 2] = [96, 64];

/// A 4 m quad facing +Y over two joints: its near edge on joint 0, its far edge blended
/// 0.3 / 0.7 toward joint 1, so a pose bends it.
fn quad() -> (Primitive, Vec<Influence>) {
    let vertices = [[-2., -2.], [2., -2.], [2., 2.], [-2., 2.]].map(|[x, z]: [f32; 2]| Vertex {
        position: [x, 0., z],
        normal: [0., 1., 0.],
        uv: [x, z],
        color: [1.; 4],
    });
    let near = Influence {
        joints: [0, 0, 0, 0],
        weights: [1., 0., 0., 0.],
    };
    let far = Influence {
        joints: [0, 1, 0, 0],
        weights: [0.3, 0.7, 0., 0.],
    };
    (
        Primitive {
            vertices: vertices.to_vec(),
            tangents: Some(TangentStream {
                basis: TangentBasis::Authored,
                values: vec![[1., 0., 0., 1.]; 4],
            }),
            // Counter-clockwise seen from +Y, as the normals face.
            indices: vec![0, 2, 1, 0, 3, 2],
            material: 0,
        },
        vec![near.clone(), near, far.clone(), far],
    )
}

/// SK1's `skin_vertices` for one vertex, in the kernel's order of operations: what the
/// pose must put on the GPU.
fn posed(v: &Vertex, influence: &Influence, palette: &[Mat4], t: [f32; 4]) -> (Vertex, [f32; 4]) {
    let (j, w) = (influence.joints.map(usize::from), influence.weights);
    let m =
        palette[j[0]] * w[0] + palette[j[1]] * w[1] + palette[j[2]] * w[2] + palette[j[3]] * w[3];
    let p = (m * Vec3::from_array(v.position).extend(1.)).truncate();
    let (a, b, c) = (
        m.x_axis.truncate(),
        m.y_axis.truncate(),
        m.z_axis.truncate(),
    );
    let n0 = Vec3::from_array(v.normal);
    let sign = if a.dot(b.cross(c)) >= 0. { 1. } else { -1. };
    let unit = |v: Vec3, fallback: Vec3| {
        let l = v.dot(v);
        if l > 1e-20 {
            v * (1. / l.sqrt())
        } else {
            fallback
        }
    };
    let n = unit(
        (b.cross(c) * n0.x + c.cross(a) * n0.y + a.cross(b) * n0.z) * sign,
        n0,
    );
    let q = a * t[0] + b * t[1] + c * t[2];
    let u = unit(q - n * q.dot(n), Vec3::new(t[0], t[1], t[2]));
    (
        Vertex {
            position: p.to_array(),
            normal: n.to_array(),
            ..*v
        },
        [u.x, u.y, u.z, t[3] * sign],
    )
}

/// The quad, normal-mapped, posed by a palette that turns and lifts joint 1, under the
/// sun: as a skinned mesh, and as a static mesh created from the vertices and tangents
/// SK1 would pose. With direct light and no history the two frames are the same bytes:
/// the pose reaches the frame, normals through the cofactor and the tangents with them,
/// and a skinned draw (geometry built for its pose, its batch word marked skinned)
/// shades as a rigid one. Nothing is refused. Then, with the radiance cache and the
/// reconstruction on, eight frames of a moving pose are finite and lit.
#[test]
fn a_posed_skinned_mesh_draws_as_its_posed_geometry() -> Result<()> {
    let path = root("engine-skinning");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let (primitive, influences) = quad();
    let palette = |time: f32| {
        [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0., 0.2 + 0.1 * time, 0.))
                * Mat4::from_rotation_x(0.3 + 0.05 * time),
        ]
    };
    let camera = Camera::look_at(
        Vec3::new(0., 4., -3.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    // A one-texel normal map tilted along the tangent and the bitangent.
    let map = || TextureData {
        role: TextureRole::NormalLinear,
        mips: vec![Mip {
            width: 1,
            height: 1,
            rgba8: vec![164, 164, 245, 255],
        }],
    };
    let settings = |history: bool| Settings {
        global_illumination: history,
        temporal_reconstruction: history,
        local_shadows: LocalShadows {
            count: 0,
            ..Default::default()
        },
        volumetric_fog: None,
        ..Default::default()
    };
    let mut frames = |skinned: bool, history: bool, poses: u32| -> Result<Vec<f32>> {
        let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, settings(history))?;
        let view = engine.create_view(&ViewDesc {
            look: ViewLook {
                exposure: Exposure::Manual { stops: -11. },
                reflections: ReflectionSettings::off(),
                ..Default::default()
            },
            ..ViewDesc::new(SIZE[0], SIZE[1])
        })?;
        engine.set_sky(Sky::Uniform([1000., 1200., 1500.]))?;
        engine.set_sun(Some(Sun {
            travel: Vec3::new(0.3, -1., 0.5).normalize(),
            illuminance: [50_000.; 3],
            angular_radius: 0.02,
        }))?;
        let mesh = if skinned {
            engine.create_skinned_mesh(&primitive, &influences, 2)?
        } else {
            let pose = palette(0.);
            let tangents = primitive.tangents.as_ref().context("tangents")?;
            let (vertices, values): (Vec<_>, Vec<_>) = primitive
                .vertices
                .iter()
                .zip(&influences)
                .zip(&tangents.values)
                .map(|((v, i), t)| posed(v, i, &pose, *t))
                .unzip();
            engine.create_mesh_with_tangents(
                &MeshData {
                    vertices: &vertices,
                    indices: &primitive.indices,
                },
                &TangentStream {
                    basis: TangentBasis::Authored,
                    values,
                },
            )?
        };
        let normal = TextureRef {
            texture: engine.create_texture(map())?,
            sampler: Sampler::default(),
        };
        let material = engine.create_material(&MaterialDesc {
            base_color: [0.6, 0.5, 0.4, 1.],
            roughness: 0.5,
            double_sided: true,
            normal_texture: Some(normal),
            ..Default::default()
        })?;
        engine.spawn(mesh, material, Mat4::IDENTITY)?;
        for pose in 0..poses {
            if skinned {
                engine.set_joint_palette(mesh, &palette(pose as f32))?;
            }
            engine.render_view(&mut gpu, view, &camera)?;
        }
        let hdr: Vec<f32> = engine
            .images(view)
            .context("no frame")?
            .hdr
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| f16::from_bits(u16::from_le_bytes(*p)).to_f32())
            .collect();
        let refusals = engine.refusals().to_vec();
        engine.retire(&mut gpu)?;
        ensure!(refusals.is_empty(), "refused: {refusals:?}");
        Ok(hdr)
    };
    let skinned = frames(true, false, 1)?;
    let fixed = frames(false, false, 1)?;
    let differing = skinned.iter().zip(&fixed).filter(|(a, b)| a != b).count();
    ensure!(
        differing == 0,
        "{differing} values differ between the skinned pose and its static geometry"
    );
    let moving = frames(true, true, 8)?;
    ensure!(
        moving.iter().all(|v| v.is_finite()) && moving.iter().any(|v| *v > 0.1),
        "a moving pose under the cache and the history left a nonfinite or unlit frame"
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
