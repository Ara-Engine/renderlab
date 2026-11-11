//! The assembled forward lit frame against Renderlab's own closed-form photometry.
//!
//! The scene, sample points, expected radiance and tolerance are the ones
//! `gpu_lit_photometry_1x_4x` holds the wgpu renderer to: a floor facing the camera,
//! one light at a time, the BRDF and attenuation evaluated in f64, and the uniform
//! environment integrated by an independent quadrature rather than read from the LUT.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, PostSettings, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, post_record, render},
};
const SIZE: [u32; 2] = [128, 96];
const BASE: [f32; 4] = [0.18, 0.4, 0.7, 1.];

fn brdf(n: DVec3, v: DVec3, l: DVec3, base: DVec3, metal: f64, rough: f64) -> DVec3 {
    let nl = n.dot(l).max(0.);
    let nv = n.dot(v).max(1e-7);
    let h = (l + v).normalize();
    let nh = n.dot(h).max(0.);
    let vh = v.dot(h).clamp(0., 1.);
    let a = rough.powi(2).max(1e-4);
    let a2 = a * a;
    let d = a2 / (std::f64::consts::PI * (1. + nh * nh * (a2 - 1.)).powi(2));
    let vis = 0.5
        / (nl * (a2 + (1. - a2) * nv * nv).sqrt() + nv * (a2 + (1. - a2) * nl * nl).sqrt())
            .max(1e-20);
    let f0 = DVec3::splat(0.04).lerp(base, metal);
    let f = f0 + (DVec3::ONE - f0) * (1. - vh).powi(5);
    (base * ((1. - metal) / std::f64::consts::PI) + d * vis * f) * nl
}
// Midpoint product quadrature over GGX's inverse CDF and azimuth.
fn integrate_dfg(nv: f64, r: f64, nu: usize, np: usize) -> [f64; 2] {
    let a = r.powi(2).max(1e-4);
    let a2 = a * a;
    let mut sum = [0.; 2];
    for y in 0..nu {
        let t = (y as f64 + 0.5) / nu as f64;
        let u = 1. - (1. - t).powi(3);
        let jacobian = 3. * (1. - t).powi(2);
        let hz = ((1. - u) / (1. - u + a2 * u)).sqrt();
        let hs = (1. - hz * hz).max(0.).sqrt();
        for x in 0..np {
            let phi = std::f64::consts::TAU * (x as f64 + 0.5) / np as f64;
            let vh = ((1. - nv * nv).sqrt() * hs * phi.cos() + nv * hz).max(0.);
            let nl = (2. * vh * hz - nv).max(0.);
            let lambda_l = nv * (a2 + (1. - a2) * nl * nl).sqrt();
            let lambda_v = nl * (a2 + (1. - a2) * nv * nv).sqrt();
            let weight = 2. * nl * vh / (hz * (lambda_l + lambda_v).max(1e-30)) * jacobian;
            let f = (1. - vh).powi(5);
            sum[0] += weight * (1. - f);
            sum[1] += weight * f;
        }
    }
    sum.map(|v| v / (nu * np) as f64)
}
/// Direction to the light and its unexposed energy at floor point `p`.
fn arriving(light: &Light, p: DVec3) -> (DVec3, DVec3) {
    match *light {
        Light::Directional { travel, lux } => (
            -travel.as_dvec3().normalize(),
            DVec3::from_array(lux.map(f64::from)),
        ),
        Light::Point {
            position,
            candela,
            range,
            ..
        }
        | Light::Spot {
            position,
            candela,
            range,
            ..
        } => {
            let delta = position.as_dvec3() - p;
            let distance = delta.length();
            let l = delta / distance;
            let mut a = 1. / distance.powi(2).max(1e-4);
            if let Some(range) = range {
                a *= (1. - (distance / f64::from(range)).powi(4))
                    .clamp(0., 1.)
                    .powi(2);
            }
            if let Light::Spot {
                travel,
                inner,
                outer,
                ..
            } = *light
            {
                a *= ((-l.dot(travel.as_dvec3().normalize()) - f64::from(outer).cos())
                    / (f64::from(inner).cos() - f64::from(outer).cos()).max(1e-4))
                .clamp(0., 1.)
                .powi(2);
            }
            (l, DVec3::from_array(candela.map(f64::from)) * a)
        }
    }
}
#[test]
fn forward_lit_frame_matches_closed_form_photometry() -> Result<()> {
    let path = root("forward");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let mut frame = |metal: f32, rough: f32, lighting: &Lighting| -> Result<Vec<f32>> {
        let bytes = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[ForwardObject {
                    vertices: &vertices,
                    indices: &[0, 1, 2, 0, 2, 3],
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        base_color: BASE,
                        metallic: metal,
                        roughness: rough,
                        ..Default::default()
                    },
                }],
                lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: None,
                primary: renderlab::backend_ara::forward::Primary::Raster,
                post: None,
            },
        )?
        .hdr;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f16::from_bits(u16::from_le_bytes(*b)).to_f32())
            .collect())
    };
    // The floor point under a pixel centre, and the unit direction back to the eye.
    let tan = (f64::from(camera.fov_y()) / 2.).tan();
    let floor = |x: usize, y: usize| {
        let p = DVec3::new(
            (2. * (x as f64 + 0.5) / 128. - 1.) * tan * 4. * 128. / 96.,
            (1. - 2. * (y as f64 + 0.5) / 96.) * tan * 4.,
            0.,
        );
        (p, (DVec3::new(0., 0., 4.) - p).normalize())
    };
    let base = DVec3::from_array([BASE[0], BASE[1], BASE[2]].map(f64::from));
    let lights = [
        Light::Directional {
            travel: Vec3::new(-0.25, 0., -1.),
            lux: [1000.; 3],
        },
        Light::Point {
            position: Vec3::Z,
            candela: [100.; 3],
            range: None,
            source_radius: 0.0,
        },
        Light::Point {
            position: Vec3::Z * 2.,
            candela: [100.; 3],
            range: None,
            source_radius: 0.0,
        },
        Light::Point {
            position: Vec3::Z,
            candela: [100.; 3],
            range: Some(0.9),
            source_radius: 0.0,
        },
        Light::Spot {
            position: Vec3::Z * 2.,
            travel: -Vec3::Z,
            candela: [100.; 3],
            range: Some(4.),
            inner: 0.25,
            outer: 0.5,
            source_radius: 0.0,
        },
    ];
    let mut worst = 0f64;
    let mut samples = 0;
    for (metal, rough) in [
        (0., 0.1),
        (0., 0.5),
        (0., 1.),
        (1., 0.1),
        (1., 0.5),
        (1., 1.),
    ] {
        for light in &lights {
            let image = frame(
                metal,
                rough,
                &Lighting {
                    lights: vec![*light],
                    exposure_stops: -8.,
                    ..Default::default()
                },
            )?;
            for y in (18..78).step_by(11) {
                for x in (18..110).step_by(13) {
                    let (p, v) = floor(x, y);
                    let (l, energy) = arriving(light, p);
                    let expected = brdf(DVec3::Z, v, l, base, f64::from(metal), f64::from(rough))
                        * energy
                        / 256.;
                    for channel in 0..3 {
                        let actual = f64::from(image[(y * 128 + x) * 4 + channel]);
                        let error = (actual - expected[channel]).abs();
                        ensure!(
                            actual.is_finite() && error <= 0.002 + expected[channel] * 0.01,
                            "{light:?} m={metal} r={rough} {x},{y} channel={channel} got {actual} expected {}",
                            expected[channel]
                        );
                        worst = worst.max(error / (0.002 + expected[channel] * 0.01));
                        samples += 1;
                    }
                }
            }
        }
        // The uniform environment through the prepared DFG, at the centre pixel.
        let image = frame(
            metal,
            rough,
            &Lighting {
                environment: [0.3; 3],
                ..Default::default()
            },
        )?;
        let (p, _) = floor(64, 48);
        let nv = 4. / (16. + p.x * p.x + p.y * p.y).sqrt();
        let ab = integrate_dfg(nv, f64::from(rough), 512, 1024);
        for channel in 0..3 {
            let metal = f64::from(metal);
            let expected = 0.3
                * (base[channel] * (1. - metal)
                    + (0.04 * (1. - metal) + base[channel] * metal) * ab[0]
                    + ab[1]);
            let actual = f64::from(image[(48 * 128 + 64) * 4 + channel]);
            ensure!(
                (actual - expected).abs() < 0.002 + 0.01 * expected,
                "environment m={metal} r={rough} channel={channel} got {actual} expected {expected}"
            );
            samples += 1;
        }
    }
    println!(
        "forward frame: {samples} radiance samples within Renderlab's photometry tolerance; worst used {:.1}% of it",
        worst * 100.
    );
    gpu.assert_clean()?;
    Ok(())
}

/// The frame's own HDR and motion surface, pushed through the scalar post replay, must
/// give the image the GPU displayed: motion blur, six bloom levels, tone and transfer
/// run in the renderer's order with its default settings.
#[test]
fn forward_frame_display_matches_cpu_post_replay_of_its_hdr() -> Result<()> {
    let path = root("forward-display");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let size = [96u32, 64];
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let settings = PostSettings::default();
    // A glossy metal floor under a tilted sun: a highlight far above one for bloom.
    let images = render(
        &mut gpu,
        &cache,
        &ForwardScene {
            objects: &[ForwardObject {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: MaterialDescriptor {
                    base_color: BASE,
                    metallic: 1.,
                    roughness: 0.3,
                    ..Default::default()
                },
            }],
            lighting: &Lighting {
                lights: vec![
                    Light::Directional {
                        travel: Vec3::new(-0.25, 0., -1.),
                        lux: [1000.; 3],
                    },
                    Light::Point {
                        position: Vec3::new(1.5, -1., 1.),
                        candela: [60., 40., 20.],
                        range: Some(6.),
                        source_radius: 0.0,
                    },
                ],
                environment: [20., 25., 40.],
                exposure_stops: -8.,
            },
            camera: Camera::look_at(
                Vec3::new(0., 0., 4.),
                Vec3::ZERO,
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                100.,
            )?,
            size,
            atmosphere: renderlab::Atmosphere::off(),
            sun_shadows: None,
            primary: renderlab::backend_ara::forward::Primary::Raster,
            post: Some(settings),
        },
    )?;
    let checked =
        display_inside_cpu_post_replay(size, &images, &post_record(&settings, size[1], 1))?;
    let (brightest, distinct) = (checked.peak, checked.colours);
    let display = images.display.expect("post ran");
    ensure!(
        brightest > 1.5 && distinct.len() > 64,
        "fixture lost its highlight or its gradient: peak {brightest}, {} colours",
        distinct.len()
    );
    println!(
        "forward display: {} pixels inside the CPU post replay's bounds; HDR peak {brightest}, {} distinct display colours",
        display.len() / 4,
        distinct.len()
    );
    gpu.assert_clean()?;
    Ok(())
}

/// The atmosphere on the frame: sun transmittance reaches the directional light exactly
/// once, the backdrop replaces the clear, and the procedural environment lights a
/// surface the sun cannot. The sky's own radiance is the stage replays' business and
/// the cross-layer comparison's; this holds the assembly.
#[test]
fn forward_frame_atmosphere_scales_the_sun_and_lights_from_the_sky() -> Result<()> {
    use renderlab::Atmosphere;
    let path = root("forward-atmosphere");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    // A floor that leaves the frame's corners to the backdrop.
    let vertices = [
        [-1.5, -1.5, 0.],
        [1.5, -1.5, 0.],
        [1.5, 1.5, 0.],
        [-1.5, 1.5, 0.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let mut frame = |travel: Vec3, atmosphere: Atmosphere| -> Result<Vec<[f32; 4]>> {
        let bytes = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[ForwardObject {
                    vertices: &vertices,
                    indices: &[0, 1, 2, 0, 2, 3],
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        base_color: BASE,
                        metallic: 0.,
                        roughness: 1.,
                        ..Default::default()
                    },
                }],
                lighting: &Lighting {
                    lights: vec![Light::Directional {
                        travel,
                        lux: [1000.; 3],
                    }],
                    exposure_stops: -8.,
                    ..Default::default()
                },
                camera,
                size: SIZE,
                atmosphere,
                sun_shadows: None,
                primary: renderlab::backend_ara::forward::Primary::Raster,
                post: None,
            },
        )?
        .hdr;
        Ok(bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
                })
            })
            .collect())
    };
    let centre = 48 * 128 + 64;
    let base = DVec3::from_array([BASE[0], BASE[1], BASE[2]].map(f64::from));

    // Sun in front of the floor, low enough for a strongly coloured transmittance.
    let travel = Vec3::new(-0.9, -0.2, -0.4);
    let sun_only = Atmosphere {
        sun_disk: false,
        aerial_perspective: false,
        environment: false,
        height_fog: None,
        volumetric_fog: None,
        ..Default::default()
    };
    let image = frame(travel, sun_only)?;
    let toward = -travel.normalize();
    let transmittance = sun_only.transmittance(0., toward);
    ensure!(
        transmittance[2] < 0.9 * transmittance[0] && transmittance[0] < 0.99,
        "fixture sun is not attenuated: {transmittance:?}"
    );
    let expected = brdf(DVec3::Z, DVec3::Z, toward.as_dvec3(), base, 0., 1.) * 1000. / 256.;
    for channel in 0..3 {
        let expected = expected[channel] * f64::from(transmittance[channel]);
        let actual = f64::from(image[centre][channel]);
        ensure!(
            (actual - expected).abs() <= 0.002 + expected * 0.01,
            "attenuated sun channel {channel}: got {actual} expected {expected}"
        );
    }
    let corner = image[0];
    ensure!(
        corner[..3].iter().all(|v| v.is_finite() && *v > 0.),
        "backdrop did not replace the black clear: {corner:?}"
    );

    // Sun behind the floor: only the sky can light it.
    let behind = Vec3::new(-0.3, -0.6, 0.4);
    let dark = frame(behind, sun_only)?[centre];
    let lit = frame(
        behind,
        Atmosphere {
            environment: true,
            ..sun_only
        },
    )?[centre];
    ensure!(
        dark[..3].iter().all(|v| *v < 1e-4),
        "floor facing away from the sun is lit without an environment: {dark:?}"
    );
    ensure!(
        lit[..3].iter().all(|v| v.is_finite() && *v > 1e-3) && lit[2] > lit[0],
        "procedural environment did not light the floor as a blue-weighted sky: {lit:?}"
    );
    println!(
        "forward atmosphere: sun transmittance {transmittance:?} applied once; backdrop {corner:?}; sky-lit floor {lit:?}"
    );
    gpu.assert_clean()?;
    Ok(())
}

/// The shading variant the smallest engine-path frame needs: the standard program on
/// the hardware soft head reflects the scene and the fresh counts beside the lit
/// interface, and the Metal adapter accepts it. Compiling it natively takes about
/// half a minute, so that stays out of this suite until a frame replays it.
#[test]
fn hardware_soft_shading_reflects_the_scene_and_fresh_counts() -> Result<()> {
    use renderlab::backend_ara::shader::{InputKey, InputKind};
    let program =
        renderlab::backend_ara::primary_shade::hardware_soft_program(renderlab::STANDARD_PROGRAM)?;
    let kind = |group, binding| {
        program
            .inputs()
            .iter()
            .find(|i| i.key == InputKey { group, binding })
            .map(|i| i.kind)
    };
    ensure!(
        matches!(kind(0, 2), Some(InputKind::AccelerationStructure))
            && matches!(kind(0, 6), Some(InputKind::Image { .. }))
            && matches!(kind(0, 18), Some(InputKind::StorageImage { .. }))
            && matches!(kind(3, 24), Some(InputKind::Image { .. })),
        "hardware soft shading interface changed: {:?}",
        program.inputs()
    );
    program.msl()?;
    // Its reflection code declares `let camera = primary_ray(..)` under the `camera`
    // global; the SPIR-V path renames that local before rewriting globals by name.
    validate_spirv(&program, "hardware-soft-shading")
}

/// The traced frame's form of the fresh estimator: the forward form's interface plus
/// the primary hit image and tile flags that gate its coverage.
#[test]
fn traced_fresh_visibility_reflects_its_primary_gates() -> Result<()> {
    use renderlab::backend_ara::shader::{InputKey, InputKind};
    let program = renderlab::backend_ara::fresh_visibility::traced_program()?;
    let keys: Vec<_> = program
        .inputs()
        .iter()
        .map(|i| (i.key.group, i.key.binding))
        .collect();
    ensure!(
        keys == [
            (0, 2),
            (0, 3),
            (0, 4),
            (0, 5),
            (0, 7),
            (0, 21),
            (3, 0),
            (3, 1),
            (3, 5)
        ] && matches!(
            program.inputs()[0],
            renderlab::backend_ara::shader::ShaderInput {
                key: InputKey { .. },
                kind: InputKind::AccelerationStructure,
                ..
            }
        ),
        "traced fresh interface changed: {:?}",
        program.inputs()
    );
    ensure!(
        program.threads()? == [32, 1, 1],
        "traced fresh workgroup changed"
    );
    program.msl()?;
    validate_spirv(&program, "traced-fresh")
}

pub(super) fn validate_spirv(
    program: &renderlab::backend_ara::shader::Program,
    label: &str,
) -> Result<()> {
    let export = root(label);
    std::fs::create_dir_all(&export)?;
    let stages = renderlab::backend_ara::cache::emit_spirv_program(program, &export)?;
    ensure!(!stages.is_empty(), "{label} emitted no SPIR-V stage");
    for stage in &stages {
        let bytes = std::fs::read(export.join(&stage.spirv))?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        assert_nonuniform_sampled_images(&words)?;
        assert_precise_arithmetic(&words)?;
        // ara-gpu enables neither hit-vertex return nor its SPIR-V extension: every
        // hit position comes from retained geometry. OpExtension is opcode 10.
        let mut at = 5;
        while at < words.len() {
            let count = (words[at] >> 16) as usize;
            ensure!(count > 0 && at + count <= words.len(), "invalid SPIR-V");
            if words[at] & 0xffff == 10 {
                let name: Vec<u8> = words[at + 1..at + count]
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .take_while(|b| *b != 0)
                    .collect();
                ensure!(
                    name != b"SPV_KHR_ray_tracing_position_fetch",
                    "{label} declares the position-fetch extension"
                );
            }
            at += count;
        }
    }
    if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
        let sdk = std::path::PathBuf::from(sdk);
        let validator = [sdk.join("bin/spirv-val"), sdk.join("Bin/spirv-val.exe")]
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| "spirv-val".into());
        for stage in &stages {
            let output = std::process::Command::new(&validator)
                .args(["--target-env", "vulkan1.3"])
                .arg(export.join(&stage.spirv))
                .output()?;
            ensure!(
                output.status.success(),
                "{label} SPIR-V: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(())
}

/// The rename behind that: WGSL scoping, case by case.
#[test]
fn locals_shadowing_a_resource_global_are_renamed_within_their_scope() -> Result<()> {
    use renderlab::backend_ara::shader::local_shadows::rename;
    let globals = std::collections::BTreeSet::from(["camera", "lights"]);
    let source = "\
@group(0) @binding(0) var<uniform> camera: Camera;
struct Ray { camera: f32, origin: vec3<f32> }
// let camera in a comment stays
fn uses_global() -> f32 { return camera.exposure; }
fn parameter(camera: Ray, other: f32) -> f32 { return camera.camera + other; }
fn block() -> f32 {
    let before = camera.exposure;
    {
        let camera = make(camera.exposure);
        var<function> lights: f32 = camera.origin.x;
        lights = lights + camera.camera;
    }
    return before + camera.exposure;
}
fn later() -> f32 { let camera = 1.0; return camera; }
";
    let expected = "\
@group(0) @binding(0) var<uniform> camera: Camera;
struct Ray { camera: f32, origin: vec3<f32> }
// let camera in a comment stays
fn uses_global() -> f32 { return camera.exposure; }
fn parameter(camera_ara_local: Ray, other: f32) -> f32 { return camera_ara_local.camera + other; }
fn block() -> f32 {
    let before = camera.exposure;
    {
        let camera_ara_local = make(camera.exposure);
        var<function> lights_ara_local: f32 = camera_ara_local.origin.x;
        lights_ara_local = lights_ara_local + camera_ara_local.camera;
    }
    return before + camera.exposure;
}
fn later() -> f32 { let camera_ara_local = 1.0; return camera_ara_local; }
";
    let renamed = rename(source, &globals)?;
    ensure!(renamed == expected, "unexpected rename:\n{renamed}");
    ensure!(
        rename(
            "fn f() { for (var camera = 0; camera < 1; camera++) {} }",
            &globals
        )
        .is_err(),
        "a loop-scoped shadow must be rejected, not guessed"
    );
    let untouched = "fn f(a: f32) -> f32 { let b = a; return b; }";
    ensure!(rename(untouched, &globals)? == untouched, "identity lost");
    Ok(())
}

/// Ray-traced sun shadows on the frame, held to geometry: a square occluder one unit
/// above the floor under a tilted sun with a disk. The floor is unshadowed radiance
/// outside the shadow, black in the umbra, and crosses a penumbra of the width the
/// disk angle predicts, centred where the occluder's edge projects.
///
/// Both frame families draw it: the mesh pass with the receiver prepass, and the
/// traced frame (camera rays, the gated estimator, shading on the hardware soft head),
/// which must also agree with the mesh pass pixel for pixel.
#[test]
fn sun_shadow_has_the_predicted_umbra_and_penumbra_on_both_frame_families() -> Result<()> {
    use renderlab::backend_ara::forward::{Primary, SunShadows};
    let path = root("forward-sun-shadow");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let quad = |half: f32| {
        [
            [-half, -half, 0.],
            [half, -half, 0.],
            [half, half, 0.],
            [-half, half, 0.],
        ]
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        })
    };
    let (floor, occluder) = (quad(4.), quad(0.5));
    let material = || MaterialDescriptor {
        base_color: BASE,
        metallic: 0.,
        roughness: 1.,
        double_sided: true,
        ..Default::default()
    };
    let travel = Vec3::new(-1.5, 0., -1.);
    let disk = 0.05f32;
    let mut frame = |primary: Primary| -> Result<Vec<f32>> {
        let bytes = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[
                    ForwardObject {
                        vertices: &floor,
                        indices: &[0, 1, 2, 0, 2, 3],
                        transform: Mat4::IDENTITY,
                        material: material(),
                    },
                    ForwardObject {
                        vertices: &occluder,
                        indices: &[0, 1, 2, 0, 2, 3],
                        transform: Mat4::from_translation(Vec3::Z),
                        material: material(),
                    },
                ],
                lighting: &Lighting {
                    lights: vec![Light::Directional {
                        travel,
                        lux: [1000.; 3],
                    }],
                    exposure_stops: -8.,
                    ..Default::default()
                },
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&SunShadows {
                    split_ends: vec![8.],
                    disk_angle: disk,
                }),
                primary,
                post: None,
            },
        )?
        .hdr;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f16::from_bits(u16::from_le_bytes(*b)).to_f32())
            .collect())
    };
    // Along the centre row the occluder's x = -0.5 edge projects to x = -2 on the floor
    // and its x = +0.5 edge to x = -1; the occluder itself hides the floor inside
    // |x| < 2/3. A disk of half-angle t around a sun at angle a from the normal spreads
    // each edge over tan(a + t) - tan(a - t) floor units per unit of height.
    let tan = (f64::from(camera.fov_y()) / 2.).tan();
    let world_x = |x: usize| (2. * (x as f64 + 0.5) / 128. - 1.) * tan * 4. * 128. / 96.;
    let toward = (-travel).normalize().as_dvec3();
    let base = DVec3::from_array([BASE[0], BASE[1], BASE[2]].map(f64::from));
    let angle = 1.5f64.atan();
    let width = (angle + f64::from(disk)).tan() - (angle - f64::from(disk)).tan();
    let unshadowed = |x: usize| {
        let p = DVec3::new(world_x(x), 0., 0.);
        let v = (DVec3::new(0., 0., 4.) - p).normalize();
        (brdf(DVec3::Z, v, toward, base, 0., 1.) * 1000. / 256.)[1]
    };
    let check = |image: &[f32], family: &str| -> Result<()> {
        let (mut lit, mut umbra, mut edge) = (0, 0, vec![]);
        for x in 0..128 {
            let wx = world_x(x);
            let fraction = f64::from(image[(48 * 128 + x) * 4 + 1]) / unshadowed(x);
            // Distances to the two projected edges, with a pixel of slack.
            let (far, near) = ((wx + 2.).abs(), (wx + 1.).abs());
            let slack = 0.5 * width + 0.06;
            if wx.abs() < 0.72 {
                continue; // the occluder, seen from above
            } else if wx < -2. - slack || (wx > -1. + slack) {
                ensure!(
                    (fraction - 1.).abs() <= 0.02,
                    "{family}: lit floor at x={wx:.3} has fraction {fraction:.4}"
                );
                lit += 1;
            } else if wx > -2. + slack && wx < -1. - slack {
                ensure!(
                    fraction.abs() <= 0.005,
                    "{family}: umbra at x={wx:.3} has fraction {fraction:.4}"
                );
                umbra += 1;
            } else if far < near {
                edge.push((wx, fraction));
            }
        }
        ensure!(
            lit > 20 && umbra > 8 && edge.len() > 6,
            "{family}: fixture lost a region: {lit} {umbra} {}",
            edge.len()
        );
        // The far edge: fully lit beyond it, falling to zero inside, half way at the edge.
        let mut previous = f64::INFINITY;
        for &(wx, fraction) in &edge {
            ensure!(
                (-0.01..=1.02).contains(&fraction) && fraction <= previous + 0.08,
                "{family}: penumbra is not a falling ramp at x={wx:.3}: {fraction:.4} after {previous:.4}"
            );
            previous = fraction;
        }
        // Where the ramp crosses one half, by linear interpolation between its neighbours.
        let crossing = edge
            .windows(2)
            .find(|w| w[0].1 >= 0.5 && w[1].1 < 0.5)
            .map(|w| w[0].0 + (w[0].1 - 0.5) / (w[0].1 - w[1].1) * (w[1].0 - w[0].0))
            .context("penumbra never crosses one half")?;
        let partial = edge
            .iter()
            .filter(|(_, f)| (0.05..0.95).contains(f))
            .count() as f64;
        let measured = partial * (world_x(1) - world_x(0));
        ensure!(
            (crossing + 2.).abs() < 0.05 && measured > 0.5 * width && measured < 1.3 * width,
            "{family}: penumbra crosses half at x={crossing:.3} (expected -2) and spans {measured:.3} (disk predicts {width:.3})"
        );
        println!(
            "{family} sun shadow: {lit} lit and {umbra} umbra pixels exact; penumbra crosses half at x={crossing:.3}, spans {measured:.3} of a predicted {width:.3}"
        );
        Ok(())
    };
    // Traced first, on a device that has drawn nothing: its target cannot hold an
    // earlier frame's pixels if the shading were to write none.
    let traced = frame(Primary::Traced)?;
    check(&traced, "traced")?;
    let raster = frame(Primary::Raster)?;
    check(&raster, "raster")?;
    // The two families shade the same receivers with the same program: every pixel
    // agrees to the photometry tolerance, plus four rays of 256 where the estimator's
    // receivers differ in their last bits.
    let (mut worst, mut differing) = (0f64, 0);
    for (i, (a, b)) in raster
        .as_chunks::<4>()
        .0
        .iter()
        .zip(traced.as_chunks::<4>().0)
        .enumerate()
    {
        let full = unshadowed(i % 128);
        for channel in 0..3 {
            let (a, b) = (f64::from(a[channel]), f64::from(b[channel]));
            let error = (a - b).abs();
            ensure!(
                error <= 0.002 + 0.01 * a.max(b) + full * 4. / 256.,
                "families differ at pixel ({}, {}) channel {channel}: raster {a} traced {b}",
                i % 128,
                i / 128
            );
            worst = worst.max(error);
            differing += usize::from(a != b);
        }
    }
    println!(
        "raster and traced frames: worst channel difference {worst:.5}; {differing} of {} channels differ at all",
        raster.len() / 4 * 3
    );
    gpu.assert_clean()?;
    Ok(())
}

/// The two frame families on a frame the geometry only partly covers, with the uniform
/// environment and with the physical sky, through bloom and tone to display bytes.
/// Misses must show the same background, covered pixels the same radiance except
/// where the estimator's receivers differ by a ray or two in a penumbra.
#[test]
fn traced_frame_agrees_with_the_mesh_pass_over_misses_sky_and_post() -> Result<()> {
    use renderlab::backend_ara::forward::{Primary, SunShadows};
    let path = root("forward-traced-misses");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let quad = |half: f32| {
        [
            [-half, -half, 0.],
            [half, -half, 0.],
            [half, half, 0.],
            [-half, half, 0.],
        ]
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        })
    };
    let (floor, occluder) = (quad(1.5), quad(0.4));
    let material = || MaterialDescriptor {
        base_color: BASE,
        metallic: 0.,
        roughness: 0.6,
        double_sided: true,
        ..Default::default()
    };
    // Motion blur's surface on a traced frame comes from hit records, not chained yet.
    let post = PostSettings {
        motion_blur: false,
        ..Default::default()
    };
    for (label, atmosphere) in [
        ("uniform environment", renderlab::Atmosphere::off()),
        (
            "physical sky",
            renderlab::Atmosphere {
                volumetric_fog: None,
                ..Default::default()
            },
        ),
    ] {
        let mut frame = |primary: Primary| -> Result<(Vec<f32>, Vec<u8>)> {
            let images = render(
                &mut gpu,
                &cache,
                &ForwardScene {
                    objects: &[
                        ForwardObject {
                            vertices: &floor,
                            indices: &[0, 1, 2, 0, 2, 3],
                            transform: Mat4::IDENTITY,
                            material: material(),
                        },
                        ForwardObject {
                            vertices: &occluder,
                            indices: &[0, 1, 2, 0, 2, 3],
                            transform: Mat4::from_translation(Vec3::Z),
                            material: material(),
                        },
                    ],
                    lighting: &Lighting {
                        lights: vec![Light::Directional {
                            travel: Vec3::new(-0.6, -0.3, -1.),
                            lux: [1000.; 3],
                        }],
                        environment: [30., 40., 60.],
                        exposure_stops: -8.,
                    },
                    camera,
                    size: SIZE,
                    atmosphere,
                    sun_shadows: Some(&SunShadows {
                        split_ends: vec![8.],
                        disk_angle: 0.03,
                    }),
                    primary,
                    post: Some(post),
                },
            )?;
            Ok((
                images
                    .hdr
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| f16::from_bits(u16::from_le_bytes(*b)).to_f32())
                    .collect(),
                images.display.context("post ran")?,
            ))
        };
        let (traced, traced_display) = frame(Primary::Traced)?;
        let (raster, raster_display) = frame(Primary::Raster)?;
        let (mut worst, mut differing) = (0f32, 0);
        for (a, b) in raster
            .as_chunks::<4>()
            .0
            .iter()
            .zip(traced.as_chunks::<4>().0)
        {
            let pixel = (0..3)
                .map(|c| (a[c] - b[c]).abs() / a[c].abs().max(b[c].abs()).max(1e-3))
                .fold(0f32, f32::max);
            worst = worst.max(pixel);
            differing += usize::from(pixel > 0.);
        }
        // The frame's corners are misses in both families.
        ensure!(
            raster[..3] == traced[..3] && raster[..3].iter().all(|v| *v > 0.),
            "{label}: backgrounds differ: raster {:?} traced {:?}",
            &raster[..3],
            &traced[..3]
        );
        ensure!(
            worst <= 0.03 && differing * 100 <= raster.len() / 4,
            "{label}: {differing} pixels differ, worst by {worst}"
        );
        let levels = raster_display
            .as_chunks::<4>()
            .0
            .iter()
            .zip(traced_display.as_chunks::<4>().0)
            .map(|(a, b)| (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0))
            .max()
            .unwrap_or(0);
        ensure!(levels <= 2, "{label}: displays differ by {levels} levels");
        println!(
            "traced against mesh pass, {label}: {differing} of {} pixels differ, worst by {:.2}%; displays within {levels} level(s)",
            raster.len() / 4,
            worst * 100.
        );
    }
    gpu.assert_clean()?;
    Ok(())
}

/// What [`display_inside_cpu_post_replay`] saw on the way.
struct Replayed {
    /// The brightest HDR channel.
    peak: f32,
    /// Distinct displayed colours.
    colours: std::collections::BTreeSet<[u8; 4]>,
    /// The largest shutter velocity in the motion surface, in pixels.
    speed: f32,
}

/// Pushes a frame's own HDR and motion surface through the scalar post replay (tile
/// maximum, motion blur, six bloom levels, tone, sRGB transfer) with these `Settings`
/// words and requires every displayed byte inside its bounds: the tone stage's 0.003
/// linear allowance carried through the transfer, plus UNORM rounding.
fn display_inside_cpu_post_replay(
    size: [u32; 2],
    images: &renderlab::backend_ara::forward::ForwardImages,
    words: &[f32; 20],
) -> Result<Replayed> {
    let image = |pixels| reference::Image { size, pixels };
    let hdr = image(
        images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
                })
            })
            .collect(),
    );
    let surface = images.surface.as_ref().context("motion blur ran")?;
    let aux = image(
        surface
            .as_chunks::<16>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f32::from_le_bytes([p[c * 4], p[c * 4 + 1], p[c * 4 + 2], p[c * 4 + 3]])
                })
            })
            .collect(),
    );
    let tiles = reference::tile_max(&aux);
    let blurred = reference::motion_blur(&hdr, &aux, &tiles, words);
    let mut levels = Vec::new();
    let mut current = blurred.clone();
    for _ in 0..6 {
        current = reference::downsample(&current, current.size.map(|n| n.div_ceil(2)));
        levels.push(current.clone());
        if current.size == [1, 1] {
            break;
        }
    }
    let coarsest = levels.last().context("bloom has a level")?;
    let mut bloom = reference::upsample(coarsest, coarsest);
    for level in levels[..levels.len() - 1].iter().rev() {
        bloom = reference::upsample(level, &bloom);
    }
    let fixture = reference::Fixture {
        hdr: hdr.clone(),
        lut: (0..8u32)
            .map(|i| [i & 1, (i >> 1) & 1, i >> 2, 1].map(|v| v as f32))
            .collect(),
        lut_size: 2,
        settings: *words,
    };
    let tone = reference::tone(&blurred, &bloom, &fixture);
    let shifted = |by: f32| {
        reference::display(&image(
            tone.pixels.iter().map(|p| p.map(|v| v + by)).collect(),
        ))
        .pixels
    };
    let (low, high) = (shifted(-0.003), shifted(0.003));
    let display = images.display.as_ref().context("post ran")?;
    let mut seen = Replayed {
        peak: 0.,
        colours: Default::default(),
        speed: aux
            .pixels
            .iter()
            .map(|p| p[0].hypot(p[1]))
            .fold(0., f32::max),
    };
    for (i, actual) in display.as_chunks::<4>().0.iter().enumerate() {
        seen.colours.insert(*actual);
        seen.peak = seen
            .peak
            .max(hdr.pixels[i][0].max(hdr.pixels[i][1]).max(hdr.pixels[i][2]));
        for channel in 0..3 {
            let value = f32::from(actual[channel]) / 255.;
            ensure!(
                value >= low[i][channel] - 0.6 / 255. && value <= high[i][channel] + 0.6 / 255.,
                "display pixel {i} channel {channel}: {value} outside [{}, {}]",
                low[i][channel],
                high[i][channel]
            );
        }
        ensure!(actual[3] == 255, "display alpha");
    }
    Ok(seen)
}

/// A sequence keeps what the renderer keeps between frames. The first frame has no
/// previous camera and a still surface; the second, after the camera moves, carries
/// serial two and real shutter velocities into the motion blur, and its display still
/// sits inside the scalar replay of its own HDR and surface. A cut forgets all of it.
#[test]
fn frames_carry_the_previous_camera_into_motion_blur() -> Result<()> {
    use renderlab::backend_ara::forward::{Frames, Primary};
    let path = root("forward-sequence");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let size = [96u32, 64];
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let settings = PostSettings::default();
    let lighting = Lighting {
        lights: vec![
            Light::Directional {
                travel: Vec3::new(-0.25, 0., -1.),
                lux: [1000.; 3],
            },
            Light::Point {
                position: Vec3::new(1.5, -1., 1.),
                candela: [60., 40., 20.],
                range: Some(6.),
                source_radius: 0.0,
            },
        ],
        environment: [20., 25., 40.],
        exposure_stops: -8.,
    };
    let mut frames = Frames::default();
    let mut frame = |frames: &mut Frames, eye_x: f32| {
        frames.render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[ForwardObject {
                    vertices: &vertices,
                    indices: &[0, 1, 2, 0, 2, 3],
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        base_color: BASE,
                        metallic: 1.,
                        roughness: 0.3,
                        ..Default::default()
                    },
                }],
                lighting: &lighting,
                camera: Camera::look_at(
                    Vec3::new(eye_x, 0., 4.),
                    Vec3::new(eye_x, 0., 0.),
                    Vec3::Y,
                    55f32.to_radians(),
                    0.1,
                    100.,
                )?,
                size,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: None,
                primary: Primary::Raster,
                post: Some(settings),
            },
        )
    };
    let first = frame(&mut frames, 0.)?;
    let still = display_inside_cpu_post_replay(size, &first, &post_record(&settings, size[1], 1))?;
    ensure!(
        frames.next_post_serial() == 2,
        "post serial did not advance"
    );
    let second = frame(&mut frames, 0.3)?;
    let moving =
        display_inside_cpu_post_replay(size, &second, &post_record(&settings, size[1], 2))?;
    // A sideways camera step of 0.3 at distance 4 moves the floor by 0.3 of the view's
    // width in world units, in pixels; the surface stores the shutter's share of it,
    // angle / 720 of the frame-to-frame motion.
    let view_width = 2. * 4. * (55f32.to_radians() / 2.).tan() * size[0] as f32 / size[1] as f32;
    let expected = 0.3 / view_width * size[0] as f32 * settings.shutter_angle_degrees / 720.;
    ensure!(
        still.speed == 0. && (moving.speed - expected).abs() < 0.01,
        "shutter velocities: first frame {}, second {} (expected {expected})",
        still.speed,
        moving.speed
    );
    frames.cut();
    let after_cut = frame(&mut frames, 0.3)?;
    let cut =
        display_inside_cpu_post_replay(size, &after_cut, &post_record(&settings, size[1], 1))?;
    ensure!(
        cut.speed == 0. && after_cut.hdr == second.hdr && after_cut.display != second.display,
        "a cut must drop the previous camera and nothing else"
    );
    println!(
        "frame sequence: still surface on frame one, {:.2} px shutter velocity on frame two, none after a cut; all three displays inside the CPU post replay",
        moving.speed
    );
    gpu.assert_clean()?;
    Ok(())
}
