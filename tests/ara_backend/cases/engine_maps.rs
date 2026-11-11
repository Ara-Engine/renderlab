//! Normal maps and sky maps through the backend's engine-level entry, each held against
//! the same scene that must give the same image without it.
use super::*;
use renderlab::{
    Camera, LocalShadows, Mat4, MeshData, ReflectionSettings, Vec3, Vertex,
    asset::{
        Mip, Sampler, TangentBasis, TangentStream, Texture as TextureData, TextureRole,
        environment::{
            CubeFace, CubeLevel, FieldGenerator, Limits, Metadata, PreparedEnvironment,
            RadianceUnits, roughness_knots,
        },
    },
    backend_ara::engine::Engine,
    engine::{
        Background, Exposure, MaterialDesc, Quality, RenderScale, Settings, Sky, Sun, TextureRef,
    },
};

const SIZE: [u32; 2] = [96, 64];
const STOPS: f32 = -11.;
/// Wound counter-clockwise seen from +Y, as its normals face.
const INDICES: [u32; 6] = [0, 2, 1, 0, 3, 2];

/// Direct light only unless `global_illumination`: no history, reflections, fog or
/// local-light shadows, at a fixed exposure.
fn settings(global_illumination: bool) -> Settings {
    Settings {
        global_illumination,
        temporal_reconstruction: false,
        render_scale: RenderScale::Native,
        reflections: ReflectionSettings::off(),
        local_shadows: LocalShadows {
            count: 0,
            ..Default::default()
        },
        exposure: Exposure::Manual { stops: STOPS },
        // Not on this layer.
        volumetric_fog: None,
        ..Default::default()
    }
}
/// A 40 m floor through the origin, facing +Y, its uv its xz.
fn floor() -> Vec<Vertex> {
    [[-20., -20.], [20., -20.], [20., 20.], [-20., 20.]]
        .into_iter()
        .map(|[x, z]: [f32; 2]| Vertex {
            position: [x, 0., z],
            normal: [0., 1., 0.],
            uv: [x, z],
            color: [1.; 4],
        })
        .collect()
}
fn sun() -> Sun {
    Sun {
        travel: Vec3::new(0.3, -1., 0.5).normalize(),
        illuminance: [50_000.; 3],
        angular_radius: 0.02,
    }
}
/// The last frame's exposed HDR.
fn hdr(engine: &Engine) -> Result<Vec<[f32; 3]>> {
    Ok(engine
        .images()
        .context("no frame")?
        .hdr
        .as_chunks::<8>()
        .0
        .iter()
        .map(|p| {
            std::array::from_fn(|c| {
                f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32()
            })
        })
        .collect())
}
/// The greatest per-channel difference relative to `b`, over pixels where `b` is lit.
fn worst(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .filter(|(_, b)| **b > 0.01)
        .map(|(a, b)| (a - b).abs() / b)
        .fold(0., f32::max)
}

/// A one-texel normal map: `normal` in tangent space, as an importer encodes it.
fn constant_map(normal: [f32; 3]) -> TextureData {
    TextureData {
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

/// The floor under a slanted sun and a black sky, built with engine calls: a mesh with
/// a tangent stream and a material with a one-texel normal map shade every pixel with
/// one world normal, so the floor without a map, turned until its real normal is that
/// one, gives the same image (the frame-level test's argument, here through
/// `create_mesh_with_tangents` and `MaterialDesc::normal_texture`). Within half a percent
/// at every pixel and nothing refused; at least two percent from the flat floor on
/// average. A map on a mesh without tangents is an error from `render`, as the wgpu
/// renderer refuses that draw, and a tangent off its normal is refused at creation.
#[test]
fn a_normal_mapped_material_reaches_the_frame_through_the_engine() -> Result<()> {
    let path = root("engine-normal-maps");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let floor = floor();
    let data = MeshData {
        vertices: &floor,
        indices: &INDICES,
    };
    // Along both the tangent and the bitangent, so the handedness counts.
    let texel = [0.28, 0.28, 0.92];
    let along_x = |handedness: f32| TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[1., 0., 0., handedness]; 4],
    };
    let camera = Camera::look_at(
        Vec3::new(0., 4., -2.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    // The exact traced frame, as the frame-level test renders it.
    let exact = Settings {
        quality: Quality::Reference,
        ..settings(false)
    };
    type Image = Vec<[f32; 3]>;
    let mut frame =
        |transform: Mat4, tangents: Option<TangentStream>, mapped: bool| -> Result<Image> {
            let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, exact)?;
            engine.resize(SIZE[0], SIZE[1])?;
            engine.set_sky(Sky::Uniform([0.; 3]))?;
            engine.set_sun(Some(sun()))?;
            let mesh = match &tangents {
                Some(stream) => engine.create_mesh_with_tangents(&data, stream)?,
                None => engine.create_mesh(&data)?,
            };
            let normal_texture = if mapped {
                Some(TextureRef {
                    texture: engine.create_texture(constant_map(texel))?,
                    sampler: Sampler::default(),
                })
            } else {
                None
            };
            let material = engine.create_material(&MaterialDesc {
                base_color: [0.5, 0.5, 0.5, 1.],
                roughness: 0.6,
                double_sided: true,
                normal_texture,
                ..Default::default()
            })?;
            engine.spawn(mesh, material, transform)?;
            let image = engine
                .render(&mut gpu, &camera)
                .map_err(anyhow::Error::from)
                .and_then(|_| hdr(&engine));
            let refusals = engine.refusals().to_vec();
            engine.retire(&mut gpu)?;
            ensure!(refusals.is_empty(), "refused: {refusals:?}");
            image
        };
    let flat = frame(Mat4::IDENTITY, None, false)?;
    for handedness in [1., -1.] {
        let mapped = frame(Mat4::IDENTITY, Some(along_x(handedness)), true)?;
        // `material_normal`: t x + b y + n z, with b = handedness * cross(n, t), from
        // the texel as the shader decodes it.
        let map = constant_map(texel);
        let d = Vec3::from_array(std::array::from_fn(|i| {
            f32::from(map.mips[0].rgba8[i]) / 255. * 2. - 1.
        }))
        .normalize();
        let (n, t) = (Vec3::Y, Vec3::X);
        let world = (t * d.x + handedness * n.cross(t) * d.y + n * d.z).normalize();
        let tilted = frame(
            Mat4::from_quat(glam::Quat::from_rotation_arc(Vec3::Y, world)),
            None,
            false,
        )?;
        let dark = tilted.iter().flatten().filter(|v| **v <= 0.01).count();
        ensure!(
            dark == 0,
            "the tilted floor is dark or missing at {dark} values"
        );
        let apart = mapped
            .iter()
            .flatten()
            .zip(flat.iter().flatten())
            .map(|(a, b)| (a - b).abs() / b)
            .sum::<f32>()
            / (3 * mapped.len()) as f32;
        let worst = worst(&mapped, &tilted);
        ensure!(
            worst < 5e-3 && apart > 0.02,
            "handedness {handedness}: {worst:.4} from the tilted floor at worst, {apart:.4} from the flat floor on average"
        );
        eprintln!(
            "engine normal map, handedness {handedness}: {worst:.5} worst, {apart:.3} from flat"
        );
    }
    let missing = frame(Mat4::IDENTITY, None, true);
    ensure!(
        missing
            .as_ref()
            .is_err_and(|e| format!("{e:#}").contains("tangent stream")),
        "a normal map on a mesh without tangents was drawn: {:?}",
        missing.map(drop)
    );
    let skewed = TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[0.6, 0.8, 0., 1.]; 4],
    };
    ensure!(
        frame(Mat4::IDENTITY, Some(skewed), false).is_err(),
        "a tangent off the plane of its normal was accepted"
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A cube level of `size` whose texels are `value` of their direction.
fn level(size: u32, value: impl Fn([f64; 3]) -> [f32; 3]) -> Result<CubeLevel> {
    let mut texels = vec![];
    for face in CubeFace::ALL {
        for y in 0..size {
            for x in 0..size {
                texels.push(value(face.direction([
                    2. * (x as f64 + 0.5) / size as f64 - 1.,
                    2. * (y as f64 + 0.5) / size as f64 - 1.,
                ])));
            }
        }
    }
    Ok(CubeLevel::new(size, texels, Limits::default())?)
}
fn metadata() -> Metadata {
    Metadata {
        units: RadianceUnits::CandelasPerSquareMetre,
        rotation_y_radians: 0.,
        source_radiance_scale: 1.,
        generator_hash: [7; 32],
    }
}
/// A map of radiance `radiance(direction)` everywhere and at every roughness (exact
/// for a constant), as a legacy mip chain (with banked levels) or an independent field.
fn prepared(
    independent: bool,
    radiance: impl Fn([f64; 3]) -> [f32; 3] + Copy,
) -> Result<PreparedEnvironment> {
    let irradiance = level(2, |d| radiance(d).map(|v| v * std::f32::consts::PI))?;
    Ok(if independent {
        PreparedEnvironment::new_independent(
            metadata(),
            irradiance,
            (0..roughness_knots().len())
                .map(|_| level(4, radiance))
                .collect::<Result<_>>()?,
            roughness_knots(),
            FieldGenerator::F64HostF32GpuV1,
            Limits::default(),
        )?
    } else {
        PreparedEnvironment::new(
            metadata(),
            irradiance,
            [4, 2, 1]
                .into_iter()
                .map(|size| level(size, radiance))
                .collect::<Result<_>>()?,
            Limits::default(),
        )?
    })
}

/// `Sky::Map` through the engine, against `Sky::Uniform`. A floor (half metal, rough
/// 0.3, so both the diffuse and the prefiltered lookups count) under the sun, seen from
/// where the sky fills the top of the image.
///
/// - A constant map of radiance L, shown as the background, is the uniform sky L: its
///   irradiance is pi L and its prefiltered radiance L at every roughness, and misses
///   read L. Within 1% at every pixel, both layouts, with direct light, with the
///   radiance cache over three frames, and with no sun at all. Nothing is refused.
/// - At intensity 2 with the background at its own scale 1, the lit floor is the uniform
///   sky 2L and the misses are still L.
/// - A map bright red toward +Z and blue elsewhere, seen toward +Z: misses are red; at a
///   rotation of half a turn they are blue.
#[test]
fn a_sky_map_lights_and_fills_misses_as_its_uniform_sky_does() -> Result<()> {
    let path = root("engine-sky-maps");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let floor = floor();
    let data = MeshData {
        vertices: &floor,
        indices: &INDICES,
    };
    let camera = Camera::look_at(
        Vec3::new(0., 2., -6.),
        Vec3::new(0., 0.5, 0.),
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    const L: [f32; 3] = [3000., 4000., 5000.];
    enum Choice {
        Uniform([f32; 3]),
        Map(PreparedEnvironment, f32, f32, Background),
    }
    /// The sun and direct light, the sun and the radiance cache, or the sky alone.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Light {
        Direct,
        Cached,
        Sunless,
    }
    let mut frames = |choice: &Choice, light: Light| -> Result<Vec<[f32; 3]>> {
        let mut engine = Engine::new(
            ShaderCache::new(path.join("shaders"))?,
            settings(light == Light::Cached),
        )?;
        engine.resize(SIZE[0], SIZE[1])?;
        if light != Light::Sunless {
            engine.set_sun(Some(sun()))?;
        }
        let sky = match choice {
            Choice::Uniform(radiance) => Sky::Uniform(*radiance),
            Choice::Map(map, rotation_y, intensity, background) => Sky::Map {
                map: engine.create_environment(map)?,
                rotation_y: *rotation_y,
                intensity: *intensity,
                background: *background,
            },
        };
        engine.set_sky(sky)?;
        let mesh = engine.create_mesh(&data)?;
        let material = engine.create_material(&MaterialDesc {
            base_color: [0.6, 0.5, 0.4, 1.],
            metallic: 0.5,
            roughness: 0.3,
            double_sided: true,
            ..Default::default()
        })?;
        engine.spawn(mesh, material, Mat4::IDENTITY)?;
        for _ in 0..if light == Light::Cached { 3 } else { 1 } {
            engine.render(&mut gpu, &camera)?;
        }
        let image = hdr(&engine)?;
        let refusals = engine.refusals().to_vec();
        engine.retire(&mut gpu)?;
        ensure!(refusals.is_empty(), "refused: {refusals:?}");
        ensure!(
            image.iter().flatten().all(|v| v.is_finite() && *v > 0.),
            "an unlit or nonfinite pixel"
        );
        Ok(image)
    };
    let constant = |_: [f64; 3]| L;
    let mut report = String::new();
    for light in [Light::Direct, Light::Cached, Light::Sunless] {
        let uniform = frames(&Choice::Uniform(L), light)?;
        for independent in [false, true] {
            let map = frames(
                &Choice::Map(prepared(independent, constant)?, 0., 1., Background::Sky),
                light,
            )?;
            let off = worst(&map, &uniform);
            ensure!(
                off < 0.01,
                "constant map (independent {independent}, {light:?}): {off:.4} from the uniform sky at worst"
            );
            report += &format!("{light:?} independent {independent}: {off:.5}; ");
        }
    }

    // Misses show the exposed background radiance; the floor does nowhere.
    let exposed = |rgb: [f32; 3]| rgb.map(|v| v * STOPS.exp2());
    let uniform = frames(&Choice::Uniform(L), Light::Direct)?;
    let misses: Vec<bool> = uniform
        .iter()
        .map(|p| worst(std::slice::from_ref(p), &[exposed(L)]) < 2e-3)
        .collect();
    let count = misses.iter().filter(|m| **m).count();
    ensure!(
        count > 100 && count < misses.len() / 2,
        "{count} sky pixels of {}",
        misses.len()
    );
    let twice = frames(&Choice::Uniform(L.map(|v| 2. * v)), Light::Direct)?;
    let scaled = frames(
        &Choice::Map(
            prepared(false, constant)?,
            0.,
            2.,
            Background::ScaledSky(1.),
        ),
        Light::Direct,
    )?;
    let (mut floor_worst, mut sky_worst) = (0f32, 0f32);
    for ((m, (s, t)), u) in misses.iter().zip(scaled.iter().zip(&twice)).zip(&uniform) {
        let (target, w) = if *m {
            (u, &mut sky_worst)
        } else {
            (t, &mut floor_worst)
        };
        *w = w.max(worst(std::slice::from_ref(s), std::slice::from_ref(target)));
    }
    ensure!(
        floor_worst < 0.01 && sky_worst < 0.01,
        "intensity 2, background at 1: floor {floor_worst:.4} from the sky 2L, sky {sky_worst:.4} from L"
    );

    let (red, blue) = ([8000., 1000., 1000.], [1000., 1000., 8000.]);
    let toward_z = move |d: [f64; 3]| if d[2] > 0. { red } else { blue };
    for (rotation, expected) in [(0., red), (std::f32::consts::PI, blue)] {
        let image = frames(
            &Choice::Map(prepared(false, toward_z)?, rotation, 1., Background::Sky),
            Light::Direct,
        )?;
        let sky: Vec<[f32; 3]> = image
            .iter()
            .zip(&misses)
            .filter(|(_, m)| **m)
            .map(|(p, _)| *p)
            .collect();
        let off = worst(&sky, &vec![exposed(expected); sky.len()]);
        ensure!(
            off < 0.02,
            "rotation {rotation}: the sky is {off:.4} from its expected colour at worst"
        );
    }
    eprintln!("engine sky map: {report}scaled floor {floor_worst:.5}, sky {sky_worst:.5}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
