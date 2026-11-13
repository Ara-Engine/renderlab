//! R0b test-only reflection experiment. No production renderer setting or default.
//! Uses reference asset packing and AS construction, and the actual user sources
//! with namespaced entry points and an explicit constant-sky service adapter.
use super::*;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
#[repr(u32)]
enum Mode {
    Primary = 0,
    Reflections = 1,
    MirrorIdentity = 2,
    MirroredCamera = 3,
}

fn function(source: &str, name: &str) -> String {
    let start = source.find(&format!("fn {name}(")).unwrap();
    let brace = start + source[start..].find('{').unwrap();
    let mut depth = 1;
    let mut end = brace + 1;
    while depth != 0 {
        depth +=
            i32::from(source.as_bytes()[end] == b'{') - i32::from(source.as_bytes()[end] == b'}');
        end += 1;
    }
    source[start..end].to_owned()
}
fn shader() -> String {
    let reference = include_str!("../shaders/reference_path_tracer.wgsl");
    let reference = &reference[..reference.find("var<private> reference_centre").unwrap()];
    let mut reference = reference
        .replace("struct Material {", "struct RefMaterial {")
        .replace("array<Material>", "array<RefMaterial>")
        .replace("m:Material", "m:RefMaterial");
    let start = reference.find("struct Geometry {").unwrap();
    let end = reference[start..].find("fn address(").unwrap() + start;
    reference.replace_range(start..end, "");
    reference = reference.replace("environment_direction(", "reference_environment_direction(");
    reference = reference
        .replace(
            "identity:vec2<u32>};",
            "identity:vec2<u32>,source_front:bool};",
        )
        .replace("vec2<u32>(0u));", "vec2<u32>(0u),true);")
        .replace(
            "if dot(ng,direction)>0.0 {ng=-ng;n=-n;}",
            "let source_front=dot(ng,direction)<=0.0; if !source_front {ng=-ng;n=-n;}",
        )
        .replace(
            "h.primitive_index+1u));",
            "h.primitive_index+1u),source_front);",
        );
    let scene = include_str!("../shaders/scene.wgsl");
    let api = include_str!("../shaders/program_api.wgsl");
    let input =
        &api[api.find("struct SurfaceInput {").unwrap()..api.find("// One light's").unwrap()];
    // The reference tracer's shadow ray needs its cull mask; scene.wgsl lends only functions.
    let mut source = crate::shadows::with_shadow_caster_mask(&format!(
        "{reference}\n{input}\n{}\n",
        include_str!("../shaders/reflection_probe.wgsl")
    ));
    // DC1: this frozen reflection experiment has no decal scene. Supply only the
    // surface value type and material helpers, with identity decal inputs.
    let decals = include_str!("../shaders/decals.wgsl");
    source.push_str(
        &decals[decals.find("struct DecalInput {").unwrap()..decals.find("fn dc_texel(").unwrap()],
    );
    source.push_str(&decals[decals.find("fn decal_base_color(").unwrap()..]);
    for name in ["exposed_product", "brdf"] {
        source.push_str(&function(scene, name));
        source.push('\n');
    }
    // P-DEFECT's hit regularization; this frozen probe never sets it, so its lobes stay exact.
    source.push_str("var<private> rl_hit_variance:f32;\n");
    source.push_str(
        "struct LightSample {index:u32,direction:vec3<f32>,attenuation:f32,intensity:vec3<f32>};\n",
    );
    // MQ2: the standard program's `view_dependence` hook returns this engine type.
    source.push_str(crate::program::VIEW_API);
    // LC1: the probe walks every light, unshadowed beyond its own hard_shadow.
    source.push_str("fn rl_light_total()->u32 {return lighting.count;}\nfn rl_light_index(i:u32)->u32 {return i;}\nfn rl_local_shadow(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {return 1.0;}\n");
    for name in [
        "light_count",
        "light_sample",
        "light_sample_untinted",
        "light_transmission",
        "light_visibility",
        // The engine's display bound: its stub here, so the probe always traces.
        "light_visibility_within",
        "rl_lb_skip",
        "expose",
        "reflection_report",
        "rl_finite",
        "material_normal",
        // TP1: the standard program's transparency inputs.
        "transparent_material",
        "transparent_opacity",
        "transparent_ior",
        "transparent_coverage",
    ] {
        source.push_str(&function(api, name));
        source.push('\n');
    }
    // The standard program's `view_data` returns the API's view-mode record.
    source.push_str(api.lines().find(|l| l.starts_with("struct ViewData")).unwrap());
    source.push('\n');
    // RL#50: the shading terms' constant gates, which the API's light helpers read.
    for line in api.lines().filter(|l| {
        l.starts_with("const RL_SHADE_TERMS")
            || l.starts_with("const RL_TERM_")
            || l.starts_with("fn rl_term(")
            || l.starts_with("const RL_LB_SKIPPED")
    }) {
        source.push_str(line);
        source.push('\n');
    }
    // TP2: this isolated reference adapter has no transparent casters.
    source.push_str("fn transparent_sun_transmission(world:vec3<f32>, direction:vec3<f32>)->vec3<f32> { return vec3<f32>(1.0); }\n");
    let standard = crate::STANDARD_PROGRAM
        .replace("Surface", "StdSurface")
        .replace("StdSurfaceInput", "SurfaceInput")
        .replace("fn surface(", "fn std_surface(")
        .replace("fn shade(", "fn std_shade(");
    let toon = include_str!("../shaders/programs/toon.wgsl")
        .replace("Surface", "ToonSurface")
        .replace("ToonSurfaceInput", "SurfaceInput")
        .replace("fn surface(", "fn toon_surface(")
        .replace("fn shade(", "fn toon_shade(")
        .replace("Parameters", "ToonParameters")
        .replace("DEFAULT_PARAMETERS", "TOON_DEFAULT_PARAMETERS")
        .replace("params.", "toon_params.");
    source.push_str(&standard);
    source.push_str(&toon);
    source.push_str("\nvar<private> toon_params:ToonParameters=TOON_DEFAULT_PARAMETERS;\n");
    source
}

#[test]
fn probe_shader_and_program_sources_validate() {
    let source = shader();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}

struct Probe {
    frame: wgpu::Buffer,
    output: wgpu::Buffer,
    binds: Vec<wgpu::BindGroup>,
    configs: Vec<wgpu::Buffer>,
    pipelines: Vec<wgpu::ComputePipeline>,
    _buffers: Vec<wgpu::Buffer>,
    _blases: Vec<wgpu::Blas>,
    _tlas: wgpu::Tlas,
    scene: Scene,
    max_pixels: u64,
}
impl Probe {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: Scene,
        view: &View,
        programs: &[u32],
    ) -> Self {
        let settings = Settings::default();
        // One shared vertex and index buffer; each BLAS reads its mesh's slice in place.
        let mut vertex_data = Vec::new();
        let mut index_data: Vec<u32> = Vec::new();
        let mut offsets = Vec::with_capacity(scene.meshes.len());
        for mesh in &scene.meshes {
            offsets.push((
                (vertex_data.len() / VERTEX_FLOATS) as u32,
                index_data.len() as u32,
            ));
            for (k, (p, n)) in mesh.positions.iter().zip(&mesh.normals).enumerate() {
                let uv = mesh.uvs.get(k).copied().unwrap_or([0.0; 2]);
                vertex_data.extend([p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]]);
                vertex_data.extend(
                    mesh.tangents
                        .get(k)
                        .copied()
                        .unwrap_or([1.0, 0.0, 0.0, 1.0]),
                );
                vertex_data.extend(mesh.colors.get(k).copied().unwrap_or([1.0; 4]));
            }
            index_data.extend(&mesh.indices);
        }
        let input = wgpu::BufferUsages::BLAS_INPUT;
        let vertices = storage(device, "reference vertices", &floats(vertex_data), input);
        let indices = storage(
            device,
            "reference indices",
            &index_data
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<_>>(),
            input,
        );
        let mut instance_data = Vec::new();
        for i in &scene.instances {
            let (vertex, index) = offsets[i.mesh];
            for word in [vertex, index, i.material as u32, 0] {
                instance_data.extend(word.to_le_bytes());
            }
            let normal = i.transform.inverse().transpose();
            for column in [normal.x_axis, normal.y_axis, normal.z_axis] {
                instance_data.extend(floats(column.truncate().extend(0.0).to_array()));
            }
        }
        let instances = storage(
            device,
            "reference instances",
            &instance_data,
            wgpu::BufferUsages::empty(),
        );
        let material_data = floats(
            scene
                .materials
                .iter()
                .enumerate()
                .flat_map(|(material_index, m)| {
                    let [r, g, b] = m.base_color;
                    let [er, eg, eb] = m.emission;
                    [
                        r,
                        g,
                        b,
                        m.metallic,
                        er,
                        eg,
                        eb,
                        programs[material_index] as f32,
                        m.roughness,
                        m.specular,
                        m.normal_texture.map_or(1.0, |t| t.scale),
                        0.0,
                    ]
                })
                .collect::<Vec<_>>(),
        );
        let material_data = inputs::material_bytes(&scene, material_data);
        let materials = storage(
            device,
            "reference materials",
            &material_data,
            wgpu::BufferUsages::empty(),
        );
        let (texels, texture_info, environment) = inputs::upload(device, &scene).unwrap();
        let pixels = u64::from(view.width) * u64::from(view.height);
        let sums_bytes = pixels * 16;
        let sums = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reference sums"),
            size: sums_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reference frame"),
            size: FRAME_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flags = match settings.acceleration_build {
            AccelerationBuild::FastTrace => wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            AccelerationBuild::FastBuild => wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
        };
        let sizes: Vec<_> = scene
            .meshes
            .iter()
            .map(|m| wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: m.positions.len() as u32,
                index_format: Some(wgpu::IndexFormat::Uint32),
                index_count: Some(m.indices.len() as u32),
                flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            })
            .collect();
        let blases: Vec<_> = sizes
            .iter()
            .map(|size| {
                device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some("reference BLAS"),
                        flags,
                        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles {
                        descriptors: vec![size.clone()],
                    },
                )
            })
            .collect();
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("reference TLAS"),
            max_instances: scene.instances.len() as u32,
            flags,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        for (slot, i) in scene.instances.iter().enumerate() {
            tlas[slot] = Some(wgpu::TlasInstance::new(
                &blases[i.mesh],
                row_transform(i.transform),
                slot as u32,
                if i.casts_shadows { 0x03 } else { 0x02 },
            ));
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        let entries: Vec<_> = scene
            .meshes
            .iter()
            .enumerate()
            .map(|(k, _)| wgpu::BlasBuildEntry {
                blas: &blases[k],
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size: &sizes[k],
                        vertex_buffer: &vertices,
                        first_vertex: offsets[k].0,
                        vertex_stride: (VERTEX_FLOATS * 4) as u64,
                        index_buffer: Some(&indices),
                        first_index: Some(offsets[k].1),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            })
            .collect();
        encoder.build_acceleration_structures(entries.iter(), [&tlas]);
        queue.submit([encoder.finish()]);
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let primary = buffer("R0b primary records", pixels * 128);
        let secondary = buffer("R0b secondary records", pixels * 128);
        let incident = buffer("R0b shaded hits", pixels * 16);
        let dfg = storage(
            device,
            "R0b production DFG",
            &floats(
                crate::lighting::DFG
                    .chunks_exact(2)
                    .map(|b| crate::decode_f16(u16::from_le_bytes([b[0], b[1]]))),
            ),
            wgpu::BufferUsages::empty(),
        );
        let configs: Vec<_> = (0..16)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("R0b test-only settings"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let storage_entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mut entries: Vec<_> = [
            (2, true),
            (3, true),
            (4, true),
            (5, true),
            (6, false),
            (7, true),
            (8, true),
            (9, true),
            (10, true),
            (12, false),
            (13, false),
            (14, false),
        ]
        .into_iter()
        .map(|(b, r)| storage_entry(b, r))
        .collect();
        for binding in [0, 11] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("R0b"),
            entries: &entries,
        });
        let binds = configs
            .iter()
            .map(|config| {
                let mut entries: Vec<_> = [
                    (0, &frame),
                    (2, &vertices),
                    (3, &indices),
                    (4, &instances),
                    (5, &materials),
                    (6, &sums),
                    (7, &texels),
                    (8, &texture_info),
                    (9, &environment),
                    (10, &dfg),
                    (11, config),
                    (12, &primary),
                    (13, &secondary),
                    (14, &incident),
                ]
                .into_iter()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding,
                    resource: buffer.as_entire_binding(),
                })
                .collect();
                entries.push(wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tlas.as_binding(),
                });
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("R0b"),
                    layout: &layout,
                    entries: &entries,
                })
            })
            .collect();
        let source = shader();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("R0b own-program reflection probe"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("R0b"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = [
            "probe_primary",
            "probe_primary_shade",
            "probe_reflect",
            "probe_hit_shade",
            "probe_resolve",
        ]
        .into_iter()
        .map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        })
        .collect();
        Self {
            frame,
            output: sums,
            binds,
            configs,
            pipelines,
            _buffers: vec![
                vertices,
                indices,
                instances,
                materials,
                texels,
                texture_info,
                environment,
                primary,
                secondary,
                incident,
                dfg,
            ],
            _blases: blases,
            _tlas: tlas,
            scene,
            max_pixels: pixels,
        }
    }

    fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &View,
        mode: Mode,
        rays: u32,
    ) -> (Vec<f32>, Vec<Vec<f64>>) {
        assert!((1..=16).contains(&rays));
        let pixels = u64::from(view.width) * u64::from(view.height);
        assert!(pixels <= self.max_pixels);
        let exposure = 2.0f32.powi(-14);
        queue.write_buffer(
            &self.frame,
            0,
            &frame_bytes(&self.scene, view, &Settings::default(), 0, 1),
        );
        for (k, config) in self.configs.iter().enumerate() {
            let mut bytes = Vec::new();
            for x in [mode as u32, rays, k as u32, exposure.to_bits()] {
                bytes.extend(x.to_le_bytes());
            }
            queue.write_buffer(config, 0, &bytes);
        }
        let reflect = matches!(mode, Mode::Reflections | Mode::MirrorIdentity);
        let passes = 2 + if reflect { 3 * rays } else { 0 };
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("R0b per-stage timings"),
            ty: wgpu::QueryType::Timestamp,
            count: 2 * passes,
        });
        let resolved = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("R0b timestamps"),
            size: u64::from(passes) * 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("R0b image and timestamps"),
            size: pixels * 16 + u64::from(passes) * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut timings = Vec::new();
        let mut first_hash = None;
        let mut image = Vec::new();
        for repeat in 0..9 {
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut index = 0;
            let mut dispatch = |stage: usize, sample: usize| {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("R0b stage"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: &queries,
                        beginning_of_pass_write_index: Some(index),
                        end_of_pass_write_index: Some(index + 1),
                    }),
                });
                index += 2;
                pass.set_pipeline(&self.pipelines[stage]);
                pass.set_bind_group(0, &self.binds[sample], &[]);
                pass.dispatch_workgroups(view.width.div_ceil(8), view.height.div_ceil(8), 1);
            };
            dispatch(0, 0);
            dispatch(1, 0);
            if reflect {
                for k in 0..rays as usize {
                    dispatch(2, k);
                    dispatch(3, k);
                    dispatch(4, k);
                }
            }
            encoder.resolve_query_set(&queries, 0..2 * passes, &resolved, 0);
            encoder.copy_buffer_to_buffer(
                &resolved,
                0,
                &readback,
                pixels * 16,
                u64::from(passes) * 16,
            );
            if repeat == 0 || repeat == 8 {
                encoder.copy_buffer_to_buffer(&self.output, 0, &readback, 0, pixels * 16);
            }
            queue.submit([encoder.finish()]);
            let bytes = read(device, &readback);
            if repeat == 0 || repeat == 8 {
                let hash = Sha256::digest(&bytes[..pixels as usize * 16]);
                if let Some(first) = first_hash {
                    assert_eq!(first, hash, "same-resource deterministic replay changed");
                } else {
                    first_hash = Some(hash);
                }
                image = bytes[..pixels as usize * 16]
                    .chunks_exact(16)
                    .flat_map(|p| {
                        (0..3).map(move |c| {
                            f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()) / exposure
                        })
                    })
                    .collect();
            }
            if repeat > 0 {
                let times: Vec<_> = bytes[pixels as usize * 16..]
                    .chunks_exact(8)
                    .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                let stage: Vec<_> = times
                    .chunks_exact(2)
                    .map(|t| (t[1] - t[0]) as f64 * f64::from(queue.get_timestamp_period()) / 1e6)
                    .collect();
                let mut row = vec![stage[0], stage[1], 0.0, 0.0, 0.0];
                for (k, value) in stage[2..].iter().enumerate() {
                    row[2 + k % 3] += value;
                }
                timings.push(row);
            }
        }
        assert!(image.iter().all(|v| v.is_finite() && *v >= 0.0));
        (image, timings)
    }
}
fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let (send, recv) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        })
        .unwrap();
    recv.recv().unwrap().unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    buffer.unmap();
    bytes
}
fn gpu() -> (wgpu::Device, wgpu::Queue) {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    );
    // The launcher owns and monitors the shared lease before this process starts.
    let owner: serde_json::Value = serde_json::from_slice(
        &std::fs::read("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").unwrap(),
    )
    .unwrap();
    assert_eq!(owner["owner"], "m7-r0b");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    eprintln!("R0b adapter: {:?}", adapter.get_info());
    let limits = crate::traced_limits(wgpu::Limits {
        max_blas_primitive_count: a.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: a.max_tlas_instance_count,
        max_acceleration_structures_per_shader_stage: 1,
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        max_storage_buffers_per_shader_stage: 12,
        ..Default::default()
    });
    // SAFETY: the caller explicitly acknowledged the experimental ray-query risk.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: FEATURES
            | wgpu::Features::TIMESTAMP_QUERY
            | crate::TRACED_FEATURES
            | crate::RECEIVER_FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}
fn save_image(
    root: &Path,
    name: &str,
    image: &[f32],
    view: &View,
    timings: &[Vec<f64>],
    extra: serde_json::Value,
) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join(format!("{name}.f32")),
        floats(image.iter().copied()),
    )
    .unwrap();
    let meta = serde_json::json!({"width":view.width,"height":view.height,"gpu_stage_ms":timings,
        "stages":["primary_trace","primary_shade","reflection_trace","hit_shade_with_visibility","resolve"],
        "shader_sha256":format!("{:x}",Sha256::digest(shader())),"same_resource_replay_bit_identical":true,
        "timing_eligible_for_claim":false,"extra":extra});
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&meta).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "GPU; RENDERLAB_R0B_SCENES and shared GPU lease required"]
fn gpu_r0b_fixtures() {
    let (device, queue) = gpu();
    let root = std::env::var("RENDERLAB_M7_REFERENCE_OUT").unwrap();
    let root = Path::new(&root);
    for path in std::env::var("RENDERLAB_R0B_SCENES").unwrap().split(';') {
        let path = Path::new(path);
        let bench = super::tests::load_bench(&path.join("scene.json"));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("scene.json")).unwrap()).unwrap();
        let programs: Vec<_> = json["materials"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| u32::from(m["program"] == "toon"))
            .collect();
        let probe = Probe::new(&device, &queue, bench.scene, &bench.views[0].1, &programs);
        for (pose, view) in &bench.views {
            for rays in [4, 8] {
                let (image, times) = probe.render(&device, &queue, view, Mode::Reflections, rays);
                let name = format!("{}-{pose}-k{rays}", json["bench"].as_str().unwrap());
                save_image(
                    root,
                    &name,
                    &image,
                    view,
                    &times,
                    serde_json::json!({"scene_sha256":bench.sha256,"rays":rays,"mode":"fixed-subcone/no-history"}),
                );
                eprintln!("R0b {name} complete");
            }
        }
    }
}

#[test]
#[ignore = "GPU; shared GPU lease required"]
fn gpu_r0b_identity() {
    let (device, queue) = gpu();
    let root = std::env::var("RENDERLAB_M7_REFERENCE_OUT").unwrap();
    let root = Path::new(&root);
    let (scene, view) = identity_fixture();
    for (program, tag) in [("standard", 0), ("toon", 1)] {
        let probe = Probe::new(&device, &queue, scene.clone(), &view, &[0, tag]);
        let (reflected, rt) = probe.render(&device, &queue, &view, Mode::MirrorIdentity, 1);
        let (camera, ct) = probe.render(&device, &queue, &view, Mode::MirroredCamera, 1);
        // The oracle uses a separately traced camera reflected across y=0,
        // clipped analytically at the plane. Every pixel/channel is checked.
        let mut worst = 0.0f32;
        let mut failures = 0;
        let mut max_absolute = 0.0f32;
        for (&a, &b) in reflected.iter().zip(&camera) {
            let error = (a - b).abs();
            let tolerance = 0.002 + 2e-5 * b.abs();
            max_absolute = max_absolute.max(error);
            worst = worst.max(error / tolerance);
            failures += usize::from(error > tolerance);
        }
        save_image(
            root,
            &format!("identity-{program}-reflection"),
            &reflected,
            &view,
            &rt,
            serde_json::json!({"program":program}),
        );
        save_image(
            root,
            &format!("identity-{program}-camera"),
            &camera,
            &view,
            &ct,
            serde_json::json!({"program":program}),
        );
        std::fs::write(root.join(format!("identity-{program}.json")),serde_json::to_vec_pretty(&serde_json::json!({"program":program,"pixels":view.width*view.height,
            "maximum_absolute_radiance_error":max_absolute,"maximum_tolerance_ratio":worst,"failed_channels":failures,
            "precision":"abs(a-b) <= 0.002 cd/m2 + 2e-5 * abs(camera), all RGB channels, all pixels; no masks",
            "same_program_source_sha256":format!("{:x}",Sha256::digest(if tag==0 {crate::STANDARD_PROGRAM} else {include_str!("../shaders/programs/toon.wgsl")}))})).unwrap()).unwrap();
        assert_eq!(
            failures, 0,
            "{program}: mirror-camera identity failed, max ratio {worst}"
        );
    }
}

#[test]
#[ignore = "GPU; private prepared quarry and shared GPU lease required"]
fn gpu_r0b_quarry() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read("E:/renderlab-work/m7-u0b/quarry-static.json").unwrap(),
    )
    .unwrap();
    let asset_path = manifest["asset"].as_str().unwrap();
    let bytes = std::fs::read(asset_path).unwrap();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let asset = renderlab_asset::decode(&bytes, renderlab_asset::Limits::unlimited()).unwrap();
    drop(bytes);
    let mut scene = Scene::from_prepared(asset);
    // The plain quarry is r=.85 throughout. This forced-all-roughness stress
    // measures long secondary rays; it is not the proposed R1 B0/B1 workload.
    scene.sky = [1500.0, 1800.0, 2400.0];
    scene.camera_background = Some([0.0; 3]);
    let lighting: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest["lighting"].as_str().unwrap()).unwrap())
            .unwrap();
    let v3 = |v: &serde_json::Value| {
        Vec3::new(
            v[0].as_f64().unwrap() as f32,
            v[1].as_f64().unwrap() as f32,
            v[2].as_f64().unwrap() as f32,
        )
    };
    scene.sun = Some(Sun {
        to_light: -v3(&lighting["lights"][0]["travel"]),
        illuminance: v3(&lighting["lights"][0]["lux"]).to_array(),
        angular_radius: 0.04,
    });
    let frame = &manifest["frames"][0];
    let pose = Pose {
        eye: v3(&frame["open"]["eye"]),
        target: v3(&frame["open"]["target"]),
    };
    let view = View {
        open: pose,
        close: pose,
        up: Vec3::Y,
        fov_y: frame["fov_y"].as_f64().unwrap() as f32,
        width: 1920,
        height: 1080,
    };
    let programs = vec![0; scene.materials.len()];
    let (device, queue) = gpu();
    let root = std::env::var("RENDERLAB_M7_REFERENCE_OUT").unwrap();
    let start = std::time::Instant::now();
    let probe = Probe::new(&device, &queue, scene, &view, &programs);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        })
        .unwrap();
    let initialization_seconds = start.elapsed().as_secs_f64();
    for (mode, rays) in [
        (Mode::Primary, 1),
        (Mode::Reflections, 4),
        (Mode::Reflections, 8),
    ] {
        let (image, times) = probe.render(&device, &queue, &view, mode, rays);
        let name = format!("quarry-{:?}-k{rays}", mode);
        save_image(
            Path::new(&root),
            &name,
            &image,
            &view,
            &times,
            serde_json::json!({"asset_sha256":hash,"initialization_seconds":initialization_seconds,
            "scope":"forced r=.85 reflection rays; constant sky; full-detail geometry; no comparative quality claim"}),
        );
    }
}

fn identity_fixture() -> (Scene, View) {
    let quad = |y| Mesh {
        positions: vec![
            [-8.0, y, -8.0],
            [8.0, y, -8.0],
            [8.0, y, 8.0],
            [-8.0, y, 8.0],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        indices: vec![0, 2, 1, 0, 3, 2],
        ..Default::default()
    };
    let mirror = quad(0.0);
    let mut target = quad(3.0);
    target.colors = vec![
        [0.3, 0.2, 0.8, 1.0],
        [0.9, 0.2, 0.1, 1.0],
        [0.1, 0.9, 0.2, 1.0],
        [0.8, 0.5, 0.2, 1.0],
    ];
    let pose = Pose {
        eye: Vec3::new(0.0, 2.0, 0.0),
        target: Vec3::ZERO,
    };
    let view = View {
        open: pose,
        close: pose,
        up: Vec3::Z,
        fov_y: 0.7,
        width: 256,
        height: 144,
    };
    let scene = Scene {
        meshes: vec![mirror, target],
        materials: vec![
            Material {
                base_color: [1.0; 3],
                metallic: 1.0,
                roughness: 0.0,
                ..Default::default()
            },
            Material {
                base_color: [0.7, 0.5, 0.3],
                roughness: 0.3,
                ..Default::default()
            },
        ],
        instances: vec![
            Instance {
                casts_shadows: true,
                mesh: 0,
                material: 0,
                transform: Mat4::IDENTITY,
            },
            Instance {
                casts_shadows: true,
                mesh: 1,
                material: 1,
                transform: Mat4::IDENTITY,
            },
        ],
        sun: Some(Sun {
            to_light: Vec3::new(0.3, -1.0, 0.2).normalize(),
            illuminance: [1000.0, 980.0, 940.0],
            angular_radius: 0.00465,
        }),
        sky: [30.0, 40.0, 50.0],
        camera_background: Some([0.0; 3]),
        ..Default::default()
    };
    (scene, view)
}

#[test]
#[ignore = "GPU; production traced-primary/program control and shared lease"]
fn gpu_r0b_program_primary() {
    use crate::{
        Camera, Draw, HardwareShadowOptions, Lighting, MaterialDescriptor, MeshData,
        PrimaryVisibility, ProgramDescriptor, Renderer, RendererOptions, Vertex, ViewMode,
    };
    let (device, queue) = gpu();
    let (mut scene, view) = identity_fixture();
    scene.sun = None;
    let root = std::env::var("RENDERLAB_M7_REFERENCE_OUT").unwrap();
    let root = Path::new(&root);
    // AA state (docs/iq1-quarry-image.md): 1x, TR1 off (explicit since TR1b made it the
    // default: this test compares exact single frames with a pixel-centre oracle),
    // anisotropy 1.
    let mut renderer = Renderer::new_with_hardware_shadows(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8Unorm,
        crate::Limits::default(),
        RendererOptions::default(),
        HardwareShadowOptions::default(),
        include_str!("../shaders/scene.wgsl"),
    )
    .unwrap();
    renderer
        .set_primary_visibility(PrimaryVisibility::Traced)
        .unwrap();
    renderer
        .set_reconstruction(crate::Reconstruction::off())
        .unwrap();
    renderer
        .set_lighting(&Lighting {
            lights: vec![],
            environment: scene.sky,
            exposure_stops: -14.0,
        })
        .unwrap();
    let target_mesh = &scene.meshes[1];
    let vertices: Vec<_> = target_mesh
        .positions
        .iter()
        .zip(&target_mesh.normals)
        .zip(&target_mesh.colors)
        .map(|((&position, &normal), &color)| Vertex {
            position,
            normal,
            color,
            uv: [0.0; 2],
        })
        .collect();
    let mesh = loop {
        if let Some(mesh) = renderer
            .try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &target_mesh.indices,
            })
            .unwrap()
        {
            break mesh;
        }
        renderer.test_drain("R0b native mesh");
    };
    let mut instances = loop {
        if let Some(i) = renderer.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
            break i;
        }
        renderer.test_drain("R0b native instances");
    };
    let camera = Camera::look_at(
        Vec3::new(0.0, -2.0, 0.0),
        Vec3::new(0.0, 3.0, 0.0),
        Vec3::Z,
        view.fov_y,
        0.01,
        100.0,
    )
    .unwrap();
    let target = crate::tests::target(&device, view.width, view.height);
    for (program_name, tag, source) in [
        ("standard", 0, crate::STANDARD_PROGRAM),
        ("toon", 1, include_str!("../shaders/programs/toon.wgsl")),
    ] {
        let program = renderer
            .try_create_program(&ProgramDescriptor {
                label: "R0b unchanged program native control",
                source,
            })
            .unwrap();
        let m = scene.materials[1];
        let material = loop {
            if let Some(m) = renderer
                .try_create_material(&MaterialDescriptor {
                    base_color: [m.base_color[0], m.base_color[1], m.base_color[2], 1.0],
                    metallic: m.metallic,
                    roughness: m.roughness,
                    double_sided: true,
                    program: Some(&program),
                    ..Default::default()
                })
                .unwrap()
            {
                break m;
            }
            renderer.test_drain("R0b native material");
        };
        let probe = Probe::new(&device, &queue, scene.clone(), &view, &[0, tag]);
        let (reflected, times) = probe.render(&device, &queue, &view, Mode::MirrorIdentity, 1);
        for _ in 0..2 {
            loop {
                if renderer
                    .try_render_view(
                        &target,
                        &camera,
                        &mut [Draw::new(&mesh, &mut instances).with_material(&material)],
                        ViewMode::Lit,
                    )
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                renderer.test_drain("R0b native admission");
            }
            renderer.test_drain("R0b native frame");
        }
        let mut capture = renderer.try_capture_hdr().unwrap().unwrap();
        renderer.test_drain("R0b native capture");
        let bytes = capture.try_read().unwrap().unwrap();
        let mut native = Vec::new();
        let mut failures = 0;
        let mut max_relative = 0.0f32;
        let mut max_absolute = 0.0f32;
        // A look-at camera has right=forward x up. Reflecting a complete camera
        // basis reverses handedness, so compare its horizontal pixel flip.
        for y in 0..view.height {
            for x in 0..view.width {
                for c in 0..3 {
                    let offset = ((y * view.width + (view.width - 1 - x)) * 8) as usize + c * 2;
                    let b =
                        crate::decode_f16(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
                            * 16384.0;
                    let a = reflected[((y * view.width + x) * 3) as usize + c];
                    let error = (a - b).abs();
                    max_absolute = max_absolute.max(error);
                    max_relative = max_relative.max(error / b.abs().max(1e-6));
                    failures += usize::from(error > 0.002 + 0.002 * b.abs());
                    native.push(b);
                }
            }
        }
        save_image(
            root,
            &format!("program-{program_name}-reflection"),
            &reflected,
            &view,
            &times,
            serde_json::json!({"program":program_name}),
        );
        save_image(
            root,
            &format!("program-{program_name}-native-primary"),
            &native,
            &view,
            &[],
            serde_json::json!({"program":program_name}),
        );
        std::fs::write(root.join(format!("program-{program_name}.json")),serde_json::to_vec_pretty(&serde_json::json!({"program":program_name,"failed_channels":failures,"pixels":view.width*view.height,"maximum_absolute_radiance_error":max_absolute,"maximum_relative_error":max_relative,
            "precision":"0.002 cd/m2 + 0.2% of native primary, including binary16 HDR quantization; all pixels/channels",
            "scope":"unchanged production traced-primary renderer vs reflected hit through R0b adapter; constant sky, vertex colour, back-face normal, GGX/DFG or toon rim; no sun, textures or GI"})).unwrap()).unwrap();
        assert_eq!(
            failures, 0,
            "{program_name}: production-primary program control failed"
        );
    }
}
