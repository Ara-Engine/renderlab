//! The game scene's opaque objects (`examples/game_scene.rs`) on the traced frame, shaded
//! by the scene's own two programs.
//!
//! The scene exists only as calls on `engine::Engine`, which the backend cannot take yet
//! (docs/design-engine-backend.md). Until it can, this file restates the scene's first
//! frame: the same meshes, transforms, materials, programs and parameters, the sun, the
//! lanterns, the atmosphere and the camera. The geometry builders are copies of the
//! example's and go when the example itself runs on the backend.
use super::*;
use renderlab::{
    Atmosphere, Camera, HeightFog, IndirectSettings, Light, Lighting, LocalShadows, Mat4,
    MaterialDescriptor, MeshData, PostSettings, Reconstruction, ReflectionSettings, Transparency,
    Value, Vec3, Vertex, VolumetricFog,
    backend_ara::{
        forward::{
            ForwardObject, ForwardScene, Frames, ObjectProgram, Primary, ScenePrograms, SunShadows,
            render,
        },
        gi_cache::Indirect,
        temporal::Reconstruct,
    },
    cutout::{AlphaMask, cutout},
    engine::Sun,
};
use std::f32::consts::PI;

const GAME: &str = include_str!("../../../examples/game_scene.wgsl");
const TOON: &str = include_str!("../../../examples/game_toon.wgsl");
const SIZE: [u32; 2] = [480, 270];
/// The example's dusk: 17.5 h at its latitude, exposed at -11 stops.
const HOUR: f32 = 17.5;
const LATITUDE: f32 = 0.7;
const STOPS: f32 = -11.;
/// The lamp bulbs' emitted radiance, cd/m².
const GLOW: [f32; 3] = [60_000., 40_000., 18_000.];

struct Geometry {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}
fn vertex(position: Vec3, normal: Vec3, uv: [f32; 2]) -> Vertex {
    Vertex {
        position: position.to_array(),
        normal: normal.to_array(),
        uv,
        color: [1.; 4],
    }
}
/// A unit cube centred on the origin, with flat faces.
fn cube() -> Geometry {
    let mut g = Geometry {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for (normal, u, v) in [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ] {
        let base = g.vertices.len() as u32;
        for (s, t) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
            let p = 0.5 * (normal + s * u + t * v);
            g.vertices
                .push(vertex(p, normal, [0.5 + 0.5 * s, 0.5 - 0.5 * t]));
        }
        g.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    g
}
/// A unit-radius UV sphere with smooth normals.
fn sphere(segments: u32, rings: u32) -> Geometry {
    let mut g = Geometry {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for r in 0..=rings {
        let polar = PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let azimuth = 2. * PI * s as f32 / segments as f32;
            let n = Vec3::new(
                polar.sin() * azimuth.cos(),
                polar.cos(),
                polar.sin() * azimuth.sin(),
            );
            g.vertices.push(vertex(
                n,
                n,
                [s as f32 / segments as f32, r as f32 / rings as f32],
            ));
        }
    }
    let row = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let a = r * row + s;
            let b = a + row;
            g.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    g
}
/// A square in the XZ plane facing +Y, `size` metres across, `cells` quads per side.
fn plane(size: f32, cells: u32) -> Geometry {
    let mut g = Geometry {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for z in 0..=cells {
        for x in 0..=cells {
            let (u, v) = (x as f32 / cells as f32, z as f32 / cells as f32);
            let p = Vec3::new((u - 0.5) * size, 0., (v - 0.5) * size);
            g.vertices.push(vertex(p, Vec3::Y, [u * size, v * size]));
        }
    }
    let row = cells + 1;
    for z in 0..cells {
        for x in 0..cells {
            let a = z * row + x;
            g.indices
                .extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    g
}
/// The bush: a 1.8 by 1.5 m card standing on y = 0, cut to the example's leaf mask.
fn bush() -> Result<Geometry> {
    let size = 256u32;
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    let leaves: Vec<[f32; 5]> = (0..90)
        .map(|_| {
            let (x, y) = (0.12 + 0.76 * next(), 0.08 + 0.85 * next());
            [
                x,
                y,
                0.035 + 0.035 * next(),
                0.015 + 0.012 * next(),
                PI * next(),
            ]
        })
        .filter(|l| ((l[0] - 0.5) / 0.45).powi(2) + ((l[1] - 0.55) / 0.5).powi(2) < 1.)
        .collect();
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = [
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            ];
            let inside = leaves.iter().any(|&[cx, cy, a, b, angle]| {
                let (dx, dy) = (p[0] - cx, p[1] - cy);
                let (u, v) = (
                    dx * angle.cos() + dy * angle.sin(),
                    -dx * angle.sin() + dy * angle.cos(),
                );
                (u / a).powi(2) + (v / b).powi(2) < 1.
            });
            rgba.extend_from_slice(&[255, 255, 255, if inside { 255 } else { 0 }]);
        }
    }
    let card: Vec<Vertex> = [(-0.5, 0.), (0.5, 0.), (0.5, 1.), (-0.5, 1.)]
        .iter()
        .map(|&(x, y): &(f32, f32)| {
            vertex(Vec3::new(x * 1.8, y * 1.5, 0.), Vec3::Z, [x + 0.5, 1. - y])
        })
        .collect();
    let cut = cutout(
        &MeshData {
            vertices: &card,
            indices: &[0, 1, 2, 0, 2, 3],
        },
        &AlphaMask::from_rgba8(size, size, &rgba),
        1.,
        0.5,
        64,
    )?;
    Ok(Geometry {
        vertices: cut.vertices,
        indices: cut.indices,
    })
}

/// Which of the scene's programs a material names.
#[derive(Clone, Copy, PartialEq)]
enum Shader {
    Game,
    Toon,
}
/// One spawned object: its mesh, material factors, program and transform.
struct Spawn {
    mesh: usize,
    base: [f32; 3],
    metallic: f32,
    roughness: f32,
    thin: bool,
    shader: Shader,
    glow: bool,
    transform: Mat4,
}

/// The example's first frame (`t` = 0, no quarry, no Fox): meshes, the opaque objects
/// in spawn order, the glass pane, the hero's two spheres and the lamp bulbs.
struct Scene {
    meshes: Vec<Geometry>,
    opaque: Vec<Spawn>,
    pane: Mat4,
    /// World centre and radius of the hero's body and head, and of the two bulbs.
    hero: [(Vec3, f32); 2],
    bulbs: [(Vec3, f32); 2],
    lighting: Lighting,
    shadows: SunShadows,
    camera: Camera,
    eye: Vec3,
}
impl Scene {
    fn new() -> Result<Self> {
        let meshes = vec![cube(), sphere(48, 24), plane(60., 8), bush()?];
        let (cube, sphere, plane, bush) = (0, 1, 2, 3);
        let mut opaque = Vec::new();
        let mut spawn = |mesh, material: ([f32; 3], f32, f32), shader, transform| {
            opaque.push(Spawn {
                mesh,
                base: material.0,
                metallic: material.1,
                roughness: material.2,
                thin: false,
                shader,
                glow: false,
                transform,
            });
        };
        let stone = ([0.62, 0.6, 0.56], 0., 0.9);
        let wood = ([0.56, 0.36, 0.18], 0., 0.75);
        let red = ([0.8, 0.09, 0.06], 0., 0.6);
        let gold = ([1., 0.78, 0.34], 1., 0.25);
        let paint = ([0.1, 0.3, 0.85], 0., 0.35);
        let chrome = ([0.95, 0.95, 0.97], 1., 0.02);
        let iron = ([0.08, 0.08, 0.09], 0.9, 0.45);
        let ground = ([0.34, 0.37, 0.28], 0., 0.95);
        // `MaterialDesc::default()`'s factors under the toon hero and the bulbs.
        let toon = ([0.95, 0.55, 0.2], 0., 0.5);
        let bulb = ([1., 0.9, 0.7], 0., 0.5);
        let leaves = ([0.22, 0.42, 0.12], 0., 0.7);
        let at = Vec3::new;
        let boxed = |c: Vec3, s: Vec3| Mat4::from_translation(c) * Mat4::from_scale(s);
        spawn(plane, ground, Shader::Game, Mat4::IDENTITY);
        for x in [-2.2, 2.2] {
            spawn(
                cube,
                stone,
                Shader::Game,
                boxed(at(x, 2., -3.), at(0.8, 4., 0.8)),
            );
        }
        spawn(
            cube,
            stone,
            Shader::Game,
            boxed(at(0., 4.4, -3.), at(5.2, 0.8, 1.)),
        );
        spawn(
            cube,
            red,
            Shader::Game,
            boxed(at(-5.5, 1.5, 0.), at(0.3, 3., 6.)),
        );
        for (x, y, z, turn) in [
            (4., 0.5, 1., 0.1),
            (5.1, 0.5, 1.3, -0.25),
            (4.5, 1.5, 1.1, 0.4),
            (4.2, 0.5, 2.3, 0.),
        ] {
            spawn(
                cube,
                wood,
                Shader::Game,
                Mat4::from_translation(at(x, y, z)) * Mat4::from_rotation_y(turn),
            );
        }
        spawn(
            sphere,
            gold,
            Shader::Game,
            boxed(at(1.8, 1., 3.2), Vec3::ONE),
        );
        for (x, z) in [(-4., -2.2), (3.6, -3.8), (6.2, -0.6)] {
            for k in 0..3 {
                spawn(
                    bush,
                    leaves,
                    Shader::Game,
                    Mat4::from_translation(at(x, 0., z))
                        * Mat4::from_rotation_y(k as f32 * PI / 3.),
                );
            }
        }
        let brazier = at(-3.8, 0., 2.);
        spawn(
            cube,
            stone,
            Shader::Game,
            boxed(brazier + at(0., 0.3, 0.), Vec3::splat(0.6)),
        );
        let arch_lamp = at(0., 3.55, -3.);
        spawn(
            sphere,
            bulb,
            Shader::Game,
            boxed(arch_lamp, Vec3::splat(0.12)),
        );
        spawn(
            cube,
            iron,
            Shader::Game,
            boxed(arch_lamp + at(0., 0.3, 0.), at(0.03, 0.5, 0.03)),
        );
        let lamp_offset = at(0.22, 0., -0.22);
        let post_top = at(3., 3.2, 3.);
        let post_lamp = post_top + lamp_offset;
        spawn(
            cube,
            iron,
            Shader::Game,
            boxed(at(3., 1.6, 3.), at(0.12, 3.2, 0.12)),
        );
        spawn(
            cube,
            iron,
            Shader::Game,
            boxed(
                post_top + 0.5 * lamp_offset + at(0., 0.12, 0.),
                at(0.4 + lamp_offset.x.abs(), 0.08, 0.4 + lamp_offset.z.abs()),
            ),
        );
        spawn(
            sphere,
            bulb,
            Shader::Game,
            boxed(post_lamp, Vec3::splat(0.1)),
        );
        spawn(
            sphere,
            chrome,
            Shader::Game,
            boxed(at(-1.2, 0.8, 5.6), Vec3::splat(0.8)),
        );
        spawn(
            cube,
            paint,
            Shader::Game,
            Mat4::from_translation(at(-2.5, 0.6, 3.)),
        );
        // The hero at t = 0: on the ground at (0, 0, 2), a body and a head.
        let centre = at(0., 0.55, 2.);
        let hero = [(centre, 0.55), (centre + at(0., 0.8, 0.), 0.35)];
        for (c, r) in hero {
            spawn(sphere, toon, Shader::Toon, boxed(c, Vec3::splat(r)));
        }
        for object in &mut opaque {
            object.thin = object.mesh == bush;
            object.glow = object.base == bulb.0;
        }
        // The sun as `engine::settings::lighting` hands it over, then the lanterns at t = 0.
        let sun = Sun::at_hour(HOUR, LATITUDE).context("the sun is down")?;
        let flame = brazier + at(0., 0.65, 0.);
        let flicker = 0.85 + 0.05 * 1f32.sin();
        let lighting = Lighting {
            lights: vec![
                Light::Directional {
                    travel: sun.travel,
                    lux: sun.illuminance,
                },
                Light::Point {
                    position: arch_lamp - at(0., 0.25, 0.),
                    candela: [30_000., 21_000., 11_000.],
                    range: Some(14.),
                    source_radius: 0.12,
                },
                Light::Spot {
                    position: post_lamp - at(0., 0.15, 0.),
                    travel: (at(4.6, 0., 1.5) - post_lamp).normalize(),
                    candela: [26_000., 25_000., 23_000.],
                    range: Some(12.),
                    inner: 0.3,
                    outer: 0.6,
                    source_radius: 0.1,
                },
                Light::Point {
                    position: flame + at(0., 0.3, 0.),
                    candela: [24_000., 12_000., 4_000.].map(|c| c * flicker),
                    range: Some(10.),
                    source_radius: 0.25,
                },
            ],
            environment: [0.; 3],
            exposure_stops: STOPS,
        };
        // The fly path's first pose.
        let eye = at(14. * 0.6f32.sin(), 5.5, 14. * 0.6f32.cos());
        Ok(Self {
            meshes,
            opaque,
            pane: Mat4::from_translation(at(5.4, 0.8, 3.6))
                * Mat4::from_rotation_y(0.8)
                * Mat4::from_scale(at(2.2, 1.6, 0.04)),
            hero,
            bulbs: [(arch_lamp, 0.12), (post_lamp, 0.1)],
            lighting,
            shadows: SunShadows {
                split_ends: vec![4., 12., 40.],
                disk_angle: sun.angular_radius,
            },
            camera: Camera::look_at(
                eye,
                centre.lerp(Vec3::Y, 0.5),
                Vec3::Y,
                55f32.to_radians(),
                0.05,
                500.,
            )?,
            eye,
        })
    }
    fn objects(&self) -> Vec<ForwardObject<'_>> {
        self.opaque
            .iter()
            .map(|o| ForwardObject {
                vertices: &self.meshes[o.mesh].vertices,
                indices: &self.meshes[o.mesh].indices,
                transform: o.transform,
                material: MaterialDescriptor {
                    base_color: [o.base[0], o.base[1], o.base[2], 1.],
                    metallic: o.metallic,
                    roughness: o.roughness,
                    double_sided: true,
                    thin: o.thin,
                    ..Default::default()
                },
            })
            .collect()
    }
    /// `engine::settings::atmosphere` for the default sky and the scene's settings.
    fn atmosphere(volumetric_fog: Option<VolumetricFog>) -> Atmosphere {
        Atmosphere {
            sky: true,
            sun_disk: true,
            aerial_perspective: true,
            sun_transmittance: true,
            environment: true,
            medium: Default::default(),
            height_fog: Some(HeightFog::default()),
            volumetric_fog,
        }
    }
    /// The pixel a world point projects to, and how many pixels one metre covers there.
    fn project(&self, point: Vec3) -> ([f32; 2], f32) {
        let ndc = self.camera.matrix(SIZE[0], SIZE[1]).project_point3(point);
        let scale =
            SIZE[1] as f32 * 0.5 / ((point - self.eye).length() * 27.5f32.to_radians().tan());
        (
            [
                (ndc.x + 1.) * 0.5 * SIZE[0] as f32,
                (1. - ndc.y) * 0.5 * SIZE[1] as f32,
            ],
            scale,
        )
    }
    /// Whether pixel `i`'s centre lies within a sphere's outline grown by half its
    /// radius and two pixels: every pixel that can show the sphere is inside.
    fn near(&self, spheres: &[(Vec3, f32)], i: usize) -> bool {
        let centre = [
            (i as u32 % SIZE[0]) as f32 + 0.5,
            (i as u32 / SIZE[0]) as f32 + 0.5,
        ];
        spheres.iter().any(|&(c, r)| {
            let (at, scale) = self.project(c);
            (centre[0] - at[0]).hypot(centre[1] - at[1]) < 1.5 * r * scale + 2.
        })
    }
}

fn rgb(hdr: &[u8]) -> Vec<[f32; 3]> {
    hdr.as_chunks::<8>()
        .0
        .iter()
        .map(|p| {
            std::array::from_fn(|c| {
                f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32()
            })
        })
        .collect()
}

/// The game scene's 29 opaque objects, its sun through the atmosphere and its three
/// lanterns, from the fly path's first pose.
///
/// With direct light only (no radiance cache, reflections or history, so one object's
/// shading cannot reach another's pixels):
/// - Every material on its own program, as the scene has them (the props on `game`, the
///   hero on `toon` with its style): the frame shades two programs by their lists.
///   Against the frame where the hero is given the `game` program, which shades the
///   whole region with one pipeline, only pixels near the hero differ, and most of the
///   hero's do. So do the sky's pixels above the horizon agree, which the first
///   program's list writes: they are lit, and bit for bit the one-program frame's.
/// - The bulbs' `glow`: against the frame without it, only pixels at the two bulbs
///   differ, and the arch bulb's centre gains the exposed glow less what 17 m of haze
///   takes.
///
/// Then the scene's own settings as far as the frame goes: the radiance cache, traced
/// reflections, local-light shadows, temporal reconstruction, post with the scene's
/// bloom, and the glass pane. Four frames run and the last is written to `target/` as
/// a PNG.
///
/// Refused by the frame: the scene's volumetric fog. Not expressible in the frame's
/// description at all: the two particle emitters, the two decals, the skinned Fox, and
/// the pane's traced reflection.
#[test]
fn the_game_scenes_opaque_objects_are_shaded_by_their_own_programs() -> Result<()> {
    let path = root("game-scene");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let scene = Scene::new()?;
    let objects = scene.objects();
    let style = [
        ("bands", Value::F32(3.)),
        ("shadow_tint", Value::Vec3([0.22, 0.14, 0.38])),
        ("rim_color", Value::Vec3([0.35, 0.5, 0.7])),
    ];
    let glow = [("glow", Value::Vec3(GLOW))];
    // Each object's program: the hero on toon or on game, the bulbs glowing or not.
    let programs = |toon_hero: bool, glowing: bool| -> Vec<Option<ObjectProgram<'_>>> {
        scene
            .opaque
            .iter()
            .map(|o| {
                Some(match (o.shader, toon_hero) {
                    (Shader::Toon, true) => ObjectProgram {
                        source: TOON,
                        parameters: &style,
                    },
                    _ => ObjectProgram {
                        source: GAME,
                        parameters: if o.glow && glowing { &glow } else { &[] },
                    },
                })
            })
            .collect()
    };
    let direct = |gpu: &mut GpuDevice, programs: &[Option<ObjectProgram<'_>>]| {
        let images = render(
            gpu,
            &cache,
            &ForwardScene {
                objects: &objects,
                lighting: &scene.lighting,
                camera: scene.camera,
                size: SIZE,
                atmosphere: Scene::atmosphere(None),
                sun_shadows: Some(&scene.shadows),
                primary: Primary::TracedInteractive,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &[],
                programs: Some(ScenePrograms { objects: programs }),
            },
        )?;
        Ok::<_, anyhow::Error>(rgb(&images.hdr))
    };
    let (own, one, dark) = (
        direct(&mut gpu, &programs(true, true))?,
        direct(&mut gpu, &programs(false, true))?,
        direct(&mut gpu, &programs(true, false))?,
    );
    ensure!(
        own.iter().flatten().all(|v| v.is_finite()),
        "the frame holds a value that is not finite"
    );
    // Rows whose centre rays all leave above the horizon: the 60 m ground ends below it.
    let inverse = scene.camera.matrix(SIZE[0], SIZE[1]).inverse();
    let rises = |y: u32| {
        let far = inverse.project_point3(Vec3::new(
            0.,
            1. - (y as f32 + 0.5) / SIZE[1] as f32 * 2.,
            0.5,
        ));
        (far - scene.eye).normalize().y > 0.02
    };
    let (mut sky, mut heroes, mut bulbs) = (0, 0, 0);
    for (i, ((own, one), dark)) in own.iter().zip(&one).zip(&dark).enumerate() {
        let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
        if rises(y) {
            sky += 1;
            ensure!(
                own == one && own.iter().all(|v| *v > 0.),
                "sky at ({x}, {y}) is {own:?} by lists and {one:?} by one program"
            );
        }
        if own != one {
            heroes += 1;
            ensure!(
                scene.near(&scene.hero, i),
                "the hero's program changed ({x}, {y}), away from the hero: {own:?} against {one:?}"
            );
        }
        if own != dark {
            bulbs += 1;
            ensure!(
                scene.near(&scene.bulbs, i),
                "the bulbs' glow changed ({x}, {y}), away from the bulbs: {own:?} against {dark:?}"
            );
        }
    }
    // The hero's outlines hold about 314 and 127 pixels at this size, and the gold
    // sphere stands in front of most of the body.
    ensure!(
        sky > 5000 && heroes > 120 && bulbs >= 4,
        "the fixture lost its regions: {sky} sky pixels, {heroes} changed by the hero's program, {bulbs} by the glow"
    );
    let (at, _) = scene.project(scene.bulbs[0].0);
    let centre = at[1] as usize * SIZE[0] as usize + at[0] as usize;
    let exposure = 2f32.powf(STOPS);
    let mut kept = 1f32;
    for c in 0..3 {
        let (gained, emitted) = (own[centre][c] - dark[centre][c], GLOW[c] * exposure);
        kept = kept.min(gained / emitted);
        ensure!(
            gained <= emitted * 1.001 && gained >= emitted * 0.99,
            "the arch bulb's centre gained {gained} in channel {c}; its glow exposes to {emitted}"
        );
    }

    // The scene's own settings, as far as the frame goes.
    let pane = [ForwardObject {
        vertices: &scene.meshes[0].vertices,
        indices: &scene.meshes[0].indices,
        transform: scene.pane,
        material: MaterialDescriptor {
            base_color: [0.86, 0.95, 0.93, 1.],
            metallic: 0.,
            roughness: 0.04,
            double_sided: true,
            transparency: Some(Transparency::default()),
            ..Default::default()
        },
    }];
    let all = programs(true, true);
    let full = |volumetric_fog| ForwardScene {
        objects: &objects,
        lighting: &scene.lighting,
        camera: scene.camera,
        size: SIZE,
        atmosphere: Scene::atmosphere(volumetric_fog),
        sun_shadows: Some(&scene.shadows),
        primary: Primary::TracedInteractive,
        post: Some(PostSettings {
            bloom: 0.06,
            ..Default::default()
        }),
        indirect: Some(Indirect {
            settings: IndirectSettings::default(),
            read_table: false,
        }),
        reconstruction: Some(Reconstruct {
            settings: Reconstruction::default(),
            read_history: false,
        }),
        reflections: Some(ReflectionSettings::default()),
        textures: None,
        local_shadows: Some(LocalShadows::default()),
        transparent: &pane,
        programs: Some(ScenePrograms { objects: &all }),
    };
    let mut frames = Frames::default();
    let mut last = None;
    for _ in 0..4 {
        last = Some(frames.render(&mut gpu, &cache, &full(None))?);
    }
    // The scene's haze: 0.0015 per metre over the default froxel volume.
    let fog = frames.render(
        &mut gpu,
        &cache,
        &full(Some(VolumetricFog {
            extra_density: 0.0015,
            ..Default::default()
        })),
    );
    frames.retire(&mut gpu)?;
    ensure!(fog.is_err(), "the frame accepted volumetric fog");
    let images = last.context("no frame")?;
    ensure!(
        rgb(&images.hdr).iter().flatten().all(|v| v.is_finite()),
        "the full frame holds a value that is not finite"
    );
    let display = images.display.context("post did not run")?;
    let lit = display
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] != [0; 3])
        .count();
    ensure!(
        lit * 10 > display.len() / 4 * 9,
        "the displayed frame is mostly black: {lit} lit pixels"
    );
    let file = path.join("game-scene-opaque.png");
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(&file)?),
        SIZE[0],
        SIZE[1],
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&display)?;
    eprintln!(
        "game scene: {} draws, 2 programs; {sky} sky pixels exact, {heroes} pixels changed by the hero's program and {bulbs} by the glow, all in place; the arch bulb keeps {kept:.4} of its glow; frame 4 at {}",
        objects.len(),
        file.display()
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
