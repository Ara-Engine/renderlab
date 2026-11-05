//! Offscreen determinism reproducer. The manifest controls repetitions and shader
//! probes; summaries stay small even when a complete HDR image differs.
use super::*;
use sha2::{Digest, Sha256};

struct ArithmeticGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl ArithmeticGpu {
    fn new(validate: bool, passthrough: bool) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: if validate {
                wgpu::InstanceFlags::debugging()
            } else {
                wgpu::InstanceFlags::empty()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        eprintln!("[determinism] adapter={:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: if passthrough {
                wgpu::Features::PASSTHROUGH_SHADERS
            } else {
                wgpu::Features::empty()
            },
            ..Default::default()
        }))
        .unwrap();
        Self { device, queue }
    }

    fn read(&self, texture: &wgpu::Texture) -> Vec<u8> {
        let tight = texture.width() * 16;
        let row = tight.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arithmetic readback"),
            size: u64::from(row) * u64::from(texture.height()),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(texture.height()),
                },
            },
            texture.size(),
        );
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            callback.set(result).unwrap();
        });
        self.queue.submit([encoder.finish()]);
        let start = Instant::now();
        while done.get().is_none() {
            self.device.poll(wgpu::PollType::Poll).unwrap();
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "arithmetic readback timeout"
            );
            std::thread::yield_now();
        }
        done.get().unwrap().as_ref().unwrap();
        let view = buffer.get_mapped_range(..).unwrap();
        let bytes = view
            .chunks_exact(row as usize)
            .flat_map(|r| r[..tight as usize].iter().copied())
            .collect();
        drop(view);
        buffer.unmap();
        bytes
    }
}

fn ray_module(
    device: &wgpu::Device,
    source: &str,
    contraction: bool,
    passthrough: bool,
    out: &std::path::Path,
) -> wgpu::ShaderModule {
    if !contraction && !passthrough {
        return device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("minimal camera arithmetic"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    }
    let module = naga::front::wgsl::parse_str(source).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let mut words = naga::back::spv::write_vec(
        &module,
        &info,
        &Default::default(),
        Some(&naga::back::spv::PipelineOptions {
            shader_stage: naga::ShaderStage::Compute,
            entry_point: "ray_probe".into(),
        }),
    )
    .unwrap();
    let mut decorations = vec![];
    let mut insert = None;
    let mut at = 5;
    while at < words.len() {
        let op = words[at] & 0xffff;
        let count = (words[at] >> 16) as usize;
        assert!(count > 0);
        if op == 19 {
            insert.get_or_insert(at);
        }
        // SPIR-V OpFAdd/FSub/FMul/FDiv and vector/matrix arithmetic result IDs.
        if contraction && [129, 131, 133, 136, 142, 143, 144, 145, 146, 147, 148].contains(&op) {
            decorations.extend([0x00030047, words[at + 2], 42]);
        }
        at += count;
    }
    words.splice(insert.unwrap()..insert.unwrap(), decorations);
    fs::write(
        out.join("ray.spv"),
        words
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    // SAFETY: naga validates this fixed, bounds-checked test kernel. Only the
    // NoContraction decoration is added to floating arithmetic; validation layers
    // independently validate the emitted SPIR-V during the experiment.
    unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some("minimal arithmetic, SPIR-V control"),
            spirv: Some(words.into()),
            entry_points: vec![wgpu::PassthroughShaderEntryPoint {
                name: "ray_probe".into(),
                workgroup_size: (8, 4, 1),
            }]
            .into(),
            ..Default::default()
        })
    }
}

#[test]
#[ignore = "offscreen; GPU slot, >=6 GiB available RAM, experimental acknowledgment and manifest"]
fn gpu_ray_arithmetic_repeat() {
    let m = manifest();
    let validate = m["validate"].as_bool().unwrap_or(true);
    let _validation = validation::begin(validate);
    let r = ArithmeticGpu::new(
        validate,
        m["no_contraction"].as_bool().unwrap_or(false)
            || m["passthrough"].as_bool().unwrap_or(false),
    );
    let cameras =
        cameras::Cameras::load(std::path::Path::new(m["cameras"].as_str().unwrap())).unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let storage = m["storage"].as_bool().unwrap_or(false);
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ray probe inverse"),
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST
            | if storage {
                wgpu::BufferUsages::STORAGE
            } else {
                wgpu::BufferUsages::UNIFORM
            },
        mapped_at_creation: false,
    });
    let width = m["width"].as_u64().unwrap_or(1920) as u32;
    let height = m["height"].as_u64().unwrap_or(1080) as u32;
    let output = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ray probe bits"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Uint,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut source = include_str!("../support/determinism_ray.wgsl").to_owned();
    if m["explicit_fma"].as_bool().unwrap_or(false) {
        source = edit(
            &source,
            (
                "let ndc=(vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);",
                "let ndc=fma((vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));",
            ),
        );
        source = edit(
            &source,
            (
                "let a=inverse*vec4<f32>(ndc,1.0,1.0);",
                "let xy=fma(inverse[0],vec4<f32>(ndc.x),inverse[1]*ndc.y);\n    let a=xy+inverse[2]+inverse[3];",
            ),
        );
        source = edit(
            &source,
            (
                "let b=inverse*vec4<f32>(ndc,0.0,1.0);",
                "let b=xy+inverse[3];",
            ),
        );
    }
    if storage {
        source = source.replace("var<uniform>", "var<storage,read>");
    }
    for e in m["edits"].as_array().into_iter().flatten() {
        source = edit(&source, (e[0].as_str().unwrap(), e[1].as_str().unwrap()));
    }
    let module = ray_module(
        &r.device,
        &source,
        m["no_contraction"].as_bool().unwrap_or(false),
        m["passthrough"].as_bool().unwrap_or(false),
        &out,
    );
    let layout = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("minimal ray ABI"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if storage {
                            wgpu::BufferBindingType::Storage { read_only: true }
                        } else {
                            wgpu::BufferBindingType::Uniform
                        },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
    let pipeline_layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("minimal ray ABI"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let make_pipeline = || {
        r.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ray_probe"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("ray_probe"),
                compilation_options: Default::default(),
                cache: None,
            })
    };
    let pipeline = make_pipeline();
    let view = output.create_view(&Default::default());
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ray probe inputs"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
        ],
    });
    let run = |pipeline: &wgpu::ComputePipeline| {
        let mut e = r.device.create_command_encoder(&Default::default());
        {
            let mut p = e.begin_compute_pass(&Default::default());
            p.set_pipeline(pipeline);
            p.set_bind_group(0, &bind, &[]);
            p.dispatch_workgroups(width.div_ceil(8), height.div_ceil(4), 1);
        }
        r.queue.submit([e.finish()]);
        r.read(&output)
    };
    let repeats = m["repetitions"].as_u64().unwrap_or(128);
    let mut rows = vec![];
    let mut failures = 0;
    let mut original_pose: Option<Vec<u8>> = None;
    for pose in [0, 1, 0] {
        let matrix = cameras.frames[pose]
            .camera()
            .unwrap()
            .matrix(1920, 1080)
            .inverse();
        r.queue.write_buffer(&buffer, 0, &matrix_bytes(matrix));
        let first = run(&pipeline);
        if pose == 0 {
            if let Some(original) = &original_pose {
                if first != *original {
                    failures += 1;
                    rows.push(serde_json::json!({"return_diff":difference(original,&first,16)}));
                }
            } else {
                original_pose = Some(first.clone());
            }
        }
        for repeat in 0..repeats {
            let next = run(&pipeline);
            if next != first {
                failures += 1;
                let fresh = make_pipeline();
                let components: [usize; 4] = std::array::from_fn(|lane| {
                    first
                        .chunks_exact(16)
                        .zip(next.chunks_exact(16))
                        .filter(|(a, b)| a[lane * 4..lane * 4 + 4] != b[lane * 4..lane * 4 + 4])
                        .count()
                });
                let sample: Vec<_>=first.chunks_exact(4).zip(next.chunks_exact(4)).enumerate().filter(|(_, (a,b))|a!=b).take(8)
                    .map(|(word,(a,b))|serde_json::json!({"word":word,"a":f32::from_le_bytes(a.try_into().unwrap()),"b":f32::from_le_bytes(b.try_into().unwrap()),
                        "a_bits":u32::from_le_bytes(a.try_into().unwrap()),"b_bits":u32::from_le_bytes(b.try_into().unwrap())})).collect();
                let d = serde_json::json!({"pose":pose,"repeat":repeat,"diff":difference(&first,&next,16),"components":components,"sample":sample,"fresh":difference(&first,&run(&fresh),16)});
                eprintln!("[determinism] ray {d}");
                rows.push(d);
                break;
            }
        }
        rows.push(
            serde_json::json!({"pose":pose,"first":hash(&first),"matrix":matrix.to_cols_array()}),
        );
    }
    fs::write(out.join("ray.json"),serde_json::to_vec_pretty(&serde_json::json!({"rows":rows,"failures":failures,"storage":storage,"repetitions":repeats,
        "source_sha256":hash(source.as_bytes()),"no_contraction":m["no_contraction"],"passthrough":m["passthrough"],"width":width,"height":height})).unwrap()).unwrap();
    assert_eq!(failures, 0, "ray arithmetic changed; see ray.json");
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn difference(a: &[u8], b: &[u8], stride: usize) -> serde_json::Value {
    assert_eq!(a.len(), b.len());
    let changed: Vec<_> = a
        .chunks_exact(stride)
        .zip(b.chunks_exact(stride))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    serde_json::json!({"pixels":changed.len(), "first":changed.iter().take(32).collect::<Vec<_>>(), "a":hash(a), "b":hash(b)})
}

fn shade(r: &mut Renderer, pipeline: &wgpu::ComputePipeline) -> Vec<u8> {
    let mut encoder = r.device.create_command_encoder(&Default::default());
    r.encode_primary_shade(
        &mut encoder,
        pipeline,
        primary_ray_gpu::SHADE_TILE,
        primary_ray_gpu::Shading::Soft,
        None,
    );
    r.queue.submit([encoder.finish()]);
    capture(r)
}

pub(super) fn investigate(
    r: &mut Renderer,
    reference: &[u8],
    deviant: &[u8],
    hits: &[u8],
    counts: &[u8],
    seed: u32,
    out: &std::path::Path,
) {
    let [next_hits, next_counts] = interactive_outputs(r);
    let sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let mut fresh = r
        .primary
        .as_ref()
        .unwrap()
        .compile(&r.device, &r.ledger, &sources);
    let fresh_image = shade(r, &fresh.shade[1]);
    std::mem::swap(
        &mut r.pipeline.primary.as_mut().unwrap().shade[1],
        &mut fresh.shade[1],
    );
    let mut old_images = vec![];
    for _ in 0..32 {
        old_images.push(difference(deviant, &shade(r, &fresh.shade[1]), 8));
    }
    std::mem::swap(
        &mut r.pipeline.primary.as_mut().unwrap().shade[1],
        &mut fresh.shade[1],
    );
    let module = seeded_module(r);
    let fresh_trace = r.primary.as_ref().unwrap().interactive_pipeline(
        &r.device,
        &r.ledger,
        &module,
        &[("FRESH_SEED", f64::from(seed)), ("FRESH_DIRECTIONS", 8.)],
    );
    let mut encoder = r.device.create_command_encoder(&Default::default());
    r.encode_primary_interactive(
        &mut encoder,
        &fresh_trace,
        primary_ray_gpu::TRACE_TILE,
        None,
    );
    r.queue.submit([encoder.finish()]);
    let [fresh_hits, fresh_counts] = interactive_outputs(r);
    let fresh_frame = shade(r, &fresh.shade[1]);
    let row = serde_json::json!({"hdr":difference(reference,deviant,8),
        "hits":difference(hits,&next_hits,16),
        "counts":difference(counts,&next_counts,4),
        "fresh_trace_hits":difference(hits,&fresh_hits,16),"fresh_trace_counts":difference(counts,&fresh_counts,4),
        "fresh_frame":difference(reference,&fresh_frame,8),
        "fresh_vs_reference":difference(reference,&fresh_image,8),
        "old_vs_deviant":old_images});
    eprintln!(
        "[determinism] hdr={} hits={} counts={} fresh_frame={}",
        row["hdr"]["pixels"],
        row["hits"]["pixels"],
        row["counts"]["pixels"],
        row["fresh_frame"]["pixels"]
    );
    fs::write(
        out.join("determinism.json"),
        serde_json::to_vec_pretty(&row).unwrap(),
    )
    .unwrap();
    fs::write(out.join("reference.rgba16f"), reference).unwrap();
    fs::write(out.join("deviant.rgba16f"), deviant).unwrap();
    fs::write(out.join("reference.rgba32u"), hits).unwrap();
    fs::write(out.join("deviant.rgba32u"), next_hits).unwrap();
}

#[test]
#[ignore = "offscreen; GPU slot, >=6 GiB available RAM, experimental acknowledgment and manifest"]
fn gpu_static_repeat() {
    assert!(
        manifest()["static_repeats"].as_u64().unwrap_or(0) >= 30,
        "set static_repeats >= 30"
    );
    super::soft_shadow_acceptance_tests::gpu_soft_shadow_traced_acceptance();
}
