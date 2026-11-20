//! Real-size fixtures for the lighting cache at real triangle counts, with a walking camera:
//! shade's software traversal (`RENDERLAB_GI_RS`) and gicut's pricing of a gather with ray
//! hardware. No game assets; every scene is generated here.
//!
//! Both scenes sit in a closed furnace room. Every surface, the room, the scene and two skinned
//! columns, has albedo 0.5 and emits 0.5 cd/m^2, so the exact radiance is 1 everywhere, as in
//! the furnace box. A sun shines on the room from outside. The room occludes it completely, so
//! the answer stays 1, but the cache still traces its light rays (any-hit) through the scene.
//! The sun is 1 lux by default (`LIGHTBENCH_CITY_SUN_LUX`). At 20,000 lux the closed room read
//! 23x (`city`) and 37x (`terrain`) the answer on 20 Nov: sunlight reaching the inside of the room's
//! thin walls through the cache, a leak worth its own check, kept out of the default's energy.
//!
//! - `city`: about 3,000 boxes (12 triangles each, one mesh, instanced) and 150 rocks
//!   (320-triangle icospheres, instanced): about 84k triangles in 3,150 placements, where the
//!   top level and the instance boxes dominate.
//! - `terrain`: one 320 x 320 heightfield (204,800 triangles in one mesh), where the in-mesh BVH
//!   and the triangle loads dominate.
//!
//! The camera settles still for `LIGHTBENCH_CITY_SETTLE` frames (default 32), then walks 0.05 m
//! a frame (`LIGHTBENCH_CITY_STEP`) for `LIGHTBENCH_CITY_FRAMES` frames (default 64). The
//! columns' joints swing every frame. Per walking frame the fixture reads the cache's counters
//! (`series`: `gather_rays`, `light_rays`, `aimed_rays`). Its metrics are `energy`, `err` and
//! `off5` on the last frame, `geometry_bytes` (the cache's software geometry), `triangles` and
//! `placements`. The bench's cache milliseconds cover the walking frames.
use super::furnace::FURNACE_PROGRAM;
use super::{BenchPath, Fixture, RefImage, kit};
use crate::asset::skin::Influence;
use crate::renderer::Renderer;
use crate::{
    Camera, Draw, Error, IndirectSettings, Instances, Light, Lighting, Mat4, Material, MaterialDescriptor,
    ProgramDescriptor, Reconstruction, ShadowRequest, Shadows, SkinnedMeshInstance, SoftShadows, Value, Vec3,
    Vertex, ViewMode,
};

const SIZE: [u32; 2] = [320, 180];
const FOV: f32 = 1.1;
/// The room's half extents: 48 x 12 x 48 m.
const ROOM: Vec3 = Vec3::new(24.0, 6.0, 24.0);
const GLOW: [f32; 3] = [0.5; 3];
const ALBEDO: [f32; 3] = [0.5; 3];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Boxes,
    Terrain,
}
struct Skin {
    skin: SkinnedMeshInstance,
    instances: Instances,
    material: Material,
}
struct Loaded {
    _program: crate::Program,
    parts: Vec<kit::Part>,
    skins: Vec<Skin>,
    target: wgpu::Texture,
    triangles: u64,
    placements: u64,
}
pub(crate) struct City {
    name: &'static str,
    kind: Kind,
    loaded: Option<Loaded>,
    series: Vec<(String, Vec<f64>)>,
}
impl City {
    pub(crate) fn boxes() -> Self {
        Self { name: "city", kind: Kind::Boxes, loaded: None, series: vec![] }
    }
    pub(crate) fn terrain() -> Self {
        Self { name: "terrain", kind: Kind::Terrain, loaded: None, series: vec![] }
    }
}

/// A small deterministic generator (xorshift), so every process builds the same scene.
struct Random(u64);
impl Random {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// An outward unit box (half extent 0.5, base at y = 0): 12 triangles.
fn unit_box() -> (Vec<Vertex>, Vec<u32>) {
    triangles_of(&kit::box_faces(Vec3::new(0.0, 0.5, 0.0), Vec3::splat(0.5), false))
}
fn triangles_of(faces: &[([Vec3; 4], Vec3)]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    let mut indices = vec![];
    for (corners, n) in faces {
        let base = vertices.len() as u32;
        vertices.extend(corners.iter().map(|p| kit::vertex(*p, *n)));
        indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
    }
    (vertices, indices)
}
/// A unit icosphere subdivided twice: 320 triangles, flat-shaded.
fn rock() -> (Vec<Vertex>, Vec<u32>) {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut points: Vec<Vec3> = [
        (-1.0, t, 0.0), (1.0, t, 0.0), (-1.0, -t, 0.0), (1.0, -t, 0.0), (0.0, -1.0, t), (0.0, 1.0, t),
        (0.0, -1.0, -t), (0.0, 1.0, -t), (t, 0.0, -1.0), (t, 0.0, 1.0), (-t, 0.0, -1.0), (-t, 0.0, 1.0),
    ]
    .into_iter()
    .map(|(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut faces: Vec<[usize; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6],
        [7, 1, 8], [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10],
        [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..2 {
        let mut next = vec![];
        for [a, b, c] in faces {
            let mut mid = |i: usize, j: usize| {
                points.push(((points[i] + points[j]) * 0.5).normalize());
                points.len() - 1
            };
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    flat(&points, &faces)
}
/// Flat-shaded triangles: each gets its own three vertices and its face normal.
fn flat(points: &[Vec3], faces: &[[usize; 3]]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    for &[a, b, c] in faces {
        let n = (points[b] - points[a]).cross(points[c] - points[a]).normalize_or_zero();
        vertices.extend([a, b, c].map(|i| kit::vertex(points[i], n)));
    }
    let indices = (0..vertices.len() as u32).collect();
    (vertices, indices)
}
/// A 320 x 320 heightfield over the room's floor, a few metres of rolling ground.
fn terrain() -> (Vec<Vertex>, Vec<u32>) {
    const N: usize = 320;
    let extent = ROOM.x - 1.0;
    let height = |x: f32, z: f32| {
        1.2 + 0.9 * (x * 0.31).sin() * (z * 0.27).cos() + 0.35 * (x * 1.13 + z * 0.71).sin() + 0.12 * (x * 3.7).cos() * (z * 4.1).sin()
            - ROOM.y
    };
    let at = |i: usize, j: usize| {
        let x = -extent + 2.0 * extent * i as f32 / N as f32;
        let z = -extent + 2.0 * extent * j as f32 / N as f32;
        Vec3::new(x, height(x, z), z)
    };
    let mut vertices = Vec::with_capacity((N + 1) * (N + 1));
    for j in 0..=N {
        for i in 0..=N {
            let p = at(i, j);
            let dx = at((i + 1).min(N), j) - at(i.saturating_sub(1), j);
            let dz = at(i, (j + 1).min(N)) - at(i, j.saturating_sub(1));
            vertices.push(kit::vertex(p, dz.cross(dx).normalize()));
        }
    }
    let mut indices = Vec::with_capacity(N * N * 6);
    let index = |i: usize, j: usize| (j * (N + 1) + i) as u32;
    for j in 0..N {
        for i in 0..N {
            indices.extend([index(i, j), index(i, j + 1), index(i + 1, j + 1), index(i, j), index(i + 1, j + 1), index(i + 1, j)]);
        }
    }
    (vertices, indices)
}
/// A column of 8 x 8 quads a face (a 0.5 x 3 x 0.5 m box, 768 triangles) on two joints: the
/// bottom half follows joint 0, the top joint 1, blended through the middle.
fn column() -> (crate::asset::Primitive, Vec<Influence>) {
    let mut faces = vec![];
    let h = Vec3::new(0.25, 1.5, 0.25);
    for (corners, n) in kit::box_faces(Vec3::new(0.0, 1.5, 0.0), h, false) {
        let [a, b, _, d] = corners;
        for v in 0..8 {
            for u in 0..8 {
                let p = |s: f32, t: f32| a + (b - a) * s + (d - a) * t;
                let (s0, s1, t0, t1) = (u as f32 / 8.0, (u + 1) as f32 / 8.0, v as f32 / 8.0, (v + 1) as f32 / 8.0);
                faces.push(([p(s0, t0), p(s1, t0), p(s1, t1), p(s0, t1)], n));
            }
        }
    }
    let (vertices, indices) = triangles_of(&faces);
    let influences = vertices
        .iter()
        .map(|v| {
            let top = (v.position[1] / 3.0).clamp(0.0, 1.0);
            Influence { joints: [0, 1, 0, 0], weights: [1.0 - top, top, 0.0, 0.0] }
        })
        .collect();
    (crate::asset::Primitive { vertices, tangents: None, indices, material: 0 }, influences)
}

fn setting(key: &str, default: f32) -> f32 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl City {
    fn draw(r: &mut Renderer, loaded: &mut Loaded, camera: &Camera) -> Result<(), Error> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let mut draws: Vec<Draw<'_>> = loaded
                .parts
                .iter_mut()
                .map(|p| Draw::new(&p.mesh, &mut p.instances).with_material(&p.material))
                .chain(loaded.skins.iter_mut().map(|s| Draw::new(s.skin.mesh(), &mut s.instances).with_material(&s.material)))
                .collect();
            if r.try_render_view(&loaded.target, camera, &mut draws, ViewMode::Lit)?.is_some() {
                return Ok(());
            }
            drop(draws);
            if std::time::Instant::now() > until {
                return Err(Error::InvalidInput("bench frame: not admitted in 20 s"));
            }
            kit::drain(r)?;
        }
    }
}

impl Fixture for City {
    fn name(&self) -> &str {
        self.name
    }
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error> {
        // The sun's shadows as the game's frames draw them: the virtual clipmap without ray
        // hardware (the bench's `vsm` pass reloads it), the traced interactive sun with it.
        if path == BenchPath::NoRt {
            r.set_soft_shadows(SoftShadows::Virtual);
        }
        let shadows = Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional { split_ends: vec![4.0, 12.0, 40.0] }],
            ..Default::default()
        };
        r.set_lighting_and_shadows(&Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(0.3, -1.0, 0.2).normalize(),
                lux: [setting("LIGHTBENCH_CITY_SUN_LUX", 1.0); 3],
            }],
            environment: [0.0; 3],
            exposure_stops: 0.0,
        }, &shadows)?;
        r.set_reconstruction(Reconstruction::off())?;
        r.set_indirect(IndirectSettings::default())?;
        r.gi.request_emitters();
        let program = r.try_create_program(&ProgramDescriptor { label: "furnace", source: FURNACE_PROGRAM })?;
        let glow = [("glow", Value::Vec3(GLOW))];
        let mut parts = vec![kit::part(r, &kit::box_faces(Vec3::ZERO, ROOM, true), &program, ALBEDO, &glow)?];
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        let floor = -ROOM.y;
        let (mut triangles, mut placements) = (12u64, 1u64);
        match self.kind {
            Kind::Boxes => {
                let mut boxes = vec![];
                for j in 0..55 {
                    for i in 0..55 {
                        if boxes.len() == 3000 {
                            break;
                        }
                        let x = -21.0 + i as f32 * 0.78 + random.next() * 0.3;
                        let z = -21.0 + j as f32 * 0.78 + random.next() * 0.3;
                        let size = Vec3::new(0.2 + random.next() * 0.4, 0.3 + random.next() * random.next() * 3.0, 0.2 + random.next() * 0.4);
                        let turn = glam::Quat::from_rotation_y(random.next() * std::f32::consts::TAU);
                        boxes.push(Mat4::from_scale_rotation_translation(size, turn, Vec3::new(x, floor, z)));
                    }
                }
                let (v, i) = unit_box();
                triangles += (i.len() / 3 * boxes.len()) as u64;
                placements += boxes.len() as u64;
                parts.push(kit::mesh_part(r, &v, &i, &program, ALBEDO, &glow, &boxes)?);
                let rocks: Vec<Mat4> = (0..150)
                    .map(|_| {
                        let p = Vec3::new(-22.0 + random.next() * 44.0, floor + random.next() * 0.4, -22.0 + random.next() * 44.0);
                        Mat4::from_scale_rotation_translation(Vec3::splat(0.3 + random.next() * 0.9), glam::Quat::from_rotation_x(random.next() * 3.0), p)
                    })
                    .collect();
                let (v, i) = rock();
                triangles += (i.len() / 3 * rocks.len()) as u64;
                placements += rocks.len() as u64;
                parts.push(kit::mesh_part(r, &v, &i, &program, ALBEDO, &glow, &rocks)?);
            }
            Kind::Terrain => {
                let (v, i) = terrain();
                triangles += (i.len() / 3) as u64;
                placements += 1;
                parts.push(kit::mesh_part(r, &v, &i, &program, ALBEDO, &glow, &[Mat4::IDENTITY])?);
            }
        }
        // Two skinned columns near the walk, swung every frame (`run`).
        let (primitive, influences) = column();
        let mut skins = vec![];
        for x in [-8.0, 2.0] {
            let skin = kit::admitted(r, "bench skinned mesh", |r| r.try_upload_skinned_mesh(&primitive, &influences, 2))?;
            let positions: Vec<[f32; 3]> = primitive.vertices.iter().map(|v| v.position).collect();
            r.set_indirect_geometry(skin.mesh(), &positions, &primitive.indices)?;
            let placed = [Mat4::from_translation(Vec3::new(x, floor + 1.5, 4.0))];
            let instances = kit::admitted(r, "bench skinned instances", |r| r.try_upload_instances(&placed))?;
            let material = kit::admitted(r, "bench skinned material", |r| {
                r.try_create_material(&MaterialDescriptor {
                    base_color: [ALBEDO[0], ALBEDO[1], ALBEDO[2], 1.0],
                    metallic: 0.0,
                    roughness: 1.0,
                    double_sided: true,
                    program: Some(&program),
                    parameters: &glow,
                    ..Default::default()
                })
            })?;
            triangles += (primitive.indices.len() / 3) as u64;
            placements += 1;
            skins.push(Skin { skin, instances, material });
        }
        let target = kit::color_target(r, SIZE);
        self.loaded = Some(Loaded { _program: program, parts, skins, target, triangles, placements });
        Ok(())
    }
    fn run(&mut self, r: &mut Renderer, _path: BenchPath, _reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let loaded = self.loaded.as_mut().ok_or(Error::InvalidInput("city fixture not loaded"))?;
        r.reset_indirect();
        let settle = setting("LIGHTBENCH_CITY_SETTLE", 32.0) as u32;
        let walked = setting("LIGHTBENCH_CITY_FRAMES", 64.0).max(1.0) as u32;
        let step = setting("LIGHTBENCH_CITY_STEP", 0.05);
        let eye_y = -ROOM.y + if self.kind == Kind::Terrain { 4.5 } else { 1.7 };
        let start = Vec3::new(-14.0, eye_y, 1.0);
        let look = Vec3::new(1.0, -0.08, 0.35);
        let mut series: [Vec<f64>; 3] = Default::default();
        for frame in 0..settle + walked {
            let walking = frame >= settle;
            let at = start + Vec3::X * step * frame.saturating_sub(settle) as f32;
            let camera = Camera::look_at(at, at + look, Vec3::Y, FOV, 0.05, 200.0)?;
            let swing = (frame as f32 * 0.15).sin() * 0.4;
            for s in &mut loaded.skins {
                let top = Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0)) * Mat4::from_rotation_z(swing) * Mat4::from_translation(Vec3::new(0.0, -1.5, 0.0));
                s.skin.set_joint_palette(&[Mat4::IDENTITY, top])?;
            }
            Self::draw(r, loaded, &camera)?;
            if walking {
                let c = r.indirect_counters()?.unwrap_or_default();
                for (values, v) in series.iter_mut().zip([c.gather_rays, c.light_rays, c.aimed_rays]) {
                    values.push(f64::from(v));
                }
            }
        }
        let pixels = kit::capture_radiance(r, 0.0)?;
        let lum: Vec<f64> = pixels.iter().map(|&p| f64::from(kit::luminance(p))).collect();
        let n = lum.len() as f64;
        let answer = f64::from(kit::luminance([GLOW[0] / (1.0 - ALBEDO[0]); 3]));
        self.series = ["gather_rays", "light_rays", "aimed_rays"].into_iter().map(str::to_owned).zip(series).collect();
        let stats = r.indirect_stats();
        Ok(vec![
            ("energy".into(), lum.iter().sum::<f64>() / n / answer),
            ("err".into(), lum.iter().map(|l| (l / answer - 1.0).abs()).sum::<f64>() / n),
            ("off5".into(), lum.iter().filter(|l| (*l / answer - 1.0).abs() > 0.05).count() as f64 / n),
            ("geometry_bytes".into(), stats.geometry_bytes as f64),
            ("triangles".into(), loaded.triangles as f64),
            ("placements".into(), loaded.placements as f64),
            ("frames_measured".into(), f64::from(walked)),
        ])
    }
    fn series(&mut self) -> Vec<(String, Vec<f64>)> {
        std::mem::take(&mut self.series)
    }
}
