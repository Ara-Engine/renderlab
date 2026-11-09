//! MP1 GPU gate (docs/m7-mp1.md), one path per process (RENDERLAB_ES1_PATH rt or nonrt):
//! traced primaries with Interactive shadows on RT, the visibility buffer with Virtual
//! shadows on non-RT. A floor and eight boxes in a row under a sun.
//! - Identity: with TR1 and the GI cache off and nothing in the traced reflection band,
//!   every pixel of a frame mixing three programs equals, bit for bit, the same pixel of
//!   the all-standard, all-toon or all-custom frame, and each program owns its boxes.
//! - Function: 2 and 8 programs with the defaults (TR1 and GI on), and a standard mirror
//!   reflecting the toon box (R1's other-program hit fallback), are finite and saved as
//!   PNGs for inspection.
use super::*;
use crate::{
    Camera, Draw, HardwareShadowOptions, Light, Lighting, Limits, Mat4, Material,
    MaterialDescriptor, MeshData, Program, ProgramDescriptor, Renderer, RendererOptions,
    ShadowRequest, Shadows, Vec3, Vertex, ViewMode,
};
use std::{
    f32::consts::TAU,
    path::{Path, PathBuf},
};

// 1080p: slot 0's list then outgrows one dispatch row (65,535 workgroups).
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// A third style: Lambert with engine visibility and a flat geometric normal, no environment.
const CUSTOM: &str = "struct Surface { albedo:vec3<f32>, n:vec3<f32> };\nfn surface(input:SurfaceInput)->Surface { return Surface(input.material.base_color.rgb*input.color.rgb,input.geometric); }\nfn shade(s:Surface,input:SurfaceInput)->vec3<f32> {\n    var lit=ambient(s.n)*0.5;\n    for(var i=0u;i<light_count();i+=1u) { let l=light_sample(i,input); lit+=l.intensity*l.attenuation*max(dot(s.n,l.direction),0.0)*light_visibility(input,l); }\n    return expose(s.albedo*lit/PI);\n}\n";

fn nonrt() -> bool {
    std::env::var("RENDERLAB_ES1_PATH").as_deref() == Ok("nonrt")
}

fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let descriptor = if nonrt() {
        let mut limits = crate::visibility_limits(wgpu::Limits::default());
        limits.max_buffer_size = a.max_buffer_size;
        limits.max_storage_buffer_binding_size = a.max_storage_buffer_binding_size;
        wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | crate::VISIBILITY_FEATURES
                | crate::VISIBILITY_RASTER_FEATURES
                | crate::VIRTUAL_FEATURES,
            required_limits: limits,
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        }
    } else {
        assert_eq!(
            std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
            Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        );
        wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | wgpu::Features::EXPERIMENTAL_RAY_QUERY
                | crate::TRACED_FEATURES,
            required_limits: crate::traced_limits(wgpu::Limits {
                max_blas_primitive_count: a.max_blas_primitive_count,
                max_blas_geometry_count: 1,
                max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
                max_acceleration_structures_per_shader_stage: 1,
                max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
                max_buffer_size: a.max_buffer_size,
                ..Default::default()
            }),
            // SAFETY: the test caller explicitly accepts wgpu's experimental contract.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        }
    };
    // CC1: opt-in native compiler timing for the stage-ablation diagnostic below.
    if std::env::var_os("RENDERLAB_CC1_PROFILE").is_some() {
        crate::shader_compiler::request_device(&adapter, &descriptor)
            .unwrap()
            .unwrap()
    } else {
        pollster::block_on(adapter.request_device(&descriptor)).unwrap()
    }
}

/// CC1 diagnostic: compile the same mixed-program kernel with one stage removed at a
/// time. No ablated pipeline renders a frame. Native SPIR-V bytes and driver time are
/// captured together, including a unique live tint to exclude the private driver cache.
#[test]
#[ignore = "CC1 shared GPU lease; RENDERLAB_CC1_PROFILE and RENDERLAB_CC1_OUT"]
fn gpu_cc1_codegen() {
    fn body(source: &str, name: &str, replacement: &str) -> String {
        let start = source.find(&format!("fn {name}(")).unwrap();
        let open = start + source[start..].find('{').unwrap();
        let mut depth = 1;
        let end = source[open + 1..]
            .char_indices()
            .find_map(|(i, c)| {
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                (depth == 0).then_some(open + 1 + i)
            })
            .unwrap();
        format!("{}{{{replacement}}}{}", &source[..open], &source[end + 1..])
    }
    let r = renderer();
    let out = PathBuf::from(std::env::var("RENDERLAB_CC1_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let program = include_str!("../examples/game_scene.wgsl");
    let source = crate::program::compose_list(
        &r.primary_sources.as_ref().unwrap().1[1],
        program,
        crate::program::Stage::Traced,
        true,
    )
    .source;
    let mut rows = Vec::new();
    let cases = std::env::var("RENDERLAB_CC1_CASES")
        .unwrap_or_else(|_| "full,no-hit-shade,no-gi,no-lights,no-decals,no-software".into());
    for (index, variant) in cases.split(',').enumerate() {
        if variant == "no-software" && !nonrt() {
            continue;
        }
        let mut edited = match variant {
            "no-hit-shade" => {
                let from = "rl_finite(shade(hs,h.input))";
                assert_eq!(source.matches(from).count(), 1);
                source.replace(from, "vec3<f32>(0.0)")
            }
            "no-gi" => body(&source, "gi_prepare", "gi_rx_valid=false;"),
            "no-lights" => body(&source, "light_count", "return 0u;"),
            "no-decals" => body(&source, "decal_inputs", "return decal_none();"),
            "no-software" => body(&source, "rs_trace", "return RsHit(tmax,vec4<u32>(0u));"),
            "full" => source.clone(),
            other => std::fs::read_to_string(
                PathBuf::from(std::env::var("RENDERLAB_CC1_VARIANTS").unwrap()).join(format!(
                    "{other}-{}.wgsl",
                    if nonrt() { "nonrt" } else { "rt" }
                )),
            )
            .unwrap(),
        };
        edited = edited.replace(
            "return exposed;",
            &format!(
                "return exposed*vec3<f32>({},1.0,1.0);",
                0.8 + f64::from(salt % 100000 + index as u32) / 1e6
            ),
        );
        std::fs::write(out.join(format!("{variant}.wgsl")), &edited).unwrap();
        let ir = naga::front::wgsl::parse_str(&edited)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&edited)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&ir)
        .unwrap();
        crate::shader_compiler::take_timings(&r.device);
        let scopes = OperationScopes::new(&r.device);
        let started = std::time::Instant::now();
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(variant),
            source: wgpu::ShaderSource::Wgsl(edited.into()),
        });
        let module_ms = started.elapsed().as_secs_f64() * 1e3;
        let started = std::time::Instant::now();
        let _pipeline = r
            .primary
            .as_ref()
            .unwrap()
            .list_pipeline(&r.device, &r.ledger, &module, 1, false, None);
        let pipeline_ms = started.elapsed().as_secs_f64() * 1e3;
        let native = crate::shader_compiler::take_timings(&r.device);
        assert!(scopes.finish().is_none(), "{variant} pipeline rejected");
        assert_eq!(native.len(), 1, "{variant}: native compilation missing");
        let row = serde_json::json!({"variant":variant, "module_ms":module_ms,"pipeline_ms":pipeline_ms,
            "native":native.iter().map(|t| serde_json::json!({"kind":t.kind,"ms":t.duration.as_secs_f64()*1e3,"bytes":t.shader_bytes,"cache_hit":t.application_cache_hit})).collect::<Vec<_>>()});
        eprintln!("CC1 {row}");
        rows.push(row);
        std::fs::write(
            out.join("codegen.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
}

fn renderer() -> Renderer {
    let (device, queue) = device();
    let source = include_str!("../shaders/scene.wgsl");
    let mut r = if nonrt() {
        Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions::default(),
            source,
        )
        .unwrap()
    } else {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions::default(),
            HardwareShadowOptions::default(),
            source,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r.set_soft_shadows(SoftShadows::Interactive);
        r
    };
    assert_eq!(
        r.primary_visibility(),
        if nonrt() {
            PrimaryVisibility::VisibilityBuffer
        } else {
            PrimaryVisibility::Traced
        }
    );
    r.set_lighting(&Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.4, -1.0, -0.3).normalize(),
            lux: [60000.0, 58000.0, 55000.0],
        }],
        environment: [3000.0, 3800.0, 5000.0],
        exposure_stops: -14.0,
    })
    .unwrap();
    r.set_shadows(&Shadows {
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![30.0],
        }],
        resolution: 2048,
        directional_disk_angle: 0.02,
        ..Default::default()
    })
    .unwrap();
    r
}

/// A box of half-size `h` at `c`, with per-face normals.
fn cube(c: [f32; 3], h: f32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    let mut indices = vec![];
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut n = [0.0; 3];
            n[axis] = sign;
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut p = c;
                p[axis] += sign * h;
                p[u] += a * h;
                p[v] += b * h;
                vertices.push(Vertex {
                    position: p,
                    normal: n,
                    uv: [0.0; 2],
                    color,
                });
            }
            // Counter-clockwise seen from outside.
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

fn quad(corners: [[f32; 3]; 4], normal: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
    let vertices = corners
        .map(|position| Vertex {
            position,
            normal,
            uv: [0.0; 2],
            color: [1.0; 4],
        })
        .to_vec();
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

struct Scene {
    meshes: Vec<crate::Mesh>,
    instances: Vec<crate::Instances>,
}
impl Scene {
    fn new(r: &mut Renderer) -> Self {
        let mut shapes = vec![quad(
            [
                [-10.0, 0.0, -10.0],
                [-10.0, 0.0, 10.0],
                [10.0, 0.0, 10.0],
                [10.0, 0.0, -10.0],
            ],
            [0.0, 1.0, 0.0],
        )];
        for i in 0..8 {
            let hue = i as f32 / 8.0;
            let color = [
                0.5 + 0.45 * (TAU * hue).cos(),
                0.5 + 0.45 * (TAU * (hue + 0.33)).cos(),
                0.5 + 0.45 * (TAU * (hue + 0.67)).cos(),
                1.0,
            ];
            shapes.push(cube([-3.5 + i as f32, 0.4, 0.0], 0.35, color));
        }
        // A standing mirror behind the row, facing the camera (+z).
        shapes.push(quad(
            [
                [-4.5, 0.0, -1.5],
                [4.5, 0.0, -1.5],
                [4.5, 2.5, -1.5],
                [-4.5, 2.5, -1.5],
            ],
            [0.0, 0.0, 1.0],
        ));
        let mut meshes = vec![];
        let mut instances = vec![];
        for (vertices, indices) in &shapes {
            meshes.push(loop {
                if let Some(m) = r.try_upload_mesh(&MeshData { vertices, indices }).unwrap() {
                    break m;
                }
                r.test_drain("MP1 mesh");
            });
            instances.push(loop {
                if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                    break i;
                }
                r.test_drain("MP1 instances");
            });
        }
        Self { meshes, instances }
    }
}

fn material(r: &mut Renderer, program: &Program, roughness: f32, metallic: f32) -> Material {
    loop {
        if let Some(m) = r
            .try_create_material(&MaterialDescriptor {
                metallic,
                roughness,
                double_sided: true,
                program: Some(program),
                ..Default::default()
            })
            .unwrap()
        {
            break m;
        }
        r.test_drain("MP1 material");
    }
}

/// Renders `frames` frames of `materials` (floor, eight boxes, then the mirror when
/// `mirror`) and returns the HDR bytes and the presented RGBA8 image.
fn render(
    r: &mut Renderer,
    scene: &mut Scene,
    materials: &[&Material],
    mirror: bool,
    frames: usize,
) -> (Vec<u8>, Vec<u8>) {
    let target = crate::tests::target(r.device(), WIDTH, HEIGHT);
    let camera = Camera::look_at(
        Vec3::new(0.0, 2.2, 5.5),
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::Y,
        0.8,
        0.01,
        100.0,
    )
    .unwrap();
    let count = if mirror { 10 } else { 9 };
    for _ in 0..frames {
        let mut draws: Vec<_> = scene
            .meshes
            .iter()
            .zip(scene.instances.iter_mut())
            .zip(materials)
            .take(count)
            .map(|((mesh, instances), m)| Draw::new(mesh, instances).with_material(m))
            .collect();
        while r
            .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            r.test_drain("MP1 admission");
        }
        r.test_drain("MP1 frame");
    }
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("MP1 capture");
    let hdr = capture.try_read().unwrap().unwrap();
    (hdr, read_rgba(r, &target))
}

fn read_rgba(r: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let tight = texture.width() * 4;
    let row = tight.div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("MP1 readback"),
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    r.queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    r.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    buffer
        .slice(..)
        .get_mapped_range()
        .unwrap()
        .chunks_exact(row as usize)
        .flat_map(|r| r[..tight as usize].to_vec())
        .collect()
}

fn save_png(path: &Path, rgba: &[u8]) {
    let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

fn finite(hdr: &[u8]) -> bool {
    hdr.chunks_exact(2)
        .all(|b| crate::decode_f16(u16::from_le_bytes([b[0], b[1]])).is_finite())
}

#[test]
#[ignore = "GPU lease; RENDERLAB_ES1_PATH, RENDERLAB_ES1_OUT"]
fn gpu_mp1_mixed_programs() {
    let out = PathBuf::from(std::env::var("RENDERLAB_ES1_OUT").unwrap()).join(if nonrt() {
        "mp1-nonrt"
    } else {
        "mp1-rt"
    });
    std::fs::create_dir_all(&out).unwrap();
    let mut r = renderer();
    let mut scene = Scene::new(&mut r);
    let toon = r
        .try_create_program(&ProgramDescriptor {
            label: "toon",
            source: include_str!("../shaders/programs/toon.wgsl"),
        })
        .unwrap();
    let custom = r
        .try_create_program(&ProgramDescriptor {
            label: "custom",
            source: CUSTOM,
        })
        .unwrap();
    // Eight programs: standard, toon, custom and five toon variants (distinct sources).
    let mut eight = vec![r.standard_program().clone(), toon.clone(), custom.clone()];
    for k in 0..5 {
        let source = include_str!("../shaders/programs/toon.wgsl").replace(
            "struct Surface",
            &format!("const VARIANT_TINT:f32={}.0;\nstruct Surface", k + 2),
        );
        eight.push(
            r.try_create_program(&ProgramDescriptor {
                label: "toon variant",
                source: &source,
            })
            .unwrap(),
        );
    }
    // Floor and boxes stay out of the traced reflection band (roughness >= 0.5).
    let all = |r: &mut Renderer, p: &Program| -> Vec<Material> {
        (0..9).map(|_| material(r, p, 0.6, 0.0)).collect()
    };
    let standard = r.standard_program().clone();
    let configs = [
        ("standard", all(&mut r, &standard)),
        ("toon", all(&mut r, &toon)),
        ("custom", all(&mut r, &custom)),
    ];
    // Mixed: floor standard; boxes standard, toon, custom in turn.
    let mixed: Vec<Material> = (0..9)
        .map(|i| {
            let p = [&standard, &toon, &custom][if i == 0 { 0 } else { (i - 1) % 3 }];
            material(&mut r, p, 0.6, 0.0)
        })
        .collect();
    r.set_reconstruction(crate::Reconstruction::off()).unwrap();
    r.set_indirect(crate::IndirectSettings::off()).unwrap();
    let mut singles = vec![];
    for (name, materials) in &configs {
        let refs: Vec<&Material> = materials.iter().collect();
        let (hdr, rgba) = render(&mut r, &mut scene, &refs, false, 3);
        save_png(&out.join(format!("identity-{name}.png")), &rgba);
        singles.push(hdr);
    }
    let refs: Vec<&Material> = mixed.iter().collect();
    let (hdr, rgba) = render(&mut r, &mut scene, &refs, false, 3);
    save_png(&out.join("identity-mixed.png"), &rgba);
    let pixels = (WIDTH * HEIGHT) as usize;
    let (mut unmatched, mut owned, mut max_error) = (0usize, [0usize; 3], 0.0f32);
    for p in 0..pixels {
        let at = p * 8..p * 8 + 8;
        let matches: Vec<bool> = singles
            .iter()
            .map(|s| s[at.clone()] == hdr[at.clone()])
            .collect();
        if !matches.iter().any(|&m| m) {
            unmatched += 1;
            for s in &singles {
                for c in 0..3 {
                    let v = |b: &[u8]| {
                        crate::decode_f16(u16::from_le_bytes([
                            b[p * 8 + 2 * c],
                            b[p * 8 + 2 * c + 1],
                        ]))
                    };
                    max_error = max_error.max((v(s) - v(&hdr)).abs() / v(s).abs().max(1e-3));
                }
            }
        }
        if matches.iter().filter(|&&m| m).count() == 1 {
            owned[matches.iter().position(|&m| m).unwrap()] += 1;
        }
    }
    let identity = serde_json::json!({"pixels": pixels, "unmatched": unmatched,
        "owned_only_by": {"standard": owned[0], "toon": owned[1], "custom": owned[2]},
        "max_relative_error_of_unmatched_vs_nearest_bound": max_error,
        "bar": "every mixed pixel bit-identical to the all-standard, all-toon or all-custom frame; TR1 and GI off"});
    eprintln!("mp1 identity {identity}");

    // Function: the defaults (TR1 and GI on), 2 and 8 programs, and the mirror.
    r.set_reconstruction(Default::default()).unwrap();
    r.set_indirect(Default::default()).unwrap();
    let two: Vec<Material> = (0..10)
        .map(|i| {
            let p = if i % 2 == 1 { &toon } else { &standard };
            material(
                &mut r,
                p,
                if i == 9 { 0.0 } else { 0.6 },
                if i == 9 { 1.0 } else { 0.0 },
            )
        })
        .collect();
    let refs: Vec<&Material> = two.iter().collect();
    let (two_hdr, rgba) = render(&mut r, &mut scene, &refs, false, 16);
    save_png(&out.join("two-programs.png"), &rgba);
    let (mirror_hdr, rgba) = render(&mut r, &mut scene, &refs, true, 16);
    save_png(&out.join("two-programs-mirror.png"), &rgba);
    let many: Vec<Material> = (0..10)
        .map(|i| {
            let p = &eight[if i == 0 { 0 } else { (i - 1) % 8 }];
            material(
                &mut r,
                p,
                if i == 9 { 0.0 } else { 0.6 },
                if i == 9 { 1.0 } else { 0.0 },
            )
        })
        .collect();
    let refs: Vec<&Material> = many.iter().collect();
    let (eight_hdr, rgba) = render(&mut r, &mut scene, &refs, true, 16);
    save_png(&out.join("eight-programs-mirror.png"), &rgba);
    let summary = serde_json::json!({"path": if nonrt() {"nonrt"} else {"rt"}, "identity": identity,
        "finite": {"two": finite(&two_hdr), "mirror": finite(&mirror_hdr), "eight": finite(&eight_hdr)},
        "pipelines_created": r.stats().pipelines.created});
    std::fs::write(
        out.join("mp1.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    assert!(finite(&two_hdr) && finite(&mirror_hdr) && finite(&eight_hdr));
    assert!(
        owned.iter().all(|&n| n > 500),
        "each program owns its boxes: {owned:?}"
    );
    assert_eq!(
        unmatched, 0,
        "mixed pixels must equal a single-program frame: {identity}"
    );
}
