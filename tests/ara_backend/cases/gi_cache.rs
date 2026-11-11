//! The world radiance cache's sky-visibility kernels through the adapter.
use super::*;
use gi_cache_reference::Table;
use glam::DVec3;
use renderlab::{
    Camera, IndirectSettings, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::{
        forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows},
        gi_cache::{Entry, Indirect},
        kernel::Kernel,
    },
};

/// An entry, its workgroup size, and the inputs it adds to the three every stage takes.
type Stage = (Entry, [u32; 3], &'static [(u32, u32)]);

/// Every sky-visibility entry of the hardware-ray module reflects the interface its
/// stage needs, emits SPIR-V that validates, and becomes a native pipeline on a
/// ray-query device. None of them is replayed against a reference here.
#[test]
fn sky_visibility_kernels_reflect_validate_and_compile() -> Result<()> {
    let path = root("gi-cache-kernels");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // Table, work words and frame record everywhere; then what each stage adds.
    let expected: [Stage; 9] = [
        (
            Entry::Probe,
            [8, 8, 1],
            &[(0, 7), (0, 21), (1, 0), (1, 1), (1, 2), (1, 3), (1, 5)],
        ),
        (Entry::Claim, [64, 1, 1], &[]),
        (Entry::Resolve, [64, 1, 1], &[(0, 33), (0, 37)]),
        (Entry::ResolveLast, [64, 1, 1], &[(0, 33), (0, 37)]),
        (Entry::Scan, [64, 1, 1], &[(3, 0), (3, 1)]),
        (
            Entry::Setup,
            [64, 1, 1],
            &[
                (0, 34),
                (0, 36),
                (0, 37),
                (1, 0),
                (1, 1),
                (1, 2),
                (1, 3),
                (1, 5),
            ],
        ),
        (Entry::Rays, [64, 1, 1], &[(0, 34)]),
        (
            Entry::Finish,
            [64, 1, 1],
            &[(0, 37), (3, 0), (3, 6), (3, 7), (3, 9)],
        ),
        (
            Entry::Integrate,
            [64, 1, 1],
            &[(0, 37), (3, 0), (3, 6), (3, 7), (3, 9)],
        ),
    ];
    ensure!(
        expected.map(|e| e.0) == Entry::ALL,
        "entry list is out of encode order"
    );
    for (entry, threads, extra) in expected {
        let program = entry.program()?;
        let mut keys = vec![(0, 30), (0, 31), (0, 32)];
        keys.extend_from_slice(extra);
        keys.sort_unstable();
        let reflected: Vec<_> = program
            .inputs()
            .iter()
            .map(|i| (i.key.group, i.key.binding))
            .collect();
        ensure!(
            reflected == keys && program.threads()? == threads,
            "{entry:?} interface changed: {:?} in groups of {:?}",
            program.inputs(),
            program.threads()
        );
        program.msl()?;
        forward_cases::validate_spirv(&program, &format!("gi-{entry:?}"))?;
        Kernel::from_program(&mut gpu, &cache, &program)?.retire(&mut gpu)?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A 12 m floor in the xy plane with a 12 m wall standing on its y axis, grey and
/// rough, under a uniform unit sky and a sun too dim to matter. The wall hides half
/// the sky from the floor at its foot and less with distance.
/// One frame of a sequence: its HDR red channel per pixel, and the cache's table when
/// the cache ran and was read.
type YardFrame = (Vec<f32>, Option<Vec<u32>>);
struct Yard {
    floor: [Vertex; 4],
    wall: [Vertex; 4],
    lighting: Lighting,
    shadows: SunShadows,
}
const HALF: f64 = 6.;
const HEIGHT: f64 = 12.;
const SIZE: [u32; 2] = [128, 96];
impl Yard {
    fn new() -> Self {
        let quad = |corners: [[f32; 3]; 4], normal: [f32; 3]| {
            corners.map(|position| Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            })
        };
        Self {
            floor: quad(
                [[-6., -6., 0.], [6., -6., 0.], [6., 6., 0.], [-6., 6., 0.]],
                [0., 0., 1.],
            ),
            wall: quad(
                [[0., -6., 0.], [0., 6., 0.], [0., 6., 12.], [0., -6., 12.]],
                [1., 0., 0.],
            ),
            lighting: Lighting {
                lights: vec![Light::Directional {
                    travel: Vec3::new(0., 0., -1.),
                    lux: [1e-3; 3],
                }],
                environment: [1.; 3],
                exposure_stops: 0.,
            },
            shadows: SunShadows {
                split_ends: vec![16.],
                disk_angle: 0.02,
            },
        }
    }
    /// The camera `step` quarter-metres along x from its first position.
    fn camera(step: u32) -> Result<Camera> {
        let x = step as f32 * 0.25;
        Ok(Camera::look_at(
            Vec3::new(3. + x, 0., 5.),
            Vec3::new(1.5 + x, 0., 0.),
            Vec3::Y,
            55f32.to_radians(),
            0.1,
            100.,
        )?)
    }
    /// One traced frame per camera through one history: each frame's HDR red channel
    /// per pixel, and the cache's table when it ran.
    fn sequence(
        &self,
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        indirect: Option<Indirect>,
        cameras: &[Camera],
    ) -> Result<Vec<YardFrame>> {
        let material = || MaterialDescriptor {
            base_color: [0.5, 0.5, 0.5, 1.],
            metallic: 0.,
            roughness: 1.,
            double_sided: true,
            ..Default::default()
        };
        let objects = [&self.floor, &self.wall].map(|vertices| ForwardObject {
            vertices,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material: material(),
        });
        let mut history = Frames::default();
        let mut frames = Vec::new();
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
                    indirect,
                },
            )?;
            let red = images
                .hdr
                .as_chunks::<8>()
                .0
                .iter()
                .map(|p| f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32())
                .collect();
            frames.push((red, images.indirect_table));
        }
        history.retire(gpu)?;
        Ok(frames)
    }
    /// Whether a ray from `o` along `d` hits the floor or the wall; None when it passes
    /// within 2 mm of an edge, where the kernel's f32 ray may fall either side.
    fn blocked(o: DVec3, d: DVec3) -> Option<bool> {
        let mut hit = false;
        // Each rectangle: the plane's axis, then the two extents in its plane.
        for (axis, (u, v)) in [
            (2, ((0, -HALF, HALF), (1, -HALF, HALF))),
            (0, ((1, -HALF, HALF), (2, 0., HEIGHT))),
        ] {
            if d[axis] == 0. {
                continue;
            }
            let t = -o[axis] / d[axis];
            if t <= 0. {
                continue;
            }
            let p = o + d * t;
            let inside = |(i, lo, hi): (usize, f64, f64)| (p[i] - lo).min(hi - p[i]);
            let margin = inside(u).min(inside(v));
            if margin.abs() < 2e-3 {
                return None;
            }
            hit |= margin > 0.;
        }
        Some(hit)
    }
}
fn sky_visibility(read_table: bool) -> Option<Indirect> {
    Some(Indirect {
        settings: IndirectSettings {
            bounce: false,
            filter: 0,
            slots_log2: 14,
            ..Default::default()
        },
        read_table,
    })
}
/// Compares every settled cell's 64 direction bits with exact rays against the yard;
/// returns the settled cells and the rays compared.
fn replay_bits(words: &[u32]) -> Result<(usize, usize)> {
    let table = Table::new(words)?;
    let (mut cells, mut rays) = (0, 0);
    for cell in table.cells().iter().filter(|c| c.settled) {
        cells += 1;
        let origin = gi_cache_reference::origin(cell, table.min_edge());
        for k in 0..gi_cache_reference::DIRECTIONS {
            let d = gi_cache_reference::direction(cell, table.rotates(), k).as_dvec3();
            let Some(blocked) = Yard::blocked(origin, d) else {
                continue;
            };
            rays += 1;
            ensure!(
                (cell.bits >> k & 1 == 0) == blocked,
                "slot {} at {:?} normal {:?}: direction {k} is {} in the table",
                cell.slot,
                cell.point,
                cell.normal,
                if blocked { "open" } else { "blocked" }
            );
        }
    }
    Ok((cells, rays))
}

/// One frame of the yard fills the table. Every cell traced from its own query point
/// holds exactly the visibility bits its 64 rays have against the scene, on the floor
/// and on the wall, and every valid cell's stored SH is the sum over its open
/// directions that `gi_finish` forms, to f16 precision.
#[test]
fn sky_visibility_cells_hold_exact_ray_bits_and_their_sh() -> Result<()> {
    let path = root("gi-cache-cells");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let yard = Yard::new();
    let frames = yard.sequence(&mut gpu, &cache, sky_visibility(true), &[Yard::camera(0)?])?;
    let words = frames[0].1.as_ref().context("table was not read")?;
    let (settled, rays) = replay_bits(words)?;
    let table = Table::new(words)?;
    let cells = table.cells();
    let on = |axis: usize| {
        cells
            .iter()
            .filter(|c| c.settled && c.normal[axis] > 0.9)
            .count()
    };
    // Half-blocked cells exist only at the wall's foot; open floor cells see all 64.
    let half = cells
        .iter()
        .filter(|c| c.settled && (24..=40).contains(&c.bits.count_ones()))
        .count();
    let open = cells
        .iter()
        .filter(|c| c.settled && c.bits == u64::MAX)
        .count();
    ensure!(
        settled >= 500 && rays >= settled * 60 && on(2) >= 100 && on(0) >= 100 && half >= 50,
        "replay covered too little: {settled} cells, {rays} rays, floor {}, wall {}, \
         half-blocked {half}, open {open}",
        on(2),
        on(0)
    );
    let mut worst = (0f64, 0f64);
    for cell in cells.iter().filter(|c| c.valid && c.settled) {
        let sh = gi_cache_reference::sky_sh(cell, table.rotates(), cell.bits);
        let scale = sh.iter().fold(1e-30f64, |m, v| m.max(v.abs()));
        worst.0 = worst.0.max((f64::from(cell.scale) / scale - 1.).abs());
        for (j, stored) in cell.sh.iter().enumerate() {
            worst.1 = worst.1.max((f64::from(*stored) - sh[j / 3] / scale).abs());
        }
    }
    eprintln!(
        "{settled} settled cells, {rays} rays, half-blocked {half}, open {open}; \
         SH scale error {:.2e}, halves {:.2e}",
        worst.0, worst.1
    );
    // f32 sums of at most 64 terms, then halves of values in [-1, 1].
    ensure!(
        worst.0 < 1e-5 && worst.1 < 1e-3,
        "stored SH departs from its bits: scale {:.3e}, halves {:.3e}",
        worst.0,
        worst.1
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// With the cache on, a floor pixel keeps the cosine-weighted share of the sky the
/// wall leaves it: one minus the wall's form factor from that point, in closed form.
#[test]
fn sky_visibility_darkens_the_floor_by_the_walls_share_of_the_sky() -> Result<()> {
    let path = root("gi-cache-image");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let yard = Yard::new();
    let camera = Yard::camera(0)?;
    let off = yard.sequence(&mut gpu, &cache, None, &[camera])?;
    let on = yard.sequence(&mut gpu, &cache, sky_visibility(false), &[camera])?;
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    // Differential floor element to a perpendicular rectangle that starts at the foot
    // of its perpendicular: width a along the wall, height h, at distance d.
    let corner = |a: f64, d: f64| {
        let r = (d * d + HEIGHT * HEIGHT).sqrt();
        ((a / d).atan() - d / r * (a / r).atan()) / std::f64::consts::TAU
    };
    let (mut worst, mut samples, mut least) = (0f64, 0, 1f64);
    for y in (4..SIZE[1]).step_by(8) {
        for x in (4..SIZE[0]).step_by(8) {
            let ndc = |depth: f64| {
                inverse.project_point3(DVec3::new(
                    (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2. - 1.,
                    1. - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.,
                    depth,
                ))
            };
            let (a, b) = (ndc(0.25), ndc(0.75));
            let p = a + (b - a) * (-a.z / (b.z - a.z));
            // Floor in front of the wall, clear of the floor's own edges.
            if !(p.x > 0.05 && p.x < 5.5 && p.y.abs() < 5.5) {
                continue;
            }
            let at = (y * SIZE[0] + x) as usize;
            let measured = f64::from(on[0].0[at]) / f64::from(off[0].0[at]);
            let expected = 1. - corner(HALF - p.y, p.x) - corner(HALF + p.y, p.x);
            worst = worst.max((measured - expected).abs());
            least = least.min(expected);
            samples += 1;
        }
    }
    eprintln!("floor samples {samples}, least expected share {least:.3}, worst error {worst:.4}");
    // 64 directions a cell, 4 px cells and an L2 fit of a hard edge: a few percent.
    ensure!(
        samples >= 60 && least < 0.6 && worst < 0.06,
        "{samples} floor samples down to a share of {least:.3}: worst error {worst:.4}"
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Sequences. Two runs of the same still sequence give the same table words, frame by
/// frame. A still view changes only the frame stamps until the planner's idle
/// threshold, after which no pass runs and the table stops changing. A moving camera
/// claims new cells every frame, and every cell still holds exact ray bits.
#[test]
fn cache_sequences_repeat_idle_when_still_and_follow_the_camera() -> Result<()> {
    let path = root("gi-cache-sequence");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let yard = Yard::new();
    // `IDLE_AFTER` identical frames, then idle ones.
    const RUNNING: usize = 9;
    let still = vec![Yard::camera(0)?; RUNNING + 2];
    let tables = |frames: Vec<YardFrame>| -> Result<Vec<Vec<u32>>> {
        frames
            .into_iter()
            .map(|f| f.1.context("table was not read"))
            .collect()
    };
    let first = tables(yard.sequence(&mut gpu, &cache, sky_visibility(true), &still)?)?;
    let again = tables(yard.sequence(&mut gpu, &cache, sky_visibility(true), &still)?)?;
    ensure!(first == again, "the same sequence gave different tables");
    let slots = first[0][1] as usize + 1;
    let cold_at = 64 + slots * 24;
    let stride = (first[0].len() - cold_at) / slots;
    for (index, table) in first.iter().enumerate().skip(1) {
        let stamp = index.min(RUNNING - 1) as u32;
        ensure!(
            table[3] == stamp,
            "frame {index} carries stamp {}",
            table[3]
        );
        let previous = &first[index - 1];
        let changed: Vec<usize> = (0..table.len())
            .filter(|i| table[*i] != previous[*i])
            .collect();
        if index >= RUNNING {
            ensure!(changed.is_empty(), "idle frame {index} changed the table");
            continue;
        }
        // The header's stamp, and seen stamps (cold word 2) of the cells in view.
        ensure!(
            changed
                .iter()
                .all(|i| *i == 3 || (*i >= cold_at && (*i - cold_at) % stride == 2)),
            "still frame {index} changed more than stamps"
        );
        ensure!(
            (index == RUNNING - 1) == (changed == [3]),
            "frame {index} changed {} words",
            changed.len()
        );
    }
    let moving: Vec<Camera> = (0..6).map(Yard::camera).collect::<Result<_>>()?;
    let moved = tables(yard.sequence(&mut gpu, &cache, sky_visibility(true), &moving)?)?;
    let mut occupied = Vec::new();
    for table in &moved {
        replay_bits(table)?;
        occupied.push(Table::new(table)?.cells().len());
    }
    ensure!(
        occupied.is_sorted_by(|a, b| a < b),
        "a moving camera did not claim cells every frame: {occupied:?}"
    );
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
