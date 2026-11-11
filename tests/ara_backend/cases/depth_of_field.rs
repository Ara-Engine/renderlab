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

fn camera() -> Result<Camera> {
    Ok(Camera::look_at(
        Vec3::new(0.03, 0.02, 4.),
        Vec3::new(0.03, 0.02, 0.),
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?)
}
/// The fixture's frame under a sun of `lux`: a card of albedo 0.9 a metre over a floor
/// of albedo 0.2, through post with depth of field alone, focused at `focus` metres.
fn fixture_frame(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    lux: f32,
    primary: Primary,
    focus: Option<f32>,
) -> Result<renderlab::backend_ara::forward::ForwardImages> {
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
            lux: [lux; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![8.],
        disk_angle: 0.02,
    };
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
    render(
        gpu,
        cache,
        &ForwardScene {
            meshes: None,
            instances: None,
            objects: &objects,
            lighting: &lighting,
            camera: camera()?,
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
    )
}

/// A bright card 3 m from the camera over a dark floor 4 m from it, seen from straight
/// above, through the post chain with depth of field alone. Along the row through the
/// card's middle, across its left edge:
///
/// - Focused on the floor, the card is out of focus by the thin lens's radius for its
///   distance (3.4 px). Pixels further than that from the edge, on either side, keep
///   the sharp frame's value; pixels well inside it are between the floor and the
///   card, and the row never falls from the floor's side to the card's.
/// - Focused on the card, the floor is out of focus by the same formula and is flat, so
///   it does not change away from the edge, and every card pixel keeps the sharp
///   frame's value: an in-focus foreground keeps its edge.
/// - "Keeps its value" is measured where the pass writes it, in the half floats depth
///   of field leaves, and allows one half-float step (0.1%): out of the blur's reach a
///   pixel is still a weighted mean of up to 48 bilinear taps of its own flat surface,
///   which need not round back to the value it started from. Displayed levels cannot
///   carry this check: a step is 0.035 of a level here, the card sits 0.08 of a level
///   above a rounding boundary, and devices place that boundary a few hundredths
///   apart, so one step shows as a whole level on some and not on others.
/// - The mesh pass, whose surface comes from the depth target, and the traced frame,
///   whose surface comes from the hit records, display the same image in every case.
#[test]
fn depth_of_field_blurs_by_the_thin_lens_radius_and_keeps_the_focused_edge() -> Result<()> {
    let path = root("depth-of-field");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let camera = camera()?;
    let mut frame = |primary: Primary, focus: Option<f32>| {
        fixture_frame(&mut gpu, &cache, 1000., primary, focus)
    };
    let display = |images: &renderlab::backend_ara::forward::ForwardImages| {
        images.display.clone().context("post did not run")
    };
    // The red half float of each pixel of the row through the card's middle.
    let halves = |hdr: &[u8]| -> Vec<i32> {
        (0..SIZE[0] as usize)
            .map(|x| {
                let at = (48 * SIZE[0] as usize + x) * 8;
                i32::from(u16::from_le_bytes([hdr[at], hdr[at + 1]]))
            })
            .collect()
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
        let (was, now) = (
            halves(&sharp.hdr),
            halves(
                blurred
                    .focused
                    .as_ref()
                    .context("depth of field did not run")?,
            ),
        );
        let (sharp, blurred) = (display(&sharp)?, display(&blurred)?);
        let traced = display(&frame(Primary::Traced, Some(focus))?)?;
        let apart = blurred
            .iter()
            .zip(&traced)
            .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
            .max()
            .unwrap_or(0);
        ensure!(
            apart <= 1 && display(&frame(Primary::Traced, None)?)? == sharp,
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
            if distance.abs() > radius + 1.5 || (focus == 3. && distance > 0.) {
                // Positive half floats order as their bits do: one apart is one step.
                checked += 1;
                exact += usize::from(now[x] == was[x]);
                ensure!(
                    (now[x] - was[x]).abs() <= 1,
                    "focused on the {name}: pixel {x}, {distance:.2} px from the edge, is the half float {:#06x}, sharp {:#06x}",
                    now[x],
                    was[x]
                );
            } else if focus == 4. && distance.abs() < radius - 1. {
                let level = blurred[x];
                ensure!(
                    level > dark + 2 && level < bright - 2,
                    "focused on the {name}: pixel {x}, {distance:.2} px from the edge, is {level}: not between {dark} and {bright}"
                );
            }
            ensure!(
                blurred[x] >= blurred[x - 1] - 1,
                "focused on the {name}: the row falls at pixel {x}"
            );
        }
        report += &format!(
            "{name}: radius {radius:.2} px, {checked} out of reach within a half-float step ({exact} exact), mesh pass and traced {apart} levels apart; "
        );
    }
    eprintln!("depth of field: {report}");
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// What the blur does to a flat region, in half-float steps, and where each device
/// puts a display level's boundary.
///
/// A card pixel out of the edge's reach is a weighted mean of 48 bilinear taps of one
/// value, mixed and stored as a half float, where the sharp frame stores that value
/// once. The test steps the sun down by 0.05% a frame across more than one display
/// level and prints, for one such pixel:
///
/// - the card's HDR half float and the level a CPU conversion of it gives before
///   rounding (the scalar tone and sRGB replay);
/// - the level the device displays for it: its own tone stage, sRGB encode and
///   UNORM store, which no CPU code touches;
/// - the half float depth of field leaves, how many steps below the sharp one it is,
///   and the level displayed for that.
///
/// It requires what the pass must give anywhere: the blurred value within one
/// half-float step of the sharp one, and each displayed level within one of the CPU's.
///
/// Measured so far: Metal loses the step at 1 of the 16 brightnesses; an RTX 4070 SUPER
/// loses it at 12 of 16, always downward. That is a bias and not noise: a blurred flat
/// region is 0.03% darker on that vendor (a weighted sum and divide that rounds down).
/// It is harmless at this size, and the first place to look if a later test sees
/// blurred regions drift dark on NVIDIA.
#[test]
fn depth_of_field_keeps_a_flat_region_within_a_level_at_every_brightness() -> Result<()> {
    let path = root("depth-of-field-flat");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // A card pixel 8 px inside its left edge, on the row through its middle.
    let at = (48 * SIZE[0] + 56) as usize;
    // The frame's post settings as the tone stage receives them.
    let words = renderlab::backend_ara::forward::post_record(
        &PostSettings {
            bloom: 0.,
            motion_blur: false,
            vignette: 0.,
            ..Default::default()
        },
        SIZE[1],
        1,
    );
    let cpu_level = |half: u16| {
        let v = f16::from_bits(half).to_f32();
        let image = reference::Image {
            size: [1, 1],
            pixels: vec![[v, v, v, 1.]],
        };
        let fixture = reference::Fixture {
            hdr: image.clone(),
            lut: (0..8u32)
                .map(|i| [i & 1, (i >> 1) & 1, i >> 2, 1].map(|v| v as f32))
                .collect(),
            lut_size: 2,
            settings: words,
        };
        reference::display(&reference::tone(&image, &image, &fixture)).pixels[0][0] * 255.
    };
    let mut table = String::new();
    let (mut lost_most, mut apart) = (0i32, 0);
    for step in 0..16 {
        let lux = 1000. * (1. - step as f32 * 5e-4);
        let sharp = fixture_frame(&mut gpu, &cache, lux, Primary::Raster, None)?;
        let blurred = fixture_frame(&mut gpu, &cache, lux, Primary::Raster, Some(4.))?;
        let half = |bytes: &[u8]| u16::from_le_bytes([bytes[at * 8], bytes[at * 8 + 1]]);
        let level = |images: &renderlab::backend_ara::forward::ForwardImages| {
            images
                .display
                .as_ref()
                .map(|d| i32::from(d[at * 4]))
                .context("post did not run")
        };
        let (was, now) = (
            half(&sharp.hdr),
            half(
                blurred
                    .focused
                    .as_ref()
                    .context("depth of field did not run")?,
            ),
        );
        let (shown, shown_blurred) = (level(&sharp)?, level(&blurred)?);
        // Both are in one binade here, so their bits differ by half-float steps.
        let lost = i32::from(was) - i32::from(now);
        lost_most = lost_most.max(lost.abs());
        apart += usize::from(shown != shown_blurred);
        table += &format!(
            "{was:#06x} cpu {:.2} shown {shown} | blurred {now:#06x} ({lost} steps) cpu {:.2} shown {shown_blurred}; ",
            cpu_level(was),
            cpu_level(now)
        );
        ensure!(
            lost.abs() <= 1,
            "at {lux} lux depth of field leaves the flat card {lost} half-float steps below {was:#06x}"
        );
        for (half, shown) in [(was, shown), (now, shown_blurred)] {
            ensure!(
                (cpu_level(half) - shown as f32).abs() <= 1.,
                "at {lux} lux {half:#06x} is displayed as {shown}, the CPU conversion gives {:.2}",
                cpu_level(half)
            );
        }
    }
    eprintln!(
        "depth of field, flat card per 0.05% step of the sun: {table}most steps lost {lost_most}, {apart} of 16 a level apart"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
