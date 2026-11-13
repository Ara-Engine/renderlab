//! The material pass on the port (RENDERLAB_MATERIAL_PASS): cold compile times of the standard
//! program's single shading kernel against the material pass's record writer and the
//! record-reading shading, tiles and list. Measurement, ignored by default.
use super::*;
use renderlab::backend_ara::primary_shade;

#[test]
#[ignore]
fn material_pass_compile_seconds() -> Result<()> {
    let path = root("material-pass");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let standard = renderlab::STANDARD_PROGRAM;
    for list in [false, true] {
        let full = if list {
            primary_shade::hardware_soft_list_program(standard)?
        } else {
            primary_shade::hardware_soft_program(standard)?
        };
        let writer = primary_shade::hardware_soft_material_writer(standard, list)?
            .context("the standard program declares the record mapping")?;
        let reader = primary_shade::hardware_soft_material_reader(standard, list)?
            .context("the standard program declares the record mapping")?;
        for (name, program) in [
            ("single kernel", full),
            ("material writer", writer),
            ("record reader", reader),
        ] {
            let started = std::time::Instant::now();
            renderlab::backend_ara::kernel::Kernel::from_program(&mut gpu, &cache, &program)?;
            println!(
                "material pass compile: list={list} {name}: {:.2} s",
                started.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}

/// The receiver case's frame (`the_record_rebuilds_the_receiver`, and the producers'
/// comparison): a floor, a wall folded along a crease the middle column's rays land on (toon),
/// a corrugated sheet, a steep valley and a far wall, at `RECEIVER_SIZE`.
fn receiver_frame(name: &str) -> Result<renderlab::backend_ara::forward::ForwardImages> {
    use renderlab::{
        Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
        backend_ara::forward::{
            ForwardObject, ForwardScene, ObjectProgram, Primary, ScenePrograms, SunShadows, render,
        },
    };
    const SIZE: [u32; 2] = RECEIVER_SIZE;
    // Odd width: the middle column's camera ray lies in the plane x = 0, the crease's.
    let path = root(name);
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let quad = |corners: [[f32; 3]; 4], normals: [[f32; 3]; 4]| -> Vec<Vertex> {
        (0..4)
            .map(|i| Vertex {
                position: corners[i],
                normal: normals[i],
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect()
    };
    let up = [0., 0., 1.];
    let floor = quad(
        [[-8., -8., 0.], [8., -8., 0.], [8., 8., 0.], [-8., 8., 0.]],
        [up; 4],
    );
    let toward = |x: f32, z: f32| Vec3::new(x, -1., z).normalize().to_array();
    // A wall folded along x = 0 (a concave crease, where the edge repair acts; both faces
    // share the edge's vertices), with tilted vertex normals.
    let crease: Vec<Vertex> = [
        ([0., 2.6, 0.], toward(0., -0.2)),
        ([0., 2.6, 2.], toward(0., 0.5)),
        ([-1.5, 2., 0.], toward(0.6, -0.2)),
        ([-1.5, 2., 2.], toward(0.5, 0.4)),
        ([1.5, 2., 0.], toward(-0.6, -0.2)),
        ([1.5, 2., 2.], toward(-0.5, 0.4)),
    ]
    .map(|(position, normal)| Vertex {
        position,
        normal,
        uv: [0.; 2],
        color: [1.; 4],
    })
    .to_vec();
    let far = quad(
        [
            [-40., 60., 0.],
            [40., 60., 0.],
            [40., 60., 30.],
            [-40., 60., 30.],
        ],
        [[0., -1., 0.]; 4],
    );
    let quad_indices: &[u32] = &[0, 1, 2, 0, 2, 3];
    let crease_indices: &[u32] = &[2, 0, 1, 2, 1, 3, 0, 4, 5, 0, 5, 1];
    // A corrugated sheet in front of the crease, ridges across the view, seen at a grazing
    // angle: each ridge's far facet overlaps its near one in projection, where traversal can
    // return the farther facet and the edge repair moves the point.
    let (mut sheet, mut sheet_indices) = (Vec::new(), Vec::<u32>::new());
    for i in 0..=40u32 {
        let y = -1.0 + 0.05 * i as f32;
        let z = if i % 2 == 0 { 0.0 } else { 0.04 };
        for x in [-2.0f32, 2.0] {
            sheet.push(Vertex {
                position: [x, y, z],
                normal: up,
                uv: [0.; 2],
                color: [1.; 4],
            });
        }
        if i > 0 {
            let (a, b, c, d) = (2 * i - 2, 2 * i - 1, 2 * i, 2 * i + 1);
            sheet_indices.extend([a, b, d, a, d, c]);
        }
    }
    // A steep, narrow valley along the middle column's plane (receiver_fold_tests' fold,
    // slope 64): its two facets share the bottom edge the column's rays land on.
    let extent = 1.0 / (1.0f32 + 64.0 * 64.0).sqrt();
    let valley: Vec<Vertex> = [
        [0., -1., 0.],
        [extent, -1., 64. * extent],
        [0., 1.5, 0.],
        [-extent, -1., 64. * extent],
    ]
    .map(|position| Vertex {
        position,
        normal: up,
        uv: [0.; 2],
        color: [1.; 4],
    })
    .to_vec();
    let objects = [
        (&floor, quad_indices),
        (&crease, crease_indices),
        (&far, quad_indices),
        (&sheet, sheet_indices.as_slice()),
        (&valley, quad_indices),
    ]
    .map(|(vertices, indices)| ForwardObject {
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        material: MaterialDescriptor {
            base_color: [0.7, 0.7, 0.7, 1.],
            roughness: 0.5,
            double_sided: true,
            ..Default::default()
        },
    });
    render(
        &mut gpu,
        &cache,
        &ForwardScene {
            meshes: None,
            materials: None,
            instances: None,
            objects: &objects,
            // Misses write black, so only hits carry errors.
            lighting: &Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(0.3, 0.4, -1.).normalize(),
                    lux: [1.; 3],
                }],
                environment: [0.; 3],
                exposure_stops: 0.,
            },
            camera: Camera::look_at(
                Vec3::new(0., -4., 1.6),
                Vec3::new(0., 1., 0.6),
                Vec3::Z,
                70f32.to_radians(),
                0.1,
                200.,
            )?,
            size: SIZE,
            atmosphere: renderlab::Atmosphere::off(),
            sun_shadows: Some(&SunShadows {
                split_ends: vec![32.],
                disk_angle: 0.02,
            }),
            primary: Primary::Traced,
            post: None,
            indirect: None,
            reconstruction: None,
            reflections: None,
            textures: None,
            local_shadows: None,
            transparent: &[],
            // The folded wall draws with the toon program, which has no record mapping: a
            // receiver-only record (model 255) beside the standard program's full one.
            programs: Some(ScenePrograms {
                objects: &[
                    None,
                    Some(ObjectProgram {
                        source: include_str!("../../../shaders/programs/toon.wgsl"),
                        parameters: &[],
                    }),
                    None,
                    None,
                    None,
                ],
                transparent: &[],
            }),
            sky_map: None,
            skins: None,
            casts: None,
            colour_lut: None,
            ids: None,
        },
    )
}
// Odd width: the middle column's camera ray lies in the plane x = 0, the crease's.
const RECEIVER_SIZE: [u32; 2] = [161, 121];

/// The record's receiver (words 1 and 2) against the sun path's receiver for a fresh camera
/// ray (its edge repair included), through `RENDERLAB_MATERIAL_DEBUG=receiver` (program.rs
/// `material_debug`): wherever the repair leaves the hit, the trace point, slot, `thin`, both
/// sides and the epsilon come back bit for bit and the geometric normal within its octahedral
/// 2 x 16 bits; wherever the repair moves the point, the record marks the hit near an edge
/// (the reader repairs it, as the sun path does). The interpolated vertex normal (against this
/// pixel's decode) is within 2 x 12 bits. The toon program (no record mapping) has model 255
/// and the same receiver; the standard program's pixels do not. A floor, a wall folded along a
/// crease the middle column's rays land on (toon), a corrugated sheet, a steep valley and a far wall whose epsilon is
/// above the 1e-5 floor. Runs with `RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_DEBUG=receiver`
/// (read once per process); without them it says so and passes.
#[test]
fn the_record_rebuilds_the_receiver() -> Result<()> {
    let set = |name: &str, value: &str| std::env::var(name).as_deref() == Ok(value);
    if !(set("RENDERLAB_MATERIAL_PASS", "1") && set("RENDERLAB_MATERIAL_DEBUG", "receiver")) {
        println!(
            "record receiver: RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_DEBUG=receiver not set, nothing checked"
        );
        return Ok(());
    }
    const SIZE: [u32; 2] = RECEIVER_SIZE;
    let images = receiver_frame("material-receiver")?;
    // Hits write minus (1e-6 plus) the vertex normal's angle in w; misses write alpha 0 or 1.
    let (mut worst, mut hits, mut edges, mut unmapped) = ([0f32; 4], 0, 0, 0);
    let (mut misses, mut stale) = (0, 0);
    for pixel in images.hdr.as_chunks::<8>().0 {
        let value = |c: usize| {
            f16::from_bits(u16::from_le_bytes([pixel[2 * c], pixel[2 * c + 1]])).to_f32()
        };
        // Misses write w 3 and x 0 where word 1 holds the miss sentinel.
        if value(3) == 3.0 {
            misses += 1;
            stale += usize::from(value(0) != 0.0);
            continue;
        }
        if value(3) >= 0.0 {
            continue;
        }
        hits += 1;
        // z is negative (minus 1e-6) at repaired edges, where the record's point is the
        // repair's rather than shading's barycentric one.
        edges += usize::from(value(2) < 0.0);
        // x is negative (minus 1e-6) on the toon program's pixels.
        unmapped += usize::from(value(0) < 0.0);
        for (c, worst) in worst.iter_mut().enumerate() {
            *worst = worst.max(if value(c) < 0.0 && c != 1 {
                -value(c) - 1e-6
            } else {
                value(c)
            });
        }
    }
    println!(
        "record receiver: {hits} hits ({unmapped} toon), {edges} at repaired edges; trace point {:e}, epsilon {:e}, geometric normal {:.2e} rad, vertex normal {:.2e} rad ({:.3} deg)",
        worst[0],
        worst[1],
        worst[2],
        worst[3],
        worst[3].to_degrees()
    );
    ensure!(
        hits > (SIZE[0] * SIZE[1] / 2) as usize,
        "too few hits ({hits})"
    );
    ensure!(edges > 0, "no pixel reached an edge the repair moves");
    // Two programs shade from lists, which hold hits only; any miss shaded here is checked.
    ensure!(stale == 0, "{stale} of {misses} misses lack the sentinel");
    ensure!(
        unmapped > 0 && unmapped < hits,
        "toon and standard pixels both drawn ({unmapped} of {hits})"
    );
    ensure!(
        worst[0] == 0.0,
        "the trace point, slot, thin or a side moved, a moved point lacks the near-edge mark, or a model is wrong ({})",
        worst[0]
    );
    ensure!(worst[1] == 0.0, "the epsilon moved ({})", worst[1]);
    ensure!(
        worst[2] < 2e-4,
        "the geometric normal moved {} rad",
        worst[2]
    );
    ensure!(worst[3] < 2e-3, "the vertex normal moved {} rad", worst[3]);
    Ok(())
}

/// The miss sentinel after a camera move: a forward frame sequence (one program, so
/// shading visits the misses of every covered tile) looks at a wall, then turns up past its
/// top edge. In the second frame every miss holds the sentinel in word 1: those shading
/// visits (the debug reader checks them), the pixels the wall covered a frame earlier
/// included, and those it never visits (fast-cleared tiles of sky, read back from word 1).
/// No hit carries it. Runs with
/// `RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_DEBUG=receiver`.
#[test]
fn misses_carry_the_sentinel_after_the_camera_moves() -> Result<()> {
    use renderlab::{
        Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
        backend_ara::forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows},
    };
    let set = |name: &str, value: &str| std::env::var(name).as_deref() == Ok(value);
    if !(set("RENDERLAB_MATERIAL_PASS", "1") && set("RENDERLAB_MATERIAL_DEBUG", "receiver")) {
        println!(
            "miss sentinel: RENDERLAB_MATERIAL_PASS=1 RENDERLAB_MATERIAL_DEBUG=receiver not set, nothing checked"
        );
        return Ok(());
    }
    const SIZE: [u32; 2] = [128, 96];
    let path = root("material-sentinel");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // A 4 m wall square to the first camera, 5 m out.
    let wall =
        [[-2., 5., -2.], [2., 5., -2.], [2., 5., 2.], [-2., 5., 2.]].map(|position| Vertex {
            position,
            normal: [0., -1., 0.],
            uv: [0.; 2],
            color: [1.; 4],
        });
    let objects = [ForwardObject {
        vertices: &wall,
        indices: &[0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        material: MaterialDescriptor {
            double_sided: true,
            ..Default::default()
        },
    }];
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.3, 0.4, -1.).normalize(),
            lux: [1.; 3],
        }],
        environment: [1.; 3],
        exposure_stops: 0.,
    };
    let shadows = SunShadows {
        split_ends: vec![32.],
        disk_angle: 0.02,
    };
    let mut frames = Frames::default();
    // Each pixel as the debug reader left it, with word 1 read back: shading writes w < 0 on a
    // hit and w 3 on a miss (x 0 where word 1 holds the sentinel); a pixel it never visits
    // (fast-cleared tiles of sky, the region's exterior) keeps the clear value.
    #[derive(Clone, Copy, PartialEq)]
    enum Seen {
        Hit,
        Miss { sentinel_seen: bool },
        Unshaded,
    }
    let mut frame = |frames: &mut Frames, target: Vec3| -> Result<Vec<(Seen, bool)>> {
        let images = frames.render(
            &mut gpu,
            &cache,
            &ForwardScene {
                meshes: None,
                materials: None,
                instances: None,
                objects: &objects,
                lighting: &lighting,
                camera: Camera::look_at(
                    Vec3::ZERO,
                    target,
                    Vec3::Z,
                    60f32.to_radians(),
                    0.1,
                    100.,
                )?,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&shadows),
                primary: Primary::Traced,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &[],
                programs: None,
                sky_map: None,
                skins: None,
                casts: None,
                colour_lut: None,
                ids: None,
            },
        )?;
        let [words, _] = images
            .material_receiver
            .context("words 1 and 2 read back")?;
        Ok(images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .zip(words.as_chunks::<16>().0)
            .map(|(p, w)| {
                let c = |k: usize| {
                    f16::from_bits(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])).to_f32()
                };
                let seen = if c(3) < 0.0 {
                    Seen::Hit
                } else if c(3) == 3.0 {
                    Seen::Miss {
                        sentinel_seen: c(0) == 0.0,
                    }
                } else {
                    Seen::Unshaded
                };
                (
                    seen,
                    u32::from_le_bytes([w[12], w[13], w[14], w[15]]) == u32::MAX,
                )
            })
            .collect())
    };
    let first = frame(&mut frames, Vec3::new(0., 5., 0.))?;
    let second = frame(&mut frames, Vec3::new(0., 5., 5.))?;
    let count = |f: &dyn Fn(&(Seen, bool)) -> bool| second.iter().filter(|p| f(p)).count();
    let uncovered = first
        .iter()
        .zip(&second)
        .filter(|(a, b)| a.0 == Seen::Hit && b.0 != Seen::Hit)
        .count();
    let misses = count(&|p| matches!(p.0, Seen::Miss { .. }));
    let seen_bad = count(&|p| {
        p.0 == Seen::Miss {
            sentinel_seen: false,
        }
    });
    let unshaded = count(&|p| p.0 == Seen::Unshaded);
    let unshaded_bad = count(&|p| p.0 == Seen::Unshaded && !p.1);
    let hits_marked = count(&|p| p.0 == Seen::Hit && p.1);
    println!(
        "miss sentinel: {uncovered} pixels went from the wall to the sky; {misses} shaded misses ({seen_bad} without the sentinel), {unshaded} never shaded ({unshaded_bad} without it), {hits_marked} hits carrying it"
    );
    ensure!(
        uncovered > 50,
        "the camera move uncovered too few pixels ({uncovered})"
    );
    ensure!(
        misses > 0 && seen_bad == 0,
        "{seen_bad} of {misses} shaded misses lack the sentinel"
    );
    ensure!(unshaded > 0, "no fast-cleared pixel in the frame");
    ensure!(
        unshaded_bad == 0,
        "{unshaded_bad} of {unshaded} fast-cleared pixels lack the sentinel"
    );
    ensure!(
        hits_marked == 0,
        "{hits_marked} hits carry the miss sentinel"
    );
    Ok(())
}

/// The two producers of the receiver words write the same bits for the same frame: the
/// material pass's writer and the trace tail (`RENDERLAB_MATERIAL_PROBE=tracetail`), packing
/// through the shared `rl_mr_receiver_words`. Words 1 (geometric normal, slot, `thin`, sides)
/// and 2 (trace point, epsilon, interpolated normal) are compared bit for bit on every pixel,
/// with three differences by design: the pixels the edge repair moves (the tail holds the
/// repaired point, the writer shading's, marked), the near-edge mark itself (the writer marks
/// every near-edge hit, the tail none), and word 1's material fields (y and z: the tail writes
/// none). The producer is read once per process, so the case renders the receiver frame in two
/// child runs of itself; it needs no switch of its own.
#[test]
fn the_writer_and_the_trace_tail_write_the_same_receiver_words() -> Result<()> {
    const NAME: &str =
        "material_pass_cases::the_writer_and_the_trace_tail_write_the_same_receiver_words";
    let pixels = (RECEIVER_SIZE[0] * RECEIVER_SIZE[1]) as usize;
    // In a child run: write words 1 and 2 and the moved-pixel mask, then stop.
    if let Ok(dump) = std::env::var("RL_RECEIVER_DUMP") {
        let images = receiver_frame("material-producers")?;
        let [word1, word2] = images
            .material_receiver
            .context("words 1 and 2 read back")?;
        // The receiver check writes z < 0 on hits whose point the repair moved (w < 0: a hit).
        let moved: Vec<u8> = images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                let c = |k: usize| {
                    f16::from_bits(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])).to_f32()
                };
                u8::from(c(3) < 0.0 && c(2) < 0.0)
            })
            .collect();
        std::fs::write(dump, [word1, word2, moved].concat())?;
        return Ok(());
    }
    let dir = root("material-producers");
    std::fs::create_dir_all(&dir)?;
    let run = |tail: bool| -> Result<Vec<u8>> {
        let dump = dir.join(if tail { "tail.bin" } else { "writer.bin" });
        let status = std::process::Command::new(std::env::current_exe()?)
            .args([NAME, "--exact", "--test-threads=1"])
            .env("RENDERLAB_MATERIAL_PASS", "1")
            .env("RENDERLAB_MATERIAL_DEBUG", "receiver")
            .env(
                "RENDERLAB_MATERIAL_PROBE",
                if tail { "tracetail" } else { "" },
            )
            .env("RL_RECEIVER_DUMP", &dump)
            .status()?;
        ensure!(
            status.success(),
            "the {} run failed",
            if tail { "trace-tail" } else { "writer" }
        );
        let bytes = std::fs::read(&dump)?;
        ensure!(bytes.len() == pixels * 33, "dump size {}", bytes.len());
        Ok(bytes)
    };
    let (writer, tail) = (run(false)?, run(true)?);
    let word = |bytes: &[u8], base: usize, i: usize| -> [u32; 4] {
        let at = base + i * 16;
        std::array::from_fn(|k| {
            u32::from_le_bytes(bytes[at + 4 * k..at + 4 * k + 4].try_into().unwrap())
        })
    };
    let (mut hits, mut misses, mut moved, mut differ) = (0, 0, 0, 0);
    for i in 0..pixels {
        let (w1, t1) = (word(&writer, 0, i), word(&tail, 0, i));
        let (w2, t2) = (word(&writer, pixels * 16, i), word(&tail, pixels * 16, i));
        if writer[pixels * 32 + i] != 0 || tail[pixels * 32 + i] != 0 {
            moved += 1;
            continue;
        }
        if w1[3] == u32::MAX {
            misses += 1;
        } else {
            hits += 1;
        }
        let mark = !(1u32 << 28);
        if w1[0] != t1[0] || w1[3] & mark != t1[3] & mark || w2 != t2 {
            differ += 1;
        }
    }
    println!(
        "receiver producers: {hits} hits and {misses} misses compared, {moved} moved by the repair skipped, {differ} differ"
    );
    ensure!(
        hits > 10000 && misses > 0,
        "too little compared ({hits} hits, {misses} misses)"
    );
    ensure!(
        differ == 0,
        "{differ} pixels' receiver words differ between the producers"
    );
    Ok(())
}
