//! U-RTVIS reflection pass (docs/design-reflection-pass.md): the traces after the frame's
//! shading. With the pass (on by default on the non-RT path; RENDERLAB_RL_PASS=0 forces it off,
//! =1 forces it on RT too), primary shading writes each glossy receiver's lobe (L,
//! shaders/reflections_lobe.wgsl). With the deferred composite (the non-RT default, or
//! RENDERLAB_RL_DEFERRED=1), `rl_trace_pixels` traces every reporting program's glossy pixel's
//! strata into R (misses' sky) and H (hits, shaded by the Mac thread's `reflection_hits`), and the
//! composite adds w x reflectance x their mean before TR1. Without it `rl_trace` traces one ray
//! per 2x2 quad and the picture comes from the inline trace.
//! A child of primary_ray_gpu (its targets, layouts and bindings).
use super::*;

/// Group-0 bindings (U-RTVIS 54-63; the Mac's reflection_hits reads H and R at 64 and 65).
pub(super) const LOBE_A: u32 = 56;
pub(super) const LOBE_B: u32 = 57;
pub(super) const PASS_FRAME: u32 = 58;
const DISTANCE: u32 = 59;
/// The deferred composite's trace (rl_trace_pixels): its counters and per-pixel slot ranges.
pub(super) const STATS: u32 = 60;
/// The same counters in the primary shading layout, where 60 is the resolve's phase uniform
/// (primary_ray_gpu.rs RL_PHASE): the room reservations (`rl_room_reserve`).
pub(super) const ROOM_STATS: u32 = 69;
/// The counters: the trace's four words, the Mac thread's S4a (4, 5), the receivers that did not
/// fit the room (6), and measurement runs' low-weight strata and receivers (7-10).
const STATS_BYTES: u64 = 64;
const RANGES: u32 = 61;
/// The receivers' own ray epsilon (the primary trace's or visibility resolve's per-pixel
/// `receiver.epsilon`): the compacted trace starts its rays there, as the inline loop does.
const RECEIVER_EPSILON: u32 = 79;
/// With the composite, R holds every glossy pixel's strata (this many slots a row) and H one
/// record a slot: the room. It starts at ROOM_START slots and grows to the content (the frame's
/// requested strata, summed at shading, read back a frame or two late) up to ROOM_PER_PIXEL a
/// pixel. A glossy receiver that does not fit keeps its inline estimate that frame
/// (`rl_room_reserve`, reflections_lobe.wgsl), so no frame is ever scaled: a glossier view or
/// a cut costs inline time on its overflow pixels until the room grows, at today's picture.
const SAMPLE_ROW: u32 = 2048;
const ROOM_START: u64 = 1 << 18;
const ROOM_PER_PIXEL: u64 = 4;
/// Measurement: `RENDERLAB_RL_ROOM=fixed:N` pins the room at N slots and never grows it, so
/// glossy receivers past it take the inline estimate every frame (a mixed-estimate row).
fn room_fixed() -> Option<u64> {
    static FIXED: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FIXED.get_or_init(|| {
        let value = std::env::var("RENDERLAB_RL_ROOM").ok()?;
        value.strip_prefix("fixed:")?.parse().ok().filter(|&n: &u64| n > 0)
    })
}
/// Measurement: `RENDERLAB_RL_ROOM_LOG=PATH` appends each read-back frame's counters (requested
/// strata, slots, receivers that did not fit, the room) to PATH, one JSON line a frame.
fn room_log() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var("RENDERLAB_RL_ROOM_LOG").ok()).as_deref()
}
pub(super) const HITS: u32 = 64;
pub(super) const SAMPLES: u32 = 65;
/// H: the count and three dispatch arguments, then 48-byte records.
const HITS_HEADER: u64 = 16;
const RECORD_BYTES: u64 = 48;
const SLOT_GROUP: [u32; 2] = [8, 8];

/// The shading stage's storage buffers and textures the pass and composite need beyond the
/// visibility path's (src/primary_ray.rs visibility_limits): H and the trace counters; the lobes,
/// R and the deferred outputs.
pub(crate) const STORAGE_BUFFERS: u32 = 2;
pub(crate) const STORAGE_TEXTURES: u32 = 12;
/// Whether this device's renderer runs the pass and the composite by default: the non-RT path
/// (no ray queries; its reflections trace in software) on a device whose limits hold them.
pub(crate) fn default_on(device: &wgpu::Device) -> bool {
    let limits = device.limits();
    !device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        && limits.max_storage_buffers_per_shader_stage
            >= crate::visibility_limits(wgpu::Limits::default()).max_storage_buffers_per_shader_stage
                + STORAGE_BUFFERS
        && limits.max_storage_textures_per_shader_stage >= STORAGE_TEXTURES
}
/// Whether the pass runs on this device's renderer: RENDERLAB_RL_PASS when set, else its default.
pub(crate) fn on(device: &wgpu::Device) -> bool {
    crate::program::rl_pass_env().unwrap_or_else(|| default_on(device))
}
/// Whether this device's renderer hands reporting programs' glossy reflections to the composite.
pub(crate) fn deferred(device: &wgpu::Device) -> bool {
    crate::program::rl_deferred_env().unwrap_or_else(|| default_on(device))
}

/// The shading layout's lobe entries (write-only storage), frame uniform and the trace counters
/// (the composite's room reservations, `rl_room_reserve`).
pub(super) fn shade_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
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
        buffer(ROOM_STATS, wgpu::BufferBindingType::Storage { read_only: false }, STATS_BYTES),
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
            // The counters, S4a's `reused` at word 4 among them.
            buffer(STATS, wgpu::BufferBindingType::Storage { read_only: false }, STATS_BYTES),
            image(RANGES, F::R32Uint),
            texture(RECEIVER_EPSILON, wgpu::TextureSampleType::Float { filterable: false }),
        ];
        // The Mac thread's S4a: hits reused before the H append (hit_reuse_gpu.rs).
        entries.extend(super::hit_reuse_gpu::layout_entries());
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
pub(in crate::renderer) struct PassTargets {
    pub lobe_a: Depth,
    pub lobe_b: Depth,
    pub frame: Buffer,
    pub hits: Buffer,
    pub samples: Depth,
    pub distance: Depth,
    pub slots: [u32; 2],
    /// The composite's trace: per-pixel slot ranges, its counters, and the composite's uniform.
    pub ranges: Depth,
    pub stats: Buffer,
    pub composite: Buffer,
    /// The composite's room (R's slots and H's records) and the counters' readback ring.
    pub room: u64,
    pub readback: Vec<Readback>,
}
/// One counters readback: copied on one frame, mapped on the next, read when the map lands.
pub(in crate::renderer) struct Readback {
    buffer: wgpu::Buffer,
    /// 0 idle, 1 copied (map next frame), 2 mapping, 3 mapped.
    state: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// The frame whose counters it holds (RENDERLAB_RL_ROOM_LOG).
    frame: std::sync::atomic::AtomicU64,
}

/// `rl_trace` and `rl_hit_args`, compiled from a head's standard-program traced module.
#[derive(Clone)]
pub(super) struct TracePipelines {
    trace: Owned<wgpu::ComputePipeline>,
    args: Owned<wgpu::ComputePipeline>,
    pixels: Owned<wgpu::ComputePipeline>,
    /// The deferred composite: its group layout and additive render pipeline.
    composite: Option<(Owned<wgpu::BindGroupLayout>, Owned<wgpu::RenderPipeline>)>,
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
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
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
        // The composite traces every glossy pixel's set into a room sized to the content (it
        // starts small and grows); the quad trace keeps one slot and record a quad.
        let composite = deferred(&self.device);
        let room = room_fixed()
            .unwrap_or(ROOM_START)
            .min(ROOM_PER_PIXEL * u64::from(size.width) * u64::from(size.height));
        let samples = if composite {
            [SAMPLE_ROW, (room as u32).div_ceil(SAMPLE_ROW)]
        } else {
            slots
        };
        let records = if composite {
            room
        } else {
            u64::from(slots[0]) * u64::from(slots[1])
        };
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
                HITS_HEADER + records * RECORD_BYTES,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            ),
            samples: target("reflection samples", samples[0], samples[1], F::Rgba16Float),
            // D is the quad trace's alone: one texel under the composite.
            distance: if composite {
                target("reflection ray distance", 1, 1, F::R32Float)
            } else {
                target("reflection ray distance", slots[0], slots[1], F::R32Float)
            },
            slots,
            ranges: target("reflection sample ranges", size.width, size.height, F::R32Uint),
            // Four words, then S4a's `reused` (the room's readback copies the first 16 bytes).
            stats: Buffer::new(
                self,
                "reflection trace counters",
                STATS_BYTES,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            ),
            composite: Buffer::new(
                self,
                "reflection composite frame",
                16,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            room,
            readback: (0..if composite { 3 } else { 0 })
                .map(|_| Readback {
                    buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("reflection trace counters readback"),
                        size: STATS_BYTES,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    state: Default::default(),
                    frame: Default::default(),
                })
                .collect(),
        }
    }

    /// The pass's group 0 for these targets and the frame's ray geometry.
    pub(super) fn rl_trace_group(
        &self,
        abi: &TraceAbi,
        pass: &PassTargets,
        frame: wgpu::BindingResource<'_>,
        epsilon: &wgpu::TextureView,
        scene: (u32, wgpu::BindingResource<'_>),
        reuse: &[(u32, wgpu::BindingResource<'_>)],
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
            (STATS, pass.stats.value.as_entire_binding()),
            (RANGES, R::TextureView(&pass.ranges.view)),
            (RECEIVER_EPSILON, R::TextureView(epsilon)),
            scene,
        ];
        let entries: Vec<_> = entries
            .into_iter()
            .chain(reuse.iter().cloned())
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
        let composite = deferred(&self.device).then(|| self.rl_composite_kit());
        let pipelines = TracePipelines {
            trace: pipeline("rl_trace"),
            args: pipeline("rl_hit_args"),
            pixels: pipeline("rl_trace_pixels"),
            composite,
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
        if pipelines.composite.is_none() {
            self.rl_probe(encoder, pass);
        }
        encoder.clear_buffer(&pass.hits.value, 0, Some(HITS_HEADER));
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection trace"),
            timestamp_writes: None,
        });
        compute.set_bind_group(0, &**group, &[]);
        compute.set_bind_group(1, &*g.arrays, &[]);
        compute.set_bind_group(3, self.lighting.bind(), &[]);
        if pipelines.composite.is_some() {
            // The composite: every glossy pixel's set, one invocation a pixel.
            let size = pass.lobe_b.texture.size();
            let groups = [size.width.div_ceil(8), size.height.div_ceil(8)];
            compute.set_pipeline(&pipelines.pixels);
            compute.dispatch_workgroups(groups[0], groups[1], 1);
            compute.set_pipeline(&pipelines.args);
            compute.dispatch_workgroups(1, 1, 1);
            drop(compute);
            self.rl_probe(encoder, pass);
            self.rl_room_copy(encoder, pass);
            return;
        } else {
            compute.set_pipeline(&pipelines.trace);
            compute.dispatch_workgroups(
                pass.slots[0].div_ceil(SLOT_GROUP[0]),
                pass.slots[1].div_ceil(SLOT_GROUP[1]),
                1,
            );
        }
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
        let composite = u32::from(deferred(&self.device));
        // y: the composite's room (rl_room_reserve), or the quad trace's grid width.
        let y = if composite != 0 { pass.room as u32 } else { pass.slots[0] };
        let words = [self.frames as u32, y, 1, composite];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        self.queue.write_buffer(&pass.frame.value, 0, &bytes);
        // The counters start each frame at zero before shading reserves into them.
        self.queue.write_buffer(&pass.stats.value, 0, &[0; STATS_BYTES as usize]);
        if composite != 0 {
            // The lone-stratum bound and the strata it needs (reflections.wgsl RL_GLOSSY_RAYS).
            let lone = self.view.reflections.frame_words()[2];
            let words = [lone, 4.0, 0.0, 0.0];
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
            self.queue.write_buffer(&pass.composite.value, 0, &bytes);
        }
    }
}

/// rl_resolve's source; `RENDERLAB_RL_RESOLVE_TUNE=cap=C,clamp=K,wide=R` (measurement) replaces
/// its RL_RESOLVE_CAP, RL_RESOLVE_CLAMP and RL_RESOLVE_WIDE.
fn resolve_source() -> String {
    let mut source = include_str!("../shaders/reflections_resolve.wgsl").to_owned();
    let list = std::env::var("RENDERLAB_RL_RESOLVE_TUNE").unwrap_or_default();
    for (key, value) in list.split(',').filter_map(|s| s.trim().split_once('=')) {
        let name = match key {
            "cap" => "RL_RESOLVE_CAP",
            "clamp" => "RL_RESOLVE_CLAMP",
            "wide" => "RL_RESOLVE_WIDE",
            "sigma" => "RL_ATROUS_SIGMA",
            "footprint" => "RL_ATROUS_FOOTPRINT",
            "sharp" => "RL_RESOLVE_SHARP",
            _ => continue,
        };
        let prefix = format!("const {name}:f32=");
        let at = source.find(&prefix).expect("RENDERLAB_RL_RESOLVE_TUNE seam");
        let end = at + source[at..].find(';').unwrap();
        source.replace_range(at..end, &format!("{prefix}{:?}", value.parse::<f32>().unwrap()));
    }
    source
}
/// rl_resolve (RENDERLAB_RL_RESOLVE_PASS=1, shaders/reflections_resolve.wgsl): its layout and
/// kernel, over the reflection history texture and its frame record. No program code.
#[derive(Clone)]
pub(super) struct ResolveKit {
    group: Owned<wgpu::BindGroupLayout>,
    pipeline: Owned<wgpu::ComputePipeline>,
    /// The a-trous passes, steps 1, 2 and 4.
    atrous: [Owned<wgpu::ComputePipeline>; 3],
}
impl ResolveKit {
    pub fn new(device: &PipelineDevice, ledger: &Ledger) -> Self {
        let group = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reflection resolve"),
                entries: &[
                    entry(
                        RL_HISTORY,
                        wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::ReadWrite,
                            format: wgpu::TextureFormat::R32Uint,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                        },
                    ),
                    uniform(RL_HISTORY_FRAME, temporal_reconstruction::RL_HISTORY_BYTES as u64),
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("reflection resolve ABI"),
                bind_group_layouts: &[Some(&group)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("reflection resolve"),
                source: wgpu::ShaderSource::Wgsl(resolve_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let pipeline = |entry| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("reflection resolve"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        Self {
            group,
            pipeline: pipeline("rl_resolve"),
            atrous: ["rl_atrous1", "rl_atrous2", "rl_atrous4"].map(pipeline),
        }
    }
}

impl Renderer {
    pub(super) fn rl_resolve_group(&self, kit: &ResolveKit, history: &RlHistoryTargets) -> Owned<wgpu::BindGroup> {
        self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("reflection resolve"),
                layout: &kit.group,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: RL_HISTORY,
                        resource: wgpu::BindingResource::TextureView(&history.history.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: RL_HISTORY_FRAME,
                        resource: history.frame.value.as_entire_binding(),
                    },
                ],
            }),
            ResourceKind::BindGroup,
        )
    }

    /// rl_resolve after the frame's shading: nothing unless RENDERLAB_RL_RESOLVE_PASS=1.
    pub(crate) fn encode_reflection_resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        let (Some(kit), Some(group), Some(t)) = (
            state.rl_resolve.as_ref(),
            state.bindings.as_ref().and_then(|b| b.rl_resolve.as_ref()),
            state.targets.as_ref(),
        ) else {
            return;
        };
        let size = t.hit.texture.size();
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection resolve"),
            timestamp_writes: None,
        });
        compute.set_bind_group(0, &**group, &[]);
        for pipeline in std::iter::once(&kit.pipeline).chain(&kit.atrous) {
            compute.set_pipeline(pipeline);
            compute.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        }
    }
}

// ---- measurement only: RENDERLAB_RL_PROBE=<json path> ----
/// The frame whose H, R and D the probe reads: copied at the start of the next frame's trace,
/// after every stage of the probed frame (rl_trace, rl_hit_args, reflection_hits) has written them.
const PROBE_FRAME: u64 = 30;
struct Probe {
    staging: wgpu::Buffer,
    rows: [u64; 2],
    slots: [u32; 2],
    path: String,
}
static PROBE: std::sync::Mutex<Option<Probe>> = std::sync::Mutex::new(None);
fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1f);
    let mantissa = f32::from(h & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}
impl Renderer {
    /// Copies the probed frame's H count, R and D (frame PROBE_FRAME + 1), then reads them on the
    /// frame after and writes counts as JSON: slots with a hit, those whose R reflection_hits
    /// wrote (a = 1) and whose radiance is non-zero, and misses with a sky sample.
    fn rl_probe(&self, encoder: &mut wgpu::CommandEncoder, pass: &PassTargets) {
        static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        let Some(path) = PATH.get_or_init(|| std::env::var("RENDERLAB_RL_PROBE").ok()) else {
            return;
        };
        let mut probe = PROBE.lock().unwrap();
        if self.frames == PROBE_FRAME + 1 && probe.is_none() {
            let [w, h] = pass.slots;
            let rows = [u64::from(w * 8).next_multiple_of(256), u64::from(w * 4).next_multiple_of(256)];
            let size = 256 + (rows[0] + rows[1]) * u64::from(h);
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("reflection probe"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&pass.hits.value, 0, &staging, 0, HITS_HEADER);
            // The composite's trace counters (glossy pixels, slots, pixels without room; S4a's
            // reused hits at words 4 and 5).
            encoder.copy_buffer_to_buffer(&pass.stats.value, 0, &staging, 16, STATS_BYTES);
            let composite = deferred(&self.device);
            let extent = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
            // With the composite R is a slot list, not a quad image: only the counters are read.
            for (texture, offset, row) in if composite { vec![] } else { vec![
                (&pass.samples.texture, 256, rows[0]),
                (&pass.distance.texture, 256 + rows[0] * u64::from(h), rows[1]),
            ] } {
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset,
                            bytes_per_row: Some(row as u32),
                            rows_per_image: Some(h),
                        },
                    },
                    extent,
                );
            }
            *probe = Some(Probe { staging, rows, slots: pass.slots, path: path.clone() });
            return;
        }
        if self.frames != PROBE_FRAME + 2 {
            return;
        }
        let Some(p) = probe.take() else {
            return;
        };
        p.staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(20)),
        });
        // wgpu 30 returns the mapped view as a Result; a probe that cannot map reports nothing.
        let Ok(data) = p.staging.slice(..).get_mapped_range() else {
            return;
        };
        let count = u32::from_ne_bytes(data[0..4].try_into().unwrap());
        let word = |i: usize| u32::from_ne_bytes(data[16 + 4 * i..20 + 4 * i].try_into().unwrap());
        let (pixels, slots, requested, dropped) = (word(0), word(1), word(2), word(3));
        // S4a: hits reused before the H append, beside the records appended (`count`).
        let reused = crate::program::hit_reuse()
            .map(|_| {
                format!(
                    ",\"reused\":{},\"appended\":{count},\"hit_reused\":{}",
                    word(4),
                    word(5)
                )
            })
            .unwrap_or_default();
        if deferred(&self.device) {
            // The strata counter: trace slots (strata) and hit records per glossy pixel; the
            // receivers that did not fit the room (inline this frame) and their strata.
            let per = |n: u32| if pixels > 0 { f64::from(n) / f64::from(pixels) } else { 0.0 };
            let overflow_pixels = word(6);
            let json = format!(
                "{{\"frame\":{PROBE_FRAME},\"glossy_pixels\":{pixels},\"strata_requested\":{requested},\"slots\":{slots},\"records\":{count},\"records_dropped\":{dropped},\"overflow_pixels\":{overflow_pixels},\"overflow_strata\":{},\"traced_under_5pc\":{{\"strata\":{},\"pixels\":{}}},\"contribution_under_0005\":{{\"strata\":{},\"pixels\":{}}},\"strata_per_glossy_pixel\":{:.3},\"records_per_glossy_pixel\":{:.3}{reused}}}\n",
                requested.saturating_sub(slots),
                word(7),
                word(8),
                word(9),
                word(10),
                per(slots),
                per(count)
            );
            let _ = std::fs::write(&p.path, json);
            return;
        }
        let [w, h] = p.slots;
        let (mut hits, mut written, mut lit, mut misses, mut sky) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for y in 0..u64::from(h) {
            for x in 0..u64::from(w) {
                let r = (256 + y * p.rows[0] + x * 8) as usize;
                let d = (256 + p.rows[0] * u64::from(h) + y * p.rows[1] + x * 4) as usize;
                let texel: Vec<f32> = (0..4)
                    .map(|i| f16_to_f32(u16::from_ne_bytes(data[r + 2 * i..r + 2 * i + 2].try_into().unwrap())))
                    .collect();
                let distance = f32::from_ne_bytes(data[d..d + 4].try_into().unwrap());
                if distance > 0.0 {
                    hits += 1;
                    written += u64::from(texel[3] == 1.0);
                    lit += u64::from(texel[0] + texel[1] + texel[2] > 0.0);
                } else if distance == -1.0 {
                    misses += 1;
                    sky += u64::from(texel[3] == 1.0);
                }
            }
        }
        let json = format!(
            "{{\"frame\":{PROBE_FRAME},\"records\":{count},\"hit_slots\":{hits},\"hit_written\":{written},\"hit_nonzero\":{lit},\"miss_slots\":{misses},\"miss_sky\":{sky}}}\n"
        );
        let _ = std::fs::write(&p.path, json);
    }
}

impl Renderer {
    /// Sets the resolve's shading phase (0 the pre-pass, 1 the shading pass) for the next
    /// shading dispatch.
    pub(crate) fn encode_reflection_phase(&self, encoder: &mut wgpu::CommandEncoder, phase: u64) {
        let Some(t) = self.view.primary.as_ref().and_then(|s| s.targets.as_ref()) else {
            return;
        };
        encoder.copy_buffer_to_buffer(&t.rl_history.phases.value, 16 * phase, &t.rl_history.phase.value, 0, 16);
    }
}

// ---- the deferred reflection composite (RENDERLAB_RL_DEFERRED=1 with RENDERLAB_RL_PASS=1) ----
impl Renderer {
    /// The composite's group layout and its additive render pipeline into the HDR target.
    fn rl_composite_kit(&self) -> (Owned<wgpu::BindGroupLayout>, Owned<wgpu::RenderPipeline>) {
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let group = self.ledger.own(
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reflection composite"),
                entries: &[
                    texture(0, wgpu::TextureSampleType::Uint),
                    texture(1, wgpu::TextureSampleType::Float { filterable: false }),
                    texture(2, wgpu::TextureSampleType::Float { filterable: false }),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(16),
                        },
                        count: None,
                    },
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = self.ledger.own(
            self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("reflection composite ABI"),
                bind_group_layouts: &[Some(&group)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let module = self.ledger.own(
            self.device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("reflection composite"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/reflections_composite.wgsl").into()),
            }),
            ResourceKind::ShaderModule,
        );
        let pipeline = self.ledger.own(
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("reflection composite"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("rl_composite_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("rl_composite_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        // Colour adds; alpha (TR1's view hint) stays.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::Zero,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        (group, pipeline)
    }

    /// The composite after reflection_hits: each glossy pixel's strata mean, times its
    /// reflectance and w, added into the HDR target before TR1.
    pub(crate) fn encode_reflection_composite(&self, encoder: &mut wgpu::CommandEncoder, shading: Shading) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        let (Some(pipelines), Some(t)) = (state.rl_pipelines[shading as usize].as_ref(), state.targets.as_ref()) else {
            return;
        };
        let (Some((layout, pipeline)), Some(pass), Some(deferred), Some(hdr)) =
            (pipelines.composite.as_ref(), t.rl_pass.as_ref(), t.deferred.as_ref(), self.view.hdr.as_ref())
        else {
            return;
        };
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection composite"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&pass.ranges.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&pass.samples.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&deferred.0.view) },
                wgpu::BindGroupEntry { binding: 3, resource: pass.composite.value.as_entire_binding() },
            ],
        });
        let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("reflection composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &hdr.resolved.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render.set_pipeline(pipeline);
        render.set_bind_group(0, &group, &[]);
        render.draw(0..3, 0..1);
    }
}

// ---- the composite's room sized to the content ----
impl Renderer {
    /// Copies this frame's counters into an idle readback buffer, and maps last frame's copy.
    fn rl_room_copy(&self, encoder: &mut wgpu::CommandEncoder, pass: &PassTargets) {
        use std::sync::atomic::Ordering::SeqCst;
        // Last frame's copies are submitted now: start their maps.
        for r in &pass.readback {
            if r.state.compare_exchange(1, 2, SeqCst, SeqCst).is_ok() {
                let state = r.state.clone();
                r.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    state.store(if result.is_ok() { 3 } else { 0 }, SeqCst);
                });
            }
        }
        if let Some(r) = pass.readback.iter().find(|r| r.state.load(SeqCst) == 0) {
            encoder.copy_buffer_to_buffer(&pass.stats.value, 0, &r.buffer, 0, STATS_BYTES);
            r.frame.store(self.frames, SeqCst);
            r.state.store(1, SeqCst);
        }
    }
    /// Grows the composite's room when a read-back frame asked for more than it holds: R and H
    /// to the next power of two above 1.25 times the request, at most ROOM_PER_PIXEL a pixel; the
    /// frame's bindings are dropped, so it runs once a frame before `prepare_primary` builds them
    /// (the trace and hit groups belong to them; the composite's group is made each frame).
    pub(crate) fn rl_room_grow(&mut self) {
        use std::sync::atomic::Ordering::SeqCst;
        let Some(pass) = self
            .view
            .primary
            .as_ref()
            .and_then(|s| s.targets.as_ref())
            .and_then(|t| t.rl_pass.as_ref())
        else {
            return;
        };
        // Only with the pass: the default path's frame is unchanged.
        let _ = self.device.poll(wgpu::PollType::Poll);
        let mut requested = 0u64;
        for r in &pass.readback {
            if r.state.load(SeqCst) == 3 {
                if let Ok(data) = r.buffer.slice(..).get_mapped_range() {
                    let word = |i: usize| u32::from_ne_bytes(data[4 * i..4 * i + 4].try_into().unwrap());
                    requested = requested.max(u64::from(word(2)));
                    if let Some(path) = room_log() {
                        use std::io::Write;
                        let line = format!(
                            "{{\"frame\":{},\"requested\":{},\"slots\":{},\"overflow_pixels\":{},\"room\":{},\"traced_under_5pc\":[{},{}],\"contribution_under_0005\":[{},{}]}}\n",
                            r.frame.load(SeqCst),
                            word(2),
                            word(1),
                            word(6),
                            pass.room,
                            word(7),
                            word(8),
                            word(9),
                            word(10)
                        );
                        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                            let _ = file.write_all(line.as_bytes());
                        }
                    }
                }
                r.buffer.unmap();
                r.state.store(0, SeqCst);
            }
        }
        let size = pass.lobe_b.texture.size();
        let most = ROOM_PER_PIXEL * u64::from(size.width) * u64::from(size.height);
        if requested <= pass.room || pass.room >= most || room_fixed().is_some() {
            return;
        }
        let room = (requested * 5 / 4).next_power_of_two().min(most);
        let samples = self.rl_room_samples(room);
        let hits = Buffer::new(
            self,
            "reflection hit records",
            HITS_HEADER + room * RECORD_BYTES,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        );
        let state = self.view.primary.as_mut().unwrap();
        let pass = state.targets.as_mut().unwrap().rl_pass.as_mut().unwrap();
        pass.samples = samples;
        pass.hits = hits;
        pass.room = room;
        state.bindings = None;
    }
    fn rl_room_samples(&self, room: u64) -> Depth {
        let width = SAMPLE_ROW;
        let height = (room as u32).div_ceil(SAMPLE_ROW);
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("reflection samples"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(texture.create_view(&Default::default()), ResourceKind::TextureView);
        Depth {
            texture,
            view,
            _allocation: self.ledger.allocate(AllocationKind::Target, u64::from(width) * u64::from(height) * 8),
        }
    }
}
