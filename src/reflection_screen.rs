//! Screen intersections for software reflections. Uncertain intersections keep the full tree ray.
use super::*;

pub(super) fn enabled(device: &wgpu::Device) -> bool {
    !device
        .features()
        .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        && std::env::var("RENDERLAB_SW_REFLECTION_SCREEN").as_deref() != Ok("0")
}

#[derive(Clone)]
pub(super) struct Abi {
    trace: Owned<wgpu::BindGroupLayout>,
    depth: Owned<wgpu::BindGroupLayout>,
    reduce: Owned<wgpu::BindGroupLayout>,
    colour: Owned<wgpu::BindGroupLayout>,
    depth_layout: Owned<wgpu::PipelineLayout>,
    reduce_layout: Owned<wgpu::PipelineLayout>,
    colour_layout: Owned<wgpu::PipelineLayout>,
}
impl Abi {
    pub(super) fn new(
        device: &PipelineDevice,
        ledger: &Ledger,
        group: &wgpu::BindGroupLayout,
        arrays: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::TextureSampleType as T;
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let trace = layout(
            "reflection screen trace",
            &[
                read(1, 16),
                uniform(2, 256),
                texture(3, T::Uint),
                texture(4, T::Uint),
                texture(6, T::Uint),
                texture(7, T::Float { filterable: false }),
            ],
        );
        let depth = layout(
            "reflection screen depth",
            &[
                uniform(2, 256),
                texture(4, T::Uint),
                texture(5, T::Uint),
                image(12, wgpu::TextureFormat::Rgba32Uint),
            ],
        );
        let reduce = layout(
            "reflection screen hierarchy",
            &[
                texture(3, T::Uint),
                image(12, wgpu::TextureFormat::Rgba32Uint),
            ],
        );
        let colour = layout(
            "reflection screen colour",
            &[
                texture(7, T::Float { filterable: false }),
                image(11, wgpu::TextureFormat::Rgba16Float),
            ],
        );
        let pipeline = |label, extra: &wgpu::BindGroupLayout| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[Some(group), Some(arrays), Some(extra), Some(lighting)],
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        let depth_layout = pipeline("reflection screen depth ABI", &depth);
        let reduce_layout = pipeline("reflection screen reduce ABI", &reduce);
        let colour_layout = pipeline("reflection screen colour ABI", &colour);
        Self {
            trace,
            depth,
            reduce,
            colour,
            depth_layout,
            reduce_layout,
            colour_layout,
        }
    }
    pub(super) fn trace(&self) -> &wgpu::BindGroupLayout {
        &self.trace
    }
    pub(super) fn pipelines(
        &self,
        make: impl Fn(&str, &wgpu::PipelineLayout) -> Owned<wgpu::ComputePipeline>,
    ) -> Pipelines {
        Pipelines {
            depth: make("rl_screen_depth", &self.depth_layout),
            reduce: make("rl_screen_reduce", &self.reduce_layout),
            colour: make("rl_screen_colour_reduce", &self.colour_layout),
        }
    }
}
#[derive(Clone)]
pub(super) struct Pipelines {
    depth: Owned<wgpu::ComputePipeline>,
    reduce: Owned<wgpu::ComputePipeline>,
    colour: Owned<wgpu::ComputePipeline>,
}
struct Previous {
    matrix: glam::Mat4,
    eye: glam::Vec3,
    frame: u64,
    edits: u64,
    lighting: u64,
}
pub(super) struct Targets {
    frame: Buffer,
    depth: Depth,
    previous: Depth,
    colour: Depth,
    depth_mips: Vec<wgpu::TextureView>,
    colour_mips: Vec<wgpu::TextureView>,
    state: std::sync::Mutex<Option<Previous>>,
}
impl Targets {
    pub(super) fn new(r: &Renderer, size: wgpu::Extent3d) -> Self {
        let mips = 6.min(32 - size.width.min(size.height).leading_zeros());
        let target = |label, format: wgpu::TextureFormat| {
            let texture = r.ledger.own(
                r.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: mips,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = r.ledger.own(
                texture.create_view(&Default::default()),
                ResourceKind::TextureView,
            );
            Depth {
                texture,
                view,
                _allocation: r.ledger.allocate(
                    AllocationKind::Target,
                    u64::from(size.width)
                        * u64::from(size.height)
                        * u64::from(format.block_copy_size(None).unwrap())
                        * 4
                        / 3,
                ),
            }
        };
        let depth = target("reflection screen depth", wgpu::TextureFormat::Rgba32Uint);
        let previous = target("reflection previous depth", wgpu::TextureFormat::Rgba32Uint);
        let colour = target(
            "reflection previous colour",
            wgpu::TextureFormat::Rgba16Float,
        );
        let views = |t: &Depth| {
            (0..mips)
                .map(|base_mip_level| {
                    t.texture.create_view(&wgpu::TextureViewDescriptor {
                        base_mip_level,
                        mip_level_count: Some(1),
                        ..Default::default()
                    })
                })
                .collect()
        };
        let depth_mips = views(&depth);
        let colour_mips = views(&colour);
        Self {
            frame: Buffer::new(
                r,
                "reflection screen frame",
                256,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            depth,
            previous,
            colour,
            depth_mips,
            colour_mips,
            state: Default::default(),
        }
    }
    pub(super) fn write(&self, r: &Renderer, camera: Camera, size: [u32; 2]) {
        let matrix = camera.matrix(size[0], size[1]);
        let eye = camera.view().inverse().w_axis.truncate();
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        r.lighting.pending.records.hash(&mut hasher);
        for x in r
            .lighting
            .pending
            .environment
            .into_iter()
            .chain([r.lighting.pending.exposure])
        {
            x.to_bits().hash(&mut hasher);
        }
        let lighting = hasher.finish();
        let mut state = self.state.lock().unwrap();
        let valid = state.as_ref().is_some_and(|p| {
            p.frame + 1 == r.frames
                && p.edits == r.scene_edits
                && p.lighting == lighting
                && p.eye.distance(eye) < 1.0
        }) && !r
            .view
            .reconstruction_state
            .as_ref()
            .is_some_and(|s| s.plan.reset);
        let prev = state.as_ref().map_or(matrix, |p| p.matrix);
        let mut bytes = Vec::with_capacity(256);
        for m in [matrix, prev, prev.inverse()] {
            bytes.extend_from_slice(&crate::mesh::matrix_bytes(m));
        }
        for v in [
            eye.extend(0.0),
            state.as_ref().map_or(eye, |p| p.eye).extend(0.0),
        ] {
            for x in v.to_array() {
                bytes.extend_from_slice(&x.to_ne_bytes());
            }
        }
        for x in [
            u32::from(valid),
            size[0],
            size[1],
            self.depth_mips.len() as u32,
        ] {
            bytes.extend_from_slice(&x.to_ne_bytes());
        }
        for x in [1.2f32, 0.2, 128.0, 0.0] {
            bytes.extend_from_slice(&x.to_ne_bytes());
        }
        r.queue.write_buffer(&self.frame.value, 0, &bytes);
        *state = Some(Previous {
            matrix,
            eye,
            frame: r.frames,
            edits: r.scene_edits,
            lighting,
        });
    }
}

pub(super) fn source(source: String, device: &wgpu::Device) -> String {
    if !enabled(device) {
        return source;
    }
    let seam = "let traced=rl_cast(origin,d,epsilon);";
    assert_eq!(source.matches(seam).count(), 1);
    let source=source.replacen(seam,"let screen=rl_screen_hit(origin,d,epsilon,cone,p);\n        if screen.a>0.0 {textureStore(rl_samples,at,rl_f16_nearest(screen));continue;}\n        let traced=rl_cast(origin,d,epsilon);",1);
    let source=source.replace("dropped:atomic<u32> };", "dropped:atomic<u32>, screen_pad:array<u32,7>, screen_try:atomic<u32>, screen_candidate:atomic<u32>, screen_hit:atomic<u32>, screen_valid:atomic<u32>, screen_end:u32 };");
    format!(
        "{source}\n{}",
        include_str!("../shaders/reflections_screen.wgsl")
    )
}
impl Renderer {
    pub(super) fn rl_screen_group(
        &self,
        abi: &Abi,
        t: &Targets,
        hit: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        use wgpu::BindingResource as R;
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection screen trace"),
            layout: &abi.trace,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.gi.shading_outgoing().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: t.frame.value.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: R::TextureView(&t.depth.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: R::TextureView(hit),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: R::TextureView(&t.previous.view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: R::TextureView(&t.colour.view),
                },
            ],
        })
    }
    pub(super) fn rl_screen_prepare(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        abi: &Abi,
        pipelines: &Pipelines,
        t: &Targets,
        bind: impl Fn(&mut wgpu::ComputePass<'_>),
    ) {
        use wgpu::BindingResource as R;
        let primary = self
            .view
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection screen depth"),
            layout: &abi.depth,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: t.frame.value.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: R::TextureView(&primary.hit.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: R::TextureView(&primary.tiles.view),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: R::TextureView(&t.depth_mips[0]),
                },
            ],
        });
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection screen depth"),
            timestamp_writes: None,
        });
        bind(&mut compute);
        compute.set_bind_group(2, &group, &[]);
        compute.set_pipeline(&pipelines.depth);
        let size = t.depth.texture.size();
        compute.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        drop(compute);
        for i in 1..t.depth_mips.len() {
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("reflection screen hierarchy"),
                layout: &abi.reduce,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: R::TextureView(&t.depth_mips[i - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: R::TextureView(&t.depth_mips[i]),
                    },
                ],
            });
            let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("reflection screen hierarchy"),
                timestamp_writes: None,
            });
            bind(&mut compute);
            compute.set_bind_group(2, &group, &[]);
            compute.set_pipeline(&pipelines.reduce);
            compute.dispatch_workgroups(
                (size.width >> i).max(1).div_ceil(8),
                (size.height >> i).max(1).div_ceil(8),
                1,
            );
        }
    }
    pub(super) fn rl_screen_capture(&self, encoder: &mut wgpu::CommandEncoder, shading: Shading) {
        use wgpu::BindingResource as R;
        let state = self.view.primary.as_ref().unwrap();
        let p = state.rl_pipelines[shading as usize].as_ref().unwrap();
        let Some(pipelines) = p.screen.as_ref() else {
            return;
        };
        let abi = state.rl_trace.as_ref().unwrap().screen.as_ref().unwrap();
        let t = state
            .targets
            .as_ref()
            .unwrap()
            .rl_pass
            .as_ref()
            .unwrap()
            .screen
            .as_ref()
            .unwrap();
        let group = state.bindings.as_ref().unwrap().rl_trace.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let size = t.depth.texture.size();
        for i in 0..t.depth_mips.len() as u32 {
            let extent = wgpu::Extent3d {
                width: (size.width >> i).max(1),
                height: (size.height >> i).max(1),
                depth_or_array_layers: 1,
            };
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.depth.texture,
                    mip_level: i,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &t.previous.texture,
                    mip_level: i,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                extent,
            );
        }
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.view.hdr.as_ref().unwrap().resolved.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &t.colour.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            size,
        );
        for i in 1..t.colour_mips.len() {
            let colours = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("reflection screen colour hierarchy"),
                layout: &abi.colour,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: R::TextureView(&t.colour_mips[i - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: R::TextureView(&t.colour_mips[i]),
                    },
                ],
            });
            let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("reflection screen colour hierarchy"),
                timestamp_writes: None,
            });
            compute.set_bind_group(0, &**group, &[]);
            compute.set_bind_group(1, &*g.arrays, &[]);
            compute.set_bind_group(2, &colours, &[]);
            compute.set_bind_group(3, self.lighting.bind(), &[]);
            compute.set_pipeline(&pipelines.colour);
            compute.dispatch_workgroups(
                (size.width >> i).max(1).div_ceil(8),
                (size.height >> i).max(1).div_ceil(8),
                1,
            );
        }
    }
}
