//! The world radiance cache's kernels, composed as the wgpu renderer composes them.
//!
//! Sky visibility first (`IndirectSettings` with bounce and the filter off): visible
//! pixels request cells, claims insert them, a scan refreshes the rest on a rotation,
//! hardware rays resolve each cell's direction bits and an integration turns the bits
//! into the occluded sky's SH. [`Cache`] is what a frame sequence keeps: the table and
//! work buffers on the GPU, and the wgpu renderer's own planner for what each frame runs.
use super::{
    bindings::HeapBindings,
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    kernel::Kernel,
    shader::{InputKey, Pass, Program},
};
use crate::renderer::gi_cache::{
    self as shared, FrameInputs, Plan, PlanConfig, Planner, Sizes, WorkLayout,
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{Access, GpuDevice, Resource, commands::Command, commands::Recorder};

/// The cache's entries, in `GiState::encode` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    /// Visible pixels look their cells up and request the missing ones.
    Probe,
    /// Bounce: queued cells gather last frame's outgoing radiance along their blocked
    /// directions, and may request the cells their rays hit.
    Gather,
    /// One round of `atomicMax` claims for requested cells.
    Claim,
    /// The winner of each claim writes its cell; losers retry next round.
    Resolve,
    /// The third round's resolve, which also closes the frame's requests.
    ResolveLast,
    /// Queues new and invalidated cells, and the rotation's share of the rest.
    Scan,
    /// Lays the queued cells' visibility rays out as ray items.
    Setup,
    /// One visibility ray per item, in parallel.
    Rays,
    /// Folds the ray results into each cell's direction bits.
    Finish,
    /// The occluded sky as fixed-weight L2 SH per cell.
    Integrate,
    /// The cell-space filter over the frame's written values.
    Filter,
    /// Bounce: light visibility at the query points of the cells to shade.
    Light,
    /// Bounce: the material program shades the queued cells' query points. Composed
    /// with a program ([`shade_program`]), not part of the kernel module.
    ShadeCells,
}
impl Entry {
    /// The kernel module's entries, in encode order: sky visibility is these without
    /// `Gather`, `Filter` and `Light`.
    pub const ALL: [Self; 12] = [
        Self::Probe,
        Self::Gather,
        Self::Claim,
        Self::Resolve,
        Self::ResolveLast,
        Self::Scan,
        Self::Setup,
        Self::Rays,
        Self::Finish,
        Self::Integrate,
        Self::Filter,
        Self::Light,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Probe => "gi_probe",
            Self::Gather => "gi_gather",
            Self::Filter => "gi_filter",
            Self::Light => "gi_light",
            Self::ShadeCells => "gi_shade_cells",
            Self::Claim => "gi_claim",
            Self::Resolve => "gi_resolve",
            Self::ResolveLast => "gi_resolve_last",
            Self::Scan => "gi_scan",
            Self::Setup => "gi_setup",
            Self::Rays => "gi_rays",
            Self::Finish => "gi_finish",
            Self::Integrate => "gi_integrate",
        }
    }
    /// The hardware-ray kernel module with the default direction count, and this
    /// entry selected.
    pub fn program(self) -> Result<Program> {
        self.program_for(crate::IndirectSettings::default().directions)
    }
    /// The same for `directions` fixed directions per cell (64 or 128).
    pub fn program_for(self, directions: u32) -> Result<Program> {
        ensure!(
            self != Self::ShadeCells,
            "cell shading is composed with a material program"
        );
        Program::new(Pass::GiCache(self), source(directions)?)
    }
}

/// `gi_shade_cells` for `material`, composed as `Renderer::gi_shade_pipeline` composes
/// it on a hardware-ray renderer: the soft shading head, the cell stage, and the
/// direction count the kernels' cold layout uses.
pub fn shade_program(material: &str, directions: u32) -> Result<Program> {
    Program::new(
        Pass::GiCache(Entry::ShadeCells),
        shade_source(material, directions)?,
    )
}
/// `material`'s cell shading in a frame of several programs, where it holds `slot`:
/// `shade_program` shading only the cells whose draw selects that slot
/// (`gi_cache::mixed_cell_entry`).
pub fn mixed_shade_program(material: &str, directions: u32, slot: u32) -> Result<Program> {
    Program::new(
        Pass::GiCache(Entry::ShadeCells),
        shared::mixed_cell_entry(&shade_source(material, directions)?, u64::from(slot)),
    )
}
pub(crate) fn shade_source(material: &str, directions: u32) -> Result<String> {
    let [_, head] =
        crate::renderer::primary_ray_gpu::shading_heads(include_str!("../../shaders/scene.wgsl"))?;
    let composed = crate::program::compose(
        &head,
        material,
        crate::program::Stage::Cell,
        crate::program::declares_parameters(material),
    )
    .source;
    let line = "const GI_DIRECTIONS:u32=64u;";
    ensure!(
        composed.matches(line).count() == 1,
        "cell shading's direction count seam changed"
    );
    Ok(composed.replace(line, &format!("const GI_DIRECTIONS:u32={directions}u;")))
}

/// `gi_cache::kernel_source` for hardware rays, with its one hit-position intrinsic
/// replaced by a fetch from the module's own per-draw vertex and index arrays (the
/// hit's placement names its draw, as `gi_receiver` already resolves it), and the
/// vertex-return qualifiers dropped. Traversal and everything else are unchanged.
pub(crate) fn source(directions: u32) -> Result<String> {
    let original = shared::kernel_source(false, directions);
    ensure!(
        original
            .matches("getCommittedHitVertexPositions(&query)")
            .count()
            == 1
            && original.matches("HitVertexPositions").count() == 1
            && original.matches("ray_query<vertex_return>").count() == 2
            && original
                .matches("acceleration_structure<vertex_return>")
                .count()
                == 1,
        "original radiance cache hit-position seams changed"
    );
    Ok(format!(
        "{}\n{FETCH}",
        original
            .replace(
                "getCommittedHitVertexPositions(&query)",
                "gi_hit_vertices(primary_placements[hit.instance_custom_data]&0x1fffffffu,hit.primitive_index)",
            )
            .replace("ray_query<vertex_return>", "ray_query")
            .replace("acceleration_structure<vertex_return>", "acceleration_structure")
    ))
}
/// Object-space corners of one triangle from the retained 48-byte vertex records, three
/// vec4 per vertex with the position first, in the build's primitive order.
const FETCH: &str = "fn gi_hit_vertices(batch:u32,primitive:u32)->array<vec3<f32>,3> {
    let i=primitive*3u;
    return array<vec3<f32>,3>(gi_vertices[batch].data[gi_indices[batch].data[i]*3u].xyz,
        gi_vertices[batch].data[gi_indices[batch].data[i+1u]*3u].xyz,
        gi_vertices[batch].data[gi_indices[batch].data[i+2u]*3u].xyz);
}
";

/// What a frame sequence asks of the cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Indirect {
    /// Enabled, on hardware rays. Bounce and the cell filter follow the settings.
    pub settings: crate::IndirectSettings,
    /// Copy the whole table out after each frame, as `Renderer::indirect_table` does,
    /// and with bounce every slot's outgoing radiance (`Renderer::indirect_outgoing`).
    pub read_table: bool,
}

/// What the planner needs from one frame's scene.
pub(crate) struct CacheScene<'a> {
    pub camera: crate::Camera,
    pub size: [u32; 2],
    /// Every placement's world bounds, in draw order then instance order.
    pub bounds: Vec<[f32; 6]>,
    /// Each draw's instance count and program slot (its place among the frame's
    /// programs, as `Cache::new` took them), in draw order. The counts' sum is `bounds`'
    /// length.
    pub draws: Vec<[u32; 2]>,
    /// `scene_pixels`: the scene's pixel rectangle.
    pub region: [u32; 4],
    /// Placement model matrices, then their normal transforms, as the traced tables
    /// publish them.
    pub models: &'a [u8],
    pub normals: &'a [u8],
    /// A digest that changes exactly when the environment's content or rotation does.
    pub environment: [u8; 32],
    /// An environment map is selected.
    pub field: bool,
    pub lights: u32,
    /// A digest of everything cell shading reads besides the cache: the lights, the
    /// environment and the program. Bounce relights when it changes.
    pub light_key: [u8; 32],
}

/// One frame's records and its plan; the packets borrow the records.
pub(crate) struct CacheFrame {
    record: StorageBuffer,
    draws: StorageBuffer,
    models: StorageBuffer,
    /// None: the cache is idle this frame and no pass runs.
    plan: Option<Plan>,
}
impl CacheFrame {
    pub(crate) fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for buffer in [self.record, self.draws, self.models] {
            buffer.retire(gpu)?;
        }
        Ok(())
    }
}

/// The cache across frames: `GiState`'s table, work words, outgoing radiance and
/// filter sources, the planner's history, the kernel module's twelve entries and, with
/// bounce, cell shading composed with the standard program.
pub struct Cache {
    settings: crate::IndirectSettings,
    slots: u32,
    sizes: Sizes,
    table: StorageBuffer,
    work: StorageBuffer,
    outgoing: StorageBuffer,
    raw: StorageBuffer,
    /// wgpu zero-initialises its buffers; these are filled on their first frame.
    zeroed: bool,
    planner: Planner,
    stages: Vec<(Entry, Program, Kernel)>,
    /// Cell shading, with bounce: one program for a frame of one program, or one per
    /// reflecting program of a frame of several, each shading its own draws' cells.
    shade: Vec<(Program, Kernel)>,
    /// Each program slot's word in the draw table: zero in a frame of one program and
    /// for a program that reflects nothing, else `cell_selector` of the slot.
    selectors: Vec<u32>,
    /// A digest of the programs the cell shading was composed from.
    programs: [u8; 32],
}
/// Cell shading's programs, and each program slot's word in the draw table.
type CellPrograms = (Vec<(Program, Kernel)>, Vec<u32>);
/// Cell shading for a frame's programs, in slot order, and each slot's word in the
/// draw table.
///
/// `Renderer::ensure_gi_shade`: a frame of one program shades its cells with it. A
/// frame of several (MP1) shades each draw's cells with the draw's own program, one
/// cell program per program that reflects; cells of the others reflect nothing. Some
/// program must read the cache and some program must reflect into it.
fn cell_programs(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    settings: &crate::IndirectSettings,
    materials: &[&str],
) -> Result<CellPrograms> {
    ensure!(
        materials.iter().any(|m| crate::program::uses_indirect(m))
            && materials.iter().any(|m| crate::program::reflects(m)),
        "none of the frame's programs reads the cache, or none reflects into it"
    );
    let mixed = materials.len() > 1;
    let mut shade = Vec::new();
    let mut selectors = vec![0; materials.len()];
    for (slot, material) in materials.iter().enumerate() {
        if !settings.bounce || !crate::program::reflects(material) {
            continue;
        }
        let program = if mixed {
            selectors[slot] = shared::cell_selector(slot as u64);
            mixed_shade_program(material, settings.directions, slot as u32)?
        } else {
            shade_program(material, settings.directions)?
        };
        let kernel = Kernel::from_program(gpu, cache, &program)?;
        shade.push((program, kernel));
    }
    Ok((shade, selectors))
}
/// A digest of a frame's programs, in slot order.
fn programs_digest(materials: &[&str]) -> [u8; 32] {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for material in materials {
        hash.update((material.len() as u64).to_le_bytes());
        hash.update(material.as_bytes());
    }
    hash.finalize().into()
}
impl Cache {
    /// Allocates the table for `1 << slots_log2` slots (no device limit shrinks it
    /// here) and a light bitset for `lights`, and compiles the kernels.
    pub fn new(
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        settings: crate::IndirectSettings,
        lights: u32,
        materials: &[&str],
    ) -> Result<Self> {
        settings
            .validate()
            .map_err(|e| anyhow::anyhow!("indirect settings: {e}"))?;
        ensure!(
            settings.enabled && !settings.force_software,
            "the cache is assembled on hardware rays only"
        );

        let slots = 1u32 << settings.slots_log2;
        let sizes = Sizes::of(slots, lights, &settings);
        let stages = Entry::ALL
            .into_iter()
            .map(|entry| {
                let program = entry.program_for(settings.directions)?;
                let kernel = Kernel::from_program(gpu, cache, &program)?;
                Ok((entry, program, kernel))
            })
            .collect::<Result<Vec<_>>>()?;
        let (shade, selectors) = cell_programs(gpu, cache, &settings, materials)?;
        Ok(Self {
            settings,
            slots,
            table: StorageBuffer::device(gpu, sizes.table)?,
            work: StorageBuffer::device(gpu, sizes.work)?,
            outgoing: StorageBuffer::device(gpu, sizes.outgoing)?,
            raw: StorageBuffer::device(gpu, sizes.raw)?,
            sizes,
            zeroed: false,
            planner: Planner::default(),
            stages,
            shade,
            selectors,
            programs: programs_digest(materials),
        })
    }
    pub(crate) fn settings(&self) -> crate::IndirectSettings {
        self.settings
    }
    /// Composes the cell shading again when the frame's programs, in slot order, are
    /// not the ones it was composed from. The table and everything in it stay, as the
    /// renderer remakes its cell pipelines and keeps its cache.
    pub(crate) fn shade_with(
        &mut self,
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        materials: &[&str],
    ) -> Result<()> {
        let digest = programs_digest(materials);
        if self.programs != digest {
            (self.shade, self.selectors) = cell_programs(gpu, cache, &self.settings, materials)?;
            self.programs = digest;
        }
        Ok(())
    }
    /// The table, for shading (lighting binding 20) and for reading back.
    pub(crate) fn table(&self) -> &StorageBuffer {
        &self.table
    }
    /// Every slot's outgoing radiance (bounce), for reading back.
    pub(crate) fn outgoing(&self) -> Option<&StorageBuffer> {
        self.settings.bounce.then_some(&self.outgoing)
    }
    /// The filter's sources (22 words a slot: hot words 1 to 22 before filtering), for
    /// reading back; None unless the filter runs.
    pub(crate) fn raw(&self) -> Option<&StorageBuffer> {
        (self.settings.bounce && self.settings.filter != 0).then_some(&self.raw)
    }
    /// `Renderer::reset_indirect`: the next frame starts from an empty table.
    pub fn reset(&mut self) {
        self.planner.reset();
    }
    /// `prepare_gi` for a traced frame: plans it, writes the table header and uploads
    /// the frame's records. The fills and the header update are recorded into `rec`.
    pub(crate) fn prepare(
        &mut self,
        gpu: &mut GpuDevice,
        graph: &mut Graph,
        rec: &mut Recorder,
        scene: CacheScene<'_>,
    ) -> Result<CacheFrame> {
        ensure!(
            Sizes::of(self.slots, scene.lights, &self.settings).cold_words <= self.sizes.cold_words,
            "more lights than the cache's light bitset was allocated for"
        );
        let config = PlanConfig {
            settings: self.settings,
            slots: self.slots,
            misses: self.sizes.misses,
            cold_words: self.sizes.cold_words,
            raw_slots: self.sizes.raw_slots,
            software: false,
            field: scene.field,
            profiling: false,
            near_bins: 0,
            eye: scene.camera.view().inverse().w_axis.truncate(),
            fov_y: scene.camera.fov_y(),
            size: scene.size,
        };
        // `ensure_gi_shade`: the setting is on and the frame's program reflects light.
        let bounce = self.settings.bounce;
        let begun = self.planner.begin(&config, true, bounce);
        // The buffers are idle between frames: every frame waits for its submission.
        for buffer in [&self.table, &self.work, &self.outgoing, &self.raw] {
            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
            if !self.zeroed {
                fill(graph, rec, buffer, 0, buffer.len)?;
            }
        }
        self.zeroed = true;
        graph.node(
            rec,
            &[(Resource::Buffer(self.table.buffer), Access::CopyDestination)],
        )?;
        rec.push(Command::UpdateBuffer {
            destination: self.table.buffer,
            offset: 0,
            bytes: begun.header.iter().flat_map(|w| w.to_le_bytes()).collect(),
        });
        // Per draw its stable id, first placement, instance count and, in a frame of
        // several programs, its cell program's selector. Ids are the draws' indices
        // plus one, as the renderer's first frame numbers its (mesh, material) pairs.
        let objects = scene.bounds.len();
        ensure!(
            scene.draws.iter().map(|d| d[0]).sum::<u32>() as usize == objects
                && scene
                    .draws
                    .iter()
                    .all(|d| (d[1] as usize) < self.selectors.len()),
            "the cache's draws do not account for its placements and programs"
        );
        let table = shared::draw_table(
            scene.draws.len(),
            scene
                .draws
                .iter()
                .enumerate()
                .map(|(b, &[instances, slot])| {
                    [b as u32 + 1, instances, self.selectors[slot as usize]]
                }),
        );
        let draws = StorageBuffer::new(gpu, bytemuck::cast_slice(&table))?;
        let models = StorageBuffer::new(gpu, scene.models)?;
        let finished = self.planner.finish(
            &config,
            &begun,
            bounce,
            FrameInputs {
                bounds: scene.bounds,
                poses: vec![None; objects],
                environment: scene.environment,
                light_key: bounce.then_some(scene.light_key),
                region: scene.region,
                still_matrix: crate::mesh::matrix_bytes(
                    scene
                        .camera
                        .unjittered()
                        .matrix(scene.size[0], scene.size[1]),
                ),
                still_extent: scene.size.map(u32::to_le_bytes).concat(),
                instances: &[scene.models, scene.normals].concat(),
                skin_revisions: vec![],
            },
        );
        let record = StorageBuffer::new(gpu, &finished.frame)?;
        for buffer in [&draws, &models, &record] {
            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
        }
        if let Some(plan) = &finished.plan {
            self.planner.advance(&begun, plan.environment_only);
        }
        Ok(CacheFrame {
            record,
            draws,
            models,
            plan: finished.plan,
        })
    }
    /// The cache's own buffer for a kernel input, or None where the frame supplies it.
    pub(crate) fn input<'a>(
        &'a self,
        frame: &'a CacheFrame,
        key: InputKey,
    ) -> Option<&'a StorageBuffer> {
        Some(match (key.group, key.binding) {
            (0, 30) => &self.table,
            (0, 31) => &self.work,
            (0, 32) => &frame.record,
            (0, 33) => &self.outgoing,
            (0, 36) => &frame.draws,
            (0, 37) => &self.raw,
            (1, 0) => &frame.models,
            _ => return None,
        })
    }
    /// Cell shading's group 0 (`Layouts::shade`): the work words, the outgoing radiance
    /// and the draw table. Its other groups are the traced shading interface.
    pub(crate) fn shade_input<'a>(
        &'a self,
        frame: &'a CacheFrame,
        key: InputKey,
    ) -> Option<&'a StorageBuffer> {
        Some(match (key.group, key.binding) {
            (0, 31) => &self.work,
            (0, 33) => &self.outgoing,
            (0, 36) => &frame.draws,
            _ => return None,
        })
    }
    /// The kernel module's programs in encode order, to build one packet each.
    pub(crate) fn programs(&self) -> impl Iterator<Item = &Program> {
        self.stages.iter().map(|(_, program, _)| program)
    }
    /// Cell shading's programs, with bounce, in the order `record` takes their packets.
    pub(crate) fn shade_programs(&self) -> impl Iterator<Item = &Program> {
        self.shade.iter().map(|(program, _)| program)
    }
    /// `GiState::encode` without profiling: the clears, then the passes this frame's
    /// plan runs. `packets` are in `programs` order; `shade` is cell shading's.
    pub(crate) fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        frame: &CacheFrame,
        packets: &[HeapBindings<'_>],
        shade: &[HeapBindings<'_>],
    ) -> Result<()> {
        let Some(plan) = &frame.plan else {
            return Ok(());
        };
        ensure!(
            packets.len() == self.stages.len()
                && (!plan.bounce || (shade.len() == self.shade.len() && !shade.is_empty())),
            "cache packets do not match the plan's passes"
        );
        let layout = WorkLayout::of_counts(self.slots, self.sizes.misses);
        let header = shared::HEADER_WORDS * 4;
        if plan.clear {
            fill(graph, rec, &self.table, header, self.table.len - header)?;
            fill(graph, rec, &self.outgoing, 0, self.outgoing.len)?;
            for at in layout.queues {
                fill(graph, rec, &self.work, u64::from(at) * 4, 4)?;
            }
        }
        fill(graph, rec, &self.work, 0, shared::COUNTER_WORDS * 4)?;
        // Next frame's gather queue starts empty; this frame's holds last frame's appends.
        if plan.bounce {
            fill(graph, rec, &self.work, plan.next_queue_at, 4)?;
        }
        // The staged ray bits, after the ray items.
        fill(
            graph,
            rec,
            &self.work,
            u64::from(layout.items + shared::RAY_ITEMS) * 4,
            u64::from(shared::RAY_ITEMS / 32) * 4,
        )?;
        let mut pass = |entry: Entry, groups: [u32; 2]| -> Result<()> {
            let at = Entry::ALL
                .iter()
                .position(|e| *e == entry)
                .context("unknown cache entry")?;
            let (_, program, kernel) = &self.stages[at];
            let threads = program.threads()?;
            kernel.record(
                graph,
                rec,
                &packets[at],
                [groups[0] * threads[0], groups[1] * threads[1], 1],
            )
        };
        let traced = !plan.environment_only;
        if traced {
            pass(Entry::Probe, plan.groups)?;
        }
        // Gathers read last frame's outgoing radiance and may request hit cells for
        // the claim rounds that follow.
        if plan.bounce {
            pass(Entry::Gather, [shared::GATHER_GROUPS, 1])?;
        }
        if traced {
            for round in 0..shared::CLAIM_ROUNDS {
                pass(Entry::Claim, [shared::CLAIM_GROUPS, 1])?;
                let last = round + 1 == shared::CLAIM_ROUNDS;
                pass(
                    if last {
                        Entry::ResolveLast
                    } else {
                        Entry::Resolve
                    },
                    [shared::CLAIM_GROUPS, 1],
                )?;
            }
        }
        if plan.scan {
            pass(Entry::Scan, [plan.scan_groups, 1])?;
        }
        if traced {
            pass(Entry::Setup, [shared::SETUP_GROUPS, 1])?;
            pass(Entry::Rays, [shared::RAY_GROUPS, 1])?;
            pass(Entry::Finish, [shared::CELL_GROUPS, 1])?;
        }
        if plan.integrate {
            pass(Entry::Integrate, [shared::CELL_GROUPS, 1])?;
        }
        // The filter once every value of the frame is written; then light visibility at
        // the cells to shade, and the program's cell shading.
        if plan.filter {
            pass(Entry::Filter, [shared::FILTER_GROUPS, 1])?;
        }
        if plan.bounce {
            pass(Entry::Light, [shared::LIGHT_GROUPS, 1])?;
            // MP1: each reflecting program over the queue in turn.
            for ((program, kernel), packet) in self.shade.iter().zip(shade) {
                let threads = program.threads()?;
                kernel.record(
                    graph,
                    rec,
                    packet,
                    [shared::LIGHT_GROUPS * threads[0], 1, 1],
                )?;
            }
        }
        Ok(())
    }
    /// Retire after every list that names the cache has been submitted.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for buffer in [self.table, self.work, self.outgoing, self.raw] {
            buffer.retire(gpu)?;
        }
        Ok(())
    }
}

/// `encoder.clear_buffer` for a word range.
pub(super) fn fill(
    graph: &mut Graph,
    rec: &mut Recorder,
    buffer: &StorageBuffer,
    offset: u64,
    size: u64,
) -> Result<()> {
    graph.node(
        rec,
        &[(Resource::Buffer(buffer.buffer), Access::CopyDestination)],
    )?;
    rec.push(Command::FillBuffer {
        destination: buffer.buffer,
        offset,
        size,
        value: 0,
    });
    Ok(())
}
