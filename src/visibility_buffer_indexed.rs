//! Indexed camera raster: one unique provoking corner per triangle carries its ID;
//! the other two corners share vertices and the hardware post-transform cache.
use super::*;

pub(super) fn enabled(r: &Renderer) -> bool {
    r.hardware_shadows.is_none() && std::env::var("RENDERLAB_VIS_INDEXED").is_ok_and(|v| v == "1")
}

pub(super) struct Mesh {
    vertices: Buffer,
    pub offsets: Vec<[u32; 2]>,
    count: u32,
}
impl Mesh {
    pub fn new(r: &Renderer, clusters: &[Cluster], data: &[u32]) -> Option<Self> {
        if !enabled(r) {
            return None;
        }
        let count: u64 = clusters
            .iter()
            .map(|c| u64::from((c.counts >> 8) & 255) + u64::from(c.counts & 255))
            .sum();
        if count * 16 > u64::from(r.device.limits().max_storage_buffer_binding_size) {
            return None;
        }
        let mut vertices = Vec::with_capacity(count as usize * 4);
        let mut offsets = Vec::with_capacity(clusters.len());
        for (index, c) in clusters.iter().enumerate() {
            let base = vertices.len() as u32 / 4;
            let n = (c.counts >> 8) & 255;
            offsets.push([base, base + n]);
            for v in 0..n {
                let p = c.vertices as usize + v as usize * 3;
                vertices.extend_from_slice(&data[p..p + 3]);
                vertices.push((index as u32) << 7);
            }
            for triangle in 0..(c.counts & 255) {
                let v = data[c.triangles as usize + triangle as usize] & 255;
                let p = c.vertices as usize + v as usize * 3;
                vertices.extend_from_slice(&data[p..p + 3]);
                vertices.push(((index as u32) << 7) | triangle);
            }
        }
        let buffer = Buffer::new(
            r,
            "visibility indexed source vertices",
            (count * 16).max(16),
            wgpu::BufferUsages::STORAGE,
        );
        if !vertices.is_empty() {
            r.queue.write_buffer(&buffer.value, 0, &words(vertices));
        }
        Some(Self {
            vertices: buffer,
            offsets,
            count: count as u32,
        })
    }
}

pub(super) struct Plan {
    metadata: Vec<[u32; 8]>,
    capacity: u32,
}
impl Plan {
    pub fn new(r: &Renderer, draws: &[PreparedDraw<'_>]) -> Option<Self> {
        if !enabled(r) || draws.is_empty() || draws.iter().any(|d| d.mesh.skin.is_some()) {
            return None;
        }
        let mut metadata = Vec::new();
        let (mut indices, mut cluster_base, mut placement) = (0u64, 0u32, 0u32);
        for d in draws {
            let c = d.mesh.clusters.as_ref()?;
            let mesh = c.indexed.as_ref()?;
            let capacity = c
                .clusters
                .iter()
                .map(|c| u64::from(c.counts & 255) * 3)
                .sum::<u64>()
                * d.instances.len() as u64;
            // vertex_index encodes the placement and its mesh-local vertex.
            if u64::from(mesh.count) * d.instances.len() as u64 > u64::from(u32::MAX) {
                return None;
            }
            metadata.push([
                indices as u32,
                mesh.count,
                cluster_base,
                placement,
                c.clusters.len() as u32,
                0,
                0,
                0,
            ]);
            indices += capacity;
            if indices * 4 > u64::from(r.device.limits().max_storage_buffer_binding_size)
                || indices > u64::from(u32::MAX)
            {
                return None;
            }
            cluster_base += c.clusters.len() as u32 * d.instances.len() as u32;
            placement += d.instances.len() as u32;
        }
        Some(Self {
            metadata,
            capacity: indices as u32,
        })
    }
}

pub(super) struct Raster {
    pub indices: [Buffer; 2],
    pub args: [Buffer; 2],
    _metadata: Buffer,
    _params: Vec<Buffer>,
    pub build: wgpu::ComputePipeline,
    pub reset: wgpu::ComputePipeline,
    pub groups: [wgpu::BindGroup; 2],
    pub draws: Vec<wgpu::BindGroup>,
}
impl Raster {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        r: &Renderer,
        plan: Plan,
        draws: &[PreparedDraw<'_>],
        configs: &[Buffer; 2],
        clusters: &Buffer,
        lists: &[Buffer; 2],
        cull_args: &[Buffer; 2],
        visibility: &Buffer,
        hardware_layout: &wgpu::BindGroupLayout,
        array_layout: &wgpu::BindGroupLayout,
        data_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::ShaderStages as S;
        let storage = wgpu::BufferUsages::STORAGE;
        let metadata = Buffer::new(
            r,
            "visibility indexed draw metadata",
            (plan.metadata.len().max(1) * 32) as u64,
            storage,
        );
        r.queue.write_buffer(
            &metadata.value,
            0,
            &words(plan.metadata.iter().flatten().copied()),
        );
        let indices = std::array::from_fn(|_| {
            Buffer::new(
                r,
                "visibility compacted indices",
                (u64::from(plan.capacity) * 4).max(4),
                storage | wgpu::BufferUsages::INDEX,
            )
        });
        let args = std::array::from_fn(|_| {
            let b = Buffer::new(
                r,
                "visibility indexed indirect draws",
                (plan.metadata.len().max(1) * 20) as u64,
                storage | wgpu::BufferUsages::INDIRECT,
            );
            let values = plan.metadata.iter().flat_map(|m| [0, 1, m[0], 0, 0]);
            r.queue.write_buffer(&b.value, 0, &words(values));
            b
        });
        let build_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(1, false, false, S::COMPUTE),
                buffer_entry(3, false, false, S::COMPUTE),
                buffer_entry(5, false, false, S::COMPUTE),
                buffer_entry(6, false, false, S::COMPUTE),
                buffer_entry(7, false, true, S::COMPUTE),
                buffer_entry(8, false, true, S::COMPUTE),
            ],
        );
        let shader = r
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("visibility index compaction"),
                source: wgpu::ShaderSource::Wgsl(
                    stage_source(include_str!("../shaders/visibility_buffer_indices.wgsl")).into(),
                ),
            });
        let pipeline_layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("visibility index compaction"),
                bind_group_layouts: &[Some(&build_layout), Some(array_layout), Some(data_layout)],
                immediate_size: 0,
            });
        let pipeline = |entry| {
            r.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let build = pipeline("build_indices");
        let reset = pipeline("reset_indices");
        let groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &build_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (3, lists[i].value.as_entire_binding()),
                    (5, cull_args[i].value.as_entire_binding()),
                    (6, metadata.value.as_entire_binding()),
                    (7, args[i].value.as_entire_binding()),
                    (8, indices[i].value.as_entire_binding()),
                ],
            )
        });
        let params: Vec<_> = plan
            .metadata
            .iter()
            .map(|m| {
                let b = Buffer::new(
                    r,
                    "visibility indexed mesh parameters",
                    32,
                    wgpu::BufferUsages::UNIFORM,
                );
                r.queue.write_buffer(&b.value, 0, &words(*m));
                b
            })
            .collect();
        let groups_draw = draws
            .iter()
            .enumerate()
            .map(|(i, d)| {
                bind(
                    &r.device,
                    hardware_layout,
                    &[
                        (0, configs[0].value.as_entire_binding()),
                        (1, clusters.value.as_entire_binding()),
                        (3, lists[0].value.as_entire_binding()),
                        (8, visibility.value.as_entire_binding()),
                        (
                            9,
                            d.mesh
                                .clusters
                                .as_ref()
                                .unwrap()
                                .indexed
                                .as_ref()
                                .unwrap()
                                .vertices
                                .value
                                .as_entire_binding(),
                        ),
                        (10, params[i].value.as_entire_binding()),
                    ],
                )
            })
            .collect();
        Self {
            indices,
            args,
            _metadata: metadata,
            _params: params,
            build,
            reset,
            groups,
            draws: groups_draw,
        }
    }
}
