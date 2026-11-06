//! N1b: full-detail cluster IDs/depth, two-phase conservative HZB rejection, and
//! exact pixel-centre receiver reconstruction into the primary hit ABI. No LOD,
//! lighting-model decisions, acceleration structures or ray-query instructions.
use super::*;
use primary_ray_gpu::Buffer;

const TRIANGLES: u32 = 128;
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cluster {
    bounds: Aabb,
    first: u32,
    count: u32,
}

pub(crate) fn clusters(data: &MeshData<'_>) -> (Vec<Cluster>, Vec<u32>) {
    // Morton order gives small spatial clusters even for arbitrary source index order.
    // Source primitive IDs stay unchanged: the raster follows this indirection table.
    let bounds = Aabb::from_positions(data.vertices.iter().map(|v| v.position)).unwrap();
    let lo = Vec3::from_array(bounds.min);
    let scale = Vec3::splat(1023.) / (Vec3::from_array(bounds.max) - lo).max(Vec3::splat(1e-20));
    let spread = |v: u32| {
        let mut x = v & 1023;
        x = (x | (x << 16)) & 0x030000ff;
        x = (x | (x << 8)) & 0x0300f00f;
        x = (x | (x << 4)) & 0x030c30c3;
        (x | (x << 2)) & 0x09249249
    };
    let mut order: Vec<u64> = data
        .indices
        .chunks_exact(3)
        .enumerate()
        .map(|(id, ix)| {
            let centre = ix
                .iter()
                .map(|&i| Vec3::from_array(data.vertices[i as usize].position))
                .sum::<Vec3>()
                / 3.;
            let q = ((centre - lo) * scale)
                .clamp(Vec3::ZERO, Vec3::splat(1023.))
                .as_uvec3();
            let morton = spread(q.x) | (spread(q.y) << 1) | (spread(q.z) << 2);
            (u64::from(morton) << 32) | id as u64
        })
        .collect();
    order.sort_unstable();
    let triangles: Vec<_> = order.into_iter().map(|v| v as u32).collect();
    let clusters = triangles
        .chunks(TRIANGLES as usize)
        .enumerate()
        .map(|(i, ids)| Cluster {
            bounds: Aabb::from_positions(ids.iter().flat_map(|&p| {
                data.indices[p as usize * 3..p as usize * 3 + 3]
                    .iter()
                    .map(|&i| data.vertices[i as usize].position)
            }))
            .unwrap(),
            first: i as u32 * TRIANGLES,
            count: ids.len() as u32,
        })
        .collect();
    (clusters, triangles)
}
pub(super) fn supported(device: &wgpu::Device) -> bool {
    let limits = device.limits();
    device.features().contains(crate::VISIBILITY_FEATURES)
        && crate::visibility_limits(limits.clone()) == limits
}

/// The same repair arithmetic as the hardware two-pass check. The slow path for
/// ambiguous/missing adjacency searches exact shared endpoints in the original mesh;
/// this also supports upload policies that cannot fit the optional adjacency suffix.
pub(super) fn repair_source() -> String {
    let s = include_str!("../shaders/primary_trace.wgsl");
    let s =
        &s[s.find("struct PrimaryTrace {").unwrap()..s.find("// The pixel's camera ray").unwrap()];
    let start = s.find("// The full near-to-far").unwrap();
    let end = s.find("fn primary_repair(").unwrap();
    let mut s = format!("{}{}", &s[..start], &s[end..]);
    s = s.replace("    var query:ray_query<vertex_return>;", "");
    let start = s.find("            if neighbour==0xffffffffu {").unwrap();
    let end = start
        + s[start..]
            .find("            var direct_pending=true;")
            .unwrap();
    s.replace_range(start..end, "            if neighbour==0xffffffffu && primary_repair_scan[batch]==0u {continue;}\n            var search=0u;\n");
    let start = s
        .find("                    if !rayQueryProceed(&query)")
        .unwrap();
    let end = start
        + s[start..]
            .find("                }\n                var common_vertices")
            .unwrap();
    s.replace_range(start..end, "                    let words=arrayLength(&primary_indices[batch].data);\n                    let triangles=select(words/3u,(words-1u)/6u,words%3u==1u);\n                    if search>=triangles {break;}candidate_primitive=search;search+=1u;\n                    if candidate_primitive==trace.record.y {continue;}\n                    v=primary_triangle_vertices(batch,candidate_primitive);\n                    var shared_count=0u;for(var j=0u;j<3u;j+=1u) {shared_count+=u32(all(v[j]==e0)||all(v[j]==e1));}\n                    if shared_count!=2u || !primary_probe_intersects(v,slot,start,probe_direction,reach) {continue;}\n                    if !visible {visible=primary_edge_dot(primary_geometric(v[0],v[1],v[2],slot),direction)<0.0;}\n");
    assert!(!s.contains("rayQuery") && !s.contains("ray_query"));
    s
}
pub(super) fn resolve_source(wgsl: &str) -> Result<String, Error> {
    let head = primary_ray_gpu::non_rt_heads(wgsl)?;
    Ok(format!(
        "{}\n@group(0) @binding(30) var<storage,read> primary_repair_scan:array<u32>;\n{}\n{}",
        head[0],
        repair_source(),
        include_str!("../shaders/visibility_buffer_resolve.wgsl")
    ))
}

pub(super) struct VisibilityBuffer {
    size: wgpu::Extent3d,
    key: Vec<(wgpu::Buffer, usize)>,
    transforms: Vec<Mat4>,
    matrix: Mat4,
    count: u32,
    _clusters: Buffer,
    _triangles: Buffer,
    configs: [Buffer; 2],
    _lists: [Buffer; 2],
    args: [Buffer; 2],
    _rejected: Buffer,
    _repair_scan: Buffer,
    ids: Depth,
    depth: Depth,
    _hzb: Depth,
    _hzb_absent: Depth,
    hzb_sizes: Vec<[u32; 2]>,
    cull: wgpu::ComputePipeline,
    raster: wgpu::RenderPipeline,
    base: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
    resolve: [wgpu::ComputePipeline; 2],
    cull_groups: [wgpu::BindGroup; 2],
    raster_groups: [wgpu::BindGroup; 2],
    hzb_groups: Vec<wgpu::BindGroup>,
    resolve_group: wgpu::BindGroup,
    // Binding resources are retained by wgpu command buffers across replacements.
    hit: wgpu::Texture,
    world: wgpu::Texture,
}
fn layout(device: &wgpu::Device, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("N1b ABI"),
        entries,
    })
}
fn buffer_entry(
    binding: u32,
    uniform: bool,
    write: bool,
    stages: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stages,
        count: None,
        ty: wgpu::BindingType::Buffer {
            ty: if uniform {
                wgpu::BufferBindingType::Uniform
            } else {
                wgpu::BufferBindingType::Storage { read_only: !write }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    }
}
fn tex_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        count: None,
        ty,
    }
}
fn sampled(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    tex_entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}
fn image(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    tex_entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}
fn target(
    r: &Renderer,
    label: &str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
    mips: u32,
    usage: wgpu::TextureUsages,
) -> Depth {
    let texture = r.ledger.own(
        r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = r.ledger.own(
        texture.create_view(&Default::default()),
        ResourceKind::TextureView,
    );
    let bytes = u64::from(size.width)
        * u64::from(size.height)
        * u64::from(format.block_copy_size(None).unwrap_or(4))
        * if mips > 1 { 2 } else { 1 };
    Depth {
        texture,
        view,
        _allocation: r.ledger.allocate(AllocationKind::Target, bytes),
    }
}
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    entries: &[(u32, wgpu::BindingResource<'_>)],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("N1b resources"),
        layout,
        entries: &entries
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: resource.clone(),
            })
            .collect::<Vec<_>>(),
    })
}
impl VisibilityBuffer {
    fn new(r: &Renderer, draws: &[PreparedDraw<'_>], size: wgpu::Extent3d) -> Result<Self, Error> {
        use wgpu::{
            BindingResource as R, ShaderStages as S, TextureFormat as F, TextureUsages as U,
        };
        let state = r.primary.as_ref().unwrap();
        let t = state.targets.as_ref().unwrap();
        let fresh = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let count: usize = draws
            .iter()
            .map(|d| d.mesh.clusters.len() * d.instances.len())
            .sum();
        let bytes = (count.max(1) as u64)
            .checked_mul(48)
            .ok_or(Error::InvalidInput("visibility cluster table overflow"))?;
        if bytes > r.device.limits().max_storage_buffer_binding_size {
            return Err(Error::InvalidInput(
                "visibility clusters exceed storage binding limit",
            ));
        }
        let storage = wgpu::BufferUsages::STORAGE;
        let clusters = Buffer::new(r, "N1b source clusters", bytes, storage);
        let configs = std::array::from_fn(|_| {
            Buffer::new(r, "N1b view and pass", 80, wgpu::BufferUsages::UNIFORM)
        });
        let lists = std::array::from_fn(|_| {
            Buffer::new(
                r,
                "N1b compact visible clusters",
                count.max(1) as u64 * 4,
                storage,
            )
        });
        let args = std::array::from_fn(|_| {
            Buffer::new(
                r,
                "N1b indirect cluster raster",
                16,
                storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
            )
        });
        let rejected = Buffer::new(
            r,
            "N1b previous depth rejects",
            count.max(1) as u64 * 4,
            storage,
        );
        let repair_scan = Buffer::new(
            r,
            "N1b ambiguous-edge fallback policy",
            crate::TRACED_DRAWS as u64 * 4,
            storage,
        );
        let flags: Vec<_> = draws
            .iter()
            .map(|d| u32::from(d.mesh.repair_scan))
            .chain(std::iter::repeat(0))
            .take(crate::TRACED_DRAWS as usize)
            .flat_map(u32::to_ne_bytes)
            .collect();
        r.queue.write_buffer(&repair_scan.value, 0, &flags);
        let mut references = vec![];
        let mut bases = vec![];
        let mut meshes: Vec<(wgpu::Buffer, u32)> = vec![];
        for d in draws {
            let buffer = (**d.mesh.index).clone();
            let base = if let Some((_, base)) = meshes.iter().find(|(index, _)| *index == buffer) {
                *base
            } else {
                let base = references.len() as u32;
                references.extend_from_slice(&d.mesh.cluster_triangles);
                meshes.push((buffer, base));
                base
            };
            bases.push(base);
        }
        if references.len() as u64 * 4 > r.device.limits().max_storage_buffer_binding_size {
            return Err(Error::InvalidInput(
                "visibility triangle indirection exceeds storage binding limit",
            ));
        }
        let triangles = Buffer::new(
            r,
            "N1b spatial-to-source triangle IDs",
            (references.len().max(1) as u64) * 4,
            storage,
        );
        if !references.is_empty() {
            let bytes: Vec<_> = references.into_iter().flat_map(u32::to_ne_bytes).collect();
            r.queue.write_buffer(&triangles.value, 0, &bytes);
        }
        let mut packed = Vec::with_capacity(bytes as usize);
        let mut placement = 0u32;
        for (d, base) in draws.iter().zip(bases) {
            for _ in 0..d.instances.len() {
                for c in &d.mesh.clusters {
                    for x in c.bounds.min {
                        packed.extend_from_slice(&x.to_ne_bytes());
                    }
                    packed.extend_from_slice(&(c.first + base).to_ne_bytes());
                    for x in c.bounds.max {
                        packed.extend_from_slice(&x.to_ne_bytes());
                    }
                    packed.extend_from_slice(&c.count.to_ne_bytes());
                    for x in [placement, 0, 0, 0] {
                        packed.extend_from_slice(&x.to_ne_bytes());
                    }
                }
                placement += 1;
            }
        }
        if !packed.is_empty() {
            r.queue.write_buffer(&clusters.value, 0, &packed);
        }
        let ids = target(
            r,
            "N1b instance and source primitive",
            size,
            F::Rg32Uint,
            1,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC,
        );
        let depth = target(
            r,
            "N1b full-detail depth",
            size,
            F::Depth32Float,
            1,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let extent = wgpu::Extent3d {
            width: size.width.next_power_of_two(),
            height: size.height.next_power_of_two(),
            depth_or_array_layers: 1,
        };
        let levels = 32 - extent.width.max(extent.height).leading_zeros();
        let hzb = target(
            r,
            "N1b conservative min depth",
            extent,
            F::R32Float,
            levels,
            U::STORAGE_BINDING | U::TEXTURE_BINDING,
        );
        let hzb_absent = target(
            r,
            "N1b unused HZB base input",
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            F::R32Float,
            1,
            U::TEXTURE_BINDING,
        );
        let cull_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(1, false, false, S::COMPUTE),
                buffer_entry(2, false, true, S::COMPUTE),
                buffer_entry(3, false, true, S::COMPUTE),
                buffer_entry(4, false, true, S::COMPUTE),
                sampled(5, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        );
        let raster_layout = layout(
            &r.device,
            &[0, 1, 2, 6].map(|b| buffer_entry(b, b == 0, false, S::VERTEX)),
        );
        let hzb_layout = layout(
            &r.device,
            &[
                sampled(0, wgpu::TextureSampleType::Depth),
                image(1, F::R32Float),
                sampled(2, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        );
        let resolve_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(10, true, false, S::COMPUTE),
                buffer_entry(30, false, false, S::COMPUTE),
                sampled(29, wgpu::TextureSampleType::Uint),
                image(13, F::Rgba32Uint),
                image(16, F::Rgba32Float),
                image(17, F::Rgba32Float),
                image(20, F::R32Uint),
                image(23, F::R32Float),
            ],
        );
        let pipeline_layout = |groups: &[Option<&wgpu::BindGroupLayout>]| {
            r.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("N1b pipeline"),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                })
        };
        let compute = |source: &str,
                       entry: &str,
                       groups: &[Option<&wgpu::BindGroupLayout>],
                       constants: &[(&str, f64)]| {
            let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(entry),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            r.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout(groups)),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    cache: None,
                })
        };
        let source = include_str!("../shaders/visibility_buffer.wgsl");
        let cull = compute(
            &format!(
                "{source}\n{}",
                include_str!("../shaders/visibility_buffer_cull.wgsl")
            ),
            "cull",
            &[Some(&cull_layout), Some(state.array_layout())],
            &[],
        );
        let shader = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("N1b ID raster"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{source}\n{}",
                    include_str!("../shaders/visibility_buffer_raster.wgsl")
                )
                .into(),
            ),
        });
        let raster = r
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("N1b ID raster"),
                layout: Some(&pipeline_layout(&[
                    Some(&raster_layout),
                    Some(state.array_layout()),
                ])),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: F::Rg32Uint,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: F::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let hzb_source = include_str!("../shaders/visibility_buffer_hzb.wgsl");
        let base = compute(hzb_source, "base", &[Some(&hzb_layout)], &[]);
        let reduce = compute(hzb_source, "reduce", &[Some(&hzb_layout)], &[]);
        let resolve_source = resolve_source(&r.source)?;
        let resolve = [0.0, 1.0].map(|v| {
            compute(
                &resolve_source,
                "resolve",
                &[Some(&resolve_layout), Some(state.array_layout())],
                &[("WRITE_RECEIVERS", v)],
            )
        });
        let cull_groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &cull_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (2, lists[i].value.as_entire_binding()),
                    (3, args[i].value.as_entire_binding()),
                    (4, rejected.value.as_entire_binding()),
                    (5, R::TextureView(&hzb.view)),
                ],
            )
        });
        let raster_groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &raster_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (2, lists[i].value.as_entire_binding()),
                    (6, triangles.value.as_entire_binding()),
                ],
            )
        });
        let views: Vec<_> = (0..levels)
            .map(|i| {
                hzb.texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        // The unused source of the base pass must not alias its storage output.
        let hzb_groups = (0..levels)
            .map(|i| {
                bind(
                    &r.device,
                    &hzb_layout,
                    &[
                        (0, R::TextureView(&depth.view)),
                        (1, R::TextureView(&views[i as usize])),
                        (
                            2,
                            R::TextureView(if i == 0 {
                                &hzb_absent.view
                            } else {
                                &views[i as usize - 1]
                            }),
                        ),
                    ],
                )
            })
            .collect();
        let resolve_group = bind(
            &r.device,
            &resolve_layout,
            &[
                (0, r.uniform.as_entire_binding()),
                (10, state.frame_buffer().as_entire_binding()),
                (30, repair_scan.value.as_entire_binding()),
                (29, R::TextureView(&ids.view)),
                (13, R::TextureView(&t.hit.view)),
                (16, R::TextureView(&fresh.world.view)),
                (17, R::TextureView(&fresh.normal.view)),
                (20, R::TextureView(&t.tiles.view)),
                (23, R::TextureView(&t.epsilon.view)),
            ],
        );
        Ok(Self {
            size,
            key: draws
                .iter()
                .map(|d| ((**d.mesh.index).clone(), d.instances.len()))
                .collect(),
            transforms: vec![],
            matrix: Mat4::ZERO,
            count: count as u32,
            _clusters: clusters,
            _triangles: triangles,
            configs,
            _lists: lists,
            args,
            _rejected: rejected,
            _repair_scan: repair_scan,
            ids,
            depth,
            _hzb: hzb,
            _hzb_absent: hzb_absent,
            hzb_sizes: (0..levels)
                .map(|i| [(extent.width >> i).max(1), (extent.height >> i).max(1)])
                .collect(),
            cull,
            raster,
            base,
            reduce,
            resolve,
            cull_groups,
            raster_groups,
            hzb_groups,
            resolve_group,
            hit: (*t.hit.texture).clone(),
            world: (*fresh.world.texture).clone(),
        })
    }
    fn pyramid(&self, encoder: &mut wgpu::CommandEncoder) {
        for (i, (group, size)) in self.hzb_groups.iter().zip(&self.hzb_sizes).enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N1b HZB"),
                timestamp_writes: None,
            });
            pass.set_pipeline(if i == 0 { &self.base } else { &self.reduce });
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
    }
    #[cfg(test)]
    pub(super) fn test_counts(&self, r: &Renderer) -> [u32; 3] {
        let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("N1b 32-byte culling counters"),
            size: 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut e = r.device.create_command_encoder(&Default::default());
        for i in 0..2 {
            e.copy_buffer_to_buffer(&self.args[i].value, 0, &buffer, i as u64 * 16, 16);
        }
        let done = Arc::new(OnceLock::new());
        let signal = done.clone();
        e.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            signal.set(result).unwrap();
        });
        r.queue.submit([e.finish()]);
        r.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        done.get().unwrap().as_ref().unwrap();
        let bytes = buffer.get_mapped_range(..).unwrap();
        [
            self.count,
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spatial_clusters_preserve_each_source_triangle_and_its_bounds() {
        let vertices: Vec<_> = (0..200u32)
            .flat_map(|i| [[i as f32, 0., 0.], [i as f32, 1., 0.], [i as f32, 0., 1.]])
            .map(|position| Vertex {
                position,
                normal: [0., 1., 0.],
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect();
        let indices: Vec<_> = (0..vertices.len() as u32).collect();
        let (clusters, ids) = clusters(&MeshData {
            vertices: &vertices,
            indices: &indices,
        });
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..200).collect::<Vec<_>>());
        for c in clusters {
            for &id in &ids[c.first as usize..(c.first + c.count) as usize] {
                for &v in &indices[id as usize * 3..id as usize * 3 + 3] {
                    let p = vertices[v as usize].position;
                    assert!(
                        (0..3)
                            .all(|axis| p[axis] >= c.bounds.min[axis]
                                && p[axis] <= c.bounds.max[axis])
                    );
                }
            }
        }
    }
    #[test]
    fn visibility_shaders_have_no_ray_queries_and_validate() {
        let base = include_str!("../shaders/visibility_buffer.wgsl");
        let mut sources = vec![
            format!(
                "{base}\n{}",
                include_str!("../shaders/visibility_buffer_cull.wgsl")
            ),
            format!(
                "{base}\n{}",
                include_str!("../shaders/visibility_buffer_raster.wgsl")
            ),
            include_str!("../shaders/visibility_buffer_hzb.wgsl").to_owned(),
            resolve_source(include_str!("../shaders/scene.wgsl")).unwrap(),
        ];
        let (kernels, heads) =
            primary_ray_gpu::non_rt_sources(include_str!("../shaders/scene.wgsl")).unwrap();
        let temporal = primary_ray_gpu::film_accumulation::shading_head(&heads[1]).unwrap();
        sources.push(
            primary_ray_gpu::film_accumulation::compose_shading(
                &temporal,
                crate::STANDARD_PROGRAM,
                false,
            )
            .source,
        );
        sources.extend(kernels);
        for head in heads {
            sources.push(
                crate::program::compose(
                    &head,
                    crate::STANDARD_PROGRAM,
                    crate::program::Stage::Traced,
                    false,
                )
                .source,
            );
        }
        for source in sources {
            assert!(
                !source.contains("ray_query")
                    && !source.contains("rayQuery")
                    && !source.contains("acceleration_structure")
            );
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
    }
}
impl Renderer {
    pub(super) fn prepare_visibility_buffer(
        &mut self,
        draws: &[PreparedDraw<'_>],
        camera: &Camera,
        size: wgpu::Extent3d,
    ) -> Result<(), Error> {
        // Unshadowed raster-primary frames still need receiver targets bound to resolve.
        if self
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .is_none_or(|t| t.world.texture.size() != size)
        {
            self.ensure_fresh_targets(size, false);
        }
        let primary = self.primary.as_ref().unwrap().targets.as_ref().unwrap();
        let world = &self
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .world
            .texture;
        let key: Vec<_> = draws
            .iter()
            .map(|d| ((**d.mesh.index).clone(), d.instances.len()))
            .collect();
        if self.visibility_buffer.as_ref().is_none_or(|v| {
            v.size != size || v.key != key || v.hit != *primary.hit.texture || v.world != **world
        }) {
            let scopes = OperationScopes::new(&self.device);
            let next = VisibilityBuffer::new(self, draws, size)?;
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
            self.visibility_buffer = Some(next);
        }
        let v = self.visibility_buffer.as_mut().unwrap();
        let matrix = camera.matrix(size.width, size.height);
        let transforms: Vec<_> = draws
            .iter()
            .flat_map(|d| d.instances.state.models.iter().copied())
            .collect();
        let valid = v.matrix == matrix && v.transforms == transforms;
        for i in 0..2 {
            let mut bytes = matrix_bytes(matrix).to_vec();
            for word in [
                v.count,
                u32::from(i == 1 || valid),
                i as u32,
                size.width | (size.height << 16),
            ] {
                bytes.extend_from_slice(&word.to_ne_bytes());
            }
            self.queue.write_buffer(&v.configs[i].value, 0, &bytes);
            let args: [u32; 4] = [TRIANGLES * 3, 0, 0, 0];
            let bytes: Vec<_> = args.into_iter().flat_map(u32::to_ne_bytes).collect();
            self.queue.write_buffer(&v.args[i].value, 0, &bytes);
        }
        v.matrix = matrix;
        v.transforms = transforms;
        Ok(())
    }
    pub(super) fn encode_visibility_buffer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        soft: bool,
        stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let v = self.visibility_buffer.as_ref().unwrap();
        let arrays = self.primary.as_ref().unwrap().virtual_geometry().unwrap().1;
        // A tiny first pass records the start; final resolve records the end. The
        // existing primary timestamp span includes both culls, rasters and pyramids.
        {
            let _pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N1b begin"),
                timestamp_writes: stamps.as_ref().map(|s| wgpu::ComputePassTimestampWrites {
                    query_set: s.query_set,
                    beginning_of_pass_write_index: s.beginning_of_pass_write_index,
                    end_of_pass_write_index: None,
                }),
            });
        }
        for i in 0..2 {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("N1b cluster cull"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&v.cull);
                pass.set_bind_group(0, &v.cull_groups[i], &[]);
                pass.set_bind_group(1, arrays, &[]);
                let groups = v.count.div_ceil(128);
                pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535).max(1), 1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("N1b cluster IDs"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &v.ids.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if i == 0 {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &v.depth.view,
                        depth_ops: Some(wgpu::Operations {
                            load: if i == 0 {
                                wgpu::LoadOp::Clear(0.0)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&v.raster);
                pass.set_bind_group(0, &v.raster_groups[i], &[]);
                pass.set_bind_group(1, arrays, &[]);
                pass.draw_indirect(&v.args[i].value, 0);
            }
            v.pyramid(encoder);
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N1b exact hit resolve"),
            timestamp_writes: stamps.as_ref().map(|s| wgpu::ComputePassTimestampWrites {
                query_set: s.query_set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: s.end_of_pass_write_index,
            }),
        });
        pass.set_pipeline(&v.resolve[usize::from(soft)]);
        pass.set_bind_group(0, &v.resolve_group, &[]);
        pass.set_bind_group(1, arrays, &[]);
        pass.dispatch_workgroups(v.size.width.div_ceil(8), v.size.height.div_ceil(4), 1);
    }
}
