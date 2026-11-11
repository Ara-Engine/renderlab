//! Temporal reconstruction through the adapter.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Reconstruction, Vec3, Vertex,
    backend_ara::{
        forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows},
        kernel::Kernel,
        temporal::{Reconstruct, resolve_program},
    },
};

/// `tr1_resolve` reflects the interface `reconstruction_layout` declares, emits SPIR-V
/// that validates and becomes a native pipeline. Nothing is replayed here.
#[test]
fn temporal_resolve_reflects_validates_and_compiles() -> Result<()> {
    let path = root("temporal-kernels");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let program = resolve_program()?;
    let reflected: Vec<_> = program
        .inputs()
        .iter()
        .map(|i| (i.key.group, i.key.binding))
        .collect();
    // The hit records, the primary frame, the tiles, the TR1 frame record, the frame's
    // samples, history in and out (colour, anchor, layer), the output; then the
    // geometry arrays with previous models and previous skinned vertices.
    let expected = [
        (0, 7),
        (0, 10),
        (0, 21),
        (0, 40),
        (0, 41),
        (0, 42),
        (0, 43),
        (0, 44),
        (0, 45),
        (0, 46),
        (0, 47),
        (0, 48),
        (1, 0),
        (1, 2),
        (1, 3),
        (1, 5),
        (1, 6),
    ];
    ensure!(
        reflected == expected && program.threads()? == [8, 8, 1],
        "temporal resolve interface changed: {:?}",
        program.inputs()
    );
    program.msl()?;
    forward_cases::validate_spirv(&program, "tr1-resolve")?;
    Kernel::from_program(&mut gpu, &cache, &program)?.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A bright card over a dark floor, both facing a camera that looks straight down at
/// them under a sun straight down: flat radiance on each, and straight silhouette
/// edges at known sub-pixel positions.
struct Card {
    floor: [Vertex; 4],
    card: [Vertex; 4],
    lighting: Lighting,
    shadows: SunShadows,
}
const SIZE: [u32; 2] = [128, 96];
/// One frame: its HDR as RGBA f32, and its history colour and reset flag.
type Resolved = (Vec<f32>, Option<(Vec<f32>, bool)>);
impl Card {
    fn new() -> Self {
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
        Self {
            floor: quad(4., 0.),
            card: quad(0.5, 1.),
            lighting: Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(0., 0., -1.),
                    lux: [1000.; 3],
                }],
                environment: [0.; 3],
                exposure_stops: -8.,
            },
            shadows: SunShadows {
                split_ends: vec![8.],
                disk_angle: 0.02,
            },
        }
    }
    /// Looking down from 4 m, `shift` metres off the card's centre in x and y.
    fn camera(shift: [f32; 2]) -> Result<Camera> {
        Ok(Camera::look_at(
            Vec3::new(shift[0], shift[1], 4.),
            Vec3::new(shift[0], shift[1], 0.),
            Vec3::Y,
            55f32.to_radians(),
            0.1,
            100.,
        )?)
    }
    /// One traced frame per camera through `history`.
    fn frames(
        &self,
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        history: &mut Frames,
        reconstruction: Option<Reconstruct>,
        cameras: &[Camera],
    ) -> Result<Vec<Resolved>> {
        let material = |base: f32| MaterialDescriptor {
            base_color: [base, base, base, 1.],
            metallic: 0.,
            roughness: 1.,
            double_sided: true,
            ..Default::default()
        };
        let objects =
            [(&self.floor, 0.2), (&self.card, 0.9)].map(|(vertices, base)| ForwardObject {
                vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: material(base),
            });
        let mut out = Vec::new();
        for camera in cameras {
            let images = history.render(
                gpu,
                cache,
                &ForwardScene {
                    objects: &objects,
                    lighting: &self.lighting,
                    camera: *camera,
                    size: SIZE,
                    atmosphere: renderlab::Atmosphere::off(),
                    sun_shadows: Some(&self.shadows),
                    primary: Primary::Traced,
                    post: None,
                    indirect: None,
                    reconstruction,
                },
            )?;
            let hdr = images
                .hdr
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f16::from_bits(u16::from_le_bytes(*b)).to_f32())
                .collect();
            out.push((hdr, images.reconstruction_history));
        }
        Ok(out)
    }
}
fn at(x: u32, y: u32) -> usize {
    ((y * SIZE[0] + x) * 4) as usize
}
fn reconstructed() -> Option<Reconstruct> {
    Some(Reconstruct {
        settings: Reconstruction::default(),
        read_history: true,
    })
}
/// The least, mean and greatest accumulated weight of a history colour image.
fn weights(colour: &[f32]) -> [f32; 3] {
    let all = colour.as_chunks::<4>().0.iter().map(|p| p[3]);
    let (lo, hi, sum) = all.fold((f32::MAX, 0f32, 0f64), |(lo, hi, sum), w| {
        (lo.min(w), hi.max(w), sum + f64::from(w))
    });
    [lo, (sum / (colour.len() / 4) as f64) as f32, hi]
}

/// A still camera over the card. The first frame restarts: every pixel has weight one
/// and the image is the unreconstructed frame, whose sample is the pixel centre. Each
/// later frame adds one sample of weight to the mean and leaves flat regions at their
/// exact radiance. After 48 frames a pixel the card's edge crosses holds the card's and
/// the floor's radiance in proportion to the area of the pixel the card covers, which
/// the camera gives in closed form: the reconstruction has turned 48 jittered point
/// samples into coverage.
#[test]
fn reconstruction_accumulates_jittered_samples_into_edge_coverage() -> Result<()> {
    let path = root("temporal-coverage");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let scene = Card::new();
    let camera = Card::camera([0.03, 0.02])?;
    const FRAMES: usize = 48;
    let plain = scene.frames(&mut gpu, &cache, &mut Frames::default(), None, &[camera])?;
    let mut history = Frames::default();
    let frames = scene.frames(
        &mut gpu,
        &cache,
        &mut history,
        reconstructed(),
        &[camera; FRAMES],
    )?;
    history.retire(&mut gpu)?;
    let (card, floor) = (plain[0].0[at(64, 48)], plain[0].0[at(10, 10)]);
    ensure!(
        card > 4. * floor && floor > 0.1,
        "the fixture lost its contrast: card {card}, floor {floor}"
    );
    for (index, (hdr, history)) in frames.iter().enumerate() {
        let (colour, reset) = history.as_ref().context("history was not read")?;
        let [least, mean, most] = weights(colour);
        ensure!(*reset == (index == 0), "frame {index} has reset {reset}");
        if index == 0 {
            // Colour only: the plain frame's alpha still holds shading's view hint.
            let same = hdr
                .as_chunks::<4>()
                .0
                .iter()
                .zip(plain[0].0.as_chunks::<4>().0)
                .all(|(a, b)| a[..3] == b[..3]);
            ensure!(
                [least, most] == [1., 1.] && same,
                "the first frame is not the plain frame at weight one"
            );
        }
        // One more sample a frame on average, within a percent: silhouette pixels
        // start from their neighbours and are held to a shorter window.
        ensure!(
            (mean / (index + 1) as f32 - 1.).abs() < 0.01 && least >= 1.,
            "frame {index}: weights {least} to {most}, mean {mean}"
        );
        // Away from the edges the shading is smooth, so jittered samples agree with the
        // pixel-centre one.
        for (x, y) in [(64, 48), (10, 10), (100, 80)] {
            let (value, flat) = (hdr[at(x, y)], plain[0].0[at(x, y)]);
            ensure!(
                (value - flat).abs() <= 1e-3 * flat && hdr[at(x, y) + 3] == 1.,
                "frame {index}: flat pixel ({x}, {y}) is {value}, not {flat}"
            );
        }
    }
    // Where the card's left and top edges fall, in pixels.
    let matrix = camera.matrix(SIZE[0], SIZE[1]);
    let pixel = |world: Vec3| {
        let ndc = matrix.project_point3(world);
        [
            (ndc.x + 1.) * 0.5 * SIZE[0] as f32,
            (1. - ndc.y) * 0.5 * SIZE[1] as f32,
        ]
    };
    let (left, top) = (
        pixel(Vec3::new(-0.5, 0., 1.))[0],
        pixel(Vec3::new(0., 0.5, 1.))[1],
    );
    let last = &frames[FRAMES - 1].0;
    let mut report = String::new();
    for (name, x, y, covered) in [
        ("left", left as u32, 48, 1. - left.fract()),
        ("top", 64, top as u32, 1. - top.fract()),
    ] {
        ensure!(
            (0.15..0.85).contains(&covered),
            "the {name} edge sits too close to a pixel boundary: {covered}"
        );
        let measured = (last[at(x, y)] - floor) / (card - floor);
        report += &format!("{name} edge: covered {covered:.3}, measured {measured:.3}; ");
        // A reconstruction filter is not a box; 48 samples of a 256-sample sequence.
        ensure!(
            (measured - covered).abs() < 0.03,
            "{name} edge pixel ({x}, {y}) holds {measured:.3} of the card, its area is {covered:.3}"
        );
    }
    eprintln!("{report}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// History across events. A camera that pans half a pixel a frame keeps its history
/// (no restart; the floor's weight keeps growing through reprojection). A cut restarts
/// every pixel, and so does turning the sun past the turn angle: the weights fall back
/// to what a spatial start can give (`start_window`), and the frame after a restart
/// adds one sample again. Two runs of the same sequence give the same image and the
/// same history.
#[test]
fn reconstruction_history_survives_a_pan_and_restarts_on_cuts_and_sun_turns() -> Result<()> {
    let path = root("temporal-history");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let mut scene = Card::new();
    // Half a pixel a frame at the floor: 4 m away, 96 rows over 55 degrees.
    let step = 0.5 * 2. * 4. * (27.5f32.to_radians()).tan() / SIZE[1] as f32;
    let pan: Vec<Camera> = (0..10)
        .map(|i| Card::camera([0.03 + step * i as f32, 0.02]))
        .collect::<Result<_>>()?;
    let mut history = Frames::default();
    let first = scene.frames(&mut gpu, &cache, &mut history, reconstructed(), &pan)?;
    for (index, (_, read)) in first.iter().enumerate() {
        let (colour, reset) = read.as_ref().context("history was not read")?;
        ensure!(
            *reset == (index == 0),
            "pan frame {index} has reset {reset}"
        );
        // A floor pixel well inside the image, away from the card and its shadow.
        let weight = colour[at(100, 80) + 3];
        ensure!(
            weight > 0.6 * (index + 1) as f32,
            "pan frame {index}: the floor's weight is {weight}"
        );
    }
    // A cut, then a still frame; then the sun turned by 0.2 rad.
    history.cut();
    let still = [*pan.last().context("no camera")?];
    let cut = scene.frames(&mut gpu, &cache, &mut history, reconstructed(), &still)?;
    let kept = scene.frames(&mut gpu, &cache, &mut history, reconstructed(), &still)?;
    scene.lighting.lights = vec![Light::Directional {
        travel: Vec3::new(0.2f32.sin(), 0., -(0.2f32.cos())),
        lux: [1000.; 3],
    }];
    let turned = scene.frames(&mut gpu, &cache, &mut history, reconstructed(), &still)?;
    history.retire(&mut gpu)?;
    let start = Reconstruction::default().start_window;
    let before = weights(&first[9].1.as_ref().context("history was not read")?.0);
    let mut means = Vec::new();
    for (name, frame, restarted) in [
        ("cut", &cut, true),
        ("still", &kept, false),
        ("turn", &turned, true),
    ] {
        let (colour, reset) = frame[0].1.as_ref().context("history was not read")?;
        let [least, mean, most] = weights(colour);
        ensure!(
            *reset == restarted && (!restarted || (least >= 1. && most <= start)),
            "after the {name}: reset {reset}, weights {least} to {most}"
        );
        means.push(mean);
    }
    ensure!(
        before[1] > 2. * means[0] && (means[1] - means[0] - 1.).abs() < 0.1 && means[2] < means[1],
        "weights across the events: {before:?} before, then means {means:?}"
    );
    let scene = Card::new();
    let mut again = Frames::default();
    let second = scene.frames(&mut gpu, &cache, &mut again, reconstructed(), &pan)?;
    again.retire(&mut gpu)?;
    ensure!(first == second, "the same sequence gave different frames");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// The fused Interactive trace's three entries, in the production soft module and in
/// reconstruction's rotated variant of it, reflect the interface their stage needs,
/// emit SPIR-V that validates and compile natively. Nothing is replayed here.
#[test]
fn interactive_trace_entries_reflect_validate_and_compile() -> Result<()> {
    use renderlab::backend_ara::primary_trace::Interactive;
    let path = root("interactive-kernels");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // The scene twice (visibility and camera rays), the visibility counts, the primary
    // frame, the hit records, the geometry and the lights with their shadow views
    // everywhere; then the tile flags and the repair queue, or the queue as input.
    let common = [
        (0, 2),
        (0, 5),
        (0, 10),
        (0, 13),
        (0, 19),
        (1, 0),
        (1, 1),
        (1, 2),
        (1, 3),
        (1, 5),
        (3, 0),
        (3, 1),
        (3, 5),
    ];
    let stages: [(Interactive, [u32; 3], &[(u32, u32)]); 3] = [
        (Interactive::Deferred, [8, 4, 1], &[(0, 20), (0, 24)]),
        (Interactive::Full, [8, 4, 1], &[(0, 20)]),
        (Interactive::Repair, [32, 1, 1], &[(0, 26)]),
    ];
    for rotated in [false, true] {
        for (entry, threads, extra) in stages {
            let program = entry.program(rotated)?;
            let mut keys = common.to_vec();
            keys.extend_from_slice(extra);
            keys.sort_unstable();
            let reflected: Vec<_> = program
                .inputs()
                .iter()
                .map(|i| (i.key.group, i.key.binding))
                .collect();
            ensure!(
                reflected == keys && program.threads()? == threads,
                "{entry:?} (rotated {rotated}) interface changed: {:?}",
                program.inputs()
            );
            program.msl()?;
            forward_cases::validate_spirv(&program, &format!("interactive-{entry:?}-{rotated}"))?;
            Kernel::from_program(&mut gpu, &cache, &program)?.retire(&mut gpu)?;
        }
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
