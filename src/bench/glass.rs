//! The transparency fixture: a tinted glass pane (index 1.5) and a blended film (index 1, half
//! coverage) standing over a sun-lit floor in front of a red and a white wall, under a uniform
//! sky; scored against the live path tracer (kit.rs `traced_reference`, 16,384 paths per pixel,
//! cached on disk), which traces both as the engine models them: thin, no refraction offset.
//!
//! Metrics, luminance over each pane's pixels (the CPU's ray cast, one pixel in from its outline):
//! `<pane>.energy` (the frame's mean over the reference's) and `<pane>.err` (mean absolute
//! difference over the reference's mean), for `glass` and `film`; `energy` and `err` over both;
//! `shadow.err` over the floor in their sun shadows (the glass's coloured, the film's grey).
//!
//! Frames: LIGHTBENCH_GLASS_FRAMES (default 32) per run with temporal reconstruction and the
//! lighting cache on, as the engine renders, each run from a cut and an empty cache.
use super::{BenchPath, Fixture, RefImage, kit};
use crate::renderer::Renderer;
use crate::{
    Error, IndirectSettings, Light, Lighting, MaterialDescriptor, Mat4, Reconstruction, ShadowRequest, Shadows, SoftShadows,
    Transparency, Vec3, VirtualShadowSettings,
};

const VIEW: kit::View = kit::View { eye: Vec3::new(0.0, 0.9, 2.4), target: Vec3::new(0.0, 0.4, -0.3), fov: 0.8, size: [192, 144] };
const TO_SUN: Vec3 = Vec3::new(0.25, 0.85, 0.45);
const SUN_RADIUS: f32 = 0.01;
const LUX: f32 = 1000.0;
const SKY: f32 = 20.0;
const SAMPLES: u32 = 16384;
const FLOOR_HALF: f32 = 1.5;
/// The panes: centre and half-extent (square, facing +z).
const GLASS: (Vec3, f32) = (Vec3::new(-0.4, 0.45, 0.2), 0.3);
const FILM: (Vec3, f32) = (Vec3::new(0.4, 0.45, 0.2), 0.3);
const WALL_Z: f32 = -0.7;

fn frames() -> u32 {
    std::env::var("LIGHTBENCH_GLASS_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(32)
}
fn lighting() -> Lighting {
    Lighting { lights: vec![Light::Directional { travel: -TO_SUN.normalize(), lux: [LUX; 3] }], environment: [SKY; 3], exposure_stops: 0.0 }
}
fn shadows() -> Shadows {
    Shadows {
        requests: vec![ShadowRequest::Directional { split_ends: vec![4.0, 12.0, 40.0, 1000.0] }],
        directional_disk_angle: SUN_RADIUS,
        ..Default::default()
    }
}
fn pane((centre, half): (Vec3, f32)) -> ([Vec3; 4], Vec3) {
    kit::quad(centre, Vec3::Z, Vec3::X * half, half)
}
fn walls() -> [([Vec3; 4], Vec3); 2] {
    [
        kit::quad(Vec3::new(-0.75, 0.6, WALL_Z), Vec3::Z, Vec3::X * 0.75, 0.6),
        kit::quad(Vec3::new(0.75, 0.6, WALL_Z), Vec3::Z, Vec3::X * 0.75, 0.6),
    ]
}

/// The scene's parts on `r`: floor, walls, then the two panes (drawn last, as transparents).
fn build(r: &mut Renderer) -> Result<Vec<kit::Part>, Error> {
    let program = r.standard_program().clone();
    let see_through = |base: [f32; 3], transparency: Transparency| MaterialDescriptor {
        base_color: [base[0], base[1], base[2], 1.0],
        metallic: 0.0,
        roughness: 0.05,
        double_sided: true,
        program: Some(&program),
        transparency: Some(transparency),
        ..Default::default()
    };
    let [red, white] = walls();
    let glass = kit::faces_mesh(&[pane(GLASS)]);
    let film = kit::faces_mesh(&[pane(FILM)]);
    Ok(vec![
        kit::part(r, &[kit::quad(Vec3::ZERO, Vec3::Y, Vec3::X * FLOOR_HALF, FLOOR_HALF)], &program, [0.5; 3], &[])?,
        kit::part(r, &[red], &program, [0.7, 0.12, 0.1], &[])?,
        kit::part(r, &[white], &program, [0.8; 3], &[])?,
        kit::material_part(r, &glass.0, &glass.1, &see_through([0.6, 0.9, 0.7], Transparency { opacity: 0.1, ior: 1.5, thickness: 0.0 }), &[Mat4::IDENTITY])?,
        kit::material_part(r, &film.0, &film.1, &see_through([0.2, 0.4, 0.9], Transparency { opacity: 0.5, ior: 1.0, thickness: 0.0 }), &[Mat4::IDENTITY])?,
    ])
}

/// Per pixel: 1 the glass, 2 the film (their centre rays meet the pane), 3 floor in a pane's
/// sun shadow (seen directly), 0 anything else.
fn classes() -> Vec<u8> {
    let (w, h) = (VIEW.size[0], VIEW.size[1]);
    let floor = kit::quad(Vec3::ZERO, Vec3::Y, Vec3::X * FLOOR_HALF, FLOOR_HALF).0;
    let (glass, film) = (pane(GLASS).0, pane(FILM).0);
    let walls = walls();
    let s = TO_SUN.normalize();
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let d = VIEW.ray(x, y);
            let at = |c: &[Vec3; 4]| kit::quad_hit(VIEW.eye, d, c);
            let wall = walls.iter().filter_map(|(c, _)| at(c)).reduce(f32::min).unwrap_or(f32::INFINITY);
            let ground = at(&floor).unwrap_or(f32::INFINITY);
            let opaque = wall.min(ground);
            if at(&glass).is_some_and(|t| t < opaque) {
                return 1;
            }
            if at(&film).is_some_and(|t| t < opaque) {
                return 2;
            }
            let p = VIEW.eye + d * ground;
            let shaded = |c: &[Vec3; 4]| kit::quad_hit(p + s * 1e-3, s, c).is_some();
            if ground < wall && (shaded(&glass) || shaded(&film)) { 3 } else { 0 }
        })
        .collect()
}
fn interior(classes: &[u8], id: u8) -> Vec<bool> {
    let (w, h) = (VIEW.size[0] as usize, VIEW.size[1] as usize);
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            x > 0 && y > 0 && x + 1 < w && y + 1 < h && [i, i - 1, i + 1, i - w, i + w].iter().all(|&j| classes[j] == id)
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Glass {
    loaded: Option<(Vec<kit::Part>, wgpu::Texture, Vec<u8>)>,
}
impl Fixture for Glass {
    fn name(&self) -> &str {
        "glass"
    }
    fn scene_key(&self) -> String {
        format!("glass-1 {:?} {TO_SUN} {SUN_RADIUS} {LUX} {SKY} {SAMPLES} {GLASS:?} {FILM:?} {WALL_Z}", VIEW.size)
    }
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error> {
        r.set_atmosphere(crate::Atmosphere::off())?;
        r.set_indirect(IndirectSettings::default())?;
        r.set_reconstruction(Reconstruction::default())?;
        match path {
            BenchPath::Rt => r.set_soft_shadows(SoftShadows::Interactive),
            BenchPath::NoRt => r.set_virtual_shadow_settings(VirtualShadowSettings::default())?,
        }
        r.set_lighting_and_shadows(&lighting(), &shadows())?;
        let parts = build(r)?;
        self.loaded = Some((parts, kit::color_target(r, VIEW.size), classes()));
        Ok(())
    }
    fn reference(&mut self, _r: &mut Renderer) -> Option<Result<RefImage, Error>> {
        Some(kit::traced_reference(&build, &VIEW, &lighting(), SUN_RADIUS, SAMPLES))
    }
    fn run(&mut self, r: &mut Renderer, path: BenchPath, reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let (parts, target, classes) = self.loaded.as_mut().ok_or(Error::InvalidInput("glass fixture not loaded"))?;
        let reference = reference.ok_or(Error::InvalidInput("glass fixture: no reference"))?;
        r.cut_reconstruction();
        r.reset_indirect();
        r.reset_virtual_shadow_cache();
        let camera = VIEW.camera()?;
        let n = frames();
        for _ in 0..n {
            kit::frame(r, target, &camera, parts)?;
        }
        let image = kit::capture_radiance(r, 0.0)?;
        kit::dump(&format!("glass-{}", path.name()), VIEW.size, &image);
        kit::dump("glass-reference", VIEW.size, &reference.rgb);
        let masks = [("glass", interior(classes, 1)), ("film", interior(classes, 2))];
        let both: Vec<bool> = masks[0].1.iter().zip(&masks[1].1).map(|(a, b)| *a || *b).collect();
        let (energy, err) = kit::against(&image, &reference.rgb, &both);
        let mut metrics = vec![("err".to_owned(), err), ("energy".to_owned(), energy)];
        for (name, mask) in &masks {
            let (energy, err) = kit::against(&image, &reference.rgb, mask);
            metrics.push((format!("{name}.err"), err));
            metrics.push((format!("{name}.energy"), energy));
        }
        metrics.push(("shadow.err".into(), kit::against(&image, &reference.rgb, &interior(classes, 3)).1));
        metrics.push(("frames_measured".into(), f64::from(n)));
        Ok(metrics)
    }
}
