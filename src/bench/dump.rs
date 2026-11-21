//! A game scene from disk: an engine scene dump (`RENDERLAB_SCENE_DUMP`, src/engine/scene_dump.rs)
//! of Glint, walked at the game's walking speed into the dump's own view and scored there against
//! its path-traced reference. Nothing is committed: the dump and reference live under
//! D:/Coding/renderlab-work/bench-scenes/<scene> (`LIGHTBENCH_GLINT`), with `scene.rla`,
//! `scene.json`, the sky map `scene.json` names, and `reference.pfm`.
//!
//! The camera starts `LIGHTBENCH_WALK_SECONDS` (default 3) of walking behind the dump's view,
//! settles there `LIGHTBENCH_DUMP_SETTLE` frames (default 30), walks at `LIGHTBENCH_WALK_SPEED`
//! m/s (default 4.5) in 60 Hz frames into the view, then holds it for `LIGHTBENCH_DUMP_HOLD`
//! frames (default 90). Per walking frame it reads the cache's counters (`series`: rays,
//! gather_rays, light_rays, created, queued). Its metrics:
//! - `arrive_energy`, `arrive_err`: the first frame at the view, against the reference: the
//!   cache as walking left it;
//! - `hold_energy`, `hold_err`: the last held frame;
//! - `converge_frames`: held frames until the marked region (the view's central 20%) is within
//!   2% of the reference there, -1 if never; `arrive_region` and `hold_region` are its ratio.
//!
//! The bench's per-frame cache milliseconds cover the walking frames (`frames_measured`).
//!
//! `glint-course` rolls Glint's own course instead, the walk the play gate reads (about 20 ms of
//! cache in a 28.3 ms frame on 20 Nov): the ball's path as the bot drove it (bot_probe positions of
//! an uninterrupted run, u-sunrt/roll/bd1), at 9.5 m/s (`LIGHTBENCH_COURSE_SPEED`) in the game's
//! 33 Hz frames (`LIGHTBENCH_COURSE_FPS`: 0.29 m a frame, the step that sets new cells per frame),
//! seen by Glint's course camera (5.4 m behind, 2.2 m up, 16 degrees down; levels/glint.scene).
//! It has no reference, so it prints the cost, rays, cells and queue only.
use super::{BenchPath, Fixture, RefImage, kit};
use crate::renderer::Renderer;
use crate::{
    Camera, Error, IndirectSettings, Light, Lighting, ProgramDescriptor, Reconstruction, ShadowRequest,
    Shadows, SoftShadows, Value, Vec3, ViewMode, Visibility,
};
use std::path::PathBuf;

#[allow(dead_code)]
#[path = "../../viewer/src/prepared.rs"]
mod prepared;

const SECONDS_PER_FRAME: f32 = 1.0 / 60.0;
/// The ball's path on Glint's course: bot_probe positions from an uninterrupted roll (ticks
/// 570-1440 of u-sunrt/roll/bd1/glint-nort-1920_1080-course).
const COURSE: [[f32; 3]; 5] = [[-8.0, 0.5, -2.3], [-22.1, 0.5, 9.5], [-33.0, 0.5, 18.7], [-43.8, 0.5, 27.7], [-47.6, -0.9, 32.1]];
/// Glint's course camera (levels/glint.scene): behind and above the ball, pitched down.
const CAMERA_DISTANCE: f32 = 5.4;
const CAMERA_HEIGHT: f32 = 2.2;
const CAMERA_PITCH_DEGREES: f32 = 16.0;

/// The standard program with a per-material `glow` (cd/m^2), shown and sent into the lighting
/// cache through `emission`: the same edits Ara's emissive program makes, without its emissive
/// maps.
fn emissive_source() -> Result<String, Error> {
    let standard = crate::STANDARD_PROGRAM;
    let edits = [
        ("alpha:f32 };", "alpha:f32, glow:vec3<f32> };"),
        ("decal_mr.x,alpha);", "decal_mr.x,alpha,max(params.glow,vec3<f32>(0.0)));"),
        ("    return exposed;", "    return min(exposed+exposed_product(s.glow,vec3<f32>(1.0)),vec3<f32>(65504.0));"),
    ];
    let mut source = standard.to_owned();
    for (from, to) in edits {
        if source.matches(from).count() != 1 {
            return Err(Error::InvalidInput("the standard program no longer has the shape the bench's emissive edits expect"));
        }
        source = source.replacen(from, to, 1);
    }
    // The material pass's record carries the glow, where the standard program has one.
    source = source
        .replace("s.n,s.base,s.alpha,s.metal,s.rough,vec3<f32>(0.0),", "s.n,s.base,s.alpha,s.metal,s.rough,s.glow,")
        .replace("    s.alpha=m.alpha;\n    return s;", "    s.alpha=m.alpha;\n    s.glow=m.emission;\n    return s;");
    Ok(format!(
        "struct Parameters {{ glow:vec3<f32> }};\nconst DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.0));\n{source}\n\
         fn emission(s:Surface, input:SurfaceInput) -> vec3<f32> {{ return max(params.glow,vec3<f32>(0.0)); }}\n"
    ))
}

/// A PFM's RGB, rows top first.
fn read_pfm(path: &std::path::Path) -> Option<RefImage> {
    let bytes = std::fs::read(path).ok()?;
    let mut fields = vec![];
    let mut at = 0;
    while fields.len() < 4 {
        let start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..at]).into_owned());
        at += 1;
    }
    let (w, h): (usize, usize) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
    let little = fields[3].parse::<f32>().ok()? < 0.0;
    let floats: Vec<f32> = bytes[at..]
        .chunks_exact(4)
        .map(|b| {
            let b: [u8; 4] = b.try_into().unwrap();
            if little { f32::from_le_bytes(b) } else { f32::from_be_bytes(b) }
        })
        .collect();
    (fields[0] == "PF" && floats.len() >= w * h * 3).then(|| RefImage {
        size: [w as u32, h as u32],
        rgb: (0..h).rev().flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| {
            let i = (y * w + x) * 3;
            [floats[i], floats[i + 1], floats[i + 2]]
        }).collect(),
    })
}

fn setting(key: &str, default: f32) -> f32 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn json_vec3(v: &serde_json::Value) -> Option<Vec3> {
    Some(Vec3::new(v[0].as_f64()? as f32, v[1].as_f64()? as f32, v[2].as_f64()? as f32))
}

struct Loaded {
    asset: prepared::Prepared,
    _program: crate::Program,
    target: wgpu::Texture,
    size: [u32; 2],
    eye: Vec3,
    forward: Vec3,
    fov: f32,
    exposure: f32,
    reference: Option<RefImage>,
    triangles: u64,
}
pub(crate) struct Dump {
    name: &'static str,
    /// Roll Glint's course (`glint-course`) instead of walking into the dump's view.
    course: bool,
    dir: PathBuf,
    loaded: Option<Loaded>,
    series: Vec<(String, Vec<f64>)>,
}
impl Dump {
    pub(crate) fn glint() -> Self {
        let dir = std::env::var("LIGHTBENCH_GLINT").unwrap_or_else(|_| "D:/Coding/renderlab-work/bench-scenes/glint-goal".into());
        Self { name: "glint", course: false, dir: dir.into(), loaded: None, series: vec![] }
    }
    pub(crate) fn glint_course() -> Self {
        Self { name: "glint-course", course: true, ..Self::glint() }
    }
}

/// One run's cameras, (eye, forward) per frame: settling still, walking (the range returned),
/// then holding. The goal walk ends at the dump's own view; the course follows the ball.
fn cameras(course: bool, eye: Vec3, forward: Vec3, settle: usize, hold: usize) -> (Vec<(Vec3, Vec3)>, std::ops::Range<usize>) {
    let mut frames = vec![];
    if course {
        let speed = setting("LIGHTBENCH_COURSE_SPEED", 9.5);
        let step = speed / setting("LIGHTBENCH_COURSE_FPS", 33.0);
        let pitch = CAMERA_PITCH_DEGREES.to_radians();
        let mut ball = Vec3::from(COURSE[0]);
        let mut walk = vec![];
        for target in COURSE[1..].iter().map(|&p| Vec3::from(p)) {
            let dir = Vec3::new(target.x - ball.x, 0.0, target.z - ball.z).normalize();
            while ball.distance(target) > step {
                ball += (target - ball).normalize() * step;
                walk.push((ball - dir * CAMERA_DISTANCE + Vec3::Y * CAMERA_HEIGHT, dir * pitch.cos() - Vec3::Y * pitch.sin()));
            }
        }
        frames.extend(std::iter::repeat_n(walk[0], settle));
        let walked = settle..settle + walk.len();
        frames.extend(walk);
        return (frames, walked);
    }
    let speed = setting("LIGHTBENCH_WALK_SPEED", 4.5);
    let walked = (setting("LIGHTBENCH_WALK_SECONDS", 3.0) / SECONDS_PER_FRAME).round().max(1.0) as usize;
    // Walking on the ground plane (y up) towards the view's eye, looking where it looks.
    let step = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero() * speed * SECONDS_PER_FRAME;
    let start = eye - step * walked as f32;
    for frame in 0..settle + walked + hold {
        frames.push((start + step * frame.saturating_sub(settle).min(walked) as f32, forward));
    }
    (frames, settle..settle + walked)
}

impl Fixture for Dump {
    fn name(&self) -> &str {
        self.name
    }
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error> {
        let bad = |_| Error::InvalidInput("bench scene dump: unreadable scene.json, scene.rla or sky");
        let scene: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.dir.join("scene.json")).map_err(bad)?).map_err(|_| Error::InvalidInput("bench scene dump: scene.json"))?;
        // The dump's light, as the game drew it: sun, local lights, sky map and exposure.
        let mut lights = vec![];
        if let (Some(to), Some(lux)) = (json_vec3(&scene["sun"]["to_light"]), json_vec3(&scene["sun"]["illuminance"])) {
            lights.push(Light::Directional { travel: -to.normalize(), lux: lux.to_array() });
        }
        for l in scene["lights"].as_array().into_iter().flatten() {
            if let (Some(position), Some(candela)) = (json_vec3(&l["position"]), json_vec3(&l["candela"])) {
                lights.push(Light::Point {
                    position,
                    candela: candela.to_array(),
                    range: l["range"].as_f64().map(|v| v as f32),
                    source_radius: l["radius"].as_f64().unwrap_or(0.0) as f32,
                    cast_shadows: None,
                });
            }
        }
        let exposure = scene["exposure_stops"].as_f64().unwrap_or(0.0) as f32;
        if path == BenchPath::NoRt {
            r.set_soft_shadows(SoftShadows::Virtual);
        }
        // The reference was traced without fog; the dump's sky is its map.
        r.set_atmosphere(crate::Atmosphere::off())?;
        // One request per light: the sun's cascades (lights[0], which the virtual clipmap needs),
        // the local lights' shadows left to the engine's ranking (`cast_shadows: None`).
        let requests = lights
            .iter()
            .map(|l| match l {
                Light::Directional { .. } => ShadowRequest::Directional { split_ends: vec![4.0, 12.0, 40.0] },
                _ => ShadowRequest::None,
            })
            .collect();
        r.set_lighting_and_shadows(
            &Lighting { lights, environment: [0.0; 3], exposure_stops: exposure },
            &Shadows {
                directional_disk_angle: 2.0 * scene["sun"]["angular_radius"].as_f64().unwrap_or(0.0047) as f32,
                requests,
                ..Default::default()
            },
        )?;
        if let Some(file) = scene["sky"]["file"].as_str() {
            let bytes = std::fs::read(self.dir.join(file)).map_err(bad)?;
            let limits = crate::asset::environment::Limits {
                max_dimension: 4096,
                max_texels: 64 * 1024 * 1024,
                max_payload_bytes: 256 * 1024 * 1024,
            };
            let map = crate::asset::environment::decode(&bytes, limits).map_err(|_| Error::InvalidInput("bench scene dump: sky map"))?;
            let map = kit::admitted(r, "bench sky map", |r| r.try_upload_environment(&map))?;
            r.set_environment(Some(&map), crate::EnvironmentParameters::default())?;
        }
        r.set_reconstruction(Reconstruction::off())?;
        r.set_indirect(IndirectSettings::default())?;
        // The scene: each primitive's triangles kept for the cache's software rays (the
        // uploader moves them into the meshes), then uploaded.
        let limits = crate::asset::Limits {
            max_file_bytes: 2 << 30,
            max_decoded_bytes: 4 << 30,
            ..crate::asset::Limits::default()
        };
        let bytes = std::fs::read(self.dir.join("scene.rla")).map_err(bad)?;
        let source = crate::asset::decode(&bytes, limits).map_err(|_| Error::InvalidInput("bench scene dump: scene.rla"))?;
        let geometry: Vec<(Vec<[f32; 3]>, Vec<u32>)> = source
            .meshes
            .iter()
            .flat_map(|m| m.primitives.iter().map(|p| (p.vertices.iter().map(|v| v.position).collect(), p.indices.clone())))
            .collect();
        let triangles = geometry.iter().map(|(_, i)| i.len() as u64 / 3).sum();
        let mut asset = prepared::Prepared::new_with_limits(source, limits, 1_000_000)
            .map_err(|_| Error::InvalidInput("bench scene dump: the asset does not fit the uploader"))?;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !asset.ready() {
            asset.advance(r)?;
            r.collect()?;
            if std::time::Instant::now() > until {
                return Err(Error::InvalidInput("bench scene dump: upload took over 120 s"));
            }
        }
        for (mesh, (positions, indices)) in asset.meshes_mut().iter().zip(&geometry) {
            r.set_indirect_geometry(mesh, positions, indices)?;
        }
        // Emitting materials glow with the dump's emission; the rest are standard surfaces.
        let mut glow = std::collections::HashMap::new();
        for e in scene["emission"].as_array().into_iter().flatten() {
            if let (Some(i), Some(g)) = (e[0].as_u64(), Some([1, 2, 3].map(|k| e[k].as_f64().unwrap_or(0.0) as f32))) {
                glow.insert(i as usize, g);
            }
        }
        let program = r.try_create_program(&ProgramDescriptor { label: "bench-emissive", source: &emissive_source()? })?;
        asset.set_programs_with(r, |i| (&program, vec![("glow", Value::Vec3(glow.get(&i).copied().unwrap_or([0.0; 3])))]))?;
        let size = [scene["size"][0].as_u64().unwrap_or(1920) as u32, scene["size"][1].as_u64().unwrap_or(1080) as u32];
        let eye = json_vec3(&scene["camera"]["eye"]).ok_or(Error::InvalidInput("bench scene dump: camera"))?;
        let target = json_vec3(&scene["camera"]["target"]).ok_or(Error::InvalidInput("bench scene dump: camera"))?;
        let reference = read_pfm(&self.dir.join("reference.pfm")).filter(|r| r.size == size);
        self.loaded = Some(Loaded {
            asset,
            _program: program,
            target: kit::color_target(r, size),
            size,
            eye,
            forward: (target - eye).normalize(),
            fov: scene["camera"]["fov_y"].as_f64().unwrap_or(0.96) as f32,
            exposure,
            reference,
            triangles,
        });
        Ok(())
    }
    fn run(&mut self, r: &mut Renderer, _path: BenchPath, _reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let l = self.loaded.as_mut().ok_or(Error::InvalidInput("dump fixture not loaded"))?;
        r.reset_indirect();
        let settle = setting("LIGHTBENCH_DUMP_SETTLE", 30.0) as usize;
        let hold = if self.course { 0 } else { setting("LIGHTBENCH_DUMP_HOLD", 90.0) as usize };
        let (frames, walking) = cameras(self.course, l.eye, l.forward, settle, hold);
        let mut series: [Vec<f64>; 5] = Default::default();
        let (w, h) = (l.size[0] as usize, l.size[1] as usize);
        let region = |p: &[[f32; 3]]| {
            let (x0, x1, y0, y1) = (w * 2 / 5, w * 3 / 5, h * 2 / 5, h * 3 / 5);
            let mut sum = 0.0;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += f64::from(kit::luminance(p[y * w + x]));
                }
            }
            sum / ((x1 - x0) * (y1 - y0)) as f64
        };
        let score = |p: &[[f32; 3]], reference: &RefImage| {
            let lum = |q: [f32; 3]| f64::from(kit::luminance(q));
            let ours: f64 = p.iter().map(|&q| lum(q)).sum();
            let theirs: f64 = reference.rgb.iter().map(|&q| lum(q)).sum();
            let err = p.iter().zip(&reference.rgb).map(|(&a, &b)| (lum(a) - lum(b)).abs()).sum::<f64>() / theirs.max(1e-9);
            (ours / theirs.max(1e-9), err)
        };
        let mut metrics = vec![];
        let mut converge = -1.0;
        // RENDERLAB_GI_WIDE_COUNT: closest and any-hit rays and their nodes and triangles visited.
        let mut visits = [0u64; 6];
        for (frame, &(at, forward)) in frames.iter().enumerate() {
            let camera = Camera::look_at(at, at + forward, Vec3::Y, l.fov, 0.1, 2000.0)?;
            let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let mut draws = l.asset.draws(Visibility::Frustum);
                if r.try_render_view(&l.target, &camera, &mut draws, ViewMode::Lit)?.is_some() {
                    break;
                }
                drop(draws);
                if std::time::Instant::now() > until {
                    return Err(Error::InvalidInput("bench frame: not admitted in 20 s"));
                }
                kit::drain(r)?;
            }
            if walking.contains(&frame) {
                let c = r.indirect_counters()?.unwrap_or_default();
                for (s, v) in visits.iter_mut().zip(c.wide) {
                    *s += u64::from(v);
                }
                for (values, v) in series.iter_mut().zip([c.rays, c.gather_rays, c.light_rays, c.created, c.bounce_queued]) {
                    values.push(f64::from(v));
                }
            }
            let held = frame.checked_sub(walking.end);
            if let (Some(k), Some(reference)) = (held, l.reference.as_ref()) {
                // Scored at arrival, until the region converges, and on the last held frame.
                let last = k + 1 == hold;
                let k = k as u32;
                if k == 0 || last || converge < 0.0 {
                    let pixels = kit::capture_radiance(r, l.exposure)?;
                    if converge < 0.0 && (region(&pixels) / region(&reference.rgb) - 1.0).abs() <= 0.02 {
                        converge = f64::from(k);
                    }
                    if k == 0 || last {
                        let (energy, err) = score(&pixels, reference);
                        let tag = if k == 0 { "arrive" } else { "hold" };
                        metrics.push((format!("{tag}_energy"), energy));
                        metrics.push((format!("{tag}_err"), err));
                        metrics.push((format!("{tag}_region"), region(&pixels) / region(&reference.rgb)));
                    }
                }
            }
        }
        if l.reference.is_some() && !self.course {
            metrics.push(("converge_frames".into(), converge));
        }
        let per = |n: u64, d: u64| if d == 0 { 0.0 } else { n as f64 / d as f64 };
        let visited = [
            ("closest_nodes/ray", per(visits[1], visits[0])),
            ("closest_tris/ray", per(visits[2], visits[0])),
            ("anyhit_nodes/ray", per(visits[4], visits[3])),
            ("anyhit_tris/ray", per(visits[5], visits[3])),
        ];
        if visits[0] + visits[3] > 0 {
            metrics.extend(visited.map(|(k, v)| (k.to_owned(), v)));
        }
        metrics.push(("triangles".into(), l.triangles as f64));
        metrics.push(("geometry_bytes".into(), r.indirect_stats().geometry_bytes as f64));
        let walked_m: f32 = frames[walking.clone()].windows(2).map(|w| w[0].0.distance(w[1].0)).sum();
        metrics.push(("walked_m".into(), f64::from(walked_m)));
        metrics.push(("walk_frames".into(), walking.len() as f64));
        // The bench's pass window: the walking frames, before the held ones.
        metrics.push(("frames_measured".into(), walking.len() as f64));
        metrics.push(("frames_after".into(), hold as f64));
        self.series = ["rays", "gather_rays", "light_rays", "created", "queued"].into_iter().map(str::to_owned).zip(series).collect();
        Ok(metrics)
    }
    fn series(&mut self) -> Vec<(String, Vec<f64>)> {
        std::mem::take(&mut self.series)
    }
}
