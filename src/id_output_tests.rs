//! Entity-ID output against a CPU ray cast (docs/design-id-output.md), on both paths: a
//! fixture of several objects (three sharing one mesh-material batch, one without an id, one
//! holding two entities by primitive range), glass, a cut-out card and a particle emitter.
//! The fixture has no skinned objects: a CPU cast cannot see a GPU-skinned pose.
use crate::engine::*;
use glam::{DVec3, DVec4};

const SIZE: [u32; 2] = [320, 180];

/// A box of half-size `h` at `c`, with per-face normals.
fn cube(c: [f32; 3], h: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
    let (mut vertices, mut indices) = (vec![], vec![]);
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut normal = [0.0; 3];
            normal[axis] = sign;
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut position = c;
                position[axis] += sign * h[axis];
                position[u] += a * h[u];
                position[v] += b * h[v];
                vertices.push(Vertex {
                    position,
                    normal,
                    uv: [(a + 1.0) * 0.5, (b + 1.0) * 0.5],
                    color: [1.0; 4],
                });
            }
            let quad = if sign > 0.0 {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            indices.extend(quad.map(|i| base + i));
        }
    }
    (vertices, indices)
}

/// World-space triangles of one object with each one's expected id, and whether they are
/// glass.
struct Shape {
    glass: bool,
    triangles: Vec<([DVec3; 3], u32)>,
}
/// The expected id of primitive `t` under `ranges`, as the engine documents it.
fn expected(ranges: &[(u32, u32, u32)], t: u32) -> u32 {
    ranges
        .iter()
        .find(|&&(first, count, _)| t >= first && t - first < count)
        .map_or(ID_UNTAGGED, |r| r.2)
}
fn shape(ranges: &[(u32, u32, u32)], glass: bool, vertices: &[Vertex], indices: &[u32], model: Mat4) -> Shape {
    let m = model.as_dmat4();
    let p = |i: u32| m.transform_point3(DVec3::from(vertices[i as usize].position.map(f64::from)));
    Shape {
        glass,
        triangles: indices
            .chunks_exact(3)
            .enumerate()
            .map(|(k, t)| ([p(t[0]), p(t[1]), p(t[2])], expected(ranges, k as u32)))
            .collect(),
    }
}

/// Distance along a unit ray to a triangle (both sides), Möller-Trumbore in f64.
fn hit(origin: DVec3, d: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 {
        return None;
    }
    let s = origin - a;
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    let t = e2.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some(t)
}

/// The CPU's ids through each pixel centre of the unjittered camera.
fn cast(shapes: &[Shape], camera: Camera) -> Vec<IdPixel> {
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    let unproject = |x: f64, y: f64, z: f64| {
        let p = inverse * DVec4::new(x, y, z, 1.0);
        p.truncate() / p.w
    };
    let mut out = Vec::with_capacity((SIZE[0] * SIZE[1]) as usize);
    for y in 0..SIZE[1] {
        for x in 0..SIZE[0] {
            let nx = (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2.0 - 1.0;
            let ny = 1.0 - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.0;
            let near = unproject(nx, ny, 1.0);
            let d = (unproject(nx, ny, 0.5) - near).normalize();
            let (mut opaque, mut glass) = ((f64::INFINITY, 0), (f64::INFINITY, 0));
            for s in shapes {
                for &(t, id) in &s.triangles {
                    if let Some(t) = hit(near, d, t) {
                        let best = if s.glass { &mut glass } else { &mut opaque };
                        if t < best.0 {
                            *best = (t, id);
                        }
                    }
                }
            }
            out.push(IdPixel {
                opaque: opaque.1,
                front: if glass.0 < opaque.0 { glass.1 } else { opaque.1 },
            });
        }
    }
    out
}

struct Fixture {
    engine: Engine,
    shapes: Vec<Shape>,
    /// Per shape, its object, and the shared cube mesh and stone material.
    objects: Vec<ObjectId>,
    unit: (MeshId, MaterialId, Vec<Vertex>, Vec<u32>),
}

fn fixture(path: RenderPath) -> Fixture {
    let adapter = headless_adapter().unwrap();
    // SAFETY: the test suite's documented experimental-feature acknowledgement.
    let experimental_ray_query =
        (path == RenderPath::HardwareRt).then(|| unsafe { ExperimentalRayQuery::acknowledge() });
    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            ..Default::default()
        },
    )
    .unwrap();
    engine.resize(SIZE[0], SIZE[1]).unwrap();
    let view = engine.view();
    engine.set_outputs(view, Outputs { ids: true }).unwrap();
    let stone = engine
        .create_material(&MaterialDesc {
            base_color: [0.6, 0.55, 0.5, 1.0],
            ..Default::default()
        })
        .unwrap();
    let glass = engine
        .create_material(&MaterialDesc {
            base_color: [0.8, 0.9, 1.0, 1.0],
            transparency: Some(Transparency::default()),
            ..Default::default()
        })
        .unwrap();
    let mut shapes = Vec::new();
    let mut objects = Vec::new();
    let mut add = |engine: &mut Engine, ranges: &[(u32, u32, u32)], glass_shape: bool, mesh: (&[Vertex], &[u32]), mid: MeshId, material, model: Mat4| {
        let object = engine.spawn(mid, material, model).unwrap();
        engine.set_object_ids(object, ranges).unwrap();
        shapes.push(shape(ranges, glass_shape, mesh.0, mesh.1, model));
        objects.push(object);
    };
    let one = |id: u32| [(0, u32::MAX, id)];
    let (gv, gi) = cube([0.0, -0.05, 0.0], [6.0, 0.05, 6.0]);
    let ground = engine.create_mesh(&MeshData { vertices: &gv, indices: &gi }).unwrap();
    add(&mut engine, &one(1), false, (&gv, &gi), ground, stone, Mat4::IDENTITY);
    // Three objects of one mesh and material share a batch; the third has no id, and the
    // first holds two entities, as a merged object does: its x and bottom faces (triangles
    // 0-5) are entity 2, its top and z faces (6-11) entity 8.
    let (cv, ci) = cube([0.0; 3], [0.5; 3]);
    let unit = engine.create_mesh(&MeshData { vertices: &cv, indices: &ci }).unwrap();
    for (ranges, at, scale) in [
        (&[(6, 6, 8), (0, 6, 2)][..], [-1.2, 0.5, 0.0], 1.0),
        (&one(3)[..], [0.2, 0.4, -0.6], 0.8),
        (&[][..], [1.3, 0.3, 0.4], 0.6),
    ] {
        let model = Mat4::from_translation(Vec3::from(at)) * Mat4::from_scale(Vec3::splat(scale));
        add(&mut engine, ranges, false, (&cv, &ci), unit, stone, model);
    }
    // Glass in front of the first cube.
    let (pv, pi) = cube([-1.2, 0.6, 1.0], [0.6, 0.5, 0.02]);
    let pane = engine.create_mesh(&MeshData { vertices: &pv, indices: &pi }).unwrap();
    add(&mut engine, &one(4), true, (&pv, &pi), pane, glass, Mat4::IDENTITY);
    // A cut-out card: a checker of holes in a vertical quad, cut to opaque geometry.
    let card = [
        ([0.4, 0.2, 1.2], [0.0, 0.0]),
        ([1.6, 0.2, 1.2], [1.0, 0.0]),
        ([1.6, 1.4, 1.2], [1.0, 1.0]),
        ([0.4, 1.4, 1.2], [0.0, 1.0]),
    ]
    .map(|(position, uv)| Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv,
        color: [1.0; 4],
    });
    let mask = AlphaMask {
        width: 4,
        height: 4,
        alpha: (0..16).map(|i| ((i % 4 + i / 4) % 2) as f32).collect(),
    };
    let cut = cutout(
        &MeshData { vertices: &card, indices: &[0, 1, 2, 0, 2, 3] },
        &mask,
        1.0,
        0.5,
        8,
    )
    .unwrap();
    let leaf = engine.create_mesh(&cut.data()).unwrap();
    add(&mut engine, &one(5), false, (&cut.vertices, &cut.indices), leaf, stone, Mat4::IDENTITY);
    // Particles are effects, never ids.
    engine
        .create_emitter(
            &EmitterDescriptor {
                position: Vec3::new(0.3, 1.2, 0.6),
                burst: 128,
                size: [0.4, 0.4],
                color: [[0.8, 0.8, 0.8, 0.9]; 2],
                ..Default::default()
            },
            None,
        )
        .unwrap();
    engine
        .set_sun(Some(Sun {
            travel: Vec3::new(-0.4, -1.0, -0.3).normalize(),
            ..Sun::default()
        }))
        .unwrap();
    Fixture {
        engine,
        shapes,
        objects,
        unit: (unit, stone, cv, ci),
    }
}

fn camera(offset: f32) -> Camera {
    Camera::look_at(
        Vec3::new(0.5 + offset, 2.2, 5.0),
        Vec3::new(offset, 0.4, 0.0),
        Vec3::Y,
        0.9,
        0.05,
        200.0,
    )
    .unwrap()
}

/// Renders one frame and reads back every id of it.
fn frame(engine: &mut Engine, camera: &Camera) -> Vec<IdPixel> {
    engine.render(camera).unwrap();
    let view = engine.view();
    let request = engine
        .request_ids(view, IdRect { x: 0, y: 0, width: SIZE[0], height: SIZE[1] })
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(r) = engine.poll().unwrap().ids.into_iter().find(|r| r.request == request) {
            return r.ids;
        }
        assert!(std::time::Instant::now() < deadline, "id readback timed out");
        std::thread::yield_now();
    }
}

/// Every pixel whose 3x3 CPU neighbourhood shows one id must report it exactly; an edge
/// pixel must report an id of its 5x5 neighbourhood (temporal reconstruction's second slot
/// comes from the jittered 3x3 samples, up to 1.5 pixels away). Returns the number of edge
/// pixels.
fn check(gpu: &[IdPixel], cpu: &[IdPixel], what: &str) -> usize {
    let (w, h) = (SIZE[0] as i64, SIZE[1] as i64);
    let mut edges = 0;
    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) as usize;
            for (layer, read) in [("opaque", (|p: &IdPixel| p.opaque) as fn(&IdPixel) -> u32), ("front", |p| p.front)] {
                let around = |r: i64| {
                    let mut near = vec![];
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let (nx, ny) = (x + dx, y + dy);
                            if (0..w).contains(&nx) && (0..h).contains(&ny) {
                                near.push(read(&cpu[(ny * w + nx) as usize]));
                            }
                        }
                    }
                    near
                };
                let near = around(1);
                let got = read(&gpu[at]);
                if near.iter().all(|&n| n == near[0]) {
                    assert_eq!(got, read(&cpu[at]), "{what}: {layer} id at ({x}, {y})");
                } else {
                    edges += 1;
                    let wide = around(2);
                    assert!(wide.contains(&got), "{what}: {layer} id {got} at edge ({x}, {y}), 5x5 neighbourhood {wide:?}");
                }
            }
        }
    }
    edges
}

fn ids_match_a_cpu_ray_cast_and_hold_still_under_jitter(path: RenderPath) {
    let Fixture {
        mut engine,
        mut shapes,
        mut objects,
        unit,
    } = fixture(path);
    let still = camera(0.0);
    let cpu = cast(&shapes, still);
    let ids: std::collections::HashSet<u32> = cpu.iter().flat_map(|p| [p.opaque, p.front]).collect();
    assert!([1, 2, 3, 4, 5, 8, ID_UNTAGGED].iter().all(|i| ids.contains(i)), "the fixture shows every id: {ids:?}");
    // 16 jittered frames of a still camera: identical ID images, each matching the cast.
    let first = frame(&mut engine, &still);
    let edges = check(&first, &cpu, "first frame");
    for k in 1..16 {
        let next = frame(&mut engine, &still);
        assert!(next == first, "frame {k} of a still camera changed {} pixels", next.iter().zip(&first).filter(|(a, b)| a != b).count());
    }
    eprintln!("{path:?}: {edges} edge pixel ids of {}", SIZE[0] * SIZE[1] * 2);
    // A slow pan (about a third of a pixel a frame): each frame matches its own cast, and
    // no pixel flips more often than its centre ray's id changes (at least once allowed).
    let mut flips = vec![0u32; first.len()];
    let mut cpu_flips = vec![0u32; first.len()];
    let (mut last, mut last_cpu) = (first, cpu);
    for k in 1..=30 {
        let moved = camera(k as f32 * 0.004);
        let gpu = frame(&mut engine, &moved);
        let cpu = cast(&shapes, moved);
        check(&gpu, &cpu, &format!("pan frame {k}"));
        for i in 0..gpu.len() {
            flips[i] += u32::from(gpu[i].opaque != last[i].opaque);
            cpu_flips[i] += u32::from(cpu[i].opaque != last_cpu[i].opaque);
        }
        (last, last_cpu) = (gpu, cpu);
    }
    let worse: Vec<usize> = (0..flips.len()).filter(|&i| flips[i] > cpu_flips[i].max(1)).collect();
    let mut kinds = std::collections::BTreeMap::new();
    for &i in &worse {
        *kinds.entry((flips[i], cpu_flips[i])).or_insert(0) += 1;
    }
    let w = SIZE[0] as usize;
    let at: Vec<(usize, usize)> = worse.iter().take(8).map(|&i| (i % w, i / w)).collect();
    assert!(
        worse.is_empty(),
        "{path:?}: {} pixels flipped more often than the edges crossing them; (gpu flips, cast flips) -> pixels {kinds:?}; first at {at:?}",
        worse.len()
    );
    // Spawning and despawning renumber placements: after a run of still frames, the very
    // next frame's ids follow the new placements (history restarts with them).
    let still = camera(0.0);
    for _ in 0..4 {
        frame(&mut engine, &still);
    }
    engine.despawn(objects.remove(2)).unwrap();
    shapes.remove(2);
    let (mesh, material, vertices, indices) = unit;
    let model = Mat4::from_translation(Vec3::new(-0.2, 0.25, 0.9)) * Mat4::from_scale(Vec3::splat(0.5));
    let object = engine.spawn(mesh, material, model).unwrap();
    engine.set_object_ids(object, &[(0, u32::MAX, 9)]).unwrap();
    shapes.push(shape(&[(0, u32::MAX, 9)], false, &vertices, &indices, model));
    let cpu = cast(&shapes, still);
    assert!(cpu.iter().any(|p| p.opaque == 9) && !cpu.iter().any(|p| p.opaque == 3));
    check(&frame(&mut engine, &still), &cpu, "after a membership change");
}

#[test]
fn ids_match_a_cpu_ray_cast_on_the_non_rt_path() {
    ids_match_a_cpu_ray_cast_and_hold_still_under_jitter(RenderPath::NonRt);
}

#[test]
fn ids_match_a_cpu_ray_cast_on_the_rt_path() {
    ids_match_a_cpu_ray_cast_and_hold_still_under_jitter(RenderPath::HardwareRt);
}

#[test]
fn ids_off_encode_nothing_and_requests_name_their_rectangle() {
    let Fixture { mut engine, .. } = fixture(RenderPath::NonRt);
    let view = engine.view();
    engine.set_outputs(view, Outputs { ids: false }).unwrap();
    engine.render(&camera(0.0)).unwrap();
    assert!(engine.view_ids(view).is_none(), "no ID target while ids are off");
    assert!(engine.request_ids(view, IdRect::pixel(0, 0)).is_err());
    engine.set_outputs(view, Outputs { ids: true }).unwrap();
    let all = frame(&mut engine, &camera(0.0));
    let rect = IdRect { x: 100, y: 60, width: 7, height: 3 };
    let request = engine.request_ids(view, rect).unwrap();
    let readback = loop {
        if let Some(r) = engine.poll().unwrap().ids.pop() {
            break r;
        }
        std::thread::yield_now();
    };
    assert_eq!((readback.request, readback.rect), (request, rect));
    for y in 0..3 {
        for x in 0..7 {
            assert_eq!(readback.at(x, y), all[((60 + y) * SIZE[0] + 100 + x) as usize]);
        }
    }
    assert!(engine.request_ids(view, IdRect { x: SIZE[0] - 1, y: 0, width: 2, height: 1 }).is_err());
    let texture = engine.view_ids(view).unwrap();
    assert_eq!((texture.width(), texture.height(), texture.depth_or_array_layers()), (SIZE[0], SIZE[1], 2));
}

/// naga validation of the ID pass and of the glass raster on the engine prelude, on the CPU.
#[test]
fn id_shaders_validate() {
    for source in [
        super::id_source(),
        super::handoff_source(),
        format!("{}\n{}", include_str!("../shaders/scene.wgsl"), super::GLASS_SHADER),
    ] {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
    }
}

/// A program of its own surface: flat base colour and its own view data, so the data modes
/// show program outputs (roughness, metalness, emission and occlusion it declares).
const VIEW_PROGRAM: &str = "
struct Surface { n:vec3<f32>, base:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface { return Surface(material_normal(input), input.material.base_color.rgb); }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> { return s.base; }
fn view_data(s:Surface, input:SurfaceInput) -> ViewData {
    return ViewData(s.base, s.n, 0.75, 0.25, vec3<f32>(0.5,0.25,0.125), 0.25);
}
";

/// The sRGB byte of a linear value, as an Rgba8UnormSrgb output stores it.
fn srgb_byte(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    255.0 * if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Per pixel, the opaque shape and triangle the centre ray hits first.
fn cast_hits(shapes: &[Shape], camera: Camera) -> Vec<Option<(usize, usize)>> {
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    let unproject = |x: f64, y: f64, z: f64| {
        let p = inverse * DVec4::new(x, y, z, 1.0);
        p.truncate() / p.w
    };
    let mut out = Vec::new();
    for y in 0..SIZE[1] {
        for x in 0..SIZE[0] {
            let nx = (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2.0 - 1.0;
            let ny = 1.0 - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.0;
            let near = unproject(nx, ny, 1.0);
            let d = (unproject(nx, ny, 0.5) - near).normalize();
            let mut best = (f64::INFINITY, None);
            for (i, s) in shapes.iter().enumerate() {
                for (k, &(t, _)) in s.triangles.iter().enumerate() {
                    if let Some(t) = hit(near, d, t)
                        && t < best.0
                    {
                        best = (t, Some((i, k)));
                    }
                }
            }
            out.push(best.1);
        }
    }
    out
}

fn view_modes_match_their_cpu_reference(path: RenderPath) {
    let adapter = headless_adapter().unwrap();
    // SAFETY: the test suite's documented experimental-feature acknowledgement.
    let experimental_ray_query =
        (path == RenderPath::HardwareRt).then(|| unsafe { ExperimentalRayQuery::acknowledge() });
    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            ..Default::default()
        },
    )
    .unwrap();
    engine.resize(SIZE[0], SIZE[1]).unwrap();
    let program = engine.create_program("view", VIEW_PROGRAM).unwrap();
    // (base colour, metallic, roughness, program); the custom program declares its own.
    let looks = [
        ([0.6f32, 0.55, 0.5], 0.0f32, 0.5f32, None),
        ([0.9, 0.2, 0.1], 1.0, 0.25, None),
        ([0.1, 0.3, 0.8], 0.4, 0.9, None),
        ([0.2, 0.7, 0.3], 0.0, 0.5, Some(program)),
    ];
    let places = [
        ([0.0, -0.05, 0.0], [6.0, 0.05, 6.0]),
        ([-1.2, 0.5, 0.0], [0.5; 3]),
        ([0.2, 0.4, -0.6], [0.4; 3]),
        ([1.2, 0.35, 0.5], [0.35; 3]),
    ];
    let mut shapes = Vec::new();
    for (&(base, metallic, roughness, program), &(c, h)) in looks.iter().zip(&places) {
        let material = engine
            .create_material(&MaterialDesc {
                base_color: [base[0], base[1], base[2], 1.0],
                metallic,
                roughness,
                program,
                ..Default::default()
            })
            .unwrap();
        let (v, i) = cube(c, h);
        let mesh = engine.create_mesh(&MeshData { vertices: &v, indices: &i }).unwrap();
        engine.spawn(mesh, material, Mat4::IDENTITY).unwrap();
        shapes.push((shape(&[], false, &v, &i, Mat4::IDENTITY), v, i));
    }
    engine
        .set_sun(Some(Sun {
            travel: Vec3::new(-0.4, -1.0, -0.3).normalize(),
            ..Sun::default()
        }))
        .unwrap();
    let view = engine.view();
    let camera = camera(0.0);
    let only: Vec<Shape> = shapes
        .iter()
        .map(|(s, ..)| Shape { glass: false, triangles: s.triangles.clone() })
        .collect();
    let hits = cast_hits(&only, camera);
    // Interior pixels: the 3x3 neighbourhood hits one triangle (or misses).
    let w = SIZE[0] as usize;
    let interior = |p: usize| {
        let (x, y) = (p % w, p / w);
        x > 0 && y > 0 && x + 1 < w && y + 1 < SIZE[1] as usize && {
            let all: Vec<_> = (0..9).map(|k| hits[(y + k / 3 - 1) * w + x + k % 3 - 1]).collect();
            all.iter().all(|h| *h == all[0])
        }
    };
    for mode in [
        ViewMode::BaseColor,
        ViewMode::Normal,
        ViewMode::Metallic,
        ViewMode::Roughness,
        ViewMode::Emission,
        ViewMode::Occlusion,
    ] {
        engine.set_view_mode(view, mode).unwrap();
        engine.render(&camera).unwrap();
        let image = engine.capture().unwrap();
        let mut checked = 0usize;
        for p in (0..hits.len()).filter(|&p| interior(p)) {
            let expected: [f64; 3] = match hits[p] {
                None => [0.0; 3],
                Some((s, t)) => {
                    let (base, metallic, roughness, program) = looks[s];
                    let custom = program.is_some();
                    let (_, v, i) = &shapes[s];
                    let n = DVec3::from(v[i[t * 3] as usize].normal.map(f64::from));
                    match mode {
                        ViewMode::BaseColor => base.map(f64::from),
                        ViewMode::Normal => (n * 0.5 + DVec3::splat(0.5)).to_array(),
                        ViewMode::Metallic => [if custom { 0.25 } else { f64::from(metallic) }; 3],
                        ViewMode::Roughness => [if custom { 0.75 } else { f64::from(roughness) }; 3],
                        ViewMode::Emission if custom => [0.5, 0.25, 0.125],
                        ViewMode::Emission => [0.0; 3],
                        ViewMode::Occlusion => [if custom { 0.25 } else { 1.0 }; 3],
                        _ => unreachable!(),
                    }
                }
            };
            for c in 0..3 {
                let got = f64::from(image.rgba8[p * 4 + c]);
                let want = srgb_byte(expected[c]);
                assert!(
                    (got - want).abs() <= 1.5,
                    "{path:?} {mode:?}: pixel ({}, {}) channel {c} is {got}, the reference {want:.1}",
                    p % w,
                    p / w
                );
            }
            checked += 1;
        }
        assert!(checked > hits.len() / 2, "{mode:?}: only {checked} interior pixels");
        // Data modes are not jittered: a second frame is the same image.
        engine.render(&camera).unwrap();
        assert!(engine.capture().unwrap().rgba8 == image.rgba8, "{mode:?} changed between frames");
    }
    // LightingOnly is lit (reconstructed and tone-mapped), so it has no exact reference
    // here, and white albedo under the sun saturates the tone map. It is program-independent
    // white lighting: the sunlit top faces of a standard and the custom cube read alike.
    engine.set_view_mode(view, ViewMode::LightingOnly).unwrap();
    for _ in 0..8 {
        engine.render(&camera).unwrap();
    }
    let image = engine.capture().unwrap();
    let luma = |p: usize| image.rgba8[p * 4..p * 4 + 3].iter().map(|&c| f64::from(c)).sum::<f64>();
    let face = |s: usize, normal: [f32; 3]| -> f64 {
        let picked: Vec<f64> = (0..hits.len())
            .filter(|&p| interior(p))
            .filter(|&p| {
                hits[p].is_some_and(|(i, t)| i == s && shapes[i].1[shapes[i].2[t * 3] as usize].normal == normal)
            })
            .map(luma)
            .collect();
        assert!(!picked.is_empty(), "face {normal:?} of shape {s} is not in view");
        picked.iter().sum::<f64>() / picked.len() as f64
    };
    let (standard, custom) = (face(1, [0.0, 1.0, 0.0]), face(3, [0.0, 1.0, 0.0]));
    assert!(
        (standard - custom).abs() < 0.08 * standard,
        "white lighting differs by program: {standard} vs {custom}"
    );
    // Back to Lit: the ordinary frame again.
    engine.set_view_mode(view, ViewMode::Lit).unwrap();
    engine.render(&camera).unwrap();
}

#[test]
fn view_modes_match_their_cpu_reference_on_the_non_rt_path() {
    view_modes_match_their_cpu_reference(RenderPath::NonRt);
}

#[test]
fn view_modes_match_their_cpu_reference_on_the_rt_path() {
    view_modes_match_their_cpu_reference(RenderPath::HardwareRt);
}

/// Reads a single-sample RGBA8 texture back as one u32 per pixel (r = bits 0-7), blocking.
fn read_words(engine: &Engine, texture: &wgpu::Texture) -> Vec<u32> {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = engine.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("test handoff readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = engine.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    engine.queue().submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    engine
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap();
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = y * row as usize + x * 4;
            out.push(u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()));
        }
    }
    out
}

/// The overlays' handoff: both id layers into a caller's Rgba8Unorm texture, bit-exact
/// against the readback (which the cast test checks against the CPU).
fn handoff_matches_the_readback(path: RenderPath) {
    let Fixture { mut engine, .. } = fixture(path);
    let view = engine.view();
    let ids = frame(&mut engine, &camera(0.0));
    let target = engine.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("test id handoff"),
        size: wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    for (layer, read) in [
        (IdLayer::Opaque, (|p: &IdPixel| p.opaque) as fn(&IdPixel) -> u32),
        (IdLayer::Front, |p| p.front),
    ] {
        engine.render_view_ids_to(view, &target, layer).unwrap();
        let words = read_words(&engine, &target);
        let expected: Vec<u32> = ids.iter().map(read).collect();
        let wrong: Vec<(usize, u32, u32)> = (0..words.len())
            .filter(|&i| words[i] != expected[i])
            .map(|i| (i, words[i], expected[i]))
            .collect();
        assert!(
            wrong.is_empty(),
            "{path:?} {layer:?}: {} of {} handoff texels differ from the readback; first (index, handoff, readback): {:x?}",
            wrong.len(),
            words.len(),
            &wrong[..wrong.len().min(6)]
        );
    }
    // Wrong formats and sizes are refused.
    let srgb = engine.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: target.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    assert!(engine.render_view_ids_to(view, &srgb, IdLayer::Opaque).is_err());
    // The handoff copies from the target: without COPY_SRC it is refused.
    let uncopyable = engine.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: target.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    assert!(engine.render_view_ids_to(view, &uncopyable, IdLayer::Opaque).is_err());
}

#[test]
fn handoff_matches_the_readback_on_the_non_rt_path() {
    handoff_matches_the_readback(RenderPath::NonRt);
}

#[test]
fn handoff_matches_the_readback_on_the_rt_path() {
    handoff_matches_the_readback(RenderPath::HardwareRt);
}
