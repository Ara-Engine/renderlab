//! Unit N4 of docs/non-rt-path-2.md: the light-space caster grid (LSG).
//!
//! CPU: fixture rays against brute force, WGSL validation, and (manifest `cpu_only`) the
//! quarry replay: every covered pose-0 receiver of the saved R1 buffers with 8 rays, and
//! 256 on a 64x64 crop. Grid answers are compared with N0's any-hit BVH traversal on the
//! same f32 object-space rays, and per-receiver traffic is counted for a cell-size sweep.
//! GPU, one batched process: the grid is built on the GPU (count, scan, scatter, sort,
//! pyramid), then at poses 0, 1 and return the grid's 8- and 256-ray counts must equal N0's
//! software kernels byte for byte, with 8 repeats, tilted grids for a moving sun, rebuild
//! timing, and paired timing against hardware and N0.
//! Nested under software_bvh_tests for its quarry, BVH, bindings and timing helpers.
use super::*;
use crate::light_grid::{self as lsg, Frame, GpuPlacement, Grid, Placement, TraceStats};
use glam::Vec4;

/// 1 / d with software_ray's tiny-component rule.
fn inverse3(d: Vec3) -> Vec3 {
    let tiny = Vec3::select(d.cmpge(Vec3::ZERO), Vec3::splat(1e-30), Vec3::splat(-1e-30));
    Vec3::ONE / Vec3::select(d.abs().cmplt(Vec3::splat(1e-30)), tiny, d)
}
/// software_box: the widened slab entry of a box within [tmin, tmax], or infinity.
fn slab(o: Vec3, inv: Vec3, lo: Vec3, hi: Vec3, tmin: f32, tmax: f32) -> f32 {
    let (a, b) = ((lo - o) * inv, (hi - o) * inv);
    let (near, far) = (a.min(b).max_element(), a.max(b).min_element());
    let entry = (near - near.abs() * bvh::WIDEN).max(tmin);
    let exit = (far + far.abs() * bvh::WIDEN).min(tmax);
    if entry <= exit { entry } else { f32::INFINITY }
}

/// One placement for CPU replays: mesh slot, world-to-object rows as its instance record
/// holds them, and its padded world box.
struct Placed {
    slot: usize,
    rows: [Vec4; 3],
    lo: Vec3,
    hi: Vec3,
}
struct CpuWorld<'a> {
    tris: Vec<Triangles<'a>>,
    bvhs: &'a [Bvh],
    placed: Vec<Placed>,
    scene: bvh::Aabb,
}
impl<'a> CpuWorld<'a> {
    fn new(meshes: &'a [MeshData32], bvhs: &'a [Bvh], models: &[(usize, Mat4)]) -> Self {
        let mut scene = bvh::Aabb::EMPTY;
        let placed = models
            .iter()
            .map(|&(slot, model)| {
                let rows = model.as_dmat4().inverse().transpose();
                let b = bvh::world_bounds(model, &bvhs[slot]);
                scene.lo = scene.lo.min(b.lo);
                scene.hi = scene.hi.max(b.hi);
                Placed {
                    slot,
                    rows: [0, 1, 2].map(|r| rows.col(r).as_vec4()),
                    lo: b.lo,
                    hi: b.hi,
                }
            })
            .collect();
        Self { tris: meshes.iter().map(tris).collect(), bvhs, placed, scene }
    }
    fn object(&self, k: usize, ray: &Ray) -> Ray {
        let (r, q) = (&self.placed[k].rows, ray.origin.extend(1.0));
        Ray {
            origin: Vec3::new(r[0].dot(q), r[1].dot(q), r[2].dot(q)),
            direction: Vec3::new(
                r[0].truncate().dot(ray.direction),
                r[1].truncate().dot(ray.direction),
                r[2].truncate().dot(ray.direction),
            ),
            ..*ray
        }
    }
    fn boxed(&self, k: usize, ray: &Ray, inv: Vec3) -> bool {
        let p = &self.placed[k];
        slab(ray.origin, inv, p.lo, p.hi, ray.tmin, ray.tmax) <= ray.tmax
    }
    /// Margins and GPU placement records, in placement order.
    fn placements(&self, models: &[(usize, Mat4)]) -> Vec<GpuPlacement> {
        let mut first = 0;
        models
            .iter()
            .map(|&(slot, model)| {
                let count = self.tris[slot].len() as u32;
                let p = GpuPlacement {
                    first,
                    count,
                    margin: lsg::margin(model, self.bvhs[slot].bounds, self.scene),
                    mesh: slot as u32,
                };
                first += count;
                p
            })
            .collect()
    }
    fn grid(&self, models: &[(usize, Mat4)], l: Vec3, cell: f32) -> Grid {
        let margin = self.placements(models).iter().map(|p| p.margin).fold(0.0, f32::max);
        let boxes: Vec<_> = self.placed.iter().map(|p| bvh::Aabb { lo: p.lo, hi: p.hi }).collect();
        self.grid_in(models, Frame::new(l, cell, &boxes, margin))
    }
    fn grid_in(&self, models: &[(usize, Mat4)], frame: Frame) -> Grid {
        let gpu = self.placements(models);
        let placements: Vec<_> = models
            .iter()
            .zip(&gpu)
            .map(|(&(slot, model), g)| Placement { tris: self.tris[slot], model, margin: g.margin })
            .collect();
        Grid::build(frame, &placements)
    }
}

/// N0's work on one ray: BVH children read and triangles tested.
#[derive(Clone, Copy, Default)]
struct N0Count {
    children: u64,
    tests: u64,
}
/// N0's any-hit (software_blas_any) on CPU, counting its work.
fn bvh_any(b: &Bvh, t: Triangles<'_>, ray: &Ray, count: &mut N0Count) -> bool {
    let inv = inverse3(ray.direction);
    let mut stack: Vec<u32> = Vec::with_capacity(64);
    let mut node = 0u32;
    loop {
        for child in &b.nodes[node as usize] {
            count.children += 1;
            if child.kind == 0 {
                break;
            }
            let (lo, hi) = (Vec3::from_array(child.lo), Vec3::from_array(child.hi));
            if slab(ray.origin, inv, lo, hi, ray.tmin, ray.tmax) > ray.tmax {
                continue;
            }
            if child.kind == bvh::INNER {
                stack.push(child.index);
                continue;
            }
            for i in 0..child.kind {
                count.tests += 1;
                if bvh::intersect(t, ray, b.prims[(child.index + i) as usize]).is_some() {
                    return true;
                }
            }
        }
        match stack.pop() {
            Some(n) => node = n,
            None => return false,
        }
    }
}
/// N0's software_occluded on CPU: placement boxes, then each placement's any-hit BVH.
fn n0_blocked(world: &CpuWorld<'_>, ray: &Ray, count: &mut N0Count) -> bool {
    let inv = inverse3(ray.direction);
    (0..world.placed.len()).any(|k| {
        world.boxed(k, ray, inv) && {
            let p = &world.placed[k];
            bvh_any(&world.bvhs[p.slot], world.tris[p.slot], &world.object(k, ray), count)
        }
    })
}
/// The grid query with N0's per-placement box and object-space watertight test.
fn grid_blocked(world: &CpuWorld<'_>, grid: &Grid, ray: &Ray, start: usize, stats: &mut TraceStats) -> Option<bool> {
    let inv = inverse3(ray.direction);
    let mut cache: [Option<Option<Ray>>; 16] = [None; 16];
    grid.trace(ray.origin, ray.direction, ray.tmin, ray.tmax, start, stats, None, |id| {
        let k = (id >> lsg::PLACEMENT_SHIFT) as usize;
        let object = *cache[k].get_or_insert_with(|| world.boxed(k, ray, inv).then(|| world.object(k, ray)));
        object.is_some_and(|o| {
            bvh::intersect(world.tris[world.placed[k].slot], &o, id & lsg::PRIMITIVE_MASK).is_some()
        })
    })
}

/// Unit vector toward the sun of a lighting file, as the renderer normalizes it.
fn sun(lighting: &crate::Lighting) -> Vec3 {
    match lighting.lights[0] {
        crate::Light::Directional { travel, .. } => -travel.as_dvec3().normalize().as_vec3(),
        _ => panic!("the quarry's first light is the sun"),
    }
}
/// `l` tilted by `alpha` radians toward a fixed perpendicular, for moving-sun grids.
fn tilted(l: Vec3, alpha: f32) -> Vec3 {
    let l64 = l.as_dvec3();
    let side = l64.cross(glam::DVec3::Z).normalize();
    (l64 * f64::from(alpha).cos() + side * f64::from(alpha).sin()).normalize().as_vec3()
}

fn fresh_hash(input: u32) -> u32 {
    let mut x = input;
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb_352d);
    x = (x ^ (x >> 15)).wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}
/// fresh_estimate's rays on CPU for one receiver: the same R2 directions, Îµ, thin origins
/// and per-pixel shift, around `l` in a basis of its own (the CPU has no shadow view), with
/// tmax at the scene box. Directions are not bit-identical to the GPU's; both CPU sides
/// trace the same rays.
struct Rays {
    l: Vec3,
    u: Vec3,
    v: Vec3,
    slope: f32,
    eye: Vec3,
    width: u32,
}
impl Rays {
    fn of(&self, world: [f32; 4], normal: [f32; 4], pixel: [u32; 2], count: u32, scene: &bvh::Aabb) -> Vec<Ray> {
        let p = Vec3::new(world[0], world[1], world[2]);
        let g = Vec3::new(normal[0], normal[1], normal[2]);
        let (epsilon, thin) = (world[3], normal[3] < 0.0);
        let origin_toward = |toward: Vec3| {
            let n = if g.dot(toward) >= 0.0 { g } else { -g };
            n.mul_add(Vec3::splat(epsilon), p)
        };
        let base = origin_toward(if thin { self.l } else { self.eye - p });
        let seed = fresh_hash(pixel[0] + pixel[1] * self.width);
        let shift = glam::Vec2::new((seed >> 8) as f32, (fresh_hash(seed ^ 0xa511_e9b3) >> 8) as f32)
            * (1.0 / 16_777_216.0);
        (0..count)
            .filter_map(|j| {
                let r = glam::Vec2::splat(j as f32) * glam::Vec2::new(0.754_877_7, 0.569_840_3) + shift;
                let r = r - r.floor();
                let a = 6.283_185_5 * r.y;
                let d = (self.l + self.slope * r.x.sqrt() * (a.cos() * self.u + a.sin() * self.v)).normalize();
                let o = if thin { origin_toward(d) } else { base };
                let inv = d.as_dvec3().recip();
                let (t0, t1) = ((scene.lo.as_dvec3() - o.as_dvec3()) * inv, (scene.hi.as_dvec3() - o.as_dvec3()) * inv);
                let tmax = t0.max(t1).min_element() as f32;
                (tmax > epsilon).then_some(Ray { origin: o, direction: d, tmin: epsilon, tmax })
            })
            .collect()
    }
}

#[test]
fn light_grid_matches_brute_force_on_fixtures() {
    let mut rng = Rng(0x15c0);
    let far = {
        let (p, i) = grid(&mut rng, 30);
        let p = p.iter().map(|v| [v[0] - 400.0, v[2], v[1] - 300.0]).collect();
        (p, i)
    };
    let meshes = [soup(&mut rng, 1500), grid(&mut rng, 30), stacked(), far];
    let bvhs: Vec<Bvh> = meshes.iter().map(|m| bvh::build(tris(m), 4, 16.0)).collect();
    // Placement 4 holds object coordinates 300-410 m from its origin, as quarry placement 11.
    let models = [
        (0, Mat4::IDENTITY),
        (1, Mat4::from_translation(Vec3::new(1.0, 2.0, 0.5))),
        (2, Mat4::from_rotation_y(0.3) * Mat4::from_translation(Vec3::new(3.0, 1.0, 2.0))),
        (1, Mat4::from_rotation_x(-1.2) * Mat4::from_translation(Vec3::new(0.5, -0.5, 4.0))),
        (3, Mat4::from_translation(Vec3::new(402.0, 1.5, 303.0))),
    ];
    let world = CpuWorld::new(&meshes, &bvhs, &models);
    let brutes: Vec<_> = meshes.iter().zip(&bvhs).map(|(m, b)| bvh::Brute::new(tris(m), scale(b))).collect();
    let brute = |ray: &Ray| {
        let inv = inverse3(ray.direction);
        (0..models.len()).any(|k| {
            world.boxed(k, ray, inv) && {
                let slot = world.placed[k].slot;
                brutes[slot].trace(world.tris[slot], &world.object(k, ray), Mode::Any).is_some()
            }
        })
    };
    let l = Vec3::new(0.4, 0.8, 0.3).normalize();
    // Rays from points on random triangles (a fifth on a vertex or an edge), on either side
    // at 10 Âµm-1 mm, around the sun within 0.15 rad; one in ten random, some sinking.
    let mut rays = vec![];
    for i in 0..3000 {
        let k = rng.below(models.len());
        let (slot, model) = models[k];
        let v = tris(&meshes[slot]).vertices(rng.below(meshes[slot].1.len() / 3) as u32);
        let (a, b) = (rng.unit(), rng.unit());
        let w = match rng.below(10) {
            0 => [1.0, 0.0, 0.0],
            1 => [a, 1.0 - a, 0.0],
            _ => [1.0 - a.sqrt(), a.sqrt() * (1.0 - b), a.sqrt() * b],
        };
        let point = model.transform_point3(v[0] * w[0] + v[1] * w[1] + v[2] * w[2]);
        let n = model.transform_vector3((v[1] - v[0]).cross(v[2] - v[0])).normalize_or_zero();
        let epsilon = 1e-5 * 100f32.powf(rng.unit());
        let side = if rng.below(2) == 0 { 1.0 } else { -1.0 };
        let direction = if i % 10 == 0 {
            rng.direction()
        } else {
            (l + rng.direction() * 0.15 * rng.unit()).normalize()
        };
        rays.push(Ray { origin: point + n * side * epsilon, direction, tmin: epsilon, tmax: 1e30 });
    }
    let truth: Vec<bool> = rays.iter().map(brute).collect();
    let blocked = truth.iter().filter(|b| **b).count();
    assert!(blocked > 300 && blocked < 2700, "fixture rays should mix: {blocked}");
    for (cell, alpha, start) in [(0.1, 0.0, 0), (0.1, 0.0, 3), (0.04, 0.1, 2), (0.25, 0.05, 9)] {
        let grid = world.grid(&models, tilted(l, alpha), cell);
        assert!(grid.stats.entries > grid.stats.triangles);
        let mut stats = TraceStats::default();
        for (ray, &truth) in rays.iter().zip(&truth) {
            let walked = grid_blocked(&world, &grid, ray, start, &mut stats);
            let rises = ray.direction.dot(grid.frame.l) > 0.0;
            assert_eq!(walked.is_some(), rises, "only sinking rays fall back");
            assert_eq!(
                walked.unwrap_or(truth),
                truth,
                "cell {cell} alpha {alpha} start {start}: {ray:?}"
            );
        }
    }
}

#[test]
fn light_grid_wgsl_validates() {
    let soft = &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1];
    let frame = Frame {
        u: Vec3::X,
        v: Vec3::Y,
        l: Vec3::Z,
        origin: glam::Vec2::ZERO,
        cell: 0.01,
        dims: [3000, 2000],
    };
    let layout = lsg::Layout::new(&frame, 20_000_000, 13_000_000);
    assert_eq!((layout.front_chunks(), layout.entry_chunks()), (3, 4));
    let constants = lsg::constants(layout.front_chunks(), layout.entry_chunks(), 12);
    let query = lsg::traced_source(soft, &bvh::constants(12, 12, 32), &constants);
    assert!(query.contains("blocked=light_grid_occluded(") && !query.contains("blocked=software_occluded("));
    let (module, info) = validate_wgsl(&query);
    let allowed = [&[3, 4, 5, 7, 10, 13, 20, 21, 23][..], &[0, 1, 5], &[40, 41, 42, 43, 44, 45, 46, 47, 48, 49]];
    for (group, binding) in bindings(&module, &info, "fresh_compute") {
        assert!(group == 3 || allowed[group as usize].contains(&binding), "{group}/{binding}");
    }
    let (module, info) = validate_wgsl(&lsg::build_source(&constants));
    for entry in ["light_grid_clear", "light_grid_count", "light_grid_scan", "light_grid_scan_sums", "light_grid_scan_add", "light_grid_scatter", "light_grid_sort", "light_grid_pyramid"] {
        assert!(bindings(&module, &info, entry).iter().all(|&(g, b)| g == 0 && b <= 5), "{entry}");
    }
    assert_eq!(lsg::params(&frame, &layout, 0, &[GpuPlacement { first: 0, count: 1, margin: 0.0, mesh: 0 }]).len(), 112 + 128 + 256);
}

/// mean, p50, p99 and max of per-receiver counters.
fn spread(values: &mut [u64]) -> serde_json::Value {
    values.sort_unstable();
    let n = values.len().max(1);
    let at = |q: f64| values.get(((n - 1) as f64 * q) as usize).copied().unwrap_or(0);
    let mean = values.iter().sum::<u64>() as f64 / n as f64;
    serde_json::json!({"mean": mean, "p50": at(0.5), "p99": at(0.99), "max": at(1.0)})
}

/// Per-receiver work of one grid configuration, and the identity verdict.
#[derive(Default)]
struct Replay {
    rays: u64,
    blocked: u64,
    mismatches: u64,
    fallbacks: u64,
    examples: Vec<serde_json::Value>,
    lookups: Vec<u64>,
    lists: Vec<u64>,
    entries: Vec<u64>,
    tests: Vec<u64>,
    bytes: Vec<u64>,
}
/// Modelled bytes: 4 per pyramid cell, 8 per list (two ends), 12 per entry, 60 per
/// triangle test (three indices, three vertex vec4 reads).
fn grid_bytes(s: &TraceStats) -> u64 {
    4 * u64::from(s.lookups) + 8 * u64::from(s.lists) + 12 * u64::from(s.entries) + 60 * u64::from(s.tests)
}
/// N0's: 32 per child read, 60 per triangle test.
fn n0_bytes(c: &N0Count) -> u64 {
    32 * c.children + 60 * c.tests
}

/// Replays `receivers` with `count` rays each against the grid (and N0 when `truth` is
/// empty, filling it), on `threads` threads.
#[allow(clippy::too_many_arguments)]
fn replay(
    world: &CpuWorld<'_>,
    grid: Option<&Grid>,
    rays: &Rays,
    receivers: &[(u32, [f32; 4], [f32; 4])],
    count: u32,
    start: usize,
    truth: &mut Vec<Vec<bool>>,
    n0: &mut Vec<N0Count>,
    threads: usize,
) -> Replay {
    let fill = truth.is_empty();
    let width = rays.width;
    let chunk = receivers.len().div_ceil(threads);
    let results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = receivers
            .chunks(chunk)
            .enumerate()
            .map(|(c, part)| {
                let truth = &*truth;
                s.spawn(move || {
                    let mut out = Replay::default();
                    let mut bits = vec![];
                    let mut works = vec![];
                    for (i, &(pixel, w, n)) in part.iter().enumerate() {
                        let index = c * chunk + i;
                        let rays_of = rays.of(w, n, [pixel % width, pixel / width], count, &world.scene);
                        let mut mine = vec![];
                        let mut work = N0Count::default();
                        for ray in &rays_of {
                            mine.push(if fill { n0_blocked(world, ray, &mut work) } else { false });
                        }
                        if fill {
                            bits.push(mine.clone());
                            works.push(work);
                        }
                        let truth_of = if fill { &mine } else { &truth[index] };
                        let Some(grid) = grid else { continue };
                        let mut stats = TraceStats::default();
                        for (j, ray) in rays_of.iter().enumerate() {
                            out.rays += 1;
                            let walked = grid_blocked(world, grid, ray, start, &mut stats);
                            out.fallbacks += u64::from(walked.is_none());
                            let answer = walked.unwrap_or(truth_of[j]);
                            out.blocked += u64::from(truth_of[j]);
                            if answer != truth_of[j] {
                                out.mismatches += 1;
                                if out.examples.len() < 16 {
                                    out.examples.push(serde_json::json!({"pixel": pixel, "ray": j,
                                        "origin": ray.origin.to_array(), "direction": ray.direction.to_array(),
                                        "tmin": ray.tmin, "tmax": ray.tmax, "n0": truth_of[j]}));
                                }
                            }
                        }
                        out.lookups.push(stats.lookups.into());
                        out.lists.push(stats.lists.into());
                        out.entries.push(stats.entries.into());
                        out.tests.push(stats.tests.into());
                        out.bytes.push(grid_bytes(&stats));
                    }
                    (out, bits, works)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut total = Replay::default();
    for (part, bits, works) in results {
        total.rays += part.rays;
        total.blocked += part.blocked;
        total.mismatches += part.mismatches;
        total.fallbacks += part.fallbacks;
        total.examples.extend(part.examples);
        total.lookups.extend(part.lookups);
        total.lists.extend(part.lists);
        total.entries.extend(part.entries);
        total.tests.extend(part.tests);
        total.bytes.extend(part.bytes);
        if fill {
            truth.extend(bits);
            n0.extend(works);
        }
    }
    total
}
impl Replay {
    fn summary(mut self) -> serde_json::Value {
        let bytes: u64 = self.bytes.iter().sum();
        serde_json::json!({"rays": self.rays, "blocked": self.blocked, "mismatches": self.mismatches,
            "fallbacks": self.fallbacks, "examples": self.examples, "total_bytes": bytes,
            "per_receiver": {"pyramid_lookups": spread(&mut self.lookups), "lists": spread(&mut self.lists),
                "entries_read": spread(&mut self.entries), "triangle_tests": spread(&mut self.tests),
                "bytes": spread(&mut self.bytes)}})
    }
}

/// N0's measured 8-ray software shadow cost at pose 0 (docs/non-rt-n0.md), ms, which the
/// CPU traffic model scales by bytes; and hardware's.
const N0_SHADOWS8_MS: f64 = 9.432;
const HW_SHADOWS8_MS: f64 = 0.425;

/// The CPU replay of step 1 (no GPU): the quarry, N0's BVHs and the R1 pose-0 receivers.
fn cpu_replay(manifest: &serde_json::Value) -> serde_json::Value {
    let started = Instant::now();
    let mut quarry = decode_quarry(manifest);
    drop(quarry.source.take());
    let (bvhs, bvh_build) = build_all(&quarry.meshes, 4, 16.0);
    let models: Vec<(usize, Mat4)> = quarry
        .models
        .iter()
        .enumerate()
        .flat_map(|(slot, ms)| ms.iter().map(move |&m| (slot, m)))
        .collect();
    let world = CpuWorld::new(&quarry.meshes, &bvhs, &models);
    let r1 = manifest_path(manifest, "r1");
    let positions = f32s(&fs::read(r1.join("pose-0.world.f32")).unwrap());
    let normals = f32s(&fs::read(r1.join("pose-0.normal.f32")).unwrap());
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(r1.join("pose-0.json")).unwrap()).unwrap();
    let eye = Vec3::from_slice(&meta["eye"].as_array().unwrap()[..3].iter().map(|v| v.as_f64().unwrap() as f32).collect::<Vec<_>>());
    let lighting = lighting_settings::parse(&fs::read_to_string(manifest_path(manifest, "lighting")).unwrap()).unwrap();
    let l = sun(&lighting);
    let u = l.cross(Vec3::Z).normalize();
    let rays = Rays { l, u, v: l.cross(u), slope: 0.04f32.tan(), eye, width: 1920 };
    let receivers: Vec<(u32, [f32; 4], [f32; 4])> = (0..1920 * 1080)
        .filter(|&i| normals[4 * i + 3] != 0.0)
        .map(|i| {
            let at = |b: &[f32]| [b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]];
            (i as u32, at(&positions), at(&normals))
        })
        .collect();
    drop((positions, normals));
    let threads = manifest["threads"].as_u64().unwrap_or(4) as usize;
    eprintln!("light_grid_replay receivers={} threads={threads}", receivers.len());
    // N0 first: the reference bits and its traffic.
    let (mut truth, mut n0) = (vec![], vec![]);
    let n0_started = Instant::now();
    replay(&world, None, &rays, &receivers, 8, 0, &mut truth, &mut n0, threads);
    let n0_s = n0_started.elapsed().as_secs_f64();
    let n0_total: u64 = n0.iter().map(n0_bytes).sum();
    let blocked: usize = truth.iter().flatten().filter(|b| **b).count();
    let rays8: usize = truth.iter().map(Vec::len).sum();
    let mut summary = serde_json::Map::new();
    summary.insert("bvh_build".into(), bvh_build);
    summary.insert("light".into(), l.to_array().to_vec().into());
    summary.insert("receivers".into(), receivers.len().into());
    summary.insert("n0".into(), serde_json::json!({"rays": rays8, "blocked": blocked, "seconds": n0_s,
        "total_bytes": n0_total,
        "per_receiver": {"children_read": spread(&mut n0.iter().map(|c| c.children).collect::<Vec<_>>()),
            "triangle_tests": spread(&mut n0.iter().map(|c| c.tests).collect::<Vec<_>>()),
            "bytes": spread(&mut n0.iter().map(n0_bytes).collect::<Vec<_>>())}}));
    eprintln!("light_grid_replay n0 rays={rays8} blocked={blocked} s={n0_s:.1}");
    let cells: Vec<f32> = manifest["cells"].as_array().map_or(vec![0.008, 0.012, 0.016], |a| a.iter().map(|v| v.as_f64().unwrap() as f32).collect());
    let start = manifest["start"].as_u64().unwrap_or(0) as usize;
    let mut configs = vec![];
    let mut best = (u64::MAX, cells[0]);
    let mut others = 0;
    for &cell in &cells {
        let grid = world.grid(&models, l, cell);
        eprintln!("light_grid_replay cell={cell} build={:?}", grid.stats);
        let replay_started = Instant::now();
        let r = replay(&world, Some(&grid), &rays, &receivers, 8, start, &mut truth, &mut n0, threads);
        let seconds = replay_started.elapsed().as_secs_f64();
        others += r.mismatches;
        let total: u64 = r.bytes.iter().sum();
        if total < best.0 {
            best = (total, cell);
        }
        let mut c = r.summary();
        c["cell_m"] = cell.into();
        c["start_level"] = start.into();
        c["seconds"] = seconds.into();
        c["build"] = serde_json::to_value(grid.stats).unwrap();
        c["frame"] = grid.frame.summary();
        c["projected_ms_by_n0_bytes"] = (N0_SHADOWS8_MS * total as f64 / n0_total as f64).into();
        c["projected_ms_at_1TBps"] = (total as f64 / 1e9).into();
        eprintln!("light_grid_replay cell={cell} mismatches={} entries={} bytes={total} s={seconds:.1}", c["mismatches"], c["per_receiver"]["entries_read"]["mean"]);
        configs.push(c);
    }
    summary.insert("interactive".into(), configs.into());
    // Start levels and the Exact crop at the cell size with the least traffic.
    let cell = best.1;
    let grid = world.grid(&models, l, cell);
    let mut levels = vec![];
    for s in [2usize, 4, 99] {
        let r = replay(&world, Some(&grid), &rays, &receivers, 8, s, &mut truth, &mut n0, threads);
        others += r.mismatches;
        let mut c = r.summary();
        c["start_level"] = s.into();
        levels.push(c);
    }
    summary.insert("start_levels".into(), serde_json::json!({"cell_m": cell, "runs": levels}));
    let mut pixels: Vec<u32> = receivers.iter().map(|r| r.0).collect();
    pixels.sort_unstable();
    let middle = pixels[pixels.len() / 2];
    let (cx, cy) = ((middle % 1920).clamp(32, 1888) - 32, (middle / 1920).clamp(32, 1048) - 32);
    let crop: Vec<_> = receivers
        .iter()
        .filter(|r| (cx..cx + 64).contains(&(r.0 % 1920)) && (cy..cy + 64).contains(&(r.0 / 1920)))
        .copied()
        .collect();
    let (mut crop_truth, mut crop_n0) = (vec![], vec![]);
    replay(&world, None, &rays, &crop, 256, 0, &mut crop_truth, &mut crop_n0, threads);
    let r = replay(&world, Some(&grid), &rays, &crop, 256, start, &mut crop_truth, &mut crop_n0, threads);
    others += r.mismatches;
    let crop_n0_total: u64 = crop_n0.iter().map(n0_bytes).sum();
    let mut exact = r.summary();
    exact["crop"] = serde_json::json!({"x": cx, "y": cy, "size": 64, "receivers": crop.len()});
    exact["n0_total_bytes"] = crop_n0_total.into();
    exact["n0_per_receiver_bytes"] = spread(&mut crop_n0.iter().map(n0_bytes).collect::<Vec<_>>());
    summary.insert("exact_crop".into(), exact);
    let best_config = summary["interactive"].as_array().unwrap().iter()
        .find(|c| c["cell_m"].as_f64() == Some(f64::from(cell)))
        .unwrap()
        .clone();
    let mean_entries = best_config["per_receiver"]["entries_read"]["mean"].as_f64().unwrap();
    let projected = best_config["projected_ms_by_n0_bytes"].as_f64().unwrap();
    summary.insert("stop_rule".into(), serde_json::json!({"cell_m": cell, "mean_entries_read": mean_entries,
        "entries_above_400": mean_entries > 400.0, "projected_ms_by_n0_bytes": projected, "projected_above_1ms": projected > 1.0,
        "hardware_shadows8_ms": HW_SHADOWS8_MS, "mismatches_all_runs": others}));
    summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
    serde_json::Value::Object(summary)
}

/// The scene inputs every grid build reads.
struct GpuScene {
    models: wgpu::Buffer,
    vertices: Vec<wgpu::Buffer>,
    indices: Vec<wgpu::Buffer>,
    placements: Vec<GpuPlacement>,
    boxes: Vec<bvh::Aabb>,
}
/// One grid: its frame and its layout in the shared heap (capacity: its own entry count).
struct Spec {
    name: String,
    cell: f32,
    alpha: f32,
    frame: Frame,
    layout: lsg::Layout,
}
const BUILD_STAGES: [&str; 6] = ["clear", "count", "scan", "scatter", "sort", "pyramid"];

fn whole(buffers: &[wgpu::Buffer]) -> Vec<wgpu::BufferBinding<'_>> {
    buffers.iter().map(wgpu::Buffer::as_entire_buffer_binding).collect()
}
fn chunked(r: &Renderer, words: usize, label: &str) -> Vec<wgpu::Buffer> {
    (0..words.div_ceil(lsg::CHUNK_WORDS).max(1))
        .map(|i| {
            let n = (words - (i * lsg::CHUNK_WORDS).min(words)).clamp(4, lsg::CHUNK_WORDS);
            r.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 4 * n as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .collect()
}
/// 256-invocation workgroups over `n` items, split into rows of at most 32768.
fn groups_for(n: u32) -> [u32; 2] {
    let g = n.div_ceil(256).max(1);
    let x = g.min(32768);
    [x, g.div_ceil(x)]
}
fn read_words(r: &Renderer, buffer: &wgpu::Buffer, word: u64, count: u64) -> Vec<u32> {
    let staging = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("light grid words"),
        size: 4 * count,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    submit(r, |e| e.copy_buffer_to_buffer(buffer, 4 * word, &staging, 0, 4 * count));
    u32s(&wait_map(r, &staging))
}

/// The GPU grid storage, sized for the largest grid: one parameter block, both chunked
/// heaps and the build pipelines. Grids are built into it one at a time, so one software
/// binding set and one query module serve every grid.
struct GpuHeap {
    params: wgpu::Buffer,
    front: Vec<wgpu::Buffer>,
    entries: Vec<wgpu::Buffer>,
    group: wgpu::BindGroup,
    /// clear, count, scan, scan_sums, scan_add, scatter, sort, then pyramid levels 1..
    pipelines: Vec<wgpu::ComputePipeline>,
    triangles: u32,
    /// The grid WGSL constants (chunk counts and meshes) every module uses.
    constants: String,
}
impl GpuHeap {
    fn new(r: &Renderer, scene: &GpuScene, front_words: usize, capacity: u32, levels: usize) -> Self {
        let scopes = OperationScopes::new(&r.device);
        let meshes = scene.vertices.len();
        let params = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light grid params"),
            size: 496,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let front = chunked(r, front_words, "light grid front");
        let entries = chunked(r, lsg::ENTRY_WORDS * capacity as usize, "light grid entries");
        let bgl = group_layout(
            r,
            &[
                layout_entry(0, storage(true), Some(1)),
                layout_entry(1, storage(false), Some(front.len() as u32)),
                layout_entry(2, storage(false), Some(entries.len() as u32)),
                layout_entry(3, storage(true), None),
                layout_entry(4, storage(true), Some(meshes as u32)),
                layout_entry(5, storage(true), Some(meshes as u32)),
            ],
        );
        let (pb, fb, eb, vb, ib) = (whole(std::slice::from_ref(&params)), whole(&front), whole(&entries), whole(&scene.vertices), whole(&scene.indices));
        use wgpu::BindingResource as R;
        let group = group(
            r,
            &bgl,
            &[
                (0, R::BufferArray(&pb)),
                (1, R::BufferArray(&fb)),
                (2, R::BufferArray(&eb)),
                (3, scene.models.as_entire_binding()),
                (4, R::BufferArray(&vb)),
                (5, R::BufferArray(&ib)),
            ],
        );
        let pipeline_layout = r.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("light grid build"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let constants = lsg::constants(front.len(), entries.len(), meshes);
        let m = module(r, &lsg::build_source(&constants));
        let mut pipelines: Vec<_> = ["light_grid_clear", "light_grid_count", "light_grid_scan", "light_grid_scan_sums", "light_grid_scan_add", "light_grid_scatter", "light_grid_sort"]
            .iter()
            .map(|e| compute(r, &pipeline_layout, &m, e, &[]))
            .collect();
        for k in 1..levels {
            pipelines.push(compute(r, &pipeline_layout, &m, "light_grid_pyramid", &[("LIGHT_LEVEL", k as f64)]));
        }
        if let Some(e) = scopes.finish() {
            panic!("light grid build pipelines failed: {e}");
        }
        let triangles = scene.placements.iter().map(|p| p.count).sum();
        Self { params, front, entries, group, pipelines, triangles, constants }
    }
    fn load(&self, r: &Renderer, scene: &GpuScene, spec: &Spec, start: u32) {
        r.queue.write_buffer(&self.params, 0, &lsg::params(&spec.frame, &spec.layout, start, &scene.placements));
    }
    /// Encodes build stages `0..stages` (BUILD_STAGES) of the loaded grid, each its own
    /// pass with optional timestamps at 2i and 2i + 1.
    fn encode(&self, e: &mut wgpu::CommandEncoder, frame: &Frame, stages: usize, stamps: Option<&wgpu::QuerySet>) {
        self.encode_stages(e, frame, 0..stages, stamps);
    }
    fn encode_stages(&self, e: &mut wgpu::CommandEncoder, frame: &Frame, stages: std::ops::Range<usize>, stamps: Option<&wgpu::QuerySet>) {
        let cells = frame.dims[0] * frame.dims[1];
        let blocks = cells.div_ceil(1024);
        let levels = frame.levels();
        for stage in stages {
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(BUILD_STAGES[stage]),
                timestamp_writes: stamps.map(|q| wgpu::ComputePassTimestampWrites {
                    query_set: q,
                    beginning_of_pass_write_index: Some(2 * stage as u32),
                    end_of_pass_write_index: Some(2 * stage as u32 + 1),
                }),
            });
            pass.set_bind_group(0, &self.group, &[]);
            let mut run = |pipeline: usize, [x, y]: [u32; 2]| {
                pass.set_pipeline(&self.pipelines[pipeline]);
                pass.dispatch_workgroups(x, y, 1);
            };
            match stage {
                0 => run(0, groups_for(cells)),
                1 => run(1, groups_for(self.triangles)),
                2 => {
                    let x = blocks.min(32768);
                    run(2, [x, blocks.div_ceil(x)]);
                    run(3, [1, 1]);
                    run(4, groups_for(cells));
                }
                3 => run(5, groups_for(self.triangles)),
                4 => run(6, groups_for(cells)),
                _ => {
                    for (k, [w, h]) in levels.iter().enumerate().skip(1) {
                        run(6 + k, groups_for(w * h));
                    }
                }
            }
        }
    }
    /// A front-heap word.
    fn word(&self, r: &Renderer, word: u32) -> u32 {
        let w = word as usize;
        read_words(r, &self.front[w / lsg::CHUNK_WORDS], (w % lsg::CHUNK_WORDS) as u64, 1)[0]
    }
    /// Loads and builds `spec`, checking its entry count and the overflow flag.
    fn build(&self, r: &Renderer, scene: &GpuScene, spec: &Spec, start: u32) {
        self.load(r, scene, spec, start);
        submit(r, |e| self.encode(e, &spec.frame, BUILD_STAGES.len(), None));
        let total = spec.layout.total(&spec.frame);
        assert_eq!(self.word(r, total), spec.layout.capacity, "{}: entry count changed", spec.name);
        assert_eq!(self.word(r, total + 1), 0, "{}: grid overflow", spec.name);
    }
    fn bytes(&self) -> u64 {
        self.front.iter().chain(&self.entries).chain([&self.params]).map(wgpu::Buffer::size).sum()
    }
    /// Query bindings 47-49 of the software group, all binding arrays.
    fn extra(&self) -> Vec<(wgpu::BindGroupLayoutEntry, Vec<wgpu::BufferBinding<'_>>)> {
        vec![
            (layout_entry(47, storage(true), Some(1)), whole(std::slice::from_ref(&self.params))),
            (layout_entry(48, storage(true), Some(self.front.len() as u32)), whole(&self.front)),
            (layout_entry(49, storage(true), Some(self.entries.len() as u32)), whole(&self.entries)),
        ]
    }
    /// SHA-256 of both heaps' words that `spec` uses.
    fn hash(&self, r: &Renderer, spec: &Spec) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        for (buffers, words) in [(&self.front, spec.layout.front_words as usize), (&self.entries, lsg::ENTRY_WORDS * spec.layout.capacity as usize)] {
            for (i, b) in buffers.iter().enumerate() {
                let n = words.saturating_sub(i * lsg::CHUNK_WORDS).min(lsg::CHUNK_WORDS);
                if n > 0 {
                    h.update(u32s(&read_buffer(r, b))[..n].iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<_>>());
                }
            }
        }
        format!("{:x}", h.finalize())
    }
}

/// The first `words` words of a chunked heap.
fn heap_words(r: &Renderer, buffers: &[wgpu::Buffer], words: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(words);
    for b in buffers {
        if out.len() >= words {
            break;
        }
        out.extend(u32s(&read_buffer(r, b)));
    }
    out.truncate(words);
    out
}
/// Stage-by-stage consistency of the GPU build of `spec`: the scatter must end every list
/// at the next list's begin, lists must be sorted with their top at pyramid level 0, every
/// pyramid cell must be the max of its children, and two builds must give the same heap.
fn heap_check(r: &Renderer, heap: &GpuHeap, scene: &GpuScene, spec: &Spec, start: u32) -> serde_json::Value {
    let f = &spec.frame;
    let cells = (f.dims[0] * f.dims[1]) as usize;
    heap.load(r, scene, spec, start);
    submit(r, |e| heap.encode_stages(e, f, 0..3, None));
    let begins = heap_words(r, &heap.front, cells);
    submit(r, |e| heap.encode_stages(e, f, 3..4, None));
    let ends = heap_words(r, &heap.front, cells);
    let total = spec.layout.capacity;
    let next = |c: usize| if c + 1 < cells { begins[c + 1] } else { total };
    let bad_ends: Vec<usize> = (0..cells).filter(|&c| ends[c] != next(c)).collect();
    submit(r, |e| heap.encode_stages(e, f, 4..6, None));
    let front = heap_words(r, &heap.front, spec.layout.front_words as usize);
    let entries = heap_words(r, &heap.entries, lsg::ENTRY_WORDS * total as usize);
    let (mut unsorted, mut bad_top0) = (0usize, 0usize);
    for c in 0..cells {
        let (b, e) = (begins[c] as usize, (ends[c] as usize).min(total as usize));
        let key = |i: usize| (f32::from_bits(entries[3 * i + 1]), entries[3 * i]);
        for i in b + 1..e.max(b + 1) {
            let (t0, i0) = key(i - 1);
            let (t1, i1) = key(i);
            if !(t0 > t1 || (t0 == t1 && i0 < i1)) {
                unsorted += 1;
                break;
            }
        }
        let want = if e > b { key(b).0 } else { lsg::EMPTY };
        if f32::from_bits(front[spec.layout.tops as usize + c]) != want {
            bad_top0 += 1;
        }
    }
    let levels = f.levels();
    let mut offset = 0usize;
    let mut bad_pyramid = 0usize;
    for k in 1..levels.len() {
        let ([w, h], [cw, ch]) = (levels[k], levels[k - 1]);
        let below = offset;
        offset += (cw * ch) as usize;
        for y in 0..h {
            for x in 0..w {
                let mut m = lsg::EMPTY;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (cx, cy) = (2 * x + dx, 2 * y + dy);
                    if cx < cw && cy < ch {
                        m = m.max(f32::from_bits(front[spec.layout.tops as usize + below + (cx + cy * cw) as usize]));
                    }
                }
                if f32::from_bits(front[spec.layout.tops as usize + offset + (x + y * w) as usize]) != m {
                    bad_pyramid += 1;
                }
            }
        }
    }
    heap.build(r, scene, spec, start);
    let again_front = heap_words(r, &heap.front, spec.layout.front_words as usize);
    let again_entries = heap_words(r, &heap.entries, lsg::ENTRY_WORDS * total as usize);
    let differing_words = |a: &[u32], b: &[u32], range: std::ops::Range<usize>| range.filter(|&i| a[i] != b[i]).count();
    let tops = spec.layout.tops as usize;
    let out = serde_json::json!({"cells": cells, "entries": total,
        "ends_not_next_begin": bad_ends.len(), "first_bad_ends": bad_ends.iter().take(8).map(|&c| [c as u32, begins[c], ends[c], next(c)]).collect::<Vec<_>>(),
        "unsorted_lists": unsorted, "level0_top_mismatch": bad_top0, "pyramid_mismatch": bad_pyramid,
        "rebuild_differing_words": {"ends": differing_words(&front, &again_front, 0..tops),
            "tops": differing_words(&front, &again_front, tops..spec.layout.sums as usize),
            "entries": differing_words(&entries, &again_entries, 0..entries.len())}});
    eprintln!("light_grid_check {} {out}", spec.name);
    out
}
/// The CPU walk (Grid::trace on a CPU build of the same frame) of the first `count` rays
/// the GPU stats walk dumped at its step limit, with N0's any-hit answer for each.
#[allow(clippy::too_many_arguments)]
fn cpu_walk(r: &Renderer, sw: &Software, heap: &GpuHeap, meshes: &[MeshData32], bvhs: &[Bvh], models: &[(usize, Mat4)], spec: &Spec, start: usize, count: u32) -> serde_json::Value {
    let world = CpuWorld::new(meshes, bvhs, models);
    let grid = world.grid_in(models, spec.frame);
    // The GPU build as a CPU grid, and its differences from the CPU build.
    let front = heap_words(r, &heap.front, spec.layout.front_words as usize);
    let words = heap_words(r, &heap.entries, lsg::ENTRY_WORDS * spec.layout.capacity as usize);
    let mut tops = vec![];
    let mut offset = spec.layout.tops as usize;
    for [w, h] in &grid.levels {
        let n = (w * h) as usize;
        tops.push(front[offset..offset + n].iter().map(|b| f32::from_bits(*b)).collect::<Vec<_>>());
        offset += n;
    }
    let cells = grid.ends.len();
    let gpu = Grid {
        frame: spec.frame,
        levels: grid.levels.clone(),
        tops,
        ends: front[..cells].to_vec(),
        entries: words.chunks_exact(3).map(|e| lsg::Entry { id: e[0], top: f32::from_bits(e[1]), bbox: e[2] }).collect(),
        stats: grid.stats,
    };
    let length = |g: &Grid, c: usize| g.ends[c] - if c == 0 { 0 } else { g.ends[c - 1] };
    let top_far = (0..cells).filter(|&c| (gpu.tops[0][c] - grid.tops[0][c]).abs() > 1e-3).count();
    let length_diff = (0..cells).filter(|&c| length(&gpu, c) != length(&grid, c)).count();
    let pyramid_far: usize = (1..grid.levels.len()).map(|k| (0..grid.tops[k].len()).filter(|&c| (gpu.tops[k][c] - grid.tops[k][c]).abs() > 1e-3).count()).sum();
    let words = read_words(r, &sw.dump, 0, 8 * u64::from(count));
    let (mut steps, mut long, mut fallbacks, mut agree) = (vec![], 0, 0, 0);
    let (mut gpu_steps, mut gpu_fallbacks) = (vec![], 0);
    let mut examples = vec![];
    for w in words.chunks_exact(8) {
        let v: Vec<f32> = w.iter().map(|b| f32::from_bits(*b)).collect();
        let ray = Ray { origin: Vec3::new(v[0], v[1], v[2]), direction: Vec3::new(v[4], v[5], v[6]), tmin: v[3], tmax: v[7] };
        let mut st = TraceStats::default();
        let walked = grid_blocked(&world, &grid, &ray, start, &mut st);
        let mut on_gpu = TraceStats::default();
        let walked_gpu = grid_blocked(&world, &gpu, &ray, start, &mut on_gpu);
        gpu_steps.push(u64::from(on_gpu.lookups));
        gpu_fallbacks += u32::from(walked_gpu.is_none());
        let truth = n0_blocked(&world, &ray, &mut N0Count::default());
        fallbacks += u32::from(walked.is_none());
        agree += u32::from(walked.unwrap_or(truth) == truth);
        long += u32::from(st.lookups > 64);
        steps.push(u64::from(st.lookups));
        if examples.len() < 8 {
            let f = &spec.frame;
            let d = Vec3::new(ray.direction.dot(f.u), ray.direction.dot(f.v), ray.direction.dot(f.l));
            examples.push(serde_json::json!({"ray": v, "cpu_steps": st.lookups, "cpu_lists": st.lists, "light_direction": d.to_array(),
                "light_origin": [ray.origin.dot(f.u) - f.origin.x, ray.origin.dot(f.v) - f.origin.y, ray.origin.dot(f.l)]}));
        }
    }
    let out = serde_json::json!({"rays": count, "cpu_steps": spread(&mut steps), "cpu_over_64": long, "cpu_fallbacks": fallbacks,
        "cpu_agrees_with_n0": agree, "cpu_walk_on_gpu_grid_steps": spread(&mut gpu_steps), "cpu_walk_on_gpu_grid_fallbacks": gpu_fallbacks,
        "gpu_vs_cpu_grid": {"level0_tops_beyond_1mm": top_far, "list_lengths_differ": length_diff, "upper_tops_beyond_1mm": pyramid_far},
        "examples": examples});
    eprintln!("light_grid_cpu_walk {out}");
    out
}
/// Stats-run walk bounds: steps and entries per ray.
const STATS_BOUNDS: [(&str, f64); 2] = [("LIGHT_GRID_STEP_LIMIT", 64.0), ("LIGHT_GRID_ENTRY_LIMIT", 512.0)];
/// A single dispatch must stay well under Windows' 2 s GPU timeout.
const DISPATCH_BUDGET_MS: f64 = 1000.0;
const RECORD: &str = "fn light_grid_record(steps:u32,entries:u32,tests:u32,fallback:u32) {}";
const DUMP: &str = "fn light_grid_dump(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32) {}";
/// The stats module's walk counters, in N0's mask buffer (binding 45): rays, steps,
/// entries, tests, fallbacks, the most steps of one ray, then the dump count. Rays that
/// exhaust the step limit go to N0's dump buffer (binding 46): origin and tmin, then
/// direction and tmax, at most DUMPED.
const STATS: &str = "@group(2) @binding(45) var<storage,read_write> software_masks:array<atomic<u32>>;
@group(2) @binding(46) var<storage,read_write> software_dump_rays:array<vec4<f32>>;
fn light_grid_record(steps:u32,entries:u32,tests:u32,fallback:u32) {
    atomicAdd(&software_masks[0],1u);atomicAdd(&software_masks[1],steps);atomicAdd(&software_masks[2],entries);
    atomicAdd(&software_masks[3],tests);atomicAdd(&software_masks[4],fallback);atomicMax(&software_masks[5],steps);
}";
const STATS_DUMP: &str = "fn light_grid_dump(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32) {
    let i=atomicAdd(&software_masks[6],1u);
    if i<4096u {software_dump_rays[2u*i]=vec4<f32>(origin,tmin);software_dump_rays[2u*i+1u]=vec4<f32>(direction,tmax);}
}";
const DUMPED: u32 = 4096;
const STEP: &str = "fn light_grid_step(step:u32,level:u32,at:vec2<i32>,h:f32,rise:f32,t:f32) {}";
/// One-ray step trace: the first dumped ray walked by one invocation, its steps from
/// dump vec4 16 (word 64) on (level, cell x, cell y, top; ray height, t).
const STATS_STEP: &str = "var<private> light_tracing:bool;
fn light_grid_step(step:u32,level:u32,at:vec2<i32>,h:f32,rise:f32,t:f32) {
    if light_tracing && step<256u {software_dump_rays[16u+2u*step]=vec4<f32>(f32(level),f32(at.x),f32(at.y),h);software_dump_rays[17u+2u*step]=vec4<f32>(rise,t,0.0,0.0);}
}
@compute @workgroup_size(1) fn light_grid_trace_one() {
    let a=software_dump_rays[0];let b=software_dump_rays[1];
    light_tracing=true;
    let blocked=light_grid_occluded(a.xyz,b.xyz,a.w,b.w);
    software_dump_rays[2]=vec4<f32>(select(0.0,1.0,blocked),0.0,0.0,0.0);
}";
/// A count texture through a buffer copy, waiting up to N0's 20 s map limit.
fn read_counts(r: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("light grid counts"),
        size: 4 * u64::from(size.width) * u64::from(size.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    assert_eq!(size.width * 4 % 256, 0);
    submit(r, |e| {
        e.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.width * 4), rows_per_image: Some(size.height) },
            },
            size,
        )
    });
    wait_map(r, &buffer)
}
/// GPU milliseconds of one encoded pass with begin and end timestamps.
fn time_once(r: &Renderer, f: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::ComputePassTimestampWrites<'_>)) -> f64 {
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("light grid once"), ty: wgpu::QueryType::Timestamp, count: 2 });
    let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("light grid once"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    submit(r, |e| {
        f(e, wgpu::ComputePassTimestampWrites { query_set: &queries, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) });
        e.resolve_query_set(&queries, 0..2, &resolve, 0);
    });
    let w = u32s(&read_buffer(r, &resolve));
    let stamp = |i: usize| u64::from(w[2 * i]) | u64::from(w[2 * i + 1]) << 32;
    (stamp(1) - stamp(0)) as f64 * f64::from(r.queue.get_timestamp_period()) * 1e-6
}
fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
}
/// Times every build stage of the loaded grid `reps` times after `warmup`: medians per
/// stage and of the whole rebuild.
fn time_build(r: &Renderer, heap: &GpuHeap, frame: &Frame, warmup: usize, reps: usize) -> serde_json::Value {
    let n = BUILD_STAGES.len() as u32;
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("light grid build timing"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * n,
    });
    let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("light grid build stamps"),
        size: 16 * u64::from(n),
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let period = f64::from(r.queue.get_timestamp_period()) * 1e-6;
    let mut samples = vec![vec![]; BUILD_STAGES.len() + 1];
    for rep in 0..warmup + reps {
        submit(r, |e| {
            heap.encode(e, frame, BUILD_STAGES.len(), Some(&queries));
            e.resolve_query_set(&queries, 0..2 * n, &resolve, 0);
        });
        let w = u32s(&read_buffer(r, &resolve));
        let stamp = |i: usize| u64::from(w[2 * i]) | u64::from(w[2 * i + 1]) << 32;
        if rep >= warmup {
            for s in 0..BUILD_STAGES.len() {
                samples[s].push((stamp(2 * s + 1) - stamp(2 * s)) as f64 * period);
            }
            samples[BUILD_STAGES.len()].push((stamp(2 * BUILD_STAGES.len() - 1) - stamp(0)) as f64 * period);
        }
    }
    let mut out = serde_json::Map::new();
    for (s, name) in BUILD_STAGES.iter().chain(&["total"]).enumerate() {
        out.insert((*name).into(), serde_json::json!({"median_ms": median(&samples[s]), "samples_ms": samples[s]}));
    }
    serde_json::Value::Object(out)
}
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(x, y)| x != y).count()
}

/// The grid query pipelines: 8-ray and Exact kernels, and their bounded stats versions.
struct Queries {
    shadows: wgpu::ComputePipeline,
    exact: wgpu::ComputePipeline,
    stats: [wgpu::ComputePipeline; 2],
    trace_one: wgpu::ComputePipeline,
}

#[test]
#[ignore = "GPU lock, 4 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_light_grid_probe() {
    let manifest = manifest();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let write = |summary: &serde_json::Map<String, serde_json::Value>, name: &str| {
        fs::write(out.join(name), serde_json::to_vec_pretty(summary).unwrap()).unwrap();
    };
    if manifest["cpu_only"].as_bool().unwrap_or(false) {
        let summary = cpu_replay(&manifest);
        fs::write(out.join("replay.json"), serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
        assert_eq!(summary["stop_rule"]["mismatches_all_runs"], 0, "grid and N0 differ; see replay.json");
        return;
    }
    let started = Instant::now();
    let mut summary = serde_json::Map::new();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let mut r = renderer(validate);
    summary.insert("validation".into(), validate.into());
    // Upload first, then the CPU BVHs, so their memory peaks do not overlap.
    let mut quarry = decode_quarry(&manifest);
    let source = quarry.source.take().unwrap();
    let (mut asset, cameras) = upload_quarry(&manifest, &mut r, source);
    let (bvhs, bvh_build) = build_all(&quarry.meshes, 4, 16.0);
    summary.insert("bvh_build".into(), bvh_build);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    traced_frame(&mut r, &color, &cameras.frames[0].camera().unwrap(), &mut asset, true);
    r.test_drain("light grid first frame");
    let lighting = lighting_settings::parse(&fs::read_to_string(manifest_path(&manifest, "lighting")).unwrap()).unwrap();
    let l = sun(&lighting);
    let (models, scene) = {
        let draws = asset.draws(Visibility::Frustum);
        let mut models = vec![];
        let mut bytes = vec![];
        for (slot, d) in draws.iter().enumerate() {
            for m in &d.instances.state.models {
                models.push((slot, *m));
                bytes.extend(crate::mesh::matrix_bytes(*m));
            }
        }
        let world = CpuWorld::new(&quarry.meshes, &bvhs, &models);
        let scene = GpuScene {
            models: init(&r, &bytes, wgpu::BufferUsages::STORAGE),
            vertices: draws.iter().map(|d| (**d.mesh.vertex).clone()).collect(),
            indices: draws.iter().map(|d| (**d.mesh.index).clone()).collect(),
            placements: world.placements(&models),
            boxes: world.placed.iter().map(|p| bvh::Aabb { lo: p.lo, hi: p.hi }).collect(),
        };
        (models, scene)
    };
    let (meshes, instances) = (quarry.meshes.len(), models.len());
    let triangles: u32 = scene.placements.iter().map(|p| p.count).sum();
    summary.insert("placements".into(), serde_json::to_value(&scene.placements).unwrap());
    let start = manifest["start"].as_u64().unwrap_or(0) as u32;
    let margin = scene.placements.iter().map(|p| p.margin).fold(0.0, f32::max);
    // Grids: name, cell (m) and tilt from the sun (rad); the first is the base grid "lsg".
    let mut specs: Vec<Spec> = manifest["grids"]
        .as_array()
        .map_or(vec![("lsg".into(), 0.004, 0.0), ("lsg-a0.05".into(), 0.004, 0.05), ("lsg-a0.1".into(), 0.004, 0.1)], |a| {
            a.iter().map(|g| (g[0].as_str().unwrap().to_owned(), g[1].as_f64().unwrap() as f32, g[2].as_f64().unwrap() as f32)).collect()
        })
        .into_iter()
        .map(|(name, cell, alpha)| {
            let frame = Frame::new(tilted(l, alpha), cell, &scene.boxes, margin);
            Spec { name, cell, alpha, frame, layout: lsg::Layout::new(&frame, 0, triangles) }
        })
        .collect();
    // Count every grid in a heap without entries, then size the shared heap for the largest.
    let front_words = specs.iter().map(|s| s.layout.front_words as usize).max().unwrap();
    let levels = specs.iter().map(|s| s.frame.levels().len()).max().unwrap();
    {
        let probe = GpuHeap::new(&r, &scene, front_words, 0, levels);
        for spec in &mut specs {
            probe.load(&r, &scene, spec, start);
            submit(&r, |e| probe.encode(e, &spec.frame, 3, None));
            let total = probe.word(&r, spec.layout.total(&spec.frame));
            spec.layout = lsg::Layout::new(&spec.frame, total, triangles);
        }
    }
    let capacity = specs.iter().map(|s| s.layout.capacity).max().unwrap();
    let heap = GpuHeap::new(&r, &scene, front_words, capacity, levels);
    let mut grids = vec![];
    for spec in &specs {
        let built = Instant::now();
        heap.build(&r, &scene, spec, start);
        let info = serde_json::json!({"name": spec.name, "cell_m": spec.cell, "alpha": spec.alpha, "frame": spec.frame.summary(),
            "layout": spec.layout, "entries": spec.layout.capacity, "levels": spec.frame.levels().len(),
            "cells": spec.frame.dims[0] * spec.frame.dims[1],
            "bytes_used": 4 * (spec.layout.front_words as u64 + lsg::ENTRY_WORDS as u64 * u64::from(spec.layout.capacity)),
            "first_build_wall_ms": built.elapsed().as_secs_f64() * 1e3});
        eprintln!("light_grid_gpu {info}");
        grids.push(info);
    }
    summary.insert("grids".into(), grids.into());
    summary.insert("heap_bytes".into(), heap.bytes().into());
    if manifest["check_heap"].as_bool().unwrap_or(true) {
        let checks: Vec<_> = specs.iter().take(1).map(|s| heap_check(&r, &heap, &scene, s, start)).collect();
        summary.insert("heap_check".into(), checks.into());
        write(&summary, "summary.json");
        if manifest["check_only"].as_bool().unwrap_or(false) {
            return;
        }
    }
    let refs: Vec<&Bvh> = bvhs.iter().collect();
    let packed = bvh::pack(&refs);
    let stack = packed.stack;
    let (sw, _) = {
        let extra = heap.extra();
        let entries: Vec<_> = extra.iter().map(|(e, b)| (*e, wgpu::BindingResource::BufferArray(b))).collect();
        Software::new(&r, &asset.draws(Visibility::Frustum), &bvhs, &packed, &entries)
    };
    drop(refs);
    drop(packed);
    let soft = primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone();
    let query_source = lsg::traced_source(&soft, &bvh::constants(meshes, instances, stack), &heap.constants);
    let q = {
        let scopes = OperationScopes::new(&r.device);
        let source = &query_source;
        let m = module(&r, source);
        assert_eq!(source.matches(RECORD).count(), 1);
        assert_eq!(source.matches(DUMP).count(), 1);
        assert_eq!(source.matches(STEP).count(), 1);
        let counted = module(&r, &source.replacen(RECORD, STATS, 1).replacen(DUMP, STATS_DUMP, 1).replacen(STEP, STATS_STEP, 1));
        let bounded8: Vec<_> = STATS_BOUNDS.iter().copied().chain(EIGHT).collect();
        let q = Queries {
            shadows: compute(&r, &sw.layout, &m, "fresh_compute", &EIGHT),
            exact: compute(&r, &sw.layout, &m, "fresh_compute", &[]),
            stats: [
                compute(&r, &sw.layout, &counted, "fresh_compute", &bounded8),
                compute(&r, &sw.layout, &counted, "fresh_compute", &STATS_BOUNDS),
            ],
            trace_one: compute(&r, &sw.layout, &counted, "light_grid_trace_one", &[]),
        };
        if let Some(e) = scopes.finish() {
            panic!("light grid query pipelines failed: {e}");
        }
        q
    };
    let sided = {
        let draws = asset.draws(Visibility::Frustum);
        draws.iter().any(|d| !d.material.unwrap().double_sided)
    };
    let p = Pipelines::new(&r, &sw, meshes, instances, stack);
    let g8x4 = [1920u32.div_ceil(8), 1080u32.div_ceil(4)];
    let g2x2 = [1920u32.div_ceil(2), 1080u32.div_ceil(2)];
    let fresh = r.fresh.as_ref().unwrap().targets.as_ref().unwrap().visibility.texture.clone();
    let receivers_at = |r: &mut Renderer, asset: &mut prepared::Prepared, pose: usize| {
        let camera = cameras.frames[pose].camera().unwrap();
        traced_frame(r, &color, &camera, asset, true);
        r.test_drain("light grid pose");
        r.queue.write_buffer(&sw.frame, 0, &frame_bytes(r, &camera, [1920, 1080], sided));
        let receivers = &r.pipeline.primary.as_ref().unwrap().trace[1];
        submit(r, |e| r.encode_primary_trace(e, receivers, primary_ray_gpu::TRACE_TILE, true, None));
    };
    // First look at pose 0: bounded stats first; a full-frame kernel runs only when the
    // bounded walk shows no pathology, and Exact only when 32x the 8-ray time fits the
    // dispatch budget. `safe`: the 8-ray and Exact kernels each grid may run.
    receivers_at(&mut r, &mut asset, 0);
    let mut safe = vec![[false; 2]; specs.len()];
    let mut first = serde_json::Map::new();
    first.insert("n0/shadows8_ms".into(), time_once(&r, |e, s| sw.pass(&r, e, &p.sw_shadows, g8x4, Some(s))).into());
    for (spec, safe) in specs.iter().zip(&mut safe) {
        heap.build(&r, &scene, spec, start);
        let mut projected = f64::INFINITY;
        for (k, (kernel, groups)) in [("shadows8", g8x4), ("exact256", g2x2)].into_iter().enumerate() {
            if k == 1 && projected > DISPATCH_BUDGET_MS {
                eprintln!("light_grid_first {}/{kernel} skipped: projected {projected:.0} ms", spec.name);
                first.insert(format!("{}/{kernel}", spec.name), serde_json::json!({"skipped_projected_ms": projected}));
                break;
            }
            let bounded_ms = time_once(&r, |e, s| {
                e.clear_buffer(&sw.masks, 0, Some(64));
                sw.pass(&r, e, &q.stats[k], groups, Some(s));
            });
            let c = read_words(&r, &sw.masks, 0, 7);
            let mut line = serde_json::json!({"bounded_ms": bounded_ms, "rays": c[0], "steps": c[1], "entries": c[2],
                "tests": c[3], "fallbacks": c[4], "max_steps": c[5], "step_limited": c[6], "bounds": STATS_BOUNDS});
            if k == 0 && c[6] > 0 && manifest["check_walk"].as_bool().unwrap_or(true) {
                line["cpu_walk"] = cpu_walk(&r, &sw, &heap, &quarry.meshes, &bvhs, &models, spec, start as usize, c[6].min(DUMPED));
                // The first dumped ray, step by step on the GPU and on the CPU.
                submit(&r, |e| {
                    e.clear_buffer(&sw.dump, 256, Some(8192));
                    sw.pass(&r, e, &q.trace_one, [1, 1], None);
                });
                let w: Vec<f32> = read_words(&r, &sw.dump, 0, 64 + 8 * 256).iter().map(|b| f32::from_bits(*b)).collect();
                let gpu_steps: Vec<_> = w[64..].chunks_exact(8).take_while(|s| s[0] != 0.0 || s[1] != 0.0 || s[3] != 0.0).take(80).map(|s| [s[0], s[1], s[2], s[3], s[4], s[5]]).collect();
                let world = CpuWorld::new(&quarry.meshes, &bvhs, &models);
                let grid = world.grid_in(&models, spec.frame);
                let ray = Ray { origin: Vec3::new(w[0], w[1], w[2]), direction: Vec3::new(w[4], w[5], w[6]), tmin: w[3], tmax: w[7] };
                let mut cpu_steps = vec![];
                grid.trace(ray.origin, ray.direction, ray.tmin, ray.tmax, start as usize, &mut TraceStats::default(), Some(&mut cpu_steps), |_| false);
                eprintln!("light_grid_trace_one ray={:?} gpu_blocked={} gpu={gpu_steps:?} cpu={cpu_steps:?}", &w[..8], w[8]);
                line["trace_one"] = serde_json::json!({"ray": &w[..8], "gpu": gpu_steps, "cpu": cpu_steps});
                drop(grid);
            }
            let sane = u64::from(c[4]) * 100 < u64::from(c[0]).max(1) && bounded_ms < DISPATCH_BUDGET_MS;
            if sane {
                let pipeline = if k == 0 { &q.shadows } else { &q.exact };
                let ms = time_once(&r, |e, s| sw.pass(&r, e, pipeline, groups, Some(s)));
                line["ms"] = ms.into();
                safe[k] = true;
                projected = 32.0 * ms;
            }
            eprintln!("light_grid_first {}/{kernel} {line}", spec.name);
            first.insert(format!("{}/{kernel}", spec.name), line);
            if !sane {
                break;
            }
        }
    }
    first.insert("n0/exact256_ms".into(), time_once(&r, |e, s| sw.pass(&r, e, &p.sw_exact, g2x2, Some(s))).into());
    summary.insert("first_look".into(), serde_json::Value::Object(first));
    write(&summary, "summary.json");
    drop((quarry, bvhs));
    if manifest["first_look_only"].as_bool().unwrap_or(false) {
        return;
    }
    // Hillclimb screen on the base grid at pose 0: override variants of the query module,
    // each byte-identical to N0's counts before it is timed against hardware with the base.
    if let Some(variants) = manifest["screen"].as_array() {
        let spec = &specs[0];
        heap.build(&r, &scene, spec, start);
        receivers_at(&mut r, &mut asset, 0);
        submit(&r, |e| encode(&r, &sw, &p, Pass::SwShadows, e, None));
        let n0_8 = read_counts(&r, &sw.counts);
        submit(&r, |e| encode(&r, &sw, &p, Pass::SwExact, e, None));
        let n0_256 = read_counts(&r, &sw.counts);
        let m = module(&r, &query_source);
        let mut compiled = vec![];
        let mut identity = serde_json::Map::new();
        for v in variants {
            let name = v["name"].as_str().unwrap().to_owned();
            let constants: Vec<(String, f64)> = v["constants"].as_object().into_iter().flatten()
                .map(|(k, x)| (k.clone(), x.as_f64().unwrap())).collect();
            let refs: Vec<(&str, f64)> = constants.iter().map(|(k, x)| (k.as_str(), *x)).collect();
            let eight: Vec<_> = refs.iter().copied().chain(EIGHT).collect();
            let scopes = OperationScopes::new(&r.device);
            let pipes = [compute(&r, &sw.layout, &m, "fresh_compute", &eight), compute(&r, &sw.layout, &m, "fresh_compute", &refs)];
            if let Some(e) = scopes.finish() {
                panic!("screen variant {name} failed to compile: {e}");
            }
            // Every variant only adds exact BVH fallbacks, so one dispatch costs at most N0's.
            let first_ms = time_once(&r, |e, s| sw.pass(&r, e, &pipes[0], g8x4, Some(s)));
            submit(&r, |e| sw.pass(&r, e, &pipes[0], g8x4, None));
            let d8 = differing(&read_counts(&r, &sw.counts), &n0_8);
            submit(&r, |e| sw.pass(&r, e, &pipes[1], g2x2, None));
            let d256 = differing(&read_counts(&r, &sw.counts), &n0_256);
            let line = serde_json::json!({"constants": v["constants"], "vs_n0": {"shadows8": d8, "exact256": d256}, "first_ms": first_ms});
            eprintln!("light_grid_screen {name} {line}");
            identity.insert(name.clone(), line);
            assert!(d8 == 0 && d256 == 0, "screen variant {name} changes the counts");
            compiled.push((name, pipes));
        }
        let mut entries = vec![
            Timed { name: "base/shadows8".into(), hw: Pass::HwShadows, sw: &sw, pipeline: &q.shadows, groups: g8x4 },
            Timed { name: "base/exact256".into(), hw: Pass::HwExact, sw: &sw, pipeline: &q.exact, groups: g2x2 },
        ];
        for (name, pipes) in &compiled {
            entries.push(Timed { name: format!("{name}/shadows8"), hw: Pass::HwShadows, sw: &sw, pipeline: &pipes[0], groups: g8x4 });
            entries.push(Timed { name: format!("{name}/exact256"), hw: Pass::HwExact, sw: &sw, pipeline: &pipes[1], groups: g2x2 });
        }
        let reps = manifest["screen_reps"].as_u64().unwrap_or(12) as usize;
        let timing = time_pairs(&r, &sw, &p, &entries, 4, reps);
        eprintln!("light_grid_screen_timing {}", serde_json::json!(entries.iter().map(|e| (e.name.clone(),
            timing[e.name.as_str()]["software"]["median_ms"].clone(), timing[e.name.as_str()]["hardware"]["median_ms"].clone())).collect::<Vec<_>>()));
        summary.insert("screen".into(), serde_json::json!({"identity": identity, "timing": timing}));
        write(&summary, "summary.json");
        if manifest["screen_only"].as_bool().unwrap_or(false) {
            return;
        }
    }
    // Count identity against N0 at poses 0, 1 and return; 8 repeats at pose 0.
    let run = |r: &Renderer, safe: [bool; 2]| {
        let mut out = (vec![], vec![]);
        if safe[0] {
            submit(r, |e| sw.pass(r, e, &q.shadows, g8x4, None));
            out.0 = read_counts(r, &sw.counts);
        }
        if safe[1] {
            submit(r, |e| sw.pass(r, e, &q.exact, g2x2, None));
            out.1 = read_counts(r, &sw.counts);
        }
        out
    };
    let compare = |a: &[u8], b: &[u8]| if a.is_empty() { serde_json::Value::Null } else { differing(a, b).into() };
    let mut poses = vec![];
    let mut first_hashes: Vec<String> = vec![];
    let mut differences = 0;
    for (index, &pose) in [0usize, 1, 0].iter().enumerate() {
        receivers_at(&mut r, &mut asset, pose);
        submit(&r, |e| encode(&r, &sw, &p, Pass::HwShadows, e, None));
        let hw8 = read_tex(&r, &fresh);
        submit(&r, |e| encode(&r, &sw, &p, Pass::HwExact, e, None));
        let hw256 = read_tex(&r, &fresh);
        submit(&r, |e| encode(&r, &sw, &p, Pass::SwShadows, e, None));
        let n0_8 = read_counts(&r, &sw.counts);
        submit(&r, |e| encode(&r, &sw, &p, Pass::SwExact, e, None));
        let n0_256 = read_counts(&r, &sw.counts);
        let mut pose_out = serde_json::Map::new();
        pose_out.insert("pose".into(), pose.into());
        pose_out.insert("n0_vs_hardware".into(), serde_json::json!({"shadows8": differing(&n0_8, &hw8), "exact256": differing(&n0_256, &hw256)}));
        let mut hashes = vec![];
        for (spec, &safe) in specs.iter().zip(&safe) {
            heap.build(&r, &scene, spec, start);
            let (c8, c256) = run(&r, safe);
            let (d8, d256) = (compare(&c8, &n0_8), compare(&c256, &n0_256));
            differences += d8.as_u64().unwrap_or(0) + d256.as_u64().unwrap_or(0);
            let pair = [hash(&c8), hash(&c256)];
            if index == 0 {
                for _ in 0..8 {
                    let (a8, a256) = run(&r, safe);
                    assert_eq!([hash(&a8), hash(&a256)], pair, "{}: grid kernel repeat differs", spec.name);
                }
            }
            hashes.extend(pair.clone());
            let repeats = if index == 0 { 8 } else { 0 };
            pose_out.insert(spec.name.clone(), serde_json::json!({"vs_n0": {"shadows8": d8, "exact256": d256},
                "vs_hardware": {"shadows8": compare(&c8, &hw8), "exact256": compare(&c256, &hw256)},
                "hashes": pair, "identical_repeats": repeats}));
            eprintln!("light_grid_pose {index} {} vs_n0 8={d8} 256={d256}", spec.name);
        }
        if index == 0 {
            first_hashes = hashes.clone();
        }
        if index == 2 {
            pose_out.insert("return_identical".into(), (hashes == first_hashes).into());
        }
        poses.push(serde_json::Value::Object(pose_out));
        summary.insert("poses".into(), poses.clone().into());
        write(&summary, "summary.json");
    }
    // Rebuild timing per grid; the base grid's heap must be unchanged by ten rebuilds.
    let mut builds = serde_json::Map::new();
    let mut heap_identical = false;
    for (i, spec) in specs.iter().enumerate() {
        heap.build(&r, &scene, spec, start);
        let before = if i == 0 { heap.hash(&r, spec) } else { String::new() };
        builds.insert(spec.name.clone(), time_build(&r, &heap, &spec.frame, 2, 8));
        if i == 0 {
            heap_identical = heap.hash(&r, spec) == before;
        }
    }
    summary.insert("rebuild".into(), serde_json::json!({"timing": builds, "heap_identical": heap_identical}));
    write(&summary, "summary.json");
    // Paired timing at pose 0 against hardware: N0, then each grid built in turn.
    receivers_at(&mut r, &mut asset, 0);
    let warmup = manifest["warmup_reps"].as_u64().unwrap_or(8) as usize;
    let reps = manifest["reps"].as_u64().unwrap_or(24) as usize;
    let mut n0 = vec![Timed { name: "n0/shadows8".into(), hw: Pass::HwShadows, sw: &sw, pipeline: &p.sw_shadows, groups: g8x4 }];
    if !manifest["skip_n0_exact"].as_bool().unwrap_or(false) {
        n0.push(Timed { name: "n0/exact256".into(), hw: Pass::HwExact, sw: &sw, pipeline: &p.sw_exact, groups: g2x2 });
    }
    let mut timing = time_pairs(&r, &sw, &p, &n0, warmup, reps);
    for (spec, safe) in specs.iter().zip(&safe) {
        heap.build(&r, &scene, spec, start);
        let mut entries = vec![];
        if safe[0] {
            entries.push(Timed { name: format!("{}/shadows8", spec.name), hw: Pass::HwShadows, sw: &sw, pipeline: &q.shadows, groups: g8x4 });
        }
        if safe[1] {
            entries.push(Timed { name: format!("{}/exact256", spec.name), hw: Pass::HwExact, sw: &sw, pipeline: &q.exact, groups: g2x2 });
        }
        if entries.is_empty() {
            continue;
        }
        let t = time_pairs(&r, &sw, &p, &entries, warmup, reps);
        for e in &entries {
            timing[e.name.as_str()] = t[e.name.as_str()].clone();
        }
    }
    // NaN where the first look skipped a kernel.
    let ms = |name: &str, side: &str| timing[name][side]["median_ms"].as_f64().unwrap_or(f64::NAN);
    let interactive = ms("lsg/shadows8", "software");
    let exact = ms("lsg/exact256", "software");
    let hw_exact = ms("n0/exact256", "hardware");
    let rebuild = summary["rebuild"]["timing"]["lsg"]["total"]["median_ms"].as_f64().unwrap();
    summary.insert("timing".into(), timing.clone());
    summary.insert("stop_rules".into(), serde_json::json!({
        "count_differences_vs_n0": differences,
        "interactive_ms": interactive, "hardware_interactive_ms": ms("n0/shadows8", "hardware"),
        "n0_interactive_ms": ms("n0/shadows8", "software"),
        "verdict": if interactive.is_nan() { "not measured: the first look skipped the kernel" } else if interactive <= 0.6 { "proceed to N1 with LSG plus N1b" } else if interactive <= 1.2 { "proceed; record the frame at VSM parity or worse" } else { "above 1.2 ms: hillclimb (at most three retained steps), then stop" },
        "exact_ms": exact, "hardware_exact_ms": hw_exact, "exact_beats_hardware": exact < hw_exact,
        "exact_within_4ms": exact <= 4.0,
        "rebuild_ms": rebuild, "rebuild_above_2ms": rebuild > 2.0,
        "grids": specs.iter().map(|s| serde_json::json!({"name": s.name, "cell_m": s.cell, "alpha": s.alpha,
            "interactive_ms": ms(&format!("{}/shadows8", s.name), "software"),
            "exact_ms": ms(&format!("{}/exact256", s.name), "software"),
            "rebuild_ms": summary["rebuild"]["timing"][s.name.as_str()]["total"]["median_ms"]})).collect::<Vec<_>>(),
    }));
    summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
    write(&summary, "summary.json");
    eprintln!("light_grid_done differences={differences} interactive_ms={interactive:.4} exact_ms={exact:.3} rebuild_ms={rebuild:.3}");
    assert_eq!(differences, 0, "grid counts differ from N0; see summary.json");
}
