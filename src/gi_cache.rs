//! M7 G1: the world radiance cache (WRC), sky visibility only (docs/m7-g1.md).
//!
//! World cells keyed like the N8o surface cache (a power-of-two cube by distance plus
//! a normal bin) hold each cell's fixed query point, one visibility bit per fixed
//! direction and the fixed-weight L2 SH of the occluded sky. Shading reads them
//! through `indirect_irradiance` (shaders/gi_api.wgsl). Kernels and layout live in
//! shaders/gi_cells.wgsl and shaders/gi_cache.wgsl; the cache pattern is copied from
//! the shadow surface cache, not shared with it (M7 root decision).
//!
//! Patent design-arounds (docs/m7-p0-claims.md): values are replaced, never counted
//! and averaged (W1, C8); new and invalidated cells are always traced and the rest
//! refresh on a fixed rotation, with no ranking (W2); query points come from the key
//! alone (W2); lookups are unjittered (W3); no resampling anywhere (C1); inserts are
//! atomicMax claims that the winner writes (C4).
use super::*;
use crate::gi_bvh::{self, Blas};
use crate::primary_ray::{TRACED_DRAWS, placement_table};

const HEADER_WORDS: u64 = 64;
/// Hot (shading) and cold (kernel) words per slot; shaders/gi_cells.wgsl.
const SLOT_WORDS: u64 = 16 + 28;
/// Work counters, then the per-slot claim words (shaders/gi_cache.wgsl GI_W_CLAIMS).
const COUNTER_WORDS: u64 = 32;
const MISS_WORDS: u64 = 6;
/// WGSL GiFrame: region, counts, offsets, requests, then 64 vec4 box corners.
const FRAME_BYTES: usize = 64 + 64 * 16;
const MAX_BOXES: usize = 32;
/// Claim and resolve repeat in rounds, so collisions retry within the frame.
const CLAIM_ROUNDS: u32 = 3;
/// Ray items per frame (shaders/gi_cache.wgsl GI_RAY_ITEMS): one visibility ray each,
/// traced in parallel. A frame wanting more traces the rest in setup, eight lanes a cell.
const RAY_ITEMS: u32 = 1 << 21;
/// Probe, three claim and resolve rounds, scan, setup, rays, finish and integrate.
const PROFILE_STAMPS: u32 = 2 * (6 + 2 * CLAIM_ROUNDS);
/// Pixels per probe tile side (shaders/gi_cache.wgsl GI_PROBE_TILE).
const PROBE_TILE: u32 = 8;
/// The probe tile side; test builds that screen shader variants (RENDERLAB_GI_SHADERS) may
/// override it with RENDERLAB_GI_PROBE_TILE to match a variant's workgroup.
fn probe_tile() -> u32 {
    #[cfg(test)]
    if let Some(tile) = std::env::var("RENDERLAB_GI_PROBE_TILE")
        .ok()
        .and_then(|t| t.parse().ok())
    {
        return tile;
    }
    PROBE_TILE
}
/// Fixed grids, in 64-thread workgroups, of the passes whose counts only the GPU knows:
/// each kernel reads its count and loops over it. Direct dispatches need no argument
/// passes, and skip the per-dispatch validation pass wgpu adds to indirect dispatches
/// (`InstanceFlags::VALIDATION_INDIRECT_CALL`, on by default); an idle workgroup exits
/// at once. Claims and resolves, setup (eight cells per workgroup), rays (one item per
/// thread), then finish and integrate (sixteen cells per workgroup).
const CLAIM_GROUPS: u32 = 512;
const SETUP_GROUPS: u32 = 4096;
const RAY_GROUPS: u32 = 4096;
const CELL_GROUPS: u32 = 2048;
/// Frames the profile ring holds, and its per-frame stride (resolve offsets are
/// 256-byte aligned).
const PROFILE_RING: u64 = 64;
const PROFILE_STRIDE: u64 = 256;
/// Consecutive identical frames after which a frame runs no cache pass: the orphan-parent
/// chain completes on the third frame (gi_probe's rounds), a split cell's finer levels
/// appear one a frame after it (up to GI_REFINE), and claims lost on the last retry on
/// the next two.
const IDLE_AFTER: u32 = 8;
/// Slot groups a changing environment map spreads its re-integration over: on frames where
/// the map changed again (a rotating sky), each frame re-integrates the slots with
/// (slot + phase) % ENVIRONMENT_PERIOD == 0, a fixed rotation (W2), and it keeps going for
/// ENVIRONMENT_PERIOD frames after the last change, so every cell is exact again within the
/// 8-frame settle limit. A single change after a still frame re-integrates every cell at
/// once (the next frame equals a fresh cache).
const ENVIRONMENT_PERIOD: u32 = 8;
/// Entries of the frame's request table: twice the miss capacity (a power of two).
fn requests(misses: u32) -> u32 {
    misses * 2
}
/// Blocking readbacks for measurement helpers.
const WAIT: wgpu::PollType = wgpu::PollType::Wait {
    submission_index: None,
    timeout: None,
};

/// How a pixel reads cells (W3): the containing cell, or fixed trilinear weights over
/// the eight cells around it. Neither jitters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndirectLookup {
    Nearest,
    #[default]
    Trilinear,
}

/// World radiance cache settings. Off by default at this checkpoint: the standard
/// program then reads the unoccluded environment, as before M7.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndirectSettings {
    pub enabled: bool,
    pub lookup: IndirectLookup,
    /// Target cell edge, in pixels of footprint at the receiver's distance.
    pub cell_pixels: f32,
    /// Edge of level 0 in metres; each level doubles it.
    pub min_edge: f32,
    pub levels: u32,
    /// log2 of the slot count (each slot is 160 bytes).
    pub slots_log2: u32,
    /// Fixed-rotation refresh (W2): a cell re-traces when (slot + frame) % period == 0.
    /// 0 disables it; events alone keep sky visibility current.
    pub rotation_period: u32,
    /// Frames a cell may go unseen before its slot can be reused.
    pub evict_frames: u32,
    /// Trace software rays even when hardware ray queries exist (A/B measurement).
    pub force_software: bool,
    /// W1 value form. 1 replaces all 64 directions on every update (default); S > 1
    /// keeps one slot per direction and re-traces the directions k % S == frame % S,
    /// so a new or invalidated cell settles over S frames (untraced counts as open).
    pub direction_phases: u32,
    /// Exact moved-caster invalidation. Off only to measure the fixed rotation's
    /// staleness alone.
    pub invalidate_casters: bool,
    /// Fixed directions per cell: 64 or 128.
    pub directions: u32,
    /// Half-width, in levels, of the band around each level switch where two levels
    /// mix with a continuous fixed weight (no jitter, W3); 0 switches abruptly.
    pub level_blend: f32,
    /// Rotate each cell's direction set about its normal by an angle its key fixes,
    /// which decorrelates quadrature error between cells (as noise); off, cells with
    /// the same normal share directions, and the error is smooth.
    pub rotate_directions: bool,
    /// Trace visibility rays one per thread (default). Off, each queued cell traces its
    /// rays in one thread (A/B measurement); the bits and values are the same.
    pub parallel_rays: bool,
    /// Cut refill groups S: on the S frames after a reset (`reset_indirect`), a cell
    /// traces one group of its fixed directions (k % S == g, the first untraced group) a
    /// frame, so a cut costs about 1/S of its rays per frame and settles by frame S-1.
    /// Untraced directions take the nearest traced direction's visibility meanwhile. 1
    /// traces every direction of a new cell at once; 0 (the default) picks 4 for software
    /// rays, where a cut costs the most, and 1 for hardware rays (docs/m7-g1b.md).
    pub cut_phases: u32,
}
impl Default for IndirectSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            lookup: IndirectLookup::Trilinear,
            cell_pixels: 4.0,
            min_edge: 1.0 / 512.0,
            levels: 14,
            slots_log2: 19,
            rotation_period: 0,
            evict_frames: 120,
            force_software: false,
            direction_phases: 1,
            invalidate_casters: true,
            directions: 64,
            // Unrotated, cells with the same normal share their quadrature error, which
            // shows as faint cell-sized rectangles on flat faces; rotated, it becomes a
            // fine fixed mottle (SV frame error 1.93% -> 1.64%), with no extra rays.
            rotate_directions: true,
            level_blend: 0.0,
            parallel_rays: true,
            cut_phases: 0,
        }
    }
}
impl IndirectSettings {
    fn validate(&self) -> Result<(), Error> {
        let valid = self.cell_pixels.is_finite()
            && self.cell_pixels > 0.0
            && self.min_edge.is_finite()
            && self.min_edge > 0.0
            && (1..=24).contains(&self.levels)
            && (10..=21).contains(&self.slots_log2)
            && self.evict_frames != 0
            && (1..=64).contains(&self.direction_phases)
            && [64, 128].contains(&self.directions)
            && (0.0..=0.5).contains(&self.level_blend)
            && self.cut_phases <= 8;
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidInput("invalid indirect settings"))
        }
    }
}

/// Cache resources and the last frame's work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndirectStats {
    pub table_bytes: u64,
    pub work_bytes: u64,
    /// Software-ray geometry (non-RT path), near-field masks included.
    pub geometry_bytes: u64,
    /// Of which cooked near-field masks.
    pub near_bytes: u64,
    /// Instances whose transform matched a cooked near field in the last software frame.
    pub near_instances: u32,
    /// Frames that ran the cache passes.
    pub frames: u64,
    /// The last such frame traced software rays.
    pub software: bool,
    /// Frames whose unchanged inputs ran no cache pass.
    pub idle_frames: u64,
}

/// GPU microseconds per cache pass of the last profiled frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IndirectProfile {
    pub probe: f64,
    pub claim: f64,
    pub resolve: f64,
    pub scan: f64,
    /// Tracing: query points and ray items, the rays, then the bits and SH per cell.
    pub setup: f64,
    pub rays: f64,
    pub finish: f64,
    pub integrate: f64,
}
impl IndirectProfile {
    pub fn trace(&self) -> f64 {
        self.setup + self.rays + self.finish
    }
    pub fn total(&self) -> f64 {
        self.probe + self.claim + self.resolve + self.scan + self.trace() + self.integrate
    }
}

/// The kernels' counters after a frame (test readback).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndirectCounters {
    pub misses: u32,
    pub traces: u32,
    pub integrates: u32,
    pub miss_overflow: u32,
    pub full: u32,
    pub orphans: u32,
    pub local_overflow: u32,
    pub created: u32,
    pub invalidated: u32,
    pub refreshed: u32,
    /// Claims that lost their slot this frame (they retry next round or frame).
    pub lost: u32,
    /// Visibility rays traced this frame (probe rays excluded).
    pub rays: u32,
    /// Rays requested as parallel items; past RAY_ITEMS the rest trace in setup.
    pub ray_items: u32,
    /// Orphans whose candidates found no qualifying surface, and orphans whose every
    /// qualifying candidate lay inside a solid.
    pub orphans_no_surface: u32,
    pub orphans_inside: u32,
    /// Cells probed this frame whose fixed probe lines each met two surface layers
    /// (shaders/gi_cache.wgsl gi_probe_line); their pixels read finer levels.
    pub splits: u32,
    /// The probe's work, counted on profiled frames only (zero otherwise).
    pub probe: ProbeCounters,
}

/// What the probe did in one profiled frame (shaders/gi_cache.wgsl GI_W_P_*): tiles with a
/// covered pixel, covered pixels, tile-table insert calls and their probe steps, unique cells
/// from pixels, added for orphans' parents and by refinement, table finds and how many found
/// a cell, refinement walks (pixel, bin and level), tiles that refined, the most unique cells
/// in one tile, and tiles by unique cells (< 32, < 64, < 256, full).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeCounters {
    pub tiles: u32,
    pub pixels: u32,
    pub inserts: u32,
    pub steps: u32,
    pub cells: u32,
    pub parents: u32,
    pub refined: u32,
    pub finds: u32,
    pub found: u32,
    pub walks: u32,
    pub refine_tiles: u32,
    pub max_cells: u32,
    pub tiles_by_cells: [u32; 4],
}

struct Resources {
    table: Owned<wgpu::Buffer>,
    work: Owned<wgpu::Buffer>,
    frame: Owned<wgpu::Buffer>,
    slots: u32,
    misses: u32,
    _allocations: [Allocation; 2],
}
struct Layouts {
    /// Hardware (TLAS at 34) and software (geometry at 35).
    main: [Owned<wgpu::BindGroupLayout>; 2],
    arrays: Owned<wgpu::BindGroupLayout>,
    pipelines: [Owned<wgpu::PipelineLayout>; 2],
}
struct Pipelines {
    software: bool,
    directions: u32,
    probe: Owned<wgpu::ComputePipeline>,
    claim: Owned<wgpu::ComputePipeline>,
    resolve: Owned<wgpu::ComputePipeline>,
    resolve_last: Owned<wgpu::ComputePipeline>,
    scan: Owned<wgpu::ComputePipeline>,
    setup: Owned<wgpu::ComputePipeline>,
    rays: Owned<wgpu::ComputePipeline>,
    finish: Owned<wgpu::ComputePipeline>,
    integrate: Owned<wgpu::ComputePipeline>,
}
/// Group 1: the traced shading ABI's geometry tables, built from the same draws in
/// TLAS placement order, so hit records decode to the receivers shading sees.
struct Arrays {
    keys: Vec<(wgpu::Buffer, wgpu::Buffer, usize)>,
    models: Owned<wgpu::Buffer>,
    normals: Owned<wgpu::Buffer>,
    _placements: Owned<wgpu::Buffer>,
    _absent: Owned<wgpu::Buffer>,
    instances: Vec<u8>,
    bind: Owned<wgpu::BindGroup>,
}
/// Software-ray geometry: every draw's mesh records and near masks, and the flat instance
/// list.
struct Packed {
    meshes: Vec<wgpu::Buffer>,
    instances: usize,
    offsets: Vec<[u32; 4]>,
    /// Per draw, the base word of each of its mesh's cooked near placements' masks.
    near: Vec<Vec<u32>>,
    near_bytes: u64,
    head: Vec<u32>,
    buffer: Owned<wgpu::Buffer>,
    bytes: u64,
    _allocation: Allocation,
}
struct Groups {
    key: (wgpu::Texture, wgpu::Texture, u64, bool),
    main: Owned<wgpu::BindGroup>,
}
/// Pass timestamps of the last PROFILE_RING cache frames, resolved per frame into a
/// ring and read back only on request, so measured frames never wait for the CPU.
struct Profile {
    queries: Owned<wgpu::QuerySet>,
    resolve: Owned<wgpu::Buffer>,
    readback: Owned<wgpu::Buffer>,
    period: f32,
    /// Cache frames profiled so far.
    written: u64,
}
/// What `prepare_gi` decided for this frame.
struct Plan {
    groups: [u32; 2],
    /// Scan workgroups: every slot, or one environment slot group.
    scan_groups: u32,
    scan: bool,
    /// The environment changed: cells re-integrate their stored bits.
    integrate: bool,
    clear: bool,
    /// Only the environment changed, with the view and scene otherwise idle: only the scan
    /// and integrate passes run.
    environment_only: bool,
}
enum Retired {
    Resources(#[allow(dead_code)] Resources),
    Arrays(#[allow(dead_code)] Arrays),
    Packed(#[allow(dead_code)] Packed),
    Groups(#[allow(dead_code)] Groups),
}

pub(super) struct GiState {
    pub settings: IndirectSettings,
    /// A header-only table with `enabled` 0: lighting's binding 20 while the cache is
    /// off, and always in the kernels' own lighting group (they write the real table).
    pub dummy: Owned<wgpu::Buffer>,
    _dummy_allocation: Allocation,
    resources: Option<Resources>,
    /// Bumped when the real table is replaced; lighting rebinds then.
    pub generation: u64,
    layouts: Option<Layouts>,
    pipelines: Option<Pipelines>,
    arrays: Option<Arrays>,
    meshes: Vec<(wgpu::Buffer, Arc<Blas>)>,
    /// The octahedral bins per side every cooked mesh's near-field masks share (0: none).
    near_bins: u32,
    packed: Option<Packed>,
    /// Last frame's world bounds per placement, for moved-caster boxes.
    bounds: Vec<[f32; 6]>,
    environment: Option<[u8; 32]>,
    frame: u32,
    pending_clear: bool,
    /// Cache frames since the table was last emptied (cut refill groups).
    since_clear: u32,
    /// The last frame's inputs (camera, transforms, environment, settings) and how many
    /// frames in a row they have been unchanged; idle frames run no cache pass.
    idle_key: Option<[u8; 32]>,
    idle_frames: u32,
    /// The idle key without the environment, and its count of identical frames.
    view_key: Option<[u8; 32]>,
    view_frames: u32,
    /// The environment changed on the last cache frame; spread re-integrations left after
    /// the last change; and the slot group the next spread frame re-integrates.
    environment_streak: bool,
    environment_catchup: u32,
    environment_phase: u32,
    plan: Option<Plan>,
    profile: Option<Profile>,
    groups: Option<Groups>,
    retired: Vec<(u64, Retired)>,
    stats: IndirectStats,
}

impl GiState {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let dummy = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("indirect cache header (off)"),
                size: HEADER_WORDS * 4 + SLOT_WORDS * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        Self {
            settings: IndirectSettings::default(),
            _dummy_allocation: ledger.allocate(AllocationKind::Light, dummy.size()),
            dummy,
            resources: None,
            generation: 0,
            layouts: None,
            pipelines: None,
            arrays: None,
            meshes: vec![],
            near_bins: 0,
            packed: None,
            bounds: vec![],
            environment: None,
            frame: 0,
            pending_clear: false,
            since_clear: u32::MAX,
            idle_key: None,
            idle_frames: 0,
            view_key: None,
            view_frames: 0,
            environment_streak: false,
            environment_catchup: 0,
            environment_phase: 0,
            plan: None,
            profile: None,
            groups: None,
            retired: vec![],
            stats: IndirectStats::default(),
        }
    }
    /// The table shading binds (lighting group 3, binding 20).
    pub fn shading_table(&self) -> &wgpu::Buffer {
        self.resources.as_ref().map_or(&*self.dummy, |r| &*r.table)
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
}

/// The kernel module: built-in prelude, primary ray helpers, cell definitions, one ray
/// source and the kernels.
pub(super) fn kernel_source(software: bool, directions: u32) -> String {
    let enables = if software {
        "enable wgpu_binding_array;\n"
    } else {
        "enable wgpu_binding_array;\nenable wgpu_ray_query;\nenable wgpu_ray_query_vertex_return;\n"
    };
    use crate::program::gi_shader;
    let rays = if software {
        gi_shader(
            "gi_rays_software.wgsl",
            include_str!("../shaders/gi_rays_software.wgsl"),
        )
    } else {
        gi_shader(
            "gi_rays_hardware.wgsl",
            include_str!("../shaders/gi_rays_hardware.wgsl"),
        )
    };
    let cells = gi_shader("gi_cells.wgsl", include_str!("../shaders/gi_cells.wgsl"));
    let line = "const GI_DIRECTIONS:u32=64u;";
    assert_eq!(cells.matches(line).count(), 1);
    let cells = cells.replace(line, &format!("const GI_DIRECTIONS:u32={directions}u;"));
    let kernels = gi_shader("gi_cache.wgsl", include_str!("../shaders/gi_cache.wgsl"));
    assert_eq!(
        kernels
            .matches(&format!("const GI_RAY_ITEMS:u32={RAY_ITEMS}u;"))
            .count(),
        1
    );
    let line = "const GI_NEIGHBOURS:array<u32,256>=array<u32,256>();";
    assert_eq!(kernels.matches(line).count(), 1);
    let words: Vec<String> = neighbour_table(directions)
        .iter()
        .map(|w| format!("{w}u"))
        .collect();
    let kernels = kernels.replace(
        line,
        &format!(
            "const GI_NEIGHBOURS:array<u32,256>=array<u32,256>({});",
            words.join(",")
        ),
    );
    format!(
        "{enables}{}\nconst PRIMARY_BATCHES:u32={TRACED_DRAWS}u;\n{}\n{cells}\n{rays}\n{kernels}",
        include_str!("../shaders/scene.wgsl"),
        include_str!("../shaders/primary_rays.wgsl"),
    )
}

/// Each fixed direction's eight nearest directions of the unrotated set (the kernels'
/// gi_local_direction, in f64), nearest first with ties to the lower index, as 8-bit
/// indices four to a word, padded to 256 words (128 directions).
fn neighbour_table(directions: u32) -> Vec<u32> {
    let direction = |k: u32| {
        let z = 1.0 - (f64::from(k) + 0.5) / f64::from(directions);
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = f64::from(k) * 2.399963229728653;
        [r * phi.cos(), r * phi.sin(), z]
    };
    let mut words = vec![0u32; 256];
    for k in 0..directions {
        let d = direction(k);
        let mut near: Vec<(f64, u32)> = (0..directions)
            .filter(|&j| j != k)
            .map(|j| {
                let e = direction(j);
                (d[0] * e[0] + d[1] * e[1] + d[2] * e[2], j)
            })
            .collect();
        near.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (i, &(_, j)) in near.iter().take(8).enumerate() {
            let at = (k * 8) as usize + i;
            words[at / 4] |= j << (8 * (at % 4));
        }
    }
    words
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}
fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}
fn uint_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

impl Renderer {
    /// Selects world radiance cache settings. Enabling allocates the table (all
    /// slots empty); a new slot count reallocates it. Takes effect on the next lit
    /// traced or visibility-buffer frame whose programs call `indirect_irradiance`.
    pub fn set_indirect(&mut self, settings: IndirectSettings) -> Result<(), Error> {
        self.check_fault()?;
        settings.validate()?;
        let slots = 1u32 << settings.slots_log2;
        let reallocate =
            settings.enabled && self.gi.resources.as_ref().is_none_or(|r| r.slots != slots);
        if reallocate {
            let misses = slots;
            let table_bytes = (HEADER_WORDS + u64::from(slots) * SLOT_WORDS) * 4;
            // Counters, claims, misses, trace and integrate queues, ray items and their
            // staged bits, then the request table (the frame offsets follow this order).
            let work_bytes = (COUNTER_WORDS
                + u64::from(slots)
                + u64::from(misses) * MISS_WORDS
                + 2 * u64::from(slots)
                + u64::from(RAY_ITEMS + RAY_ITEMS / 32)
                + 2 * u64::from(requests(misses)))
                * 4;
            let buffer = |label, size, usage| {
                self.ledger.own(
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: usage | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ResourceKind::Buffer,
                )
            };
            let storage = wgpu::BufferUsages::STORAGE;
            let resources = Resources {
                table: buffer(
                    "indirect cache table",
                    table_bytes,
                    storage | wgpu::BufferUsages::COPY_SRC,
                ),
                work: buffer(
                    "indirect cache work",
                    work_bytes,
                    storage | wgpu::BufferUsages::COPY_SRC,
                ),
                frame: buffer(
                    "indirect cache frame",
                    FRAME_BYTES as u64,
                    wgpu::BufferUsages::UNIFORM,
                ),
                slots,
                misses,
                _allocations: [
                    self.ledger.allocate(AllocationKind::Light, table_bytes),
                    self.ledger.allocate(AllocationKind::Light, work_bytes),
                ],
            };
            self.gi.stats.table_bytes = table_bytes;
            self.gi.stats.work_bytes = work_bytes;
            if let Some(old) = self.gi.resources.replace(resources) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Resources(old)));
            }
            if let Some(old) = self.gi.groups.take() {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Groups(old)));
            }
            self.gi.generation += 1;
            self.gi.environment = None;
            self.gi.bounds.clear();
        } else if self.gi.settings.lookup != settings.lookup
            || self.gi.settings.cell_pixels != settings.cell_pixels
            || self.gi.settings.min_edge != settings.min_edge
            || self.gi.settings.levels != settings.levels
            || self.gi.settings.direction_phases != settings.direction_phases
            || self.gi.settings.directions != settings.directions
            || self.gi.settings.rotate_directions != settings.rotate_directions
        {
            // Keys change meaning: start from an empty table.
            self.gi.pending_clear = true;
        }
        if self.gi.settings.force_software != settings.force_software {
            self.gi.pending_clear = true;
        }
        self.gi.settings = settings;
        Ok(())
    }
    pub fn indirect(&self) -> IndirectSettings {
        self.gi.settings
    }
    pub fn indirect_stats(&self) -> IndirectStats {
        let mut stats = self.gi.stats;
        stats.geometry_bytes = self.gi.packed.as_ref().map_or(0, |p| p.bytes);
        stats.near_bytes = self.gi.packed.as_ref().map_or(0, |p| p.near_bytes);
        stats
    }
    /// Empties the cache on the next frame: a camera cut or scene replacement that
    /// should not reuse earlier cells.
    pub fn reset_indirect(&mut self) {
        self.gi.pending_clear = true;
        // Seen stamps restart too, so a reset sequence repeats bit for bit.
        self.gi.frame = 0;
    }
    /// Software-ray geometry for `mesh` on the non-RT path: a full-detail BVH built now
    /// from the mesh's object-space triangles. Hardware renderers ignore it unless
    /// `force_software` is set.
    pub fn set_indirect_geometry(
        &mut self,
        mesh: &Mesh,
        positions: &[[f32; 3]],
        indices: &[u32],
    ) -> Result<(), Error> {
        let bvh = gi_bvh::build(positions, indices)?;
        self.set_indirect_bvh(mesh, bvh, 0)
    }
    /// Software-ray geometry from a cooked cluster BVH (an R0c sidecar), traced at the
    /// given level: one complete cut.
    pub fn set_indirect_bvh(
        &mut self,
        mesh: &Mesh,
        bvh: crate::asset::cluster_bvh::Bvh,
        level: usize,
    ) -> Result<(), Error> {
        self.set_indirect_blas(mesh, Blas::new(bvh, level)?)
    }
    /// Software-ray geometry for `mesh` from mesh `index` of a cooked near field
    /// (renderlab_asset::near_field, docs/m7-g1c.md): its far-field cut, traced at runtime in
    /// place of full detail. When the field has masks, each instance whose transform has the
    /// exact bits of one of the field's placements of this mesh reads that placement's masks:
    /// a cell on it also blocks the directions all three vertex masks block. Every cooked mesh
    /// with masks must share the field's `bins`.
    pub fn set_indirect_near_field(
        &mut self,
        mesh: &Mesh,
        field: &crate::asset::near_field::NearField,
        index: usize,
    ) -> Result<(), Error> {
        let bins = field.settings.bins;
        if bins != 0 && self.gi.near_bins != 0 && self.gi.near_bins != bins {
            return Err(Error::InvalidInput(
                "near-field masks must share their bins",
            ));
        }
        let bvh = field
            .meshes
            .get(index)
            .ok_or(Error::InvalidInput("near-field mesh index"))?;
        let mut blas = Blas::new(bvh.clone(), 0)?;
        blas.near = field
            .placements
            .iter()
            .filter(|p| p.mesh as usize == index && !p.masks.is_empty())
            .map(|p| gi_bvh::NearMasks {
                model: Mat4::from_cols_array_2d(&p.transform),
                masks: p.masks.clone(),
            })
            .collect();
        if bins != 0 {
            self.gi.near_bins = bins;
        }
        self.set_indirect_blas(mesh, blas)
    }
    fn set_indirect_blas(&mut self, mesh: &Mesh, blas: Blas) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.identity, &mesh.renderer) {
            return Err(Error::InvalidInput("mesh belongs to another renderer"));
        }
        let key: wgpu::Buffer = (**mesh.vertex).clone();
        self.gi.meshes.retain(|(k, _)| *k != key);
        self.gi.meshes.push((key, Arc::new(blas)));
        if self.gi.meshes.iter().all(|(_, b)| b.near.is_empty()) {
            self.gi.near_bins = 0;
        }
        if let Some(old) = self.gi.packed.take() {
            self.gi
                .retired
                .push((self.submissions, Retired::Packed(old)));
        }
        self.gi.pending_clear = true;
        Ok(())
    }
    /// Per-pass GPU timing of later frames into a readback (`indirect_profile`).
    pub fn set_indirect_profiling(&mut self, enabled: bool) {
        if !enabled
            || !self
                .device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            self.gi.profile = None;
            return;
        }
        if self.gi.profile.is_some() {
            return;
        }
        let queries = self.ledger.own(
            self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("indirect cache passes"),
                ty: wgpu::QueryType::Timestamp,
                count: PROFILE_STAMPS * PROFILE_RING as u32,
            }),
            ResourceKind::QuerySet,
        );
        let bytes = PROFILE_STRIDE * PROFILE_RING;
        let buffer = |label, usage| {
            self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        self.gi.profile = Some(Profile {
            queries,
            resolve: buffer(
                "indirect cache timestamps",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "indirect cache timestamp readback",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            period: self.queue.get_timestamp_period(),
            written: 0,
        });
    }
    /// Blocks until the GPU is idle and reads the last profiled frame's pass times.
    pub fn indirect_profile(&mut self) -> Result<Option<IndirectProfile>, Error> {
        Ok(self.indirect_profiles()?.pop())
    }
    /// Blocks until the GPU is idle and reads the pass times of up to the last 64
    /// profiled cache frames, oldest first. Idle frames run no pass and add none.
    pub fn indirect_profiles(&mut self) -> Result<Vec<IndirectProfile>, Error> {
        let Some(profile) = self.gi.profile.as_ref().filter(|p| p.written > 0) else {
            return Ok(vec![]);
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            &profile.resolve,
            0,
            &profile.readback,
            0,
            PROFILE_STRIDE * PROFILE_RING,
        );
        self.queue.submit([encoder.finish()]);
        let slice = profile.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let words: Vec<u64> = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        profile.readback.unmap();
        let period = f64::from(profile.period) / 1000.0;
        let rounds = CLAIM_ROUNDS as usize;
        let first = profile.written.saturating_sub(PROFILE_RING);
        Ok((first..profile.written)
            .map(|frame| {
                let at = ((frame % PROFILE_RING) * PROFILE_STRIDE / 8) as usize;
                let pass = |i: usize| {
                    words[at + 2 * i + 1].saturating_sub(words[at + 2 * i]) as f64 * period
                };
                IndirectProfile {
                    probe: pass(0),
                    claim: (0..rounds).map(|k| pass(1 + 2 * k)).sum(),
                    resolve: (0..rounds).map(|k| pass(2 + 2 * k)).sum(),
                    scan: pass(1 + 2 * rounds),
                    setup: pass(2 + 2 * rounds),
                    rays: pass(3 + 2 * rounds),
                    finish: pass(4 + 2 * rounds),
                    integrate: pass(5 + 2 * rounds),
                }
            })
            .collect())
    }
    /// Blocks until the GPU is idle and reads the last cache frame's counters.
    pub fn indirect_counters(&mut self) -> Result<Option<IndirectCounters>, Error> {
        let Some(r) = self.gi.resources.as_ref() else {
            return Ok(None);
        };
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect cache counters"),
            size: COUNTER_WORDS * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.work, 0, &readback, 0, COUNTER_WORDS * 4);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let w: Vec<u32> = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Some(IndirectCounters {
            misses: w[0],
            traces: w[1],
            integrates: w[2],
            miss_overflow: w[3],
            full: w[4],
            orphans: w[5],
            local_overflow: w[6],
            created: w[7],
            invalidated: w[8],
            refreshed: w[9],
            lost: w[10],
            rays: w[11],
            ray_items: w[12],
            orphans_no_surface: w[13],
            orphans_inside: w[14],
            splits: w[15],
            probe: ProbeCounters {
                tiles: w[16],
                pixels: w[17],
                inserts: w[18],
                steps: w[19],
                cells: w[20],
                parents: w[21],
                refined: w[22],
                finds: w[23],
                found: w[24],
                walks: w[25],
                refine_tiles: w[26],
                max_cells: w[27],
                tiles_by_cells: [w[28], w[29], w[30], w[31]],
            },
        }))
    }
    /// Blocks until the GPU is idle and copies the whole table (determinism gates).
    pub fn indirect_table(&mut self) -> Result<Option<Vec<u32>>, Error> {
        let Some(r) = self.gi.resources.as_ref() else {
            return Ok(None);
        };
        let size = r.table.size();
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect cache table readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&r.table, 0, &readback, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let words = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("indirect cache readback map"))?
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Some(words))
    }

    /// Whether any draw's program reads the cache (naga-free token check of its source).
    fn gi_callers(&self, draws: &[PreparedDraw<'_>]) -> bool {
        let standard = self.standard.id();
        draws.iter().any(|d| {
            let id = d.material.map_or(standard, |m| m.program.id());
            crate::program::uses_indirect(&self.program_slot(id).source)
        })
    }

    fn ensure_gi_layouts(&mut self) {
        if self.gi.layouts.is_some() {
            return;
        }
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            self.ledger.own(
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some(label),
                        entries,
                    }),
                ResourceKind::BindGroupLayout,
            )
        };
        let common = [
            uint_texture(7),
            uint_texture(21),
            storage(30, false),
            storage(31, false),
            entry(
                32,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(FRAME_BYTES as u64),
                },
            ),
        ];
        let tlas = entry(
            34,
            wgpu::BindingType::AccelerationStructure {
                vertex_return: true,
            },
        );
        // Software rays bind their geometry at 35 on every device; the TLAS entry at 34
        // exists only where ray queries do.
        let software = [&common[..], &[storage(35, true)]].concat();
        let rt = self
            .device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let hardware = if rt {
            [&common[..], &[tlas]].concat()
        } else {
            software.clone()
        };
        let main = [
            group("indirect cache, hardware rays", &hardware),
            group("indirect cache, software rays", &software),
        ];
        let per_draw = |e: wgpu::BindGroupLayoutEntry| wgpu::BindGroupLayoutEntry {
            count: std::num::NonZeroU32::new(TRACED_DRAWS),
            ..e
        };
        let arrays = group(
            "indirect cache geometry",
            &[
                storage(0, true),
                storage(1, true),
                per_draw(storage(2, true)),
                per_draw(storage(3, true)),
                storage(5, true),
            ],
        );
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            self.ledger.own(
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: groups,
                        immediate_size: 0,
                    }),
                ResourceKind::PipelineLayout,
            )
        };
        let lighting = &self.lighting.bindings;
        let pipelines = [
            layout(
                "indirect cache ABI, hardware",
                &[Some(&main[0]), Some(&arrays), None, Some(lighting)],
            ),
            layout(
                "indirect cache ABI, software",
                &[Some(&main[1]), Some(&arrays), None, Some(lighting)],
            ),
        ];
        self.gi.layouts = Some(Layouts {
            main,
            arrays,
            pipelines,
        });
    }

    fn ensure_gi_pipelines(&mut self, software: bool) -> Result<(), Error> {
        let directions = self.gi.settings.directions;
        if self
            .gi
            .pipelines
            .as_ref()
            .is_some_and(|p| p.software == software && p.directions == directions)
        {
            return Ok(());
        }
        self.ensure_gi_layouts();
        let scopes = OperationScopes::new(&self.device);
        let module = self.ledger.own(
            self.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("indirect cache kernels"),
                    source: wgpu::ShaderSource::Wgsl(kernel_source(software, directions).into()),
                }),
            ResourceKind::ShaderModule,
        );
        let layouts = self.gi.layouts.as_ref().unwrap();
        let compute = |entry: &str, layout: &wgpu::PipelineLayout| {
            self.ledger.own(
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                ResourceKind::ComputePipeline,
            )
        };
        let main = &layouts.pipelines[usize::from(software)];
        let pipelines = Pipelines {
            software,
            directions,
            probe: compute("gi_probe", main),
            claim: compute("gi_claim", main),
            resolve: compute("gi_resolve", main),
            resolve_last: compute("gi_resolve_last", main),
            scan: compute("gi_scan", main),
            setup: compute("gi_setup", main),
            rays: compute("gi_rays", main),
            finish: compute("gi_finish", main),
            integrate: compute("gi_integrate", main),
        };
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(error.to_string()));
        }
        self.gi.pipelines = Some(pipelines);
        Ok(())
    }

    fn prepare_gi_arrays(&mut self, draws: &[PreparedDraw<'_>]) -> Result<(), Error> {
        let keys: Vec<_> = draws
            .iter()
            .map(|d| {
                (
                    (**d.mesh.vertex).clone(),
                    (**d.mesh.index).clone(),
                    d.instances.len(),
                )
            })
            .collect();
        if self.gi.arrays.as_ref().is_none_or(|a| a.keys != keys) {
            let default = self.default_material.as_ref().unwrap();
            let placements = placement_table(draws.iter().map(|d| {
                let m = d.material.unwrap_or(default);
                (
                    d.instances.len(),
                    d.mesh.tangent.is_some(),
                    !m.double_sided,
                    m.thin,
                )
            }))?;
            let count = placements.len().max(1) as u64;
            let buffer = |label, size| {
                self.ledger.own(
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ResourceKind::Buffer,
                )
            };
            let models = buffer("indirect cache placement transforms", count * 64);
            let normals = buffer("indirect cache placement normals", count * 64);
            let table = buffer("indirect cache placement draws", count * 4);
            let absent = buffer("indirect cache absent storage", 48);
            let words: Vec<u8> = placements.iter().flat_map(|v| v.to_ne_bytes()).collect();
            if !words.is_empty() {
                self.queue.write_buffer(&table, 0, &words);
            }
            self.ensure_gi_layouts();
            fn pad<'a>(
                buffers: impl Iterator<Item = &'a wgpu::Buffer>,
                absent: &'a wgpu::Buffer,
            ) -> Vec<wgpu::BufferBinding<'a>> {
                buffers
                    .chain(std::iter::repeat(absent))
                    .take(TRACED_DRAWS as usize)
                    .map(|b| b.as_entire_buffer_binding())
                    .collect()
            }
            let vertices = pad(draws.iter().map(|d| &**d.mesh.vertex), &absent);
            let indices = pad(draws.iter().map(|d| &**d.mesh.index), &absent);
            let bind = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("indirect cache geometry"),
                    layout: &self.gi.layouts.as_ref().unwrap().arrays,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: models.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: normals.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::BufferArray(&vertices),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::BufferArray(&indices),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: table.as_entire_binding(),
                        },
                    ],
                }),
                ResourceKind::BindGroup,
            );
            drop(vertices);
            drop(indices);
            let next = Arrays {
                keys,
                models,
                normals,
                _placements: table,
                _absent: absent,
                instances: vec![],
                bind,
            };
            if let Some(old) = self.gi.arrays.replace(next) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Arrays(old)));
            }
        }
        // The same bytes the traced shading tables publish, in placement order.
        let mut instances = vec![];
        for d in draws {
            for model in &d.instances.state.models {
                instances.extend_from_slice(&matrix_bytes(*model));
            }
        }
        for d in draws {
            for normal in &d.instances.state.normals {
                instances.extend_from_slice(&normal.bytes());
            }
        }
        let a = self.gi.arrays.as_mut().unwrap();
        if a.instances != instances {
            let (models, normals) = instances.split_at(instances.len() / 2);
            if !models.is_empty() {
                self.queue.write_buffer(&a.models, 0, models);
                self.queue.write_buffer(&a.normals, 0, normals);
            }
            a.instances = instances;
        }
        Ok(())
    }

    /// Packs every draw's software geometry; false when a draw has none.
    fn prepare_gi_geometry(&mut self, draws: &[PreparedDraw<'_>]) -> Result<bool, Error> {
        let mut blases = vec![];
        for d in draws {
            let key: &wgpu::Buffer = &d.mesh.vertex;
            let Some((_, blas)) = self.gi.meshes.iter().find(|(k, _)| k == key) else {
                return Ok(false);
            };
            blases.push(blas.clone());
        }
        let meshes: Vec<wgpu::Buffer> = draws.iter().map(|d| (**d.mesh.vertex).clone()).collect();
        let instances: usize = draws.iter().map(|d| d.instances.len()).sum();
        if self
            .gi
            .packed
            .as_ref()
            .is_none_or(|p| p.meshes != meshes || p.instances != instances)
        {
            let refs: Vec<&Blas> = blases.iter().map(|b| &**b).collect();
            let (offsets, near, total) = gi_bvh::layout(&refs, instances);
            let words = gi_bvh::mesh_words(&refs, &offsets, &near, total);
            let bytes = (words.len() * 4) as u64;
            let near_bytes = refs
                .iter()
                .flat_map(|b| &b.near)
                .map(|n| n.masks.len() as u64 * 4)
                .sum();
            let buffer = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("indirect cache software geometry"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            self.queue.write_buffer(&buffer, 0, &raw);
            let packed = Packed {
                meshes,
                instances,
                offsets,
                near,
                near_bytes,
                head: vec![],
                buffer,
                bytes,
                _allocation: self.ledger.allocate(AllocationKind::Light, bytes),
            };
            if let Some(old) = self.gi.packed.replace(packed) {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Packed(old)));
            }
            if let Some(old) = self.gi.groups.take() {
                self.gi
                    .retired
                    .push((self.submissions, Retired::Groups(old)));
            }
        }
        let p = self.gi.packed.as_mut().unwrap();
        let mut records = vec![];
        for (((d, blas), offsets), near) in draws.iter().zip(&blases).zip(&p.offsets).zip(&p.near) {
            for model in &d.instances.state.models {
                records.push((*model, &**blas, *offsets, blas.near_base(model, near)));
            }
        }
        self.gi.stats.near_instances = records.iter().filter(|r| r.3 != 0).count() as u32;
        let head = gi_bvh::instance_words(&records)?;
        if p.head != head {
            let raw: Vec<u8> = head.iter().flat_map(|w| w.to_le_bytes()).collect();
            self.queue.write_buffer(&p.buffer, 0, &raw);
            p.head = head;
        }
        Ok(true)
    }

    /// Decides and uploads this frame's cache work. Runs after `prepare_primary`.
    /// `scene_generation` names this frame's TLAS, counting a build prepared but not yet
    /// encoded (as `prepare_primary` does), so hardware rays never trace last frame's scene.
    pub(super) fn prepare_gi(
        &mut self,
        draws: &[PreparedDraw<'_>],
        camera: &Camera,
        size: [u32; 2],
        eligible: bool,
        scene_generation: u64,
    ) -> Result<(), Error> {
        self.gi.plan = None;
        if self.gi.resources.is_none() {
            return Ok(());
        }
        let hardware = self.hardware_shadows.is_some() && !self.gi.settings.force_software;
        let run = self.gi.settings.enabled && eligible && self.gi_callers(draws);
        let software = !hardware;
        let run = run && (hardware || self.prepare_gi_geometry(draws)?);
        let eye = camera.view().inverse().w_axis.truncate();
        let s = self.gi.settings;
        let r = self.gi.resources.as_ref().unwrap();
        let mut header = [0u32; HEADER_WORDS as usize];
        header[0] = u32::from(run);
        header[1] = r.slots - 1;
        let field = self.environment.selected.is_some();
        // Probe statistics (GI_F_STATS) on profiled frames only.
        header[2] = u32::from(s.lookup == IndirectLookup::Trilinear)
            | (u32::from(field) << 1)
            | (u32::from(s.rotate_directions) << 2)
            | (u32::from(self.gi.profile.is_some()) << 3);
        header[3] = self.gi.frame;
        let footprint = 2.0 * (camera.fov_y() * 0.5).tan() / size[1] as f32 * s.cell_pixels;
        header[4] = footprint.to_bits();
        header[5] = s.min_edge.to_bits();
        header[6] = s.levels;
        header[7] = s.evict_frames;
        for (i, v) in eye.to_array().into_iter().enumerate() {
            header[8 + i] = v.to_bits();
        }
        header[11] = s.direction_phases;
        header[12] = s.level_blend.to_bits();
        header[13] = if s.parallel_rays { RAY_ITEMS } else { 0 };
        // The cut refill: S groups over the S cache frames that start at a reset.
        let since_clear = if self.gi.pending_clear {
            0
        } else {
            self.gi.since_clear
        };
        let cut_phases = match s.cut_phases {
            0 if software => 4,
            0 => 1,
            phases => phases,
        };
        let spread = cut_phases > 1 && since_clear < cut_phases;
        header[14] = if spread { cut_phases } else { 0 };
        // Software rays with cooked near-field masks: their octahedral bins per side (0: none).
        if software {
            header[16] = self.gi.near_bins;
        }
        let raw: Vec<u8> = header.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.queue.write_buffer(&r.table, 0, &raw);
        if !run {
            return Ok(());
        }
        self.ensure_gi_pipelines(software)?;
        self.prepare_gi_arrays(draws)?;
        // Moved casters: placements whose world bounds changed, as swept boxes.
        let mut bounds = vec![];
        for d in draws {
            for &model in &d.instances.state.models {
                let b = shadow_gpu::world_bounds(d.mesh.bounds, model)?;
                bounds.push([b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z]);
            }
        }
        let mut boxes: Vec<[f32; 6]> = vec![];
        let previous = std::mem::take(&mut self.gi.bounds);
        if !previous.is_empty() {
            for i in 0..bounds.len().max(previous.len()) {
                let (a, b) = (previous.get(i), bounds.get(i));
                if a == b {
                    continue;
                }
                let mut swept = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                for x in [a, b].into_iter().flatten() {
                    for k in 0..3 {
                        swept[k] = swept[k].min(x[k]);
                        swept[k + 3] = swept[k + 3].max(x[k + 3]);
                    }
                }
                boxes.push(swept);
            }
        }
        self.gi.bounds = bounds;
        if boxes.len() > MAX_BOXES {
            let all = boxes.iter().fold(boxes[0], |u, b| {
                std::array::from_fn(|k| {
                    if k < 3 {
                        u[k].min(b[k])
                    } else {
                        u[k].max(b[k])
                    }
                })
            });
            boxes = vec![all];
        }
        // An environment change re-integrates every cell from its bits (no rays):
        // yaw rotates the sky, a new map replaces it. A map's intensity and the constant
        // sky are applied at lookup, so their changes re-integrate nothing.
        let environment = {
            let mut h = Sha256::new();
            h.update(self.environment.generation.to_le_bytes());
            let u = self.environment.uniform();
            h.update(&u[..8]);
            h.finalize().into()
        };
        let environment_changed = self.gi.environment.is_some_and(|e| e != environment);
        self.gi.environment = Some(environment);
        let mut flags = 0u32;
        if !boxes.is_empty() && s.invalidate_casters {
            flags |= 1;
        }
        // Partially traced cells re-queue: form 2 always; the cut refill on the frames after
        // its first, and one frame past its window, where a cell still partial (made late in
        // the window) traces every remaining direction.
        let refill = cut_phases > 1 && (1..=cut_phases).contains(&since_clear);
        if s.direction_phases > 1 || refill {
            flags |= 8;
        }
        if s.rotation_period != 0 {
            flags |= 2;
        }
        // A step after a still frame re-integrates every cell; a map changing on
        // consecutive frames, and the ENVIRONMENT_PERIOD frames after it last changed,
        // re-integrate one fixed slot group a frame (ENVIRONMENT_PERIOD).
        let integrate_all = environment_changed && !self.gi.environment_streak;
        let spread = !integrate_all && (environment_changed || self.gi.environment_catchup > 0);
        self.gi.environment_catchup = if integrate_all {
            0
        } else if environment_changed {
            ENVIRONMENT_PERIOD
        } else {
            self.gi.environment_catchup.saturating_sub(1)
        };
        self.gi.environment_streak = environment_changed;
        let integrate = integrate_all || spread;
        let environment_group = if spread {
            self.gi.environment_phase = (self.gi.environment_phase + 1) % ENVIRONMENT_PERIOD;
            ENVIRONMENT_PERIOD | (self.gi.environment_phase << 8)
        } else {
            0
        };
        if integrate {
            flags |= 4;
        }
        let region = self.primary.as_ref().unwrap().scene;
        let r = self.gi.resources.as_ref().unwrap();
        let mut frame = vec![0u8; FRAME_BYTES];
        let mut put =
            |at: usize, v: u32| frame[at * 4..at * 4 + 4].copy_from_slice(&v.to_le_bytes());
        for (i, v) in region.into_iter().enumerate() {
            put(i, v);
        }
        put(4, boxes.len() as u32);
        put(5, s.rotation_period);
        put(6, flags);
        put(7, r.misses);
        let misses = COUNTER_WORDS as u32 + r.slots;
        let traces = misses + r.misses * MISS_WORDS as u32;
        put(8, r.slots);
        put(9, misses);
        put(10, traces);
        put(11, traces + r.slots);
        let items = traces + 2 * r.slots;
        put(12, items + RAY_ITEMS + RAY_ITEMS / 32);
        put(13, requests(r.misses) - 1);
        put(14, items);
        // The environment's slot group: period (0: every slot) and phase; bit 31 is set
        // below on environment-only frames, whose scan visits only that group.
        put(15, environment_group);
        for (i, b) in boxes.iter().enumerate() {
            for k in 0..3 {
                put(16 + i * 8 + k, b[k].to_bits());
                put(16 + i * 8 + 4 + k, b[k + 3].to_bits());
            }
        }
        self.queue.write_buffer(&r.frame, 0, &frame);
        let groups = [
            (region[2] - region[0]).div_ceil(probe_tile()),
            (region[3] - region[1]).div_ceil(probe_tile()),
        ];
        let clear = std::mem::take(&mut self.gi.pending_clear);
        // A view whose every input is unchanged has nothing to trace once its cells
        // exist: after IDLE_AFTER identical frames the passes stop (still frames cost
        // nothing). Rotation and partial cells keep working every frame.
        let idle_key: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(matrix_bytes(camera.matrix(size[0], size[1])));
            h.update(&self.gi.arrays.as_ref().unwrap().instances);
            h.update(environment);
            h.update(format!("{s:?}").as_bytes());
            h.update(region.map(u32::to_le_bytes).concat());
            h.finalize().into()
        };
        if self.gi.idle_key == Some(idle_key) && !clear {
            self.gi.idle_frames = self.gi.idle_frames.saturating_add(1);
        } else {
            self.gi.idle_frames = 0;
        }
        self.gi.idle_key = Some(idle_key);
        // The same view and scene with only the environment changing (a rotating sky over a
        // still camera): once that view is idle, no cell is new, so only the scan and the
        // integrate passes run. Such frames do not advance the cache frame, so seen stamps
        // do not age toward eviction while nothing is probed.
        let view_key: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(matrix_bytes(camera.matrix(size[0], size[1])));
            h.update(&self.gi.arrays.as_ref().unwrap().instances);
            h.update(format!("{s:?}").as_bytes());
            h.update(region.map(u32::to_le_bytes).concat());
            h.finalize().into()
        };
        if self.gi.view_key == Some(view_key) && !clear {
            self.gi.view_frames = self.gi.view_frames.saturating_add(1);
        } else {
            self.gi.view_frames = 0;
        }
        self.gi.view_key = Some(view_key);
        if self.gi.idle_frames >= IDLE_AFTER && flags == 0 {
            self.gi.stats.idle_frames += 1;
            return Ok(());
        }
        let environment_only = self.gi.view_frames >= IDLE_AFTER && flags == 4;
        let mut scan_groups = r.slots.div_ceil(64);
        if environment_only && spread {
            let word = (environment_group | (1 << 31)).to_le_bytes();
            self.queue.write_buffer(&r.frame, 60, &word);
            scan_groups = (r.slots / ENVIRONMENT_PERIOD).div_ceil(64);
        }
        self.gi.plan = Some(Plan {
            groups,
            scan_groups,
            scan: flags != 0,
            integrate,
            clear,
            environment_only,
        });
        self.prepare_gi_groups(software, scene_generation)?;
        self.gi.stats.frames += 1;
        self.gi.stats.software = software;
        if !environment_only {
            self.gi.since_clear = since_clear.saturating_add(1);
            self.gi.frame = self.gi.frame.wrapping_add(1);
        }
        Ok(())
    }

    fn prepare_gi_groups(&mut self, software: bool, scene_generation: u64) -> Result<(), Error> {
        let targets = self.primary.as_ref().unwrap().targets.as_ref().unwrap();
        // A new software geometry buffer drops the groups itself; transform changes rewrite it.
        let rays = if software {
            self.gi.packed.as_ref().map_or(0, |p| p.bytes)
        } else {
            scene_generation
        };
        let key = (
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            rays ^ (self.gi.generation << 40),
            software,
        );
        if self.gi.groups.as_ref().is_some_and(|g| g.key == key) {
            return Ok(());
        }
        let layouts = self.gi.layouts.as_ref().unwrap();
        let r = self.gi.resources.as_ref().unwrap();
        let hit = targets.hit.texture.create_view(&Default::default());
        let tiles = targets.tiles.texture.create_view(&Default::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&hit),
            },
            wgpu::BindGroupEntry {
                binding: 21,
                resource: wgpu::BindingResource::TextureView(&tiles),
            },
            wgpu::BindGroupEntry {
                binding: 30,
                resource: r.table.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 31,
                resource: r.work.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 32,
                resource: r.frame.as_entire_binding(),
            },
        ];
        if software {
            entries.push(wgpu::BindGroupEntry {
                binding: 35,
                resource: self.gi.packed.as_ref().unwrap().buffer.as_entire_binding(),
            });
        } else {
            entries.push(wgpu::BindGroupEntry {
                binding: 34,
                resource: wgpu::BindingResource::AccelerationStructure(
                    self.hardware_shadows.as_ref().unwrap().scene(),
                ),
            });
        }
        let main = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("indirect cache"),
                layout: &layouts.main[usize::from(software)],
                entries: &entries,
            }),
            ResourceKind::BindGroup,
        );
        if let Some(old) = self.gi.groups.replace(Groups { key, main }) {
            self.gi
                .retired
                .push((self.submissions, Retired::Groups(old)));
        }
        Ok(())
    }
}

impl GiState {
    /// The cache passes, after the primaries' hit records and before shading. `lighting`
    /// is lighting's kernel group (the header-only table at binding 20).
    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, lighting: &wgpu::BindGroup) {
        let Some(plan) = self.plan.take() else {
            return;
        };
        let r = self.resources.as_ref().unwrap();
        let p = self.pipelines.as_ref().unwrap();
        let g = self.groups.as_ref().unwrap();
        let arrays = &self.arrays.as_ref().unwrap().bind;
        if plan.clear {
            encoder.clear_buffer(&r.table, HEADER_WORDS * 4, None);
        }
        encoder.clear_buffer(&r.work, 0, Some(COUNTER_WORDS * 4));
        // The staged ray bits, after the counters, claims, misses, both queues and the ray
        // items. The request table after them is left clean by the last resolve round
        // (and by finish after a miss-list overflow).
        let staged_at = (COUNTER_WORDS
            + u64::from(r.slots) * 3
            + u64::from(r.misses) * MISS_WORDS
            + u64::from(RAY_ITEMS))
            * 4;
        encoder.clear_buffer(&r.work, staged_at, Some(u64::from(RAY_ITEMS / 32) * 4));
        let queries = self.profile.as_ref().map(|p| &*p.queries);
        let ring = self
            .profile
            .as_ref()
            .map_or(0, |p| (p.written % PROFILE_RING) as u32 * PROFILE_STAMPS);
        let stamps = |i: u32| {
            queries.map(|q| wgpu::ComputePassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: Some(ring + 2 * i),
                end_of_pass_write_index: Some(ring + 2 * i + 1),
            })
        };
        let main_pass = |encoder: &mut wgpu::CommandEncoder,
                         label: &str,
                         pipeline: &wgpu::ComputePipeline,
                         stamp: u32,
                         groups: Option<[u32; 2]>| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: stamps(stamp),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*g.main, &[]);
            pass.set_bind_group(1, &**arrays, &[]);
            pass.set_bind_group(3, lighting, &[]);
            if let Some([x, y]) = groups {
                pass.dispatch_workgroups(x, y, 1);
            }
        };
        // Direct fixed grids (CLAIM_GROUPS and the rest): kernels read their counts. Passes
        // with no work this frame (scan and integrate without an event) are not encoded;
        // when profiling they are empty passes, so every stamp is written.
        let profiling = queries.is_some();
        // Environment-only frames run only scan and integrate; when profiling, the other
        // passes are empty, so every stamp is written.
        let traced = !plan.environment_only;
        if traced || profiling {
            main_pass(
                encoder,
                "indirect cache probe",
                &p.probe,
                0,
                traced.then_some(plan.groups),
            );
            for k in 0..CLAIM_ROUNDS {
                let grid = traced.then_some([CLAIM_GROUPS, 1]);
                main_pass(encoder, "indirect cache claim", &p.claim, 1 + 2 * k, grid);
                let last = k + 1 == CLAIM_ROUNDS;
                let resolve = if last { &p.resolve_last } else { &p.resolve };
                main_pass(encoder, "indirect cache resolve", resolve, 2 + 2 * k, grid);
            }
        }
        let s = 1 + 2 * CLAIM_ROUNDS;
        if plan.scan || profiling {
            let groups = plan.scan.then_some([plan.scan_groups, 1]);
            main_pass(encoder, "indirect cache scan", &p.scan, s, groups);
        }
        let cells = Some([CELL_GROUPS, 1]);
        if traced || profiling {
            let setup = traced.then_some([SETUP_GROUPS, 1]);
            main_pass(encoder, "indirect cache setup", &p.setup, s + 1, setup);
            let rays = traced.then_some([RAY_GROUPS, 1]);
            main_pass(encoder, "indirect cache rays", &p.rays, s + 2, rays);
            let finish = cells.filter(|_| traced);
            main_pass(encoder, "indirect cache finish", &p.finish, s + 3, finish);
        }
        if plan.integrate || profiling {
            let groups = cells.filter(|_| plan.integrate);
            main_pass(
                encoder,
                "indirect cache integrate",
                &p.integrate,
                s + 4,
                groups,
            );
        }
        if let Some(profile) = self.profile.as_mut() {
            let slot = profile.written % PROFILE_RING;
            encoder.resolve_query_set(
                &profile.queries,
                ring..ring + PROFILE_STAMPS,
                &profile.resolve,
                slot * PROFILE_STRIDE,
            );
            profile.written += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{Stage, compose, uses_indirect};
    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");

    fn validate(label: &str, source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
    }

    #[test]
    fn kernels_validate_on_both_ray_sources() {
        for directions in [64, 128] {
            validate("hardware kernels", &kernel_source(false, directions));
            validate("software kernels", &kernel_source(true, directions));
        }
    }

    #[test]
    fn only_callers_compose_the_indirect_inputs() {
        let standard = compose(PRELUDE, crate::STANDARD_PROGRAM, Stage::Raster, false);
        assert!(standard.source.contains("var<storage,read> gi_table"));
        crate::program::analyse("standard", crate::STANDARD_PROGRAM, &standard).unwrap();
        for source in
            super::primary_ray_gpu::shading_sources(PRELUDE, crate::STANDARD_PROGRAM).unwrap()
        {
            validate("standard traced", &source);
        }
        let toon = include_str!("../shaders/programs/toon.wgsl");
        assert!(!uses_indirect(toon));
        let composed = compose(PRELUDE, toon, Stage::Raster, true);
        assert!(!composed.source.contains("gi_table"));
        assert!(!uses_indirect(
            "// indirect_irradiance(input,n)\n/* indirect_irradiance */ fn f() {}"
        ));
        assert!(uses_indirect(
            "fn f(i:SurfaceInput)->vec3<f32> { return indirect_irradiance(i,vec3<f32>(0.0,1.0,0.0)); }"
        ));
    }

    /// CPU mirror of gi_direction, gi_sh_basis and the band weights (f64).
    fn directions(n: [f64; 3], rotation: f64) -> Vec<[f64; 3]> {
        let s = if n[2] >= 0.0 { 1.0 } else { -1.0 };
        let a = -1.0 / (s + n[2]);
        let b = n[0] * n[1] * a;
        let t = [1.0 + s * n[0] * n[0] * a, s * b, -s * n[0]];
        let u = [b, s + n[1] * n[1] * a, -n[1]];
        (0..64)
            .map(|k| {
                let z = 1.0 - (k as f64 + 0.5) / 64.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                let phi = k as f64 * 2.399963229728653 + rotation;
                let (x, y) = (r * phi.cos(), r * phi.sin());
                std::array::from_fn(|i| t[i] * x + u[i] * y + n[i] * z)
            })
            .collect()
    }
    fn basis(d: [f64; 3]) -> [f64; 9] {
        let [x, y, z] = d;
        [
            0.282095,
            0.488603 * y,
            0.488603 * z,
            0.488603 * x,
            1.092548 * x * y,
            1.092548 * y * z,
            0.315392 * (3.0 * z * z - 1.0),
            1.092548 * x * z,
            0.546274 * (x * x - y * y),
        ]
    }
    fn band(i: usize) -> f64 {
        [
            std::f64::consts::PI,
            std::f64::consts::TAU / 3.0,
            std::f64::consts::FRAC_PI_4,
        ][match i {
            0 => 0,
            1..=3 => 1,
            _ => 2,
        }]
    }
    /// Cached irradiance at `shading` from visibility `v` over the fixed directions.
    fn cached(
        normal: [f64; 3],
        rotation: f64,
        shading: [f64; 3],
        v: impl Fn([f64; 3]) -> bool,
    ) -> f64 {
        let mut sh = [0.0; 9];
        for d in directions(normal, rotation) {
            if v(d) {
                for (c, y) in sh.iter_mut().zip(basis(d)) {
                    *c += std::f64::consts::TAU / 64.0 * y;
                }
            }
        }
        let y = basis(shading);
        (0..9).map(|i| band(i) * y[i] * sh[i]).sum::<f64>().max(0.0)
    }
    /// Reference: midpoint quadrature over the upper hemisphere (z, azimuth).
    fn exact(shading: [f64; 3], v: impl Fn([f64; 3]) -> bool) -> f64 {
        let (nz, np) = (800, 1600);
        let mut e = 0.0;
        for i in 0..nz {
            let z = (i as f64 + 0.5) / nz as f64;
            let r = (1.0 - z * z).sqrt();
            for j in 0..np {
                let phi = (j as f64 + 0.5) / np as f64 * std::f64::consts::TAU;
                let d = [r * phi.cos(), r * phi.sin(), z];
                let c = d[0] * shading[0] + d[1] * shading[1] + d[2] * shading[2];
                if c > 0.0 && v(d) {
                    e += c;
                }
            }
        }
        e * std::f64::consts::TAU / (nz * np) as f64
    }

    /// The representation alone (64 fixed directions, L2 SH): exact up to quadrature
    /// on an open plane for any shading normal, so open surfaces are not over-occluded;
    /// bounded error beside walls of known horizon. Spatial interpolation and query
    /// points are measured on the GPU benches (docs/m7-g1.md).
    #[test]
    fn representation_error_on_open_planes_and_walls() {
        let up = [0.0, 0.0, 1.0];
        let mut open: f64 = 0.0;
        for degrees in [0.0f64, 30.0, 60.0, 80.0] {
            let t = degrees.to_radians();
            let n = [t.sin(), 0.0, t.cos()];
            for rotation in [0.0, 1.3, 4.0] {
                let (c, e) = (cached(up, rotation, n, |_| true), exact(n, |_| true));
                open = open.max((c - e).abs() / e);
            }
        }
        // Measured 0.0145 (shading normal 80 degrees from the cell normal).
        eprintln!("G1 representation: worst relative error on an open plane {open:.4}");
        assert!(open < 0.02, "open-plane representation error {open}");
        // A wall at x = -d of height h: blocked when heading -x under the top edge.
        let mut worst: f64 = 0.0;
        for ratio in [0.25f64, 1.0, 4.0, 1e6] {
            let wall = |d: [f64; 3]| !(d[0] < 0.0 && d[2] / -d[0] < ratio);
            for rotation in [0.0, 0.7, 2.1, 3.3] {
                let (c, e) = (cached(up, rotation, up, wall), exact(up, wall));
                worst = worst.max((c - e).abs() / e);
            }
        }
        // Measured 0.0639, all of it quadrature: 1,024 directions give 0.004 and the
        // L2 truncation alone under 0.01 (docs/m7-g1.md).
        eprintln!("G1 representation: worst relative error beside a wall {worst:.4}");
        assert!(worst < 0.07, "wall representation error {worst}");
    }
}

#[cfg(test)]
#[path = "gi_cache_tests.rs"]
mod gpu_tests;
