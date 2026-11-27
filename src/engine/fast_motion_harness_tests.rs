//! Fast-motion visual quality and performance harness on the `My project 5` arena (the
//! geometry, `ara_emissive` `e_panel` and Night preset of `view_tests`' arena repros, plus a
//! sunlit Day preset), on both render paths at 1600x960.
//!
//! cargo test --lib fast_motion_and_shadow_quality_harness -- --ignored --nocapture
//!
//! - `RENDERLAB_HARNESS_REPORT=1`: print every check and never panic (baseline capture).
//!   Without it the run panics listing every failed check.
//! - `RENDERLAB_HARNESS_OUT=<dir>`: also write the key frames, 6x signed diff heatmaps
//!   (`diff_*.png`: red = brighter than the still reference, blue = darker, green boxes =
//!   measured regions) and annotated stills (`annot_*.png`). Nothing is written without it.
//!
//! Metric units: luma and RGB differences are 0-255 sRGB bytes; `brms`/`bmax` are the RMS and
//! max of 24x24 block-mean RGB differences over the frame (as in `view_tests`), `*_roi_*` the
//! same over 16x16 blocks inside a region; edge positions and widths are screen pixels.
use super::view_tests::cube;
use super::*;
use glam::Vec2;
use std::path::{Path, PathBuf};
use std::time::Instant;

const W: u32 = 1600;
const H: u32 = 960;
const WU: usize = W as usize;
const HU: usize = H as usize;
const DT: f32 = 1.0 / 60.0;
/// Night exposures: Play mode (`arena_night_south_wall_repro`) and Scene view (EV100 4).
const PLAY_STOPS: f32 = -1.5;
const SCENE_STOPS: f32 = -4.263;
/// Cover 5 view: two stops under Play mode, matching Screenshot 1's wall brightness (at
/// -1.5 the walls beside `e_panel` clip to white and hide any striping).
const COVER_STOPS: f32 = -3.5;
const DAY_STOPS: f32 = -13.0;
const PANEL_HOME: Vec3 = Vec3::new(0.0, 3.0, -19.303816);
/// Width of the screen-border strip that a pan newly reveals.
const STRIP: usize = 160;

// ---------------------------------------------------------------------------------------
// Scene: exact My project 5 arena.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preset {
    /// 4 lux moon, three warm lamps, the 8000 lm panel spot and the emissive `e_panel`.
    Night,
    /// `arena_corner_still_and_fast_pan_without_gi_spots_or_blotches`' 90,000 lux sun and
    /// atmosphere sky; no panel, no lamps.
    Day,
}

fn vtx(position: Vec3, normal: Vec3) -> Vertex {
    Vertex {
        position: position.to_array(),
        normal: normal.to_array(),
        uv: [0.0; 2],
        color: [1.0; 4],
    }
}

fn quad(v: &mut Vec<Vertex>, idx: &mut Vec<u32>, corners: [Vec3; 4]) {
    let n = (corners[1] - corners[0])
        .cross(corners[2] - corners[0])
        .normalize_or_zero();
    let base = v.len() as u32;
    v.extend(corners.map(|c| vtx(c, n)));
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Capped 32-segment cylinder (the arena's pillars).
fn cylinder(center: Vec3, radius: f32, half_h: f32) -> (Vec<Vertex>, Vec<u32>) {
    const SEGS: u32 = 32;
    let (mut v, mut idx) = (Vec::new(), Vec::new());
    let ring = |i: u32| {
        let a = i as f32 / SEGS as f32 * std::f32::consts::TAU;
        Vec3::new(a.sin(), 0.0, a.cos())
    };
    for i in 0..SEGS {
        let (n0, n1) = (ring(i), ring(i + 1));
        let base = v.len() as u32;
        v.push(vtx(center + n0 * radius - Vec3::Y * half_h, n0));
        v.push(vtx(center + n1 * radius - Vec3::Y * half_h, n1));
        v.push(vtx(center + n1 * radius + Vec3::Y * half_h, n1));
        v.push(vtx(center + n0 * radius + Vec3::Y * half_h, n0));
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    for (y, up) in [(half_h, true), (-half_h, false)] {
        let n = if up { Vec3::Y } else { -Vec3::Y };
        let c = v.len() as u32;
        v.push(vtx(center + Vec3::Y * y, n));
        for i in 0..SEGS {
            let b = v.len() as u32;
            let (a, b_) = if up { (i, i + 1) } else { (i + 1, i) };
            v.push(vtx(center + ring(a) * radius + Vec3::Y * y, n));
            v.push(vtx(center + ring(b_) * radius + Vec3::Y * y, n));
            idx.extend_from_slice(&[c, b, b + 1]);
        }
    }
    (v, idx)
}

/// The blue ramp: rises toward -Z.
fn wedge(center: Vec3, half: Vec3) -> (Vec<Vertex>, Vec<u32>) {
    let (mut v, mut idx) = (Vec::new(), Vec::new());
    let Vec3 { x, y, z } = half;
    let fl = center + Vec3::new(-x, -y, z);
    let fr = center + Vec3::new(x, -y, z);
    let bl = center + Vec3::new(-x, -y, -z);
    let br = center + Vec3::new(x, -y, -z);
    let tl = center + Vec3::new(-x, y, -z);
    let tr = center + Vec3::new(x, y, -z);
    quad(&mut v, &mut idx, [bl, br, fr, fl]);
    quad(&mut v, &mut idx, [br, bl, tl, tr]);
    quad(&mut v, &mut idx, [fl, fr, tr, tl]);
    for tri in [[fl, tl, bl], [fr, br, tr]] {
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        let b = v.len() as u32;
        v.extend(tri.map(|c| vtx(c, n)));
        idx.extend_from_slice(&[b, b + 1, b + 2]);
    }
    (v, idx)
}

/// The 8-step stairs: climb toward -Z.
fn stairs(center: Vec3, half: Vec3) -> (Vec<Vertex>, Vec<u32>) {
    const STEPS: u32 = 8;
    let (mut v, mut idx) = (Vec::new(), Vec::new());
    let Vec3 { x, y, z } = half;
    let at_z = |i: u32| z - 2.0 * z * (i as f32 / STEPS as f32);
    let at_y = |i: u32| -y + 2.0 * y * (i as f32 / STEPS as f32);
    let p = |px: f32, py: f32, pz: f32| center + Vec3::new(px, py, pz);
    quad(&mut v, &mut idx, [p(-x, -y, -z), p(x, -y, -z), p(x, -y, z), p(-x, -y, z)]);
    quad(&mut v, &mut idx, [p(x, -y, -z), p(-x, -y, -z), p(-x, y, -z), p(x, y, -z)]);
    for i in 0..STEPS {
        let (front, back, low, high) = (at_z(i), at_z(i + 1), at_y(i), at_y(i + 1));
        quad(&mut v, &mut idx, [p(-x, low, front), p(x, low, front), p(x, high, front), p(-x, high, front)]);
        quad(&mut v, &mut idx, [p(-x, high, front), p(x, high, front), p(x, high, back), p(-x, high, back)]);
    }
    let mut profile: Vec<(f32, f32)> = (0..STEPS)
        .flat_map(|i| [(at_z(i), at_y(i)), (at_z(i), at_y(i + 1))])
        .collect();
    profile.push((-z, y));
    profile.push((-z, -y));
    let corner = profile.len() as u32 - 1;
    for (sx, nx, flip) in [(-x, -1.0f32, false), (x, 1.0f32, true)] {
        let base = v.len() as u32;
        v.extend(profile.iter().map(|&(pz, py)| vtx(p(sx, py, pz), Vec3::X * nx)));
        for k in 0..(corner - 1) {
            if flip {
                idx.extend_from_slice(&[base + corner, base + k + 1, base + k]);
            } else {
                idx.extend_from_slice(&[base + corner, base + k, base + k + 1]);
            }
        }
    }
    (v, idx)
}

/// Ara's `ara_emissive` program (ara-renderlab/src/scene.rs `emissive_source`).
fn emissive_wgsl() -> String {
    format!(
        "struct Parameters {{ glow:vec3<f32>, everywhere:f32, bounce:vec3<f32>, mapped:f32 }};\n\
         const DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.0),0.0,vec3<f32>(0.0),0.0);\n\
         fn ara_glows(input:SurfaceInput) -> bool {{ return params.everywhere>0.5 || abs(input.material.base_color.a-0.737)<0.002; }}\n\
         {}\n\
         fn emission(s:Surface, input:SurfaceInput) -> vec3<f32> {{ return select(vec3<f32>(0.0),max(params.bounce,vec3<f32>(0.0)),ara_glows(input)); }}\n",
        crate::STANDARD_PROGRAM
            .replace("alpha:f32 };", "alpha:f32, glow:vec3<f32> };")
            .replace("decal_mr.x,alpha);", "decal_mr.x,alpha,select(vec3<f32>(0.0),max(params.glow,vec3<f32>(0.0)),ara_glows(input)));")
            .replace("    return exposed;", "    return min(exposed+exposed_product(s.glow,vec3<f32>(1.0)),vec3<f32>(65504.0));")
            .replace("s.n,s.base,s.alpha,s.metal,s.rough,vec3<f32>(0.0),", "s.n,s.base,s.alpha,s.metal,s.rough,s.glow,")
            .replace("    s.alpha=m.alpha;\n    return s;", "    s.alpha=m.alpha;\n    s.glow=m.emission;\n    return s;")
    )
}

fn view_desc(preset: Preset, stops: f32) -> ViewDesc {
    let post = match preset {
        Preset::Night => crate::PostSettings {
            bloom: 0.05,
            motion_blur: false,
            tonemapper: crate::Tonemapper::Filmic,
            ..Default::default()
        },
        Preset::Day => crate::PostSettings {
            motion_blur: false,
            ..Default::default()
        },
    };
    ViewDesc {
        look: ViewLook {
            exposure: Exposure::Manual { stops },
            post,
            ..Default::default()
        },
        ..ViewDesc::new(W, H)
    }
}

/// One engine and view over the arena; `panel` is the movable `e_panel` (Night only).
struct Arena {
    engine: Engine,
    view: crate::ViewId,
    panel: Option<ObjectId>,
    tag: &'static str,
}

impl Arena {
    fn new(path: RenderPath, preset: Preset) -> Self {
        // SAFETY: the test suite's documented experimental-feature acknowledgement.
        let experimental_ray_query =
            (path == RenderPath::HardwareRt).then(|| unsafe { ExperimentalRayQuery::acknowledge() });
        let mut engine = Engine::new(
            test_adapter(),
            EngineDescriptor {
                path,
                experimental_ray_query,
                ..Default::default()
            },
        )
        .unwrap();
        let spawn = |engine: &mut Engine, (vertices, indices): (Vec<Vertex>, Vec<u32>), colour, roughness| {
            let mesh = engine
                .create_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })
                .unwrap();
            let material = engine
                .create_material(&MaterialDesc {
                    base_color: colour,
                    roughness,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap();
            engine.spawn(mesh, material, Mat4::IDENTITY).unwrap()
        };
        const FLOOR: [f32; 4] = [0.18, 0.18, 0.18, 1.0];
        const WALL: [f32; 4] = [0.42, 0.41, 0.39, 1.0];
        const STONE: [f32; 4] = [0.70, 0.67, 0.62, 1.0];
        const RUST: [f32; 4] = [0.52, 0.18, 0.04, 1.0];
        const CRATE: [f32; 4] = [0.4, 0.24, 0.12, 1.0];
        let e = &mut engine;
        spawn(e, cube([0.0, -0.25, 0.0], [20.0, 0.25, 20.0]), FLOOR, 0.88);
        spawn(e, cube([0.0, 2.0, -19.75], [20.0, 2.0, 0.25]), WALL, 0.92);
        spawn(e, cube([0.0, 2.0, 19.75], [20.0, 2.0, 0.25]), WALL, 0.92);
        spawn(e, cube([-19.75, 2.0, 0.0], [0.25, 2.0, 19.5]), WALL, 0.92);
        spawn(e, cube([19.75, 2.0, 0.0], [0.25, 2.0, 19.5]), WALL, 0.92);
        spawn(e, cube([0.0, 1.0, -8.0], [4.0, 1.0, 3.0]), FLOOR, 0.88);
        spawn(e, cube([-12.4, 2.5, 4.0], [0.6, 2.5, 0.5]), WALL, 0.92);
        spawn(e, cube([-7.6, 2.5, 4.0], [0.6, 2.5, 0.5]), WALL, 0.92);
        spawn(e, cube([-10.0, 4.5, 4.0], [1.8, 0.5, 0.5]), WALL, 0.92);
        spawn(e, wedge(Vec3::new(-2.0, 1.0, -2.0), Vec3::new(1.5, 1.0, 3.0)), [0.04, 0.16, 0.60, 1.0], 0.78);
        spawn(e, stairs(Vec3::new(2.0, 1.0, -3.0), Vec3::new(1.5, 1.0, 2.0)), STONE, 0.90);
        for [px, pz] in [[-14.0, -14.0], [14.0, -14.0], [-14.0, 14.0], [14.0, 14.0]] {
            spawn(e, cylinder(Vec3::new(px, 2.0, pz), 0.6, 2.0), STONE, 0.90);
        }
        for (c, h) in [
            ([6.0, 0.6, 6.0], [1.0, 0.6, 0.3]),
            ([-6.0, 0.6, 9.0], [1.0, 0.6, 0.3]),
            ([9.0, 0.6, -2.0], [0.3, 0.6, 1.0]),
            ([12.0, 0.6, 11.0], [1.0, 0.6, 0.3]),
            ([-14.0, 0.6, -5.0], [0.3, 0.6, 1.0]),
        ] {
            spawn(e, cube(c, h), RUST, 0.78);
        }
        for pos in [[5.0, 0.4, 12.0], [5.9, 0.4, 12.0], [5.45, 1.2, 12.0], [-7.0, 0.4, 13.0], [-1.2, 2.4, -9.0], [0.0, 2.4, -9.5]] {
            spawn(e, cube(pos, [0.4; 3]), CRATE, 0.55);
        }
        let panel = (preset == Preset::Night).then(|| {
            let program = engine.create_program("ara_emissive", &emissive_wgsl()).unwrap();
            let (vertices, indices) = cube([0.0; 3], [2.0, 0.5, 0.05]);
            let mesh = engine
                .create_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })
                .unwrap();
            let material = engine
                .create_material(&MaterialDesc {
                    base_color: [0.8, 0.8, 0.8, 0.737],
                    roughness: 0.4,
                    double_sided: true,
                    program: Some(program),
                    ..Default::default()
                })
                .unwrap();
            let glow = || crate::Value::Vec3([3000.0, 2250.0, 1450.0]);
            engine
                .set_material_parameters(
                    material,
                    &[
                        ("glow", glow()),
                        ("everywhere", crate::Value::F32(0.0)),
                        ("bounce", glow()),
                        ("mapped", crate::Value::F32(0.0)),
                    ],
                )
                .unwrap();
            let object = engine
                .spawn(mesh, material, Mat4::from_translation(PANEL_HOME))
                .unwrap();
            engine.set_casts_shadows(object, false).unwrap();
            object
        });

        match preset {
            Preset::Night => {
                // Ara EulerRot::XYZ for e_sun rotation [-35, -32, 0]: toward = quat * +Z.
                let q = glam::Quat::from_euler(glam::EulerRot::XYZ, -35f32.to_radians(), -32f32.to_radians(), 0.0);
                engine
                    .set_sun(Some(Sun {
                        travel: -(q * Vec3::Z).normalize(),
                        illuminance: [4.0 * 0.64161, 4.0 * 0.73686, 4.0],
                        angular_radius: 0.5357f32.to_radians() * 0.5,
                    }))
                    .unwrap();
                let lm_to_cd = |lm: f32, c: [f32; 3]| c.map(|c| c * lm / (4.0 * std::f32::consts::PI));
                let lamp = |position: Vec3, lm: f32| crate::engine::Light::Point {
                    position,
                    candela: lm_to_cd(lm, [1.0, 0.72, 0.45]),
                    range: Some(14.0),
                    source_radius: 0.1,
                    cast_shadows: Some(true),
                };
                engine
                    .set_lights(&[
                        lamp(Vec3::new(0.0, 3.6, -8.0), 6000.0),
                        lamp(Vec3::new(-10.0, 4.4, 5.5), 5000.0),
                        lamp(Vec3::new(12.0, 2.5, 12.0), 4000.0),
                        crate::engine::Light::Spot {
                            position: Vec3::new(0.0, 3.0, -19.35),
                            travel: Vec3::Z,
                            candela: lm_to_cd(8000.0, [1.0, 0.75, 0.48]),
                            range: Some(16.0),
                            inner: 0.0,
                            outer: 1.55,
                            source_radius: 0.5,
                            cast_shadows: Some(true),
                        },
                    ])
                    .unwrap();
            }
            Preset::Day => {
                // My project 5 sun rotation [-38, -32, 0]: West and South inner walls shadowed.
                let (el, az) = (38f32.to_radians(), 32f32.to_radians());
                let toward = Vec3::new(-el.cos() * az.sin(), el.sin(), el.cos() * az.cos());
                engine
                    .set_sun(Some(Sun {
                        travel: -toward.normalize(),
                        illuminance: [90_000.0, 85_500.0, 77_400.0],
                        ..Sun::default()
                    }))
                    .unwrap();
            }
        }
        engine
            .set_sky(crate::engine::Sky::Atmosphere(crate::engine::AtmosphereMedium::default()))
            .unwrap();
        engine
            .set_settings(crate::engine::Settings {
                aerial_perspective: true,
                height_fog: (preset == Preset::Night).then(|| crate::engine::HeightFog {
                    density: 0.004,
                    falloff: 1.0 / 30.0,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();
        let stops = if preset == Preset::Night { PLAY_STOPS } else { DAY_STOPS };
        let view = engine.create_view(&view_desc(preset, stops)).unwrap();
        let tag = if path == RenderPath::HardwareRt { "rt" } else { "nonrt" };
        Self { engine, view, panel, tag }
    }

    fn step(&mut self, pose: Pose) -> FrameReport {
        self.engine.tick(DT).unwrap();
        self.engine.render_view(self.view, &pose.camera()).unwrap()
    }

    fn settle(&mut self, pose: Pose, frames: u32) {
        for _ in 0..frames {
            self.step(pose);
        }
    }

    fn capture(&mut self) -> Image {
        self.engine.capture_view(self.view).unwrap()
    }

    fn cut(&mut self) {
        self.engine.cut_view(self.view);
    }
}

// ---------------------------------------------------------------------------------------
// Cameras and motion.

#[derive(Clone, Copy)]
struct Pose {
    eye: Vec3,
    dir: Vec3,
    fov_deg: f32,
}

impl Pose {
    /// Yaw 0 faces -Z and positive yaw turns toward +X; positive pitch looks down.
    fn yaw_pitch(eye: Vec3, yaw: f32, pitch: f32, fov_deg: f32) -> Self {
        let dir = Vec3::new(yaw.sin() * pitch.cos(), -pitch.sin(), -yaw.cos() * pitch.cos());
        Self { eye, dir, fov_deg }
    }
    fn camera(self) -> Camera {
        Camera::look_at(self.eye, self.eye + self.dir, Vec3::Y, self.fov_deg.to_radians(), 0.05, 4000.0).unwrap()
    }
    /// Turned right (clockwise seen from above) by `deg`.
    fn turned(self, deg: f32) -> Self {
        Self {
            dir: glam::Quat::from_rotation_y(-deg.to_radians()) * self.dir,
            ..self
        }
    }
    fn flat_forward(self) -> Vec3 {
        Vec3::new(self.dir.x, 0.0, self.dir.z).normalize()
    }
    fn flat_right(self) -> Vec3 {
        self.flat_forward().cross(Vec3::Y)
    }
}

/// Cover 5 (Screenshot 1): Play-mode eye fitted to the screenshot's landmarks (the West and
/// North wall corner, Pillar 1, Cover 5, the Platform and `e_panel`; residuals <= 13 px at
/// 2000x1259), looking north across the floor at the Cover 5 shadow on the West wall.
fn cover_pose() -> Pose {
    Pose::yaw_pitch(Vec3::new(-15.52, 1.32, 4.54), -0.0593, 0.2689, 75.0)
}

/// `arena_night_south_wall_repro`'s player view at x = -2 along the grazing South wall.
fn south_pose() -> Pose {
    Pose {
        eye: Vec3::new(-2.0, 1.65, 16.2),
        dir: Vec3::new(-1.0, 0.08, 0.14).normalize(),
        fov_deg: 75.0,
    }
}

/// `arena_corner`'s elevated view across the sun-shadowed West/South walls and Pillar 3.
fn day_pose() -> Pose {
    Pose::yaw_pitch(Vec3::new(-12.75, 6.66, 16.47), -1.065001, 0.725, 75.0)
}

/// `moving_emitter_and_jump_no_rings_or_flicker`'s Scene-view camera over the platform.
fn emitter_pose() -> Pose {
    let eye = Vec3::new(-12.0, 12.0, 6.0);
    Pose {
        eye,
        dir: (Vec3::new(0.0, 1.0, -7.0) - eye).normalize(),
        fov_deg: 65.0,
    }
}

/// Yaw offsets (degrees) of a whip pan at `step` deg/frame: left to -amp, across to +amp, then
/// back, ending on 0 while still moving at full speed.
fn whip(step: f32, amp: f32) -> Vec<f32> {
    let n = (amp / step).round() as i32;
    let mut out: Vec<f32> = (1..=n).map(|i| -step * i as f32).collect();
    out.extend((1..=2 * n).map(|i| -amp + step * i as f32));
    out.extend((1..=n).map(|i| amp - step * i as f32));
    out
}

/// +/-`amp` deg shake at `amp/2` deg/frame (reverses every 4 frames), three cycles, ending on 0
/// mid-stroke.
fn shake(amp: f32) -> Vec<f32> {
    let step = amp / 2.0;
    (1..=24)
        .map(|t| match t % 8 {
            0 | 4 => 0.0,
            k @ 1..=3 => step * [1.0, 2.0, 1.0][k - 1],
            k => -step * [1.0, 2.0, 1.0][k - 5],
        })
        .collect()
}

/// Run of `frames` frames at `speed` m/s arriving at `pose` (last entry), the view direction
/// fixed: forward plus a 45-degree strafe when `strafe`, picking the side that keeps the start
/// inside the arena, and a `jump` m arc.
fn sprint(pose: Pose, speed: f32, strafe: bool, jump: f32, frames: u32) -> Vec<Pose> {
    let fwd = pose.flat_forward();
    let candidates: Vec<Vec3> = if strafe {
        vec![(fwd - pose.flat_right()).normalize(), (fwd + pose.flat_right()).normalize()]
    } else {
        vec![fwd]
    };
    let run = speed * DT * frames as f32;
    let velocity = candidates
        .iter()
        .copied()
        .find(|v| {
            let start = pose.eye - *v * run;
            start.x.abs() < 18.5 && start.z.abs() < 18.5
        })
        .unwrap_or(candidates[0]);
    (0..=frames)
        .rev()
        .map(|i| {
            let t = i as f32 / frames as f32;
            Pose {
                eye: pose.eye - velocity * speed * DT * i as f32 + Vec3::Y * jump * (t * std::f32::consts::PI).sin(),
                ..pose
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum Side {
    Left,
    Right,
}

/// The screen border newly revealed when the camera went from `prev` to `now`: the opposite
/// side from where the previous view centre lands. None for (nearly) no rotation.
fn revealed(prev: Pose, now: Pose) -> Option<Side> {
    let p = project(&now.camera(), now.eye + prev.dir * 10.0)?;
    let dx = p.x - W as f32 * 0.5;
    if dx.abs() < 2.0 {
        None
    } else if dx < 0.0 {
        Some(Side::Right)
    } else {
        Some(Side::Left)
    }
}

/// World point to pixel coordinates (pixel centres at +0.5), None behind the eye.
fn project(camera: &Camera, p: Vec3) -> Option<Vec2> {
    let c = camera.matrix(W, H) * p.extend(1.0);
    (c.w > 1e-4).then(|| Vec2::new((c.x / c.w * 0.5 + 0.5) * W as f32, (0.5 - c.y / c.w * 0.5) * H as f32))
}

// ---------------------------------------------------------------------------------------
// Image measurements.

/// x0, y0, x1, y1 (exclusive) in pixels.
type Roi = (usize, usize, usize, usize);
const FULL: Roi = (0, 0, WU, HU);

/// Rec. 709 luma plane.
struct Luma(Vec<f32>);

impl Luma {
    fn of(img: &Image) -> Self {
        Self(
            img.rgba8
                .chunks_exact(4)
                .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
                .collect(),
        )
    }
    fn at(&self, x: usize, y: usize) -> f32 {
        self.0[y * WU + x]
    }
    /// Bilinear at a pixel-coordinate point (centres at +0.5); None outside the frame.
    fn sample(&self, p: Vec2) -> Option<f32> {
        let (fx, fy) = (p.x - 0.5, p.y - 0.5);
        if !(0.0..(WU - 1) as f32).contains(&fx) || !(0.0..(HU - 1) as f32).contains(&fy) {
            return None;
        }
        let (x, y) = (fx as usize, fy as usize);
        let (tx, ty) = (fx - x as f32, fy - y as f32);
        let top = self.at(x, y) * (1.0 - tx) + self.at(x + 1, y) * tx;
        let bottom = self.at(x, y + 1) * (1.0 - tx) + self.at(x + 1, y + 1) * tx;
        Some(top * (1.0 - ty) + bottom * ty)
    }
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len().max(1) as f64
}

fn std_dev(xs: &[f64]) -> f64 {
    let m = mean(xs);
    (xs.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / xs.len().max(1) as f64).sqrt()
}

/// RMS and max over `block`-px blocks inside `roi` of the block-mean RGB difference length.
fn block_diff(a: &Image, b: &Image, roi: Roi, block: usize) -> (f64, f64) {
    let (x0, y0, x1, y1) = roi;
    let (mut sq, mut max, mut n) = (0.0f64, 0.0f64, 0u32);
    for ty in (y0..y1.saturating_sub(block - 1)).step_by(block) {
        for tx in (x0..x1.saturating_sub(block - 1)).step_by(block) {
            let mut d = [0.0f64; 3];
            for y in ty..ty + block {
                for x in tx..tx + block {
                    let i = (y * WU + x) * 4;
                    for (c, dc) in d.iter_mut().enumerate() {
                        *dc += f64::from(a.rgba8[i + c]) - f64::from(b.rgba8[i + c]);
                    }
                }
            }
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / (block * block) as f64;
            sq += len * len;
            max = max.max(len);
            n += 1;
        }
    }
    if n == 0 {
        return (0.0, 0.0);
    }
    ((sq / f64::from(n)).sqrt(), max)
}

/// Std of the per-pixel luma difference inside `roi`.
fn luma_diff_std(a: &Luma, b: &Luma, roi: Roi) -> f64 {
    let (x0, y0, x1, y1) = roi;
    let d: Vec<f64> = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y, x)))
        .map(|(y, x)| f64::from(a.at(x, y) - b.at(x, y)))
        .collect();
    std_dev(&d)
}

/// Luma-difference std over the newly revealed border strip; both strips' max when unknown.
fn strip_std(a: &Luma, b: &Luma, side: Option<Side>) -> f64 {
    let left = (0, 0, STRIP, HU);
    let right = (WU - STRIP, 0, WU, HU);
    match side {
        Some(Side::Left) => luma_diff_std(a, b, left),
        Some(Side::Right) => luma_diff_std(a, b, right),
        None => luma_diff_std(a, b, left).max(luma_diff_std(a, b, right)),
    }
}

fn strip_roi(side: Option<Side>) -> Roi {
    match side {
        Some(Side::Right) => (WU - STRIP, 0, WU, HU),
        _ => (0, 0, STRIP, HU),
    }
}

/// Largest sinusoid amplitude of `xs` over integer periods in `periods` (samples).
fn periodic_amp(xs: &[f64], periods: std::ops::RangeInclusive<usize>) -> f64 {
    let n = xs.len() as f64;
    periods
        .map(|p| {
            let w = std::f64::consts::TAU / p as f64;
            let (re, im) = xs
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (i, v)| (re + v * (w * i as f64).cos(), im - v * (w * i as f64).sin()));
            2.0 * (re * re + im * im).sqrt() / n
        })
        .fold(0.0, f64::max)
}

/// Screen bounding box of world points (in front of the eye), clipped to the frame.
fn screen_box(camera: &Camera, points: &[Vec3]) -> Roi {
    let ps: Vec<Vec2> = points.iter().filter_map(|&p| project(camera, p)).collect();
    let (lo, hi) = ps.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let c = |v: f32, m: usize| (v.max(0.0) as usize).min(m);
    (c(lo.x, WU), c(lo.y, HU), c(hi.x, WU), c(hi.y, HU))
}

/// Largest screen box inside a world rectangle seen roughly face-on (inner bounds of its
/// projected corners `[bottom-left, bottom-right, top-right, top-left]`).
fn inner_box(camera: &Camera, corners: [Vec3; 4]) -> Roi {
    let p = corners.map(|c| project(camera, c).unwrap());
    let x0 = p[0].x.max(p[3].x);
    let x1 = p[1].x.min(p[2].x);
    let y0 = p[2].y.max(p[3].y);
    let y1 = p[0].y.min(p[1].y);
    let c = |v: f32, m: usize| (v.max(0.0) as usize).min(m);
    (c(x0, WU), c(y0, HU), c(x1, WU), c(y1, HU))
}

/// A cast-shadow boundary on the floor: the world line from the occluder's silhouette corner
/// `start` toward `end`, the shadow lying on the `inside` side. Optional `max_s` and
/// `skip_x` exclude regions where a foreign shadow or vertical wall crosses the transverse
/// +/-0.45 m profile window so the estimator measures Cover 5's shadow boundary alone.
struct ShadowEdge {
    start: Vec3,
    end: Vec3,
    inside: Vec3,
    max_s: f32,
    skip_x: Option<(f32, f32)>,
}

#[derive(Default, Clone, Copy)]
struct EdgeStats {
    /// RMS (px) of the boundary position about a smooth quadratic through it.
    rough: f64,
    /// Largest 12-24 px periodic amplitude (px) along the boundary: tile staircasing.
    p16: f64,
    /// Largest deviation (px) from the smooth boundary.
    jag: f64,
    /// Median 10-90% transition width (px) over the first 0.6 m beside the occluder.
    contact: f64,
    /// Screen-pixel samples used.
    samples: usize,
}

/// Samples the boundary once per screen pixel of its length: at each sample a 61-point luma
/// profile across the edge (+/-0.45 m) gives the edge position (area estimator, robust to the
/// penumbra's shape) and its 10-90% width.
fn shadow_edge(l: &Luma, camera: &Camera, edge: &ShadowEdge) -> EdgeStats {
    const R: f32 = 0.45;
    const K: usize = 61;
    let axis = edge.end - edge.start;
    let len = axis.length();
    let u = axis / len;
    let n = edge.inside.normalize();
    // Fine world samples, then resample at one screen pixel of arc length.
    let fine: Vec<(f32, Vec2)> = (0..=2000)
        .filter_map(|k| {
            let s = 0.15 + (len - 0.45) * k as f32 / 2000.0;
            project(camera, edge.start + u * s).map(|p| (s, p))
        })
        .collect();
    let mut picks = Vec::new();
    let mut arc = 0.0f32;
    let mut next = 0.0f32;
    for w in fine.windows(2) {
        let seg = (w[1].1 - w[0].1).length();
        while next <= arc + seg && seg > 0.0 {
            let t = (next - arc) / seg;
            picks.push(w[0].0 + (w[1].0 - w[0].0) * t);
            next += 1.0;
        }
        arc += seg;
    }
    let (mut ss, mut pos, mut widths) = (Vec::new(), Vec::new(), Vec::new());
    for s in picks {
        if s > edge.max_s {
            continue;
        }
        let p = edge.start + u * s;
        let q_lo = p - n * R;
        let q_hi = p + n * R;
        // Skip transverse profiles whose endpoints leave the floor onto the vertical West wall
        // (x = -19.5), clip Cover 5's own vertical side, or straddle a foreign shadow boundary.
        if q_lo.x.min(q_hi.x) <= -19.45 {
            continue;
        }
        if q_lo.z.min(q_hi.z) <= -3.95 && q_lo.x.max(q_hi.x) >= -14.35 && q_lo.x.min(q_hi.x) <= -13.65 {
            continue;
        }
        if let Some((x_lo, x_hi)) = edge.skip_x {
            if q_lo.x.min(q_hi.x) <= x_hi && q_lo.x.max(q_hi.x) >= x_lo {
                continue;
            }
        }
        let (Some(c0), Some(c1)) = (project(camera, p), project(camera, p - n * 0.01)) else {
            continue;
        };
        let px_per_m = f64::from((c1 - c0).length()) * 100.0;
        let profile: Option<Vec<f32>> = (0..K)
            .map(|i| {
                let o = -R + 2.0 * R * i as f32 / (K - 1) as f32;
                // o > 0 is outside the shadow.
                l.sample(project(camera, p - n * o)?)
            })
            .collect();
        let Some(profile) = profile else { continue };
        let tail = K / 5;
        let lo = profile[..tail].iter().sum::<f32>() / tail as f32;
        let hi = profile[K - tail..].iter().sum::<f32>() / tail as f32;
        if hi - lo < 6.0 {
            continue;
        }
        let norm: Vec<f32> = profile.iter().map(|v| ((v - lo) / (hi - lo)).clamp(0.0, 1.0)).collect();
        let step = 2.0 * R / (K - 1) as f32;
        let area: f32 = norm.windows(2).map(|w| (w[0] + w[1]) * 0.5 * step).sum();
        let e = R - area;
        let cross = |level: f32| {
            norm.windows(2).enumerate().find_map(|(i, w)| {
                (w[0] < level && w[1] >= level).then(|| -R + step * (i as f32 + (level - w[0]) / (w[1] - w[0])))
            })
        };
        if s < 0.75 {
            if let (Some(a), Some(b)) = (cross(0.1), cross(0.9)) {
                widths.push(f64::from(b - a) * px_per_m);
            }
        }
        ss.push(f64::from(s));
        pos.push(f64::from(e) * px_per_m);
    }
    if pos.len() < 24 {
        return EdgeStats {
            rough: f64::NAN,
            p16: f64::NAN,
            jag: f64::NAN,
            contact: f64::NAN,
            samples: pos.len(),
        };
    }
    // Quadratic least squares through the boundary positions along true world arc length s.
    let s0 = ss[0];
    let s_span = (ss[ss.len() - 1] - s0).max(1e-6);
    let xs: Vec<f64> = ss.iter().map(|&s| (s - s0) / s_span).collect();
    let fit = quadratic_fit(&xs, &pos);
    let residual: Vec<f64> = xs.iter().zip(&pos).map(|(x, y)| y - (fit[0] + fit[1] * x + fit[2] * x * x)).collect();
    widths.sort_by(f64::total_cmp);
    EdgeStats {
        rough: (residual.iter().map(|r| r * r).sum::<f64>() / residual.len() as f64).sqrt(),
        p16: periodic_amp(&residual, 12..=24),
        jag: residual.iter().fold(0.0, |m, r| m.max(r.abs())),
        contact: widths.get(widths.len() / 2).copied().unwrap_or(f64::NAN),
        samples: pos.len(),
    }
}

fn quadratic_fit(xs: &[f64], ys: &[f64]) -> [f64; 3] {
    let mut m = [[0.0f64; 4]; 3];
    for (&x, &y) in xs.iter().zip(ys) {
        let p = [1.0, x, x * x];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] += p[r] * p[c];
            }
            m[r][3] += p[r] * y;
        }
    }
    for i in 0..3 {
        let pivot = m[i][i];
        for c in i..4 {
            m[i][c] /= pivot;
        }
        for r in 0..3 {
            if r != i {
                let f = m[r][i];
                for c in i..4 {
                    m[r][c] -= f * m[i][c];
                }
            }
        }
    }
    [m[0][3], m[1][3], m[2][3]]
}

/// Column-banding of a wall region: std (luma) of the column means after removing a local
/// 33-column degree-2 Savitzky-Golay quadratic trend on the unclipped interior, and the largest
/// 14-18 px periodic amplitude. Quadratic detrending removes smooth physical 1/d^2 emitter
/// falloff curvature without attenuating 16-px tile stripes.
fn column_stripes(l: &Luma, roi: Roi) -> (f64, f64) {
    let (x0, y0, x1, y1) = roi;
    const R: usize = 16;
    if x1 <= x0 + 2 * R + 16 || y1 <= y0 + 4 {
        return (f64::NAN, f64::NAN);
    }
    let cols: Vec<f64> = (x0..x1)
        .map(|x| (y0..y1).map(|y| f64::from(l.at(x, y))).sum::<f64>() / (y1 - y0) as f64)
        .collect();
    let n = (2 * R + 1) as f64;
    let s2: f64 = (-(R as i64)..=(R as i64)).map(|k| (k * k) as f64).sum();
    let s4: f64 = (-(R as i64)..=(R as i64)).map(|k| (k * k * k * k) as f64).sum();
    let denom = n * s4 - s2 * s2;
    let hp: Vec<f64> = (R..cols.len() - R)
        .map(|i| {
            let trend: f64 = (-(R as i64)..=(R as i64))
                .map(|k| {
                    let w = (s4 - s2 * (k * k) as f64) / denom;
                    w * cols[(i as isize + k as isize) as usize]
                })
                .sum();
            cols[i] - trend
        })
        .collect();
    (std_dev(&hp), periodic_amp(&hp, 14..=18))
}

/// Temporal boil: mean per-pixel luma std across `frames` warped into `target` (pure-rotation
/// reprojection: every camera shares the eye, so it is exact without depth), over `roi` minus
/// strong still-image edges (bilinear resampling error) and pixels any frame does not cover.
fn boil(frames: &[(Luma, Camera)], target: &Camera, still: &Luma, roi: Roi) -> f64 {
    let inverse = target.matrix(W, H).inverse();
    let (x0, y0, x1, y1) = roi;
    let (mut sum, mut n) = (0.0f64, 0u64);
    for y in y0.max(1)..y1.min(HU - 1) {
        for x in x0.max(1)..x1.min(WU - 1) {
            let gx = still.at(x + 1, y) - still.at(x - 1, y);
            let gy = still.at(x, y + 1) - still.at(x, y - 1);
            if gx.abs() + gy.abs() > 12.0 {
                continue;
            }
            let ndc = Vec2::new((x as f32 + 0.5) / W as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / H as f32 * 2.0);
            let world = inverse * glam::Vec4::new(ndc.x, ndc.y, 0.5, 1.0);
            let world = world.truncate() / world.w;
            let values: Option<Vec<f64>> = frames
                .iter()
                .map(|(l, c)| project(c, world).and_then(|p| l.sample(p)).map(f64::from))
                .collect();
            if let Some(values) = values {
                sum += std_dev(&values);
                n += 1;
            }
        }
    }
    sum / n.max(1) as f64
}

// ---------------------------------------------------------------------------------------
// Output.

struct Out(Option<PathBuf>);

impl Out {
    fn png(&self, name: &str, width: u32, height: u32, rgba: &[u8]) {
        let Some(dir) = &self.0 else { return };
        write_png(&dir.join(format!("{name}.png")), width, height, rgba);
    }
    fn image(&self, name: &str, img: &Image) {
        self.png(name, img.width, img.height, &img.rgba8);
    }
    /// 6x signed luma difference over a dim copy of the reference, with `rois` outlined.
    fn diff(&self, name: &str, img: &Luma, reference: &Luma, rois: &[Roi]) {
        if self.0.is_none() {
            return;
        }
        let mut rgba = vec![255u8; WU * HU * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let d = (img.0[i] - reference.0[i]) * 6.0;
            let base = reference.0[i] * 0.2;
            px[0] = (base + d.max(0.0)).min(255.0) as u8;
            px[1] = base.min(255.0) as u8;
            px[2] = (base + (-d).max(0.0)).min(255.0) as u8;
        }
        for &roi in rois {
            outline(&mut rgba, roi, [0, 255, 0]);
        }
        self.png(&format!("diff_{name}"), W, H, &rgba);
    }
}

fn outline(rgba: &mut [u8], (x0, y0, x1, y1): Roi, colour: [u8; 3]) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let mut put = |x: usize, y: usize| {
        let i = (y.min(HU - 1) * WU + x.min(WU - 1)) * 4;
        rgba[i..i + 3].copy_from_slice(&colour);
    };
    for x in x0..x1 {
        put(x, y0);
        put(x, y1 - 1);
    }
    for y in y0..y1 {
        put(x0, y);
        put(x1 - 1, y);
    }
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// Pass bar of one metric.
#[derive(Clone, Copy)]
enum Bar {
    Below(f64),
    Above(f64),
    Info,
}

struct Row {
    scenario: &'static str,
    mode: &'static str,
    metric: &'static str,
    bar: Bar,
    /// [HardwareRt, NonRt].
    values: [Option<f64>; 2],
}

/// Every metric of the run, printed as one RT/Non-RT table.
#[derive(Default)]
struct Report(Vec<Row>);

impl Report {
    fn put(&mut self, path: usize, scenario: &'static str, mode: &'static str, metric: &'static str, value: f64, bar: Bar) {
        let found = self
            .0
            .iter_mut()
            .find(|r| r.scenario == scenario && r.mode == mode && r.metric == metric);
        match found {
            Some(row) => row.values[path] = Some(value),
            None => {
                let mut values = [None; 2];
                values[path] = Some(value);
                self.0.push(Row {
                    scenario,
                    mode,
                    metric,
                    bar,
                    values,
                });
            }
        }
    }

    /// Prints the table; returns the failed checks.
    fn print(&self) -> Vec<String> {
        let pass = |v: f64, bar: Bar| match bar {
            Bar::Below(limit) => v < limit,
            Bar::Above(limit) => v > limit,
            Bar::Info => true,
        };
        let mut failures = Vec::new();
        eprintln!(
            "\n{:<14} {:<12} {:<17} {:>10} {:>10}  {:<10} {}",
            "scenario", "mode", "metric", "rt", "nonrt", "bar", "status (rt/nonrt)"
        );
        eprintln!("{}", "-".repeat(96));
        for row in &self.0 {
            let bar = match row.bar {
                Bar::Below(l) => format!("< {l}"),
                Bar::Above(l) => format!("> {l}"),
                Bar::Info => "info".into(),
            };
            let cell = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.3}"));
            let status: Vec<&str> = row
                .values
                .iter()
                .map(|v| match (v, row.bar) {
                    (_, Bar::Info) => "",
                    (None, _) => "-",
                    (Some(v), bar) if !v.is_nan() && pass(*v, bar) => "PASS",
                    _ => "FAIL",
                })
                .collect();
            eprintln!(
                "{:<14} {:<12} {:<17} {:>10} {:>10}  {:<10} {}",
                row.scenario,
                row.mode,
                row.metric,
                cell(row.values[0]),
                cell(row.values[1]),
                bar,
                if matches!(row.bar, Bar::Info) { String::new() } else { status.join("/") }
            );
            for (i, v) in row.values.iter().enumerate() {
                if let (Some(v), false) = (v, matches!(row.bar, Bar::Info)) {
                    if v.is_nan() || !pass(*v, row.bar) {
                        failures.push(format!(
                            "{}/{}/{} {}: {v:.3} (bar {bar})",
                            row.scenario,
                            row.mode,
                            row.metric,
                            ["rt", "nonrt"][i]
                        ));
                    }
                }
            }
        }
        failures
    }
}

// ---------------------------------------------------------------------------------------
// Scenarios.

/// Shared per-path context.
struct Ctx<'a> {
    path: usize,
    out: &'a Out,
    report: &'a mut Report,
}

impl Ctx<'_> {
    fn put(&mut self, scenario: &'static str, mode: &'static str, metric: &'static str, value: f64, bar: Bar) {
        self.report.put(self.path, scenario, mode, metric, value, bar);
    }
}

/// Scenario 1: Cover 5's `e_panel`-side shadow (Screenshot 1), still and under fast motion.
fn cover_shadow(arena: &mut Arena, cx: &mut Ctx) {
    const S: &str = "cover_shadow";
    let tag = arena.tag;
    let pose = cover_pose();
    let camera = pose.camera();
    // Silhouette corners of Cover 5 (x -14.3..-13.7, z -6..-4) seen from the panel spot at
    // (0, 3, -19.35): its shadow's two floor edges run from them away from the light to the
    // West wall (x = -19.5).
    let light = Vec3::new(0.0, 3.0, -19.35);
    let edge_to_wall = |corner: Vec3| {
        let d = Vec3::new(corner.x - light.x, 0.0, corner.z - light.z);
        corner + d * ((-19.5 - corner.x) / d.x)
    };
    let (a, b) = (Vec3::new(-14.3, 0.0, -6.0), Vec3::new(-13.7, 0.0, -4.0));
    let (a_end, b_end) = (edge_to_wall(a), edge_to_wall(b));
    let inward = |from: Vec3, to: Vec3, other: Vec3| {
        let u = (to - from).normalize();
        let n = Vec3::new(-u.z, 0.0, u.x);
        if n.dot(other - from) > 0.0 { n } else { -n }
    };
    // West wall's moonlight shadow crosses the floor along x = -19.5 - toward.x * (4.0 / toward.y)
    // (~-15.14 m), and the Archway left-pillar shadow of Lamp 2 crosses `near` past s = 5.5 m.
    let moon_q = glam::Quat::from_euler(glam::EulerRot::XYZ, -35f32.to_radians(), -32f32.to_radians(), 0.0);
    let moon_toward = (moon_q * Vec3::Z).normalize();
    let moon_x = -19.5 - moon_toward.x * (4.0 / moon_toward.y);
    let edges = [
        (
            "far",
            ShadowEdge {
                start: a,
                end: a_end,
                inside: inward(a, a_end, b),
                max_s: f32::INFINITY,
                skip_x: Some((moon_x - 0.15, moon_x + 0.15)),
            },
        ),
        (
            "near",
            ShadowEdge {
                start: b,
                end: b_end,
                inside: inward(b, b_end, a),
                max_s: 5.5,
                skip_x: Some((moon_x - 0.15, moon_x + 0.15)),
            },
        ),
    ];
    let shadow_roi = screen_box(
        &camera,
        &[a, b, a_end, b_end, a_end + Vec3::Y * 1.5, b_end + Vec3::Y * 1.5],
    );
    // North wall (z = -19.5) left of `e_panel` (x -2..2), above the Platform's sightline.
    let stripe_roi = inner_box(
        &camera,
        [
            Vec3::new(-7.0, 2.5, -19.5),
            Vec3::new(-2.3, 2.5, -19.5),
            Vec3::new(-2.3, 3.8, -19.5),
            Vec3::new(-7.0, 3.8, -19.5),
        ],
    );

    // a) still: 64 settled frames, then 3 more for the still noise floor.
    arena
        .engine
        .set_view(arena.view, &view_desc(Preset::Night, COVER_STOPS))
        .unwrap();
    arena.cut();
    arena.settle(pose, 64);
    let still = arena.capture();
    let sl = Luma::of(&still);
    let mut still_frames = vec![(Luma::of(&still), camera)];
    for _ in 0..3 {
        arena.step(pose);
        still_frames.push((Luma::of(&arena.capture()), camera));
    }
    for (name, edge) in &edges {
        let e = shadow_edge(&sl, &camera, edge);
        let (rough, p16, jag, contact) = match *name {
            "far" => ("far_edge_rough", "far_edge_p16", "far_edge_jag", "far_contact_w"),
            _ => ("near_edge_rough", "near_edge_p16", "near_edge_jag", "near_contact_w"),
        };
        cx.put(S, "still", rough, e.rough, Bar::Below(0.5));
        cx.put(S, "still", p16, e.p16, Bar::Below(0.25));
        cx.put(S, "still", jag, e.jag, Bar::Below(1.5));
        // A ~3 cm penumbra beside the box (0.5 m source 20 m away); 8 px leaves room for
        // the profile reaching the box's own base.
        cx.put(S, "still", contact, e.contact, Bar::Below(8.0));
        let samples_name = if *name == "far" { "far_edge_samples" } else { "near_edge_samples" };
        cx.put(S, "still", samples_name, e.samples as f64, Bar::Above(150.0));
        eprintln!("cover_shadow path={tag} edge={name} samples={}", e.samples);
    }
    let (stripe_hp, stripe_p16) = column_stripes(&sl, stripe_roi);
    cx.put(S, "still", "nwall_stripe_hp", stripe_hp, Bar::Below(0.4));
    cx.put(S, "still", "nwall_stripe_p16", stripe_p16, Bar::Below(0.25));
    let still_boil = boil(&still_frames, &camera, &sl, shadow_roi);
    cx.put(S, "still", "boil_shadow", still_boil, Bar::Below(0.5));
    cx.out.image(&format!("cover_{tag}_still"), &still);
    if cx.out.0.is_some() {
        let mut annot = still.rgba8.clone();
        outline(&mut annot, shadow_roi, [0, 255, 0]);
        outline(&mut annot, stripe_roi, [255, 0, 255]);
        for (_, edge) in &edges {
            for k in 0..=200 {
                if let Some(p) = project(&camera, edge.start.lerp(edge.end, k as f32 / 200.0)) {
                    outline(&mut annot, (p.x as usize, p.y as usize, p.x as usize + 2, p.y as usize + 2), [255, 255, 0]);
                }
            }
        }
        cx.out.png(&format!("annot_cover_{tag}"), W, H, &annot);
    }

    // b) whip_pan: +/-40 deg at 8 deg/frame, through the pose at full speed. Frames at
    // +16, +8, 0 and -8 deg feed the boil measure.
    let offsets = whip(8.0, 40.0);
    let mut boil_frames = Vec::new();
    let mut whip_img = None;
    for (i, &o) in offsets.iter().enumerate() {
        arena.step(pose.turned(o));
        if i + 3 >= offsets.len() {
            let img = arena.capture();
            boil_frames.push((Luma::of(&img), pose.turned(o).camera()));
            if i + 1 == offsets.len() {
                whip_img = Some(img);
            }
        }
    }
    arena.step(pose.turned(-8.0));
    boil_frames.push((Luma::of(&arena.capture()), pose.turned(-8.0).camera()));
    let whip_img = whip_img.unwrap();
    let wl = Luma::of(&whip_img);
    let side = revealed(pose.turned(8.0), pose);
    let (brms, bmax) = block_diff(&whip_img, &still, FULL, 24);
    let (srms, smax) = block_diff(&whip_img, &still, shadow_roi, 16);
    let (erms, _) = block_diff(&whip_img, &still, strip_roi(side), 16);
    cx.put(S, "whip_pan", "brms", brms, Bar::Below(2.5));
    cx.put(S, "whip_pan", "bmax", bmax, Bar::Below(16.0));
    cx.put(S, "whip_pan", "shadow_roi_rms", srms, Bar::Below(2.5));
    cx.put(S, "whip_pan", "shadow_roi_max", smax, Bar::Below(12.0));
    cx.put(S, "whip_pan", "edge_strip_rms", erms, Bar::Below(4.0));
    cx.put(S, "whip_pan", "boil_shadow", boil(&boil_frames, &camera, &sl, shadow_roi), Bar::Below(1.0));
    let inner = (STRIP, 0, WU - STRIP, HU);
    cx.put(S, "whip_pan", "boil_frame", boil(&boil_frames, &camera, &sl, inner), Bar::Below(1.0));
    let e = shadow_edge(&wl, &camera, &edges[0].1);
    cx.put(S, "whip_pan", "far_edge_rough", e.rough, Bar::Below(0.75));
    cx.put(S, "whip_pan", "far_edge_samples", e.samples as f64, Bar::Above(150.0));
    cx.out.image(&format!("cover_{tag}_whip_pan"), &whip_img);
    cx.out.diff(&format!("cover_{tag}_whip_pan"), &wl, &sl, &[shadow_roi, strip_roi(side)]);

    // c) whip_stop2: 2 frames after a 10 deg/frame whip pan stops on the pose.
    arena.settle(pose, 32);
    for o in whip(10.0, 40.0) {
        arena.step(pose.turned(o));
    }
    arena.settle(pose, 2);
    let stop2 = arena.capture();
    let (brms, bmax) = block_diff(&stop2, &still, FULL, 24);
    cx.put(S, "whip_stop2", "brms", brms, Bar::Below(2.0));
    cx.put(S, "whip_stop2", "bmax", bmax, Bar::Below(12.0));
    cx.out.image(&format!("cover_{tag}_whip_stop2"), &stop2);
    cx.out.diff(&format!("cover_{tag}_whip_stop2"), &Luma::of(&stop2), &sl, &[shadow_roi]);

    // d) fast_sprint: 14 m/s diagonal strafe-run with a 1 m jump arc onto the pose.
    let run = sprint(pose, 14.0, true, 1.0, 24);
    arena.cut();
    arena.settle(run[0], 32);
    for &p in &run[1..] {
        arena.step(p);
    }
    let img = arena.capture();
    let (brms, bmax) = block_diff(&img, &still, FULL, 24);
    let (srms, smax) = block_diff(&img, &still, shadow_roi, 16);
    cx.put(S, "fast_sprint", "brms", brms, Bar::Below(3.0));
    cx.put(S, "fast_sprint", "bmax", bmax, Bar::Below(18.0));
    cx.put(S, "fast_sprint", "shadow_roi_rms", srms, Bar::Below(2.5));
    cx.put(S, "fast_sprint", "shadow_roi_max", smax, Bar::Below(12.0));
    cx.out.image(&format!("cover_{tag}_fast_sprint"), &img);
    cx.out.diff(&format!("cover_{tag}_fast_sprint"), &Luma::of(&img), &sl, &[shadow_roi]);
    arena
        .engine
        .set_view(arena.view, &view_desc(Preset::Night, PLAY_STOPS))
        .unwrap();
}

/// Count of isolated `> thresh` luma deviations from the local 5x5 median inside `roi`:
/// catches cold-cut and moving-emitter penumbra speckles that 24x24/32x32 block means average out.
fn median_outliers_5x5(l: &Luma, roi: Roi, thresh: f32) -> f64 {
    let (x0, y0, x1, y1) = roi;
    let mut count = 0u64;
    let mut buf = [0.0f32; 25];
    for y in (y0 + 2)..(y1 - 2) {
        for x in (x0 + 2)..(x1 - 2) {
            let mut k = 0;
            for dy in -2isize..=2 {
                for dx in -2isize..=2 {
                    buf[k] = l.at((x as isize + dx) as usize, (y as isize + dy) as usize);
                    k += 1;
                }
            }
            buf.sort_by(f32::total_cmp);
            if (l.at(x, y) - buf[12]).abs() > thresh {
                count += 1;
            }
        }
    }
    count as f64
}

/// Scenario 2: the grazing South wall (Screenshot 2) under extreme motion.
fn south_wall(arena: &mut Arena, cx: &mut Ctx) {
    const S: &str = "south_wall";
    let tag = arena.tag;
    let pose = south_pose();
    // Regions of `arena_night_south_wall_repro` (same pose and resolution).
    const WALL: Roi = (80, 280, 520, 620);
    const OPEN_WALL: Roi = (40, 480, 420, 760);
    const COVER: Roi = (420, 650, 620, 760);
    let wall_outliers = |l: &Luma| median_outliers_5x5(l, OPEN_WALL, 5.0);
    // Wall high-pass (24 px box blur removed): std, row-mean std, column-mean std.
    let hp = |l: &Luma| -> (f64, f64, f64) {
        let (x0, y0, x1, y1) = WALL;
        let (cw, ch, r) = (x1 - x0, y1 - y0, 24usize);
        let mut integral = vec![0.0f64; (cw + 1) * (ch + 1)];
        for y in 0..ch {
            for x in 0..cw {
                integral[(y + 1) * (cw + 1) + x + 1] = f64::from(l.at(x0 + x, y0 + y)) + integral[y * (cw + 1) + x + 1]
                    + integral[(y + 1) * (cw + 1) + x]
                    - integral[y * (cw + 1) + x];
            }
        }
        let (mut all, mut rows, mut cols) = (Vec::with_capacity(cw * ch), vec![0.0; ch], vec![0.0; cw]);
        for y in 0..ch {
            for x in 0..cw {
                let (xa, xb) = (x.saturating_sub(r), (x + r).min(cw - 1) + 1);
                let (ya, yb) = (y.saturating_sub(r), (y + r).min(ch - 1) + 1);
                let sum = integral[yb * (cw + 1) + xb] - integral[ya * (cw + 1) + xb] - integral[yb * (cw + 1) + xa]
                    + integral[ya * (cw + 1) + xa];
                let v = f64::from(l.at(x0 + x, y0 + y)) - sum / ((xb - xa) * (yb - ya)) as f64;
                all.push(v);
                rows[y] += v / cw as f64;
                cols[x] += v / ch as f64;
            }
        }
        (std_dev(&all), std_dev(&rows), std_dev(&cols))
    };
    // Std of 32x32 block means of the luma difference on the wall: low-frequency blotches.
    let blotch = |a: &Luma, b: &Luma| -> f64 {
        let (x0, y0, x1, y1) = WALL;
        let mut blocks = Vec::new();
        for ty in (y0..y1 - 31).step_by(32) {
            for tx in (x0..x1 - 31).step_by(32) {
                let s: f64 = (ty..ty + 32)
                    .flat_map(|y| (tx..tx + 32).map(move |x| (x, y)))
                    .map(|(x, y)| f64::from(a.at(x, y) - b.at(x, y)))
                    .sum();
                blocks.push(s / 1024.0);
            }
        }
        std_dev(&blocks)
    };
    // Lower cover-shadow region: RMS, max 8x8 block-mean |diff|, and p99.5 per-pixel |diff|
    // (catches isolated penumbra pepper speckles hidden by 8x8 means).
    let cover = |a: &Luma, b: &Luma| -> (f64, f64, f64) {
        let (x0, y0, x1, y1) = COVER;
        let d: Vec<f64> = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| f64::from(a.at(x, y) - b.at(x, y)))
            .collect();
        let rms = (d.iter().map(|v| v * v).sum::<f64>() / d.len() as f64).sqrt();
        let mut abs_d: Vec<f64> = d.iter().map(|v| v.abs()).collect();
        abs_d.sort_by(f64::total_cmp);
        let p995 = abs_d[((abs_d.len() * 995) / 1000).min(abs_d.len() - 1)];
        let mut max = 0.0f64;
        for ty in (y0..y1 - 7).step_by(8) {
            for tx in (x0..x1 - 7).step_by(8) {
                let s: f64 = (ty..ty + 8)
                    .flat_map(|y| (tx..tx + 8).map(move |x| (x, y)))
                    .map(|(x, y)| f64::from(a.at(x, y) - b.at(x, y)))
                    .sum();
                max = max.max((s / 64.0).abs());
            }
        }
        (rms, max, p995)
    };
    let pillar_amp = |l: &Luma| -> f64 {
        let cols: Vec<f64> = (640..770)
            .map(|x| (460..560).map(|y| f64::from(l.at(x, y))).sum::<f64>() / 100.0)
            .collect();
        cols.iter().fold(f64::MIN, |m, v| m.max(*v)) - cols.iter().fold(f64::MAX, |m, v| m.min(*v))
    };

    arena.cut();
    arena.settle(pose, 64);
    let still = arena.capture();
    let sl = Luma::of(&still);
    let (h, hr, hc) = hp(&sl);
    cx.put(S, "still", "hp", h, Bar::Below(0.85));
    cx.put(S, "still", "hp_row", hr, Bar::Below(0.48));
    cx.put(S, "still", "hp_col", hc, Bar::Below(0.48));
    cx.put(S, "still", "wall_outliers", wall_outliers(&sl), Bar::Below(20.0));
    cx.put(S, "still", "pillar_amp", pillar_amp(&sl), Bar::Above(105.0));
    cx.out.image(&format!("south_{tag}_still"), &still);

    let measure = |cx: &mut Ctx, mode: &'static str, img: &Image, side: Option<Side>, cut: bool| {
        let l = Luma::of(img);
        let (h, hr, hc) = hp(&l);
        let (crms, cmax, cspeck) = cover(&l, &sl);
        cx.put(S, mode, "hp", h, Bar::Below(0.85));
        cx.put(S, mode, "hp_row", hr, Bar::Below(0.48));
        cx.put(S, mode, "hp_col", hc, Bar::Below(0.48));
        cx.put(S, mode, "blotch", blotch(&l, &sl), Bar::Below(if cut { 1.2 } else { 0.8 }));
        cx.put(S, mode, "edge_strip_std", strip_std(&l, &sl, side), Bar::Below(3.0));
        cx.put(S, mode, "cov_rms", crms, Bar::Below(6.0));
        cx.put(S, mode, "cov_max", cmax, Bar::Below(12.0));
        cx.put(S, mode, "cov_p995", cspeck, Bar::Below(if cut { 22.0 } else { 18.0 }));
        cx.put(S, mode, "wall_outliers", wall_outliers(&l), Bar::Below(20.0));
        cx.put(S, mode, "pillar_amp", pillar_amp(&l), Bar::Above(100.0));
        let (brms, bmax) = block_diff(img, &still, FULL, 24);
        cx.put(S, mode, "brms", brms, Bar::Below(if cut { 3.0 } else { 2.5 }));
        cx.put(S, mode, "bmax", bmax, Bar::Below(if cut { 20.0 } else { 16.0 }));
        cx.out.image(&format!("south_{tag}_{mode}"), img);
        cx.out.diff(&format!("south_{tag}_{mode}"), &l, &sl, &[WALL, COVER, strip_roi(side)]);
    };

    // fast_sprint: 12 m/s forward along the wall (8 m in 40 frames) onto the pose.
    let run = sprint(pose, 12.0, false, 0.0, 40);
    arena.cut();
    arena.settle(run[0], 16);
    for &p in &run[1..] {
        arena.step(p);
    }
    let img = arena.capture();
    measure(cx, "fast_sprint", &img, None, false);

    // whip_pan: +/-40 deg at 10 deg/frame through the pose.
    arena.settle(pose, 32);
    for o in whip(10.0, 40.0) {
        arena.step(pose.turned(o));
    }
    let img = arena.capture();
    measure(cx, "whip_pan", &img, revealed(pose.turned(10.0), pose), false);

    // rapid_shake: +/-12 deg reversing every 4 frames.
    arena.settle(pose, 32);
    let offsets = shake(12.0);
    for &o in &offsets {
        arena.step(pose.turned(o));
    }
    let img = arena.capture();
    let prev = offsets[offsets.len() - 2];
    measure(cx, "rapid_shake", &img, revealed(pose.turned(prev), pose), false);

    // cut2: frame 2 after a camera cut.
    arena.cut();
    arena.settle(pose, 2);
    let img = arena.capture();
    measure(cx, "cut2", &img, None, true);
}

/// Scenario 3: Day preset, sun-shadowed walls under whip pans and sprints.
fn day_whip(arena: &mut Arena, cx: &mut Ctx) {
    const S: &str = "day";
    let tag = arena.tag;
    let pose = day_pose();
    // `arena_corner`'s shadowed West wall band and corner floor, from 1280x720 to 1600x960
    // (same vertical field of view: scale about the centre by 4/3).
    let map = |(x0, y0, x1, y1): Roi| -> Roi {
        let mx = |x: usize| (800.0 + (x as f64 - 640.0) * 4.0 / 3.0) as usize;
        let my = |y: usize| (y as f64 * 4.0 / 3.0) as usize;
        (mx(x0), my(y0), mx(x1), my(y1))
    };
    let rois = [map((240, 192, 1080, 312)), map((288, 432, 600, 624))];
    let shadow = |a: &Image, b: &Image| -> (f64, f64) {
        rois.iter().map(|&r| block_diff(a, b, r, 16)).fold((0.0, 0.0), |(r0, m0), (r, m)| ((r0 * r0 + r * r).sqrt(), m0.max(m)))
    };

    arena.cut();
    arena.settle(pose, 64);
    let still = arena.capture();
    let sl = Luma::of(&still);
    cx.out.image(&format!("day_{tag}_still"), &still);

    let measure = |cx: &mut Ctx, mode: &'static str, img: &Image, side: Option<Side>| {
        let l = Luma::of(img);
        let (brms, bmax) = block_diff(img, &still, FULL, 24);
        let (erms, emax) = block_diff(img, &still, strip_roi(side), 16);
        let (srms, smax) = shadow(img, &still);
        cx.put(S, mode, "brms", brms, Bar::Below(2.5));
        cx.put(S, mode, "bmax", bmax, Bar::Below(16.0));
        cx.put(S, mode, "edge_strip_rms", erms, Bar::Below(4.0));
        cx.put(S, mode, "edge_strip_max", emax, Bar::Below(16.0));
        cx.put(S, mode, "edge_strip_std", strip_std(&l, &sl, side), Bar::Below(3.0));
        cx.put(S, mode, "shadow_rms", srms, Bar::Below(2.0));
        cx.put(S, mode, "shadow_max", smax, Bar::Below(10.0));
        cx.out.image(&format!("day_{tag}_{mode}"), img);
        let mut boxes = rois.to_vec();
        boxes.push(strip_roi(side));
        cx.out.diff(&format!("day_{tag}_{mode}"), &l, &sl, &boxes);
    };

    for o in whip(10.0, 40.0) {
        arena.step(pose.turned(o));
    }
    let img = arena.capture();
    measure(cx, "whip_pan", &img, revealed(pose.turned(10.0), pose));

    let run = sprint(pose, 14.0, true, 1.0, 24);
    arena.cut();
    arena.settle(run[0], 32);
    for &p in &run[1..] {
        arena.step(p);
    }
    let img = arena.capture();
    measure(cx, "fast_sprint", &img, None);
}

/// Scenario 4: dragging `e_panel` over the floor and platform while panning at 6 deg/frame,
/// stopping both on the reference pose.
fn moving_emitter(arena: &mut Arena, cx: &mut Ctx) {
    const S: &str = "emitter_pan";
    let tag = arena.tag;
    let pose = emitter_pose();
    let panel = arena.panel.unwrap();
    arena
        .engine
        .set_view(arena.view, &view_desc(Preset::Night, SCENE_STOPS))
        .unwrap();
    arena.engine.set_transform(panel, Mat4::from_translation(PANEL_HOME)).unwrap();
    arena.cut();
    arena.settle(pose, 36);
    let offsets = whip(6.0, 36.0);
    let last = offsets.len() - 1;
    let mid = last / 2;
    let rest = Vec3::new(2.0, 2.5, -3.5);
    let mut mid_drag = None;
    for (f, &o) in offsets.iter().enumerate() {
        // `moving_emitter_and_jump_no_rings_or_flicker`'s drag path, landing on `rest`.
        let at = if f == last {
            rest
        } else {
            let phase = f as f32 / (last - 1) as f32 * std::f32::consts::PI * 2.5;
            Vec3::new((phase * 0.5).cos() * 3.0, 2.5, -11.0 + phase.sin() * 7.5)
        };
        arena.engine.set_transform(panel, Mat4::from_translation(at)).unwrap();
        arena.step(pose.turned(o));
        if f == mid {
            mid_drag = Some(arena.capture());
        }
    }
    if let Some(mid_img) = &mid_drag {
        cx.out.image(&format!("empan_{tag}_mid_drag"), mid_img);
    }
    let stop0 = arena.capture();
    arena.settle(pose, 4);
    let stop4 = arena.capture();
    arena.settle(pose, 4);
    let stop8 = arena.capture();
    arena.cut();
    arena.settle(pose, 64);
    let reference = arena.capture();
    let rl = Luma::of(&reference);
    // North wall penumbra behind the central platform where 1-frame moving-emitter stipple appears.
    const NORTH_WALL: Roi = (440, 280, 760, 420);
    for (mode, img, bar) in [
        ("stop0", &stop0, (5.0, 32.0, 550.0)),
        ("stop4", &stop4, (3.0, 20.0, 450.0)),
        ("stop8", &stop8, (2.0, 12.0, 400.0)),
    ] {
        let l = Luma::of(img);
        let (brms, bmax) = block_diff(img, &reference, FULL, 24);
        let outliers = median_outliers_5x5(&l, NORTH_WALL, 5.0);
        cx.put(S, mode, "brms", brms, Bar::Below(bar.0));
        cx.put(S, mode, "bmax", bmax, Bar::Below(bar.1));
        cx.put(S, mode, "wall_outliers", outliers, Bar::Below(bar.2));
        cx.out.image(&format!("empan_{tag}_{mode}"), img);
        cx.out.diff(&format!("empan_{tag}_{mode}"), &l, &rl, &[NORTH_WALL]);
    }
    cx.out.image(&format!("empan_{tag}_ref"), &reference);
}

/// Scenario 5: mean wall-clock ms/frame over 32 moving frames (7 m/s strafe-run turning
/// 1.5 deg/frame) after 8 warm-up frames, without capture readback: the closing capture's
/// cost is measured again on the idle GPU and subtracted.
fn frame_time(arena: &mut Arena, pose: Pose) -> f64 {
    let run: Vec<Pose> = sprint(pose, 7.0, true, 0.0, 40)
        .into_iter()
        .enumerate()
        .map(|(i, p)| p.turned(i as f32 * 1.5 - 30.0))
        .collect();
    arena.cut();
    for &p in &run[..8] {
        arena.step(p);
    }
    arena.capture();
    let start = Instant::now();
    for &p in &run[8..40] {
        arena.step(p);
    }
    arena.capture();
    let total = start.elapsed();
    let start = Instant::now();
    arena.capture();
    let readback = start.elapsed();
    (total.saturating_sub(readback)).as_secs_f64() * 1000.0 / 32.0
}

/// Times 32 continuous frames with an actively moving emitter (`e_panel` sweeping back and forth
/// every frame) to verify the moving-emitter GI relight budget stays bounded without frame-time cliffs.
fn emitter_frame_time(arena: &mut Arena, pose: Pose) -> f64 {
    let panel = arena.panel.expect("Night arena has e_panel");
    arena
        .engine
        .set_view(arena.view, &view_desc(Preset::Night, SCENE_STOPS))
        .unwrap();
    arena.cut();
    for i in 0..8 {
        let t = (i as f32 / 8.0) * std::f32::consts::TAU;
        arena
            .engine
            .set_transform(panel, Mat4::from_translation(Vec3::new(t.sin() * 4.0, 2.5, -5.0)))
            .unwrap();
        arena.step(pose);
    }
    arena.capture();
    let start = Instant::now();
    for i in 8..40 {
        let t = (i as f32 / 8.0) * std::f32::consts::TAU;
        arena
            .engine
            .set_transform(panel, Mat4::from_translation(Vec3::new(t.sin() * 4.0, 2.5, -5.0)))
            .unwrap();
        arena.step(pose);
    }
    arena.capture();
    let total = start.elapsed();
    let start = Instant::now();
    arena.capture();
    let readback = start.elapsed();
    (total.saturating_sub(readback)).as_secs_f64() * 1000.0 / 32.0
}

#[test]
#[ignore]
fn fast_motion_and_shadow_quality_harness() {
    let report_only = std::env::var("RENDERLAB_HARNESS_REPORT").is_ok_and(|v| v == "1");
    let out = Out(std::env::var_os("RENDERLAB_HARNESS_OUT").map(PathBuf::from));
    if let Some(dir) = &out.0 {
        std::fs::create_dir_all(dir).unwrap();
    }
    let profile = if cfg!(debug_assertions) {
        "dev (debug assertions; renderlab opt-level 1) - not a release frame time"
    } else {
        "release"
    };
    let mut report = Report::default();
    let started = Instant::now();
    for (index, path) in [RenderPath::HardwareRt, RenderPath::NonRt].into_iter().enumerate() {
        let mut cx = Ctx {
            path: index,
            out: &out,
            report: &mut report,
        };
        let mut night = Arena::new(path, Preset::Night);
        let first = night.step(cover_pose());
        eprintln!(
            "harness path={path:?} primary={:?} soft_shadows={:?} ({:.0} s)",
            first.primary,
            first.soft_shadows,
            started.elapsed().as_secs_f64()
        );
        cover_shadow(&mut night, &mut cx);
        south_wall(&mut night, &mut cx);
        let perf_bar = |limit: f64| if cfg!(debug_assertions) { Bar::Info } else { Bar::Below(limit) };
        let ms = frame_time(&mut night, cover_pose());
        cx.put("perf", "night_moving", "ms_per_frame", ms, perf_bar(10.5));
        moving_emitter(&mut night, &mut cx);
        let em_ms = emitter_frame_time(&mut night, emitter_pose());
        cx.put("perf", "emitter_moving", "ms_per_frame", em_ms, perf_bar(16.0));
        drop(night);

        let mut day = Arena::new(path, Preset::Day);
        day_whip(&mut day, &mut cx);
        let ms = frame_time(&mut day, day_pose());
        cx.put("perf", "day_moving", "ms_per_frame", ms, perf_bar(6.0));
        eprintln!("harness path={path:?} done ({:.0} s)", started.elapsed().as_secs_f64());
    }
    eprintln!("\nfast-motion harness: {W}x{H}, build profile: {profile}");
    let failures = report.print();
    eprintln!("\n{} failed checks", failures.len());
    for f in &failures {
        eprintln!("  FAIL {f}");
    }
    if !report_only {
        assert!(failures.is_empty(), "{} fast-motion quality checks failed:\n{}", failures.len(), failures.join("\n"));
    }
}
