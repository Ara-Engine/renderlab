//! MP1 (docs/m7-mp1.md): per-program deferred shading of mixed-program traced and
//! visibility-buffer frames. A frame whose draws use one program keeps the single
//! full-region dispatch (`encode_primary_shading`). With K > 1 programs, an engine kernel
//! (shaders/program_bins.wgsl) appends each 4x4 shading tile to the list of every frame
//! slot (program) it holds, and each program shades its list with its own pipeline
//! (shaders/program_traced_list.wgsl) in one indirect dispatch, all in one compute pass.
//! User programs stay separate modules; no uber-shader.
// PC1: list pipelines participate in shared-cache routing and compilation timings.
use super::*;
use crate::pipeline_compilation::PipelineDevice;

/// Programs one frame can shade: the binning mask is one u32 per tile.
pub(crate) const MAX_FRAME_PROGRAMS: usize = 32;
/// Mixed-program binning and list entries retain their own 4x4 shading tiles.
const LIST_TILE: [u32; 2] = [4, 4];
/// WGSL `RlList`: slot, list base, capacity, flags, then the slot of each of the
/// `TRACED_DRAWS` batches.
const INFO_BYTES: u64 = 16 + 4 * TRACED_DRAWS as u64;
/// Indirect arguments per slot: x, y, z and a pad word.
const ARGS_BYTES: u64 = 16;

/// The bind group layouts and pipeline layouts of binning and list shading.
pub(super) struct ListLayouts {
    bins: Owned<wgpu::BindGroupLayout>,
    bins_layout: Owned<wgpu::PipelineLayout>,
    /// The shading group 0 plus the list bindings, indexed by `Shading`.
    shade: [Owned<wgpu::BindGroupLayout>; 2],
    layouts: [Owned<wgpu::PipelineLayout>; 2],
    streamed_layouts: [Owned<wgpu::PipelineLayout>; 2],
}
impl ListLayouts {
    /// `shade` holds the single-program shading group's entries per `Shading`; the other
    /// groups are the shading ABI's (`materials` without and with texture residency).
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        shade: &[Vec<wgpu::BindGroupLayoutEntry>; 2],
        arrays: &wgpu::BindGroupLayout,
        materials: [&wgpu::BindGroupLayout; 2],
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        let rt = device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            let entries: Vec<_> = entries
                .iter()
                .copied()
                .filter(|e| rt || !matches!(e.ty, wgpu::BindingType::AccelerationStructure { .. }))
                .collect();
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries: &entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        let uint = wgpu::TextureSampleType::Uint;
        let write = wgpu::BufferBindingType::Storage { read_only: false };
        let bins = group(
            "program tile binning",
            &[
                texture(0, uint),
                texture(1, uint),
                read(2, 16),
                read(3, 4),
                uniform(4, INFO_BYTES),
                buffer(5, write, 4),
                buffer(6, write, ARGS_BYTES),
            ],
        );
        let shade = shade.each_ref().map(|entries| {
            let mut entries = entries.clone();
            entries.push(entry(
                38,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(INFO_BYTES),
                },
            ));
            entries.push(read(39, 4));
            group("program list shading", &entries)
        });
        let with = |g: &wgpu::BindGroupLayout, m: &wgpu::BindGroupLayout| {
            layout(
                "program list shading ABI",
                &[Some(g), Some(arrays), Some(m), Some(lighting)],
            )
        };
        Self {
            bins_layout: layout("program tile binning ABI", &[Some(&bins)]),
            bins,
            layouts: shade.each_ref().map(|g| with(g, materials[0])),
            streamed_layouts: shade.each_ref().map(|g| with(g, materials[1])),
            shade,
        }
    }
}

/// One mixed frame's program slots and lists, reallocated when the image or the slot
/// count outgrows them.
pub(super) struct ProgramLists {
    /// Allocated slots (a power of two) and tiles per slot list.
    slots: u32,
    capacity: u32,
    /// Dynamic-offset stride of `info`.
    stride: u32,
    info: Buffer,
    entries: Buffer,
    args: Buffer,
    written: Vec<u8>,
    /// The binning group and what it binds: hit records and geometry generations.
    bins: Option<(Owned<wgpu::BindGroup>, (wgpu::Texture, u64))>,
    pub generation: u64,
}
impl ProgramLists {
    /// The list shading group's extra entries, after the single-program shading ones.
    pub fn shade_entries(&self) -> [(u32, wgpu::BindingResource<'_>); 2] {
        [
            (
                38,
                wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.info.value,
                    offset: 0,
                    size: wgpu::BufferSize::new(INFO_BYTES),
                }),
            ),
            (39, self.entries.value.as_entire_binding()),
        ]
    }
}

/// Distinct programs of `programs` (one per draw) in first-draw order, and each draw's
/// slot among them: slot 0 is the first draw's program, which also writes misses.
pub(crate) fn frame_slots(programs: impl IntoIterator<Item = u64>) -> (Vec<u64>, Vec<u32>) {
    let mut distinct: Vec<u64> = vec![];
    let slots = programs
        .into_iter()
        .map(|id| match distinct.iter().position(|&p| p == id) {
            Some(i) => i as u32,
            None => {
                distinct.push(id);
                distinct.len() as u32 - 1
            }
        })
        .collect();
    (distinct, slots)
}

/// Words of one slot list for a `width` x `height` image: its length, then room for
/// every 4x4 shading tile of the trace-tile-widened region and every 16x16 clear block.
fn capacity([width, height]: [u32; 2]) -> u32 {
    1 + (width + TRACE_TILE[0]).div_ceil(LIST_TILE[0])
        * (height + TRACE_TILE[1]).div_ceil(LIST_TILE[1])
        + width.div_ceil(CLEAR_TILE[0]) * height.div_ceil(CLEAR_TILE[1])
}

impl PrimaryState {
    /// A program's list shading pipeline of `Shading` index `shading` from its
    /// `program::compose_list` module, through `cache` when given.
    pub fn list_pipeline(
        &self,
        device: &PipelineDevice,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        shading: usize,
        streamed: bool,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Owned<wgpu::ComputePipeline> {
        let l = &self.list_layouts;
        let layout = if streamed {
            &l.streamed_layouts[shading]
        } else {
            &l.layouts[shading]
        };
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("traced program list shading"),
                layout: Some(layout),
                module,
                entry_point: Some("primary_shade_list"),
                compilation_options: Default::default(),
                cache,
            }),
            ResourceKind::ComputePipeline,
        )
    }
    pub fn list_shade_layout(&self, shading: Shading) -> &wgpu::BindGroupLayout {
        &self.list_layouts.shade[shading as usize]
    }
    /// Whether the prepared frame shades per program (`encode_program_lists`).
    pub fn mixed(&self) -> bool {
        self.programs.len() > 1
    }
}

impl Renderer {
    /// The binning kernel, compiled by the first mixed frame. It holds no program code.
    pub(crate) fn ensure_bins(&mut self) -> Result<(), Error> {
        if self.primary.as_ref().unwrap().bins.is_some() {
            return Ok(());
        }
        let scopes = OperationScopes::new(&self.device);
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("program tile binning"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../shaders/program_bins.wgsl").into(),
                ),
            });
        let state = self.primary.as_ref().unwrap();
        let pipeline = self.ledger.own(
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("program_bins"),
                    layout: Some(&state.list_layouts.bins_layout),
                    module: &module,
                    entry_point: Some("program_bins"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            ResourceKind::ComputePipeline,
        );
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.pipelines.created += 1;
        self.primary.as_mut().unwrap().bins = Some(pipeline);
        Ok(())
    }

    /// Publishes the frame's program slots (`frame_slots` of its draws) and, for a mixed
    /// frame, its list buffers and slot table. Single-program frames release nothing and
    /// write nothing here.
    pub(super) fn prepare_program_lists(
        &mut self,
        programs: Vec<u64>,
        slots: &[u32],
        size: [u32; 2],
        uploads: &mut UploadBytes,
    ) {
        let k = programs.len() as u32;
        self.primary.as_mut().unwrap().programs = programs;
        if k < 2 {
            return;
        }
        let capacity = capacity(size);
        let state = self.primary.as_ref().unwrap();
        if state
            .lists
            .as_ref()
            .is_none_or(|l| l.slots < k || l.capacity < capacity)
        {
            let slots = k.next_power_of_two();
            let align = self.device.limits().min_uniform_buffer_offset_alignment;
            let stride = (INFO_BYTES as u32).div_ceil(align) * align;
            let storage = wgpu::BufferUsages::STORAGE;
            let next = ProgramLists {
                slots,
                capacity,
                stride,
                info: Buffer::new(
                    self,
                    "program list slots",
                    u64::from(slots * stride),
                    wgpu::BufferUsages::UNIFORM,
                ),
                entries: Buffer::new(
                    self,
                    "program tile lists",
                    u64::from(slots) * u64::from(capacity) * 4,
                    storage,
                ),
                args: Buffer::new(
                    self,
                    "program list dispatch arguments",
                    u64::from(slots) * ARGS_BYTES,
                    storage | wgpu::BufferUsages::INDIRECT,
                ),
                written: vec![],
                bins: None,
                generation: state.lists.as_ref().map_or(1, |l| l.generation + 1),
            };
            let state = self.primary.as_mut().unwrap();
            // Queued frames keep the buffers their bind groups hold.
            state.lists = Some(next);
        }
        let lists = self.primary.as_mut().unwrap().lists.as_mut().unwrap();
        let mut bytes = vec![0u8; (lists.slots * lists.stride) as usize];
        for slot in 0..k {
            let at = (slot * lists.stride) as usize;
            let words = [slot, slot * lists.capacity, lists.capacity, 0]
                .into_iter()
                .chain((0..TRACED_DRAWS as usize).map(|b| slots.get(b).copied().unwrap_or(0)));
            for (i, w) in words.enumerate() {
                bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&w.to_ne_bytes());
            }
        }
        if lists.written != bytes {
            self.queue.write_buffer(&lists.info.value, 0, &bytes);
            uploads.materials += bytes.len() as u64;
            uploads.write_calls += 1;
            lists.written = bytes;
        }
    }

    /// The binning group of a prepared mixed frame: after its targets and tables exist.
    pub(super) fn prepare_bins_group(&mut self) {
        let state = self.primary.as_ref().unwrap();
        let Some(lists) = state.lists.as_ref().filter(|_| state.mixed()) else {
            return;
        };
        let t = state.targets.as_ref().unwrap();
        let key = ((*t.hit.texture).clone(), state.geometry_generation);
        if lists.bins.as_ref().is_some_and(|(_, k)| *k == key) {
            return;
        }
        use wgpu::BindingResource as R;
        let g = state.geometry.as_ref().unwrap();
        let entries = [
            (0, R::TextureView(&t.hit.view)),
            (1, R::TextureView(&t.tiles.view)),
            (
                2,
                state
                    .region_buffer
                    .as_ref()
                    .unwrap()
                    .value
                    .as_entire_binding(),
            ),
            (3, g._placements.value.as_entire_binding()),
            (
                4,
                R::Buffer(wgpu::BufferBinding {
                    buffer: &lists.info.value,
                    offset: 0,
                    size: wgpu::BufferSize::new(INFO_BYTES),
                }),
            ),
            (5, lists.entries.value.as_entire_binding()),
            (6, lists.args.value.as_entire_binding()),
        ]
        .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
        let group = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("program tile binning"),
                layout: &state.list_layouts.bins,
                entries: &entries,
            }),
            ResourceKind::BindGroup,
        );
        self.primary.as_mut().unwrap().lists.as_mut().unwrap().bins = Some((group, key));
    }

    /// Mixed-frame deferred shading: binning, then each program's list, in one pass.
    pub(super) fn encode_program_lists(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.primary.as_ref().unwrap();
        let lists = state.lists.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let group = state.bindings.as_ref().unwrap().list[shading as usize]
            .as_ref()
            .unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        // Arguments and each list's length start at zero; the binning pass counts them.
        encoder.clear_buffer(&lists.args.value, 0, None);
        for k in 0..state.programs.len() as u64 {
            let at = k * u64::from(lists.capacity) * 4;
            encoder.clear_buffer(&lists.entries.value, at, Some(4));
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("traced deferred shading, per program"),
            timestamp_writes,
        });
        pass.set_pipeline(state.bins.as_ref().unwrap());
        pass.set_bind_group(0, &*lists.bins.as_ref().unwrap().0, &[]);
        let [x, y] = state.region_groups(LIST_TILE);
        let [bx, by] = [0, 1].map(|i| [size.width, size.height][i].div_ceil(CLEAR_TILE[i]));
        pass.dispatch_workgroups(x.max(bx), y + by, 1);
        pass.set_bind_group(1, &*g.arrays, &[]);
        pass.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        for (k, &id) in state.programs.iter().enumerate() {
            let list = &self.program_slot(id).pipelines.list[usize::from(state.requests)];
            pass.set_pipeline(list[shading as usize].as_ref().unwrap());
            pass.set_bind_group(0, &**group, &[k as u32 * lists.stride]);
            pass.dispatch_workgroups_indirect(&lists.args.value, k as u64 * ARGS_BYTES);
        }
    }
}

#[cfg(test)]
#[path = "program_lists_tests.rs"]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_slots_follow_first_draw_order() {
        let (programs, slots) = frame_slots([7, 0, 7, 3, 0]);
        assert_eq!(programs, [7, 0, 3]);
        assert_eq!(slots, [0, 1, 0, 2, 1]);
        let (programs, slots) = frame_slots([]);
        assert!(programs.is_empty() && slots.is_empty());
    }

    #[test]
    fn list_capacity_holds_every_region_tile_and_clear_block() {
        for size in [[1920u32, 1080], [1, 1], [961, 641], [3840, 2160]] {
            // The widest region: the image widened to whole trace tiles.
            let region = [0, 1].map(|i| size[i].div_ceil(TRACE_TILE[i]) * TRACE_TILE[i]);
            let tiles = region[0] / LIST_TILE[0] * (region[1] / LIST_TILE[1]);
            let blocks = size[0].div_ceil(CLEAR_TILE[0]) * size[1].div_ceil(CLEAR_TILE[1]);
            assert!(capacity(size) > tiles + blocks, "{size:?}");
        }
        // Entries pack tile coordinates in 15 bits each.
        assert!(capacity([16384, 16384]) > 0 && (16384u32 + 8) / 4 < 1 << 15);
    }

    /// The binning kernel and a program's list module validate, and the list module of a
    /// reflecting program shades other programs' hits with its own parameters.
    #[test]
    fn list_compositions_validate() {
        let validate = |label: &str, source: &str| {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        };
        validate("bins", include_str!("../shaders/program_bins.wgsl"));
        let prelude = include_str!("../shaders/scene.wgsl");
        let heads = shading_heads(prelude).unwrap();
        for (label, program) in [
            ("standard", crate::program::STANDARD_PROGRAM),
            ("toon", include_str!("../shaders/programs/toon.wgsl")),
        ] {
            let parameters = crate::program::declares_parameters(program);
            for (i, head) in heads.iter().enumerate() {
                for stage in [
                    crate::program::Stage::Traced,
                    crate::program::Stage::TracedStreamed,
                ] {
                    let list = crate::program::compose_list(head, program, stage, parameters);
                    assert_eq!(&list.source[list.program.clone()], program);
                    validate(&format!("{label} {i} {stage:?}"), &list.source);
                    let single = crate::program::compose(head, program, stage, parameters).source;
                    assert!(list.source.contains("fn primary_shade_list("));
                    assert!(!single.contains("primary_shade_list"));
                    assert_eq!(
                        list.source.contains("rl_slot_of(h.batch)"),
                        single.contains("rl_load_parameters(h.batch)"),
                        "{label} {i}"
                    );
                }
            }
        }
        let non_rt = non_rt_sources(prelude).unwrap().1;
        for head in &non_rt {
            let program = crate::program::STANDARD_PROGRAM;
            let list =
                crate::program::compose_list(head, program, crate::program::Stage::Traced, false);
            validate("standard non-RT", &list.source);
        }
    }
}
