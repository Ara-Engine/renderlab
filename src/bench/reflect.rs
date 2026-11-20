//! The reflection fixture: a mirror and a glossy metal sphere over a sun-lit grey floor, with a
//! red box both reflect, under a uniform sky; scored against the live path tracer (kit.rs
//! `traced_reference`, 16,384 paths per pixel, cached on disk).
//!
//! Metrics, luminance over each object's pixels (the CPU's ray cast, one pixel in from its
//! outline): `<object>.energy` (the frame's mean over the reference's) and `<object>.err` (mean
//! absolute difference over the reference's mean), for `mirror` and `sphere`; `energy` and
//! `err` over both; `floor.err`, the rest of the floor, as the lighting's own check.
//!
//! Frames: LIGHTBENCH_REFLECT_FRAMES (default 32) per run with temporal reconstruction and the
//! lighting cache on, as the engine renders, each run from a cut and an empty cache.
use super::{BenchPath, Fixture, RefImage, kit};
use crate::renderer::Renderer;
use crate::{Error, IndirectSettings, Light, Lighting, MaterialDescriptor, Mat4, Reconstruction, ShadowRequest, Shadows, SoftShadows, Vec3, VirtualShadowSettings};

const VIEW: kit::View = kit::View { eye: Vec3::new(0.0, 1.0, 2.6), target: Vec3::new(0.0, 0.4, 0.0), fov: 0.8, size: [192, 144] };
const TO_SUN: Vec3 = Vec3::new(0.4, 0.8, 0.45);
const SUN_RADIUS: f32 = 0.01;
const LUX: f32 = 1000.0;
const SKY: f32 = 20.0;
const SAMPLES: u32 = 16384;
const FLOOR_HALF: f32 = 1.5;
const SPHERE: (Vec3, f32) = (Vec3::new(-0.45, 0.35, 0.0), 0.35);
/// The mirror: centre, normal (toward the camera and the objects) and half-extents.
const MIRROR: (Vec3, Vec3, f32, f32) = (Vec3::new(0.35, 0.55, -0.8), Vec3::new(-0.25, 0.0, 1.0), 0.5, 0.45);
const BOX: (Vec3, Vec3) = (Vec3::new(0.55, 0.15, 0.25), Vec3::new(0.15, 0.15, 0.15));

fn frames() -> u32 {
    std::env::var("LIGHTBENCH_REFLECT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(32)
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
fn mirror_faces() -> Vec<([Vec3; 4], Vec3)> {
    let (centre, n, hu, hv) = MIRROR;
    let n = n.normalize();
    vec![kit::quad(centre, n, n.cross(Vec3::Y).normalize() * -hu, hv)]
}

/// The scene's parts on `r`: floor, sphere, mirror and box.
fn build(r: &mut Renderer) -> Result<Vec<kit::Part>, Error> {
    let program = r.standard_program().clone();
    let metal = |base: [f32; 3], roughness: f32| MaterialDescriptor {
        base_color: [base[0], base[1], base[2], 1.0],
        metallic: 1.0,
        roughness,
        double_sided: true,
        program: Some(&program),
        ..Default::default()
    };
    let floor = kit::faces_mesh(&[kit::quad(Vec3::ZERO, Vec3::Y, Vec3::X * FLOOR_HALF, FLOOR_HALF)]);
    let (sphere_v, sphere_i) = kit::sphere_mesh(SPHERE.0, SPHERE.1, 24, 48);
    let mirror = kit::faces_mesh(&mirror_faces());
    Ok(vec![
        kit::mesh_part(r, &floor.0, &floor.1, &program, [0.5; 3], &[], &[Mat4::IDENTITY])?,
        kit::material_part(r, &sphere_v, &sphere_i, &metal([0.95, 0.64, 0.54], 0.3), &[Mat4::IDENTITY])?,
        kit::material_part(r, &mirror.0, &mirror.1, &metal([0.95; 3], 0.02), &[Mat4::IDENTITY])?,
        kit::part(r, &kit::box_faces(BOX.0, BOX.1, false), &program, [0.7, 0.1, 0.1], &[])?,
    ])
}

/// Which object each pixel's centre ray meets first: 0 floor, 1 sphere, 2 mirror, 3 box, 4 none.
fn objects() -> Vec<u8> {
    let (w, h) = (VIEW.size[0], VIEW.size[1]);
    let mirror = mirror_faces()[0].0;
    let floor = kit::quad(Vec3::ZERO, Vec3::Y, Vec3::X * FLOOR_HALF, FLOOR_HALF).0;
    let boxes = kit::box_faces(BOX.0, BOX.1, false);
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let d = VIEW.ray(x, y);
            let hits = [
                kit::quad_hit(VIEW.eye, d, &floor),
                kit::sphere_hit(VIEW.eye, d, SPHERE.0, SPHERE.1),
                kit::quad_hit(VIEW.eye, d, &mirror),
                boxes.iter().filter_map(|(c, _)| kit::quad_hit(VIEW.eye, d, c)).reduce(f32::min),
            ];
            hits.iter().enumerate().filter_map(|(i, t)| t.map(|t| (i, t))).min_by(|a, b| a.1.total_cmp(&b.1)).map_or(4, |(i, _)| i as u8)
        })
        .collect()
}
/// Pixels of object `id` whose four neighbours are `id` too.
fn interior(objects: &[u8], id: u8) -> Vec<bool> {
    let (w, h) = (VIEW.size[0] as usize, VIEW.size[1] as usize);
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            x > 0 && y > 0 && x + 1 < w && y + 1 < h && [i, i - 1, i + 1, i - w, i + w].iter().all(|&j| objects[j] == id)
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Reflect {
    loaded: Option<(Vec<kit::Part>, wgpu::Texture, Vec<u8>)>,
}
impl Fixture for Reflect {
    fn name(&self) -> &str {
        "reflect"
    }
    fn scene_key(&self) -> String {
        format!("reflect-1 {:?} {TO_SUN} {SUN_RADIUS} {LUX} {SKY} {SAMPLES} {SPHERE:?} {MIRROR:?} {BOX:?}", VIEW.size)
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
        self.loaded = Some((parts, kit::color_target(r, VIEW.size), objects()));
        Ok(())
    }
    fn reference(&mut self, _r: &mut Renderer) -> Option<Result<RefImage, Error>> {
        Some(kit::traced_reference(&build, &VIEW, &lighting(), SUN_RADIUS, SAMPLES))
    }
    fn run(&mut self, r: &mut Renderer, path: BenchPath, reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error> {
        let (parts, target, objects) = self.loaded.as_mut().ok_or(Error::InvalidInput("reflect fixture not loaded"))?;
        let reference = reference.ok_or(Error::InvalidInput("reflect fixture: no reference"))?;
        r.cut_reconstruction();
        r.reset_indirect();
        r.reset_virtual_shadow_cache();
        let camera = VIEW.camera()?;
        let n = frames();
        for _ in 0..n {
            kit::frame(r, target, &camera, parts)?;
        }
        let image = kit::capture_radiance(r, 0.0)?;
        kit::dump(&format!("reflect-{}", path.name()), VIEW.size, &image);
        kit::dump("reflect-reference", VIEW.size, &reference.rgb);
        let mut metrics = vec![];
        let masks = [("mirror", interior(objects, 2)), ("sphere", interior(objects, 1))];
        let both: Vec<bool> = masks[0].1.iter().zip(&masks[1].1).map(|(a, b)| *a || *b).collect();
        let (energy, err) = kit::against(&image, &reference.rgb, &both);
        metrics.push(("err".into(), err));
        metrics.push(("energy".into(), energy));
        for (name, mask) in &masks {
            let (energy, err) = kit::against(&image, &reference.rgb, mask);
            metrics.push((format!("{name}.err"), err));
            metrics.push((format!("{name}.energy"), energy));
        }
        metrics.push(("floor.err".into(), kit::against(&image, &reference.rgb, &interior(objects, 0)).1));
        metrics.push(("frames_measured".into(), f64::from(n)));
        Ok(metrics)
    }
}
