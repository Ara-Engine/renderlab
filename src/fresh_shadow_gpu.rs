use super::*;

const PIXEL_SEAM: &str = "    let source_front=front==(input.winding>0.0);";
pub(super) const GEOMETRIC_SEAM: &str = "    let geometric=unit_normal(cross(unit_normal(dpdx(input.world)),unit_normal(dpdy(input.world))));";

/// Receiver pixels per `fresh_compute` workgroup; WGSL `FRESH_TILE` must match.
pub(super) const TILE: [u32; 2] = [2, 2];

/// Compose only after the hardware path has checked affine geometry and marker order.
pub(super) fn shader_source(source: &str) -> Result<String, Error> {
    compose(source, include_str!("../shaders/fresh_shadows.wgsl"))
}

/// Splices a fresh visibility body into the checked scene source. Tests use this to
/// compare frozen or experimental bodies against the production one.
pub(super) fn compose(source: &str, fresh: &str) -> Result<String, Error> {
    crate::hardware_shadows::shader_source(source, SampleCount::One)?;
    if source.matches(PIXEL_SEAM).count() != 1 || source.matches(GEOMETRIC_SEAM).count() != 1 {
        return Err(Error::InvalidInput(
            "fresh shadows require the unchanged fragment pixel/geometric receiver seams",
        ));
    }
    let (before, rest) = source
        .split_once(crate::hardware_shadows::VISIBILITY_BEGIN)
        .unwrap();
    let (_, after) = rest
        .split_once(crate::hardware_shadows::VISIBILITY_END)
        .unwrap();
    Ok(
        format!("enable wgpu_ray_query;\n{before}\n{fresh}\n{after}").replacen(
            PIXEL_SEAM,
            &format!("    fresh_pixel=vec2<i32>(input.position.xy);\n{PIXEL_SEAM}"),
            1,
        ),
    )
}

/// Two float32 receiver attachments and one uint visibility count per output pixel.
pub(super) fn target_bytes(width: u32, height: u32) -> Result<u64, Error> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(36))
        .ok_or(Error::InvalidInput(
            "fresh shadow target byte count overflow",
        ))
}

pub(super) fn admit_targets(settings: &Shadows, width: u32, height: u32) -> Result<(), Error> {
    let views = match settings.requests.as_slice() {
        [ShadowRequest::Directional { split_ends }] => split_ends.len() as u64,
        _ => {
            return Err(Error::InvalidInput(
                "fresh shadows require one directional request",
            ));
        }
    };
    let bytes = target_bytes(width, height)?
        .checked_add((views + 1) * crate::shadows::VIEW_BYTES + 4)
        .ok_or(Error::InvalidInput("fresh shadow byte count overflow"))?;
    if settings.max_bytes.is_some_and(|max| bytes > max) {
        return Err(Error::InvalidInput(
            "fresh receiver, visibility and shadow records exceed max_bytes",
        ));
    }
    Ok(())
}

pub(super) struct FreshPipelines {
    pub receiver: Owned<wgpu::RenderPipeline>,
    pub shade: [Owned<wgpu::RenderPipeline>; 2],
    pub compute: Owned<wgpu::ComputePipeline>,
    /// Receiver pixels per `compute` workgroup, which sets the dispatch grid.
    pub compute_tile: [u32; 2],
}
struct ComputeBinding {
    bind: Owned<wgpu::BindGroup>,
    scene_generation: u64,
}
pub(super) struct FreshTargets {
    pub world: Depth,
    pub normal: Depth,
    pub visibility: Depth,
    pub shade: Owned<wgpu::BindGroup>,
    compute: Option<ComputeBinding>,
}
pub(super) struct FreshState {
    compute_bindings: Owned<wgpu::BindGroupLayout>,
    shade_bindings: Owned<wgpu::BindGroupLayout>,
    receiver_layout: Owned<wgpu::PipelineLayout>,
    compute_layout: Owned<wgpu::PipelineLayout>,
    shade_layout: Owned<wgpu::PipelineLayout>,
    pub targets: Option<FreshTargets>,
    retired: Vec<(u64, FreshTargets)>,
    retired_binds: Vec<(u64, ComputeBinding)>,
}
impl FreshState {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        groups: [&wgpu::BindGroupLayout; 4],
    ) -> Self {
        let texture = |binding, sample_type, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let compute_bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fresh shadow compute inputs"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::AccelerationStructure {
                            vertex_return: false,
                        },
                        count: None,
                    },
                    texture(
                        3,
                        wgpu::TextureSampleType::Float { filterable: false },
                        wgpu::ShaderStages::COMPUTE,
                    ),
                    texture(
                        4,
                        wgpu::TextureSampleType::Float { filterable: false },
                        wgpu::ShaderStages::COMPUTE,
                    ),
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let shade_bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fresh shadow shading inputs"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(80),
                        },
                        count: None,
                    },
                    texture(
                        6,
                        wgpu::TextureSampleType::Uint,
                        wgpu::ShaderStages::FRAGMENT,
                    ),
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = |label, first: &wgpu::BindGroupLayout| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[
                        Some(first),
                        Some(groups[1]),
                        Some(groups[2]),
                        Some(groups[3]),
                    ],
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        Self {
            receiver_layout: ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("fresh receiver ABI"),
                    bind_group_layouts: &[Some(groups[0]), Some(groups[1])],
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            ),
            compute_layout: ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("fresh compute ABI"),
                    bind_group_layouts: &[Some(&compute_bindings), None, None, Some(groups[3])],
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            ),
            shade_layout: layout("fresh shading ABI", &shade_bindings),
            compute_bindings,
            shade_bindings,
            targets: None,
            retired: vec![],
            retired_binds: vec![],
        }
    }
    pub fn compile(&self, device: &wgpu::Device, ledger: &Ledger, source: &str) -> FreshPipelines {
        let shader = ledger.own(
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fresh receiver compute shading candidate"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }),
            ResourceKind::ShaderModule,
        );
        let receiver_formats = [
            Some(wgpu::TextureFormat::Rgba32Float.into()),
            Some(wgpu::TextureFormat::Rgba32Float.into()),
        ];
        let shade_formats = [Some(wgpu::TextureFormat::Rgba16Float.into())];
        let mesh = |tangent, receiver| {
            ledger.own(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(if receiver {
                        "fresh depth-winning receivers"
                    } else {
                        "fresh ordinary lighting"
                    }),
                    layout: Some(if receiver {
                        &self.receiver_layout
                    } else {
                        &self.shade_layout
                    }),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(if tangent { "vs_tangent" } else { "vs" }),
                        compilation_options: Default::default(),
                        buffers: &[
                            Some(wgpu::VertexBufferLayout {
                                array_stride: 48,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &VERTEX_ATTRIBUTES,
                            }),
                            Some(wgpu::VertexBufferLayout {
                                array_stride: 4,
                                step_mode: wgpu::VertexStepMode::Instance,
                                attributes: &MODEL_ATTRIBUTES,
                            }),
                            if tangent {
                                Some(wgpu::VertexBufferLayout {
                                    array_stride: 16,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &TANGENT_ATTRIBUTES,
                                })
                            } else {
                                None
                            },
                        ],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(if receiver { "fs_receiver" } else { "fs_lit" }),
                        compilation_options: Default::default(),
                        targets: if receiver {
                            &receiver_formats
                        } else {
                            &shade_formats
                        },
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Greater),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
                ResourceKind::RenderPipeline,
            )
        };
        FreshPipelines {
            receiver: mesh(false, true),
            shade: [false, true].map(|t| mesh(t, false)),
            compute: self.compute_pipeline(device, ledger, &shader, "fresh_compute", &[]),
            compute_tile: TILE,
        }
    }
    /// `entry` from a composed module, on the persistent compute ABI, with optional
    /// pipeline-constant overrides (tests seed or replace the production estimator).
    pub fn compute_pipeline(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        shader: &wgpu::ShaderModule,
        entry: &str,
        constants: &[(&str, f64)],
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fresh 256 visibility rays"),
                layout: Some(&self.compute_layout),
                module: shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..Default::default()
                },
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        )
    }
    pub fn release(&mut self, last: u64) {
        if let Some(old) = self.targets.take() {
            self.retired.push((last, old));
        }
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
        self.retired_binds.retain(|(last, _)| *last > completed);
    }
}
impl Renderer {
    pub(super) fn ensure_fresh_targets(&mut self, size: wgpu::Extent3d, hardware_build: bool) {
        // Traced frames write the same receivers from compute.
        let receiver_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | if self.primary.is_some() {
                wgpu::TextureUsages::STORAGE_BINDING
            } else {
                wgpu::TextureUsages::empty()
            };
        let state = self.fresh.as_mut().unwrap();
        let ledger = &self.ledger;
        let device = &self.device;
        if state
            .targets
            .as_ref()
            .is_none_or(|t| t.world.texture.size() != size)
        {
            let target = |label, format: wgpu::TextureFormat, usage| {
                let texture = ledger.own(
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: usage
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            // Tests read receivers and inject reference visibility.
                            | if cfg!(test) {
                                wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST
                            } else {
                                wgpu::TextureUsages::empty()
                            },
                        view_formats: &[],
                    }),
                    ResourceKind::Texture,
                );
                let view = ledger.own(
                    texture.create_view(&Default::default()),
                    ResourceKind::TextureView,
                );
                Depth {
                    texture,
                    view,
                    _allocation: ledger.allocate(
                        AllocationKind::Target,
                        u64::from(size.width)
                            * u64::from(size.height)
                            * u64::from(format.block_copy_size(None).unwrap()),
                    ),
                }
            };
            let world = target(
                "fresh exact world position and depth",
                wgpu::TextureFormat::Rgba32Float,
                receiver_usage,
            );
            let normal = target(
                "fresh geometric normal and coverage",
                wgpu::TextureFormat::Rgba32Float,
                receiver_usage,
            );
            let visibility = target(
                "fresh visible ray counts",
                wgpu::TextureFormat::R32Uint,
                wgpu::TextureUsages::STORAGE_BINDING,
            );
            let shade = ledger.own(
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fresh lighting"),
                    layout: &state.shade_bindings,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::TextureView(&visibility.view),
                        },
                    ],
                }),
                ResourceKind::BindGroup,
            );
            state.release(self.submissions);
            state.targets = Some(FreshTargets {
                world,
                normal,
                visibility,
                shade,
                compute: None,
            });
        }
        let t = state.targets.as_mut().unwrap();
        let hardware = self.hardware_shadows.as_ref().unwrap();
        let scene_generation = hardware.stats.tlas_builds + u64::from(hardware_build);
        if t.compute
            .as_ref()
            .is_none_or(|b| b.scene_generation != scene_generation)
        {
            let bind = ledger.own(
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fresh persistent compute inputs"),
                    layout: &state.compute_bindings,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: hardware.scene().as_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&t.world.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&t.normal.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(&t.visibility.view),
                        },
                    ],
                }),
                ResourceKind::BindGroup,
            );
            if let Some(old) = t.compute.replace(ComputeBinding {
                bind,
                scene_generation,
            }) {
                state.retired_binds.push((self.submissions, old));
            }
        }
    }
    pub(super) fn encode_fresh(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[PreparedDraw<'_>],
        slot: usize,
        timed: bool,
    ) {
        let t = self.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let p = self.pipeline.fresh.as_ref().unwrap();
        {
            let attachments = [&t.world, &t.normal].map(|t| {
                Some(wgpu::RenderPassColorAttachment {
                    view: &t.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fresh receiver raster"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().unwrap().view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timing.as_ref().filter(|_| timed).map(|t| {
                    wgpu::RenderPassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: Some(slot as u32 * 10 + 6),
                        end_of_pass_write_index: Some(slot as u32 * 10 + 7),
                    }
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &*self.bind, &[]);

            for draw in draws {
                if draw.slots.is_empty() {
                    continue;
                }
                pass.set_pipeline(&p.receiver);
                pass.set_bind_group(1, &*draw.instances.bind, &[]);
                pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                #[cfg(test)]
                let ids = draw.reference_ids.unwrap_or(&draw.instances.id_buffer);
                #[cfg(not(test))]
                let ids = &draw.instances.id_buffer;
                pass.set_vertex_buffer(1, ids.slice(..));
                pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(draw.indices.clone(), 0, draw.slots.clone());
            }
        }
        self.encode_fresh_compute(
            encoder,
            &p.compute,
            p.compute_tile,
            self.timing
                .as_ref()
                .filter(|_| timed)
                .map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: &t.queries,
                    beginning_of_pass_write_index: Some(slot as u32 * 10 + 8),
                    end_of_pass_write_index: Some(slot as u32 * 10 + 9),
                }),
        );
    }
    /// One visibility dispatch over the current receiver targets. `tile` is the receiver
    /// pixel extent of one pipeline workgroup; tests pass experimental pipelines here.
    pub(super) fn encode_fresh_compute(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        tile: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let t = self.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fresh 256-ray visibility"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*t.compute.as_ref().unwrap().bind, &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(
            t.world.texture.width().div_ceil(tile[0]),
            t.world.texture.height().div_ceil(tile[1]),
            1,
        );
    }
}
