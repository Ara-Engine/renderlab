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
            required_features: wgpu::Features::TIMESTAMP_QUERY
                | crate::RECEIVER_FEATURES
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
            required_features: wgpu::Features::TIMESTAMP_QUERY
                | crate::RECEIVER_FEATURES
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
            // P-SHC2 compile-only boundary screen. Reuse an existing read binding
            // to model full precision deferred inputs; these pipelines never render.
            "buffered-gi" | "buffered-engine" => {
                let gi = body(
                    &source,
                    "gi_prepare",
                    r#"
                    let at=(input.pixel.y*textureDimensions(primary_hdr).x+input.pixel.x)*14u;
                    gi_rx_valid=gi_table[at].x!=0u;
                    for(var i=0u;i<9u;i+=1u) {gi_rx_sky[i]=bitcast<vec4<f32>>(gi_table[at+1u+i]).xyz;}
                    for(var i=0u;i<4u;i+=1u) {gi_rx_bounce[i]=bitcast<vec4<f32>>(gi_table[at+10u+i]).xyz;}
                "#,
                );
                if variant == "buffered-gi" {
                    gi
                } else {
                    let reflection = body(
                        &gi,
                        "rl_reflection_prepare",
                        "rl_specular=bitcast<vec4<f32>>(gi_table[(pixel.y*size.x+pixel.x)*14u]);",
                    );
                    body(
                        &reflection,
                        "light_visibility",
                        "return bitcast<f32>(gi_table[input.pixel.y*textureDimensions(primary_hdr).x+input.pixel.x][s.index&3u]);",
                    )
                }
            }
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
        // P-SHC2: the game return gained emission; never time a no-op tint again.
        let seam =
            "return min(exposed+expose(max(params.glow,vec3<f32>(0.0))),vec3<f32>(65504.0));";
        assert_eq!(edited.matches(seam).count(), 1, "fresh edit seam moved");
        edited = edited.replace(
            seam,
            &format!(
                "return min(exposed+expose(max(params.glow,vec3<f32>(0.0))),vec3<f32>(65504.0))*vec3<f32>({},1.0,1.0);",
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
    let tight = texture.width() * texture.format().block_copy_size(None).unwrap();
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

/// P-SHC: a mixed program's worker must compile its active list before publication.
/// Rendering during compilation keeps the old revision; the first new frame compiles
/// nothing. Exercise both watcher/background and synchronous API transactions.
#[test]
#[ignore = "GPU lease; RENDERLAB_ES1_PATH"]
fn gpu_shc_mixed_reload() {
    let mut r = renderer();
    r.set_reconstruction(crate::Reconstruction::off()).unwrap();
    r.set_indirect(crate::IndirectSettings::off()).unwrap();
    let mut scene = Scene::new(&mut r);
    let custom = r
        .try_create_program(&ProgramDescriptor {
            label: "reload",
            source: CUSTOM,
        })
        .unwrap();
    let standard = r.standard_program().clone();
    let materials: Vec<_> = (0..9)
        .map(|i| {
            material(
                &mut r,
                if i % 2 == 0 { &standard } else { &custom },
                0.6,
                0.0,
            )
        })
        .collect();
    let refs: Vec<_> = materials.iter().collect();
    let before = render(&mut r, &mut scene, &refs, false, 1).0;
    let created = r.pipelines.created;
    let revision = r.revision;
    let edited = CUSTOM.replace("ambient(s.n)*0.5", "ambient(s.n)*0.47");
    assert_ne!(edited, CUSTOM);
    let reload = r.start_program_reload(&custom, &edited).unwrap();
    assert!(reload.current(&r));
    assert_eq!(render(&mut r, &mut scene, &refs, false, 1).0, before);
    assert_eq!(r.revision, revision);
    assert_eq!(r.pipelines.created, created);
    r.finish_program_reload(reload).unwrap();
    assert_eq!(r.pipelines.created, created + 1);
    let after = render(&mut r, &mut scene, &refs, false, 1).0;
    assert_eq!(
        r.pipelines.created,
        created + 1,
        "first new frame must not compile a list"
    );
    assert_ne!(after, before);
    let bad = r.start_program_reload(&custom, "bad WGSL").unwrap();
    assert!(r.finish_program_reload(bad).is_err());
    assert_eq!(render(&mut r, &mut scene, &refs, false, 1).0, after);
    r.reload_program(&custom, CUSTOM).unwrap();
    let restored_created = r.pipelines.created;
    assert_eq!(render(&mut r, &mut scene, &refs, false, 1).0, before);
    assert_eq!(
        r.pipelines.created, restored_created,
        "sync reload must also install a complete list"
    );
    let pending = r.start_program_reload(&custom, &edited).unwrap();
    let shading = if nonrt() { 0 } else { 1 };
    r.ensure_lists(&[custom.id()], true, shading).unwrap();
    assert!(
        !pending.current(&r),
        "newly used list invalidates an older recipe"
    );
    pending.discard();

    assert_eq!(render(&mut r, &mut scene, &refs, false, 1).0, before);
    let created = r.pipelines.created;
    r.reload_program(&custom, CUSTOM).unwrap();
    assert_eq!(
        r.pipelines.created,
        created + 1,
        "unused warmed variants must stay lazy"
    );

    // A genuinely new, GI-capable program exercises the driver's long compile,
    // rather than certifying responsiveness with a cached or tiny shader.
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    assert_eq!(
        crate::STANDARD_PROGRAM.matches("return exposed;").count(),
        1
    );
    let source = crate::STANDARD_PROGRAM.replace(
        "return exposed;",
        &format!(
            "return exposed*vec3<f32>({},1.0,1.0);",
            0.6 + f64::from(salt) / 1e10
        ),
    );
    let out = std::env::var_os("RENDERLAB_SHC2_OUT").map(PathBuf::from);
    // P-SHC2: measure the actual file-watch publication endpoint, rather than
    // discarding the expensive result after eight old-image frames.
    let file = out
        .as_ref()
        .map(|p| p.join("watched.wgsl"))
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("renderlab-shc2-{}.wgsl", std::process::id()))
        });
    if let Some(out) = &out {
        std::fs::create_dir_all(out).unwrap();
    }
    std::fs::write(&file, crate::STANDARD_PROGRAM).unwrap();
    let mut watch = crate::program_watch::ShaderWatch::programs_only();
    watch.watch_program(&standard, &file, crate::STANDARD_PROGRAM);
    let edit_start = std::time::Instant::now();
    std::fs::write(&file, &source).unwrap();
    assert!(watch.poll_async(&mut r).is_empty());
    let mut max_frame_ms = 0.0f64;
    let mut active_frames = 0;
    let published_seconds = loop {
        let events = watch.poll_async(&mut r);
        if !events.is_empty() {
            assert_eq!(events.len(), 1);
            assert!(events[0].result.is_ok());
            break edit_start.elapsed().as_secs_f64();
        }
        assert!(
            edit_start.elapsed().as_secs() < 150,
            "reload did not publish"
        );
        active_frames += 1;
        let start = std::time::Instant::now();
        assert_eq!(render(&mut r, &mut scene, &refs, false, 1).0, before);
        max_frame_ms = max_frame_ms.max(start.elapsed().as_secs_f64() * 1e3);
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let created = r.pipelines.created;
    let updated = render(&mut r, &mut scene, &refs, false, 1).0;
    let image_seconds = edit_start.elapsed().as_secs_f64();
    assert_ne!(updated, before);
    assert_eq!(
        r.pipelines.created, created,
        "publication left first-frame compile work"
    );
    r.reload_program(&standard, &source).unwrap();
    assert_eq!(
        render(&mut r, &mut scene, &refs, false, 1).0,
        updated,
        "background output differs from synchronous output"
    );
    r.reload_program(&standard, crate::STANDARD_PROGRAM)
        .unwrap();
    assert_eq!(render(&mut r, &mut scene, &refs, false, 1).0, before);
    std::fs::remove_file(&file).unwrap();
    let row = serde_json::json!({"path":if nonrt(){"nonrt"}else{"rt"},
        "publish_seconds":published_seconds,"edit_to_readback_seconds":image_seconds,
        "old_frames":active_frames,"max_old_frame_readback_ms":max_frame_ms,
        "background_equals_sync_hdr":true,"restore_equals_baseline_hdr":true});
    if let Some(out) = out {
        std::fs::write(
            out.join("background.json"),
            serde_json::to_vec_pretty(&row).unwrap(),
        )
        .unwrap();
    }
    eprintln!("SHC2 {row}");
    assert!(
        max_frame_ms < 2000.0,
        "background compilation blocked the render thread"
    );
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

/// Exercises the last draw, mixed program lists, glass reflection slots and VSM tables.
#[test]
#[ignore = "Vulkan GPU; RENDERLAB_ES1_PATH=rt/nonrt and ray-query acknowledgement for RT"]
fn gpu_draws_above_64() {
    let mut r = renderer();
    r.set_reconstruction(crate::Reconstruction::off()).unwrap();
    r.set_indirect(crate::IndirectSettings::off()).unwrap();
    let custom = r
        .try_create_program(&ProgramDescriptor {
            label: "draw-limit visibility",
            source: &CUSTOM.replace(
                "return expose(s.albedo*lit/PI);",
                "return vec3<f32>(1.0,0.0,0.0);",
            ),
        })
        .unwrap();
    let standard = r.standard_program().clone();
    let mut shapes = vec![quad(
        [
            [-12.0, 0.0, -10.0],
            [-12.0, 0.0, 10.0],
            [12.0, 0.0, 10.0],
            [12.0, 0.0, -10.0],
        ],
        [0.0, 1.0, 0.0],
    )];
    for i in 0..150 {
        shapes.push(cube(
            [
                (i % 15) as f32 * 1.2 - 8.4,
                0.45,
                (i / 15) as f32 * 1.2 - 5.4,
            ],
            0.4,
            [1.0; 4],
        ));
    }
    shapes.push(cube([10.0, 0.7, 0.0], 0.6, [1.0; 4]));
    let mut meshes = vec![];
    let mut instances = vec![];
    let mut materials = vec![];
    for (i, (vertices, indices)) in shapes.iter().enumerate() {
        meshes.push(loop {
            if let Some(v) = r.try_upload_mesh(&MeshData { vertices, indices }).unwrap() {
                break v;
            }
            r.test_drain("limit mesh");
        });
        instances.push(loop {
            if let Some(v) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break v;
            }
            r.test_drain("limit instances");
        });
        materials.push(loop {
            if let Some(v) = r
                .try_create_material(&MaterialDescriptor {
                    program: Some(if i != 0 && i % 2 == 0 {
                        &custom
                    } else {
                        &standard
                    }),
                    double_sided: true,
                    roughness: 0.7,
                    transparency: (i == 151).then_some(crate::Transparency {
                        opacity: 0.15,
                        ior: 1.5,
                        thickness: 0.02,
                    }),
                    ..Default::default()
                })
                .unwrap()
            {
                break v;
            }
            r.test_drain("limit material");
        });
    }
    let target = crate::tests::target(r.device(), 640, 480);
    let camera = Camera::look_at(
        Vec3::new(0.0, 23.0, 14.0),
        Vec3::ZERO,
        Vec3::Y,
        0.9,
        0.01,
        100.0,
    )
    .unwrap();
    let mut capture = |count: usize| {
        for _ in 0..3 {
            let mut draws: Vec<_> = meshes
                .iter()
                .zip(instances.iter_mut())
                .zip(&materials)
                .enumerate()
                .filter(|(i, _)| *i < count || *i == 151)
                .map(|(_, ((mesh, instances), material))| {
                    Draw::new(mesh, instances).with_material(material)
                })
                .collect();
            while r
                .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                .unwrap()
                .is_none()
            {
                r.test_drain("limit admission");
            }
            r.test_drain("limit frame");
        }
        let rgba = read_rgba(&r, &target);
        let hit = read_rgba(
            &r,
            &r.primary
                .as_ref()
                .unwrap()
                .targets
                .as_ref()
                .unwrap()
                .hit
                .texture,
        );
        (rgba, hit)
    };
    let (full, hit) = capture(151);
    let mut coverage = [0usize; 152];
    for h in hit.chunks_exact(16) {
        let id = u32::from_le_bytes(h[..4].try_into().unwrap()) as usize;
        if id < coverage.len() {
            coverage[id] += 1;
        }
    }
    for group in 64..151 {
        assert!(
            coverage[group + 1] > 4,
            "group {group} missing: {} pixels",
            coverage[group + 1]
        );
    }
    // Hit records alone do not prove that a high batch reaches the right program list.
    for group in (64..151).step_by(2) {
        let red = full
            .chunks_exact(4)
            .zip(hit.chunks_exact(16))
            .filter(|(c, h)| {
                h[..4] == ((group + 1) as u32).to_le_bytes() && c[0] > 128 && c[1] < 8 && c[2] < 8
            })
            .count();
        assert!(
            red > 4,
            "custom program not shaded for group {group}: {red}"
        );
    }
    let (control, control_hit) = capture(64);
    let changed_floor = full
        .chunks_exact(4)
        .zip(control.chunks_exact(4))
        .zip(hit.chunks_exact(16).zip(control_hit.chunks_exact(16)))
        .filter(|((a, b), (h, c))| {
            h[..4] == 1u32.to_le_bytes()
                && c[..4] == 1u32.to_le_bytes()
                && a[..3]
                    .iter()
                    .zip(&b[..3])
                    .any(|(a, b)| a.saturating_add(4) < *b)
        })
        .count();
    assert!(
        changed_floor > 100,
        "high groups cast no floor shadows: {changed_floor}"
    );
    let out = PathBuf::from("target/draw-limit");
    std::fs::create_dir_all(&out).unwrap();
    let file =
        std::fs::File::create(out.join(if nonrt() { "nonrt.png" } else { "rt.png" })).unwrap();
    let mut encoder = png::Encoder::new(file, 640, 480);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&full)
        .unwrap();
    eprintln!(
        "DRAW_LIMIT path={} high_groups=87 min_coverage={} changed_floor={changed_floor}",
        if nonrt() { "NonRt" } else { "HardwareRt" },
        coverage[65..152].iter().min().unwrap()
    );
}

/// Small existing MP1 floor/eight-box scene; timestamp span, eight warmed samples.
#[test]
#[ignore = "Vulkan GPU; same acknowledgement and path selection as gpu_mp1_mixed_programs"]
fn gpu_draw_limit_small_timing() {
    let mut r = renderer();
    let mut scene = Scene::new(&mut r);
    let standard = r.standard_program().clone();
    let materials: Vec<_> = (0..9)
        .map(|_| material(&mut r, &standard, 0.6, 0.0))
        .collect();
    let target = crate::tests::target(r.device(), 640, 480);
    let camera = Camera::look_at(
        Vec3::new(0.0, 2.2, 5.5),
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::Y,
        0.8,
        0.01,
        100.0,
    )
    .unwrap();
    let mut times = vec![];
    for frame in 0..24 {
        let mut draws: Vec<_> = scene
            .meshes
            .iter()
            .zip(scene.instances.iter_mut())
            .zip(&materials)
            .take(9)
            .map(|((m, i), a)| Draw::new(m, i).with_material(a))
            .collect();
        while r
            .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            r.test_drain("timing admission");
        }
        r.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let samples = r.poll().unwrap().gpu;
        if frame >= 16 {
            times.extend(samples.iter().map(|s| s.render_span.as_secs_f64() * 1000.0));
        }
    }
    assert_eq!(times.len(), 8);
    eprintln!(
        "DRAW_TIMING limit={} nonrt={} ms={times:?}",
        crate::TRACED_DRAWS,
        nonrt()
    );
}
