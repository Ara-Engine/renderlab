//! Local-light shadows on the traced frame, against the geometry of one occluder.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, LocalShadows, Mat4, MaterialDescriptor, Reconstruction, Vec3, Vertex,
    backend_ara::{
        forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows, render},
        temporal::Reconstruct,
    },
};

const SIZE: [u32; 2] = [128, 96];
/// The card's half side and height; the lamp.
const HALF: f64 = 0.5;
const LAMP: DVec3 = DVec3::new(-2., 0., 3.);

/// What a pixel's camera ray meets.
enum Sees {
    Card,
    /// The floor. `outside` is how far, in metres in the card's plane, the segment from
    /// the floor point to the lamp's centre passes outside the card: negative when the
    /// card blocks it.
    Floor {
        outside: f64,
    },
    /// Too close to the card's silhouette to call.
    Edge,
}

/// A card 1 m over a floor, seen from straight above, lit by a dim sun straight down
/// and a lamp up and to one side. The card's shadow from the lamp falls on the floor
/// beside it, where the camera sees it; its shadow from the sun lies under the card,
/// where the camera does not.
struct Scene {
    floor: [Vertex; 4],
    card: [Vertex; 4],
    camera: Camera,
    shadows: SunShadows,
}
impl Scene {
    fn new() -> Result<Self> {
        let quad = |half: f32, z: f32| {
            [
                [-half, -half, z],
                [half, -half, z],
                [half, half, z],
                [-half, half, z],
            ]
            .map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: [1.; 4],
            })
        };
        Ok(Self {
            floor: quad(4., 0.),
            card: quad(HALF as f32, 1.),
            camera: Camera::look_at(
                Vec3::new(0., 0., 4.),
                Vec3::ZERO,
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                100.,
            )?,
            shadows: SunShadows {
                split_ends: vec![8.],
                disk_angle: 0.02,
            },
        })
    }
    /// The sun alone, or with the lamp at `radius` metres of source.
    fn lighting(radius: Option<f32>) -> Lighting {
        let mut lights = vec![Light::Directional {
            travel: Vec3::new(0., 0., -1.),
            lux: [50.; 3],
        }];
        lights.extend(radius.map(|source_radius| Light::Point {
            position: LAMP.as_vec3(),
            candela: [4000.; 3],
            range: None,
            source_radius,
        }));
        Lighting {
            lights,
            environment: [0.; 3],
            exposure_stops: -6.,
        }
    }
    fn scene<'a>(
        &'a self,
        objects: &'a [ForwardObject<'a>],
        lighting: &'a Lighting,
        primary: Primary,
        local_shadows: Option<LocalShadows>,
        reconstruction: Option<Reconstruct>,
    ) -> ForwardScene<'a> {
        ForwardScene {
            meshes: None,
            materials: None,
            instances: None,
            objects,
            lighting,
            camera: self.camera,
            size: SIZE,
            atmosphere: renderlab::Atmosphere::off(),
            sun_shadows: Some(&self.shadows),
            primary,
            post: None,
            indirect: None,
            reconstruction,
            reflections: None,
            textures: None,
            local_shadows,
            transparent: &[],
            programs: None,
        }
    }
    fn objects(&self) -> [ForwardObject<'_>; 2] {
        [(&self.floor, 0.5), (&self.card, 0.8)].map(|(vertices, base)| ForwardObject {
            vertices,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material: MaterialDescriptor {
                base_color: [base, base, base, 1.],
                metallic: 0.,
                roughness: 1.,
                double_sided: true,
                ..Default::default()
            },
        })
    }
    /// What each pixel's centre ray meets, row-major.
    fn pixels(&self) -> Vec<Sees> {
        let inverse = self.camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
        let eye = DVec3::new(0., 0., 4.);
        let outside = |p: DVec3| p.x.abs().max(p.y.abs()) - HALF;
        (0..SIZE[1])
            .flat_map(|y| (0..SIZE[0]).map(move |x| (x, y)))
            .map(|(x, y)| {
                let far = inverse.project_point3(DVec3::new(
                    (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2. - 1.,
                    1. - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.,
                    0.5,
                ));
                let direction = far - eye;
                let at = |z: f64| eye + direction * ((z - eye.z) / direction.z);
                let silhouette = outside(at(1.));
                if silhouette.abs() < 0.02 {
                    Sees::Edge
                } else if silhouette < 0. {
                    Sees::Card
                } else {
                    // The segment to the lamp crosses the card's plane a third of the way.
                    let floor = at(0.);
                    Sees::Floor {
                        outside: outside(floor + (LAMP - floor) / LAMP.z),
                    }
                }
            })
            .collect()
    }
}

fn red(hdr: &[u8]) -> Vec<f32> {
    hdr.as_chunks::<8>()
        .0
        .iter()
        .map(|p| f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32())
        .collect()
}

/// On a traced frame with local shadows on, every local light's record names itself
/// and the shading program traces toward it.
///
/// - A lamp with no source radius traces one ray: a floor pixel is the sun-only frame
///   where the card stands between its point and the lamp, and the unshadowed frame
///   everywhere else, bit for bit, as is the card's own top. The edge is decided per
///   pixel by the segment from the pixel's floor point to the lamp.
/// - A lamp with a source radius traces four rays over a disk: away from the edge
///   nothing changes; within the source's reach of it every pixel holds a whole number
///   of quarters of the lamp's light, and some hold a strict fraction.
/// - Under reconstruction the four samples turn with the frame's sample index: the cold
///   first frame is the plain frame outside the penumbra, and inside it other points of
///   the disk are asked, still four of them.
/// - The mesh pass refuses the setting: its shadow maps are not assembled.
#[test]
fn a_lamp_behind_a_card_shadows_the_floor_where_the_card_blocks_it() -> Result<()> {
    let path = root("local-shadows");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let scene = Scene::new()?;
    let objects = scene.objects();
    let on = Some(LocalShadows::default());
    let mut frame = |radius: Option<f32>, local: Option<LocalShadows>| -> Result<Vec<f32>> {
        let lighting = Scene::lighting(radius);
        let images = render(
            &mut gpu,
            &cache,
            &scene.scene(&objects, &lighting, Primary::Traced, local, None),
        )?;
        Ok(red(&images.hdr))
    };
    let sun = frame(None, None)?;
    let (open, hard) = (frame(Some(0.), None)?, frame(Some(0.), on)?);
    const RADIUS: f32 = 0.15;
    let (soft_open, soft) = (frame(Some(RADIUS), None)?, frame(Some(RADIUS), on)?);
    let sees = scene.pixels();
    // The lamp's share of its unshadowed light at a pixel.
    let share = |frame: &[f32], open: &[f32], i: usize| (frame[i] - sun[i]) / (open[i] - sun[i]);
    let (mut lit, mut dark, mut tops, mut fractions) = (0, 0, 0, 0);
    for (i, sees) in sees.iter().enumerate() {
        let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
        match *sees {
            Sees::Edge => {}
            Sees::Card => {
                tops += 1;
                ensure!(
                    hard[i] == open[i] && soft[i] == soft_open[i] && open[i] > 2. * sun[i],
                    "the card's top at ({x}, {y}) changed: {} and {} against {} and {}",
                    hard[i],
                    soft[i],
                    open[i],
                    soft_open[i]
                );
            }
            Sees::Floor { outside } => {
                ensure!(
                    open[i] > 1.5 * sun[i],
                    "the lamp does not reach the floor at ({x}, {y})"
                );
                // One ray: lit or not, by which side of the card's edge it passes.
                if outside > 1e-3 {
                    lit += 1;
                    ensure!(
                        hard[i] == open[i],
                        "lit floor at ({x}, {y}) is {}, unshadowed {}",
                        hard[i],
                        open[i]
                    );
                } else if outside < -1e-3 {
                    dark += 1;
                    ensure!(
                        (hard[i] - sun[i]).abs() <= 1e-3 * sun[i],
                        "shadowed floor at ({x}, {y}) is {}, the sun alone gives {}",
                        hard[i],
                        sun[i]
                    );
                }
                // Four rays over a disk facing the pixel: its reach in the card's plane
                // is under the radius itself.
                let quarters = share(&soft, &soft_open, i) * 4.;
                let whole = if outside > f64::from(RADIUS) {
                    4.
                } else if outside < -f64::from(RADIUS) {
                    0.
                } else {
                    quarters.round()
                };
                ensure!(
                    (quarters - whole).abs() < 0.05 && (0. ..=4.).contains(&whole),
                    "floor at ({x}, {y}), {outside:.3} m outside the card, holds {quarters:.3} quarters of the lamp"
                );
                fractions += usize::from(whole > 0.5 && whole < 3.5);
            }
        }
    }
    ensure!(
        lit > 2000 && dark > 300 && tops > 300 && fractions > 30,
        "the fixture lost its regions: {lit} lit, {dark} shadowed, {tops} card, {fractions} partly lit"
    );
    // Reconstruction turns the disk samples: sample k draws lattice points 4k + 4 to
    // 4k + 7, where a frame without it draws 0 to 3. Its cold first frame is unjittered
    // and starts every pixel from its own sample, so it is the plain frame except in
    // the penumbra, where other points of the disk are asked.
    let lighting = Scene::lighting(Some(RADIUS));
    let mut history = Frames::default();
    let images = history.render(
        &mut gpu,
        &cache,
        &scene.scene(
            &objects,
            &lighting,
            Primary::Traced,
            on,
            Some(Reconstruct {
                settings: Reconstruction::default(),
                read_history: false,
            }),
        ),
    )?;
    history.retire(&mut gpu)?;
    let cold = red(&images.hdr);
    // One step of the half-float target, as the cold frame is held elsewhere.
    let step = |a: f32, b: f32| {
        let bits = |v: f32| i32::from(f16::from_f32(v).to_bits());
        (bits(a) - bits(b)).abs() <= 1
    };
    let mut turned = 0;
    for (i, sees) in sees.iter().enumerate() {
        let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
        match *sees {
            Sees::Edge => {}
            Sees::Floor { outside } if outside.abs() <= f64::from(RADIUS) => {
                let quarters = share(&cold, &soft_open, i) * 4.;
                ensure!(
                    (quarters - quarters.round()).abs() < 0.05,
                    "cold reconstructed floor at ({x}, {y}) holds {quarters:.3} quarters of the lamp"
                );
                turned += usize::from((quarters - share(&soft, &soft_open, i) * 4.).abs() > 0.5);
            }
            _ => ensure!(
                step(cold[i], soft[i]),
                "the cold reconstructed frame differs outside the penumbra at ({x}, {y}): {} against {}",
                cold[i],
                soft[i]
            ),
        }
    }
    ensure!(
        turned > 20,
        "the cold reconstructed frame asked the same disk points as the plain frame ({turned} penumbra pixels differ)"
    );
    let lighting = Scene::lighting(Some(0.));
    ensure!(
        render(
            &mut gpu,
            &cache,
            &scene.scene(&objects, &lighting, Primary::Raster, on, None),
        )
        .is_err(),
        "the mesh pass accepted local-light shadows"
    );
    eprintln!(
        "local shadows: {lit} lit, {dark} shadowed and {tops} card pixels exact; {fractions} partly lit on quarters; {turned} of them differ on the cold reconstructed frame"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
