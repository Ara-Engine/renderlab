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

const FILTER: &str = "filtered_roughness(clamp(material.roughness*mr.g,0.0,1.0),\n        select(0.0,material_normal_variance(input)+geometric_normal_variance(input),SPECULAR_AA))";

fn unfiltered() -> String {
    assert!(crate::STANDARD_PROGRAM.contains(FILTER));
    crate::STANDARD_PROGRAM.replace(FILTER, "clamp(material.roughness*mr.g,0.0,1.0)")
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

#[test]
fn benchmark_programs_validate_before_taking_a_gpu_lease() {
    for program in [unfiltered(), reference_source()] {
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
struct Average {
    pipeline: wgpu::ComputePipeline,
    sums: wgpu::Buffer,
    readback: wgpu::Buffer,
    sample: wgpu::Buffer,
}
impl Average {
    fn new(r: &Renderer, size: [u32; 2]) -> Self {
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("U1 film average"),
            source: wgpu::ShaderSource::Wgsl(format!("@group(0) @binding(0) var image:texture_2d<f32>; @group(0) @binding(1) var<storage,read_write> sums:array<vec4<f32>>; @group(0) @binding(2) var<uniform> sample:vec4<u32>; @compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>) {{ if any(id.xy>=vec2<u32>({}u,{}u)) {{return;}} let i=id.x+id.y*{}u; let c=textureLoad(image,vec2<i32>(id.xy),0); sums[i]+=c*(1.0/256.0); let half=select(1u,2u,(sample.x&1u)!=0u)*{}u; sums[half+i]+=c*(1.0/128.0); }}",size[0],size[1],size[0],size[0]*size[1]).into()),
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
    ) -> (Vec<f32>, f64) {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&self.sums, 0, None);
        r.queue.submit([encoder.finish()]);
        // Establish targets and the expected CPU camera identity, then override only
        // the reference raster projection. Eye/light/materials are not jittered.
        frame(r, t, c, scene, 2);
        let view = r
            .lighting
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
        for sample in 0..256u32 {
            r.queue.write_buffer(&self.sample, 0, &sample.to_le_bytes());
            // Independently scrambled samples in the 16x16 spatial strata.
            let mut h = sample.wrapping_mul(0x9e3779b9) ^ 0x83d2e19b;
            h ^= h >> 16;
            h = h.wrapping_mul(0x7feb352d);
            h ^= h >> 15;
            let jx = ((sample % 16) as f32 + ((h & 65535) as f32 + 0.5) / 65536.0) / 16.0 - 0.5;
            let jy = ((sample / 16) as f32 + ((h >> 16) as f32 + 0.5) / 65536.0) / 16.0 - 0.5;
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
        (values[..channels].to_vec(), uncertainty)
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
    let candidate = crate::STANDARD_PROGRAM.replace(
        "const SPECULAR_AA:bool=false;",
        "const SPECULAR_AA:bool=true;",
    );
    let programs: Vec<_> = [
        before.as_str(),
        candidate.as_str(),
        reference.as_str(),
        crate::STANDARD_PROGRAM,
    ]
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
                            texture: &textures[usize::from(mode == 0 || mode == 3)],
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

#[test]
#[ignore = "U1 offscreen GPU lease, fixture.rla and 4/2 GiB live guard required"]
fn gpu_specular_aa_motion_screen() {
    let _validation = validation::begin(true);
    let root = std::path::PathBuf::from(std::env::var("RENDERLAB_U1_ROOT").unwrap());
    let mode = std::env::var("RENDERLAB_U1_MODE").unwrap_or_else(|_| "software".into());
    let reference = mode == "reference";
    let native = std::env::var_os("RENDERLAB_U1_NATIVE").is_some();
    let size = if native { [1920, 1080] } else { [320, 180] };
    let (device, queue) = device(true);
    let features = format!("{:?}", device.features());
    let mut r = suite_renderer(device, queue, FORMAT, RendererOptions::default(), SOURCE).unwrap();
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
    let average = reference.then(|| Average::new(&r, size));
    let mut timings = vec![];
    let mut uncertainty = vec![];
    for track in ["static", "camera", "light"] {
        let count = if track == "static" { 1 } else { 12 };
        for index in 0..if native { 0 } else { count } {
            let c = configure(&mut r, track, index, size[0]);
            if let Some(average) = &average {
                let (image, error) = average.integrate(&mut r, &t, &c, &mut scene);
                uncertainty.push(
                    json!({"track":track,"frame":index,"half_difference_rms_over_two":error}),
                );
                write_frame(&out.join(format!("{track}-{index}.f32")), &image);
            } else {
                let mut before = None;
                for variant in 0..2 {
                    frame(&mut r, &t, &c, &mut scene, variant);
                    let image = hdr(&mut r);
                    write_frame(&out.join(format!("{track}-{index}-{variant}.f32")), &image);
                    if variant == 0 {
                        before = Some(image);
                    }
                }
                frame(&mut r, &t, &c, &mut scene, 3);
                assert_eq!(
                    hdr(&mut r),
                    before.unwrap(),
                    "default must equal the unfiltered non-caller, every pixel"
                );
            }
        }
        if !reference {
            for variant in 0..2 {
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
    fs::write(out.join("manifest.json"),serde_json::to_vec_pretty(&json!({"size":size,"mode":mode,"features":features,"primary":format!("{:?}",r.primary_mode),"samples":256,"shutter":0,"roughness":[0.08,0.15,0.3],"default_identity_frames":if reference||native {0}else{25},"reference_uncertainty":uncertainty,"timings":timings})).unwrap()).unwrap();
    assert!(r.fault.get().is_none());
}
