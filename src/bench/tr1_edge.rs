//! The temporal reconstruction (TR1) fixture: ghosts after a mover passes and after a camera cut.
//! A bright box slides across a dark floor under a high sun; the scores are against clean frames
//! of the same views rendered still from a cut at load (no reference needed). The lighting cache
//! is off, so what trails is TR1's history, not the cache's.
//!
//! - **Mover:** the box slides LIGHTBENCH_TR1_STEP m per frame (default 0.03, about 2 px) for
//!   LIGHTBENCH_TR1_FRAMES frames (default 40). On its last frame, `pass_ghost_px` is how far
//!   behind the box's trailing face the floor still differs from the clean frame (over 10% of
//!   the clean luminance), as pixels along its path: the median over five lines across the box's
//!   width, the largest as `pass_ghost_max_px`.
//! - **Cut:** the camera then jumps to a second view (no `cut_reconstruction`: TR1 must see the
//!   cut itself). `cut_ghost_px` counts the pixels of the first frame after the jump that differ
//!   from that view's clean frame, and `cut_frames` the frames until under 0.2% of the pixels do
//!   (17: not within 16).
use super::{BenchPath, Fixture, RefImage, kit};
use crate::renderer::Renderer;
use crate::{Error, IndirectSettings, Light, Lighting, Mat4, Reconstruction, ShadowRequest, Shadows, SoftShadows, Vec3, VirtualShadowSettings};

const SIZE: [u32; 2] = [256, 160];
const VIEW: kit::View = kit::View { eye: Vec3::new(0.0, 2.2, 1.6), target: Vec3::new(0.0, 0.0, 0.0), fov: 0.9, size: SIZE };
/// The view the camera cuts to.
const CUT: kit::View = kit::View { eye: Vec3::new(2.2, 1.5, -1.0), target: Vec3::new(0.3, 0.0, 0.1), fov: 0.9, size: SIZE };
const TO_SUN: Vec3 = Vec3::new(0.1, 1.0, 0.15);
const LUX: f32 = 1000.0;
const SKY: f32 = 10.0;
const HALF: f32 = 0.15;
const START_X: f32 = -1.0;
/// Frames the clean views render still.
const CLEAN_FRAMES: u32 = 48;
const CUT_FRAMES: u32 = 16;

fn step() -> f32 {
    std::env::var("LIGHTBENCH_TR1_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.03)
}
fn frames() -> u32 {
    std::env::var("LIGHTBENCH_TR1_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(40)
}
fn end_x() -> f32 {
    START_X + step() * (frames() - 1) as f32
}
fn at(x: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x, 0.0, 0.0))
}

/// Pixels of `image` that differ from `clean` by over 10% of the clean luminance (plus 2% of
/// its mean, so the darkest pixels' noise does not count).
fn differs(image: &[[f32; 3]], clean: &[[f32; 3]]) -> Vec<bool> {
    let mean = clean.iter().map(|&p| kit::luminance(p)).sum::<f32>() / clean.len().max(1) as f32;
    image.iter().zip(clean).map(|(&a, &b)| {
        let (a, b) = (kit::luminance(a), kit::luminance(b));
        (a - b).abs() > 0.1 * b + 0.02 * mean
    }).collect()
}

#[derive(Default)]
pub(crate) struct Tr1Edge {
    loaded: Option<Loaded>,
}
struct Loaded {
    parts: Vec<kit::Part>,
    target: wgpu::Texture,
    /// The clean frames: the first view with the box at its last position, the cut view likewise.
    clean: Vec<[f32; 3]>,
    clean_cut: Vec<[f32; 3]>,
}

impl Tr1Edge {
    fn still(r: &mut Renderer, l: &mut Loaded, view: &kit::View) -> Result<Vec<[f32; 3]>, Error> {
        r.cut_reconstruction();
        let camera = view.camera()?;
        for _ in 0..CLEAN_FRAMES {
            kit::frame(r, &l.target, &camera, &mut l.parts)?;
        }
        kit::capture_radiance(r, 0.0)
    }
}

impl Fixture for Tr1Edge {
    fn name(&self) -> &str {
        "tr1-edge"
    }
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error> {
        r.set_atmosphere(crate::Atmosphere::off())?;
        r.set_indirect(IndirectSettings::off())?;
        r.set_reconstruction(Reconstruction::default())?;
        match path {
            BenchPath::Rt => r.set_soft_shadows(SoftShadows::Interactive),
            BenchPath::NoRt => r.set_virtual_shadow_settings(VirtualShadowSettings::default())?,
        }
        let lighting = Lighting { lights: vec![Light::Directional { travel: -TO_SUN.normalize(), lux: [LUX; 3] }], environment: [SKY; 3], exposure_stops: 0.0 };
        let shadows = Shadows {
            requests: vec![ShadowRequest::Directional { split_ends: vec![4.0, 12.0, 40.0, 1000.0] }],
            directional_disk_angle: 0.01,
            ..Default::default()
        };
        r.set_lighting_and_shadows(&lighting, &shadows)?;
        let program = r.standard_program().clone();
        let floor = kit::part(r, &[kit::quad(Vec3::ZERO, Vec3::Y, Vec3::X * 2.5, 2.5)], &program, [0.08; 3], &[])?;
        let (v, i) = kit::faces_mesh(&kit::box_faces(Vec3::new(0.0, HALF, 0.0), Vec3::splat(HALF), false));
        let mover = kit::mesh_part(r, &v, &i, &program, [0.9; 3], &[], &[at(end_x())])?;
        let mut l = Loaded { parts: vec![floor, mover], target: kit::color_target(r, SIZE), clean: vec![], clean_cut: vec![] };
        l.clean = Self::still(r, &mut l, &VIEW)?;
        l.clean_cut = Self::still(r, &mut l, &CUT)?;
        self.loaded = Some(l);
        Ok(())
    }
    fn run(&mut self, r: &mut Renderer, path: BenchPath, _reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let l = self.loaded.as_mut().ok_or(Error::InvalidInput("tr1-edge fixture not loaded"))?;
        let camera = VIEW.camera()?;
        // The slide, from a cut.
        l.parts[1].instances.set(0, at(START_X))?;
        r.cut_reconstruction();
        let n = frames();
        for k in 0..n {
            l.parts[1].instances.set(0, at(START_X + step() * k as f32))?;
            kit::frame(r, &l.target, &camera, &mut l.parts)?;
        }
        let image = kit::capture_radiance(r, 0.0)?;
        kit::dump(&format!("tr1-pass-{}", path.name()), SIZE, &image);
        kit::dump(&format!("tr1-clean-{}", path.name()), SIZE, &l.clean);
        kit::dump(&format!("tr1-clean-cut-{}", path.name()), SIZE, &l.clean_cut);
        let ghost = differs(&image, &l.clean);
        // Along five lines across the box's width, from its trailing face backwards: the floor
        // distance over which every sample still differs (from 2 cm behind, past the outline).
        let back = end_x() - HALF;
        let mut lengths: Vec<f64> = [-0.8f32, -0.4, 0.0, 0.4, 0.8]
            .iter()
            .map(|&z| {
                let z = z * HALF;
                let pixel = |x: f32| {
                    let [u, v] = VIEW.project(Vec3::new(x, 0.0, z));
                    (u >= 0.0 && v >= 0.0 && (u as u32) < SIZE[0] && (v as u32) < SIZE[1]).then(|| (v as u32 * SIZE[0] + u as u32) as usize)
                };
                let mut reach = 0.02f32;
                while reach < 1.5 && pixel(back - reach - 0.005).is_some_and(|p| ghost[p]) {
                    reach += 0.005;
                }
                let (a, b) = (VIEW.project(Vec3::new(back - 0.02, 0.0, z)), VIEW.project(Vec3::new(back - reach, 0.0, z)));
                if reach <= 0.02 { 0.0 } else { f64::from(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()) }
            })
            .collect();
        lengths.sort_by(f64::total_cmp);
        // The cut: the jump to the second view, the box where the slide left it.
        let cut = CUT.camera()?;
        let pixels = (SIZE[0] * SIZE[1]) as f64;
        let (mut first, mut settled) = (0.0, CUT_FRAMES + 1);
        for k in 1..=CUT_FRAMES {
            kit::frame(r, &l.target, &cut, &mut l.parts)?;
            let frame = kit::capture_radiance(r, 0.0)?;
            let count = differs(&frame, &l.clean_cut).iter().filter(|&&d| d).count() as f64;
            if k == 1 {
                first = count;
                kit::dump(&format!("tr1-cut1-{}", path.name()), SIZE, &frame);
            }
            if k == CUT_FRAMES {
                kit::dump(&format!("tr1-cut{k}-{}", path.name()), SIZE, &frame);
            }
            if count < 0.002 * pixels {
                settled = k;
                break;
            }
        }
        Ok(vec![
            ("pass_ghost_px".into(), lengths[2]),
            ("pass_ghost_max_px".into(), lengths[4]),
            ("cut_ghost_px".into(), first),
            ("cut_frames".into(), f64::from(settled)),
            // The pass times cover the slide only, not the cut's frames after it.
            ("frames_measured".into(), f64::from(n)),
            ("frames_after".into(), f64::from(settled.min(CUT_FRAMES))),
        ])
    }
}
