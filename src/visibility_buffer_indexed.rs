//! Compact visible geometry into one indexed draw. The first corner owns each
//! triangle's flat ID; its other corners share the hardware vertex cache.
use super::*;

fn enabled(r: &Renderer) -> bool {
    r.hardware_shadows.is_none() && std::env::var("RENDERLAB_VIS_INDEXED").is_ok_and(|v| v == "1")
}
pub(super) struct Plan {
    vertices: u32,
    indices: u32,
}
impl Plan {
    pub fn new(r: &Renderer, draws: &[PreparedDraw<'_>]) -> Option<Self> {
        if !enabled(r) || draws.is_empty() {
            return None;
        }
        let mut vertices = 0u64;
        let mut indices = 0u64;
        for d in draws {
            let mesh = d.mesh.clusters.as_ref()?;
            vertices += mesh
                .clusters
                .iter()
                .map(|c| u64::from((c.counts >> 8) & 255) + u64::from(c.counts & 255))
                .sum::<u64>()
                * d.instances.len() as u64;
            indices += mesh
                .clusters
                .iter()
                .map(|c| u64::from(c.counts & 255) * 3)
                .sum::<u64>()
                * d.instances.len() as u64;
        }
        // Each phase visits a subset of this table, so its atomic counters cannot wrap.
        if vertices > u64::from(u32::MAX) || indices > u64::from(u32::MAX) {
            return None;
        }
        let limit = r.device.limits().max_storage_buffer_binding_size;
        Some(Self {
            vertices: vertices.min(limit / 16).max(1) as u32,
            indices: indices.min(limit / 4).max(1) as u32,
        })
    }
}
pub(super) struct Raster {
    _vertices: Buffer,
    pub indices: Buffer,
    pub args: [Buffer; 2],
    pub build: wgpu::ComputePipeline,
    pub finish: wgpu::ComputePipeline,
    pub reset: wgpu::ComputePipeline,
    pub groups: [wgpu::BindGroup; 2],
    pub empty: wgpu::BindGroup,
    pub hardware: wgpu::RenderPipeline,
    pub hardware_groups: [wgpu::BindGroup; 2],
}
impl Raster {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        r: &Renderer,
        plan: Plan,
        configs: &[Buffer; 2],
        clusters: &Buffer,
        lists: &[Buffer; 2],
        cull_args: &[Buffer; 2],
        visibility: &Buffer,
        attachment: bool,
        array_layout: &wgpu::BindGroupLayout,
        data_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::{ShaderStages as S, TextureFormat as F};
        let storage = wgpu::BufferUsages::STORAGE;
        // Phases execute in order; attachments retain phase one's result after its
        // compacted streams are reused by phase two. No host readback or growth stall.
        let vertices = Buffer::new(
            r,
            "visibility compacted vertices",
            u64::from(plan.vertices) * 16,
            storage,
        );
        let indices = Buffer::new(
            r,
            "visibility compacted indices",
            u64::from(plan.indices) * 4,
            storage | wgpu::BufferUsages::INDEX,
        );
        let args = std::array::from_fn(|_| {
            let b = Buffer::new(
                r,
                "visibility indexed and fallback draws",
                48,
                storage | wgpu::BufferUsages::INDIRECT,
            );
            r.queue
                .write_buffer(&b.value, 0, &words([0, 1, 0, 0, 0, 0, 0, 0, 384, 0, 0, 0]));
            b
        });
        let build_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::COMPUTE),
                buffer_entry(1, false, false, S::COMPUTE),
                buffer_entry(3, false, false, S::COMPUTE),
                buffer_entry(5, false, false, S::COMPUTE),
                buffer_entry(7, false, true, S::COMPUTE),
                buffer_entry(8, false, true, S::COMPUTE),
                buffer_entry(9, false, true, S::COMPUTE),
            ],
        );
        let empty_layout = layout(&r.device, &[]);
        let empty = bind(&r.device, &empty_layout, &[]);
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
                bind_group_layouts: &[Some(&build_layout), Some(&empty_layout), Some(data_layout)],
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
        let finish = pipeline("finish_indices");
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
                    (7, args[i].value.as_entire_binding()),
                    (8, indices.value.as_entire_binding()),
                    (9, vertices.value.as_entire_binding()),
                ],
            )
        });
        let hardware_layout = layout(
            &r.device,
            &[
                buffer_entry(0, true, false, S::VERTEX | S::FRAGMENT),
                buffer_entry(1, false, false, S::VERTEX),
                buffer_entry(8, false, true, S::FRAGMENT),
                buffer_entry(9, false, false, S::VERTEX),
            ],
        );
        let shader = r
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("visibility indexed raster"),
                source: wgpu::ShaderSource::Wgsl(
                    stage_source(include_str!("../shaders/visibility_buffer_indexed.wgsl")).into(),
                ),
            });
        let hardware =
            r.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("visibility indexed raster"),
                    layout: Some(&r.device.create_pipeline_layout(
                        &wgpu::PipelineLayoutDescriptor {
                            label: Some("visibility indexed raster"),
                            bind_group_layouts: &[
                                Some(&hardware_layout),
                                Some(array_layout),
                                Some(data_layout),
                            ],
                            immediate_size: 0,
                        },
                    )),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(if attachment { "fs_color" } else { "fs" }),
                        compilation_options: Default::default(),
                        targets: if attachment {
                            &[Some(wgpu::ColorTargetState {
                                format: F::R32Uint,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            })]
                        } else {
                            &[]
                        },
                    }),
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: F::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                });
        let hardware_groups = std::array::from_fn(|i| {
            bind(
                &r.device,
                &hardware_layout,
                &[
                    (0, configs[i].value.as_entire_binding()),
                    (1, clusters.value.as_entire_binding()),
                    (8, visibility.value.as_entire_binding()),
                    (9, vertices.value.as_entire_binding()),
                ],
            )
        });
        Self {
            _vertices: vertices,
            indices,
            args,
            build,
            finish,
            reset,
            groups,
            empty,
            hardware,
            hardware_groups,
        }
    }
}
