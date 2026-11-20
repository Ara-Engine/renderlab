//! The sun-shadow edge fixture: a grey floor under a soft sun, with three casters whose edges
//! the shadow lookups get wrong in different ways. The answer is exact: per floor pixel, the
//! share of the sun's disk the casters leave open, cast on the CPU once at load.
//!
//! - **Rail:** a long bar high above the floor, turned against the map's texel grid. Its
//!   shadow's edge is wide (the disk at its height spans several texels) and straight, so a
//!   staircase in it shows as wobble.
//! - **Block:** a box resting on the floor. Its shadow starts at its foot (a contact edge,
//!   where the disk's cone is narrower than a texel).
//! - **Wire:** a 5 mm bar, thinner than the coarse texels its shadow is read from.
//!
//! Visibility per pixel is the shadowed frame's luminance over the unshadowed frame's (the sun
//! only: no sky, no indirect light), so the shading's own terms cancel. Metrics, floor pixels
//! only:
//! - `err`: mean |visibility - exact| over the edge band (pixels within 3 of one whose exact
//!   visibility is strictly between 0.02 and 0.98).
//! - `lit_in_shadow`: pixels inside the exact umbra (exact under 0.05 over a 5x5 window) that
//!   read over 0.5.
//! - `rail_wobble_px`: rms, about their mean, of the rail shadow's 50% crossing per image row
//!   against the exact crossing on that row.
//! - `contact_gap_px`: pixels of the block's exact contact umbra (exact under 0.05 within 4 pixels
//!   of the block's footprint) that read over 0.5.
//! - `thin_missing`: share of the wire's exact shadow (exact under 0.5 from the wire alone) that
//!   reads over 0.8.
//! - `mask_mismatch`: floor pixels (as the CPU casts them) that the unshadowed frame shows under
//!   half the floor's median luminance: the camera agreement check, near 0 (only outlines).
//!
//! Frames: LIGHTBENCH_SHADOW_FRAMES (default 8) per run with temporal reconstruction on, as the
//! engine renders; the unshadowed frame is captured once at load.
use super::{BenchPath, Fixture, RefImage, kit};
use crate::renderer::Renderer;
use crate::{
    Camera, Error, IndirectSettings, Light, Lighting, Reconstruction, ShadowRequest, Shadows, SoftShadows, Vec3,
    VirtualShadowSettings,
};
use glam::Quat;

const SIZE: [u32; 2] = [256, 256];
const FOV: f32 = 0.9;
const EYE: Vec3 = Vec3::new(0.0, 2.4, 2.0);
const TARGET: Vec3 = Vec3::new(0.0, 0.0, 0.1);
/// The sun: toward it, its angular radius (a wide cone at the rail's height) and illuminance.
const TO_SUN: Vec3 = Vec3::new(0.3, 0.8, -0.55);
const SUN_RADIUS: f32 = 0.02;
const LUX: f32 = 1000.0;
/// Disk directions per floor pixel for the exact answer.
const DISK_SAMPLES: usize = 192;
const FLOOR_HALF: f32 = 2.0;

/// An oriented box: centre, turn about +y (radians) and half-extents.
#[derive(Clone, Copy)]
struct Caster {
    centre: Vec3,
    yaw: f32,
    half: Vec3,
}
impl Caster {
    fn faces(&self) -> Vec<([Vec3; 4], Vec3)> {
        let turn = Quat::from_rotation_y(self.yaw);
        kit::box_faces(Vec3::ZERO, self.half, false)
            .into_iter()
            .map(|(corners, n)| (corners.map(|c| self.centre + turn * c), turn * n))
            .collect()
    }
    /// Whether the ray `o + t d`, t > 0, meets the box (slabs in the box's frame).
    fn hit(&self, o: Vec3, d: Vec3) -> Option<f32> {
        let back = Quat::from_rotation_y(-self.yaw);
        let o = back * (o - self.centre);
        let d = back * d;
        let (mut t0, mut t1) = (1e-4f32, f32::INFINITY);
        for k in 0..3 {
            let (ok, dk, hk) = (o[k], d[k], self.half[k]);
            if dk.abs() < 1e-12 {
                if ok.abs() > hk {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-hk - ok) / dk, (hk - ok) / dk);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        (t0 <= t1).then_some(t0)
    }
}

fn casters() -> [Caster; 3] {
    [
        // Rail: 3 m long, 4 cm square, 0.9 m up, turned 27 degrees.
        Caster { centre: Vec3::new(-0.2, 0.9, -0.6), yaw: 0.47, half: Vec3::new(1.5, 0.02, 0.02) },
        // Block: a 0.3 m cube resting on the floor.
        Caster { centre: Vec3::new(0.55, 0.15, 0.55), yaw: 0.2, half: Vec3::splat(0.15) },
        // Wire: 1.4 m long, 5 mm square, 0.35 m up, turned 8 degrees.
        Caster { centre: Vec3::new(-0.55, 0.35, 0.75), yaw: -0.14, half: Vec3::new(0.7, 0.0025, 0.0025) },
    ]
}

/// The exact answer per pixel: whether it sees the floor, and the open share of the disk with
/// all casters, the rail alone and the wire alone.
struct Exact {
    floor: Vec<bool>,
    all: Vec<f32>,
    rail: Vec<f32>,
    wire: Vec<f32>,
    /// Pixels the block covers (its footprint seen from the camera).
    block: Vec<bool>,
    /// Pixels that see any geometry (floor or caster).
    any: Vec<bool>,
}

fn camera() -> Result<Camera, Error> {
    Camera::look_at(EYE, TARGET, Vec3::Y, FOV, 0.01, 100.0)
}

fn exact() -> Exact {
    let casters = casters();
    let n = (SIZE[0] * SIZE[1]) as usize;
    let mut out = Exact { floor: vec![false; n], all: vec![1.0; n], rail: vec![1.0; n], wire: vec![1.0; n], block: vec![false; n], any: vec![false; n] };
    let forward = (TARGET - EYE).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let tan = (FOV * 0.5).tan();
    let aspect = SIZE[0] as f32 / SIZE[1] as f32;
    // Disk directions (golden-angle spiral, uniform by area) around the sun's direction.
    let s = TO_SUN.normalize();
    let e1 = s.cross(Vec3::Z).normalize();
    let e2 = s.cross(e1);
    let dirs: Vec<Vec3> = (0..DISK_SAMPLES)
        .map(|k| {
            let r = ((k as f32 + 0.5) / DISK_SAMPLES as f32).sqrt() * SUN_RADIUS.tan();
            let a = k as f32 * 2.399_963;
            (s + r * (a.cos() * e1 + a.sin() * e2)).normalize()
        })
        .collect();
    for j in 0..SIZE[1] {
        for i in 0..SIZE[0] {
            let x = (2.0 * (i as f32 + 0.5) / SIZE[0] as f32 - 1.0) * tan * aspect;
            let y = (1.0 - 2.0 * (j as f32 + 0.5) / SIZE[1] as f32) * tan;
            let d = (forward + x * right + y * up).normalize();
            let p = (j * SIZE[0] + i) as usize;
            let nearest = casters.iter().enumerate().filter_map(|(k, c)| c.hit(EYE, d).map(|t| (k, t))).min_by(|a, b| a.1.total_cmp(&b.1));
            let t_floor = if d.y < -1e-6 { -EYE.y / d.y } else { f32::INFINITY };
            if let Some((k, t)) = nearest
                && t < t_floor
            {
                out.block[p] = k == 1;
                out.any[p] = true;
                continue;
            }
            let point = EYE + d * t_floor;
            if !t_floor.is_finite() || point.x.abs() > FLOOR_HALF || point.z.abs() > FLOOR_HALF {
                continue;
            }
            out.floor[p] = true;
            out.any[p] = true;
            let (mut all, mut rail, mut wire) = (0usize, 0usize, 0usize);
            for &l in &dirs {
                let blocked: Vec<bool> = casters.iter().map(|c| c.hit(point, l).is_some()).collect();
                all += usize::from(!blocked.iter().any(|&b| b));
                rail += usize::from(!blocked[0]);
                wire += usize::from(!blocked[2]);
            }
            let k = DISK_SAMPLES as f32;
            out.all[p] = all as f32 / k;
            out.rail[p] = rail as f32 / k;
            out.wire[p] = wire as f32 / k;
        }
    }
    out
}

fn frames() -> u32 {
    std::env::var("LIGHTBENCH_SHADOW_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(8).max(1)
}

fn lighting() -> Lighting {
    Lighting { lights: vec![Light::Directional { travel: -TO_SUN.normalize(), lux: [LUX; 3] }], environment: [0.0; 3], exposure_stops: 0.0 }
}
fn shadows(on: bool) -> Shadows {
    Shadows {
        requests: vec![if on { ShadowRequest::Directional { split_ends: vec![4.0, 12.0, 40.0, 1000.0] } } else { ShadowRequest::None }],
        directional_disk_angle: if on { SUN_RADIUS } else { 0.0 },
        ..Default::default()
    }
}

#[derive(Default)]
pub(crate) struct ShadowEdges {
    loaded: Option<Loaded>,
}
struct Loaded {
    parts: Vec<kit::Part>,
    target: wgpu::Texture,
    exact: Exact,
    /// The unshadowed frame's luminance per pixel.
    open: Vec<f32>,
}

impl Fixture for ShadowEdges {
    fn name(&self) -> &str {
        "shadow-edges"
    }
    fn scene_key(&self) -> String {
        format!("shadow-edges-2 {SIZE:?} {TO_SUN} {SUN_RADIUS}")
    }
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error> {
        r.set_indirect(IndirectSettings::off())?;
        r.set_reconstruction(Reconstruction::default())?;
        match path {
            BenchPath::Rt => r.set_soft_shadows(SoftShadows::Interactive),
            BenchPath::NoRt => r.set_virtual_shadow_settings(VirtualShadowSettings::default())?,
        }
        let program = r.standard_program().clone();
        let mut faces = vec![kit::quad(Vec3::ZERO, Vec3::Y, Vec3::X * FLOOR_HALF, FLOOR_HALF)];
        let mut parts = vec![kit::part(r, &faces, &program, [0.5; 3], &[])?];
        for caster in casters() {
            faces = caster.faces();
            parts.push(kit::part(r, &faces, &program, [0.5; 3], &[])?);
        }
        let target = kit::color_target(r, SIZE);
        let camera = camera()?;
        // The unshadowed frame, once: the sun with no shadow request.
        r.set_lighting_and_shadows(&lighting(), &shadows(false))?;
        for _ in 0..frames() {
            kit::frame(r, &target, &camera, &mut parts)?;
        }
        let open = kit::capture_radiance(r, 0.0)?.into_iter().map(kit::luminance).collect();
        r.set_lighting_and_shadows(&lighting(), &shadows(true))?;
        self.loaded = Some(Loaded { parts, target, exact: exact(), open });
        Ok(())
    }
    fn run(&mut self, r: &mut Renderer, _path: BenchPath, _reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let l = self.loaded.as_mut().ok_or(Error::InvalidInput("shadow-edges fixture not loaded"))?;
        let camera = camera()?;
        let n = frames();
        for _ in 0..n {
            kit::frame(r, &l.target, &camera, &mut l.parts)?;
        }
        let lit: Vec<f32> = kit::capture_radiance(r, 0.0)?.into_iter().map(kit::luminance).collect();
        let vis: Vec<f32> = lit.iter().zip(&l.open).map(|(&a, &b)| if b > 1e-3 { (a / b).clamp(0.0, 1.5) } else { 0.0 }).collect();
        if let Ok(dir) = std::env::var("LIGHTBENCH_SHADOW_DUMP") {
            dump(&dir, &l.exact, &vis, &l.open);
        }
        let mut metrics = score(&l.exact, &vis, &l.open);
        metrics.push(("frames_measured".into(), f64::from(n)));
        Ok(metrics)
    }
}

/// Measurement (`LIGHTBENCH_SHADOW_DUMP=<dir>`): the measured and exact visibility and the
/// unshadowed frame as 8-bit PGM images, and the CPU's geometry mask.
fn dump(dir: &str, e: &Exact, vis: &[f32], open: &[f32]) {
    let _ = std::fs::create_dir_all(dir);
    let max = open.iter().copied().fold(1e-6f32, f32::max);
    let write = |name: &str, v: &mut dyn Iterator<Item = f32>| {
        let mut bytes = format!("P5 {} {} 255
", SIZE[0], SIZE[1]).into_bytes();
        bytes.extend(v.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8));
        let _ = std::fs::write(format!("{dir}/{name}.pgm"), bytes);
    };
    write("measured", &mut vis.iter().copied());
    write("exact", &mut e.all.iter().zip(&e.floor).map(|(&v, &f)| if f { v } else { 0.0 }));
    write("open", &mut open.iter().map(|&o| o / max));
    write("geometry", &mut e.any.iter().map(|&a| if a { 1.0 } else { 0.0 }));
}

/// The metrics of a measured visibility image against the exact answer (module docs).
fn score(e: &Exact, vis: &[f32], open: &[f32]) -> Vec<(String, f64)> {
    let (w, h) = (SIZE[0] as i32, SIZE[1] as i32);
    let at = |x: i32, y: i32| (y * w + x) as usize;
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h;
    // Within `r` pixels of a pixel where `pred` holds.
    let near = |p: (i32, i32), r: i32, pred: &dyn Fn(usize) -> bool| {
        (-r..=r).any(|dy| (-r..=r).any(|dx| inside(p.0 + dx, p.1 + dy) && pred(at(p.0 + dx, p.1 + dy))))
    };
    let edge = |q: usize| e.floor[q] && e.all[q] > 0.02 && e.all[q] < 0.98;
    // The unshadowed floor's luminance (its median), for the camera agreement check.
    let floor_level = {
        let mut v: Vec<f32> = open.iter().zip(&e.floor).filter(|&(_, &f)| f).map(|(&o, _)| o).collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let (mut band, mut err, mut lit_in_shadow, mut contact, mut wire_n, mut wire_miss, mut mismatch) = (0usize, 0f64, 0usize, 0usize, 0usize, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let p = at(x, y);
            if e.floor[p] && open[p] < 0.5 * floor_level {
                mismatch += 1;
            }
            if !e.floor[p] {
                continue;
            }
            if near((x, y), 3, &edge) {
                band += 1;
                err += f64::from((vis[p] - e.all[p]).abs());
            }
            let umbra = !near((x, y), 2, &|q: usize| e.floor[q] && e.all[q] >= 0.05);
            if umbra && vis[p] > 0.5 {
                lit_in_shadow += 1;
            }
            if e.all[p] < 0.05 && near((x, y), 4, &|q: usize| e.block[q]) && vis[p] > 0.5 {
                contact += 1;
            }
            if e.wire[p] < 0.5 {
                wire_n += 1;
                wire_miss += usize::from(vis[p] > 0.8);
            }
        }
    }
    // The rail's edge: per row, the first falling 50% crossing of the rail-only exact answer
    // and the measured crossing within 6 pixels of it.
    let crossing = |row: &dyn Fn(i32) -> Option<f32>, from: i32, to: i32| -> Option<f32> {
        (from.max(0)..to.min(w - 1)).find_map(|x| {
            let (a, b) = (row(x)?, row(x + 1)?);
            (a >= 0.5 && b < 0.5).then(|| x as f32 + (a - 0.5) / (a - b))
        })
    };
    let mut residuals = vec![];
    for y in 0..h {
        let exact_row = |x: i32| e.floor[at(x, y)].then_some(e.rail[at(x, y)]);
        let Some(xe) = crossing(&exact_row, 0, w) else { continue };
        // Only rows where the rail alone decides (no other caster within 8 pixels).
        let x0 = xe as i32;
        if !(x0 - 8..=x0 + 8).all(|x| !inside(x, y) || !e.floor[at(x, y)] || (e.all[at(x, y)] - e.rail[at(x, y)]).abs() < 1e-6) {
            continue;
        }
        let measured_row = |x: i32| e.floor[at(x, y)].then_some(vis[at(x, y)]);
        if let Some(xm) = crossing(&measured_row, x0 - 6, x0 + 7) {
            residuals.push(f64::from(xm - xe));
        }
    }
    let wobble = if residuals.len() > 2 {
        let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
        (residuals.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / residuals.len() as f64).sqrt()
    } else {
        f64::NAN
    };
    vec![
        ("err".into(), err / band.max(1) as f64),
        ("lit_in_shadow".into(), lit_in_shadow as f64),
        ("rail_wobble_px".into(), wobble),
        ("contact_gap_px".into(), contact as f64),
        ("thin_missing".into(), wire_miss as f64 / wire_n.max(1) as f64),
        ("rail_rows".into(), residuals.len() as f64),
        ("mask_mismatch".into(), mismatch as f64),
    ]
}
