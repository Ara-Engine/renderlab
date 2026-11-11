//! Shading programs and their parameters on the traced frame.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Value, Vec3, Vertex,
    backend_ara::forward::{
        ForwardObject, ForwardScene, Frames, ObjectProgram, Primary, ScenePrograms, SunShadows,
        render,
    },
};

const SIZE: [u32; 2] = [128, 96];
/// The game scene's own program: the standard shading with an emitted `glow`.
const GAME: &str = include_str!("../../../examples/game_scene.wgsl");
/// A program that ignores the scene's light: it emits its colour times its gain.
const FLAT: &str = "struct Parameters { colour:vec3<f32>, gain:f32 };
const DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.25,0.5,1.0),1.0);
struct Surface { n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface { return Surface(material_normal(input)); }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> { return expose(params.colour*params.gain); }
";

/// A program whose surface sends on only the red of the light that reaches it, and
/// shows nothing to the camera itself.
const RED: &str = "struct Surface { n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface { return Surface(material_normal(input)); }
fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return vec3<f32>(max(dot(s.n,wi),0.0)/PI,0.0,0.0);
}
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> { return vec3<f32>(0.0); }
";

/// The Vulkan side of each program, from any host: the whole-region entry and the
/// list entry of the test's program and of the game scene's two emit SPIR-V that
/// validates, with every resource-array access marked non-uniform and exact arithmetic.
/// So does the cache's cell entry for one program among several, for the two programs
/// that reflect.
#[test]
fn programs_emit_valid_spirv_at_the_region_and_list_entries() -> Result<()> {
    use renderlab::backend_ara::primary_shade::{
        hardware_soft_list_program, hardware_soft_program,
    };
    let toon = include_str!("../../../examples/game_toon.wgsl");
    for (name, source) in [("flat", FLAT), ("game", GAME), ("toon", toon)] {
        forward_cases::validate_spirv(&hardware_soft_program(source)?, &format!("{name}-region"))?;
        forward_cases::validate_spirv(
            &hardware_soft_list_program(source)?,
            &format!("{name}-list"),
        )?;
    }
    for (slot, (name, source)) in [("game", GAME), ("toon", toon), ("red", RED)]
        .into_iter()
        .enumerate()
    {
        forward_cases::validate_spirv(
            &renderlab::backend_ara::gi_cache::mixed_shade_program(source, 64, slot as u32)?,
            &format!("{name}-cells"),
        )?;
    }
    Ok(())
}

fn quad(half: f32, z: f32) -> [Vertex; 4] {
    [[-half, -half], [half, -half], [half, half], [-half, half]].map(|[x, y]| Vertex {
        position: [x, y, z],
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    })
}

/// A card over a floor, seen from above. Each object names a program and its
/// parameter values, as an engine material does.
///
/// - A program of the test's own, which emits `colour * gain` and reads no light, puts
///   exactly that times the exposure in every pixel of each object: the floor at the
///   program's defaults, the card at its own values. So the program composes at the
///   shading stage and each draw reads its own parameter block.
/// - The game scene's program at its defaults shades as the standard program does.
/// - With `glow` set on the card alone, the card's pixels gain the exposed glow and the
///   floor's do not change by a bit.
/// - Different programs in one frame each shade their own object's pixels and no
///   others, down to the pixels of a tile both objects share: with the flat program on
///   one object, that object's pixels are the flat value exactly and the other's are
///   bit for bit those of the frame where its program shaded alone. Checked with the
///   flat program second, first (where it also holds the frame's first slot), and
///   beside the standard program, which declares no parameters.
/// - Refused, each with its reason: a parameter the program does not declare, and any
///   program on the mesh pass.
#[test]
fn a_program_shades_each_object_with_its_own_parameters() -> Result<()> {
    let path = root("programs");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let (floor, card) = (quad(8., 0.), quad(0.5, 1.));
    const STOPS: f32 = -2.;
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.2, 0.1, -1.).normalize(),
            lux: [4.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: STOPS,
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
    let mut frame = |programs: Option<[Option<ObjectProgram<'_>>; 2]>,
                     primary: Primary|
     -> Result<Vec<[f32; 3]>> {
        let objects = [(&floor, 0.3), (&card, 0.8)].map(|(vertices, base)| ForwardObject {
            vertices,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material: MaterialDescriptor {
                base_color: [base, base, base, 1.],
                metallic: 0.,
                roughness: 0.6,
                double_sided: true,
                ..Default::default()
            },
        });
        let images = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                meshes: None,
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
                transparent: &[],
                programs: programs.as_ref().map(|objects| ScenePrograms {
                    objects,
                    transparent: &[],
                }),
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
    // Which object a pixel's centre shows: the card inside its projected square, with
    // the pixels its outline crosses left out.
    let matrix = camera.matrix(SIZE[0], SIZE[1]);
    let corner = |x: f32, y: f32| {
        let ndc = matrix.project_point3(Vec3::new(x, y, 1.));
        [
            (ndc.x + 1.) * 0.5 * SIZE[0] as f32,
            (1. - ndc.y) * 0.5 * SIZE[1] as f32,
        ]
    };
    let (lo, hi) = (corner(-0.5, 0.5), corner(0.5, -0.5));
    let on_card = |i: usize| -> Option<bool> {
        let centre = [
            (i as u32 % SIZE[0]) as f32 + 0.5,
            (i as u32 / SIZE[0]) as f32 + 0.5,
        ];
        let signed = (0..2)
            .map(|a| (lo[a] - centre[a]).max(centre[a] - hi[a]))
            .fold(f32::MIN, f32::max);
        (signed.abs() > 1.).then_some(signed < 0.)
    };
    let exposure = 2f32.powf(STOPS);
    // The flat program: the floor at its defaults, the card at its own values.
    let own = [
        ("colour", Value::Vec3([1., 0., 0.5])),
        ("gain", Value::F32(2.)),
    ];
    let flat = frame(
        Some([
            Some(ObjectProgram {
                source: FLAT,
                parameters: &[],
            }),
            Some(ObjectProgram {
                source: FLAT,
                parameters: &own,
            }),
        ]),
        Primary::Traced,
    )?;
    let (mut floors, mut cards) = (0, 0);
    for (i, pixel) in flat.iter().enumerate() {
        let Some(card) = on_card(i) else { continue };
        let expected = if card {
            [1., 0., 0.5].map(|c: f32| c * 2. * exposure)
        } else {
            [0.25, 0.5, 1.].map(|c: f32| c * exposure)
        };
        floors += usize::from(!card);
        cards += usize::from(card);
        ensure!(
            *pixel == expected,
            "flat program: pixel {i} on the {} is {pixel:?}, expected {expected:?}",
            if card { "card" } else { "floor" }
        );
    }
    ensure!(
        floors > 9000 && cards > 700,
        "the fixture lost its regions: {floors} floor and {cards} card pixels"
    );
    // The game program at its defaults against the standard program.
    let game = |parameters| {
        Some(ObjectProgram {
            source: GAME,
            parameters,
        })
    };
    let standard = frame(None, Primary::Traced)?;
    let plain = frame(Some([game(&[]), game(&[])]), Primary::Traced)?;
    let apart = standard
        .iter()
        .zip(&plain)
        .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs() / a[c].max(1e-3)))
        .fold(0f32, f32::max);
    ensure!(
        apart < 1e-3,
        "the game program at its defaults is {apart} from the standard program"
    );
    // Glow on the card alone.
    // Small against the card's own light, so the sum keeps half-float precision.
    let glow = [0.5f32, 1., 0.25];
    let lit = [("glow", Value::Vec3(glow))];
    let glowing = frame(Some([game(&[]), game(&lit)]), Primary::Traced)?;
    let mut worst = 0f32;
    for (i, (pixel, before)) in glowing.iter().zip(&plain).enumerate() {
        match on_card(i) {
            Some(true) => {
                for c in 0..3 {
                    let gained = pixel[c] - before[c];
                    worst = worst.max((gained - glow[c] * exposure).abs());
                    ensure!(
                        (gained - glow[c] * exposure).abs() < 1e-3,
                        "glow: card pixel {i} gained {gained} in channel {c}, expected {}",
                        glow[c] * exposure
                    );
                }
            }
            Some(false) => ensure!(
                pixel == before,
                "glow: floor pixel {i} changed from {before:?} to {pixel:?}"
            ),
            None => {}
        }
    }
    // Several programs in one frame.
    let flat_only = Some(ObjectProgram {
        source: FLAT,
        parameters: &[],
    });
    let flat_own = Some(ObjectProgram {
        source: FLAT,
        parameters: &own,
    });
    let mut mixed = 0;
    for (name, programs, alone) in [
        ("game then flat", [game(&[]), flat_own], &plain),
        ("flat then game", [flat_only, game(&lit)], &glowing),
        ("standard then flat", [None, flat_own], &standard),
    ] {
        let flat_on_card = programs[1].is_some_and(|p| p.source == FLAT);
        let frame = frame(Some(programs), Primary::Traced)?;
        for (i, pixel) in frame.iter().enumerate() {
            let Some(card) = on_card(i) else { continue };
            let expected = match (card == flat_on_card, card) {
                (false, _) => alone[i],
                (true, true) => [1., 0., 0.5].map(|c: f32| c * 2. * exposure),
                (true, false) => [0.25, 0.5, 1.].map(|c: f32| c * exposure),
            };
            ensure!(
                *pixel == expected,
                "{name}: pixel {i} on the {} is {pixel:?}, expected {expected:?}",
                if card { "card" } else { "floor" }
            );
            mixed += 1;
        }
    }
    // Refusals.
    let unknown = [("sparkle", Value::F32(1.))];
    for (name, programs, primary) in [
        (
            "an undeclared parameter",
            [game(&unknown), game(&[])],
            Primary::Traced,
        ),
        (
            "a program on the mesh pass",
            [flat_only, flat_only],
            Primary::Raster,
        ),
    ] {
        ensure!(
            frame(Some(programs), primary).is_err(),
            "the frame accepted {name}"
        );
    }
    eprintln!(
        "programs: flat exact on {floors} floor and {cards} card pixels; game at defaults {apart:.2e} from standard; glow within {worst:.4}; {mixed} pixels exact in frames of two programs"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Two sunlit floors side by side, a black wall between them, and on each an upright
/// card the sun does not reach, under a black sky: a card shows only the light its own
/// floor sends up. One floor is the standard program's white, the other the test's
/// program that sends on red alone; the cards and the wall are standard.
///
/// After eight frames of one sequence with the radiance cache on, the card over the
/// white floor is lit and grey, and the card over the red floor is lit and red with
/// no green or blue. So each floor's cells were shaded by that floor's program. It
/// holds with either floor drawn first, which is what tells it from shading every
/// cell with the frame's first program: that turns one card or the other the wrong
/// colour, whichever order is drawn.
///
/// It also holds when the floors change places in the draw order part way through a
/// sequence, which gives each program the other's slot: four frames later, and on the
/// very next frame, where a cache that had started again would show no bounce light.
#[test]
fn two_programs_in_one_frame_each_light_their_own_cells() -> Result<()> {
    let path = root("program-cells");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let rectangle = |corners: [[f32; 3]; 4], normal: [f32; 3]| {
        corners.map(|position| Vertex {
            position,
            normal,
            uv: [0.; 2],
            color: [1.; 4],
        })
    };
    let floor = |x: f32| {
        rectangle(
            [
                [x - 2., -2., 0.],
                [x + 2., -2., 0.],
                [x + 2., 2., 0.],
                [x - 2., 2., 0.],
            ],
            [0., 0., 1.],
        )
    };
    let card = |x: f32| {
        rectangle(
            [
                [x - 1., 1., 0.],
                [x + 1., 1., 0.],
                [x + 1., 1., 1.5],
                [x - 1., 1., 1.5],
            ],
            [0., -1., 0.],
        )
    };
    let wall = rectangle(
        [[0., -3., 0.], [0., 3., 0.], [0., 3., 3.], [0., -3., 3.]],
        [1., 0., 0.],
    );
    const WHITE: f32 = -2.2;
    const REDS: f32 = 2.2;
    let (floors, cards) = ([floor(WHITE), floor(REDS)], [card(WHITE), card(REDS)]);
    // The sun leans away from the cards' faces, so no direct light reaches them.
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
    // The mean of the 8 by 8 pixels about the middle of the card at `x`.
    let patch = |hdr: &[u8], x: f32| -> [f32; 3] {
        let ndc = matrix.project_point3(Vec3::new(x, 1., 0.75));
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
    let red = Some(ObjectProgram {
        source: RED,
        parameters: &[],
    });
    // One sequence: a frame for each entry, with the red floor drawn first where it says.
    let mut sequence = |red_first: &[bool]| -> Result<Vec<u8>> {
        let mut frames = Frames::default();
        let mut last = Ok(Vec::new());
        for &red_first in red_first {
            let order: [usize; 2] = if red_first { [1, 0] } else { [0, 1] };
            let object = |vertices, base: f32| ForwardObject {
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
            };
            let objects = [
                object(&floors[order[0]][..], 0.8),
                object(&floors[order[1]][..], 0.8),
                object(&cards[0][..], 0.8),
                object(&cards[1][..], 0.8),
                object(&wall[..], 0.),
            ];
            let programs = [
                if red_first { red } else { None },
                if red_first { None } else { red },
                None,
                None,
                None,
            ];
            last = frames
                .render(
                    &mut gpu,
                    &cache,
                    &ForwardScene {
                        meshes: None,
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
                        transparent: &[],
                        programs: Some(ScenePrograms {
                            objects: &programs,
                            transparent: &[],
                        }),
                    },
                )
                .map(|images| images.hdr);
            if last.is_err() {
                break;
            }
        }
        frames.retire(&mut gpu)?;
        last
    };
    const F: bool = false;
    for (name, red_first) in [
        ("white floor first", &[F; 8][..]),
        ("red floor first", &[true; 8]),
        (
            "the frame after the floors change places",
            &[F, F, F, F, F, F, F, F, true],
        ),
        (
            "four frames after the floors change places",
            &[F, F, F, F, F, F, F, F, true, true, true, true],
        ),
    ] {
        let hdr = sequence(red_first)?;
        let (white, reds) = (patch(&hdr, WHITE), patch(&hdr, REDS));
        eprintln!("program cells, {name}: card over white {white:?}, card over red {reds:?}");
        ensure!(
            white[0] > 0.3 && white.iter().all(|c| (c - white[0]).abs() < 1e-3 * white[0]),
            "{name}: the card over the white floor is {white:?}, not lit grey"
        );
        ensure!(
            reds[0] > 0.3 && reds[1] < 1e-3 && reds[2] < 1e-3,
            "{name}: the card over the red floor is {reds:?}, not lit red alone"
        );
        // The red program sends on all the red it receives, the white floor 0.8 of it.
        let ratio = reds[0] / white[0];
        ensure!(
            (1.15..1.35).contains(&ratio),
            "{name}: the red floor sends {ratio} of the white floor's red, expected about 1.25"
        );
    }
    // A cache that starts again shows nothing on its first frame, so the frame after
    // the change kept its table.
    let first = patch(&sequence(&[F])?, WHITE);
    ensure!(
        first.iter().all(|c| *c < 1e-3),
        "a first frame already shows bounce light: {first:?}"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
