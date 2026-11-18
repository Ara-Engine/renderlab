//! White-furnace fixture for indirect-light estimators: scenes whose answer is analytic, so
//! any reader of the lighting cache (the cells' own lookup, accumulation, the final gather)
//! can be scored against exact numbers rather than a reference render.
//!
//! - `grey`: a closed box of inward faces, albedo 0.5, every face emitting 0.5 cd/m^2. Every
//!   point sees walls in every direction, so radiance is uniform: L = Le / (1 - rho) = 1.
//! - `orange`: the same box with albedo (0.9, 0.4, 0.1), so per channel L = Le / (1 - rho)
//!   = (5, 0.833, 0.556). The red channel needs about 30 bounces to reach 95% of its sum, so a
//!   truncated bounce series shows here first.
//! - `slab-lit` / `slab-dark`: a thin slab (3 x 0.02 x 3 m, albedo 0.5) under a uniform sky of
//!   1 cd/m^2, with a 1.5 m emitter 0.4 m below it (20 cd/m^2 in `slab-lit`, 0 in
//!   `slab-dark`). The top face sees only the sky: L = rho * sky = 0.5 in both. Light from
//!   the emitter on the top face (`slab-lit` above `slab-dark`) is leakage. `slab-lit` also
//!   reads the underside, where the emitter's light belongs.
//!
//! The estimator is whatever the process's switches select (RENDERLAB_GI_ACCUMULATE,
//! RENDERLAB_FINAL_GATHER, ...); the test records them. Each case is its own renderer, so no
//! case inherits another's cells.
//!
//! Run (one GPU lease, every case on one path):
//!   RENDERLAB_FURNACE_PATH=rt|nort (default nort; rt needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK)
//!   RENDERLAB_FURNACE_OUT=<dir>   furnace.json and one PFM per case (optional)
//!   RENDERLAB_FURNACE_CASES=grey,orange,slab-lit,slab-dark (default all)
//!   RENDERLAB_FURNACE_FRAMES=300  frames per view; the report also samples frames 1, 4, 16, 64
//!   cargo test --lib gi_furnace -- --ignored --nocapture
use super::*;

/// Lambert with a constant emission: `scatter` carries bounce light into the cache and
/// `emission` makes the surface a light source for it (cells' outgoing radiance and the
/// emitter list). The pixel is its irradiance times albedo over pi, plus its own emission.
const FURNACE_PROGRAM: &str = "struct Parameters { glow:vec3<f32> };
struct Surface { base:vec3<f32>, n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 { n=input.geometric; }
    if input.material.double_sided!=0u && !input.source_front { n=-n; }
    return Surface(input.material.base_color.rgb*input.color.rgb,n);
}
fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return s.base*(max(dot(s.n,wi),0.0)/PI);
}
fn emission(s:Surface, input:SurfaceInput) -> vec3<f32> { return params.glow; }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    return expose(indirect_irradiance(input,s.n)*s.base/PI+params.glow);
}
";

/// One quad: centre, normal (its front side), and a tangent half-extent `u`; the other
/// half-extent is `half_v` along n x u. Wound so the front faces along `n`.
fn quad(centre: Vec3, n: Vec3, u: Vec3, half_v: f32) -> ([Vec3; 4], Vec3) {
    let v = n.cross(u).normalize() * half_v;
    (
        [centre - u - v, centre + u - v, centre + u + v, centre - u + v],
        n,
    )
}
/// The six faces of an axis-aligned box of half-extents `h`, facing out (or in).
fn box_faces(centre: Vec3, h: Vec3, inward: bool) -> Vec<([Vec3; 4], Vec3)> {
    let s = if inward { -1.0 } else { 1.0 };
    [
        (Vec3::X, Vec3::Z * h.z, h.y, h.x),
        (-Vec3::X, Vec3::Z * h.z, h.y, h.x),
        (Vec3::Y, Vec3::X * h.x, h.z, h.y),
        (-Vec3::Y, Vec3::X * h.x, h.z, h.y),
        (Vec3::Z, Vec3::X * h.x, h.y, h.z),
        (-Vec3::Z, Vec3::X * h.x, h.y, h.z),
    ]
    .into_iter()
    .map(|(axis, u, half_v, reach)| quad(centre + axis * reach, axis * s, u, half_v))
    .collect()
}

/// A surface of the scene: its faces, albedo and emission.
struct Part {
    faces: Vec<([Vec3; 4], Vec3)>,
    albedo: [f32; 3],
    glow: [f32; 3],
}
/// A view and the radiance its pixels must read: Some per channel where the answer is known.
struct FurnaceView {
    name: &'static str,
    eye: Vec3,
    target: Vec3,
    up: Vec3,
    expected: Option<[f32; 3]>,
}
struct Case {
    name: &'static str,
    sky: f32,
    parts: Vec<Part>,
    views: Vec<FurnaceView>,
}
fn cases() -> Vec<Case> {
    let room = |albedo: [f32; 3]| {
        let glow = [0.5; 3];
        Case {
            name: "",
            sky: 0.0,
            parts: vec![Part {
                faces: box_faces(Vec3::ZERO, Vec3::splat(2.0), true),
                albedo,
                glow,
            }],
            views: vec![FurnaceView {
                name: "corner",
                eye: Vec3::new(-0.3, -0.2, -0.4),
                target: Vec3::new(1.0, 0.7, 0.6),
                up: Vec3::Y,
                expected: Some(std::array::from_fn(|c| glow[c] / (1.0 - albedo[c]))),
            }],
        }
    };
    let slab = |glow: f32| Case {
        name: "",
        sky: 1.0,
        parts: vec![
            Part {
                faces: box_faces(Vec3::ZERO, Vec3::new(1.5, 0.01, 1.5), false),
                albedo: [0.5; 3],
                glow: [0.0; 3],
            },
            Part {
                faces: vec![quad(
                    Vec3::new(0.0, -0.4, 0.0),
                    Vec3::Y,
                    Vec3::X * 0.75,
                    0.75,
                )],
                albedo: [0.0; 3],
                glow: [glow; 3],
            },
        ],
        views: vec![
            FurnaceView {
                name: "top",
                eye: Vec3::new(0.0, 1.2, 0.0),
                target: Vec3::ZERO,
                up: Vec3::Z,
                expected: Some([0.5; 3]),
            },
            FurnaceView {
                name: "underside",
                eye: Vec3::new(0.0, -0.25, 0.0),
                target: Vec3::ZERO,
                up: Vec3::Z,
                expected: None,
            },
        ],
    };
    let mut out = vec![
        Case {
            name: "grey",
            ..room([0.5; 3])
        },
        Case {
            name: "orange",
            ..room([0.9, 0.4, 0.1])
        },
        Case {
            name: "slab-lit",
            ..slab(20.0)
        },
        Case {
            name: "slab-dark",
            ..slab(0.0)
        },
    ];
    if let Ok(only) = std::env::var("RENDERLAB_FURNACE_CASES") {
        out.retain(|c| only.split(',').any(|n| n.trim() == c.name));
    }
    out
}

const SIZE: [u32; 2] = [320, 180];
const FOV: f32 = 1.2;
/// Frames at which the report samples each view on the way to the last.
const SAMPLES: [u32; 4] = [1, 4, 16, 64];

/// Runs every case on the path RENDERLAB_FURNACE_PATH names and prints (and writes, with
/// RENDERLAB_FURNACE_OUT) each view's measured radiance against its answer.
#[test]
#[ignore = "GPU lease; RENDERLAB_FURNACE_PATH, RENDERLAB_FURNACE_OUT, RENDERLAB_FURNACE_CASES"]
fn gi_furnace() {
    let rt = std::env::var("RENDERLAB_FURNACE_PATH").as_deref() == Ok("rt");
    let frames: u32 = std::env::var("RENDERLAB_FURNACE_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let out = std::env::var("RENDERLAB_FURNACE_OUT").ok().map(PathBuf::from);
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).unwrap();
    }
    let switches: Json = [
        "RENDERLAB_GI_ACCUMULATE",
        "RENDERLAB_FINAL_GATHER",
        "RENDERLAB_GI_EMITTERS",
        "RENDERLAB_GI_PASS",
    ]
    .into_iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), json!(v))))
    .collect::<serde_json::Map<_, _>>()
    .into();
    let mut report = json!({"path": if rt {"rt"} else {"nort"}, "frames": frames, "switches": switches});
    for case in cases() {
        let rows = run_case(&case, rt, frames, out.as_deref());
        println!("furnace {} {}", case.name, rows);
        report[case.name] = rows;
        if let Some(dir) = &out {
            std::fs::write(
                dir.join("furnace.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
        }
    }
}

fn run_case(case: &Case, rt: bool, frames: u32, out: Option<&Path>) -> Json {
    let (device, queue) = if rt { rt_device() } else { no_rt_device() };
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    let mut r = if rt {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            Limits::default(),
            options,
            HardwareShadowOptions::default(),
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r
    } else {
        Renderer::new_with_options(device, queue, FORMAT, Limits::default(), options, SOURCE)
            .unwrap()
    };
    r.set_lighting(&Lighting {
        lights: vec![],
        environment: [case.sky; 3],
        exposure_stops: 0.0,
    })
    .unwrap();
    // Single-sample frames: the estimator's own answer, not TR1's history of it.
    r.set_reconstruction(Reconstruction::off()).unwrap();
    r.set_indirect(IndirectSettings::default()).unwrap();
    let target = color_target(&r, SIZE);
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "furnace",
            source: FURNACE_PROGRAM,
        })
        .unwrap();
    let mut meshes = vec![];
    for part in &case.parts {
        let mut vertices = vec![];
        let mut indices = vec![];
        for (corners, n) in &part.faces {
            let base = vertices.len() as u32;
            vertices.extend(corners.iter().map(|p| Vertex {
                position: p.to_array(),
                normal: n.to_array(),
                uv: [0.0; 2],
                color: [1.0; 4],
            }));
            indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
        }
        let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
        let mesh = loop {
            if let Some(mesh) = r
                .try_upload_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &indices,
                })
                .unwrap()
            {
                break mesh;
            }
            r.test_drain("furnace mesh upload");
        };
        r.set_indirect_geometry(&mesh, &positions, &indices).unwrap();
        let instances = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.test_drain("furnace instance upload");
        };
        let parameters = [("glow", crate::Value::Vec3(part.glow))];
        let material = loop {
            let descriptor = MaterialDescriptor {
                base_color: [part.albedo[0], part.albedo[1], part.albedo[2], 1.0],
                metallic: 0.0,
                roughness: 1.0,
                double_sided: true,
                program: Some(&program),
                parameters: &parameters,
                ..Default::default()
            };
            if let Some(m) = r.try_create_material(&descriptor).unwrap() {
                break m;
            }
            r.test_drain("furnace material admission");
        };
        meshes.push((mesh, instances, material));
    }
    let mut rows = json!({});
    for view in &case.views {
        let camera = Camera::look_at(view.eye, view.target, view.up, FOV, 0.01, 100.0).unwrap();
        let mut samples = json!({});
        let mut last = vec![];
        for frame in 1..=frames {
            let mut draws: Vec<Draw<'_>> = meshes
                .iter_mut()
                .map(|(m, i, material)| Draw::new(m, i).with_material(material))
                .collect();
            let until = Instant::now() + Duration::from_secs(60);
            while r
                .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                .unwrap()
                .is_none()
            {
                assert!(Instant::now() < until, "furnace frame admission");
                r.test_drain("furnace admission");
            }
            drop(draws);
            if SAMPLES.contains(&frame) || frame == frames {
                last = capture_radiance(&mut r, 0.0);
                samples[frame.to_string()] = score(&last, view.expected);
            }
        }
        if let Some(dir) = out {
            write_pfm(&dir.join(format!("{}-{}.pfm", case.name, view.name)), &last);
        }
        rows[view.name] = json!({"expected": view.expected, "frames": samples});
    }
    rows
}

/// Mean radiance per channel; with an answer, also the mean ratio per channel, the 1st and
/// 99th percentiles of the luminance ratio, and the share of pixels more than 5% off.
fn score(pixels: &[[f32; 3]], expected: Option<[f32; 3]>) -> Json {
    let n = pixels.len() as f64;
    let mean: [f64; 3] =
        std::array::from_fn(|c| pixels.iter().map(|p| f64::from(p[c])).sum::<f64>() / n);
    let round = |v: f64| (v * 1e4).round() / 1e4;
    let mut row = json!({"mean": mean.map(round)});
    if let Some(e) = expected {
        let lum = |p: [f32; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        let mut ratios: Vec<f32> = pixels.iter().map(|&p| lum(p) / lum(e)).collect();
        ratios.sort_by(f32::total_cmp);
        let at = |q: f64| f64::from(ratios[((ratios.len() - 1) as f64 * q) as usize]);
        let off = ratios.iter().filter(|r| (**r - 1.0).abs() > 0.05).count() as f64 / n;
        row["ratio"] = json!(std::array::from_fn::<f64, 3, _>(|c| round(
            mean[c] / f64::from(e[c])
        )));
        row["lum_ratio_p1_p99"] = json!([round(at(0.01)), round(at(0.99))]);
        row["off_5pct"] = json!(round(off));
    }
    row
}

/// A PFM (bottom row first) of `pixels` at SIZE.
fn write_pfm(path: &Path, pixels: &[[f32; 3]]) {
    let [w, h] = SIZE.map(|v| v as usize);
    let mut bytes = format!("PF\n{w} {h}\n-1.0\n").into_bytes();
    for y in (0..h).rev() {
        for p in &pixels[y * w..(y + 1) * w] {
            for c in p {
                bytes.extend(c.to_le_bytes());
            }
        }
    }
    std::fs::write(path, bytes).unwrap();
}
