//! Traced reflections on the traced frame, against what a mirror must show.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, ReflectionSettings, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];

/// A metal mirror floor in front of a sunlit matte wall, under a black sky. The camera
/// sees the wall directly and, in the floor, its reflection.
///
/// - With reflections off, the floor shows its untraced estimate of a black sky: nothing
///   but the far tail of the sun's highlight, under a ten-thousandth of the wall.
/// - With them on, a floor pixel whose mirror ray meets the wall shows the wall's
///   radiance times the metal's reflectance toward the camera: all of it for a white
///   metal, Schlick's fraction of it for a grey one. The wall's radiance is read from
///   the pixels that see it directly; it is matte, so it sends the same radiance along
///   the mirror ray, short of its small specular share.
/// - A floor pixel whose mirror ray passes the wall's top edge sees the black sky.
/// - At roughness 0.2 the floor traces strata over its lobe instead of one ray: pixels
///   aimed at the wall's middle, dark without reflections, hold most of its radiance.
/// - The wall itself is unchanged: too rough to trace.
#[test]
fn a_mirror_floor_shows_the_wall_at_the_metals_reflectance() -> Result<()> {
    let path = root("reflections-mirror");
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
        [[-1., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-1., 4., 0.]],
        [0., 0., 1.],
    );
    const TOP: f64 = 1.5;
    let wall = quad(
        [
            [-1., -4., 0.],
            [-1., 4., 0.],
            [-1., 4., 1.5],
            [-1., -4., 1.5],
        ],
        [1., 0., 0.],
    );
    // The sun comes down the +x side onto the wall and the floor; its mirror image in
    // the floor leaves away from the camera.
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-1., 0., -1.),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![12.],
        disk_angle: 0.02,
    };
    let eye = Vec3::new(3., 0., 1.2);
    let camera = Camera::look_at(
        eye,
        Vec3::new(-1., 0., 0.4),
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let mut frame =
        |metal: f32, gloss: f32, reflections: Option<ReflectionSettings>| -> Result<Vec<f32>> {
            let material = |base: f32, metallic: f32, roughness: f32| MaterialDescriptor {
                base_color: [base, base, base, 1.],
                metallic,
                roughness,
                double_sided: true,
                ..Default::default()
            };
            let images = render(
                &mut gpu,
                &cache,
                &ForwardScene {
                    objects: &[
                        ForwardObject {
                            vertices: &floor,
                            indices: &[0, 1, 2, 0, 2, 3],
                            transform: Mat4::IDENTITY,
                            material: material(metal, 1., gloss),
                        },
                        ForwardObject {
                            vertices: &wall,
                            indices: &[0, 1, 2, 0, 2, 3],
                            transform: Mat4::IDENTITY,
                            material: material(0.6, 0., 1.),
                        },
                    ],
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
    let off = frame(1., 0.02, None)?;
    let white = frame(1., 0.02, Some(ReflectionSettings::default()))?;
    let grey = frame(0.5, 0.02, Some(ReflectionSettings::default()))?;
    // The same floor at roughness 0.2: ten strata over its lobe instead of one ray.
    let glossy_off = frame(1., 0.2, None)?;
    let glossy = frame(1., 0.2, Some(ReflectionSettings::default()))?;
    // Each pixel's camera ray, and what it and its mirror ray meet.
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    let eye = eye.as_dvec3();
    let (mut walls, mut mirrored, mut skies) = (Vec::new(), Vec::new(), Vec::new());
    for y in (2..SIZE[1]).step_by(4) {
        for x in (2..SIZE[0]).step_by(4) {
            let far = inverse.project_point3(DVec3::new(
                (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2. - 1.,
                1. - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.,
                0.5,
            ));
            let d = (far - eye).normalize();
            let at = (y * SIZE[0] + x) as usize;
            // The wall plane x = -1, then the floor z = 0.
            let on_wall = eye + d * ((-1. - eye.x) / d.x);
            if d.x < 0. && on_wall.z > 0.1 && on_wall.z < TOP - 0.1 && on_wall.y.abs() < 3.5 {
                walls.push(at);
                continue;
            }
            if d.z >= 0. {
                continue;
            }
            let on_floor = eye + d * (-eye.z / d.z);
            if !(on_floor.x > -0.9 && on_floor.x < 3.5 && on_floor.y.abs() < 3.5) {
                continue;
            }
            let up = DVec3::new(d.x, d.y, -d.z);
            let reflected = on_floor + up * ((-1. - on_floor.x) / up.x);
            let cosine = -d.z;
            if reflected.z < TOP - 0.1 && reflected.y.abs() < 3.5 {
                mirrored.push((at, cosine, reflected.z));
            } else if reflected.z > TOP + 0.1 {
                skies.push(at);
            }
        }
    }
    ensure!(
        walls.len() >= 40 && mirrored.len() >= 40 && skies.len() >= 10,
        "the fixture covers too little: {} wall, {} mirrored, {} sky pixels",
        walls.len(),
        mirrored.len(),
        skies.len()
    );
    let wall = walls.iter().map(|at| f64::from(white[*at])).sum::<f64>() / walls.len() as f64;
    for at in &walls {
        ensure!(
            white[*at] == off[*at] && (f64::from(white[*at]) / wall - 1.).abs() < 0.03,
            "the wall is not uniform and unchanged: {} against {} (mean {wall})",
            white[*at],
            off[*at]
        );
    }
    let mut worst = [0f64; 2];
    let mut glossy_share = (f64::MAX, 0f64);
    for (at, cosine, height) in &mirrored {
        // Only the far tail of the sun's own highlight.
        ensure!(
            f64::from(off[*at]) < 1e-4 * wall,
            "the floor is lit without reflections: {}",
            off[*at]
        );
        // Schlick with F0 the metal's base colour.
        let schlick = |f0: f64| f0 + (1. - f0) * (1. - cosine).powi(5);
        for (index, (image, f0)) in [(&white, 1.), (&grey, 0.5)].into_iter().enumerate() {
            let share = f64::from(image[*at]) / wall;
            worst[index] = worst[index].max((share - schlick(f0)).abs());
        }
        // A glossy lobe aimed at the wall's middle lands on the wall, give or take the
        // strata that stray to the floor or the sky.
        if (0.5..0.9).contains(height) {
            ensure!(
                f64::from(glossy_off[*at]) < 0.02 * wall,
                "the glossy floor is lit without reflections: {}",
                glossy_off[*at]
            );
            let share = f64::from(glossy[*at]) / wall;
            glossy_share = (glossy_share.0.min(share), glossy_share.1.max(share));
        }
    }
    let sky = skies.iter().map(|at| white[*at]).fold(0f32, f32::max);
    eprintln!(
        "wall radiance {wall:.4} over {} pixels; {} mirrored pixels within {:.4} (white) and \
         {:.4} (grey) of the metal's reflectance; {} sky pixels at most {sky}",
        walls.len(),
        mirrored.len(),
        worst[0],
        worst[1],
        skies.len()
    );
    // The wall's own specular share differs between the two view directions, and the
    // program's reflectance comes from its DFG table rather than Schlick's fit.
    ensure!(
        glossy_share.0 > 0.7 && glossy_share.1 < 1.05,
        "glossy floor pixels aimed at the wall hold {:.3} to {:.3} of its radiance",
        glossy_share.0,
        glossy_share.1
    );
    eprintln!(
        "glossy floor: {:.3} to {:.3} of the wall",
        glossy_share.0, glossy_share.1
    );
    ensure!(
        worst[0] < 0.01 && worst[1] < 0.01 && f64::from(sky) < 1e-4 * wall,
        "mirrored pixels are {:.4} (white) and {:.4} (grey) from the metal's reflectance; sky {sky}",
        worst[0],
        worst[1]
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Under a uniform sky with nothing to occlude it, every reflection ray misses and
/// brings back the sky's own radiance, which is what the untraced estimate assumes. So
/// a lone metal floor must look the same with reflections on as off, in each of the
/// three bands: one mirror ray, the glossy strata, and the fade toward the cutoff. This
/// holds the strata's mean, the traced fraction's blend and the sky lookup together.
#[test]
fn a_uniform_sky_reflects_as_the_untraced_estimate_in_every_band() -> Result<()> {
    let path = root("reflections-sky");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    // The sun is too dim to matter and its mirror image leaves away from the camera.
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-1., 0., -1.),
            lux: [1e-3; 3],
        }],
        environment: [1.; 3],
        exposure_stops: 0.,
    };
    let shadows = SunShadows {
        split_ends: vec![12.],
        disk_angle: 0.02,
    };
    let camera = Camera::look_at(
        Vec3::new(3., 0., 1.2),
        Vec3::new(0., 0., 0.),
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let mut frame = |roughness: f32, reflections: Option<ReflectionSettings>| -> Result<Vec<f32>> {
        let images = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &[ForwardObject {
                    vertices: &floor,
                    indices: &[0, 1, 2, 0, 2, 3],
                    transform: Mat4::IDENTITY,
                    material: MaterialDescriptor {
                        base_color: [0.9, 0.9, 0.9, 1.],
                        metallic: 1.,
                        roughness,
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
                reflections,
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
    let mut report = String::new();
    for (band, roughness) in [("mirror", 0.02), ("glossy", 0.2), ("fading", 0.45)] {
        let off = frame(roughness, None)?;
        let on = frame(roughness, Some(ReflectionSettings::default()))?;
        // Floor pixels only: the lower half of the image.
        let (mut worst, mut pixels) = (0f32, 0);
        for at in (SIZE[0] * SIZE[1] / 2) as usize..off.len() {
            if off[at] <= 0.05 {
                continue;
            }
            worst = worst.max((on[at] / off[at] - 1.).abs());
            pixels += 1;
        }
        report += &format!("{band} ({roughness}): {pixels} pixels within {worst:.4}; ");
        ensure!(
            pixels > 3000 && worst < 0.02,
            "{band} floor at roughness {roughness} changes by {worst} over {pixels} pixels"
        );
    }
    eprintln!("{report}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
