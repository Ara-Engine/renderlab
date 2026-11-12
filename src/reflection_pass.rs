//! U-RTVIS reflection pass, first stage (docs/design-reflection-pass.md): `rl_trace` after the
//! frame's shading. While `RENDERLAB_RL_PASS=1`, primary shading writes each glossy receiver's
//! lobe (L, shaders/reflections_lobe.wgsl), and `rl_trace` (shaders/reflections_trace.wgsl)
//! traces one ray per 2x2 quad from it. It writes misses' sky radiance into R and appends hit
//! records to H for the Mac thread's `reflection_hits`, then writes H's dispatch arguments. The
//! picture still comes from the inline trace; the accumulator and resolve come later.
//! A child of primary_ray_gpu (its targets, layouts and bindings).
use super::*;

/// Group-0 bindings (U-RTVIS 54-63; the Mac's reflection_hits reads H and R at 64 and 65).
pub(super) const LOBE_A: u32 = 56;
pub(super) const LOBE_B: u32 = 57;
pub(super) const PASS_FRAME: u32 = 58;
const DISTANCE: u32 = 59;
pub(super) const HITS: u32 = 64;
pub(super) const SAMPLES: u32 = 65;
/// H: the count and three dispatch arguments, then 48-byte records.
const HITS_HEADER: u64 = 16;
const RECORD_BYTES: u64 = 48;
const SLOT_GROUP: [u32; 2] = [8, 8];

/// `RENDERLAB_RL_PASS=1` (src/program.rs).
pub(crate) fn on() -> bool {
    crate::program::rl_pass_on()
}

/// The shading layout's lobe entries (write-only storage) and frame uniform.
pub(super) fn shade_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
    use wgpu::TextureFormat as F;
    let write = |binding, format| {
        entry(
            binding,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        )
    };
    [
        write(LOBE_A, F::Rgba32Float),
        write(LOBE_B, F::Rgba32Uint),
        uniform(PASS_FRAME, 16),
    ]
}

/// `rl_trace`'s group 0 and pipeline layout: the frame uniform, the shared TLAS (RT) or the
/// software reflection geometry (non-RT), L, the pass uniform, D, H and R; groups 1 and 3 are
/// the traced geometry tables and the lighting.
#[derive(Clone)]
pub(super) struct TraceAbi {
    group: Owned<wgpu::BindGroupLayout>,
    layout: Owned<wgpu::PipelineLayout>,
}
impl TraceAbi {
    pub fn new(
        device: &PipelineDevice,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::TextureFormat as F;
        let ray_query = device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let mut entries = vec![
            uniform(10, FRAME_BYTES as u64),
            texture(LOBE_A, wgpu::TextureSampleType::Float { filterable: false }),
            texture(LOBE_B, wgpu::TextureSampleType::Uint),
            uniform(PASS_FRAME, 16),
            image(DISTANCE, F::R32Float),
            buffer(
                HITS,
                wgpu::BufferBindingType::Storage { read_only: false },
                HITS_HEADER + RECORD_BYTES,
            ),
            image(SAMPLES, F::Rgba16Float),
        ];
        entries.push(if ray_query {
            scene(2)
        } else {
            read(SOFTWARE_REFLECTIONS, 16)
        });
        let group = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reflection trace"),
                entries: &entries,
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("reflection trace ABI"),
                bind_group_layouts: &[Some(&group), Some(arrays), None, Some(lighting)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        Self { group, layout }
    }
}

/// The pass's per-view resources: L (full resolution), the pass uniform, H, R and D (one texel
/// per 2x2 quad).
pub(super) struct PassTargets {
    pub lobe_a: Depth,
    pub lobe_b: Depth,
    pub frame: Buffer,
    pub hits: Buffer,
    pub samples: Depth,
    pub distance: Depth,
    pub slots: [u32; 2],
}

/// `rl_trace` and `rl_hit_args`, compiled from a head's standard-program traced module.
#[derive(Clone)]
pub(super) struct TracePipelines {
    trace: Owned<wgpu::ComputePipeline>,
    args: Owned<wgpu::ComputePipeline>,
}

impl Renderer {
    /// The pass targets for a `size` view.
    pub(super) fn rl_pass_targets(&self, size: wgpu::Extent3d) -> PassTargets {
        use wgpu::TextureFormat as F;
        let target = |label, width: u32, height: u32, format: wgpu::TextureFormat| {
            let texture = self.ledger.own(
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = self.ledger.own(texture.create_view(&Default::default()), ResourceKind::TextureView);
            let bytes = u64::from(width) * u64::from(height) * u64::from(format.block_copy_size(None).unwrap());
            Depth {
                texture,
                view,
                _allocation: self.ledger.allocate(AllocationKind::Target, bytes),
            }
        };
        let slots = [size.width.div_ceil(2), size.height.div_ceil(2)];
        PassTargets {
            lobe_a: target("reflection lobe A", size.width, size.height, F::Rgba32Float),
            lobe_b: target("reflection lobe B", size.width, size.height, F::Rgba32Uint),
            frame: Buffer::new(
                self,
                "reflection pass frame",
                16,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            hits: Buffer::new(
                self,
                "reflection hit records",
                HITS_HEADER + u64::from(slots[0]) * u64::from(slots[1]) * RECORD_BYTES,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            ),
            samples: target("reflection samples", slots[0], slots[1], F::Rgba16Float),
            distance: target("reflection ray distance", slots[0], slots[1], F::R32Float),
            slots,
        }
    }

    /// The pass's group 0 for these targets and the frame's ray geometry.
    pub(super) fn rl_trace_group(
        &self,
        abi: &TraceAbi,
        pass: &PassTargets,
        frame: wgpu::BindingResource<'_>,
        scene: (u32, wgpu::BindingResource<'_>),
    ) -> Owned<wgpu::BindGroup> {
        use wgpu::BindingResource as R;
        let entries = [
            (10, frame),
            (LOBE_A, R::TextureView(&pass.lobe_a.view)),
            (LOBE_B, R::TextureView(&pass.lobe_b.view)),
            (PASS_FRAME, pass.frame.value.as_entire_binding()),
            (DISTANCE, R::TextureView(&pass.distance.view)),
            (HITS, pass.hits.value.as_entire_binding()),
            (SAMPLES, R::TextureView(&pass.samples.view)),
            scene,
        ];
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
            .collect();
        self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("reflection trace"),
                layout: &abi.group,
                entries: &entries,
            }),
            ResourceKind::BindGroup,
        )
    }

    /// Compiles the pass's kernels for `shading`'s head the first time a frame needs them.
    pub(crate) fn ensure_reflection_trace(&mut self, shading: Shading) -> Result<(), Error> {
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        let Some(abi) = state.rl_trace.as_ref() else {
            return Ok(());
        };
        if state.rl_pipelines[shading as usize].is_some() {
            return Ok(());
        }
        let head = &self.primary_sources.as_ref().unwrap().1[shading as usize];
        let source = crate::program::rl_trace_source(head);
        let scopes = OperationScopes::new(&self.device);
        let module = self.ledger.own(
            self.device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("reflection trace"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }),
            ResourceKind::ShaderModule,
        );
        let pipeline = |entry| {
            self.ledger.own(
                self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("reflection trace"),
                    layout: Some(&abi.layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let pipelines = TracePipelines {
            trace: pipeline("rl_trace"),
            args: pipeline("rl_hit_args"),
        };
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(format!("reflection trace: {error}")));
        }
        self.view.primary.as_mut().unwrap().rl_pipelines[shading as usize] = Some(pipelines);
        Ok(())
    }

    /// `rl_trace` then `rl_hit_args`, after the frame's shading; nothing while the pass is off.
    pub(crate) fn encode_reflection_trace(&self, encoder: &mut wgpu::CommandEncoder, shading: Shading) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        let (Some(pipelines), Some(pass), Some(group)) = (
            state.rl_pipelines[shading as usize].as_ref(),
            state.targets.as_ref().and_then(|t| t.rl_pass.as_ref()),
            state.bindings.as_ref().and_then(|b| b.rl_trace.as_ref()),
        ) else {
            return;
        };
        let g = state.geometry.as_ref().unwrap();
        encoder.clear_buffer(&pass.hits.value, 0, Some(HITS_HEADER));
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection trace"),
            timestamp_writes: None,
        });
        compute.set_bind_group(0, &**group, &[]);
        compute.set_bind_group(1, &*g.arrays, &[]);
        compute.set_bind_group(3, self.lighting.bind(), &[]);
        compute.set_pipeline(&pipelines.trace);
        compute.dispatch_workgroups(
            pass.slots[0].div_ceil(SLOT_GROUP[0]),
            pass.slots[1].div_ceil(SLOT_GROUP[1]),
            1,
        );
        compute.set_pipeline(&pipelines.args);
        compute.dispatch_workgroups(1, 1, 1);
    }

    /// This frame's pass uniform: the frame counter (the lobes' tag), the slot grid's width,
    /// and the pass on.
    pub(crate) fn write_reflection_pass_frame(&self) {
        let Some(pass) = self
            .view
            .primary
            .as_ref()
            .and_then(|s| s.targets.as_ref())
            .and_then(|t| t.rl_pass.as_ref())
        else {
            return;
        };
        let words = [self.frames as u32, pass.slots[0], 1, 0];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        self.queue.write_buffer(&pass.frame.value, 0, &bytes);
    }
}
