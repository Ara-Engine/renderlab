//! Depth of field on the assembled frame, against the thin lens.
use super::*;
use renderlab::{
    Camera, DepthOfField, Light, Lighting, Mat4, MaterialDescriptor, PostSettings, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];

/// The lens both focus cases use.
const FOCAL_MM: f32 = 100.;
const F_NUMBER: f32 = 0.5;
/// A thin lens's blur radius in pixels for a point `z` metres away when focused at
/// `focus`: half the circle of confusion `f^2 / (N (s - f)) * |z - s| / z` on the
/// sensor, at the sensor's default height over the image's.
fn blur_radius(focus: f64, z: f64) -> f64 {
    let f = f64::from(FOCAL_MM) * 1e-3;
    let sensor = f64::from(DepthOfField::default().sensor_height_mm) * 1e-3;
    f * f / (f64::from(F_NUMBER) * (focus - f)) * (z - focus).abs() / z * f64::from(SIZE[1])
        / sensor
        * 0.5
}

/// A bright card 3 m from the camera over a dark floor 4 m from it, seen from straight
/// above, through the post chain with depth of field alone. Along the row through the
/// card's middle, across its left edge:
///
/// - Focused on the floor, the card is out of focus by the thin lens's radius for its
///   distance (3.4 px). Pixels further than that from the edge, on either side, are the
///   sharp frame's bytes; pixels well inside it are between the floor and the card, and
///   the row never falls from the floor's side to the card's.
/// - Focused on the card, the floor is out of focus by the same formula and is flat, so
///   it does not change away from the edge, and every card pixel is the sharp frame's:
///   an in-focus foreground keeps its edge.
/// - The mesh pass, whose surface comes from the depth target, and the traced frame,
///   whose surface comes from the hit records, display the same image in every case.
#[test]
fn depth_of_field_blurs_by_the_thin_lens_radius_and_keeps_the_focused_edge() -> Result<()> {
    let path = root("depth-of-field");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
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
    let (floor, card) = (quad(6., 0.), quad(0.5, 1.));
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0., 0., -1.),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![8.],
        disk_angle: 0.02,
    };
    let camera = Camera::look_at(
        Vec3::new(0.03, 0.02, 4.),
        Vec3::new(0.03, 0.02, 0.),
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let mut frame = |primary: Primary, focus: Option<f32>| -> Result<Vec<u8>> {
        let objects = [(&floor, 0.2), (&card, 0.9)].map(|(vertices, base)| ForwardObject {
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
        });
        let images = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                meshes: None,
                objects: &objects,
                lighting: &lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&shadows),
                primary,
                post: Some(PostSettings {
                    bloom: 0.,
                    motion_blur: false,
                    vignette: 0.,
                    depth_of_field: focus.map(|focus_distance_m| DepthOfField {
                        focus_distance_m,
                        focal_length_mm: FOCAL_MM,
                        f_number: F_NUMBER,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &[],
                programs: None,
            },
        )?;
        images.display.context("post did not run")
    };
    // The card's left edge on the row through its middle, in pixels.
    let matrix = camera.matrix(SIZE[0], SIZE[1]);
    let edge = f64::from((matrix.project_point3(Vec3::new(-0.5, 0., 1.)).x + 1.) * 0.5)
        * f64::from(SIZE[0]);
    let row = |image: &[u8]| -> Vec<i32> {
        (0..SIZE[0])
            .map(|x| i32::from(image[((48 * SIZE[0] + x) * 4) as usize]))
            .collect()
    };
    let mut report = String::new();
    for (focus, name) in [(4f32, "floor"), (3., "card")] {
        let sharp = frame(Primary::Raster, None)?;
        let blurred = frame(Primary::Raster, Some(focus))?;
        let traced = frame(Primary::Traced, Some(focus))?;
        let apart = blurred
            .iter()
            .zip(&traced)
            .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
            .max()
            .unwrap_or(0);
        ensure!(
            apart <= 1 && frame(Primary::Traced, None)? == sharp,
            "focused on the {name}: the mesh pass and the traced frame display {apart} levels apart"
        );
        let (sharp, blurred) = (row(&sharp), row(&blurred));
        let (dark, bright) = (sharp[10], sharp[64]);
        // What is out of focus: the card (3 m) or the floor (4 m).
        let radius = blur_radius(f64::from(focus), if focus == 4. { 3. } else { 4. });
        ensure!(
            (3. ..4.).contains(&radius) && bright > dark + 40,
            "the fixture lost its blur or its contrast: {radius:.2} px, {dark} and {bright}"
        );
        let (mut checked, mut exact) = (0usize, 0usize);
        // Clear of the card's right edge, 31 pixels further on.
        for x in 20..62usize {
            let distance = x as f64 + 0.5 - edge;
            let (now, was) = (blurred[x], sharp[x]);
            if distance.abs() > radius + 1.5 || (focus == 3. && distance > 0.) {
                // Out of the blur's reach a pixel is still a weighted mean of up to 48
                // taps, all of its own flat surface: the surface's value to the last
                // float bit or one off, which is one display level where that value
                // sits on a rounding boundary (an RTX card showed 192 for 193 at one
                // pixel 5.8 px inside the card, where no floor tap can reach). A leak
                // would move many pixels and by more, so most must be exact.
                checked += 1;
                exact += usize::from(now == was);
                ensure!(
                    (now - was).abs() <= 1,
                    "focused on the {name}: pixel {x}, {distance:.2} px from the edge, is {now}, sharp {was}"
                );
            } else if focus == 4. && distance.abs() < radius - 1. {
                ensure!(
                    now > dark + 2 && now < bright - 2,
                    "focused on the {name}: pixel {x}, {distance:.2} px from the edge, is {now}: not between {dark} and {bright}"
                );
            }
            ensure!(
                blurred[x] >= blurred[x - 1] - 1,
                "focused on the {name}: the row falls at pixel {x}"
            );
        }
        ensure!(
            exact * 10 >= checked * 9,
            "focused on the {name}: only {exact} of {checked} pixels out of the blur's reach are the sharp frame's"
        );
        report += &format!(
            "{name}: radius {radius:.2} px, {exact} of {checked} out of reach exact, mesh pass and traced {apart} apart; "
        );
    }
    eprintln!("depth of field: {report}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
