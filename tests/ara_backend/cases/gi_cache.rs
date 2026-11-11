//! The world radiance cache through the adapter, and its sky visibility in frames.
use super::*;
use gi_cache_reference::Table;
use glam::DVec3;
use renderlab::{
    Camera, IndirectSettings, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::{
        forward::{ForwardObject, ForwardScene, Frames, Primary, SunShadows},
        gi_cache::{Entry, Indirect, shade_program},
        kernel::Kernel,
    },
};

/// An entry, its workgroup size, and the inputs it adds to the three every stage takes.
type Stage = (Entry, [u32; 3], &'static [(u32, u32)]);

/// Every entry of the hardware-ray module, and cell shading composed with the standard
/// program, reflects the interface its stage needs, emits SPIR-V that validates, and
/// becomes a native pipeline on a ray-query device. Nothing is replayed here.
#[test]
fn cache_kernels_reflect_validate_and_compile() -> Result<()> {
    let path = root("gi-cache-kernels");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // Table, work words and frame record everywhere; then what each stage adds.
    let expected: [Stage; 12] = [
        (
            Entry::Probe,
            [8, 8, 1],
            &[(0, 7), (0, 21), (1, 0), (1, 1), (1, 2), (1, 3), (1, 5)],
        ),
        (
            Entry::Gather,
            [64, 1, 1],
            &[
                (0, 33),
                (0, 34),
                (0, 37),
                (1, 0),
                (1, 1),
                (1, 2),
                (1, 3),
                (1, 5),
            ],
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
            &[(0, 37), (3, 0), (3, 6), (3, 7), (3, 9), (3, 11)],
        ),
        (
            Entry::Integrate,
            [64, 1, 1],
            &[(0, 37), (3, 0), (3, 6), (3, 7), (3, 9), (3, 11)],
        ),
        (
            Entry::Filter,
            [64, 1, 1],
            &[(0, 37), (3, 0), (3, 6), (3, 11)],
        ),
        (Entry::Light, [64, 1, 1], &[(0, 34), (3, 0), (3, 1)]),
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
        Kernel::from_program(&mut gpu, &cache, &program)?;
    }
    // Cell shading: the work words read-only, the outgoing radiance and the draw table,
    // then the traced shading interface (geometry, materials, lighting and the table).
    let shade = shade_program(renderlab::STANDARD_PROGRAM, 64)?;
    let reflected: Vec<_> = shade
        .inputs()
        .iter()
        .map(|i| (i.key.group, i.key.binding))
        .collect();
    let groups = |group: u32| reflected.iter().filter(|k| k.0 == group).count();
    ensure!(
        reflected[..3] == [(0, 31), (0, 33), (0, 36)]
            && [groups(0), groups(1), groups(2), groups(3)] == [3, 6, 7, 7]
            && reflected.contains(&(3, 20))
            && shade.threads()? == [64, 1, 1],
        "cell shading interface changed: {reflected:?}"
    );
    shade.msl()?;
    forward_cases::validate_spirv(&shade, "gi-ShadeCells")?;
    Kernel::from_program(&mut gpu, &cache, &shade)?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A 12 m floor in the xy plane with a 12 m wall standing on its y axis, grey and
/// rough, under a uniform unit sky and a sun too dim to matter. The wall hides half
/// the sky from the floor at its foot and less with distance.
/// One frame of a sequence: its HDR red channel per pixel, and the cache's table and
/// outgoing radiance when the cache ran and was read.
type YardFrame = (
    Vec<f32>,
    Option<Vec<u32>>,
    Option<Vec<[f32; 4]>>,
    Option<Vec<u32>>,
);
struct Yard {
    floor: [Vertex; 4],
    wall: [Vertex; 4],
    /// More quads in the wall's material (none in the yard itself).
    extra: Vec<[Vertex; 4]>,
    /// Grey base colours of the floor and the wall.
    base: [f32; 2],
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
            extra: vec![],
            base: [0.5; 2],
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
        let objects: Vec<_> = [(&self.floor, self.base[0]), (&self.wall, self.base[1])]
            .into_iter()
            .chain(self.extra.iter().map(|quad| (quad, self.base[1])))
            .map(|(vertices, base)| ForwardObject {
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
            .collect();
        let mut history = Frames::default();
        let mut frames = Vec::new();
        for camera in cameras {
            let images = history.render(
                gpu,
                cache,
                &ForwardScene {
                    meshes: None,
                    materials: None,
                    instances: None,
                    objects: &objects,
                    lighting: &self.lighting,
                    camera: *camera,
                    size: SIZE,
                    atmosphere: renderlab::Atmosphere::off(),
                    sun_shadows: Some(&self.shadows),
                    primary: Primary::Traced,
                    post: None,
                    indirect,
                    reconstruction: None,
                    reflections: None,
                    textures: None,
                    local_shadows: None,
                    transparent: &[],
                    programs: None,
                    sky_map: None,
                    skins: None,
                },
            )?;
            let red = images
                .hdr
                .as_chunks::<8>()
                .0
                .iter()
                .map(|p| f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32())
                .collect();
            frames.push((
                red,
                images.indirect_table,
                images.indirect_outgoing,
                images.indirect_raw,
            ));
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
    cache.retire(&mut gpu)?;
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
    cache.retire(&mut gpu)?;
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
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Bounce on a sunlit floor with a dark wall, once the gathers have settled (frame 8 of
/// a still sequence). Three stages against the table and the outgoing radiance:
/// - **Cell shading.** Every shaded cell's outgoing radiance is one reflectance over pi
///   times its incident light (the sun where it faces it, plus its own gathered
///   bounce): the same reflectance for every cell of a material, just above its base
///   colour.
/// - **Gather.** A wall cell's blocked rays all end on the floor, whose radiance is
///   nearly uniform, so each of its four L1 coefficients is that radiance times the
///   kernel's quadrature over exactly its blocked directions.
/// - **Shading.** Wall pixels, black without bounce since the sun grazes the wall, show
///   the radiance of the wall cell they fall in.
#[test]
fn bounce_cells_shade_and_gather_what_the_scene_predicts() -> Result<()> {
    let path = root("gi-cache-bounce");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    const SUN: f64 = 1000.;
    let mut yard = Yard::new();
    yard.base = [0.8, 0.2];
    yard.lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0., 0., -1.),
            lux: [SUN as f32; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let settings = |bounce: bool| {
        Some(Indirect {
            settings: IndirectSettings {
                bounce,
                filter: 0,
                slots_log2: 14,
                ..Default::default()
            },
            read_table: true,
        })
    };
    let camera = Yard::camera(0)?;
    let plain = yard.sequence(&mut gpu, &cache, settings(false), &[camera])?;
    let frames = yard.sequence(&mut gpu, &cache, settings(true), &[camera; 8])?;
    let (red, Some(words), Some(outgoing), _) = frames.last().context("no frame")? else {
        anyhow::bail!("the table or the outgoing radiance was not read");
    };
    let previous = frames[6].1.as_ref().context("table was not read")?;
    // Every slot's hot record (the values); seen stamps in the cold records still tick.
    let hot = 64..64 + (words[1] as usize + 1) * 24;
    ensure!(
        words[hot.clone()] == previous[hot],
        "bounce had not settled by the seventh frame"
    );
    let table = Table::new(words)?;
    let cells = table.cells();
    let shaded = |axis: usize| {
        cells
            .iter()
            .filter(move |c| c.valid && c.settled && c.normal[axis] > 0.9 && c.point.x >= 0.)
    };
    // Cell shading. The sun is the floor's only light besides bounce; it grazes the wall.
    let mut reflectance = [(f64::MAX, 0f64); 2];
    for (material, (axis, sun)) in [(2, SUN), (0, 0.)].into_iter().enumerate() {
        for cell in shaded(axis) {
            let out = outgoing[cell.slot];
            ensure!(out[3] == 1., "slot {} is not shaded and final", cell.slot);
            let incident = sun + gi_cache_reference::bounce_irradiance(cell)[0];
            let a = f64::from(out[0]) * std::f64::consts::PI / incident;
            reflectance[material] = (
                reflectance[material].0.min(a),
                reflectance[material].1.max(a),
            );
        }
    }
    for ((least, most), base) in reflectance.into_iter().zip(yard.base) {
        // One value per material (to f16 in the bounce term), the base colour plus
        // the rough dielectric's specular share.
        ensure!(
            most - least < 2e-3 * most && (0.0..0.03).contains(&(least - f64::from(base))),
            "cell reflectance of base {base} spans {least:.5} to {most:.5}"
        );
    }
    // Gather: the floor's radiance, then each wall cell's coefficients.
    let floor: Vec<f64> = shaded(2).map(|c| f64::from(outgoing[c.slot][0])).collect();
    let (least, most) = floor
        .iter()
        .fold((f64::MAX, 0f64), |(lo, hi), l| (lo.min(*l), hi.max(*l)));
    ensure!(
        // A few percent brighter by the wall, which lights it back.
        floor.len() >= 500 && most / least < 1.05,
        "floor radiance is not uniform: {least} to {most} over {} cells",
        floor.len()
    );
    let (mut walls, mut worst) = (0, 0f64);
    for cell in shaded(0) {
        let weight = std::f64::consts::TAU / f64::from(gi_cache_reference::DIRECTIONS);
        let (mut sums, mut magnitudes) = ([0f64; 4], [0f64; 4]);
        for k in (0..gi_cache_reference::DIRECTIONS).filter(|k| cell.bits >> k & 1 == 0) {
            let d = gi_cache_reference::direction(cell, table.rotates(), k).as_dvec3();
            let y = [0.282095, 0.488603 * d.y, 0.488603 * d.z, 0.488603 * d.x];
            for i in 0..4 {
                sums[i] += weight * y[i];
                magnitudes[i] += weight * y[i].abs();
            }
        }
        if cell.bits == u64::MAX {
            continue;
        }
        walls += 1;
        let mean = (least + most) / 2.;
        for i in 0..4 {
            let stored = f64::from(cell.bounce[i * 3]) * f64::from(cell.bounce_scale);
            // The floor's spread, then f16 storage relative to the record's scale.
            let bound = (most - least) / 2. * magnitudes[i] + 1e-3 * f64::from(cell.bounce_scale);
            let error = (stored - mean * sums[i]).abs();
            worst = worst.max(error / bound);
            ensure!(
                error <= bound,
                "slot {} at {:?}: L1 coefficient {i} is {stored:.3}, quadrature gives {:.3}",
                cell.slot,
                cell.point,
                mean * sums[i]
            );
        }
    }
    ensure!(walls >= 500, "only {walls} wall cells gathered");
    // Shading: wall pixels against the cell nearest their point on the wall.
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    let exposure = f64::from(yard.lighting.exposure_stops).exp2();
    let wall: Vec<_> = shaded(0).collect();
    let (mut pixels, mut ratios) = (0, (f64::MAX, 0f64));
    for y in (4..SIZE[1]).step_by(8) {
        for x in (4..SIZE[0]).step_by(4) {
            let ndc = |depth: f64| {
                inverse.project_point3(DVec3::new(
                    (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2. - 1.,
                    1. - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.,
                    depth,
                ))
            };
            let (a, b) = (ndc(0.25), ndc(0.75));
            // The wall plane x = 0, above the floor and inside the wall's extent.
            let p = a + (b - a) * (-a.x / (b.x - a.x));
            if !(p.z > 0.3 && p.z < 5. && p.y.abs() < 5.) {
                continue;
            }
            let at = (y * SIZE[0] + x) as usize;
            ensure!(
                plain[0].0[at] == 0.,
                "wall pixel ({x}, {y}) is lit without bounce"
            );
            let nearest = wall
                .iter()
                .min_by(|c, d| {
                    let distance =
                        |cell: &&gi_cache_reference::Cell| cell.point.as_dvec3().distance(p);
                    distance(c).total_cmp(&distance(d))
                })
                .context("no wall cell")?;
            let ratio = f64::from(red[at]) / (f64::from(outgoing[nearest.slot][0]) * exposure);
            ratios = (ratios.0.min(ratio), ratios.1.max(ratio));
            pixels += 1;
        }
    }
    eprintln!(
        "reflectance {reflectance:?}; floor radiance {least:.2} to {most:.2}; {walls} wall cells, \
         worst coefficient at {worst:.2} of its bound; {pixels} wall pixels at {:.3} to {:.3} of \
         their cell's radiance",
        ratios.0, ratios.1
    );
    // A cell holds one value for its whole footprint and reflects as a Lambertian; the
    // pixel interpolates four cells and shades with the program's own lobes.
    ensure!(
        pixels >= 40 && ratios.0 > 0.8 && ratios.1 < 1.25,
        "{pixels} wall pixels are {:.3} to {:.3} of their cell's radiance",
        ratios.0,
        ratios.1
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// What one frame's filter replay covered.
#[derive(Clone, Copy, Debug, Default)]
struct Filtered {
    valid: usize,
    /// Cells whose filtered sky value differs from their unfiltered one.
    moved: usize,
    /// Cells with a filtered bounce value.
    bounced: usize,
    neighbours: usize,
    /// Split flags the filter's contrast refine set, and ones the probe had set.
    contrast: usize,
    probe: usize,
}
/// Recomputes `gi_filter_cell` for every valid cell of a frame read back with its
/// sources: the sky SH, the bounce SH where the filter writes one, and the split flag.
fn replay_filter(index: usize, frame: &YardFrame) -> Result<Filtered> {
    let (Some(words), Some(sources)) = (&frame.1, &frame.3) else {
        anyhow::bail!("the table or the filter sources were not read");
    };
    let table = Table::new(words)?;
    ensure!(
        table.filter() == (15, 0.5),
        "the filter is not at its default: {:?}",
        table.filter()
    );
    let raw = gi_cache_reference::Raw(sources);
    let cells = table.cells();
    let find: std::collections::HashMap<_, _> = cells
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.level, c.bin, c.lattice), i))
        .collect();
    let apart = |a: &[f64], b: &[f64]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0f64, f64::max)
    };
    let mut covered = Filtered::default();
    for cell in cells.iter().filter(|c| c.valid) {
        covered.valid += 1;
        let expected = gi_cache_reference::filter(&table, &raw, &cells, &find, cell);
        covered.neighbours += expected.neighbours;
        // Halves of values in [-1, 1], scaled; the filter's own sums are f32.
        let scale = f64::from(cell.scale);
        let stored: Vec<f64> = cell.sh.iter().map(|h| f64::from(*h) * scale).collect();
        ensure!(
            apart(&stored, &expected.sky) < 2e-3 * scale,
            "frame {index}, slot {} at {:?}: sky SH is not its filtered value",
            cell.slot,
            cell.point
        );
        covered.moved += usize::from(apart(&expected.sky, &raw.sky(cell.slot)) > 2e-3 * scale);
        if let Some(bounce) = &expected.bounce {
            let scale = f64::from(cell.bounce_scale.max(0.));
            let stored: Vec<f64> = cell.bounce.iter().map(|h| f64::from(*h) * scale).collect();
            let top = bounce.iter().fold(scale, |m, v| m.max(v.abs()));
            ensure!(
                apart(&stored, bounce) <= 2e-3 * top,
                "frame {index}, slot {} at {:?}: bounce SH is not its filtered value",
                cell.slot,
                cell.point
            );
            covered.bounced += 1;
        }
        ensure!(
            cell.split == expected.split,
            "frame {index}, slot {} at {:?}: split flag is {}, the filter's rule gives {}",
            cell.slot,
            cell.point,
            cell.split,
            expected.split
        );
        if cell.split {
            if raw.split(cell.slot) {
                covered.probe += 1;
            } else {
                covered.contrast += 1;
            }
        }
    }
    Ok(covered)
}
fn default_cache() -> Option<Indirect> {
    Some(Indirect {
        settings: IndirectSettings {
            slots_log2: 14,
            ..Default::default()
        },
        read_table: true,
    })
}

/// The cell filter at Renderlab's default settings (bounce on, filter bits 15), on the
/// yard under its uniform sky. In every frame of a still sequence, every valid cell's
/// hot record is what `gi_filter_cell` computes from the unfiltered sources: the
/// tent-, normal- and plane-weighted mean of its own value and its tangent neighbours',
/// for the sky SH and for the bounce SH. The filter moves nearly every cell's sky value,
/// so agreeing with it is not agreeing with the unfiltered one. Contrast refine does
/// not fire on this scene; the crevice test below is where it does.
#[test]
fn filtered_cells_are_the_weighted_mean_of_their_unfiltered_neighbourhood() -> Result<()> {
    let path = root("gi-cache-filter");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let yard = Yard::new();
    let frames = yard.sequence(&mut gpu, &cache, default_cache(), &[Yard::camera(0)?; 6])?;
    let mut last = Filtered::default();
    for (index, frame) in frames.iter().enumerate() {
        last = replay_filter(index, frame)?;
    }
    eprintln!("last frame: {last:?}");
    ensure!(
        last.valid >= 2000
            && last.moved * 10 >= last.valid * 9
            && last.bounced == last.valid
            && last.neighbours > 6 * last.valid
            && last.contrast + last.probe == 0,
        "the filter replay covered too little: {last:?}"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// Contrast refine, the filter's third default bit, on a slot: two 1.5 m walls 7 cm
/// apart on the floor, seen from its open end. A floor cell whose query point lies in
/// the slot sees almost no sky while the cells beside it, outside the walls, see most
/// of it; the filter marks such a cell split, and its pixels then read the next finer
/// level. The split flag of every valid cell equals the rule recomputed on the CPU
/// from the unfiltered sources (less than a quarter of an agreeing neighbour's
/// openness, that neighbour above a quarter), in every frame; the flags the probe set
/// itself on the facing walls are told apart from the filter's; and the finer cells
/// the refine asks for appear on the slot's floor after the first frame.
#[test]
fn contrast_refine_splits_the_cells_of_a_slot_as_its_rule_predicts() -> Result<()> {
    let path = root("gi-cache-crevice");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    const SLOT: [f32; 2] = [0.06, 0.13];
    let side = |x: f32| {
        [[x, -3., 0.], [x, 0., 0.], [x, 0., 1.5], [x, -3., 1.5]].map(|position| Vertex {
            position,
            normal: [1., 0., 0.],
            uv: [0.; 2],
            color: [1.; 4],
        })
    };
    let mut yard = Yard::new();
    yard.wall = side(SLOT[0]);
    yard.extra = vec![side(SLOT[1])];
    let camera = Camera::look_at(
        Vec3::new(0.095, 2.5, 1.8),
        Vec3::new(0.095, -0.5, 0.),
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let frames = yard.sequence(&mut gpu, &cache, default_cache(), &[camera; 5])?;
    // Floor cells with their query point between the walls, by level.
    let in_slot = |frame: &YardFrame| -> Result<std::collections::BTreeMap<u32, [usize; 2]>> {
        let table = Table::new(frame.1.as_ref().context("table was not read")?)?;
        let mut levels = std::collections::BTreeMap::new();
        for cell in table.cells().iter().filter(|c| {
            c.valid && c.bin == 4 && c.point.x > SLOT[0] && c.point.x < SLOT[1] && c.point.y < 0.
        }) {
            let entry: &mut [usize; 2] = levels.entry(cell.level).or_default();
            entry[usize::from(cell.split)] += 1;
        }
        Ok(levels)
    };
    let mut covered = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        covered.push(replay_filter(index, frame)?);
    }
    let (first, last) = (in_slot(&frames[0])?, in_slot(&frames[4])?);
    eprintln!(
        "first {:?}, last {:?}; slot cells by level first {first:?}, last {last:?}",
        covered[0], covered[4]
    );
    let coarsest = *first
        .keys()
        .next()
        .context("no slot cell in the first frame")?;
    ensure!(
        covered[0].contrast >= 10 && covered[0].probe == 0 && covered[4].contrast >= 30,
        "contrast refine fired on too few cells: {:?} then {:?}",
        covered[0],
        covered[4]
    );
    ensure!(
        first.len() == 1 && first[&coarsest][1] >= 10 && last.keys().any(|level| *level < coarsest),
        "the refine did not bring finer cells into the slot: {first:?} then {last:?}"
    );
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
