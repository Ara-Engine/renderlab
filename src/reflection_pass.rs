//! U-RTVIS reflection pass and deferred composite (docs/design-reflection-pass.md), on by default on
//! the non-RT path (RENDERLAB_RL_PASS=0 forces it off, =1 with RENDERLAB_RL_DEFERRED=1 forces it on
//! RT too). Primary shading appends each reporting program's glossy receiver that fits the room to
//! the frame's glossy-pixel list (L, shaders/reflections_lobe.wgsl); `rl_trace_pixels` traces each
//! entry's strata into R (misses' sky) and H (hits, shaded by the Mac thread's
//! `reflection_hits`), and the composite adds w x reflectance x their mean before TR1. Every stage
//! after shading is sized from the list's length (indirect), so a view with little to reflect
//! costs little.
//! A child of primary_ray_gpu (its targets, layouts and bindings).
use super::*;

/// Group-0 bindings (U-RTVIS 54-63; the Mac's reflection_hits reads H and R at 64 and 65).
/// L, the glossy-pixel list: a 16-byte header (its length), then LIST_ENTRY_BYTES an entry.
pub(super) const LIST: u32 = 56;
const LIST_HEADER: u64 = 16;
const LIST_ENTRY_BYTES: u64 = 48;
pub(super) const PASS_FRAME: u32 = 58;
/// The list's indirect arguments: the trace's dispatch, then the composite's draw (32 bytes).
/// The trace counters (rl_trace_pixels).
pub(super) const STATS: u32 = 60;
/// The same counters in the primary shading layout, where 60 is the resolve's phase uniform
/// (primary_ray_gpu.rs RL_PHASE): the room reservations (`rl_room_reserve`).
pub(super) const ROOM_STATS: u32 = 69;
/// The counters: the trace's four words, the Mac thread's S4a (4, 5), the receivers that did not
/// fit the room (6), and measurement runs' low-weight strata and receivers (7-10).
const STATS_BYTES: u64 = 64;
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
/// strata, slots, receivers that did not fit, the room) and the device's memory at that moment
/// (hal buffer and texture bytes and object counts, the allocator's bytes) to PATH, one JSON
/// line a frame: an in-clip creep shows which kind of resource grows.
fn room_log() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var("RENDERLAB_RL_ROOM_LOG").ok()).as_deref()
}
pub(super) const HITS: u32 = 64;
pub(super) const SAMPLES: u32 = 65;
/// H: the count and three dispatch arguments, then 48-byte records.
const HITS_HEADER: u64 = 16;
const RECORD_BYTES: u64 = 48;

/// The shading stage's storage buffers and textures the pass and composite need beyond the
/// visibility path's (src/primary_ray.rs visibility_limits): H and the trace counters; the lobes,
/// R and the deferred outputs.
pub(crate) const STORAGE_BUFFERS: u32 = 2;
pub(crate) const STORAGE_TEXTURES: u32 = 12;
/// Whether the non-RT path runs the pass by default. Off on both paths while S1's timing and
/// pictures are pending (root, 2026-11-13); `RENDERLAB_RL_PASS=1 RENDERLAB_RL_DEFERRED=1` runs it.
/// Turning it on is this one line.
pub(crate) const NON_RT_DEFAULT: bool = false;
/// Whether this device's renderer runs the pass and the composite by default: the non-RT path
/// (no ray queries; its reflections trace in software) on a device whose limits hold them, while
/// `NON_RT_DEFAULT` is on.
pub(crate) fn default_on(device: &wgpu::Device) -> bool {
    let limits = device.limits();
    NON_RT_DEFAULT
        && !device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        && limits.max_storage_buffers_per_shader_stage
            >= crate::visibility_limits(wgpu::Limits::default()).max_storage_buffers_per_shader_stage
                + STORAGE_BUFFERS
        && limits.max_storage_textures_per_shader_stage >= STORAGE_TEXTURES
}
/// Whether the pass runs on this device's renderer: RENDERLAB_RL_PASS when set, else its default;
/// it is the deferred composite's, so it runs only with it.
pub(crate) fn on(device: &wgpu::Device) -> bool {
    crate::program::rl_pass_env().unwrap_or_else(|| default_on(device)) && deferred(device)
}
/// Whether this device's renderer hands reporting programs' glossy reflections to the composite.
pub(crate) fn deferred(device: &wgpu::Device) -> bool {
    crate::program::rl_deferred_env().unwrap_or_else(|| default_on(device))
}

/// The shading layout's entries: the glossy-pixel list, the frame uniform and the trace counters
/// (the composite's room reservations, `rl_room_reserve`).
pub(super) fn shade_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
    let write = wgpu::BufferBindingType::Storage { read_only: false };
    [
        buffer(LIST, write, LIST_HEADER + LIST_ENTRY_BYTES),
        uniform(PASS_FRAME, 16),
        buffer(ROOM_STATS, write, STATS_BYTES),
    ]
}

/// The trace's group 0 and pipeline layout: the frame uniform, the shared TLAS (RT) or the
/// software reflection geometry (non-RT), L (each entry carries its receiver's epsilon), the
/// pass uniform, H, R and the counters; groups 1 and 3 are the traced geometry tables and the
/// lighting.
/// `rl_list_args` alone also binds group 2, the list's indirect arguments (`args_layout`): the
/// trace's indirect dispatch reads that buffer, so no other dispatch may bind it read-write.
#[derive(Clone)]
pub(super) struct TraceAbi {
    group: Owned<wgpu::BindGroupLayout>,
    layout: Owned<wgpu::PipelineLayout>,
    args_group: Owned<wgpu::BindGroupLayout>,
    args_layout: Owned<wgpu::PipelineLayout>,
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
        let write = wgpu::BufferBindingType::Storage { read_only: false };
        let mut entries = vec![
            uniform(10, FRAME_BYTES as u64),
            read(LIST, LIST_HEADER + LIST_ENTRY_BYTES),
            uniform(PASS_FRAME, 16),
            buffer(HITS, write, HITS_HEADER + RECORD_BYTES),
            image(SAMPLES, F::Rgba16Float),
            buffer(STATS, write, 16),
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
        let args_group = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reflection list arguments"),
                entries: &[buffer(0, write, 32)],
            }),
            ResourceKind::BindGroupLayout,
        );
        let args_layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("reflection list arguments ABI"),
                bind_group_layouts: &[Some(&group), Some(arrays), Some(&args_group), Some(lighting)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        Self { group, layout, args_group, args_layout }
    }
}

/// The pass's per-view resources: L (the glossy-pixel list) and its indirect arguments, the pass
/// uniform, H and R (the room), the counters and the composite's uniform.
pub(in crate::renderer) struct PassTargets {
    pub list: Buffer,
    pub list_args: Buffer,
    pub frame: Buffer,
    pub hits: Buffer,
    pub samples: Depth,
    /// The view's size (the room's cap is ROOM_PER_PIXEL of its pixels).
    pub size: [u32; 2],
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

/// `rl_list_args`, `rl_trace_pixels` and `rl_hit_args`, compiled from a head's standard-program
/// traced module, and the composite.
#[derive(Clone)]
pub(super) struct TracePipelines {
    list_args: Owned<wgpu::ComputePipeline>,
    pixels: Owned<wgpu::ComputePipeline>,
    args: Owned<wgpu::ComputePipeline>,
    /// The deferred composite: its group layout and additive point pipeline.
    composite: (Owned<wgpu::BindGroupLayout>, Owned<wgpu::RenderPipeline>),
}

impl Renderer {
    /// The pass targets for a `size` view.
    pub(super) fn rl_pass_targets(&self, size: wgpu::Extent3d) -> PassTargets {
        // The room (R's slots and H's records) starts small and grows to the content; the list
        // holds as many entries as the room has slots (every entry takes one at least).
        let room = room_fixed()
            .unwrap_or(ROOM_START)
            .min(ROOM_PER_PIXEL * u64::from(size.width) * u64::from(size.height));
        PassTargets {
            list: self.rl_list(room),
            list_args: Buffer::new(
                self,
                "reflection list arguments",
                32,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            ),
            frame: Buffer::new(
                self,
                "reflection pass frame",
                16,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            hits: Buffer::new(
                self,
                "reflection hit records",
                HITS_HEADER + room * RECORD_BYTES,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            ),
            samples: self.rl_room_samples(room),
            size: [size.width, size.height],
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
            readback: (0..3)
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
    /// The glossy-pixel list for a room of `room` slots.
    fn rl_list(&self, room: u64) -> Buffer {
        Buffer::new(
            self,
            "reflection glossy-pixel list",
            LIST_HEADER + room * LIST_ENTRY_BYTES,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                // The glossy-list dump's copy (usage only, while it is set).
                | if super::reflection_list_dump::enabled() {
                    wgpu::BufferUsages::COPY_SRC
                } else {
                    wgpu::BufferUsages::empty()
                },
        )
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
            (LIST, pass.list.value.as_entire_binding()),
            (PASS_FRAME, pass.frame.value.as_entire_binding()),
            (HITS, pass.hits.value.as_entire_binding()),
            (SAMPLES, R::TextureView(&pass.samples.view)),
            (STATS, pass.stats.value.as_entire_binding()),
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
        let pipeline = |entry, layout: &wgpu::PipelineLayout| {
            self.ledger.own(
                self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("reflection trace"),
                    layout: Some(layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let pipelines = TracePipelines {
            list_args: pipeline("rl_list_args", &abi.args_layout),
            pixels: pipeline("rl_trace_pixels", &abi.layout),
            args: pipeline("rl_hit_args", &abi.layout),
            composite: self.rl_composite_kit(),
        };
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(format!("reflection trace: {error}")));
        }
        self.view.primary.as_mut().unwrap().rl_pipelines[shading as usize] = Some(pipelines);
        Ok(())
    }

    /// After the frame's shading: the list's arguments, `rl_trace_pixels` over the list (indirect),
    /// then `rl_hit_args`; nothing while the pass is off.
    pub(crate) fn encode_reflection_trace(&self, encoder: &mut wgpu::CommandEncoder, shading: Shading) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        let (Some(pipelines), Some(pass), Some(group), Some(abi)) = (
            state.rl_pipelines[shading as usize].as_ref(),
            state.targets.as_ref().and_then(|t| t.rl_pass.as_ref()),
            state.bindings.as_ref().and_then(|b| b.rl_trace.as_ref()),
            state.rl_trace.as_ref(),
        ) else {
            return;
        };
        let g = state.geometry.as_ref().unwrap();
        encoder.clear_buffer(&pass.hits.value, 0, Some(HITS_HEADER));
        let bind = |compute: &mut wgpu::ComputePass<'_>| {
            compute.set_bind_group(0, &**group, &[]);
            compute.set_bind_group(1, &*g.arrays, &[]);
            compute.set_bind_group(3, self.lighting.bind(), &[]);
        };
        // The arguments first, in their own pass and group: the trace reads them as indirect
        // arguments. One entry, rebuilt each frame (the buffer outlives room growth).
        let args = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection list arguments"),
            layout: &abi.args_group,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pass.list_args.value.as_entire_binding(),
            }],
        });
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection list arguments"),
            timestamp_writes: None,
        });
        bind(&mut compute);
        compute.set_bind_group(2, &args, &[]);
        compute.set_pipeline(&pipelines.list_args);
        compute.dispatch_workgroups(1, 1, 1);
        drop(compute);
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection trace"),
            timestamp_writes: None,
        });
        bind(&mut compute);
        compute.set_pipeline(&pipelines.pixels);
        compute.dispatch_workgroups_indirect(&pass.list_args.value, 0);
        compute.set_pipeline(&pipelines.args);
        compute.dispatch_workgroups(1, 1, 1);
        drop(compute);
        super::reflection_list_dump::maybe_dump(self, encoder, pass);
        self.rl_probe(encoder, pass);
        self.rl_room_copy(encoder, pass);
    }

    /// This frame's pass uniform (the frame counter, the room, the list's capacity, the pass on),
    /// the counters and the list's length cleared before shading, and the composite's uniform.
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
        let words = [self.frames as u32, pass.room as u32, pass.room as u32, 1];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        self.queue.write_buffer(&pass.frame.value, 0, &bytes);
        // The counters and the list's length start each frame at zero before shading.
        self.queue.write_buffer(&pass.stats.value, 0, &[0; STATS_BYTES as usize]);
        self.queue.write_buffer(&pass.list.value, 0, &[0; LIST_HEADER as usize]);
        // The lone-stratum bound and the strata it needs (reflections.wgsl RL_GLOSSY_RAYS).
        let lone = self.view.reflections.frame_words()[2];
        let words = [lone, 4.0, 0.0, 0.0];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        self.queue.write_buffer(&pass.composite.value, 0, &bytes);
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
    path: String,
}
static PROBE: std::sync::Mutex<Option<Probe>> = std::sync::Mutex::new(None);
impl Renderer {
    /// Copies the probed frame's H count and counters (frame PROBE_FRAME + 1, after its trace),
    /// then reads them on the frame after and writes them as JSON.
    fn rl_probe(&self, encoder: &mut wgpu::CommandEncoder, pass: &PassTargets) {
        static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        let Some(path) = PATH.get_or_init(|| std::env::var("RENDERLAB_RL_PROBE").ok()) else {
            return;
        };
        let mut probe = PROBE.lock().unwrap();
        if self.frames == PROBE_FRAME + 1 && probe.is_none() {
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("reflection probe"),
                size: 256,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&pass.hits.value, 0, &staging, 0, HITS_HEADER);
            encoder.copy_buffer_to_buffer(&pass.stats.value, 0, &staging, 16, STATS_BYTES);
            *probe = Some(Probe { staging, path: path.clone() });
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
        // The strata counter: trace slots (strata) and hit records per glossy pixel; the
        // receivers that did not fit the room (inline this frame) and their strata.
        let per = |n: u32| if pixels > 0 { f64::from(n) / f64::from(pixels) } else { 0.0 };
        let overflow_pixels = word(6);
        let json = format!(
            "{{\"frame\":{PROBE_FRAME},\"glossy_pixels\":{pixels},\"strata_requested\":{requested},\"slots\":{slots},\"records\":{count},\"records_dropped\":{dropped},\"overflow_pixels\":{overflow_pixels},\"overflow_strata\":{},\"floored_inline\":{{\"strata\":{},\"pixels\":{}}},\"contribution_under_0005\":{{\"strata\":{},\"pixels\":{}}},\"strata_per_glossy_pixel\":{:.3},\"records_per_glossy_pixel\":{:.3}}}\n",
            requested.saturating_sub(slots),
            word(7),
            word(8),
            word(9),
            word(10),
            per(slots),
            per(count)
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
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let texture = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let group = self.ledger.own(
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reflection composite"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: both,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(LIST_HEADER + LIST_ENTRY_BYTES),
                        },
                        count: None,
                    },
                    texture(1, wgpu::ShaderStages::FRAGMENT),
                    texture(2, both),
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
                source: wgpu::ShaderSource::Wgsl(
                    crate::program::rl_pass_switches(include_str!("../shaders/reflections_composite.wgsl").to_owned()).into(),
                ),
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
                // One point a list entry, at its pixel's centre.
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::PointList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        (group, pipeline)
    }

    /// The composite after reflection_hits: each listed glossy pixel's strata mean, times its
    /// reflectance and w, added into the HDR target before TR1 (one point an entry, the draw's
    /// count from the list's arguments).
    pub(crate) fn encode_reflection_composite(&self, encoder: &mut wgpu::CommandEncoder, shading: Shading) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        let (Some(pipelines), Some(t)) = (state.rl_pipelines[shading as usize].as_ref(), state.targets.as_ref()) else {
            return;
        };
        let (layout, pipeline) = &pipelines.composite;
        let (Some(pass), Some(deferred), Some(hdr)) = (t.rl_pass.as_ref(), t.deferred.as_ref(), self.view.hdr.as_ref()) else {
            return;
        };
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection composite"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: pass.list.value.as_entire_binding() },
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
        render.draw_indirect(&pass.list_args.value, 16);
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
    /// Grows the composite's room when a read-back frame asked for more than it holds: R, H and
    /// the list to the next power of two above 1.25 times the request, at most ROOM_PER_PIXEL a
    /// pixel. The room is part of the frame bindings' key, so `prepare_primary`, which runs
    /// after this each frame, rebuilds the groups that hold them (the composite's group is made
    /// each frame) without dropping the bindings: dropping them restarted TR1 on every pixel,
    /// and every growth frame showed the composite's raw per-frame grain.
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
                        let hal = self.device.get_internal_counters().hal;
                        let allocator = self
                            .device
                            .generate_allocator_report()
                            .map_or((0, 0), |a| (a.total_allocated_bytes, a.total_reserved_bytes));
                        let memory = format!(
                            ",\"buffer_bytes\":{},\"texture_bytes\":{},\"buffers\":{},\"textures\":{},\"bind_groups\":{},\"allocations\":{},\"allocated_bytes\":{},\"reserved_bytes\":{}",
                            hal.buffer_memory.read(),
                            hal.texture_memory.read(),
                            hal.buffers.read(),
                            hal.textures.read(),
                            hal.bind_groups.read(),
                            hal.memory_allocations.read(),
                            allocator.0,
                            allocator.1
                        );
                        let line = format!(
                            "{{\"frame\":{},\"requested\":{},\"slots\":{},\"overflow_pixels\":{},\"room\":{},\"floored_inline\":[{},{}],\"contribution_under_0005\":[{},{}]{memory}}}\n",
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
        let most = ROOM_PER_PIXEL * u64::from(pass.size[0]) * u64::from(pass.size[1]);
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
        let list = self.rl_list(room);
        let state = self.view.primary.as_mut().unwrap();
        let pass = state.targets.as_mut().unwrap().rl_pass.as_mut().unwrap();
        pass.samples = samples;
        pass.hits = hits;
        pass.list = list;
        pass.room = room;
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
