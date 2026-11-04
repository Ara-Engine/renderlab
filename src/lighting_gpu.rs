use super::*;
use crate::lighting::{DFG, DFG_SIZE, PackedLighting, dirty_span};

pub(super) struct LightBuffer {
    shadow_generation: u64,
    environment_generation: u64,
    _environment: Option<EnvironmentMap>,
    buffer: Owned<wgpu::Buffer>,
    bind: Owned<wgpu::BindGroup>,
    _allocation: Allocation,
}
pub(super) struct Hdr {
    pub resolved: Depth,
    pub msaa: Option<Depth>,
    pub bind: Owned<wgpu::BindGroup>,
    pub exposure: f32,
    pub frame: FrameId,
    pub revision: Revision,
    pub shader_hash: [u8; 32],
    pub lighting_hash: [u8; 32],
}
pub(super) struct LightingGpu {
    pub bindings: Owned<wgpu::BindGroupLayout>,
    pub layout: Owned<wgpu::PipelineLayout>,
    pub present_bindings: Owned<wgpu::BindGroupLayout>,
    pub present_layout: Owned<wgpu::PipelineLayout>,
    uniform: Owned<wgpu::Buffer>,
    _uniform_allocation: Allocation,
    dfg_view: Owned<wgpu::TextureView>,
    _dfg: Owned<wgpu::Texture>,
    _dfg_allocation: Allocation,
    sampler: Owned<wgpu::Sampler>,
    dummy_environment: crate::environment::EnvironmentCube,
    active: Option<LightBuffer>,
    retired: Vec<(u64, LightBuffer)>,
    pub retired_hdr: Vec<(u64, Hdr)>,
    pub hdr: Option<Hdr>,
    pub(super) pending: PackedLighting,
    pub(super) source_lights: Vec<Light>,
    published: Vec<u8>,
    uniform_shadow: Option<[u8; 48]>,
    pub max: u32,
    pub generation: u64,
    pub target_generation: u64,
}
impl LightingGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Ledger,
        groups: [&wgpu::BindGroupLayout; 3],
    ) -> Self {
        let uniform = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lighting eye/count/environment/exposure"),
                size: 48,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lighting group 3"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(48),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(64),
                        },
                        count: None,
                    },
                    texture_entry(2),
                    cube_entry(6),
                    cube_entry(7),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(crate::shadows::VIEW_BYTES),
                        },
                        count: None,
                    },
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lit mesh ABI"),
                bind_group_layouts: &[
                    Some(groups[0]),
                    Some(groups[1]),
                    Some(groups[2]),
                    Some(&bindings),
                ],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let present_bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("resolved exposed HDR"),
                entries: &[texture_entry(1)],
            }),
            ResourceKind::BindGroupLayout,
        );
        let present_layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("presentation ABI"),
                bind_group_layouts: &[Some(&present_bindings)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let dfg = ledger.own(
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("prepared DFG RG16F"),
                size: wgpu::Extent3d {
                    width: DFG_SIZE,
                    height: DFG_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        queue.write_texture(
            dfg.as_image_copy(),
            DFG,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(DFG_SIZE * 4),
                rows_per_image: Some(DFG_SIZE),
            },
            dfg.size(),
        );
        let dfg_view = ledger.own(
            dfg.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let sampler = ledger.own(
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("DFG bilinear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        let dummy_environment = super::environment_gpu::dummy_cube(device, queue, ledger);
        let limits = device.limits();
        Self {
            bindings,
            layout,
            present_bindings,
            present_layout,
            uniform,
            _uniform_allocation: ledger.allocate(AllocationKind::Light, 48),
            dfg_view,
            _dfg: dfg,
            _dfg_allocation: ledger.allocate(AllocationKind::Lut, DFG.len() as u64),
            sampler,
            dummy_environment,
            active: None,
            retired: vec![],
            retired_hdr: vec![],
            hdr: None,
            pending: PackedLighting::new(&Lighting::default(), u32::MAX).unwrap(),
            source_lights: Vec::new(),
            published: vec![],
            uniform_shadow: None,
            max: (limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size)
                / 64)
                .min(u64::from(u32::MAX)) as u32,
            generation: 0,
            target_generation: 0,
        }
    }
    pub fn bind(&self) -> &wgpu::BindGroup {
        &self.active.as_ref().unwrap().bind
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
        self.retired_hdr.retain(|(last, _)| *last > completed);
    }
}
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
impl Renderer {
    #[cfg(test)]
    pub(crate) fn test_lighting_busy(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        mesh: &Mesh,
        instances: &mut Instances,
    ) {
        self.test_drain("lighting admission setup");
        for slot in &mut self.slots {
            slot.flight = Some(Flight {
                done: Arc::new(OnceLock::new()),
                mapped: None,
                lit: false,
                shadowed: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at: Instant::now(),
            });
        }
        let before = self.stats();
        for energy in [20., 30.] {
            self.set_lighting(&Lighting {
                lights: vec![Light::Directional {
                    travel: -Vec3::Z,
                    lux: [energy; 3],
                }],
                ..Default::default()
            })
            .unwrap();
        }
        assert!(
            self.try_render_view(
                color,
                camera,
                &mut [Draw::new(mesh, instances)],
                ViewMode::Lit
            )
            .unwrap()
            .is_none()
        );
        let after = self.stats();
        assert_eq!(before.uploads, after.uploads);
        assert_eq!(before.ownership, after.ownership);
        assert_eq!(before.pipelines, after.pipelines);
        for slot in &self.slots {
            slot.flight.as_ref().unwrap().done.set(()).unwrap();
        }
        self.collect().unwrap();
        let report = self
            .try_render_view(
                color,
                camera,
                &mut [Draw::new(mesh, instances)],
                ViewMode::Lit,
            )
            .unwrap()
            .unwrap();
        assert!(report.uploads.lights > 0);
        assert_eq!(report.uploads.light_writes, 1);
        assert_eq!(report.uploads.models, 0);
        assert_eq!(report.uploads.geometry, 0);
        assert_eq!(self.pipelines, before.pipelines);
        self.test_drain("lighting admission cleanup");
        let report = self
            .try_render_view(
                color,
                camera,
                &mut [Draw::new(mesh, instances)],
                ViewMode::Lit,
            )
            .unwrap()
            .unwrap();
        assert_eq!(report.uploads.lights, 0);
        assert_eq!(report.uploads.lighting, 0);
    }
    /// Atomic CPU replacement. No queue writes until an admitted Lit submission.
    pub fn set_lighting(&mut self, lighting: &Lighting) -> Result<(), Error> {
        self.check_fault()?;
        self.validate_shadow_settings(&lighting.lights, &self.shadows.settings)?;
        let exposure = PackedLighting::settings(lighting)?;
        if lighting.lights == self.lighting.source_lights {
            // Settings-only edits never normalize/repack the light vector.
            self.lighting.pending.environment = lighting.environment;
            self.lighting.pending.exposure = exposure;
        } else {
            let packed = PackedLighting::new(lighting, self.max_lights())?;
            self.lighting.pending = packed;
            self.lighting.source_lights.clone_from(&lighting.lights);
        }
        Ok(())
    }
    pub fn max_lights(&self) -> u32 {
        self.lighting.max
    }
    pub fn lighting_stats(&self) -> LightingStats {
        LightingStats {
            max_lights: self.max_lights(),
            lights: self.lighting.pending.count,
            buffer_generation: self.lighting.generation,
            target_generation: self.lighting.target_generation,
            retained_light_generations: self.lighting.retired.len(),
            retained_target_generations: self.lighting.retired_hdr.len(),
            lut_bytes: DFG.len() as u64,
        }
    }
    pub(super) fn upload_lighting(&mut self, eye: Vec3, uploads: &mut UploadBytes) {
        let state = &mut self.lighting;
        let needed = state.pending.records.len().max(64) as u64;
        if state.active.as_ref().is_none_or(|active| {
            active.buffer.size() < needed
                || active.shadow_generation != self.shadows.stats.resource_generation
                || active.environment_generation != self.environment.generation
        }) {
            // Exact growth is device bounded; no next_power_of_two overshoot.
            let buffer = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("light records"),
                    size: needed,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let bind = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("lighting"),
                    layout: &state.bindings,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::TextureView(
                                self.environment
                                    .selected
                                    .as_ref()
                                    .map_or(&state.dummy_environment.view, |map| {
                                        &map.0.diffuse.view
                                    }),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(
                                self.environment
                                    .selected
                                    .as_ref()
                                    .map_or(&state.dummy_environment.view, |map| {
                                        &map.0.specular.view
                                    }),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: state.uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&state.dfg_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&state.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(
                                self.shadows.sample_view(),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: self.shadows.records().as_entire_binding(),
                        },
                    ],
                }),
                ResourceKind::BindGroup,
            );
            let next = LightBuffer {
                shadow_generation: self.shadows.stats.resource_generation,
                environment_generation: self.environment.generation,
                _environment: self.environment.selected.clone(),
                buffer,
                bind,
                _allocation: self.ledger.allocate(AllocationKind::Light, needed),
            };
            if let Some(old) = state.active.replace(next) {
                state.retired.push((self.submissions, old));
            }
            state.generation += 1;
            state.published.clear();
        }
        if let Some(span) = dirty_span(&state.published, &state.pending.records) {
            self.queue.write_buffer(
                &state.active.as_ref().unwrap().buffer,
                span.start as u64,
                &state.pending.records[span.clone()],
            );
            uploads.lights += (span.end - span.start) as u64;
            uploads.light_writes += 1;
            uploads.write_calls += 1;
        }
        if state.published != state.pending.records {
            state.published.clone_from(&state.pending.records);
        }
        let mut uniform = [0; 48];
        uniform[..32].copy_from_slice(&state.pending.uniform(eye));
        uniform[32..].copy_from_slice(&self.environment.uniform());
        if let Some(span) = dirty_span(
            state.uniform_shadow.as_ref().map_or(&[], |b| b.as_slice()),
            &uniform,
        ) {
            self.queue
                .write_buffer(&state.uniform, span.start as u64, &uniform[span.clone()]);
            uploads.lighting += (span.end - span.start) as u64;
            uploads.lighting_writes += 1;
            uploads.write_calls += 1;
        }
        state.uniform_shadow = Some(uniform);
    }
    pub(super) fn ensure_hdr(&mut self, color: &wgpu::Texture) {
        if self
            .lighting
            .hdr
            .as_ref()
            .is_some_and(|h| h.resolved.texture.size() == color.size())
        {
            return;
        }
        let resolved = self.hdr_target(color.size(), 1);
        let msaa =
            (self.options.samples == SampleCount::Four).then(|| self.hdr_target(color.size(), 4));
        let bind = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("HDR presentation"),
                layout: &self.lighting.present_bindings,
                entries: &[wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&resolved.view),
                }],
            }),
            ResourceKind::BindGroup,
        );
        let hdr = Hdr {
            resolved,
            msaa,
            bind,
            exposure: 1.0,
            frame: FrameId(0),
            revision: self.revision,
            shader_hash: self.shader_hash,
            lighting_hash: [0; 32],
        };
        if let Some(old) = self.lighting.hdr.replace(hdr) {
            self.lighting.retired_hdr.push((self.submissions, old));
        }
        self.lighting.target_generation += 1;
    }
    fn hdr_target(&self, size: wgpu::Extent3d, samples: u32) -> Depth {
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("exposed linear HDR"),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | if samples == 1 {
                        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(
            texture.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        Depth {
            texture,
            view,
            _allocation: self.ledger.allocate(
                AllocationKind::Target,
                u64::from(size.width) * u64::from(size.height) * 8 * u64::from(samples),
            ),
        }
    }
    pub(super) fn lit_clear(&self) -> [f64; 3] {
        self.lighting
            .pending
            .environment
            .map(|v| (f64::from(v) * f64::from(self.lighting.pending.exposure)).min(65504.0))
    }
    pub(super) fn stamp_hdr(&mut self, frame: FrameId) {
        if let Some(hdr) = &mut self.lighting.hdr {
            hdr.exposure = self.lighting.pending.exposure;
            hdr.frame = frame;
            hdr.revision = self.revision;
            hdr.shader_hash = self.shader_hash;
            let mut hash = Sha256::new();
            hash.update(&self.lighting.pending.records);
            hash.update(self.lighting.pending.uniform(Vec3::ZERO));
            hash.update(self.environment.uniform());
            if let Some(map) = &self.environment.selected {
                hash.update(map.0.hash);
            }
            if self.shadows.settings.enabled() {
                hash.update(self.shadows.published_hash);
            }
            hdr.lighting_hash = hash.finalize().into();
        }
    }
}

fn cube_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    let mut entry = texture_entry(binding);
    entry.ty = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::Cube,
        multisampled: false,
    };
    entry
}
