//! Synthetic record resource probe. Default mode is CPU-only naga/SPIR-V validation.
//! --gpu must run inside a GPU lease; use --timing for reported costs.
use std::{error::Error, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn source(array: bool, words: u32, read: bool) -> String {
    let mut s = String::new();
    if array {
        s.push_str(if read {
            "@group(0) @binding(0) var records:texture_2d_array<u32>;\n"
        } else {
            "@group(0) @binding(0) var records:texture_storage_2d_array<rgba32uint,write>;\n"
        });
    } else {
        for i in 0..words {
            s.push_str(&format!(
                "@group(0) @binding({i}) var word{i}:{};\n",
                if read {
                    "texture_2d<u32>"
                } else {
                    "texture_storage_2d<rgba32uint,write>"
                }
            ));
        }
    }
    let n = if array { 1 } else { words };
    if read {
        s.push_str(&format!(
            "@group(0) @binding({n}) var<storage,read_write> output:array<vec4<u32>>;\n"
        ));
    }
    s.push_str(&format!("struct Params {{size:vec2<u32>,words:u32,pad:u32}}; @group(0) @binding({}) var<uniform> params:Params;\n",n+u32::from(read)));
    s.push_str("@compute @workgroup_size(8,8) fn run(@builtin(global_invocation_id) id:vec3<u32>) {\nlet p=id.xy;if any(p>=params.size) {return;}var sum=vec4<u32>(0u);\nfor(var layer=0u;layer<params.words;layer+=1u) {\n");
    if read {
        s.push_str("var v:vec4<u32>;\n");
        if array {
            s.push_str("v=textureLoad(records,vec2<i32>(p),i32(layer),0);\n");
        } else {
            for i in 0..words {
                s.push_str(&format!(
                    "if layer=={i}u {{v=textureLoad(word{i},vec2<i32>(p),0);}}\n"
                ));
            }
        }
        s.push_str("sum+=v*(layer+1u);\n");
    } else {
        s.push_str("let v=vec4<u32>(p,layer,p.x^(p.y*65537u)^(layer*0x9e3779b9u));\n");
        if array {
            s.push_str("textureStore(records,vec2<i32>(p),i32(layer),v);\n");
        } else {
            for i in 0..words {
                s.push_str(&format!(
                    "if layer=={i}u {{textureStore(word{i},vec2<i32>(p),v);}}\n"
                ));
            }
        }
    }
    s.push_str("}\n");
    if read {
        s.push_str("output[p.y*params.size.x+p.x]=sum;\n");
    }
    s.push_str("}\n");
    s
}
fn validate() -> Result<()> {
    for words in [2, 3] {
        for array in [false, true] {
            for read in [false, true] {
                let s = source(array, words, read);
                let m = naga::front::wgsl::parse_str(&s).map_err(|e| e.emit_to_string(&s))?;
                let info = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&m)?;
                let spv = naga::back::spv::write_vec(
                    &m,
                    &info,
                    &Default::default(),
                    Some(&naga::back::spv::PipelineOptions {
                        shader_stage: naga::ShaderStage::Compute,
                        entry_point: "run".into(),
                    }),
                )?;
                println!(
                    "{}",
                    serde_json::json!({"cpu":"naga+spirv","words":words,"array":array,"read":read,"spirv_words":spv.len()})
                );
            }
        }
    }
    Ok(())
}
fn readback(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    })?;
    rx.recv()??;
    let bytes = buffer.slice(..).get_mapped_range()?.to_vec();
    buffer.unmap();
    Ok(bytes)
}
struct Case {
    array: bool,
    write: wgpu::ComputePipeline,
    read: wgpu::ComputePipeline,
    wgroup: wgpu::BindGroup,
    rgroup: wgpu::BindGroup,
    output: wgpu::Buffer,
    _images: Vec<wgpu::Texture>,
    compile_s: f64,
}
impl Case {
    fn new(d: &wgpu::Device, q: &wgpu::Queue, size: [u32; 2], words: u32, array: bool) -> Self {
        let images: Vec<_> = (0..if array { 1 } else { words })
            .map(|_| {
                d.create_texture(&wgpu::TextureDescriptor {
                    label: Some("record probe"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: if array { words } else { 1 },
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Uint,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            })
            .collect();
        let views: Vec<_> = images
            .iter()
            .map(|t| {
                t.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(if array {
                        wgpu::TextureViewDimension::D2Array
                    } else {
                        wgpu::TextureViewDimension::D2
                    }),
                    ..Default::default()
                })
            })
            .collect();
        let uniform = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params: Vec<_> = [size[0], size[1], words, 0]
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect();
        q.write_buffer(&uniform, 0, &params);
        let output = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(size[0]) * u64::from(size[1]) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let begin = Instant::now();
        let pipelines = [false, true].map(|read| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("record probe"),
                source: wgpu::ShaderSource::Wgsl(source(array, words, read).into()),
            });
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some("run"),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let compile_s = begin.elapsed().as_secs_f64();
        let groups = [false, true].map(|read| {
            let mut entries: Vec<_> = views
                .iter()
                .enumerate()
                .map(|(i, v)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: wgpu::BindingResource::TextureView(v),
                })
                .collect();
            let n = entries.len() as u32;
            if read {
                entries.push(wgpu::BindGroupEntry {
                    binding: n,
                    resource: output.as_entire_binding(),
                });
            }
            entries.push(wgpu::BindGroupEntry {
                binding: n + u32::from(read),
                resource: uniform.as_entire_binding(),
            });
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipelines[read as usize].get_bind_group_layout(0),
                entries: &entries,
            })
        });
        let [write, read] = pipelines;
        let [wgroup, rgroup] = groups;
        Self {
            array,
            write,
            read,
            wgroup,
            rgroup,
            output,
            _images: images,
            compile_s,
        }
    }
}
fn gpu() -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (d, q) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    println!(
        "{}",
        serde_json::json!({"adapter":format!("{:?}",adapter.get_info()),"limits":format!("{:?}",d.limits()),"note":"synthetic load/store; no driver cache cleared"})
    );
    let queries = d.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Timestamp,
        count: 4,
    });
    let resolved = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let stamps = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    for size in [[1920u32, 1080u32], [3840, 2160]] {
        for words in [2u32, 3] {
            let cases = [
                Case::new(&d, &q, size, words, false),
                Case::new(&d, &q, size, words, true),
            ];
            let mut times: [Vec<[f64; 2]>; 2] = Default::default();
            for round in 0..10 {
                for i in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let c = &cases[i];
                    let mut enc = d.create_command_encoder(&Default::default());
                    for (stage, (pipeline, group)) in [(&c.write, &c.wgroup), (&c.read, &c.rgroup)]
                        .into_iter()
                        .enumerate()
                    {
                        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: None,
                            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                                query_set: &queries,
                                beginning_of_pass_write_index: Some(stage as u32 * 2),
                                end_of_pass_write_index: Some(stage as u32 * 2 + 1),
                            }),
                        });
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, group, &[]);
                        pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
                    }
                    enc.resolve_query_set(&queries, 0..4, &resolved, 0);
                    enc.copy_buffer_to_buffer(&resolved, 0, &stamps, 0, 32);
                    q.submit([enc.finish()]);
                    let raw = readback(&d, &stamps)?;
                    let t: Vec<_> = raw
                        .chunks_exact(8)
                        .map(|b| u64::from_ne_bytes(b.try_into().unwrap()))
                        .collect();
                    if round >= 2 {
                        times[i].push([
                            (t[1] - t[0]) as f64 * f64::from(q.get_timestamp_period()) * 1e-6,
                            (t[3] - t[2]) as f64 * f64::from(q.get_timestamp_period()) * 1e-6,
                        ]);
                    }
                }
            }
            let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
            let copy = d.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for (i, c) in cases.iter().enumerate() {
                let mut enc = d.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(&c.output, 0, &copy, 0, bytes);
                q.submit([enc.finish()]);
                let data = readback(&d, &copy)?;
                for (pixel, b) in data.chunks_exact(16).enumerate() {
                    let x = pixel as u32 % size[0];
                    let y = pixel as u32 / size[0];
                    let mut expected = [0u32; 4];
                    for layer in 0..words {
                        let v = [
                            x,
                            y,
                            layer,
                            x ^ (y * 65537) ^ layer.wrapping_mul(0x9e3779b9),
                        ];
                        for k in 0..4 {
                            expected[k] = expected[k].wrapping_add(v[k].wrapping_mul(layer + 1));
                        }
                    }
                    for k in 0..4 {
                        assert_eq!(
                            u32::from_ne_bytes(b[k * 4..k * 4 + 4].try_into().unwrap()),
                            expected[k],
                            "pixel {pixel} component {k}"
                        );
                    }
                }
                let median = |stage: usize| {
                    let mut values: Vec<_> = times[i].iter().map(|t| t[stage]).collect();
                    values.sort_by(f64::total_cmp);
                    (values[3] + values[4]) * 0.5
                };
                println!(
                    "{}",
                    serde_json::json!({"size":size,"words":words,"array":c.array,"resource_bytes":bytes*u64::from(words),"storage_bindings":if c.array {1} else {words},"compile_s":c.compile_s,"write_ms":median(0),"read_ms":median(1),"samples":times[i].len(),"checked_pixels":size[0]*size[1]})
                );
            }
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    validate()?;
    if std::env::args().any(|s| s == "--gpu") {
        gpu()?;
    }
    Ok(())
}
