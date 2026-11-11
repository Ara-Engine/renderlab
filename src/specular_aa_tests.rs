//! S2-glossy short screen: zero-shutter jittered film integration, plus native costs.
//! Generated inputs only. A failed population is reported, never hidden by an aggregate.
use super::*;
use crate::asset::{Sampler, TextureRole};
use crate::tests::{device, suite_renderer, target, traced_suite, validation};
use serde_json::json;
use std::fs;
use std::path::Path;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SOURCE: &str = include_str!("../shaders/scene.wgsl");

fn unfiltered() -> String {
    assert!(crate::STANDARD_PROGRAM.contains("const SPECULAR_AA:bool=false;"));
    crate::STANDARD_PROGRAM.replace(
        "const SPECULAR_AA:bool=true;",
        "const SPECULAR_AA:bool=false;",
    )
}

fn reference_source() -> String {
    let api = include_str!("../shaders/program_api.wgsl");
    let start = api.find("fn material_normal(").unwrap();
    let end = start + api[start..].find("\n}").unwrap() + 2;
    let helper = api[start..end]
        .replace("fn material_normal(", "fn reference_normal(")
        .replace(
            "sample_normal_map(input)",
            "sample_normal_map_grad(input,input.uv,input.uv_dx/16.0,input.uv_dy/16.0)",
        );
    format!(
        "{}\n{helper}",
        unfiltered().replace(
            "let n=material_normal(input);",
            "let n=reference_normal(input);"
        )
    )
}

const U1B_VARIANTS: [(&str, u32, usize); 24] = [
    ("before", 0, 1),
    ("covariance-point", 1, 2),
    ("reference", 0, 1),
    ("default", 0, 1),
    ("covariance-bilinear", 1, 3),
    ("mixture-9", 2, 3),
    ("mixture-25", 3, 3),
    ("footprint-mixture-9", 4, 3),
    ("footprint-mixture-25", 5, 3),
    ("gated-mixture", 6, 3),
    ("spatial-256", 7, 3),
    ("adaptive-tail", 8, 3),
    ("spatial-256-v3", 7, 0),
    ("adaptive-4-v3", 11, 0),
    ("adaptive-8-v3", 12, 0),
    ("variance-hybrid", 13, 3),
    ("spatial-gl64-v3", 14, 0),
    ("spatial-144-v3", 10, 0),
    ("spatial-64-v3", 9, 0),
    ("spatial-256-gl25-v3", 15, 0),
    ("variance-hybrid-gl25", 16, 3),
    ("separable-hybrid", 17, 3),
    ("spatial-256-legacy", 7, 1),
    ("spatial-256-gl25-legacy", 15, 1),
];
fn u1b_program(method: u32) -> String {
    unfiltered().replace(
        "brdf(n,v,light.direction,base,metal,rough)",
        &format!("filtered_brdf(input,light.direction,base,metal,rough,{method}u)"),
    )
}
fn u1b() -> bool {
    std::env::var_os("RENDERLAB_U1B").is_some()
}
fn variants() -> Vec<usize> {
    if u1b() {
        std::env::var("RENDERLAB_U1_VARIANTS")
            .ok()
            .map(|s| s.split(',').map(|n| n.parse::<usize>().unwrap()).collect())
            .unwrap_or_else(|| {
                vec![
                    0, 1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                ]
            })
    } else {
        vec![0, 1]
    }
}

#[test]
fn benchmark_programs_validate_before_taking_a_gpu_lease() {
    for program in [
        unfiltered(),
        reference_source(),
        u1b_program(1),
        u1b_program(6),
        u1b_program(12),
        u1b_program(13),
        u1b_program(16),
    ] {
        let composed =
            crate::program::compose(SOURCE, &program, crate::program::Stage::Raster, false);
        crate::program::analyse("U1 bench", &program, &composed).unwrap();
        for source in primary_ray_gpu::shading_sources(SOURCE, &program).unwrap() {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}

struct Patch {
    mesh: Mesh,
    instances: Instances,
    materials: Vec<Material>,
}

fn frame(
    r: &mut Renderer,
    t: &wgpu::Texture,
    c: &Camera,
    scene: &mut [Patch],
    mode: usize,
) -> FrameReport {
    let mut draws: Vec<_> = scene
        .iter_mut()
        .map(|p| Draw::new(&p.mesh, &mut p.instances).with_material(&p.materials[mode]))
        .collect();
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(report) = r.try_render_view(t, c, &mut draws, ViewMode::Lit).unwrap() {
            return report;
        }
        assert!(Instant::now() < until, "U1 frame admission");
        r.test_drain("U1 admission");
    }
}

fn hdr(r: &mut Renderer) -> Vec<f32> {
    r.test_drain("U1 HDR copy admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("U1 HDR copy");
    capture
        .try_read()
        .unwrap()
        .unwrap()
        .chunks_exact(2)
        .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
        .collect()
}

/// GPU sums stay in one crop-sized buffer; no per-subframe readbacks.
fn reference_side() -> u32 {
    let side = std::env::var("RENDERLAB_REFERENCE_SIDE")
        .ok()
        .map(|s| s.parse::<u32>().unwrap())
        .unwrap_or(16);
    assert!(side.is_power_of_two() && (8..=128).contains(&side));
    side
}

struct Average {
    pipeline: wgpu::ComputePipeline,
    sums: wgpu::Buffer,
    readback: wgpu::Buffer,
    sample: wgpu::Buffer,
    samples: u32,
}
impl Average {
    fn new(r: &Renderer, size: [u32; 2]) -> Self {
        let samples = reference_side() * reference_side();
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("U1 film average"),
            source: wgpu::ShaderSource::Wgsl(format!("@group(0) @binding(0) var image:texture_2d<f32>; @group(0) @binding(1) var<storage,read_write> sums:array<vec4<f32>>; @group(0) @binding(2) var<uniform> sample:vec4<u32>; @compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>) {{ if any(id.xy>=vec2<u32>({}u,{}u)) {{return;}} let i=id.x+id.y*{}u; let c=textureLoad(image,vec2<i32>(id.xy),0); sums[i]+=c*(1.0/{}.0); let half=select(1u,2u,(sample.x&1u)!=0u)*{}u; sums[half+i]+=c*(1.0/{}.0); }}",size[0],size[1],size[0],samples,size[0]*size[1],samples/2).into()),
        });
        let pipeline = r
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 48;
        let make = |usage| {
            r.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            samples,
            pipeline,
            sums: make(
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            ),
            readback: make(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            sample: r.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }
    fn integrate(
        &self,
        r: &mut Renderer,
        t: &wgpu::Texture,
        c: &Camera,
        scene: &mut [Patch],
    ) -> (Vec<f32>, f64, [f64; 7]) {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&self.sums, 0, None);
        r.queue.submit([encoder.finish()]);
        // Establish targets and the expected CPU camera identity, then override only
        // the reference raster projection. Eye/light/materials are not jittered.
        frame(r, t, c, scene, 2);
        let view = r
            .view
            .hdr
            .as_ref()
            .unwrap()
            .resolved
            .texture
            .create_view(&Default::default());
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.sample.as_entire_binding(),
                },
            ],
        });
        let side = reference_side();
        for sample in 0..self.samples {
            r.queue.write_buffer(&self.sample, 0, &sample.to_le_bytes());
            // Independently scrambled spatial strata; the material gradient remains fixed.
            let mut h = sample.wrapping_mul(0x9e3779b9) ^ 0x83d2e19b;
            h ^= h >> 16;
            h = h.wrapping_mul(0x7feb352d);
            h ^= h >> 15;
            let jx =
                ((sample % side) as f32 + ((h & 65535) as f32 + 0.5) / 65536.0) / side as f32 - 0.5;
            let jy =
                ((sample / side) as f32 + ((h >> 16) as f32 + 0.5) / 65536.0) / side as f32 - 0.5;
            let jitter = Mat4::from_translation(Vec3::new(
                -2.0 * jx / t.width() as f32,
                2.0 * jy / t.height() as f32,
                0.0,
            ));
            let matrix = jitter * c.matrix(t.width(), t.height());
            r.queue
                .write_buffer(&r.uniform, 0, &crate::mesh::matrix_bytes(matrix));
            frame(r, t, c, scene, 2);
            let mut encoder = r.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(t.width().div_ceil(8), t.height().div_ceil(8), 1);
            }
            r.queue.submit([encoder.finish()]);
        }
        r.camera_shadow = None; // The next production frame must restore its real matrix.
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.sums, 0, &self.readback, 0, self.sums.size());
        let done = Arc::new(OnceLock::new());
        let result = done.clone();
        encoder.map_buffer_on_submit(&self.readback, wgpu::MapMode::Read, .., move |v| {
            result.set(v).unwrap();
        });
        r.queue.submit([encoder.finish()]);
        r.test_drain("U1 averaged film");
        done.get().unwrap().as_ref().unwrap();
        let view = self.readback.get_mapped_range(..).unwrap();
        let values: Vec<f32> = view
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        drop(view);
        self.readback.unmap();
        let channels = (t.width() * t.height() * 4) as usize;
        let delta = values[channels..2 * channels]
            .iter()
            .zip(&values[2 * channels..])
            .enumerate()
            .filter(|(i, _)| i % 4 != 3)
            .map(|(_, (&a, &b))| f64::from(a - b).powi(2))
            .sum::<f64>();
        let uncertainty = (delta / ((channels / 4 * 3) as f64)).sqrt() / 2.0;
        let mut squares = [0.0; 7];
        let mut counts = [0usize; 7];
        let eye_x = c.view().inverse().w_axis.x as f64;
        let pitch = 8.0 * (0.4f64).tan() / f64::from(t.height());
        for pixel in 0..channels / 4 {
            let x = eye_x
                + (pixel as u32 % t.width()) as f64 * pitch
                + (0.5 - f64::from(t.width()) / 2.0) * pitch;
            let y = (f64::from(t.height()) / 2.0 - 0.5 - (pixel as u32 / t.width()) as f64) * pitch;
            let d = (0..3)
                .map(|ch| {
                    f64::from(
                        values[channels + 4 * pixel + ch] - values[2 * channels + 4 * pixel + ch],
                    )
                    .powi(2)
                })
                .sum::<f64>();
            for population in 0..7 {
                let x0 = -1.65 + (population % 3) as f64 * 1.1;
                let y0 = 0.65 - (population / 3) as f64 * 0.65;
                if x > x0 + 2.0 * pitch
                    && x < x0 + 1.05 - 2.0 * pitch
                    && y > y0 + 2.0 * pitch
                    && y < y0 + 0.6 - 2.0 * pitch
                {
                    squares[population] += d;
                    counts[population] += 3;
                }
            }
        }
        (
            values[..channels].to_vec(),
            uncertainty,
            std::array::from_fn(|i| (squares[i] / counts[i] as f64).sqrt() / 2.0),
        )
    }
}

fn scene(r: &mut Renderer, root: &Path) -> Vec<Patch> {
    let a = asset::decode(
        &fs::read(root.join("fixture.rla")).unwrap(),
        asset::Limits::default(),
    )
    .unwrap();
    assert_eq!(a.textures[0].role, TextureRole::NormalVarianceLinear);
    let textures: Vec<_> = a
        .textures
        .iter()
        .map(|t| {
            r.test_drain("U1 texture");
            r.try_upload_texture(t).unwrap().unwrap()
        })
        .collect();
    let before = unfiltered();
    let reference = reference_source();
    // Historical U1 can still be replayed after changing the standard default.
    let candidate = unfiltered().replace(
        "let rough=clamp(material.roughness*mr.g,0.0,1.0);",
        "let rough=filtered_roughness(clamp(material.roughness*mr.g,0.0,1.0),material_normal_variance(input)+geometric_normal_variance(input));",
    );
    let sources: Vec<String> = if u1b() {
        U1B_VARIANTS
            .iter()
            .enumerate()
            .map(|(i, (_, method, _))| match i {
                0 => before.clone(),
                2 => reference.clone(),
                3 => crate::STANDARD_PROGRAM.to_string(),
                _ => u1b_program(*method),
            })
            .collect()
    } else {
        vec![
            before.clone(),
            candidate,
            reference,
            crate::STANDARD_PROGRAM.to_string(),
        ]
    };
    let programs: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            r.test_drain("U1 program");
            r.try_create_program(&crate::ProgramDescriptor {
                label: &format!("U1 mode {i}"),
                source,
            })
            .unwrap()
        })
        .collect();
    (0..7)
        .map(|i| {
            let column = i % 3;
            let row = i / 3;
            let x0 = -1.65 + column as f32 * 1.1;
            let x1 = x0 + 1.05;
            let y0 = 0.65 - row as f32 * 0.65;
            let y1 = y0 + 0.6;
            let geometric = (3..6).contains(&i);
            let vertices = [
                ([x0, y0, 0.], [0., 0.]),
                ([x1, y0, 0.], [1., 0.]),
                ([x1, y1, 0.], [1., 1.]),
                ([x0, y1, 0.], [0., 1.]),
            ]
            .map(|(position, uv)| {
                let normal = if geometric {
                    Vec3::new((uv[0] - 0.5) * 0.6, (uv[1] - 0.5) * 0.6, 1.)
                        .normalize()
                        .to_array()
                } else {
                    [0., 0., 1.]
                };
                Vertex {
                    position,
                    normal,
                    uv,
                    color: [1.; 4],
                }
            });
            let tangents = asset::TangentStream {
                basis: asset::TangentBasis::Authored,
                values: vertices
                    .map(|v| {
                        let n = Vec3::from_array(v.normal);
                        let t = (Vec3::X - n * n.x).normalize();
                        [t.x, t.y, t.z, 1.]
                    })
                    .to_vec(),
            };
            r.test_drain("U1 mesh");
            let mesh = r
                .try_upload_mesh_with_tangents(
                    &MeshData {
                        vertices: &vertices,
                        indices: &[0, 1, 2, 0, 2, 3],
                    },
                    Some(&tangents),
                )
                .unwrap()
                .unwrap();
            r.test_drain("U1 instances");
            let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
            let roughness = [0.08, 0.15, 0.3][column];
            let materials = programs
                .iter()
                .enumerate()
                .map(|(mode, program)| {
                    r.test_drain("U1 material");
                    r.try_create_material(&MaterialDescriptor {
                        program: Some(program),
                        metallic: 1.,
                        roughness,
                        base_color: [0.7, 0.5, 0.25, 1.],
                        normal_texture: (i < 3).then(|| TextureBinding {
                            texture: &textures[if u1b() {
                                U1B_VARIANTS[mode].2
                            } else {
                                usize::from(mode == 0 || mode == 3)
                            }],
                            sampler: Sampler::default(),
                        }),
                        double_sided: true,
                        ..Default::default()
                    })
                    .unwrap()
                    .unwrap()
                })
                .collect();
            Patch {
                mesh,
                instances,
                materials,
            }
        })
        .collect()
}

fn configure(r: &mut Renderer, track: &str, index: u32, width: u32) -> Camera {
    let offset = if track == "camera" {
        index as f32 * 0.5
    } else {
        0.
    };
    let x = offset * 4.0 * (0.4f32).tan() * 2.0 / (width as f32 * 9.0 / 16.0);
    let light_x = if track == "light" {
        index as f32 * 0.005
    } else {
        0.
    };
    r.set_lighting(&Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-light_x, -0.05, -1.),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    })
    .unwrap();
    Camera::look_at(
        Vec3::new(x, 0., 4.),
        Vec3::new(x, 0., 0.),
        Vec3::Y,
        0.8,
        0.1,
        20.,
    )
    .unwrap()
}

fn write_frame(path: &Path, values: &[f32]) {
    assert!(values.iter().all(|v| v.is_finite()));
    fs::write(
        path,
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
}

/// The no-RT device with visibility-buffer primaries (temporal reconstruction does not
/// reconstruct raster primaries): the suite's no-RT renderer plus the visibility features.
fn visibility_renderer() -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging().with_env(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: crate::visibility_limits(wgpu::Limits::default()),
        ..Default::default()
    }))
    .unwrap();
    let r = Renderer::new_with_options(
        device,
        queue,
        FORMAT,
        Limits::default(),
        RendererOptions::default(),
        SOURCE,
    )
    .unwrap();
    assert_eq!(r.primary_visibility(), PrimaryVisibility::VisibilityBuffer);
    r
}

#[test]
#[ignore = "U1 offscreen GPU lease, fixture.rla and 4/2 GiB live guard required"]
fn gpu_specular_aa_motion_screen() {
    let _validation = validation::begin(true);
    let root = std::path::PathBuf::from(std::env::var("RENDERLAB_U1_ROOT").unwrap());
    let mode = std::env::var("RENDERLAB_U1_MODE").unwrap_or_else(|_| "software".into());
    let reference = mode.starts_with("reference");
    let native = std::env::var_os("RENDERLAB_U1_NATIVE").is_some();
    let size = if native { [1920, 1080] } else { [320, 180] };
    // TR1b's hand-off mode (docs/m7-tr1b.md): the unfiltered "before" program reconstructed
    // on traced (RT suite) or visibility-buffer (no-RT) primaries.
    let reconstructed = mode.ends_with("-tr1");
    let mut r = if reconstructed && !traced_suite() {
        visibility_renderer()
    } else {
        let (device, queue) = device(true);
        suite_renderer(device, queue, FORMAT, RendererOptions::default(), SOURCE).unwrap()
    };
    let features = format!("{:?}", r.device.features());
    if !traced_suite() {
        assert!(
            !r.device
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        );
    }
    let t = target(&r.device, size[0], size[1]);
    let mut scene = scene(&mut r, &root);
    let out = root.join(&mode);
    fs::create_dir_all(&out).unwrap();
    if reconstructed {
        // Each track from cold: `warm` frames of its first index, then its indices
        // consecutively, each frame's reconstructed HDR written as variant 1.
        let warm = 256;
        let settings = crate::Reconstruction {
            enabled: true,
            ..Default::default()
        };
        for track in ["static", "camera", "light"] {
            r.set_reconstruction(crate::Reconstruction::off()).unwrap();
            r.set_reconstruction(settings).unwrap();
            for _ in 0..warm {
                let c = configure(&mut r, track, 0, size[0]);
                frame(&mut r, &t, &c, &mut scene, 0);
            }
            for index in 0..if track == "static" { 1 } else { 12 } {
                let c = configure(&mut r, track, index, size[0]);
                frame(&mut r, &t, &c, &mut scene, 0);
                let image = hdr(&mut r);
                write_frame(&out.join(format!("{track}-{index}-1.f32")), &image);
            }
        }
        fs::write(
            out.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({"size":size,"mode":mode,
            "features":features,"primary":format!("{:?}",r.primary_mode),"warm_frames":warm,
            "reconstruction":format!("{settings:?}"),"variants":["before","tr1"]}))
            .unwrap(),
        )
        .unwrap();
        assert!(r.fault.get().is_none());
        return;
    }
    let average = reference.then(|| Average::new(&r, size));
    let mut timings = vec![];
    let mut uncertainty = vec![];
    // The first native screen exposed idle/boost transition outliers (up to 8x).
    // Warm a representative busy shader before measuring the small static batch;
    // keep the original eight measured repeats and retain the first run separately.
    if native && u1b() {
        let c = configure(&mut r, "static", 0, size[0]);
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            frame(&mut r, &t, &c, &mut scene, 10);
            let _ = r.poll().unwrap();
        }
        r.test_drain("U1 native clock warmup");
        let _ = r.poll().unwrap();
    }
    for track in ["static", "camera", "light"] {
        let count = if track == "static" { 1 } else { 12 };
        for index in 0..if native { 0 } else { count } {
            let c = configure(&mut r, track, index, size[0]);
            if let Some(average) = &average {
                let (image, error, population_errors) =
                    average.integrate(&mut r, &t, &c, &mut scene);
                uncertainty.push(
                    json!({"track":track,"frame":index,"half_difference_rms_over_two":error,"population_half_difference_rms_over_two":population_errors}),
                );
                write_frame(&out.join(format!("{track}-{index}.f32")), &image);
            } else {
                let expected_mode = 0;
                let mut expected = None;
                for variant in variants() {
                    frame(&mut r, &t, &c, &mut scene, variant);
                    let image = hdr(&mut r);
                    write_frame(&out.join(format!("{track}-{index}-{variant}.f32")), &image);
                    if variant == expected_mode {
                        expected = Some(image);
                    }
                }
                if expected.is_none() {
                    frame(&mut r, &t, &c, &mut scene, expected_mode);
                    expected = Some(hdr(&mut r));
                }
                frame(&mut r, &t, &c, &mut scene, 3);
                let actual = hdr(&mut r);
                write_frame(&out.join(format!("{track}-{index}-3.f32")), &actual);
                if u1b() {
                    assert_eq!(
                        actual,
                        expected.unwrap(),
                        "opt-in default must equal the unfiltered non-caller, every pixel"
                    );
                }
            }
        }
        if !reference {
            for variant in variants() {
                for _ in 0..8 {
                    let c = configure(&mut r, track, 0, size[0]);
                    frame(&mut r, &t, &c, &mut scene, variant);
                }
                r.test_drain("U1 warmup");
                let _ = r.poll().unwrap();
                for repeat in 0..8 {
                    let mut ids = vec![];
                    let mut samples = vec![];
                    for index in 0..count {
                        let c = configure(&mut r, track, index, size[0]);
                        let start = Instant::now();
                        let report = frame(&mut r, &t, &c, &mut scene, variant);
                        ids.push((report.frame, index, start.elapsed().as_secs_f64() * 1000.));
                        samples.extend(r.poll().unwrap().gpu);
                    }
                    r.test_drain("U1 timing");
                    samples.extend(r.poll().unwrap().gpu);
                    for (id, index, cpu) in ids {
                        let s = samples
                            .iter()
                            .find(|s| s.frame == id)
                            .expect("U1 timestamps missing");
                        timings.push(json!({"track":track,"variant":variant,"repeat":repeat,"frame":index,"cpu_service_ms":cpu,"shade_ms":s.mesh_pass.as_secs_f64()*1000.,"primary_ms":s.receiver_pass.map(|v|v.as_secs_f64()*1000.),"shadow_ms":s.shadow_generation.map(|v|v.as_secs_f64()*1000.),"tone_ms":s.present_pass.map(|v|v.as_secs_f64()*1000.),"span_ms":s.render_span.as_secs_f64()*1000.}));
                    }
                }
            }
        }
    }
    fs::write(out.join("manifest.json"),serde_json::to_vec_pretty(&json!({"size":size,"mode":mode,"features":features,"primary":format!("{:?}",r.primary_mode),"samples":reference_side()*reference_side(),"shutter":0,"roughness":[0.08,0.15,0.3],"default_identity_frames":if reference||native {0}else{25},"reference_uncertainty":uncertainty,"variants":if u1b(){serde_json::json!(U1B_VARIANTS.iter().map(|(name,_,_)|name).collect::<Vec<_>>())}else{serde_json::json!(["before","isotropic"])},"timings":timings})).unwrap()).unwrap();
    assert!(r.fault.get().is_none());
}
