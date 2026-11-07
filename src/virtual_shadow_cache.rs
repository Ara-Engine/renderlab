//! N8n/N8o (docs/non-rt-n8n.md, docs/non-rt-n8o.md): `VirtualLookup::Surface`, the exact
//! lookup's visibility kept per receiver surface cell across frames, so camera motion walks only
//! the pixels beside shadow edges and the cells it has not seen. shaders/virtual_shadow_cache.wgsl
//! describes the cells, which ones a pixel may show, when a cell stays current, and why every
//! write is order-independent; this module owns the table and its claim words, the epochs its
//! frame uniform carries, and the probe, queued-walk and two insert passes.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Hash-table buckets, four 8-byte slots each (32 MiB): about two cells per 1080p pixel.
const BUCKETS: u32 = 1 << 20;
const TABLE_BYTES: u64 = BUCKETS as u64 * 32;
/// N8o: two claim words per slot (WGSL `vsc_claims`), zero between frames.
const CLAIM_BYTES: u64 = BUCKETS as u64 * 4 * 8;
/// WGSL `VscFrame`.
const UNIFORM_BYTES: u64 = 32;
/// Widths of the value's epoch fields (WGSL: sun 10 bits, casters 6); a wrap clears the table,
/// so an old value can never look current.
const SUN_MASK: u32 = (1 << 10) - 1;
const CASTER_MASK: u32 = (1 << 6) - 1;
/// Bytes after the queue's entries: per walk-order bucket a count, offset and cursor, and the
/// walk's chunk cursor (WGSL `vsc_tail`, `vsc_cursor`).
pub(super) const QUEUE_TAIL: u64 = 128;
/// Screen constants of this module (WGSL `override VSC_*`).
pub(super) const CONSTANTS: &str = "VSC_";

/// The exact module with shaders/virtual_shadow_cache.wgsl appended.
pub(super) fn surface_source() -> String {
    format!(
        "{}\n{}",
        shader_source(),
        include_str!("../shaders/virtual_shadow_cache.wgsl")
    )
}

pub(super) struct SurfacePipelines {
    probe: Owned<wgpu::ComputePipeline>,
    args: Owned<wgpu::ComputePipeline>,
    sort: Owned<wgpu::ComputePipeline>,
    missed: Owned<wgpu::ComputePipeline>,
    claim: Owned<wgpu::ComputePipeline>,
    resolve: Owned<wgpu::ComputePipeline>,
}

/// The table and the epochs that decide which of its cells are current. Lives as long as the
/// virtual shadow state, across resource rebuilds; `flush` retires every cell at once.
pub(super) struct SurfaceCache {
    table: Owned<wgpu::Buffer>,
    claims: Owned<wgpu::Buffer>,
    uniform: Owned<wgpu::Buffer>,
    stats: Owned<wgpu::Buffer>,
    /// The queued walk's and the insert pass's indirect dispatches (vsc_args).
    dispatch: Owned<wgpu::Buffer>,
    _allocation: Allocation,
    layout: Owned<wgpu::BindGroupLayout>,
    args_layout: Owned<wgpu::BindGroupLayout>,
    insert_layout: Owned<wgpu::BindGroupLayout>,
    probe_layout: Owned<wgpu::PipelineLayout>,
    args_pipeline_layout: Owned<wgpu::PipelineLayout>,
    insert_pipeline_layout: Owned<wgpu::PipelineLayout>,
    module: Owned<wgpu::ShaderModule>,
    pipelines: Option<SurfacePipelines>,
    built: Option<Vec<(&'static str, f64)>>,
    /// The next encode clears the table first (`flush`).
    clear: AtomicBool,
    frame: u32,
    sun: u32,
    casters: u32,
    last_sun: Option<[u32; 3]>,
    producer: Vec<u64>,
    /// Frames since the sun last turned: cells it left behind refresh on their phase frames in
    /// the 16 after it, so those frames are never idle.
    since_turn: u32,
    /// This frame changes nothing the cells or the target depend on: no pass runs.
    idle: bool,
}

impl SurfaceCache {
    pub(super) fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::ShaderStages as S;
        use wgpu::TextureSampleType as T;
        use wgpu::TextureViewDimension as D;
        let uint = T::Uint;
        let float = T::Float { filterable: false };
        let uniform = |binding, size| {
            layout_entry(
                binding,
                S::COMPUTE,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
            )
        };
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let layout = group(
            "N8n surface cache",
            &[
                frame_uniform(S::COMPUTE),
                sampled(7, uint, D::D2),
                sampled(8, uint, D::D2),
                sampled(9, float, D::D2),
                sampled(10, float, D::D2),
                written(15),
                sampled(18, uint, D::D2),
                storage(20, false, S::COMPUTE),
                uniform(49, UNIFORM_BYTES),
                storage(50, false, S::COMPUTE),
                storage(51, false, S::COMPUTE),
            ],
        );
        let args_layout = group(
            "N8n surface cache dispatch",
            &[
                storage(20, false, S::COMPUTE),
                storage(48, false, S::COMPUTE),
            ],
        );
        // The insert passes read the finished counts, so they bind the target sampled.
        let insert_layout = group(
            "N8n surface cache insert",
            &[
                frame_uniform(S::COMPUTE),
                sampled(7, uint, D::D2),
                sampled(8, uint, D::D2),
                sampled(9, float, D::D2),
                sampled(10, float, D::D2),
                storage(20, false, S::COMPUTE),
                uniform(49, UNIFORM_BYTES),
                storage(50, false, S::COMPUTE),
                storage(51, false, S::COMPUTE),
                sampled(52, uint, D::D2),
                storage(53, false, S::COMPUTE),
            ],
        );
        let pipeline_layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        let probe_layout = pipeline_layout(
            "N8n surface cache ABI",
            &[Some(&layout), None, None, Some(lighting)],
        );
        let args_pipeline_layout =
            pipeline_layout("N8n surface cache dispatch ABI", &[Some(&args_layout)]);
        let insert_pipeline_layout = pipeline_layout(
            "N8n surface cache insert ABI",
            &[Some(&insert_layout), None, None, Some(lighting)],
        );
        let module = ledger.own(
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("N8n surface cache"),
                source: wgpu::ShaderSource::Wgsl(surface_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let buffer = |label, size, usage: wgpu::BufferUsages| {
            ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        use wgpu::BufferUsages as B;
        Self {
            table: buffer("N8n surface cells", TABLE_BYTES, B::STORAGE),
            claims: buffer("N8o surface cell claims", CLAIM_BYTES, B::STORAGE),
            uniform: buffer("N8n surface cache frame", UNIFORM_BYTES, B::UNIFORM),
            stats: buffer("N8n surface cache counters", 32, B::STORAGE | B::COPY_SRC),
            dispatch: buffer("N8n surface cache dispatch", 32, B::STORAGE | B::INDIRECT),
            _allocation: ledger.allocate(
                AllocationKind::Shadow,
                TABLE_BYTES + CLAIM_BYTES + UNIFORM_BYTES + 64,
            ),
            layout,
            args_layout,
            insert_layout,
            probe_layout,
            args_pipeline_layout,
            insert_pipeline_layout,
            module,
            pipelines: None,
            built: None,
            clear: AtomicBool::new(false),
            frame: 0,
            sun: 0,
            casters: 0,
            last_sun: None,
            producer: vec![],
            since_turn: 0,
            idle: false,
        }
    }

    /// A built override's value, or `default` (the WGSL declaration's).
    fn built_constant(&self, name: &str, default: f64) -> f64 {
        self.built
            .iter()
            .flatten()
            .find(|(n, _)| *n == name)
            .map_or(default, |&(_, value)| value)
    }

    /// Retires every cell: the next encode clears the table, so what follows does not depend
    /// on any earlier sequence (N8o: repeated runs and fresh processes give the same cells).
    pub(super) fn flush(&mut self) {
        *self.clear.get_mut() = true;
    }
    /// A restart (`VirtualState::reset`): every cell retired and the frame, sun and caster
    /// counters from zero, so a replayed sequence repeats its refresh schedule.
    pub(super) fn restart(&mut self) {
        self.flush();
        (self.frame, self.sun, self.casters, self.since_turn) = (0, 0, 0, 0);
        self.last_sun = None;
    }

    /// This frame's epochs and uniform. `sun`: the sun direction's bits; `moved`: casters moved
    /// (their bounds are this frame's `VsmFrame::rects`); `flush`: placements changed in a way
    /// the rectangles do not describe; `producer`: whatever else a cell's value depends on (the
    /// receiver producer and its cut, the casters' draw list); `still`: the frame keeps the
    /// last frame's camera, receivers, casters, map and target (N8e's rolling condition).
    pub(super) fn advance(
        &mut self,
        queue: &wgpu::Queue,
        sun: [u32; 3],
        moved: bool,
        flush: bool,
        producer: Vec<u64>,
        still: bool,
    ) {
        self.frame = self.frame.wrapping_add(1);
        let restarted = flush || producer != self.producer;
        if restarted {
            self.flush();
            self.producer = producer;
        }
        let turned = self.last_sun.is_some_and(|s| s != sun);
        if turned {
            self.sun = (self.sun + 1) & SUN_MASK;
            if self.sun == 0 {
                self.flush();
            }
        }
        self.last_sun = Some(sun);
        self.since_turn = if turned {
            0
        } else {
            self.since_turn.saturating_add(1)
        };
        // Nothing a count depends on changed, and no cell is still due for the last turn, so
        // the target already holds this frame's counts.
        self.idle = still && self.since_turn >= 16 && !moved && !restarted;
        let still = still && !restarted;
        if moved {
            self.casters = (self.casters + 1) & CASTER_MASK;
            if self.casters == 0 {
                self.flush();
            }
        }
        let words = [
            self.frame,
            self.sun,
            self.casters,
            u32::from(moved) | u32::from(still) << 1,
            BUCKETS - 1,
            0,
            0,
            0,
        ];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        queue.write_buffer(&self.uniform, 0, &bytes);
    }

    /// Builds (or, for new screen constants, rebuilds) the passes. `constants` are the exact
    /// lookup's and this module's overrides; the queued walk uses the exact lookup's layout.
    pub(super) fn ensure_pipelines(
        &mut self,
        device: &wgpu::Device,
        ledger: &Ledger,
        lookup_layout: &wgpu::PipelineLayout,
        constants: &[(&'static str, f64)],
    ) {
        let constants: Vec<_> = constants
            .iter()
            .copied()
            .filter(|(name, _)| !name.starts_with("VSM_QUEUED") && !name.starts_with("VMAP_"))
            .chain([("VSM_DEBUG", 0.0)])
            .collect();
        if self.pipelines.is_some() && self.built.as_ref() == Some(&constants) {
            return;
        }
        let compute = |entry, layout: &wgpu::PipelineLayout| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &self.module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        self.pipelines = Some(SurfacePipelines {
            probe: compute("vsc_probe", &self.probe_layout),
            args: compute("vsc_args", &self.args_pipeline_layout),
            // Bound with the probe's group: the dispatch group's arguments are its own source.
            sort: compute("vsc_sort", &self.probe_layout),
            missed: compute("vsc_missed", lookup_layout),
            claim: compute("vsc_claim", &self.insert_pipeline_layout),
            resolve: compute("vsc_resolve", &self.insert_pipeline_layout),
        });
        self.built = Some(constants);
    }

    /// The probe, dispatch and insert passes' groups 0, over this frame's targets.
    pub(super) fn groups(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        r: &Resources,
        views: [&wgpu::TextureView; 5],
    ) -> [Owned<wgpu::BindGroup>; 3] {
        let [hit, tiles, world, normal, output] = views;
        let group =
            |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry<'_>]| {
                ledger.own(
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(label),
                        layout,
                        entries,
                    }),
                    ResourceKind::BindGroup,
                )
            };
        [
            group(
                "N8n surface cache",
                &self.layout,
                &[
                    buffer(0, &r.frame),
                    texture(7, hit),
                    texture(8, tiles),
                    texture(9, world),
                    texture(10, normal),
                    texture(15, output),
                    texture(18, &r.level_view),
                    buffer(20, &r.queue),
                    buffer(49, &self.uniform),
                    buffer(50, &self.table),
                    buffer(51, &self.stats),
                ],
            ),
            group(
                "N8n surface cache dispatch",
                &self.args_layout,
                &[buffer(20, &r.queue), buffer(48, &self.dispatch)],
            ),
            group(
                "N8n surface cache insert",
                &self.insert_layout,
                &[
                    buffer(0, &r.frame),
                    texture(7, hit),
                    texture(8, tiles),
                    texture(9, world),
                    texture(10, normal),
                    buffer(20, &r.queue),
                    buffer(49, &self.uniform),
                    buffer(50, &self.table),
                    buffer(51, &self.stats),
                    texture(52, output),
                    buffer(53, &self.claims),
                ],
            ),
        ]
    }

    /// Probe every covered pixel, size the queue's dispatches, sort the queue heaviest first,
    /// walk the misses at their own receiver points, and claim and resolve the results' slots,
    /// in one compute pass.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        r: &Resources,
        groups: &[Owned<wgpu::BindGroup>; 3],
        queued: &wgpu::BindGroup,
        arrays: &wgpu::BindGroup,
        records: &wgpu::BindGroup,
        lighting: &wgpu::BindGroup,
    ) {
        let p = self.pipelines.as_ref().unwrap();
        let [width, height] = r.size;
        if self.idle {
            // Only the timestamps (the frame's shadow span).
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8n surface cache idle"),
                timestamp_writes: timestamps,
            }));
            return;
        }
        if self.clear.swap(false, Ordering::Relaxed) {
            encoder.clear_buffer(&self.table, 0, None);
        }
        encoder.clear_buffer(&r.queue, 0, Some(16));
        // The sort counters and the walk's cursor (vsc_tail, vsc_cursor).
        let tail = 16 + 8 * u64::from(width) * u64::from(height);
        encoder.clear_buffer(&r.queue, tail, Some(QUEUE_TAIL));
        encoder.clear_buffer(&self.stats, 0, None);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N8n surface cache visibility"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&p.probe);
        pass.set_bind_group(0, &*groups[0], &[]);
        pass.set_bind_group(3, lighting, &[]);
        // The probe's workgroup (WGSL VSC_PROBE_X, VSC_PROBE_Y).
        let x = self.built_constant("VSC_PROBE_X", 16.0) as u32;
        let y = self.built_constant("VSC_PROBE_Y", 8.0) as u32;
        pass.dispatch_workgroups(width.div_ceil(x), height.div_ceil(y), 1);
        pass.set_pipeline(&p.args);
        pass.set_bind_group(0, &*groups[1], &[]);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&p.sort);
        pass.set_bind_group(0, &*groups[0], &[]);
        pass.dispatch_workgroups_indirect(&self.dispatch, 16);
        pass.set_pipeline(&p.missed);
        pass.set_bind_group(0, queued, &[]);
        pass.set_bind_group(1, arrays, &[]);
        pass.set_bind_group(2, records, &[]);
        pass.set_bind_group(3, lighting, &[]);
        pass.dispatch_workgroups_indirect(&self.dispatch, 0);
        pass.set_pipeline(&p.claim);
        pass.set_bind_group(0, &*groups[2], &[]);
        pass.set_bind_group(3, lighting, &[]);
        pass.dispatch_workgroups_indirect(&self.dispatch, 16);
        pass.set_pipeline(&p.resolve);
        pass.dispatch_workgroups_indirect(&self.dispatch, 16);
    }

    /// Tests: the last frame's counters (probed receivers, hits, cells the caster test retired,
    /// sun refreshes, cells found but not shown, certified walks past the largest margin, fill
    /// claims; `VSC_STATS` only) and its queued walks.
    #[cfg(test)]
    pub(super) fn read_stats(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        r: &Resources,
    ) -> [u32; 8] {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("N8n surface counters readback"),
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.stats, 0, &staging, 0, 32);
        encoder.copy_buffer_to_buffer(&r.queue, 0, &staging, 32, 16);
        queue.submit([encoder.finish()]);
        staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(20)),
            })
            .unwrap();
        let bytes = staging.slice(..).get_mapped_range().unwrap();
        let word = |i: usize| u32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        [
            word(0),
            word(1),
            word(2),
            word(3),
            word(4),
            word(8),
            word(5),
            word(6),
        ]
    }
}

#[cfg(test)]
impl Renderer {
    /// Tests: `SurfaceCache::read_stats` of the last frame, after a drain.
    pub(crate) fn test_surface_stats(&mut self) -> Option<[u32; 8]> {
        self.test_drain("N8n surface counters");
        let state = self.virtual_shadows.as_ref()?;
        let surface = state.surface.as_ref()?;
        let r = state.resources.as_ref()?;
        Some(surface.read_stats(&self.device, &self.queue, r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each pass uses only bindings its pipeline layout provides (a mismatch fails only at
    /// pipeline creation, inside a GPU lease).
    #[test]
    fn surface_passes_bind_only_their_layouts() {
        let source = surface_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        // (group, binding) of each layout; None: any binding of that group (shared layouts).
        type Layout<'a> = &'a [(u32, Option<&'a [u32]>)];
        let cache: Layout = &[
            (0, Some(&[0, 7, 8, 9, 10, 15, 18, 20, 49, 50, 51])),
            (3, None),
        ];
        let lookup: Layout = &[
            (0, Some(&[0, 7, 8, 9, 10, 15, 16, 18, 19, 20, 24, 35])),
            (1, None),
            (2, None),
            (3, None),
        ];
        let insert: Layout = &[
            (0, Some(&[0, 7, 8, 9, 10, 20, 49, 50, 51, 52, 53])),
            (3, None),
        ];
        let passes: [(&str, Layout); 6] = [
            ("vsc_probe", cache),
            ("vsc_claim", insert),
            ("vsc_resolve", insert),
            ("vsc_args", &[(0, Some(&[20, 48]))]),
            ("vsc_sort", cache),
            ("vsc_missed", lookup),
        ];
        for (entry, layout) in passes {
            let index = module
                .entry_points
                .iter()
                .position(|e| e.name == entry)
                .unwrap();
            let used = info.get_entry_point(index);
            for (handle, global) in module.global_variables.iter() {
                let Some(b) = &global.binding else { continue };
                if used[handle].is_empty() {
                    continue;
                }
                let allowed = layout.iter().any(|&(group, bindings)| {
                    group == b.group && bindings.is_none_or(|list| list.contains(&b.binding))
                });
                assert!(
                    allowed,
                    "{entry} uses @group({}) @binding({}), outside its layout",
                    b.group, b.binding
                );
            }
        }
    }
}
