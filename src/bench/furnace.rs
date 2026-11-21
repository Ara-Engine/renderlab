//! The white-furnace fixtures (gicut's cases, src/gi_furnace_tests.rs): scenes whose indirect
//! light is analytic, scored against the exact answer.
//!
//! - `grey`: a closed box of inward faces, albedo 0.5, every face emitting 0.5 cd/m^2:
//!   uniform radiance L = Le / (1 - rho) = 1.
//! - `orange`: the same box with albedo (0.9, 0.4, 0.1): L = (5, 0.833, 0.556). The red
//!   channel needs about 30 bounces to reach 95%, so a truncated bounce series shows here.
//! - `slab`: a thin slab (albedo 0.5) under a sky of 1 cd/m^2 with a 20 cd/m^2 emitter 0.4 m
//!   below it. The top face sees only the sky: L = 0.5. More is leakage through the slab.
//! - `slab-roof`: the slab under a black sky with a grey roof 0.5 m above it. No path from the
//!   emitter reaches the roof or the top face: both must read 0.
//!
//! - `walk`: the grey box seen by a walking camera: settled still for the usual frames, then
//!   moved a fixed step per frame (LIGHTBENCH_WALK_STEP metres, default 0.02, for
//!   LIGHTBENCH_WALK_FRAMES frames, default 48), scored on its last frame. The bench's
//!   per-frame cache milliseconds cover the walking frames only (`frames_measured`), the cost
//!   the budgeted cache is judged on.
//! - `walk-wrap`: `walk` with the cache's frame stamp started 24 frames into the walk before 4096
//!   (`Renderer::bench_start_frame`), so the walk crosses a 12-bit stamp's wrap (gicut).
//!
//! Metrics per view (`<view>.` prefix where a case has several): `energy`, the mean luminance
//! over the exact answer's; `err`, the mean per-pixel |luminance / answer - 1|; `off5`, the share
//! of pixels more than 5% off; `mean`, the mean luminance. Where the answer is black, `mean` is the
//! leak and `leak_max` the brightest pixel's luminance.
//! Frames per view: LIGHTBENCH_FURNACE_FRAMES (default 64); the cache starts empty each run.
use super::{BenchPath, Fixture, RefImage, kit};
use crate::renderer::Renderer;
use crate::{Camera, Error, IndirectSettings, Lighting, ProgramDescriptor, Reconstruction, Value, Vec3};

/// Lambert with a constant emission: `scatter` carries bounce light into the cache and
/// `emission` makes the surface a light source for it. The pixel is its irradiance times
/// albedo over pi, plus its own emission.
pub(crate) const FURNACE_PROGRAM: &str = "struct Parameters { glow:vec3<f32> };
struct Surface { base:vec3<f32>, n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 { n=input.geometric; }
    if input.material.double_sided!=0u && !input.source_front { n=-n; }
    return Surface(input.material.base_color.rgb*input.color.rgb,n);
}
fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return s.base*(max(dot(s.n,wi),0.0)/PI);
}
fn emission(s:Surface, input:SurfaceInput) -> vec3<f32> { return params.glow; }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    return expose(indirect_irradiance(input,s.n)*s.base/PI+params.glow);
}
";

const SIZE: [u32; 2] = [320, 180];
const FOV: f32 = 1.2;

/// A surface of a case: its faces, albedo and emission.
struct Shape {
    faces: Vec<([Vec3; 4], Vec3)>,
    albedo: [f32; 3],
    glow: [f32; 3],
}
/// A view and the radiance its pixels must read.
struct View {
    name: &'static str,
    eye: Vec3,
    target: Vec3,
    up: Vec3,
    expected: [f32; 3],
    /// Walking views move eye and target this far each frame after settling.
    walk: Option<Vec3>,
}
pub(crate) struct Furnace {
    name: &'static str,
    sky: f32,
    shapes: Vec<Shape>,
    views: Vec<View>,
    loaded: Option<(crate::Program, Vec<kit::Part>, wgpu::Texture)>,
    /// The cache's frame stamp after each run's reset (`walk-wrap`); None starts at 0.
    start_frame: Option<u32>,
}

impl Furnace {
    fn room(name: &'static str, albedo: [f32; 3]) -> Self {
        let glow = [0.5; 3];
        Self {
            name,
            sky: 0.0,
            shapes: vec![Shape { faces: kit::box_faces(Vec3::ZERO, Vec3::splat(2.0), true), albedo, glow }],
            views: vec![View {
                name: "corner",
                eye: Vec3::new(-0.3, -0.2, -0.4),
                target: Vec3::new(1.0, 0.7, 0.6),
                up: Vec3::Y,
                expected: std::array::from_fn(|c| glow[c] / (1.0 - albedo[c])),
                walk: None,
            }],
            loaded: None,
            start_frame: None,
        }
    }
    pub(crate) fn grey() -> Self {
        Self::room("grey", [0.5; 3])
    }
    pub(crate) fn orange() -> Self {
        Self::room("orange", [0.9, 0.4, 0.1])
    }
    /// The grey box with a walking camera (along +x, through the room's middle).
    pub(crate) fn walk() -> Self {
        let mut room = Self::room("walk", [0.5; 3]);
        let step = std::env::var("LIGHTBENCH_WALK_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.02);
        room.views[0].name = "walk";
        room.views[0].eye = Vec3::new(-1.2, -0.2, -0.4);
        room.views[0].target = Vec3::new(0.3, 0.5, 0.6);
        room.views[0].walk = Some(Vec3::X * step);
        room
    }
    /// `walk` whose walking frames cross frame 4096 of the cache's stamp, 24 frames in.
    pub(crate) fn walk_wrap() -> Self {
        let mut walk = Self::walk();
        walk.name = "walk-wrap";
        walk.start_frame = Some(4096 - frames() - 24);
        walk
    }
    fn slab_shapes() -> Vec<Shape> {
        vec![
            Shape { faces: kit::box_faces(Vec3::ZERO, Vec3::new(1.5, 0.01, 1.5), false), albedo: [0.5; 3], glow: [0.0; 3] },
            Shape {
                faces: vec![kit::quad(Vec3::new(0.0, -0.4, 0.0), Vec3::Y, Vec3::X * 0.75, 0.75)],
                albedo: [0.0; 3],
                glow: [20.0; 3],
            },
        ]
    }
    pub(crate) fn slab() -> Self {
        Self {
            name: "slab",
            sky: 1.0,
            shapes: Self::slab_shapes(),
            views: vec![
                View { name: "top", eye: Vec3::new(0.0, 1.2, 0.0), target: Vec3::ZERO, up: Vec3::Z, expected: [0.5; 3], walk: None },
                // The emitter seen directly from below the slab: no exact answer (`mean` only).
                View { name: "underside", eye: Vec3::new(0.0, -0.25, 0.0), target: Vec3::ZERO, up: Vec3::Z, expected: [-1.0; 3], walk: None },
            ],
            loaded: None,
            start_frame: None,
        }
    }
    pub(crate) fn slab_roof() -> Self {
        let mut shapes = Self::slab_shapes();
        shapes.push(Shape {
            faces: vec![kit::quad(Vec3::new(0.0, 0.5, 0.0), -Vec3::Y, Vec3::X * 1.5, 1.5)],
            albedo: [0.5; 3],
            glow: [0.0; 3],
        });
        Self {
            name: "slab-roof",
            sky: 0.0,
            shapes,
            views: vec![
                View { name: "top", eye: Vec3::new(0.0, 0.3, 0.0), target: Vec3::ZERO, up: Vec3::Z, expected: [0.0; 3], walk: None },
                View { name: "roof", eye: Vec3::new(0.0, 0.2, 0.0), target: Vec3::new(0.0, 0.5, 0.0), up: Vec3::Z, expected: [0.0; 3], walk: None },
            ],
            loaded: None,
            start_frame: None,
        }
    }
}

/// Frames per view (LIGHTBENCH_FURNACE_FRAMES, default 64).
fn frames() -> u32 {
    std::env::var("LIGHTBENCH_FURNACE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(64).max(1)
}

impl Fixture for Furnace {
    fn name(&self) -> &str {
        self.name
    }
    fn load(&mut self, r: &mut Renderer, _path: BenchPath) -> Result<(), Error> {
        // As the furnace test runs (lib tests start with the atmosphere off): the uniform sky is
        // the environment, and no fog sits in the box. The default physical sky without a sun is
        // black, which read the slab's top as 0 instead of 0.5.
        r.set_atmosphere(crate::Atmosphere::off())?;
        r.set_lighting(&Lighting { lights: vec![], environment: [self.sky; 3], exposure_stops: 0.0 })?;
        // Single-sample frames: the estimator's own answer, not TR1's history of it.
        r.set_reconstruction(Reconstruction::off())?;
        r.set_indirect(IndirectSettings::default())?;
        // The emitter list on every gathering frame, as the furnace test builds it.
        r.gi.request_emitters();
        let program = r.try_create_program(&ProgramDescriptor { label: "furnace", source: FURNACE_PROGRAM })?;
        let parts = self
            .shapes
            .iter()
            .map(|s| kit::part(r, &s.faces, &program, s.albedo, &[("glow", Value::Vec3(s.glow))]))
            .collect::<Result<Vec<_>, _>>()?;
        let target = kit::color_target(r, SIZE);
        self.loaded = Some((program, parts, target));
        Ok(())
    }
    fn run(&mut self, r: &mut Renderer, _path: BenchPath, _reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let (_, parts, target) = self.loaded.as_mut().ok_or(Error::InvalidInput("furnace fixture not loaded"))?;
        // Every run starts from an empty cache, as a fresh renderer would.
        r.reset_indirect();
        if let Some(frame) = self.start_frame {
            r.bench_start_frame(frame);
        }
        let mut metrics = vec![];
        let prefix = self.views.len() > 1;
        for view in &self.views {
            let camera = Camera::look_at(view.eye, view.target, view.up, FOV, 0.01, 100.0)?;
            for _ in 0..frames() {
                kit::frame(r, target, &camera, parts)?;
            }
            if let Some(step) = view.walk {
                let walked: u32 = std::env::var("LIGHTBENCH_WALK_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(48);
                for k in 1..=walked {
                    let at = step * k as f32;
                    let camera = Camera::look_at(view.eye + at, view.target + at, view.up, FOV, 0.01, 100.0)?;
                    kit::frame(r, target, &camera, parts)?;
                }
                metrics.push(("frames_measured".into(), f64::from(walked)));
            }
            let pixels = kit::capture_radiance(r, 0.0)?;
            let name = |m: &str| if prefix { format!("{}.{m}", view.name) } else { m.to_owned() };
            let lum: Vec<f64> = pixels.iter().map(|&p| f64::from(kit::luminance(p))).collect();
            let n = lum.len() as f64;
            let answer = f64::from(kit::luminance(view.expected));
            let mean = lum.iter().sum::<f64>() / n;
            metrics.push((name("mean"), mean));
            if view.expected[0] < 0.0 {
                // No exact answer for this view.
            } else if answer > 0.0 {
                metrics.push((name("energy"), mean / answer));
                metrics.push((name("err"), lum.iter().map(|l| (l / answer - 1.0).abs()).sum::<f64>() / n));
                metrics.push((name("off5"), lum.iter().filter(|l| (*l / answer - 1.0).abs() > 0.05).count() as f64 / n));
            } else {
                metrics.push((name("leak_max"), lum.iter().copied().fold(0.0, f64::max)));
            }
        }
        Ok(metrics)
    }
}
