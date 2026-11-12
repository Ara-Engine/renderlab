//! Transparent panes on the traced frame, against what a thin pane must pass.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Transparency, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];
const IOR: f64 = 1.5;
/// A pane: its square, its coverage and its tint.
#[derive(Clone, Copy)]
struct Pane<'a> {
    vertices: &'a [Vertex; 4],
    opacity: f32,
    tint: [f32; 3],
}

fn quad(half: f32, z: f32, centre: [f32; 2]) -> [Vertex; 4] {
    [[-half, -half], [half, -half], [half, half], [-half, half]].map(|[x, y]| Vertex {
        position: [centre[0] + x, centre[1] + y, z],
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    })
}

/// A pane's light is its own shading plus what it passes of the colour behind it:
/// `out = lit + T * behind`. The floor behind is lit by the sun alone, and a pane
/// between a point and the sun passes `S` of that light (the pane's coloured shadow,
/// TP2). Two frames with different floors behind the same panes differ only in the
/// floor's light, so their difference over the two bare floors' difference is the
/// product of every `T` the view ray crosses and every `S` the floor point's sun ray
/// crosses, per pixel and channel, whatever the panes' own lighting is.
///
/// - `T` is the standard program's transmittance: what the pane's coverage leaves,
///   times what Schlick's Fresnel reflectance at the pixel's view angle leaves, times
///   the tint. `S` is the same at the sun's angle to the pane.
/// - Outside every pane and every shadow the frame is the bare floor's, bit for bit.
///   Outside every pane but inside a shadow, the floor keeps `S` of its light.
/// - At a pane's edge over unshadowed floor the four coverage samples leave whole
///   quarters: the pixel passes `1 - c (1 - T)` for `c` a quarter, a half or three
///   quarters.
/// - Where two different panes overlap, the floor's light passes both, and the far
///   pane's own light, dimmed where the near pane shadows it, passes the near one.
///   The frame is that sum, built from each pane alone over the floor. It holds in
///   either submission order, because the pass sorts far to near; drawn in the other
///   order the two panes' own light would swap roles, a difference the test measures
///   and requires to be large against its tolerance.
/// - The mesh pass refuses panes.
#[test]
fn a_pane_passes_what_its_coverage_fresnel_and_tint_leave() -> Result<()> {
    let path = root("panes");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor = quad(8., 0., [0.; 2]);
    // The near pane over the middle, the far one under its right half and beyond.
    let (near, far) = (quad(0.6, 1.5, [0.; 2]), quad(0.6, 0.75, [0.7, 0.]));
    let near = Pane {
        vertices: &near,
        opacity: 0.25,
        tint: [0.8, 0.9, 1.0],
    };
    let far = Pane {
        vertices: &far,
        opacity: 0.5,
        tint: [1.0, 0.6, 0.5],
    };
    let lighting = Lighting {
        lights: vec![Light::Directional {
            // Low enough that each pane's shadow reaches floor the camera sees past it.
            travel: Vec3::new(0.6, 0.1, -1.).normalize(),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![8.],
        disk_angle: 0.02,
    };
    let eye = Vec3::new(0., 0., 4.);
    let camera = Camera::look_at(eye, Vec3::ZERO, Vec3::Y, 55f32.to_radians(), 0.1, 100.)?;
    let mut frame = |albedo: f32, panes: &[Pane<'_>], primary: Primary| -> Result<Vec<[f32; 3]>> {
        let material = |base: [f32; 3], transparency| MaterialDescriptor {
            base_color: [base[0], base[1], base[2], 1.],
            metallic: 0.,
            roughness: 0.5,
            double_sided: true,
            transparency,
            ..Default::default()
        };
        let objects = [ForwardObject {
            vertices: &floor,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material: material([albedo; 3], None),
        }];
        let transparent: Vec<_> = panes
            .iter()
            .map(|pane| ForwardObject {
                vertices: pane.vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: material(
                    pane.tint,
                    Some(Transparency {
                        opacity: pane.opacity,
                        ior: IOR as f32,
                        thickness: 0.,
                    }),
                ),
            })
            .collect();
        let images = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                meshes: None,
                materials: None,
                instances: None,
                objects: &objects,
                lighting: &lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&shadows),
                primary,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &transparent,
                programs: None,
                sky_map: None,
                skins: None,
                colour_lut: None,
                ids: None,
            },
        )?;
        Ok(images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32()
                })
            })
            .collect())
    };
    let bare = [
        frame(0.2, &[], Primary::Traced)?,
        frame(0.6, &[], Primary::Traced)?,
    ];
    // What each pixel's centre ray meets: how far inside a pane's square it crosses the
    // pane's plane (negative outside), in pixels, and the cosine of its angle to the
    // pane's normal.
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    let eye = eye.as_dvec3();
    let pixels_per_metre =
        |z: f64| f64::from(SIZE[1]) / (2. * (eye.z - z) * (0.5 * 55f64.to_radians()).tan());
    let rays: Vec<DVec3> = (0..SIZE[0] * SIZE[1])
        .map(|i| {
            let (x, y) = (i % SIZE[0], i / SIZE[0]);
            let far = inverse.project_point3(DVec3::new(
                (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2. - 1.,
                1. - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.,
                0.5,
            ));
            (far - eye).normalize()
        })
        .collect();
    let inside = |ray: DVec3, z: f64, centre: [f64; 2]| {
        let at = eye + ray * ((z - eye.z) / ray.z);
        (0.6 - (at.x - centre[0]).abs().max((at.y - centre[1]).abs())) * pixels_per_metre(z)
    };
    let transmittance = |pane: Pane<'_>, ray: DVec3| -> [f64; 3] {
        let f0 = ((IOR - 1.) / (IOR + 1.)).powi(2);
        let fresnel = f0 + (1. - f0) * (1. - ray.z.abs()).powi(5);
        pane.tint
            .map(|tint| (1. - f64::from(pane.opacity)) * (1. - fresnel) * f64::from(tint))
    };
    // Toward the sun, a metre up: the lighting's `travel` reversed.
    let sunward = DVec3::new(-0.6, -0.1, 1.);
    // How far inside a pane's square, in pixels at the receiver, the sun's ray from a
    // receiver point crosses the pane's plane; negative outside or for a pane below.
    let shadowed = |at: DVec3, z: f64, centre: [f64; 2]| {
        if z <= at.z {
            return f64::NEG_INFINITY;
        }
        let crossing = at + sunward * (z - at.z);
        (0.6 - (crossing.x - centre[0])
            .abs()
            .max((crossing.y - centre[1]).abs()))
            * pixels_per_metre(at.z)
    };
    // Each pane alone over the darker floor, for the far pane's own light.
    let alone = [
        frame(0.2, &[near], Primary::Traced)?,
        frame(0.2, &[far], Primary::Traced)?,
    ];
    let mut report = String::new();
    for (name, panes) in [
        ("one pane", vec![near]),
        ("two panes, near first", vec![near, far]),
        ("two panes, far first", vec![far, near]),
    ] {
        let through = [
            frame(0.2, &panes, Primary::Traced)?,
            frame(0.6, &panes, Primary::Traced)?,
        ];
        let (mut interior, mut outside, mut edges, mut both, mut worst) = (0, 0, 0, 0, 0f64);
        let mut shaded = 0;
        // Under both panes: how far the frame is from the right order, and how far the
        // wrong order would have been from it.
        let (mut ordered, mut swapped) = (0f64, 0f64);
        let two = panes.len() == 2;
        for (i, &ray) in rays.iter().enumerate() {
            let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
            let far_depth = |d: f64| if two { d } else { f64::NEG_INFINITY };
            let depths = [
                inside(ray, 1.5, [0., 0.]),
                far_depth(inside(ray, 0.75, [0.7, 0.])),
            ];
            // The floor point this pixel sees, and each pane's shadow on it.
            let ground = eye + ray * (-eye.z / ray.z);
            let shadows = [
                shadowed(ground, 1.5, [0., 0.]),
                far_depth(shadowed(ground, 0.75, [0.7, 0.])),
            ];
            let passed: [f64; 3] = std::array::from_fn(|c| {
                f64::from(through[1][i][c] - through[0][i][c])
                    / f64::from(bare[1][i][c] - bare[0][i][c])
            });
            let (one, other) = (transmittance(near, ray), transmittance(far, ray));
            // The sun's passage: its angle to a pane is the same everywhere.
            let sun = [
                transmittance(near, sunward.normalize()),
                transmittance(far, sunward.normalize()),
            ];
            // Pixels a shadow's edge crosses are left out everywhere.
            if shadows.iter().any(|d| d.abs() <= 1.5) {
                continue;
            }
            let lit: [f64; 3] = std::array::from_fn(|c| {
                (0..2)
                    .filter(|p| shadows[*p] > 0.)
                    .map(|p| sun[p][c])
                    .product()
            });
            if depths.iter().all(|d| *d < -1.5) {
                if shadows.iter().all(|d| *d < 0.) {
                    outside += 1;
                    ensure!(
                        through[0][i] == bare[0][i] && through[1][i] == bare[1][i],
                        "{name}: pixel ({x}, {y}) outside every pane and shadow is not the bare floor's"
                    );
                } else {
                    shaded += 1;
                    for c in 0..3 {
                        worst = worst.max((passed[c] - lit[c]).abs());
                        ensure!(
                            (passed[c] - lit[c]).abs() < 0.01,
                            "{name}: floor pixel ({x}, {y}) in a pane's shadow keeps {:.4} of channel {c}, expected {:.4}",
                            passed[c],
                            lit[c]
                        );
                    }
                }
            } else if depths.iter().all(|d| d.abs() > 1.5) {
                // Whole pixels: inside one pane or both.
                let count = depths.iter().filter(|d| **d > 0.).count() as i32;
                interior += 1;
                both += usize::from(count == 2);
                let under = [depths[0] > 0., depths[1] > 0.];
                // Where the view ray meets the far pane, and the near pane's shadow there.
                let on_far = eye + ray * ((0.75 - eye.z) / ray.z);
                let far_shadow = shadowed(on_far, 1.5, [0., 0.]);
                for c in 0..3 {
                    let expected = [one[c], other[c]]
                        .into_iter()
                        .zip(under)
                        .map(|(t, covers)| if covers { t } else { 1. })
                        .product::<f64>()
                        * lit[c];
                    if count == 2 && far_shadow.abs() > 1.5 {
                        // Each pane's own light is its frame alone less the floor's
                        // light it passed, under its own shadow alone.
                        let floor = f64::from(bare[0][i][c]);
                        let alone_shadow = |p: usize| if shadows[p] > 0. { sun[p][c] } else { 1. };
                        let own_near = f64::from(alone[0][i][c]) - one[c] * floor * alone_shadow(0);
                        let own_far =
                            f64::from(alone[1][i][c]) - other[c] * floor * alone_shadow(1);
                        let on_far_lit = if far_shadow > 0. { sun[0][c] } else { 1. };
                        let got = f64::from(through[0][i][c]);
                        let right =
                            own_near + one[c] * (own_far * on_far_lit + other[c] * floor * lit[c]);
                        let wrong =
                            own_far * on_far_lit + other[c] * (own_near + one[c] * floor * lit[c]);
                        ordered = ordered.max((got - right).abs());
                        swapped = swapped.max((right - wrong).abs());
                    }
                    worst = worst.max((passed[c] - expected).abs());
                    ensure!(
                        (passed[c] - expected).abs() < 0.01,
                        "{name}: pixel ({x}, {y}) inside {count} pane(s) passes {:.4} of channel {c}, expected {expected:.4}",
                        passed[c]
                    );
                }
            } else if depths[0].abs() < 0.4 && depths[1] < -1.5 && shadows.iter().all(|d| *d < 0.) {
                // The near pane's own edge over bare, unshadowed floor: quarters of its
                // coverage.
                let covered = (1. - passed[1]) / (1. - one[1]) * 4.;
                ensure!(
                    (covered - covered.round()).abs() < 0.08
                        && (0. ..=4.).contains(&covered.round()),
                    "{name}: edge pixel ({x}, {y}) is covered {covered:.3} quarters"
                );
                edges += usize::from(covered.round() > 0.5 && covered.round() < 3.5);
            }
        }
        ensure!(
            panes.len() == 1 || (ordered < 0.004 && swapped > 10. * ordered.max(1e-3)),
            "{name}: under both panes the frame is {ordered:.4} from far-then-near, and the other order would differ by {swapped:.4}"
        );
        ensure!(
            interior > 1000
                && outside > 5000
                && edges > 10
                && shaded > 20
                && (panes.len() == 1 || both > 200),
            "{name}: the fixture lost its regions: {interior} inside, {outside} outside, {shaded} in a shadow alone, {edges} partly covered, {both} under both"
        );
        report += &format!(
            "{name}: {interior} inside and {shaded} in a shadow alone within {worst:.4}, {edges} edge pixels on quarters, {both} under both ({ordered:.4} from far-then-near, the other order {swapped:.4} away); "
        );
    }
    ensure!(
        frame(0.2, &[near], Primary::Raster).is_err(),
        "the mesh pass accepted a transparent pane"
    );
    eprintln!("panes: {report}");
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A sunlit floor, an upright card on it that the sun does not reach, and a red pane
/// overhead between the floor and the sun, under a black sky with the radiance cache
/// on: the card shows only the floor's bounce light, which is the pane's red.
///
/// After ten such frames the pane is carried away, and six frames later (the frame
/// that relights every cell, the cache's three gathers, and two more) the card is
/// what it is in a sequence whose pane was never overhead, within 2%, and grey. So a
/// change in the panes' sun map is a lighting change to the cache, as
/// `Renderer::prepare_gi` keys it. Nothing else tells the cache: a pane is no caster
/// in its world and the view is still, so without the map in its lighting key the
/// card stays red.
#[test]
fn a_pane_carried_away_relights_the_cells_it_shaded() -> Result<()> {
    let path = root("pane-relight");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor = quad(3., 0., [0.; 2]);
    let card = [[-1., 0.], [1., 0.], [1., 1.5], [-1., 1.5]].map(|[x, z]| Vertex {
        position: [x, 1., z],
        normal: [0., -1., 0.],
        uv: [0.; 2],
        color: [1.; 4],
    });
    let pane = quad(2.8, 2.5, [0.; 2]);
    // The sun leans away from the card's face, so no direct light reaches it.
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0., -0.05, -1.).normalize(),
            lux: [8.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: 0.,
    };
    let shadows = SunShadows {
        split_ends: vec![16.],
        disk_angle: 0.02,
    };
    let camera = Camera::look_at(
        Vec3::new(0., -7., 1.6),
        Vec3::new(0., 1., 0.7),
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let matrix = camera.matrix(SIZE[0], SIZE[1]);
    // The mean of the 8 by 8 pixels about the middle of the card.
    let patch = |hdr: &[u8]| -> [f32; 3] {
        let ndc = matrix.project_point3(Vec3::new(0., 1., 0.75));
        let centre = [
            ((ndc.x + 1.) * 0.5 * SIZE[0] as f32) as usize,
            ((1. - ndc.y) * 0.5 * SIZE[1] as f32) as usize,
        ];
        let mut sum = [0f32; 3];
        for y in centre[1] - 4..centre[1] + 4 {
            for x in centre[0] - 4..centre[0] + 4 {
                let p = &hdr[(y * SIZE[0] as usize + x) * 8..][..8];
                for (c, sum) in sum.iter_mut().enumerate() {
                    *sum += f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32();
                }
            }
        }
        sum.map(|c| c / 64.)
    };
    // One sequence: a frame for each entry, with the pane overhead where it says and
    // far to one side otherwise. The card after each frame.
    let mut sequence = |overhead: &[bool]| -> Result<Vec<[f32; 3]>> {
        let material = |base: [f32; 3], transparency| MaterialDescriptor {
            base_color: [base[0], base[1], base[2], 1.],
            metallic: 0.,
            roughness: 1.,
            double_sided: true,
            transparency,
            ..Default::default()
        };
        let objects = [&floor, &card].map(|vertices| ForwardObject {
            vertices,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material: material([0.8; 3], None),
        });
        let mut frames = Frames::default();
        let mut cards = Ok(Vec::new());
        for &overhead in overhead {
            let transparent = [ForwardObject {
                vertices: &pane,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::from_translation(Vec3::new(
                    if overhead { 0. } else { 20. },
                    0.,
                    0.,
                )),
                material: material(
                    [1., 0.2, 0.2],
                    Some(Transparency {
                        opacity: 0.,
                        ior: IOR as f32,
                        thickness: 0.,
                    }),
                ),
            }];
            let image = frames.render(
                &mut gpu,
                &cache,
                &ForwardScene {
                    meshes: None,
                    materials: None,
                    instances: None,
                    objects: &objects,
                    lighting: &lighting,
                    camera,
                    size: SIZE,
                    atmosphere: renderlab::Atmosphere::off(),
                    sun_shadows: Some(&shadows),
                    primary: Primary::Traced,
                    post: None,
                    indirect: Some(renderlab::backend_ara::gi_cache::Indirect {
                        settings: renderlab::IndirectSettings {
                            slots_log2: 17,
                            ..Default::default()
                        },
                        read_table: false,
                    }),
                    reconstruction: None,
                    reflections: None,
                    textures: None,
                    local_shadows: None,
                    transparent: &transparent,
                    programs: None,
                    sky_map: None,
                    skins: None,
                    colour_lut: None,
                    ids: None,
                },
            );
            match (image, &mut cards) {
                (Ok(image), Ok(cards)) => cards.push(patch(&image.hdr)),
                (Err(error), _) => {
                    cards = Err(error);
                    break;
                }
                _ => unreachable!("the loop stops at the first error"),
            }
        }
        frames.retire(&mut gpu)?;
        cards
    };
    let mut overhead = [true; 16];
    overhead[10..].fill(false);
    let (carried, never) = (sequence(&overhead)?, sequence(&[false; 16])?);
    let (under, after, open) = (carried[9], carried[15], never[15]);
    eprintln!(
        "pane relight: under the pane {under:?}, six frames after {after:?}, never under {open:?}"
    );
    ensure!(
        under[0] > 0.3 && under[1] < 0.3 * under[0],
        "the card under the pane is {under:?}, not lit red"
    );
    ensure!(
        open[0] > 0.3 && open[1] == open[0] && open[2] == open[0],
        "the card with the pane away is {open:?}, not lit grey"
    );
    ensure!(
        (0..3).all(|c| (after[c] - open[c]).abs() < 0.02 * open[c]),
        "six frames after the pane left the card is {after:?}, and {open:?} where it was never overhead"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
