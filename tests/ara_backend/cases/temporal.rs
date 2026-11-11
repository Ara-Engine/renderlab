//! Temporal reconstruction through the adapter.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Reconstruction, RenderScale, Vec3, Vertex,
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
    Kernel::from_program(&mut gpu, &cache, &program)?;
    cache.retire(&mut gpu)?;
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
    /// `Primary::Traced` (exact shadows) unless a test asks for the Interactive trace.
    primary: Primary,
    /// Off unless a test asks for the physical sky.
    atmosphere: renderlab::Atmosphere,
    /// None unless a test asks for the post chain, whose displayed image must then be
    /// the frame's size.
    post: Option<renderlab::PostSettings>,
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
            primary: Primary::Traced,
            atmosphere: renderlab::Atmosphere::off(),
            post: None,
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
                    meshes: None,
                    instances: None,
                    objects: &objects,
                    lighting: &self.lighting,
                    camera: *camera,
                    size: SIZE,
                    atmosphere: self.atmosphere,
                    sun_shadows: Some(&self.shadows),
                    primary: self.primary,
                    post: self.post,
                    indirect: None,
                    reconstruction,
                    reflections: None,
                    textures: None,
                    local_shadows: None,
                    transparent: &[],
                    programs: None,
                },
            )?;
            ensure!(
                images
                    .display
                    .as_ref()
                    .is_none_or(|d| d.len() == (SIZE[0] * SIZE[1] * 4) as usize)
                    && images.display.is_some() == self.post.is_some(),
                "the displayed image is not the frame's size"
            );
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
    // The card's outline in pixels: its least and greatest corner.
    let outline = {
        let matrix = camera.matrix(SIZE[0], SIZE[1]);
        let corner = |x: f32, y: f32| {
            let ndc = matrix.project_point3(Vec3::new(x, y, 1.));
            [
                (ndc.x + 1.) * 0.5 * SIZE[0] as f32,
                (1. - ndc.y) * 0.5 * SIZE[1] as f32,
            ]
        };
        [corner(-0.5, 0.5), corner(0.5, -0.5)]
    };
    for (index, (hdr, history)) in frames.iter().enumerate() {
        let (colour, reset) = history.as_ref().context("history was not read")?;
        let [least, mean, most] = weights(colour);
        ensure!(*reset == (index == 0), "frame {index} has reset {reset}");
        if index == 0 {
            // Colour only: the plain frame's alpha still holds shading's view hint.
            // Within one step of the half-float target, and a weight within 1e-4 of
            // one: the spatial start divides a tent-weighted sum by its weight, and
            // RADV rounds that to 1 + 2^-16 on some pixels where Metal and the RTX
            // card give exactly one (48 of 12,288 pixels moved one step there).
            let step = |a: f32, b: f32| {
                let bits = |v: f32| i32::from(f16::from_f32(v).to_bits());
                (bits(a) - bits(b)).abs() <= 1
            };
            let same = hdr
                .as_chunks::<4>()
                .0
                .iter()
                .zip(plain[0].0.as_chunks::<4>().0)
                .all(|(a, b)| (0..3).all(|c| step(a[c], b[c])));
            ensure!(
                least >= 1. && most <= 1.0001 && same,
                "the first frame is not the plain frame at weight one: weights {least} to {most}"
            );
        }
        // One more sample a frame on average, within a percent: silhouette pixels
        // start from their neighbours and are held to a shorter window.
        // What a failure needs to say: how many pixels fall short of one sample a frame,
        // by how much in all, and where they are. On Metal 92 pixels do from frame 4 on,
        // every one within a pixel of the card's outline (30 of them on the card), by
        // 809 samples in all at frame 16 (0.39%) and 2,116 at frame 47. RADV at frame 16
        // has 659 short by 2,135 (1.02%): more pixels, not yet placed.
        let samples = (index + 1) as f32;
        let all = colour.as_chunks::<4>().0.iter().map(|p| p[3]);
        let deficit: f32 = all.clone().map(|w| (samples - w).max(0.)).sum();
        // Short pixels by how far their centre is from the card's outline, in pixels:
        // within one, two, three, further; and how many of them lie on the card.
        let (mut rings, mut inside) = ([0usize; 4], 0usize);
        // And those on the image's own border, with the sum of their weights.
        let (mut border, mut border_weight) = (0usize, 0f32);
        for (i, w) in all.enumerate().filter(|(_, w)| *w < samples - 0.01) {
            let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
            if x == 0 || y == 0 || x == SIZE[0] - 1 || y == SIZE[1] - 1 {
                border += 1;
                border_weight += w;
            }
            let centre = [i as u32 % SIZE[0], i as u32 / SIZE[0]].map(|c| c as f32 + 0.5);
            let signed = (0..2)
                .map(|a| (outline[0][a] - centre[a]).max(centre[a] - outline[1][a]))
                .fold(f32::MIN, f32::max);
            rings[(signed.abs() as usize).min(3)] += 1;
            inside += usize::from(signed < 0.);
        }
        let short: usize = rings.iter().sum();
        ensure!(
            (mean / samples - 1.).abs() < 0.01 && least >= 1.,
            "frame {index}: weights {least} to {most}, mean {mean}; {short} pixels short of {samples} by {deficit:.0} samples in all; {rings:?} of them within 1, 2, 3 and more pixels of the card's outline, {inside} on the card; {border} on the image's border with a mean weight of {:.2}",
            border_weight / border.max(1) as f32
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
    cache.retire(&mut gpu)?;
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
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Below native scale every pass before the resolve renders at an internal size (two
/// thirds or half of the target in each axis) and the resolve fills the target from
/// those samples over frames. Under a still camera:
///
/// - the frame, its history and the displayed image are the target's size;
/// - away from the edges every frame equals the native frame, since a flat region's
///   samples all hold its value;
/// - once the history is no longer young, the mean weight grows each frame by the
///   share of target pixels a frame samples: the internal pixel count over the
///   target's (0.25 and 0.44);
/// - after 64 frames the pixel the card's left edge crosses holds the card's and the
///   floor's radiance near the proportion of its area the card covers, and the pixels
///   two away on either side are still pure.
#[test]
fn reconstruction_below_native_scale_fills_the_target_from_fewer_samples() -> Result<()> {
    let path = root("temporal-scale");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let mut scene = Card::new();
    let camera = Card::camera([0.03, 0.02])?;
    let plain = scene.frames(&mut gpu, &cache, &mut Frames::default(), None, &[camera])?;
    let (card, floor) = (plain[0].0[at(64, 48)], plain[0].0[at(10, 10)]);
    let matrix = camera.matrix(SIZE[0], SIZE[1]);
    let left = (matrix.project_point3(Vec3::new(-0.5, 0., 1.)).x + 1.) * 0.5 * SIZE[0] as f32;
    let (edge, covered) = (left as u32, 1. - left.fract());
    let texels = (SIZE[0] * SIZE[1]) as usize;
    let mut report = String::new();
    for scale in [RenderScale::Performance, RenderScale::Quality] {
        let [width, height] = scale.internal_size(SIZE[0], SIZE[1]);
        let share = (width * height) as f32 / texels as f32;
        let upscaled = Some(Reconstruct {
            settings: Reconstruction {
                scale,
                ..Default::default()
            },
            read_history: true,
        });
        const FRAMES: usize = 64;
        let mut history = Frames::default();
        let frames = scene.frames(&mut gpu, &cache, &mut history, upscaled, &[camera; FRAMES])?;
        history.retire(&mut gpu)?;
        let mut means = Vec::new();
        for (index, (hdr, read)) in frames.iter().enumerate() {
            let (colour, reset) = read.as_ref().context("history was not read")?;
            ensure!(
                hdr.len() == texels * 4 && colour.len() == texels * 4 && *reset == (index == 0),
                "{scale:?} frame {index}: {} and {} channels, reset {reset}",
                hdr.len(),
                colour.len()
            );
            for (x, y) in [(64, 48), (10, 10), (100, 80)] {
                let (value, flat) = (hdr[at(x, y)], plain[0].0[at(x, y)]);
                ensure!(
                    (value - flat).abs() <= 1e-3 * flat,
                    "{scale:?} frame {index}: flat pixel ({x}, {y}) is {value}, not {flat}"
                );
            }
            means.push(weights(colour)[1]);
        }
        let growth = (means[FRAMES - 1] - means[FRAMES / 2 - 1]) / (FRAMES / 2) as f32;
        ensure!(
            (growth / share - 1.).abs() < 0.1,
            "{scale:?}: the mean weight grows {growth:.3} a frame where a frame samples {share:.3} of the target"
        );
        let last = &frames[FRAMES - 1].0;
        let held = |x: u32| (last[at(x, 48)] - floor) / (card - floor);
        ensure!(
            (held(edge) - covered).abs() < 0.1
                && held(edge - 2).abs() < 0.02
                && held(edge + 2) > 0.98,
            "{scale:?}: the edge pixel holds {:.3} of the card where it covers {covered:.3}; two pixels out {:.3} and {:.3}",
            held(edge),
            held(edge - 2),
            held(edge + 2)
        );
        report += &format!(
            "{scale:?}: weight growth {growth:.3} for {share:.3}, edge {:.3} for {covered:.3}; ",
            held(edge)
        );
    }
    // Post runs at the target's size on the resolved image.
    scene.post = Some(renderlab::PostSettings::default());
    let performance = Some(Reconstruct {
        settings: Reconstruction {
            scale: RenderScale::Performance,
            ..Default::default()
        },
        read_history: false,
    });
    let mut history = Frames::default();
    scene.frames(&mut gpu, &cache, &mut history, performance, &[camera; 2])?;
    history.retire(&mut gpu)?;
    eprintln!("{report}");
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// What the planner follows does not restart history. A camera that climbs under the
/// physical sky changes what the procedural sky's content is keyed on (the eye's
/// altitude) on every frame: the renderer rewrites the sky and relights the cache, and
/// its reconstruction keeps accumulating. A sun that turns by less than the turn angle
/// keeps its history too, held to `turn_angle / turn` samples. The port once hashed
/// both into its restart key, and restarted every pixel on every such frame.
#[test]
fn reconstruction_keeps_history_under_a_climbing_camera_and_a_slow_sun() -> Result<()> {
    let path = root("temporal-followed");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let mut scene = Card::new();
    scene.atmosphere = renderlab::Atmosphere {
        volumetric_fog: None,
        ..Default::default()
    };
    // Half a pixel a frame along world y, the atmosphere's up, from 2 m: above the
    // kilometre-thousandth the sky's altitude is clamped to, so every frame's differs.
    let step = 0.5 * 2. * 4. * (27.5f32.to_radians()).tan() / SIZE[1] as f32;
    let climb: Vec<Camera> = (0..8)
        .map(|i| Card::camera([0.03, 2. + step * i as f32]))
        .collect::<Result<_>>()?;
    let mut history = Frames::default();
    let climbed = scene.frames(&mut gpu, &cache, &mut history, reconstructed(), &climb)?;
    // A floor pixel away from the card and its shadow, which sit below the view.
    let floor = at(100, 20) + 3;
    for (index, (_, read)) in climbed.iter().enumerate() {
        let (colour, reset) = read.as_ref().context("history was not read")?;
        ensure!(
            *reset == (index == 0) && colour[floor] > 0.6 * (index + 1) as f32,
            "climb frame {index}: reset {reset}, the floor's weight is {}",
            colour[floor]
        );
    }
    let turn = 0.01f32;
    let settings = Reconstruction::default();
    ensure!(
        turn < settings.turn_angle,
        "the turn is not under the turn angle"
    );
    scene.lighting.lights = vec![Light::Directional {
        travel: Vec3::new(turn.sin(), 0., -turn.cos()),
        lux: [1000.; 3],
    }];
    let still = [*climb.last().context("no camera")?];
    let turned = scene.frames(&mut gpu, &cache, &mut history, reconstructed(), &still)?;
    history.retire(&mut gpu)?;
    let (colour, reset) = turned[0].1.as_ref().context("history was not read")?;
    let cap = settings.turn_angle / turn;
    let most = weights(colour)[2];
    ensure!(
        !*reset && colour[floor] > settings.start_window && most <= cap * 1.01,
        "after the slow turn: reset {reset}, the floor's weight is {}, the most {most}, the cap {cap}",
        colour[floor]
    );
    cache.retire(&mut gpu)?;
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
    /// An entry, its workgroup size and the inputs it adds to the common ones.
    type Stage = (Interactive, [u32; 3], &'static [(u32, u32)]);
    let stages: [Stage; 3] = [
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
            Kernel::from_program(&mut gpu, &cache, &program)?;
        }
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// The Interactive frame against the exact one, under a slanted sun with a wide disk.
/// - Without reconstruction it traces eight sun directions a pixel where the exact
///   frame traces 256: the two agree bit for bit wherever the exact frame is fully lit
///   or fully shadowed, and where they differ the Interactive pixel holds a whole
///   number of eighths of the sun's light. That also holds the fused trace's hits,
///   repaired near triangle edges, to the exact trace's.
/// - With reconstruction the planner's cold budget is 256 rays at rotation zero, so the
///   first frame is the exact frame in every pixel: the rotated kernels, the budget
///   words and the indirectly dispatched repair reproduce the trace and the gated
///   estimator they replace. Later frames, on 64 then 32 then 8 rotated rays, stay
///   within a tenth of the contrast of the exact sequence (0.05 measured) on a few
///   hundred penumbra pixels.
#[test]
fn interactive_frame_is_the_exact_frame_outside_the_penumbra_and_on_a_cold_start() -> Result<()> {
    let path = root("interactive-frame");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let mut scene = Card::new();
    scene.lighting.lights = vec![Light::Directional {
        travel: Vec3::new(-0.6, 0., -1.),
        lux: [1000.; 3],
    }];
    scene.shadows.disk_angle = 0.08;
    let camera = Card::camera([0.03, 0.02])?;
    let mut render =
        |primary: Primary, tr: Option<Reconstruct>, n: usize| -> Result<Vec<Vec<f32>>> {
            scene.primary = primary;
            let mut history = Frames::default();
            let out = scene.frames(&mut gpu, &cache, &mut history, tr, &vec![camera; n])?;
            history.retire(&mut gpu)?;
            Ok(out.into_iter().map(|frame| frame.0).collect())
        };
    let reds =
        |frame: &[f32]| -> Vec<f32> { frame.as_chunks::<4>().0.iter().map(|p| p[0]).collect() };
    let exact = reds(&render(Primary::Traced, None, 1)?[0]);
    let fast = reds(&render(Primary::TracedInteractive, None, 1)?[0]);
    // The floor lit and in the card's umbra: the shadow's contrast.
    let lit = exact[at(10, 10) / 4];
    let umbra = exact.iter().cloned().fold(f32::MAX, f32::min);
    let contrast = lit - umbra;
    // The sky is black, so the umbra is too.
    ensure!(
        contrast > 0.1 && umbra == 0.,
        "the fixture has no shadow: lit {lit}, umbra {umbra}"
    );
    let (mut differing, mut worst, mut off_grid) = (0, 0f32, 0f32);
    for (a, b) in exact.iter().zip(&fast) {
        if a == b {
            continue;
        }
        differing += 1;
        worst = worst.max((a - b).abs());
        // Only where the exact frame is partly shadowed.
        ensure!(
            *a > umbra + 1e-3 * contrast && *a < lit - 1e-3 * contrast,
            "a pixel outside the penumbra differs: {a} against {b}"
        );
        // Eight rays: the sun's light arrives in eighths.
        let eighths = (b - umbra) / contrast * 8.;
        off_grid = off_grid.max((eighths - eighths.round()).abs());
    }
    ensure!(
        (50..400).contains(&differing) && off_grid < 0.05 && worst < 0.5 * contrast,
        "{differing} pixels differ by up to {worst} of a contrast of {contrast}, {off_grid} of an \
         eighth off the eight-ray grid"
    );
    let exact_sequence = render(Primary::Traced, reconstructed(), 24)?;
    let fast_sequence = render(Primary::TracedInteractive, reconstructed(), 24)?;
    ensure!(
        exact_sequence[0] == fast_sequence[0],
        "the cold reconstructed frame is not the exact frame"
    );
    let mut later = (0, 0f32);
    for (a, b) in exact_sequence.iter().zip(&fast_sequence).skip(1) {
        let (a, b) = (reds(a), reds(b));
        let apart = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        let most = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0f32, f32::max);
        later = (later.0.max(apart), later.1.max(most));
    }
    eprintln!(
        "plain: {differing} pixels differ by up to {:.3} of the contrast, {off_grid:.4} of an \
         eighth off the grid; reconstructed: frame 0 identical, later frames up to {} pixels by \
         up to {:.4} of the contrast",
        worst / contrast,
        later.0,
        later.1 / contrast
    );
    ensure!(
        later.0 < 400 && later.1 < contrast / 10.,
        "reconstructed Interactive frames drift from the exact sequence: {later:?}"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
