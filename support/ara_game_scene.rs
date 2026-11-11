//! TEMPORARY: the game scene (`examples/game_scene.rs`) as calls on `engine::Scene`, for
//! the ara-gpu backend's test and comparison tool.
//!
//! This is a second description of the scene, copied from the example: its construction
//! at the defaults (no quarry, no Fox), its per-frame animation, its settings and its
//! geometry builders. It goes when the example's own construction and animation are
//! functions both layers call (docs/design-engine-backend.md on main, the PR after
//! N-VSM1); the test and the tool then include that file instead.
use renderlab::{
    Camera, Decal, DecalBlend, DecalTexture, DecalTextureKind, DecalTextures, EmitterDescriptor,
    Error, Light, Mat4, MeshData, PostSettings, Transparency, Value, Vec3, Vertex, VolumetricFog,
    cutout::{AlphaMask, cutout},
    engine::{Exposure, MaterialDesc, MaterialId, ObjectId, Scene, Settings, Sun},
};
use std::f32::consts::PI;

const GAME: &str = include_str!("../examples/game_scene.wgsl");
const TOON: &str = include_str!("../examples/game_toon.wgsl");
/// The example's dusk: 17.5 h at its latitude, exposed at -11 stops, 30 frames a second.
const HOUR: f32 = 17.5;
const LATITUDE: f32 = 0.7;
pub const STOPS: f32 = -11.;
const FPS: f32 = 30.;
/// The lamp bulbs' emitted radiance, cd/m².
pub const GLOW: [f32; 3] = [60_000., 40_000., 18_000.];

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
fn bush() -> Result<Geometry, Error> {
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

/// What `animate` and the test need of the built scene.
pub struct Built {
    pub spinner: ObjectId,
    pub hero: [ObjectId; 2],
    pub bulb: MaterialId,
    pub arch_lamp: Vec3,
    pub post_lamp: Vec3,
    pub flame: Vec3,
}

/// Which parts of the example's scene to build.
#[derive(Clone, Copy)]
pub struct Options {
    /// The hero on the toon program with its style; false gives it the props' program.
    pub toon_hero: bool,
    /// The fire and smoke emitters over the brazier.
    pub particles: bool,
    /// The two ground decals.
    pub decals: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            toon_hero: true,
            particles: true,
            decals: true,
        }
    }
}

/// TEMPORARY copy of the example's scene construction at its defaults (no quarry, no
/// Fox, glass on): programs, materials, meshes, objects, the sun, the two emitters and
/// the two decals. The decals' textures are flat stand-ins for the example's emblem and
/// puddle.
pub fn build<S: Scene>(scene: &mut S, options: Options) -> Result<Built, Error> {
    let toon_hero = options.toon_hero;
    let game = scene.create_program("game", GAME)?;
    let toon_program = scene.create_program("toon", TOON)?;
    let material = |scene: &mut S, color: [f32; 3], metallic: f32, roughness: f32| {
        scene.create_material(&MaterialDesc {
            base_color: [color[0], color[1], color[2], 1.],
            metallic,
            roughness,
            program: Some(game),
            ..Default::default()
        })
    };
    let stone = material(scene, [0.62, 0.6, 0.56], 0., 0.9)?;
    let wood = material(scene, [0.56, 0.36, 0.18], 0., 0.75)?;
    let red = material(scene, [0.8, 0.09, 0.06], 0., 0.6)?;
    let gold = material(scene, [1., 0.78, 0.34], 1., 0.25)?;
    let paint = material(scene, [0.1, 0.3, 0.85], 0., 0.35)?;
    let chrome = material(scene, [0.95, 0.95, 0.97], 1., 0.02)?;
    let iron = material(scene, [0.08, 0.08, 0.09], 0.9, 0.45)?;
    let ground = material(scene, [0.34, 0.37, 0.28], 0., 0.95)?;
    let style = [
        ("bands", Value::F32(3.)),
        ("shadow_tint", Value::Vec3([0.22, 0.14, 0.38])),
        ("rim_color", Value::Vec3([0.35, 0.5, 0.7])),
    ];
    let toon = scene.create_material(&MaterialDesc {
        base_color: [0.95, 0.55, 0.2, 1.],
        program: Some(if toon_hero { toon_program } else { game }),
        parameters: if toon_hero { &style } else { &[] },
        ..Default::default()
    })?;
    let glass = scene.create_material(&MaterialDesc {
        base_color: [0.86, 0.95, 0.93, 1.],
        roughness: 0.04,
        transparency: Some(Transparency::default()),
        ..Default::default()
    })?;
    let leaves = scene.create_material(&MaterialDesc {
        base_color: [0.22, 0.42, 0.12, 1.],
        roughness: 0.7,
        thin: true,
        program: Some(game),
        ..Default::default()
    })?;
    let bulb = scene.create_material(&MaterialDesc {
        base_color: [1., 0.9, 0.7, 1.],
        program: Some(game),
        parameters: &[("glow", Value::Vec3(GLOW))],
        ..Default::default()
    })?;
    let mesh = |scene: &mut S, g: &Geometry| {
        scene.create_mesh(&MeshData {
            vertices: &g.vertices,
            indices: &g.indices,
        })
    };
    let cube = mesh(scene, &cube())?;
    let sphere = mesh(scene, &sphere(48, 24))?;
    let bush = mesh(scene, &bush()?)?;
    let plane = mesh(scene, &plane(60., 8))?;
    let at = Vec3::new;
    let boxed = |c: Vec3, s: Vec3| Mat4::from_translation(c) * Mat4::from_scale(s);
    scene.spawn(plane, ground, Mat4::IDENTITY)?;
    for x in [-2.2, 2.2] {
        scene.spawn(cube, stone, boxed(at(x, 2., -3.), at(0.8, 4., 0.8)))?;
    }
    scene.spawn(cube, stone, boxed(at(0., 4.4, -3.), at(5.2, 0.8, 1.)))?;
    scene.spawn(cube, red, boxed(at(-5.5, 1.5, 0.), at(0.3, 3., 6.)))?;
    for (x, y, z, turn) in [
        (4., 0.5, 1., 0.1),
        (5.1, 0.5, 1.3, -0.25),
        (4.5, 1.5, 1.1, 0.4),
        (4.2, 0.5, 2.3, 0.),
    ] {
        scene.spawn(
            cube,
            wood,
            Mat4::from_translation(at(x, y, z)) * Mat4::from_rotation_y(turn),
        )?;
    }
    scene.spawn(sphere, gold, boxed(at(1.8, 1., 3.2), Vec3::ONE))?;
    scene.spawn(
        cube,
        glass,
        Mat4::from_translation(at(5.4, 0.8, 3.6))
            * Mat4::from_rotation_y(0.8)
            * Mat4::from_scale(at(2.2, 1.6, 0.04)),
    )?;
    for (x, z) in [(-4., -2.2), (3.6, -3.8), (6.2, -0.6)] {
        for k in 0..3 {
            scene.spawn(
                bush,
                leaves,
                Mat4::from_translation(at(x, 0., z)) * Mat4::from_rotation_y(k as f32 * PI / 3.),
            )?;
        }
    }
    let brazier = at(-3.8, 0., 2.);
    scene.spawn(
        cube,
        stone,
        boxed(brazier + at(0., 0.3, 0.), Vec3::splat(0.6)),
    )?;
    let arch_lamp = at(0., 3.55, -3.);
    scene.spawn(sphere, bulb, boxed(arch_lamp, Vec3::splat(0.12)))?;
    scene.spawn(
        cube,
        iron,
        boxed(arch_lamp + at(0., 0.3, 0.), at(0.03, 0.5, 0.03)),
    )?;
    let lamp_offset = at(0.22, 0., -0.22);
    let post_top = at(3., 3.2, 3.);
    let post_lamp = post_top + lamp_offset;
    scene.spawn(cube, iron, boxed(at(3., 1.6, 3.), at(0.12, 3.2, 0.12)))?;
    scene.spawn(
        cube,
        iron,
        boxed(
            post_top + 0.5 * lamp_offset + at(0., 0.12, 0.),
            at(0.4 + lamp_offset.x.abs(), 0.08, 0.4 + lamp_offset.z.abs()),
        ),
    )?;
    scene.spawn(sphere, bulb, boxed(post_lamp, Vec3::splat(0.1)))?;
    scene.spawn(sphere, chrome, boxed(at(-1.2, 0.8, 5.6), Vec3::splat(0.8)))?;
    let spinner = scene.spawn(cube, paint, Mat4::from_translation(at(-2.5, 0.6, 3.)))?;
    let hero = [
        scene.spawn(sphere, toon, Mat4::IDENTITY)?,
        scene.spawn(sphere, toon, Mat4::IDENTITY)?,
    ];
    scene.set_sun(Sun::at_hour(HOUR, LATITUDE))?;
    // Fire and smoke over the brazier.
    scene.set_particle_step(1. / FPS)?;
    let flame = brazier + at(0., 0.65, 0.);
    if options.particles {
        scene.create_emitter(
            &EmitterDescriptor {
                position: flame,
                spawn_radius: 0.18,
                spawn_rate: 140.,
                lifetime: [0.45, 0.9],
                velocity: at(0., 0.9, 0.),
                velocity_spread: 0.35,
                acceleration: at(0., 1.6, 0.),
                drag: 0.6,
                size: [0.32, 0.08],
                color: [[1., 0.55, 0.15, 0.85], [0.4, 0.1, 0.02, 0.]],
                emission: [[9000., 3800., 900.], [1200., 250., 40.]],
                fade: [0.1, 0.4],
                soft_distance: 0.15,
                capacity: 128,
                seed: 7,
                ..Default::default()
            },
            None,
        )?;
        scene.create_emitter(
            &EmitterDescriptor {
                position: flame + at(0., 0.6, 0.),
                spawn_radius: 0.15,
                spawn_rate: 24.,
                lifetime: [2., 3.],
                velocity: at(0.15, 0.6, 0.),
                velocity_spread: 0.15,
                acceleration: at(0.25, 0.2, 0.),
                drag: 0.3,
                size: [0.35, 1.2],
                color: [[0.3, 0.3, 0.32, 0.35], [0.5, 0.5, 0.52, 0.]],
                fade: [0.2, 0.6],
                soft_distance: 0.3,
                capacity: 80,
                seed: 11,
                ..Default::default()
            },
            None,
        )?;
    }
    let ground_decal = |center: Vec3, width: f32, depth: f32, turn: f32| {
        Mat4::from_translation(center)
            * Mat4::from_rotation_y(turn)
            * Mat4::from_rotation_x(-PI / 2.)
            * Mat4::from_scale(at(width, width * depth, 0.6))
    };
    let flat = || DecalTexture::from_rgba8(DecalTextureKind::BaseColorSrgb, [2; 2], &[255; 16]);
    if options.decals {
        scene.set_decals(&[
            Decal {
                transform: ground_decal(at(0., 0., -0.6), 2.6, 1., 0.3),
                textures: DecalTextures {
                    base_color: Some(flat()?),
                    ..Default::default()
                },
                channels: [1., 0., 0., 0.],
                ..Default::default()
            },
            Decal {
                transform: ground_decal(at(1.5, 0., 4.6), 4., 0.7, -0.4),
                textures: DecalTextures {
                    base_color: Some(flat()?),
                    ..Default::default()
                },
                base_color: [0.55, 0.57, 0.62, 1.],
                roughness: 0.04,
                channels: [1., 1., 0., 0.],
                blend: DecalBlend::Multiply,
                ..Default::default()
            },
        ])?;
    }
    Ok(Built {
        spinner,
        hero,
        bulb,
        arch_lamp,
        post_lamp,
        flame,
    })
}

/// The hero's body and head at time `t`: world centre and radius.
pub fn hero(t: f32) -> [(Vec3, f32); 2] {
    let walk = 0.35 * t;
    let ground = Vec3::new(3. * walk.sin(), 0., -1. + 3. * walk.cos());
    let centre = ground + Vec3::new(0., 0.55 + 0.12 * (6. * t).sin().abs(), 0.);
    [(centre, 0.55), (centre + Vec3::new(0., 0.8, 0.), 0.35)]
}
/// The fly path's eye at time `t`.
pub fn eye(t: f32) -> Vec3 {
    let orbit = 0.6 + 0.25 * t;
    let radius = 14. - 4. * (t / 2.).min(1.);
    Vec3::new(
        radius * orbit.sin(),
        5.5 - 2. * (t / 2.).min(1.),
        radius * orbit.cos(),
    )
}
/// TEMPORARY copy of the example's per-frame animation: the hero's walk, the lanterns'
/// flicker, the spinning crate and the fly path. Returns the frame's camera.
pub fn animate<S: Scene>(scene: &mut S, built: &Built, frame: u32) -> Result<Camera, Error> {
    let t = frame as f32 / FPS;
    let spheres = hero(t);
    for (object, (centre, radius)) in built.hero.iter().zip(spheres) {
        scene.set_transform(
            *object,
            Mat4::from_translation(centre) * Mat4::from_scale(Vec3::splat(radius)),
        )?;
    }
    let flicker = 0.85 + 0.1 * (23. * t).sin() + 0.05 * (37. * t + 1.).sin();
    scene.set_lights(&[
        Light::Point {
            position: built.arch_lamp - Vec3::new(0., 0.25, 0.),
            candela: [30_000., 21_000., 11_000.],
            range: Some(14.),
            source_radius: 0.12,
        },
        Light::Spot {
            position: built.post_lamp - Vec3::new(0., 0.15, 0.),
            travel: (Vec3::new(4.6, 0., 1.5) - built.post_lamp).normalize(),
            candela: [26_000., 25_000., 23_000.],
            range: Some(12.),
            inner: 0.3,
            outer: 0.6,
            source_radius: 0.1,
        },
        Light::Point {
            position: built.flame + Vec3::new(0., 0.3, 0.),
            candela: [24_000., 12_000., 4_000.].map(|c| c * flicker),
            range: Some(10.),
            source_radius: 0.25,
        },
    ])?;
    scene.set_transform(
        built.spinner,
        Mat4::from_translation(Vec3::new(-2.5, 0.6 + 0.3 * (2. * t).sin(), 3.))
            * Mat4::from_rotation_y(1.5 * t)
            * Mat4::from_rotation_x(0.6 * t),
    )?;
    Camera::look_at(
        eye(t),
        spheres[0].0.lerp(Vec3::Y, 0.5),
        Vec3::Y,
        55f32.to_radians(),
        0.05,
        500.,
    )
}

/// The example's settings: its haze, its bloom, its exposure, everything else default.
pub fn settings() -> Settings {
    Settings {
        volumetric_fog: Some(VolumetricFog {
            extra_density: 0.0015,
            ..Default::default()
        }),
        post: PostSettings {
            bloom: 0.06,
            ..Default::default()
        },
        exposure: Exposure::Manual { stops: STOPS },
        ..Default::default()
    }
}
