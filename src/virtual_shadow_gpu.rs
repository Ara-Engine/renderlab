//! N8/N8b/N8c (docs/non-rt-n8.md, docs/non-rt-n8b.md, docs/non-rt-n8c.md):
//! `SoftShadows::Virtual`, Interactive's eight sun directions with no ray queries or
//! acceleration structure in the visibility pass.
//!
//! Near field: the receiver triangle and its vertex 1-ring (the exact-position edge
//! adjacency stored after each mesh's indices), tested with the watertight test over the
//! whole ray, unless a per-triangle ring horizon shows no ring triangle can block. Far
//! field: a sun clipmap whose levels hold about one texel per receiver pixel, each with two
//! 64-bit layers of (height, triangle id): the highest surface toward the sun and the next
//! one below it (a depth peel). Like a virtual shadow map, work is page-driven: receivers
//! request 128-texel pages at their level and every coarser one the march continues into,
//! only stale requested pages are rasterized, and rasterized pages stay cached until the sun
//! turns, the level's window moves, or a moved caster's light-space bounds touch them.
//! Casters are the meshes' visibility-buffer clusters, cut per level at a texel-scale error
//! (N8c) and rasterized in software (small clusters) or hardware into the stale pages only.
//! Memory is dense per level (no physical page pool). The march uses the map as an index
//! only: a candidate texel is decided by an exact walk to the triangle under the ray.
//!
//! Receivers and `(placement, primitive)` records come from traced primaries or the N1c
//! visibility buffer; the shadow pass itself only rasterizes and reads buffers.
//!
//! N8e (docs/non-rt-n8e.md) spends quality surplus over Unreal on cost: a walk cap, a map
//! that keeps its sun basis through small turns, and a rolling refresh that looks up one
//! trace tile in sixteen per frame while the camera, casters and settings hold still.
// PC1: only the device type changes; pipeline creation uses the shared cache.
use super::primary_ray_gpu::temporal_reconstruction::ZONES_BYTES;
use super::*;
use crate::pipeline_compilation::PipelineDevice;
use visibility_buffer::{ClusterInstances, cluster_data_group, data_entry};
#[path = "sunvis_s4.rs"]
mod sunvis_s4;
#[path = "virtual_shadow_cache.rs"]
mod surface_cache;
use surface_cache::SurfaceCache;
// VF1: a read-only fog accessor; cache internals and page work stay owned here.
#[path = "atmosphere_shadow.rs"]
pub(super) mod atmosphere_shadow;

/// Levels a frame can use; also the WGSL `VSM_MAX_LEVELS`.
pub(super) const MAX_LEVELS: usize = 8;
const MAX_RECTS: usize = 16;
const PAGE: u32 = 128;
/// WGSL `VsmFrame`: six vec4s, two uvec4s, eight levels, sixteen light-space and sixteen
/// screen rectangles, the TRACED_DRAWS per-batch horizon offsets and (first record, recorded) pairs,
/// N8e's uvec4, P-NSQ's sampling uvec4 (`SAMPLING_OFFSET`) and TR1's restart zones
/// (`ZONES_OFFSET`).
const FRAME_BYTES: usize = 6 * 16
    + 2 * 16
    + MAX_LEVELS * 32
    + 2 * MAX_RECTS * 16
    + 12 * crate::TRACED_DRAWS as usize
    + 16
    + 16
    + ZONES_BYTES;
/// `VsmFrame::nsq`: direction batches and the frame's sequence rotation (docs/p-nsq.md).
const SAMPLING_OFFSET: u64 = ZONES_OFFSET - 16;
/// `VsmFrame::zones` (P-RELIT): TR1's restart zones, where a receiver evaluates the restart
/// count instead of the frame's batches.
const ZONES_OFFSET: u64 = (FRAME_BYTES - ZONES_BYTES) as u64;
/// Per-draw `VsmHorizonPass` uniforms at the minimum dynamic-offset alignment.
const HORIZON_STRIDE: u64 = 256;
/// `VsmPass` uniforms (a clip level or a mip index) at the minimum dynamic-offset alignment.
const PASS_STRIDE: u64 = 256;
const PASS_ENTRIES: usize = 16;
/// Per-level WGSL `VsmArgs`: the software dispatch and the hardware draw.
const ARGS_BYTES: u64 = 32;
/// The WGSL `VsmScratch` header (N8m): the persistent lookup's tile queue, three reserved words
/// (the hybrid's queue keeps its spread in the first) and the tile order's bucket counts and
/// cursors (`VSM_HEAD` words), zeroed before each persistent lookup.
const SCRATCH_HEADER: u64 = 16 + 4 * 32;
/// N8m: the lookup schedule's defaults, the module's `VSM_PERSIST` and `VSM_ORDER` declarations
/// (shaders/virtual_shadow.wgsl). A module without them (a test variant file of main's lookup
/// before N8m) launches one workgroup per tile, unordered.
fn schedule_defaults(source: &str) -> (u32, bool) {
    let groups = source
        .split("override VSM_PERSIST:u32=")
        .nth(1)
        .and_then(|rest| rest.split('u').next()?.parse().ok())
        .unwrap_or(0);
    (groups, source.contains("override VSM_ORDER:bool=true;"))
}
/// Trace tiles (WGSL `VSM_TRACE_TILE`, 8x4 pixels) over a `size` image.
fn trace_tiles(size: [u32; 2]) -> u64 {
    u64::from(size[0].div_ceil(8)) * u64::from(size[1].div_ceil(4))
}
/// The scratch's bytes before the decision trace: the header, then per trace tile its place
/// in the order and its recorded work.
pub(super) fn scratch_trace_offset(size: [u32; 2]) -> u64 {
    SCRATCH_HEADER + 8 * trace_tiles(size)
}
/// Device features the page raster needs: 64-bit (height, id) max atomics. Casters are the
/// meshes' visibility-buffer clusters, so the device must also support
/// `visibility_buffer::supported` when meshes are uploaded.
pub const VIRTUAL_FEATURES: wgpu::Features =
    wgpu::Features::SHADER_INT64.union(wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX);

/// Tuning of `SoftShadows::Virtual`. Surface is the default lookup (docs/non-rt-n8p.md);
/// Exact remains selectable as the reference. Other defaults follow docs/non-rt-n8.md,
/// non-rt-n8c.md and non-rt-n8e.md; N8c's output is
/// `walk_cap: 0, refresh: 1, sun_hysteresis: 0.0` with `lookup: VirtualLookup::Exact`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualShadowSettings {
    /// Metres beyond which other surfaces are found only through the map. The receiver's
    /// own 1-ring is always tested exactly over the whole ray.
    pub near: f32,
    /// log2 of map texels per screen pixel at the receiver; 0 is one texel per pixel.
    pub texels_per_pixel_log2: f32,
    /// Window side in 128-texel pages, a power of two from 4 to 32.
    pub pages: u32,
    /// Map samples per direction beyond `near`, geometrically spaced.
    pub march_samples: u32,
    /// Texel of the finest absolute level, metres; level k has `finest_texel * 2^k`.
    pub finest_texel: f32,
    /// Height-comparison bias in units of 2^-24 of the scene's height range.
    pub bias_ulps: u32,
    /// Walk the receiver's vertex 1-ring for the exact near test; false tests only the
    /// receiver's own triangle (cheaper; nearby own-surface blockers then come from the map).
    pub ring: bool,
    /// Also test the peel layer where it belongs to the same placement as the top layer
    /// (N8b): surfaces hidden under their own mesh, such as scan overhangs. Off, those are
    /// treated as folds the top layer decides: fewer exact walks (the quarry's static shadow
    /// cost falls from about 4.9 to 2.8 ms) for more leaks (docs/non-rt-n8b.md).
    pub peel_own: bool,
    /// Caster LOD (N8c): a level rasterizes each mesh's clusterLOD cut whose simplification
    /// error is within this many of its texels. Negative rasterizes original geometry only.
    /// Meshes without an attached hierarchy always cast original geometry. Opt-in: on the
    /// quarry the cut's caster surfaces shadow lit receivers beyond Unreal's floor, with and
    /// without N1c's receiver lift, and moving shadow origins out by the cut error instead
    /// loses too many umbra receivers (docs/non-rt-n8c.md). Hence the negative default.
    pub caster_error_texels: f32,
    /// The march's first step also tests the 2x2 texel block around the ray: near-contact
    /// occluder edges that miss the texel centre (N8b). 0 off; 1 every surface (N8b); 2 only
    /// other placements' surfaces beyond the texel under the ray (N8c): their contacts are
    /// what it was for (docs/non-rt-n8b.md S1), and the receiver's own relief there cost the
    /// quarry 0.85 ms for few blocks.
    pub first_block: u32,
    /// N8e: directions evaluated per batch, 1-8: the first of each receiver's R2 sequence.
    /// Visibility is the visible share of those (docs/non-rt-n8e.md). P-NSQ: on TR1 frames
    /// the Exact and Surface lookups follow RT Interactive's sampling: the engine's
    /// `ReconstructionShadowBudget` sets the batches (its ray count over eight, so 256, 64,
    /// 32 or 8 directions with the default 8) and every frame rotates each pixel's sequence
    /// as RT does, so TR1's history integrates the disk (docs/p-nsq.md).
    pub directions: u32,
    /// N8e walk cap: once a receiver's exact walks have visited this many triangles and one
    /// of its directions is proven blocked, every direction still open is taken as blocked
    /// (0: no cap, the default since P-NSQ). It cut leaks and a sixth of the cost
    /// (docs/non-rt-n8e.md), but a penumbra receiver of a distant caster always reaches it,
    /// so it turned wide penumbrae into umbra (docs/p-nsq.md). `truncated_walks_block` now
    /// decides the undecided directions one by one.
    pub walk_cap: u32,
    /// N8e rolling refresh: on a frame with the last frame's camera, casters, settings and
    /// map, one trace tile in every `refresh` (1, 2, 4, 8 or 16) is looked up again and the
    /// others keep their counts, so each is at most `refresh - 1` frames old. With the sun
    /// turning inside `sun_hysteresis`, kept counts lag the sun by up to that many frames.
    /// 1 looks up every tile every frame.
    pub refresh: u32,
    /// N8g/N8h per-triangle walk records (docs/non-rt-n8h.md): 36-byte copies of a
    /// triangle's position bits, which the lookup reads instead of index -> vertex chains.
    /// They change cost and VRAM, never output.
    pub walk_records: WalkRecords,
    /// N8e map hysteresis (radians): the clipmap keeps its light basis, and its cached pages,
    /// while the sun stays within this angle of the direction it was rasterized for. Every
    /// lookup still tests the current sun's directions exactly; the map only finds candidates,
    /// and its march and page requests widen their cone by this angle. 0 rasterizes again
    /// whenever the sun turns.
    pub sun_hysteresis: f32,
    /// Which lookup decides visibility from the shared clipmap: Surface by default,
    /// Exact for the reference (docs/non-rt-n8p.md).
    pub lookup: VirtualLookup,
    /// P-NSQ (docs/p-nsq.md), Exact and Surface lookups: a direction one of whose candidate
    /// walks ran out of steps before locating the triangle under its aim is blocked once its
    /// march ends (that map texel's surface is above the ray), and only that direction.
    /// Every other direction keeps its exact verdict.
    pub truncated_walks_block: bool,
    /// Triangles one candidate walk may cross at the receiver's level before it is
    /// undecided (32; N8b used 8); each coarser level doubles it, up to four times. Walks that
    /// locate their triangle sooner stop sooner, so a larger budget costs only long walks.
    pub walk_steps: u32,
    /// P-S4B/P-S4C (docs/p-s4b.md, docs/p-s4c.md): on Surface frames of 8 or more direction
    /// batches (cuts and history restarts) where it is measured to beat the march
    /// (`sunvis_s4::chosen`: large screens, small suns), each queued pixel's directions are
    /// decided by S4's occluder-centric gather instead of the march: every direction's exact hit
    /// test over every triangle in its cone, on game-class scenes whose GI keeps full-detail
    /// geometry (src/sunvis_s4.rs). Other frames and scenes keep the march. Scaffolding until S4
    /// replaces the march everywhere (docs/p-s4c.md "Retiring the march"), then it goes.
    pub occluder_gather: bool,
}

/// The per-pixel decision of `SoftShadows::Virtual` (`VirtualShadowSettings::lookup`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VirtualLookup {
    /// Exact watertight tests of the receiver's 1-ring and of every map candidate's triangles
    /// (shaders/virtual_shadow.wgsl): the reference.
    Exact,
    /// N8k: light-space map data only (shaders/virtual_shadow_lookup.wgsl): per-texel height
    /// planes and planarity errors written at page-draw time, rebuilt between texel centres.
    /// No triangle is read per pixel, and no walk records or ring horizons are kept.
    Map,
    /// N8k: `Map`, then the exact lookup only for the receivers whose map decision used an
    /// ambiguous block (shaders/virtual_shadow_hybrid.wgsl); keeps both lookups' resources.
    Hybrid,
    /// The default. N8n/N8o: `Exact`'s walks kept per receiver surface cell across frames in a hash table
    /// (shaders/virtual_shadow_cache.wgsl, docs/non-rt-n8n.md, docs/non-rt-n8o.md). A pixel
    /// shows its cell's count only where the cell is solid and every pixel that computed it agreed
    /// with its neighbourhood; every other pixel walks as `Exact` does. Static frames run nothing,
    /// and camera motion walks only shadow-boundary pixels and cells not seen yet. Deterministic;
    /// caster motion retires exactly the cells it can reach. Adds the table and its claim words
    /// (64 MiB) and the miss queue (8 bytes per pixel).
    #[default]
    Surface,
    /// N-VSM1 (docs/n-vsm1.md, shaders/virtual_shadow_disk.wgsl): the disk's visible share
    /// from the cached map alone. Every map texel near the receiver is projected back onto
    /// the sun disk and the blocked directions are OR-ed into a 52-sample mask, level by
    /// level as the occluder's height grows; the visible surface around the pixel supplies
    /// the occluders closer than a texel. No triangle is read per pixel, no walk records or
    /// ring horizons are kept, and the cost does not depend on TR1's shadow budget, so a cut
    /// frame costs what a moving frame does.
    Disk,
}
impl VirtualLookup {
    /// Whether the map lookup and its per-texel data run.
    fn map(self) -> bool {
        matches!(self, Self::Map | Self::Hybrid)
    }
    /// Whether the lookup reads triangles: ring horizons and walk records are kept for it.
    fn walks(self) -> bool {
        !matches!(self, Self::Map | Self::Disk)
    }
    /// Whether the lookup queues receivers in `Resources::queue` (8 bytes per pixel).
    fn queues(self) -> bool {
        matches!(self, Self::Hybrid | Self::Surface)
    }
}
impl Default for VirtualShadowSettings {
    fn default() -> Self {
        Self {
            near: 0.002,
            texels_per_pixel_log2: 0.0,
            pages: 16,
            march_samples: 10,
            finest_texel: 1.0 / 16384.0,
            bias_ulps: 4,
            ring: true,
            peel_own: false,
            caster_error_texels: -1.0,
            first_block: 2,
            directions: 8,
            walk_cap: 0,
            refresh: 16,
            sun_hysteresis: 0.01,
            walk_records: WalkRecords::Source,
            lookup: VirtualLookup::Surface,
            truncated_walks_block: true,
            walk_steps: 32,
            occluder_gather: true,
        }
    }
}
impl VirtualShadowSettings {
    fn validate(&self) -> Result<(), Error> {
        if !(self.near.is_finite() && self.near > 0.0)
            || !self.texels_per_pixel_log2.is_finite()
            || !self.pages.is_power_of_two()
            || !(4..=32).contains(&self.pages)
            || !(2..=64).contains(&self.march_samples)
            || !(self.finest_texel.is_finite() && self.finest_texel > 0.0)
            || !self.caster_error_texels.is_finite()
            || self.first_block > 2
            || !(1..=8).contains(&self.directions)
            || ![1, 2, 4, 8, 16].contains(&self.refresh)
            || !(self.sun_hysteresis.is_finite() && (0.0..=0.05).contains(&self.sun_hysteresis))
            || !(1..=256).contains(&self.walk_steps)
        {
            return Err(Error::InvalidInput(
                "virtual shadow settings: near > 0, pages a power of two in [4,32], 2..=64 samples, positive texel, finite caster error, 1..=8 directions, refresh 1/2/4/8/16, hysteresis in [0, 0.05] rad, 1..=256 walk steps",
            ));
        }
        Ok(())
    }
}

/// Which triangles get walk records (`VirtualShadowSettings::walk_records`), 36 bytes each.
/// On the no-RT quarry, Source keeps the speed of All at half its memory (docs/non-rt-n8h.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkRecords {
    /// None: the lookup reads the index, vertex and adjacency buffers.
    Off,
    /// Each mesh's source triangles: every caster, and every receiver of traced visibility.
    Source,
    /// Source triangles and appended LOD triangles (the visibility buffer's LOD receivers).
    All,
}

/// Cache counters for the last prepared frame (tests and reports).
#[derive(Clone, Copy, Debug, Default)]
pub struct VirtualShadowStats {
    pub levels: u32,
    pub finest_level: i32,
    /// Levels whose cached pages were all invalidated this frame (sun, window, range).
    pub invalidated_levels: u32,
    /// Moved-caster rectangles uploaded this frame.
    pub moved_rects: u32,
    /// Caster cluster instances the page raster culls.
    pub clusters: u32,
    pub bytes: u64,
    /// Whether this frame ran the request, compaction and raster passes.
    pub page_work: bool,
    /// P-S4B: the scene qualifies for the occluder gather (its records are resident), and this
    /// frame ran it.
    pub occluder_gather_ready: bool,
    pub occluder_gather_frame: bool,
}

struct Pipelines {
    mark: Owned<wgpu::ComputePipeline>,
    compact: Owned<wgpu::ComputePipeline>,
    clear: Owned<wgpu::ComputePipeline>,
    cull: Owned<wgpu::ComputePipeline>,
    finish: Owned<wgpu::ComputePipeline>,
    mip: Owned<wgpu::ComputePipeline>,
    /// Software and hardware rasters: top layer, then peel layer.
    software: [Owned<wgpu::ComputePipeline>; 2],
    hardware: [Owned<wgpu::RenderPipeline>; 2],
    lookup: Owned<wgpu::ComputePipeline>,
    horizon: Owned<wgpu::ComputePipeline>,
    /// N8g walk records (shaders/virtual_shadow_records.wgsl).
    record: Owned<wgpu::ComputePipeline>,
    /// The lookup with `VSM_DEBUG`, built on first use (tests only).
    debug: Option<Owned<wgpu::ComputePipeline>>,
    /// N8m: the tile order's count and place passes, built when a persistent lookup first
    /// orders its tiles.
    order: Option<[Owned<wgpu::ComputePipeline>; 2]>,
    /// N8k (shaders/virtual_shadow_lookup.wgsl): per-texel planes of rasterized pages, and
    /// the map-only lookup.
    resolve: Owned<wgpu::ComputePipeline>,
    map: Owned<wgpu::ComputePipeline>,
    /// N8k: this frame's mip dispatch arguments (zero without a rendered page), and the top
    /// layer's min-mips.
    mip_dispatch: Owned<wgpu::ComputePipeline>,
    minmip: Owned<wgpu::ComputePipeline>,
    /// N8k hybrid: the queue's indirect dispatch, and the exact lookup over the queue (built
    /// on first use with the exact lookup's constants).
    queue_args: Owned<wgpu::ComputePipeline>,
    queued: Option<Owned<wgpu::ComputePipeline>>,
    /// N-VSM1 (shaders/virtual_shadow_disk.wgsl): its page requests and its lookup.
    disk_mark: Owned<wgpu::ComputePipeline>,
    disk: Owned<wgpu::ComputePipeline>,
}
struct Layouts {
    mark: Owned<wgpu::BindGroupLayout>,
    compact: Owned<wgpu::BindGroupLayout>,
    cull: Owned<wgpu::BindGroupLayout>,
    raster: Owned<wgpu::BindGroupLayout>,
    mip: Owned<wgpu::BindGroupLayout>,
    data: Owned<wgpu::BindGroupLayout>,
    pass: Owned<wgpu::BindGroupLayout>,
    lookup: Owned<wgpu::BindGroupLayout>,
    horizon: Owned<wgpu::BindGroupLayout>,
    /// The lookup's group 2: every batch's walk records.
    records: Owned<wgpu::BindGroupLayout>,
    /// N8k: the plane resolve's and the map-only lookup's group 0.
    resolve: Owned<wgpu::BindGroupLayout>,
    map: Owned<wgpu::BindGroupLayout>,
    mip_dispatch: Owned<wgpu::BindGroupLayout>,
    minmip: Owned<wgpu::BindGroupLayout>,
    queue: Owned<wgpu::BindGroupLayout>,
    /// N-VSM1: the disk lookup's group 0.
    disk: Owned<wgpu::BindGroupLayout>,
}
/// Ring horizons (N8b lever 1): 4 bytes per triangle of every traced draw, computed once
/// per draw list by `vsm_horizon`. Elevations do not change under the similarity transforms
/// the lookup accepts, so camera, sun and instance motion keep them.
///
/// N8g walk records, built with them: per recorded triangle, its object-space position bits
/// in 36 contiguous bytes (`RECORD_BYTES`), so the lookup's ring fans and walks read one
/// record per triangle instead of index -> vertex chains. Camera, sun and instance
/// independent; copied bits, so the lookup's output is unchanged. N8h packs every batch's
/// records into one numbering, so all but the last segment are whole 72 MiB buffers.
struct Horizons {
    /// Each draw's index buffer, addressed triangle count and recorded triangle count, in
    /// batch order.
    key: Vec<(usize, u32, u32, u64)>,
    /// First word of each batch's triangles.
    offsets: Vec<u32>,
    buffer: Owned<wgpu::Buffer>,
    params: Owned<wgpu::Buffer>,
    group: Owned<wgpu::BindGroup>,
    /// Per batch its first record and recorded triangles (both 0 without records).
    records: Vec<[u32; 2]>,
    /// The record segments, then per build piece (one batch within one segment) its triangle
    /// count and build group (pass uniforms in `record_params`: batch, count, first triangle,
    /// first record in the segment).
    segments: Vec<Owned<wgpu::Buffer>>,
    pieces: Vec<(u32, u32, Owned<wgpu::BindGroup>)>,
    record_params: Owned<wgpu::Buffer>,
    /// The lookup's group 2 over `records`, placeholders after them.
    record_array: Owned<wgpu::BindGroup>,
    _absent: Owned<wgpu::Buffer>,
    _allocation: Allocation,
    /// Whether this frame computes them (the first frame of a draw list).
    dispatch: bool,
    /// SK1: only changed poses rebuild their horizons and copied walk records.
    dirty: Vec<bool>,
}
/// One walk record: three corners' position bits.
const RECORD_BYTES: u64 = 36;
/// Records per segment (WGSL `1 << VSM_SEGMENT_BITS`): 72 MiB, within the default 128 MiB
/// storage binding.
const SEGMENT: u32 = 1 << 21;
const SEGMENT_BYTES: u64 = SEGMENT as u64 * RECORD_BYTES;
/// Build pieces at most: every batch plus every segment boundary.
const MAX_PIECES: u64 = 2 * crate::TRACED_DRAWS as u64;
/// The draw list's caster clusters (N8c): every draw's clusters per placement, their raster
/// lists and the cluster data bindings. Rebuilt when the draw list changes.
struct Casters {
    /// Each draw's index buffer and placement count.
    key: Vec<(usize, usize)>,
    /// Bumped on every rebuild, so bind groups over these buffers follow.
    generation: u64,
    instances: ClusterInstances,
    /// Per level the raster's indirect arguments (`ARGS_BYTES`), then per level software
    /// and hardware cluster slots (`instances.count` each).
    work: Owned<wgpu::Buffer>,
    data: Owned<wgpu::BindGroup>,
    _absent: Owned<wgpu::Buffer>,
    /// Bits of the largest primitive ID (source and appended LOD triangles) in the map IDs.
    primitive_bits: u32,
    _allocation: Allocation,
}
/// Map storage sized by (levels, pages) and the per-pixel level target by image size.
struct Resources {
    levels: usize,
    pages: u32,
    size: [u32; 2],
    /// Both layers: one 64-bit word per texel per level (normalized height << 32 | id).
    top: Owned<wgpu::Buffer>,
    peel: Owned<wgpu::Buffer>,
    /// Max-mips of the top layer's heights per level (mips 1..log2(side)).
    mips: Owned<wgpu::Buffer>,
    /// N8k: per texel per level, the top and peel planes (8 bytes) and their planarity
    /// errors (4 bytes); 16 bytes each unless the lookup is `VirtualLookup::Map`.
    planes: Owned<wgpu::Buffer>,
    errors: Owned<wgpu::Buffer>,
    /// N8k: min-mips of the top layer (as `mips`; 16 bytes unless `VirtualLookup::Map`), and
    /// the max- and min-mip passes' indirect arguments, three words per mip.
    minmips: Owned<wgpu::Buffer>,
    mip_args: Owned<wgpu::Buffer>,
    /// N8k hybrid: the queue of ambiguous receivers (count, three reserved words, then two
    /// words per receiver; 32 bytes unless `VirtualLookup::Hybrid`) and its dispatch.
    queue: Owned<wgpu::Buffer>,
    queue_dispatch: Owned<wgpu::Buffer>,
    /// N-VSM1: the disk lookup's WGSL `VdFrame` (`DISK_FRAME_BYTES`).
    disk_frame: Owned<wgpu::Buffer>,
    /// The hardware raster's attachment, which only sizes its raster area.
    _area: Owned<wgpu::Texture>,
    area: Owned<wgpu::TextureView>,
    _level: Owned<wgpu::Texture>,
    level_view: Owned<wgpu::TextureView>,
    /// The lookup's scratch (`VsmScratch`, N8m): the header, the per-tile order and work
    /// words (`scratch_trace_offset`), then the per-pixel decision trace (`VSM_DEBUG`), two
    /// uvec4s per pixel, which is one placeholder uvec4 unless `VirtualState::debug`.
    debug: Owned<wgpu::Buffer>,
    debug_sized: bool,
    frame: Owned<wgpu::Buffer>,
    requests: Owned<wgpu::Buffer>,
    state: Owned<wgpu::Buffer>,
    /// Per page slot: 1 when it is cleared and rasterized this frame.
    render: Owned<wgpu::Buffer>,
    /// N8f: the caster cull's indirect dispatch, set by `vsm_compact` once a page is stale.
    cull_args: Owned<wgpu::Buffer>,
    pass: Owned<wgpu::Buffer>,
    _allocation: Allocation,
    groups: Option<Groups>,
}
/// Bind groups over this frame's receiver targets and casters; rebuilt when any changes.
struct Groups {
    key: [wgpu::Texture; 4],
    models: wgpu::Buffer,
    horizons: wgpu::Buffer,
    casters: u64,
    mark: Owned<wgpu::BindGroup>,
    compact: Owned<wgpu::BindGroup>,
    cull: Owned<wgpu::BindGroup>,
    raster: Owned<wgpu::BindGroup>,
    mip: Owned<wgpu::BindGroup>,
    pass: Owned<wgpu::BindGroup>,
    lookup: Owned<wgpu::BindGroup>,
    resolve: Owned<wgpu::BindGroup>,
    map: Owned<wgpu::BindGroup>,
    mip_dispatch: Owned<wgpu::BindGroup>,
    minmip: Owned<wgpu::BindGroup>,
    queue: Owned<wgpu::BindGroup>,
    disk: Owned<wgpu::BindGroup>,
    /// The exact lookup's group with the queue in its scratch binding.
    queued: Owned<wgpu::BindGroup>,
    /// N8n: the surface cache's probe, dispatch and insert groups.
    surface: Option<[Owned<wgpu::BindGroup>; 3]>,
}
/// What the cache remembers between frames to decide which pages are still valid.
#[derive(Default)]
struct Cache {
    light: Option<[u32; 3]>,
    /// The sun direction the map was rasterized for (N8e `sun_hysteresis`).
    toward: Option<glam::DVec3>,
    finest: Option<i32>,
    levels: usize,
    /// Window minimum page per level, in absolute light-space page units.
    origins: [Option<[i64; 2]>; MAX_LEVELS],
    epochs: [u32; MAX_LEVELS],
    /// Grow-only caster height range along the light, which the depth encoding uses.
    heights: Option<[f64; 2]>,
    models: Vec<[u32; 16]>,
    bounds: Vec<crate::shadow_plan::Bounds>,
    /// Skinned poses per placement (`skinning::placement_poses`).
    poses: Vec<crate::renderer::skinning::PlacementPose>,
}
pub(super) struct VirtualState {
    pub settings: VirtualShadowSettings,
    pub stats: VirtualShadowStats,
    layouts: Layouts,
    lookup_layout: Owned<wgpu::PipelineLayout>,
    pipelines: Pipelines,
    resources: Option<Resources>,
    retired: Vec<(u64, Resources)>,
    cache: Cache,
    casters: Option<Casters>,
    retired_casters: Vec<(u64, Casters)>,
    levels: usize,
    /// Page requests depend only on the camera, the sun, the casters and the cache
    /// windows. A frame identical in all of them finds nothing stale, so it skips the
    /// request, compaction and raster passes and only runs the lookup.
    key: Vec<u64>,
    page_work: bool,
    /// N8e rolling refresh: frames since the last full lookup, whether the next frame may
    /// roll (cleared by the renderer when another pass writes the visibility target), and
    /// this frame's schedule word (`VsmFrame::n8e.y`; 0 looks up every tile).
    history_frames: u32,
    pub rolling: bool,
    schedule: u32,
    /// N8f: this rolling frame moved casters only, so the lookup visits every tile and also
    /// looks up the pixels they can reach (`VsmFrame::n8e.w` bit 0).
    casters_only: bool,
    horizons: Option<Horizons>,
    /// Tests: run the `VSM_DEBUG` lookup and keep its per-pixel trace in `debug_buffer`.
    pub debug: bool,
    /// Tests: lookup override constants for variant screens (`built`: those compiled).
    pub constants: Vec<(&'static str, f64)>,
    built: Vec<(&'static str, f64)>,
    module: Owned<wgpu::ShaderModule>,
    /// N8m: the module's lookup schedule defaults (`schedule_defaults`).
    lookup_defaults: (u32, bool),
    /// N8k: shaders/virtual_shadow_lookup.wgsl and the map lookup's layout (rebuilt with
    /// the screens' `VMAP_` constants).
    map_module: Owned<wgpu::ShaderModule>,
    map_layout: Owned<wgpu::PipelineLayout>,
    /// N8m: the hybrid queue dispatch's layout (its pass is rebuilt with the `VMAP_` constants).
    queue_layout: Owned<wgpu::PipelineLayout>,
    /// N-VSM1: shaders/virtual_shadow_disk.wgsl and its lookup's layout (rebuilt with the
    /// screens' `VD_` constants).
    disk_module: Owned<wgpu::ShaderModule>,
    disk_layout: Owned<wgpu::PipelineLayout>,
    /// N8k hybrid: the exact module with shaders/virtual_shadow_hybrid.wgsl appended (first use).
    hybrid_module: Option<Owned<wgpu::ShaderModule>>,
    /// N8n: the surface cache while `VirtualLookup::Surface` is the lookup; kept across resets,
    /// which retire its cells.
    surface: Option<SurfaceCache>,
    /// P-S4B: the occluder gather while the Surface lookup and `occluder_gather` are on.
    s4: Option<sunvis_s4::S4>,
}
/// Screen constants with this prefix belong to the map lookup's module, the others to the
/// exact one.
const MAP_CONSTANTS: &str = "VMAP_";
/// And these to the disk lookup's.
const DISK_CONSTANTS: &str = "VD_";
/// The disk lookup's window radius in texels per level (WGSL `VD_R`): a level serves the
/// occluder heights whose reach is half to all of this many of its texels.
const DISK_RADIUS: u32 = 3;
/// WGSL `VdFrame`: the world-to-clip matrix, the image size, the 65 y-rank masks and the 64
/// sample positions.
const DISK_FRAME_BYTES: u64 = 80 + 33 * 16 + 32 * 16;
/// The disk lookup's 64 sample directions (tools/n-vsm1/pattern.py): sample s has the s-th
/// smallest x in the disk, and this is its y rank. Both coordinates sit at the 64 quantiles of
/// the disk's marginal distribution (`disk_quantile`). No sample lies outside the disk, and the
/// nearest pair is 0.205 radii apart (a square lattice of 64 in the disk: 0.222).
const DISK_Y_RANK: [u8; 64] = [
    34, 48, 19, 40, 30, 11, 51, 4, 23, 36, 57, 45, 13, 29, 6, 21, 1, 59, 42, 50, 33, 9, 63, 17, 25,
    55, 46, 38, 5, 61, 12, 20, 0, 28, 56, 49, 41, 7, 14, 62, 22, 31, 54, 2, 39, 47, 16, 60, 8, 24,
    53, 32, 43, 3, 18, 10, 26, 58, 35, 44, 52, 15, 27, 37,
];
/// The coordinate below which share `u` of a unit disk's area lies (the inverse of WGSL
/// `vd_share`).
fn disk_quantile(u: f64) -> f64 {
    let share = |g: f64| 0.5 + (g * (1.0 - g * g).sqrt() + g.asin()) / std::f64::consts::PI;
    let (mut lo, mut hi) = (-1.0, 1.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if share(mid) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}
/// `VdFrame`'s constant tail: per k in 0..=64 the mask of the samples whose y rank is at least
/// k (two u32s each, padded to 33 vec4s), then the 64 sample positions.
fn disk_pattern() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(33 * 16 + 32 * 16);
    for k in 0..66u8 {
        let mask = (0..64).fold(0u64, |m, s| m | u64::from(DISK_Y_RANK[s] >= k) << s);
        bytes.extend_from_slice(&(mask as u32).to_ne_bytes());
        bytes.extend_from_slice(&((mask >> 32) as u32).to_ne_bytes());
    }
    let quantile = |rank: usize| disk_quantile((rank as f64 + 0.5) / 64.0) as f32;
    for s in 0..64 {
        bytes.extend_from_slice(&quantile(s).to_ne_bytes());
        bytes.extend_from_slice(&quantile(usize::from(DISK_Y_RANK[s])).to_ne_bytes());
    }
    bytes
}

fn layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}
fn storage(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        visibility,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}
fn frame_uniform(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        0,
        visibility,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(FRAME_BYTES as u64),
        },
    )
}
fn sampled(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        wgpu::ShaderStages::COMPUTE,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        },
    )
}
fn written(binding: u32) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        wgpu::ShaderStages::COMPUTE,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}

/// The module source: the shared draw-slot count, then shaders/virtual_shadow.wgsl. Tests
/// may substitute a shader variant file through `RENDERLAB_VSM_SHADER` (N8d cost bisection
/// without a rebuild per variant).
pub(super) fn shader_source() -> String {
    #[cfg(test)]
    let variant = std::env::var("RENDERLAB_VSM_SHADER")
        .ok()
        .map(|path| std::fs::read_to_string(path).unwrap());
    #[cfg(not(test))]
    let variant: Option<String> = None;
    // P-RELIT: the restart-zone tests over VsmFrame's copy (vsm_prepare's batches).
    format!(
        "enable wgpu_binding_array;\nconst PRIMARY_BATCHES:u32={}u;\n{}\n{}",
        crate::TRACED_DRAWS,
        variant
            .as_deref()
            .unwrap_or(include_str!("../shaders/virtual_shadow.wgsl")),
        super::primary_ray_gpu::temporal_reconstruction::zones_source("vsm.zones")
    )
}

/// The walk-record build module (N8g): the shared draw-slot count, then
/// shaders/virtual_shadow_records.wgsl.
fn records_source() -> String {
    format!(
        "enable wgpu_binding_array;\nconst PRIMARY_BATCHES:u32={}u;\n{}",
        crate::TRACED_DRAWS,
        include_str!("../shaders/virtual_shadow_records.wgsl")
    )
}

/// The N8k hybrid's exact module: shader_source() with shaders/virtual_shadow_hybrid.wgsl
/// appended, whose entry calls the exact lookup's per-pixel functions.
pub(super) fn hybrid_source() -> String {
    format!(
        "{}\n{}",
        shader_source(),
        include_str!("../shaders/virtual_shadow_hybrid.wgsl")
    )
}

/// The N8k map-only lookup module: the shared draw-slot count, then
/// shaders/virtual_shadow_lookup.wgsl. Tests may substitute a variant file through
/// `RENDERLAB_VMAP_SHADER`, as `RENDERLAB_VSM_SHADER` does for the exact lookup.
pub(super) fn map_source() -> String {
    #[cfg(test)]
    let variant = std::env::var("RENDERLAB_VMAP_SHADER")
        .ok()
        .map(|path| std::fs::read_to_string(path).unwrap());
    #[cfg(not(test))]
    let variant: Option<String> = None;
    format!(
        "enable wgpu_binding_array;\nconst PRIMARY_BATCHES:u32={}u;\n{}",
        crate::TRACED_DRAWS,
        variant
            .as_deref()
            .unwrap_or(include_str!("../shaders/virtual_shadow_lookup.wgsl"))
    )
}

/// The N-VSM1 disk lookup module: the window radius, then shaders/virtual_shadow_disk.wgsl.
/// Measurement only: `RENDERLAB_VD=NAME=VALUE,...` replaces the defaults of the module's `VD_`
/// override constants (screens and cost splits without a build per variant; docs/n-vsm1.md).
/// `RENDERLAB_VD_SHADER=PATH` reads the module from a variant file instead (the same layouts
/// and entries), as `RENDERLAB_VSM_SHADER` does for the exact lookup. Unset, the module is the
/// file.
pub(super) fn disk_source() -> String {
    let variant = std::env::var("RENDERLAB_VD_SHADER")
        .ok()
        .map(|path| std::fs::read_to_string(path).unwrap());
    let mut source = format!(
        "const VD_R:u32={DISK_RADIUS}u;\n{}",
        variant
            .as_deref()
            .unwrap_or(include_str!("../shaders/virtual_shadow_disk.wgsl"))
    );
    for (name, value) in std::env::var("RENDERLAB_VD")
        .iter()
        .flat_map(|list| list.split(','))
        .filter_map(|item| item.split_once('='))
    {
        let declaration = format!("override {name}:");
        let Some(at) = source.find(&declaration) else {
            panic!("RENDERLAB_VD: no override named {name}");
        };
        let equals = at + source[at..].find('=').unwrap() + 1;
        let end = equals + source[equals..].find(';').unwrap();
        source.replace_range(equals..end, value);
    }
    source
}

fn corners(b: &crate::shadow_plan::Bounds) -> impl Iterator<Item = glam::DVec3> + '_ {
    (0..8).map(move |i| {
        glam::DVec3::new(
            f64::from(if i & 1 == 0 { b.min.x } else { b.max.x }),
            f64::from(if i & 2 == 0 { b.min.y } else { b.max.y }),
            f64::from(if i & 4 == 0 { b.min.z } else { b.max.z }),
        )
    })
}
fn buffer(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}
fn texture(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

/// N8e rolling refresh: the block of trace tiles (a x b) that shares one lookup per frame.
fn refresh_block(refresh: u32) -> [u32; 2] {
    match refresh {
        2 => [2, 1],
        4 => [2, 2],
        8 => [4, 2],
        16 => [4, 4],
        _ => [1, 1],
    }
}
/// The tile of the block a rolling frame looks up: blocks are visited in ordered-dither
/// (Bayer) order, so consecutive frames refresh tiles far apart.
fn refresh_tile(refresh: u32, frame: u32) -> [u32; 2] {
    const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let [a, b] = refresh_block(refresh);
    let mut cells: Vec<(u32, [u32; 2])> = (0..b)
        .flat_map(|y| (0..a).map(move |x| (BAYER[y as usize][x as usize], [x, y])))
        .collect();
    cells.sort_unstable();
    cells[(frame % refresh) as usize].1
}

/// Orthonormal sun basis (u, v, toward-light l), from the light direction alone so cached
/// pages survive camera motion.
fn basis(toward: glam::DVec3) -> [glam::DVec3; 3] {
    let l = toward.normalize();
    let up = if l.y.abs() < 0.999 {
        glam::DVec3::Y
    } else {
        glam::DVec3::X
    };
    let u = up.cross(l).normalize();
    let v = l.cross(u);
    [u, v, l]
}

/// Triangles a mesh's primitive IDs address: source triangles plus appended LOD triangles,
/// from the index buffer's layout (an optional exact-adjacency suffix doubles it, plus one
/// word).
fn addressed_triangles(mesh: &Mesh) -> u64 {
    let words = mesh.index.size() / 4;
    if words % 3 == 1 {
        (words - 1) / 6
    } else {
        words / 3
    }
}

impl VirtualState {
    pub fn new(
        device: &PipelineDevice,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::ShaderStages as S;
        use wgpu::TextureSampleType as T;
        use wgpu::TextureViewDimension as D;
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let uint = T::Uint;
        let float = T::Float { filterable: false };
        let raster_stages = S::COMPUTE | S::VERTEX_FRAGMENT;
        // Writable storage stays out of the vertex stage (no VERTEX_WRITABLE_STORAGE).
        let writes = S::COMPUTE | S::FRAGMENT;
        let reads = S::COMPUTE | S::VERTEX;
        let layouts = Layouts {
            mark: group(
                "N8 page requests",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(1, false, S::COMPUTE),
                    sampled(7, uint, D::D2),
                    sampled(8, uint, D::D2),
                    sampled(9, float, D::D2),
                    written(14),
                ],
            ),
            compact: group(
                "N8 page cache",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(1, false, S::COMPUTE),
                    storage(2, false, S::COMPUTE),
                    storage(29, false, S::COMPUTE),
                    storage(37, false, S::COMPUTE),
                ],
            ),
            cull: group(
                "N8c caster cut and cull",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(25, true, S::COMPUTE),
                    storage(26, true, S::COMPUTE),
                    storage(27, false, S::COMPUTE),
                    storage(29, false, S::COMPUTE),
                ],
            ),
            raster: group(
                "N8c page raster",
                &[
                    frame_uniform(raster_stages),
                    storage(25, true, reads),
                    storage(29, false, writes),
                    storage(31, false, writes),
                    storage(32, false, writes),
                    storage(33, true, reads),
                ],
            ),
            mip: group(
                "N8c max-mips",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(16, true, S::COMPUTE),
                    storage(29, false, S::COMPUTE),
                    storage(36, false, S::COMPUTE),
                ],
            ),
            data: group(
                "N8c caster cluster data",
                &[data_entry(8, S::COMPUTE | S::VERTEX)],
            ),
            pass: group(
                "N8 page raster level",
                &[layout_entry(
                    30,
                    raster_stages,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                )],
            ),
            lookup: group(
                "N8 visibility",
                &[
                    frame_uniform(S::COMPUTE),
                    sampled(7, uint, D::D2),
                    sampled(8, uint, D::D2),
                    sampled(9, float, D::D2),
                    sampled(10, float, D::D2),
                    written(15),
                    storage(16, true, S::COMPUTE),
                    sampled(18, uint, D::D2),
                    storage(19, true, S::COMPUTE),
                    storage(20, false, S::COMPUTE),
                    storage(24, true, S::COMPUTE),
                    storage(35, true, S::COMPUTE),
                ],
            ),
            horizon: group(
                "N8 ring horizon pass",
                &[
                    layout_entry(
                        22,
                        S::COMPUTE,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(16),
                        },
                    ),
                    storage(23, false, S::COMPUTE),
                ],
            ),
            records: group(
                "N8g walk records",
                &[wgpu::BindGroupLayoutEntry {
                    count: std::num::NonZeroU32::new(crate::TRACED_DRAWS),
                    ..storage(9, true, S::COMPUTE)
                }],
            ),
            resolve: group(
                "N8k texel planes",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(16, true, S::COMPUTE),
                    storage(19, true, S::COMPUTE),
                    storage(29, true, S::COMPUTE),
                    storage(41, false, S::COMPUTE),
                    storage(43, false, S::COMPUTE),
                ],
            ),
            map: group(
                "N8k map visibility",
                &[
                    frame_uniform(S::COMPUTE),
                    sampled(7, uint, D::D2),
                    sampled(8, uint, D::D2),
                    sampled(9, float, D::D2),
                    sampled(10, float, D::D2),
                    written(15),
                    storage(16, true, S::COMPUTE),
                    sampled(18, uint, D::D2),
                    storage(19, true, S::COMPUTE),
                    storage(35, true, S::COMPUTE),
                    storage(40, true, S::COMPUTE),
                    storage(42, true, S::COMPUTE),
                    storage(45, true, S::COMPUTE),
                    storage(47, false, S::COMPUTE),
                ],
            ),
            queue: group(
                "N8k hybrid queue dispatch",
                &[
                    storage(47, false, S::COMPUTE),
                    storage(48, false, S::COMPUTE),
                ],
            ),
            mip_dispatch: group(
                "N8k mip dispatch arguments",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(37, true, S::COMPUTE),
                    storage(46, false, S::COMPUTE),
                ],
            ),
            minmip: group(
                "N8k min-mips",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(16, true, S::COMPUTE),
                    storage(29, true, S::COMPUTE),
                    storage(44, false, S::COMPUTE),
                ],
            ),
            disk: group(
                "N-VSM1 disk visibility",
                &[
                    frame_uniform(S::COMPUTE),
                    sampled(7, uint, D::D2),
                    sampled(8, uint, D::D2),
                    sampled(9, float, D::D2),
                    sampled(10, float, D::D2),
                    written(15),
                    storage(16, true, S::COMPUTE),
                    sampled(18, uint, D::D2),
                    storage(19, true, S::COMPUTE),
                    storage(35, true, S::COMPUTE),
                    layout_entry(
                        50,
                        S::COMPUTE,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(DISK_FRAME_BYTES),
                        },
                    ),
                ],
            ),
        };
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
        let mark_layout = pipeline_layout("N8 page request ABI", &[Some(&layouts.mark)]);
        let compact_layout = pipeline_layout("N8 page cache ABI", &[Some(&layouts.compact)]);
        let cull_layout =
            pipeline_layout("N8c caster cull ABI", &[Some(&layouts.cull), Some(arrays)]);
        let clear_layout = pipeline_layout("N8c page clear ABI", &[Some(&layouts.raster)]);
        let mip_layout = pipeline_layout(
            "N8c max-mip ABI",
            &[Some(&layouts.mip), None, None, Some(&layouts.pass)],
        );
        let raster_layout = pipeline_layout(
            "N8c page raster ABI",
            &[
                Some(&layouts.raster),
                Some(arrays),
                Some(&layouts.data),
                Some(&layouts.pass),
            ],
        );
        let horizon_layout = pipeline_layout(
            "N8 ring horizon ABI",
            &[Some(&layouts.horizon), Some(arrays)],
        );
        let lookup_layout = pipeline_layout(
            "N8 visibility ABI",
            &[
                Some(&layouts.lookup),
                Some(arrays),
                Some(&layouts.records),
                Some(lighting),
            ],
        );
        let resolve_layout = pipeline_layout(
            "N8k texel plane ABI",
            &[Some(&layouts.resolve), Some(arrays)],
        );
        let map_layout = pipeline_layout(
            "N8k map visibility ABI",
            &[Some(&layouts.map), None, None, Some(lighting)],
        );
        let map_module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("N8k map-only sun visibility"),
                source: wgpu::ShaderSource::Wgsl(map_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let map_compute = |entry, layout: &wgpu::PipelineLayout, constants: &[(&str, f64)]| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &map_module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let resolve = map_compute("vmap_resolve", &resolve_layout, &[]);
        let mip_dispatch_layout =
            pipeline_layout("N8k mip dispatch ABI", &[Some(&layouts.mip_dispatch)]);
        let minmip_layout = pipeline_layout(
            "N8k min-mip ABI",
            &[Some(&layouts.minmip), None, None, Some(&layouts.pass)],
        );
        let mip_dispatch = map_compute("vmap_mip_dispatch", &mip_dispatch_layout, &[]);
        let minmip = map_compute("vmap_minmip", &minmip_layout, &[]);
        let queue_layout = pipeline_layout("N8k hybrid queue ABI", &[Some(&layouts.queue)]);
        let queue_args = map_compute("vmap_queue_args", &queue_layout, &[]);
        let map = map_compute("vmap_lookup", &map_layout, &[]);
        let disk_layout = pipeline_layout(
            "N-VSM1 disk visibility ABI",
            &[Some(&layouts.disk), None, None, Some(lighting)],
        );
        let disk_module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("N-VSM1 disk sun visibility"),
                source: wgpu::ShaderSource::Wgsl(disk_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let disk_compute = |entry, layout: &wgpu::PipelineLayout| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &disk_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let disk_mark = disk_compute("vd_mark", &mark_layout);
        let disk = disk_compute("vd_lookup", &disk_layout);
        let record_module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("N8g walk records"),
                source: wgpu::ShaderSource::Wgsl(records_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let record = ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("vsm_record"),
                layout: Some(&horizon_layout),
                module: &record_module,
                entry_point: Some("vsm_record"),
                compilation_options: Default::default(),
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        );
        let module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("N8 virtual sun shadows"),
                source: wgpu::ShaderSource::Wgsl(shader_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let compute = |entry, layout: &wgpu::PipelineLayout, constants: &[(&str, f64)]| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let hardware = |label, peel: f64| {
            let constants = [("VSM_PEEL_PASS", peel)];
            ledger.own(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&raster_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_vsm_cluster"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_vsm_cluster"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants,
                            ..Default::default()
                        },
                        targets: &[],
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    // The 64-bit max decides; the attachment only sizes the raster area.
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: AREA_FORMAT,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Always),
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
        let pipelines = Pipelines {
            mark: compute("vsm_mark", &mark_layout, &[]),
            compact: compute("vsm_compact", &compact_layout, &[]),
            clear: compute("vsm_clear", &clear_layout, &[]),
            cull: compute("vsm_cull", &cull_layout, &[]),
            finish: compute("vsm_finish", &cull_layout, &[]),
            mip: compute("vsm_mip", &mip_layout, &[]),
            software: [0.0, 1.0]
                .map(|peel| compute("vsm_raster", &raster_layout, &[("VSM_PEEL_PASS", peel)])),
            hardware: [
                hardware("N8c hardware page casters", 0.0),
                hardware("N8c hardware page peel", 1.0),
            ],
            lookup: compute("vsm_lookup", &lookup_layout, &[]),
            horizon: compute("vsm_horizon", &horizon_layout, &[]),
            record,
            debug: None,
            order: None,
            resolve,
            map,
            mip_dispatch,
            minmip,
            queue_args,
            queued: None,
            disk_mark,
            disk,
        };
        Self {
            settings: VirtualShadowSettings::default(),
            stats: VirtualShadowStats::default(),
            layouts,
            lookup_layout,
            pipelines,
            resources: None,
            retired: vec![],
            cache: Cache::default(),
            casters: None,
            retired_casters: vec![],
            levels: 0,
            key: vec![],
            page_work: true,
            history_frames: 0,
            rolling: false,
            schedule: 0,
            casters_only: false,
            horizons: None,
            debug: false,
            constants: vec![],
            built: vec![],
            module,
            lookup_defaults: schedule_defaults(&shader_source()),
            map_module,
            map_layout,
            queue_layout,
            disk_module,
            disk_layout,
            hybrid_module: None,
            surface: None,
            s4: None,
        }
    }

    /// A lookup override's compiled value (`built`), or `default`, the WGSL declaration's.
    fn built_constant(&self, name: &str, default: f64) -> f64 {
        self.built
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(default, |&(_, value)| value)
    }

    /// N8m: the lookup's persistent workgroups (WGSL `VSM_PERSIST`; 0 launches one per tile),
    /// and whether they take tiles heaviest first (`VSM_ORDER`).
    fn persistent_groups(&self) -> (u32, bool) {
        let (groups, order) = self.lookup_defaults;
        let groups = self.built_constant("VSM_PERSIST", f64::from(groups)) as u32;
        let order = self.built_constant("VSM_ORDER", f64::from(u8::from(order))) != 0.0;
        (groups, groups != 0 && order)
    }

    /// The lookup targets' size (tests: the scratch's trace offset).
    #[cfg(test)]
    pub fn size(&self) -> [u32; 2] {
        self.resources.as_ref().map_or([0, 0], |r| r.size)
    }

    /// The last debug frame's scratch buffer (`debug` must have been set for it): the
    /// `scratch_trace_offset` bytes, then the trace.
    #[cfg(test)]
    pub fn debug_buffer(&self) -> Option<&wgpu::Buffer> {
        self.resources
            .as_ref()
            .filter(|r| r.debug_sized)
            .map(|r| &*r.debug)
    }

    pub fn set_settings(
        &mut self,
        settings: VirtualShadowSettings,
        last: u64,
    ) -> Result<(), Error> {
        settings.validate()?;
        if settings != self.settings {
            self.settings = settings;
            self.reset(last);
        }
        Ok(())
    }

    fn reset(&mut self, last: u64) {
        // Pages, hysteresis anchors and kept counts belong to the same history. Geometry
        // records and pipelines are independent of it and can stay warm.
        self.cache = Cache::default();
        self.key.clear();
        self.history_frames = 0;
        self.rolling = false;
        self.schedule = 0;
        self.casters_only = false;
        if let Some(surface) = &mut self.surface {
            surface.restart();
        }
        if let Some(old) = self.resources.take() {
            self.retired.push((last, old));
        }
    }

    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
        self.retired_casters.retain(|(last, _)| *last > completed);
    }

    fn ensure_resources(
        &mut self,
        device: &PipelineDevice,
        ledger: &Ledger,
        levels: usize,
        size: [u32; 2],
        last: u64,
    ) -> Result<bool, Error> {
        let pages = self.settings.pages;
        if self.resources.as_ref().is_some_and(|r| {
            r.levels >= levels && r.pages == pages && r.size == size && r.debug_sized == self.debug
        }) {
            return Ok(false);
        }
        // Grow-only in levels, so the ladder can shift without reallocating.
        let levels = levels.max(self.resources.as_ref().map_or(0, |r| r.levels));
        let side = pages * PAGE;
        let map_bytes = u64::from(side) * u64::from(side) * levels as u64 * 8;
        if map_bytes > device.limits().max_storage_buffer_binding_size {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual clipmap layer exceeds the storage binding limit; use fewer pages",
            ));
        }
        if let Some(old) = self.resources.take() {
            self.retired.push((last, old));
        }
        // Fresh storage is all stale without touching the cache: its page epochs are zero,
        // and level epochs start at one.
        let area = ledger.own(
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("N8c hardware raster area"),
                size: wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: AREA_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let area_view = ledger.own(
            area.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let pixels = |label, [width, height]: [u32; 2]| {
            let texture = ledger.own(
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R32Uint,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = ledger.own(
                texture.create_view(&Default::default()),
                ResourceKind::TextureView,
            );
            (texture, view)
        };
        let (level, level_view) = pixels("N8 receiver levels", size);
        let slots = levels as u64 * u64::from(pages * pages);
        let debug_bytes = scratch_trace_offset(size)
            + if self.debug {
                u64::from(size[0]) * u64::from(size[1]) * 32
            } else {
                16
            };
        let buffer = |label, size: u64, usage| {
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
        let mip_bytes = (u64::from(side) * u64::from(side) - 1) / 3 * 4 * levels as u64;
        // N8k planes are as large as a layer, and their errors half of one.
        let (plane_bytes, error_bytes) = if self.settings.lookup.map() {
            (map_bytes, map_bytes / 2)
        } else {
            (16, 16)
        };
        let minmip_bytes = if self.settings.lookup.map() {
            mip_bytes.max(4)
        } else {
            16
        };
        // N8n/N8o: the surface cache's queue keeps its sort counters and chunk cursor after
        // the entries (surface_cache::QUEUE_TAIL).
        let queue_bytes = if self.settings.lookup.queues() {
            16 + 8 * u64::from(size[0]) * u64::from(size[1])
                + surface_cache::QUEUE_TAIL
                    * u64::from(self.settings.lookup == VirtualLookup::Surface)
        } else {
            32
        };
        let bytes = 2 * map_bytes
            + plane_bytes
            + error_bytes
            + minmip_bytes
            + queue_bytes
            + mip_bytes
            + u64::from(side) * u64::from(side) * 2
            + u64::from(size[0]) * u64::from(size[1]) * 4
            + slots * 12
            + 16;
        self.resources = Some(Resources {
            levels,
            pages,
            size,
            top: buffer("N8c clipmap top layer", map_bytes, B::STORAGE),
            peel: buffer("N8c clipmap peel layer", map_bytes, B::STORAGE),
            mips: buffer("N8c clipmap max-mips", mip_bytes.max(4), B::STORAGE),
            planes: buffer("N8k clipmap texel planes", plane_bytes, B::STORAGE),
            errors: buffer("N8k clipmap planarity errors", error_bytes, B::STORAGE),
            minmips: buffer("N8k clipmap min-mips", minmip_bytes, B::STORAGE),
            mip_args: buffer(
                "N8k mip dispatch arguments",
                16 * 12,
                B::STORAGE | B::INDIRECT,
            ),
            queue: buffer("N8k hybrid queue", queue_bytes, B::STORAGE | B::COPY_SRC),
            queue_dispatch: buffer("N8k hybrid queue dispatch", 16, B::STORAGE | B::INDIRECT),
            disk_frame: buffer("N-VSM1 disk frame", DISK_FRAME_BYTES, B::UNIFORM),
            _area: area,
            area: area_view,
            _level: level,
            level_view,
            debug: buffer("N8 decision trace", debug_bytes, B::STORAGE | B::COPY_SRC),
            debug_sized: self.debug,
            frame: buffer("N8 frame", FRAME_BYTES as u64, B::UNIFORM),
            requests: buffer("N8 page requests", slots * 4, B::STORAGE),
            state: buffer("N8 page epochs", slots * 4, B::STORAGE),
            render: buffer("N8c pages rasterized this frame", slots * 4, B::STORAGE),
            cull_args: buffer("N8f caster cull dispatch", 16, B::STORAGE | B::INDIRECT),
            pass: buffer(
                "N8 level and mip uniforms",
                PASS_ENTRIES as u64 * PASS_STRIDE,
                B::UNIFORM,
            ),
            _allocation: ledger.allocate(AllocationKind::Shadow, bytes),
            groups: None,
        });
        self.stats.bytes = bytes;
        Ok(true)
    }
}
/// The hardware raster's area attachment.
const AREA_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

impl Renderer {
    /// Tuning for `SoftShadows::Virtual`; a change drops cached pages and count history.
    /// Setting the same value keeps history; use `reset_virtual_shadow_cache` to restart it.
    pub fn set_virtual_shadow_settings(
        &mut self,
        settings: VirtualShadowSettings,
    ) -> Result<(), Error> {
        settings.validate()?;
        if let Some(state) = &mut self.virtual_shadows {
            state.set_settings(settings, self.submissions)?;
        }
        self.virtual_settings = settings;
        Ok(())
    }

    /// P-NSQ: this frame's rotation into each pixel's direction sequence: TR1's (RT
    /// Interactive's `temporal_reconstruction::rotation`) on reconstructed frames of the Exact
    /// and Surface lookups, else 0. Map and Hybrid keep N8k's fixed directions.
    fn virtual_rotation(&self, s: VirtualShadowSettings) -> u32 {
        let exact = matches!(s.lookup, VirtualLookup::Exact | VirtualLookup::Surface);
        self.reconstruction_frame
            .filter(|_| exact && self.reconstruction.rotate_directions)
            .map_or(0, super::primary_ray_gpu::temporal_reconstruction::rotation)
    }

    /// P-NSQ: after TR1 chose this frame's shadow budget (`prepare_reconstruction`), the
    /// Exact and Surface lookups evaluate it in batches of eight, like RT Interactive's rays.
    /// Frames without TR1, Map and Hybrid keep one batch. P-RELIT: they also take TR1's
    /// restart zones, where a receiver evaluates the restart count instead.
    pub(super) fn prepare_virtual_sampling(&mut self) {
        let Some((rays, restart_rays, zones)) = self
            .reconstruction_state
            .as_ref()
            .map(|r| (r.plan.shadow_rays, r.plan.restart_rays, r.zones))
            .filter(|_| self.reconstruction_frame.is_some())
        else {
            return;
        };
        let Some(state) = self.virtual_shadows.as_ref() else {
            return;
        };
        if !matches!(
            state.settings.lookup,
            VirtualLookup::Exact | VirtualLookup::Surface
        ) {
            return;
        }
        let Some(r) = state.resources.as_ref() else {
            return;
        };
        let batches = (rays / 8).max(1);
        self.queue
            .write_buffer(&r.frame, SAMPLING_OFFSET, &batches.to_ne_bytes());
        self.queue.write_buffer(&r.frame, ZONES_OFFSET, &zones);
        self.uploads.shadows += 4 + ZONES_BYTES as u64;
        // P-S4C: the occluder gather serves the frames where it beats the march. It evaluates
        // the frame's batches for every pixel, so it sits out frames whose restart zones
        // evaluate more (non-default budgets only: its frames have eight batches or more).
        let pixels = u64::from(r.size[0]) * u64::from(r.size[1]);
        let disk = self.shadows.settings.directional_disk_angle;
        let state = self.virtual_shadows.as_mut().unwrap();
        if let Some(s4) = state.s4.as_mut() {
            s4.frame = state.stats.occluder_gather_ready
                && restart_rays == 0
                && sunvis_s4::chosen(batches, pixels, disk);
            state.stats.occluder_gather_frame = s4.frame;
        }
    }

    /// Restart virtual shadows from the current scene, camera, sun and settings. The next
    /// frame rebuilds requested pages and looks up every pixel, discarding rolling counts
    /// and map hysteresis from earlier frames. In-flight resources retire normally.
    pub fn reset_virtual_shadow_cache(&mut self) {
        if let Some(state) = &mut self.virtual_shadows {
            state.reset(self.submissions);
        }
    }
    pub fn virtual_shadow_stats(&self) -> VirtualShadowStats {
        self.virtual_shadows
            .as_ref()
            .map_or_else(Default::default, |s| s.stats)
    }

    /// The draw list's caster clusters, rebuilt when its meshes or placement counts change.
    fn ensure_virtual_casters(&mut self, draws: &[PreparedDraw<'_>]) -> Result<(), Error> {
        let key: Vec<(usize, usize)> = draws
            .iter()
            .map(|d| (Arc::as_ptr(&d.mesh.index) as usize, d.instances.len()))
            .collect();
        let state = self.virtual_shadows.as_ref().unwrap();
        if state.casters.as_ref().is_some_and(|c| c.key == key) {
            return Ok(());
        }
        if draws.iter().any(|d| d.mesh.clusters.is_none()) {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual casts from visibility-buffer clusters; upload meshes on a device with VISIBILITY_FEATURES, VISIBILITY_RASTER_FEATURES and visibility limits",
            ));
        }
        let instances = ClusterInstances::new(self, draws)?;
        let mut primitive_bits = 1u32;
        for d in draws {
            let triangles = addressed_triangles(d.mesh).clamp(1, u64::from(u32::MAX)) as u32;
            primitive_bits = primitive_bits.max(32 - triangles.leading_zeros());
        }
        let work_bytes =
            MAX_LEVELS as u64 * (ARGS_BYTES + 2 * u64::from(instances.count.max(1)) * 4);
        let work = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("N8c page raster arguments and cluster lists"),
                size: work_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let absent = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("N8c absent cluster data"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let state = self.virtual_shadows.as_ref().unwrap();
        let data = self.ledger.own(
            cluster_data_group(self, &state.layouts.data, draws, &absent),
            ResourceKind::BindGroup,
        );
        let allocation = self.ledger.allocate(AllocationKind::Shadow, work_bytes);
        let state = self.virtual_shadows.as_mut().unwrap();
        let generation = state.casters.as_ref().map_or(0, |c| c.generation + 1);
        if let Some(old) = state.casters.take() {
            state.retired_casters.push((self.submissions, old));
        }
        state.casters = Some(Casters {
            key,
            generation,
            instances,
            work,
            data,
            _absent: absent,
            primitive_bits,
            _allocation: allocation,
        });
        Ok(())
    }

    /// Plans this frame's levels, windows and cache invalidation, uploads them, and builds
    /// bind groups over the receiver targets. Call after the primaries are prepared.
    pub(super) fn prepare_virtual(
        &mut self,
        draws: &[PreparedDraw<'_>],
        camera: &Camera,
        size: wgpu::Extent3d,
    ) -> Result<(), Error> {
        if !self.device.features().contains(VIRTUAL_FEATURES) {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual requires VIRTUAL_FEATURES (64-bit storage max atomics)",
            ));
        }
        let Some(Light::Directional { travel, .. }) = self.lighting.source_lights.first().copied()
        else {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual shadows lights[0], which must be directional",
            ));
        };
        let primary = self.primary.as_ref().unwrap();
        if self.virtual_shadows.is_none() {
            let mut state = VirtualState::new(
                &self.device,
                &self.ledger,
                primary.array_layout(),
                &self.lighting.bindings,
            );
            state.settings = self.virtual_settings;
            self.pipelines.created += 10;
            self.virtual_shadows = Some(state);
        }
        self.ensure_virtual_casters(draws)?;
        let rotation = self.virtual_rotation(self.virtual_settings);
        // N-VSM1: the disk lookup's receiver jitter follows TR1's frames (`VdFrame::size.z`).
        let disk_jitter = self
            .reconstruction_frame
            .map_or(0.0, |frame| (frame % 4096 + 1) as f32);
        let state = self.virtual_shadows.as_mut().unwrap();
        let s = state.settings;
        // N8n: the surface cache exists only while it is the lookup (wgpu keeps the table alive
        // for frames in flight).
        if s.lookup != VirtualLookup::Surface {
            state.surface = None;
        } else if state.surface.is_none() {
            state.surface = Some(SurfaceCache::new(
                &self.device,
                &self.ledger,
                &self.lighting.bindings,
            ));
        }
        // P-S4B: the gather's geometry follows the draws; whether this frame runs it waits for
        // TR1's budget (prepare_virtual_sampling).
        if s.lookup != VirtualLookup::Surface || !s.occluder_gather {
            state.s4 = None;
        } else {
            let s4 = state.s4.get_or_insert_with(|| {
                sunvis_s4::S4::new(
                    &self.device,
                    &self.ledger,
                    &state.layouts.lookup,
                    &self.lighting.bindings,
                )
            });
            s4.frame = false;
        }
        if let Some(mut s4) = self.virtual_shadows.as_mut().unwrap().s4.take() {
            let ready = s4.prepare(
                &self.device,
                &self.queue,
                &self.ledger,
                draws,
                |mesh| self.exact_indirect_geometry(mesh),
                -travel,
            );
            self.virtual_shadows.as_mut().unwrap().s4 = Some(s4);
            self.virtual_shadows
                .as_mut()
                .unwrap()
                .stats
                .occluder_gather_ready = ready;
        }
        let state = self.virtual_shadows.as_mut().unwrap();
        // N8e: the map keeps the sun direction it was rasterized for while the sun stays
        // within `sun_hysteresis` of it; the march and page requests then widen their cone by
        // that angle (the lookup's directions are always the current sun's).
        let kept = state.cache.toward.zip(state.cache.light).filter(|(m, _)| {
            s.sun_hysteresis > 0.0
                && m.angle_between(-travel.as_dvec3()) <= f64::from(s.sun_hysteresis)
        });
        let disk = f64::from(self.shadows.settings.directional_disk_angle);
        let slope = (disk + f64::from(s.sun_hysteresis)).tan() * 1.001;
        let eye = camera.view().inverse().w_axis.truncate().as_dvec3();
        let (toward, light_bits) = kept.unwrap_or((
            -travel.as_dvec3(),
            (-travel).normalize().to_array().map(f32::to_bits),
        ));
        state.cache.toward = Some(toward);
        let [u, v, l] = basis(toward);

        // Placements in traced order: draw order, then instance slot.
        let mut models = vec![];
        let mut bounds = vec![];
        for draw in draws {
            for &model in &draw.instances.state.models {
                models.push(model.to_cols_array().map(f32::to_bits));
                bounds.push(shadow_gpu::world_bounds(draw.mesh.bounds, model)?);
            }
        }
        let placements = models.len();
        let poses = crate::renderer::skinning::placement_poses(draws);
        // Light-space xy rectangle of a box, relative to the eye.
        let rect = |b: &crate::shadow_plan::Bounds| {
            let mut lo = glam::DVec2::INFINITY;
            let mut hi = glam::DVec2::NEG_INFINITY;
            for c in corners(b) {
                let p = glam::DVec2::new((c - eye).dot(u), (c - eye).dot(v));
                lo = lo.min(p);
                hi = hi.max(p);
            }
            [lo, hi]
        };
        let (mut zlo, mut zhi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut near, mut far) = (f64::INFINITY, 0.0f64);
        for b in &bounds {
            for c in corners(b) {
                let z = c.dot(l);
                zlo = zlo.min(z);
                zhi = zhi.max(z);
                far = far.max((c - eye).length());
            }
            let nearest = eye.clamp(b.min.as_dvec3(), b.max.as_dvec3());
            near = near.min((nearest - eye).length());
        }
        if bounds.is_empty() {
            (zlo, zhi, near, far) = (0.0, 1.0, 1.0, 1.0);
        }
        let mut invalidated = [false; MAX_LEVELS];
        let cache = &mut state.cache;
        // N8n: a sun leaving the map's hysteresis retires the surface cache's cells too, as the
        // rebased map looks up every pixel again (measured: lagged cells after a sun jump).
        let sun_rebased = cache.light != Some(light_bits);
        if cache.light != Some(light_bits) {
            cache.light = Some(light_bits);
            invalidated = [true; MAX_LEVELS];
        }
        // The depth encoding spans a padded, grow-only height range.
        let heights = match cache.heights {
            Some([a, b]) if a <= zlo && b >= zhi && !invalidated[0] => [a, b],
            _ => {
                let pad = 0.01 * (zhi - zlo) + 0.001;
                invalidated = [true; MAX_LEVELS];
                [zlo - pad, zhi + pad]
            }
        };
        // N8o: the surface cache's cells hold walks against these heights and levels; a new
        // height encoding or level ladder retires them (a moved window only re-rasterizes the
        // same texels).
        let mut ladder_changed = cache.heights != Some(heights);
        cache.heights = Some(heights);
        let footprint = 2.0 * (f64::from(camera.fov_y()) * 0.5).tan() / f64::from(size.height);
        let level_of = |d: f64| {
            ((d.max(1e-6) * footprint / f64::from(s.finest_texel)).log2()
                - f64::from(s.texels_per_pixel_log2))
            .floor() as i32
        };
        let finest = level_of(near.max(f64::from(camera.near())));
        let levels = ((level_of(far) - finest + 2).clamp(1, MAX_LEVELS as i32)) as usize;
        if cache.finest != Some(finest) || cache.levels != levels {
            cache.finest = Some(finest);
            cache.levels = levels;
            invalidated = [true; MAX_LEVELS];
            ladder_changed = true;
        }
        // Moved casters invalidate the pages under their old and new light-space bounds.
        let mut rects = vec![];
        // N8f: their old and new bounds' pixel rectangles too (whole image when a corner is
        // not in front of the camera): a static camera's pixels outside them keep their receivers.
        let mut screens = vec![];
        let clip = camera.matrix(size.width, size.height).as_dmat4();
        let screen = |b: &crate::shadow_plan::Bounds| {
            let (w, h) = (f64::from(size.width), f64::from(size.height));
            let mut lo = glam::DVec2::INFINITY;
            let mut hi = glam::DVec2::NEG_INFINITY;
            for c in corners(b) {
                let p = clip * c.extend(1.0);
                if p.w <= 1e-6 {
                    return [glam::DVec2::ZERO, glam::DVec2::new(w, h)];
                }
                let q = glam::DVec2::new((p.x / p.w * 0.5 + 0.5) * w, (0.5 - p.y / p.w * 0.5) * h);
                lo = lo.min(q);
                hi = hi.max(q);
            }
            [lo - 2.0, hi + 2.0]
        };
        // N8n: placements the rectangles do not describe retire the surface cache's cells.
        let mut surface_flush = false;
        if cache.models.len() != placements {
            invalidated = [true; MAX_LEVELS];
            surface_flush = true;
        } else {
            // SK1: deformation invalidates caster pages even at a fixed model matrix; P-SHOW:
            // only a new pose deforms (per placement, in placement order, collected once:
            // P-NRT's O(placements^2) find).
            for (p, model) in models.iter().enumerate() {
                if *model != cache.models[p]
                    || crate::renderer::skinning::deformed(&cache.poses, &poses, p)
                {
                    let [a0, a1] = rect(&cache.bounds[p]);
                    let [b0, b1] = rect(&bounds[p]);
                    rects.push([a0.min(b0), a1.max(b1)]);
                    let [c0, c1] = screen(&cache.bounds[p]);
                    let [d0, d1] = screen(&bounds[p]);
                    screens.push([c0.min(d0), c1.max(d1)]);
                }
            }
            if rects.len() > MAX_RECTS {
                invalidated = [true; MAX_LEVELS];
                rects.clear();
                surface_flush = true;
            }
        }
        cache.models = models;
        cache.bounds = bounds;
        cache.poses = poses;
        let pages = i64::from(s.pages);
        let eye_light = glam::DVec2::new(eye.dot(u), eye.dot(v));
        let mut level_bytes = [0u8; MAX_LEVELS * 32];
        for i in 0..levels {
            let texel = f64::from(s.finest_texel) * 2f64.powi(finest + i as i32);
            let page = texel * f64::from(PAGE);
            let centre = [eye_light.x, eye_light.y].map(|x| (x / page).floor() as i64);
            let keep = cache.origins[i].filter(|o| {
                !invalidated[i] && (0..2).all(|a| (centre[a] - (o[a] + pages / 2)).abs() <= 1)
            });
            let origin = keep.unwrap_or([centre[0] - pages / 2, centre[1] - pages / 2]);
            if keep.is_none() {
                invalidated[i] = true;
                cache.origins[i] = Some(origin);
                cache.epochs[i] = cache.epochs[i].wrapping_add(1).max(1);
            }
            let lo = glam::DVec2::new(origin[0] as f64 * page, origin[1] as f64 * page) - eye_light;
            // The caster cut's error bound at this level (negative: original geometry).
            let caster_error = if s.caster_error_texels < 0.0 {
                -1.0
            } else {
                f64::from(s.caster_error_texels) * texel
            };
            let entry = &mut level_bytes[i * 32..i * 32 + 32];
            for (k, value) in [lo.x as f32, lo.y as f32, texel as f32, caster_error as f32]
                .iter()
                .enumerate()
            {
                entry[k * 4..k * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            for (k, value) in [cache.epochs[i], (finest + i as i32) as u32, 0, 0]
                .iter()
                .enumerate()
            {
                entry[16 + k * 4..20 + k * 4].copy_from_slice(&value.to_ne_bytes());
            }
        }
        // Absolute level of index 0, which vsm_select reads from levels[0].info.y.
        level_bytes[20..24].copy_from_slice(&finest.to_ne_bytes());
        let casters = state.casters.as_ref().unwrap();
        let primitive_bits = casters.primitive_bits;
        let cluster_count = casters.instances.count;
        if 32 - (placements as u32 + 1).leading_zeros() + primitive_bits > 32 {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual packs placement and primitive into 32 bits; the scene exceeds that",
            ));
        }
        let mut frame = Vec::with_capacity(FRAME_BYTES);
        let mut f32s = |values: [f64; 4]| {
            for value in values {
                frame.extend_from_slice(&(value as f32).to_ne_bytes());
            }
        };
        f32s([u.x, u.y, u.z, 0.0]);
        f32s([v.x, v.y, v.z, 0.0]);
        f32s([l.x, l.y, l.z, 0.0]);
        f32s([eye.x, eye.y, eye.z, footprint]);
        let range = heights[1] - heights[0];
        f32s([heights[0], 1.0 / range, range, heights[1]]);
        f32s([
            f64::from(s.near),
            f64::from(s.finest_texel),
            slope,
            f64::from(s.texels_per_pixel_log2),
        ]);
        for value in [
            levels as u32,
            s.pages,
            cluster_count,
            rects.len() as u32,
            primitive_bits,
            s.march_samples,
            s.bias_ulps,
            // N8k: bit 8, the hybrid's queue (virtual_shadow_lookup.wgsl vmap_hybrid).
            u32::from(s.ring)
                | u32::from(s.peel_own) << 1
                | s.first_block << 2
                | u32::from(s.lookup == VirtualLookup::Hybrid) << 8,
        ] {
            frame.extend_from_slice(&value.to_ne_bytes());
        }
        frame.extend_from_slice(&level_bytes);
        for list in [&rects, &screens] {
            for i in 0..MAX_RECTS {
                let [lo, hi] = list.get(i).copied().unwrap_or([glam::DVec2::ZERO; 2]);
                for value in [lo.x, lo.y, hi.x, hi.y] {
                    frame.extend_from_slice(&(value as f32).to_ne_bytes());
                }
            }
        }
        // Ring horizons: (re)built when the draw list's geometry changes.
        // Every addressed triangle, appended LOD triangles included (LOD receivers).
        // N8k, N-VSM1: the map and disk lookups read neither, so they keep none.
        let map = !state.settings.lookup.walks();
        let coverage = state.settings.walk_records;
        let horizon_key: Vec<(usize, u32, u32, u64)> = draws
            .iter()
            .map(|d| {
                let addressed = if map {
                    0
                } else {
                    addressed_triangles(d.mesh) as u32
                };
                let recorded = match coverage {
                    _ if map => 0,
                    WalkRecords::Off => 0,
                    WalkRecords::Source => d.mesh.index_count / 3,
                    WalkRecords::All => addressed,
                };
                (
                    Arc::as_ptr(&d.mesh.index) as usize,
                    addressed,
                    recorded,
                    d.mesh.skin.as_ref().map_or(0, |s| s.revision),
                )
            })
            .collect();
        if state.horizons.as_ref().is_none_or(|h| {
            h.key.len() != horizon_key.len()
                || h.key
                    .iter()
                    .zip(&horizon_key)
                    .any(|(a, b)| (a.0, a.1, a.2) != (b.0, b.1, b.2))
        }) {
            let mut offsets = vec![];
            let mut words = 0u32;
            for &(_, triangles, _, _) in &horizon_key {
                offsets.push(words);
                words += triangles;
            }
            let store = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("N8 ring horizons"),
                    size: u64::from(words.max(1)) * 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let params = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("N8 ring horizon passes"),
                    size: crate::TRACED_DRAWS as u64 * HORIZON_STRIDE,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let mut bytes = vec![0u8; crate::TRACED_DRAWS as usize * HORIZON_STRIDE as usize];
            for (d, (&(_, triangles, _, _), &offset)) in
                horizon_key.iter().zip(&offsets).enumerate()
            {
                let at = d * HORIZON_STRIDE as usize;
                for (k, value) in [d as u32, triangles, offset, 0].iter().enumerate() {
                    bytes[at + k * 4..at + k * 4 + 4].copy_from_slice(&value.to_ne_bytes());
                }
            }
            self.queue.write_buffer(&params, 0, &bytes);
            let group = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("N8 ring horizon pass"),
                    layout: &state.layouts.horizon,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 22,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &params,
                                offset: 0,
                                size: wgpu::BufferSize::new(16),
                            }),
                        },
                        buffer(23, &store),
                    ],
                }),
                ResourceKind::BindGroup,
            );
            if words > 1 << 25 {
                return Err(Error::InvalidInput(
                    "SoftShadows::Virtual addresses at most 2^25 triangles per draw list",
                ));
            }
            // N8g walk records, N8h layout: every batch's recorded triangles in one numbering,
            // in segments of `SEGMENT` records, each one binding of the lookup's array. Batches
            // past the array (or a device whose binding limit is below a segment) read the
            // chains instead.
            let limits = self.device.limits();
            let usable = SEGMENT_BYTES
                <= limits
                    .max_storage_buffer_binding_size
                    .min(limits.max_buffer_size);
            let capacity = if usable {
                u64::from(crate::TRACED_DRAWS) * u64::from(SEGMENT)
            } else {
                0
            };
            let mut records = vec![];
            let mut total = 0u64;
            for &(_, _, recorded, _) in &horizon_key {
                if total + u64::from(recorded) <= capacity {
                    records.push([total as u32, recorded]);
                    total += u64::from(recorded);
                } else {
                    records.push([0, 0]);
                }
            }
            // Build pieces: one batch within one segment.
            let mut pieces = vec![];
            for (d, &[first, recorded]) in records.iter().enumerate() {
                let mut done = 0;
                while done < recorded {
                    let at = first + done;
                    let count = (recorded - done).min(SEGMENT - at % SEGMENT);
                    pieces.push((at / SEGMENT, [d as u32, count, done, at % SEGMENT]));
                    done += count;
                }
            }
            let record_params = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("N8g walk record passes"),
                    size: MAX_PIECES * HORIZON_STRIDE,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let mut bytes = vec![0u8; pieces.len().max(1) * HORIZON_STRIDE as usize];
            for (i, (_, piece)) in pieces.iter().enumerate() {
                let at = i * HORIZON_STRIDE as usize;
                for (k, value) in piece.iter().enumerate() {
                    bytes[at + k * 4..at + k * 4 + 4].copy_from_slice(&value.to_ne_bytes());
                }
            }
            self.queue.write_buffer(&record_params, 0, &bytes);
            let segments: Vec<_> = (0..total.div_ceil(u64::from(SEGMENT)))
                .map(|k| {
                    let count = (total - k * u64::from(SEGMENT)).min(u64::from(SEGMENT));
                    self.ledger.own(
                        self.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("N8g walk records"),
                            size: count * RECORD_BYTES,
                            usage: wgpu::BufferUsages::STORAGE,
                            mapped_at_creation: false,
                        }),
                        ResourceKind::Buffer,
                    )
                })
                .collect();
            let pieces: Vec<_> = pieces
                .iter()
                .map(|&(segment, [batch, count, _, _])| {
                    let build = self.ledger.own(
                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("N8g walk record pass"),
                            layout: &state.layouts.horizon,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 22,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &record_params,
                                        offset: 0,
                                        size: wgpu::BufferSize::new(16),
                                    }),
                                },
                                buffer(23, &segments[segment as usize]),
                            ],
                        }),
                        ResourceKind::BindGroup,
                    );
                    (count, batch, build)
                })
                .collect();
            let absent = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("N8g absent walk records"),
                    size: RECORD_BYTES,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let bindings: Vec<_> = segments
                .iter()
                .map(|b| b.as_entire_buffer_binding())
                .chain(std::iter::repeat(absent.as_entire_buffer_binding()))
                .take(crate::TRACED_DRAWS as usize)
                .collect();
            let record_array = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("N8g walk records"),
                    layout: &state.layouts.records,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::BufferArray(&bindings),
                    }],
                }),
                ResourceKind::BindGroup,
            );
            state.horizons = Some(Horizons {
                key: horizon_key,
                offsets,
                _allocation: self.ledger.allocate(
                    AllocationKind::Shadow,
                    u64::from(words) * 4 + total * RECORD_BYTES,
                ),
                buffer: store,
                params,
                group,
                records,
                segments,
                pieces,
                record_params,
                record_array,
                _absent: absent,
                dispatch: true,
                dirty: vec![true; draws.len()],
            });
        } else if let Some(h) = &mut state.horizons {
            h.dirty = h
                .key
                .iter()
                .zip(&horizon_key)
                .map(|(a, b)| a.3 != b.3)
                .collect();
            h.dispatch = h.dirty.iter().any(|&dirty| dirty);
            h.key = horizon_key;
        }
        // Each batch's first horizon word, then (N8h) its first record and recorded triangles.
        let h = state.horizons.as_ref().unwrap();
        for b in 0..crate::TRACED_DRAWS as usize {
            let offset = h.offsets.get(b).copied().unwrap_or(0);
            frame.extend_from_slice(&offset.to_ne_bytes());
        }
        for b in 0..crate::TRACED_DRAWS as usize {
            for value in h.records.get(b).copied().unwrap_or([0, 0]) {
                frame.extend_from_slice(&value.to_ne_bytes());
            }
        }
        state.levels = levels;
        state.stats = VirtualShadowStats {
            levels: levels as u32,
            finest_level: finest,
            invalidated_levels: invalidated[..levels].iter().filter(|&&b| b).count() as u32,
            moved_rects: rects.len() as u32,
            clusters: cluster_count,
            bytes: state.stats.bytes,
            page_work: true,
            occluder_gather_ready: state.stats.occluder_gather_ready,
            occluder_gather_frame: false,
        };
        let created = state.ensure_resources(
            &self.device,
            &self.ledger,
            levels,
            [size.width, size.height],
            self.submissions,
        )?;
        let constants = state.constants.clone();
        let rebuild = constants != state.built;
        // N8k: `VMAP_` constants are the map lookup's, the others the exact lookup's (N8n: but
        // `VSC_`, the surface cache's).
        let (map_constants, exact): (Vec<_>, Vec<_>) = constants
            .iter()
            .copied()
            .filter(|(name, _)| {
                !name.starts_with(surface_cache::CONSTANTS) && !name.starts_with(DISK_CONSTANTS)
            })
            .partition(|(name, _)| name.starts_with(MAP_CONSTANTS));
        if rebuild {
            // N-VSM1: `VD_` constants are the disk lookup's.
            let disk_constants: Vec<_> = constants
                .iter()
                .copied()
                .filter(|(name, _)| name.starts_with(DISK_CONSTANTS))
                .collect();
            state.pipelines.disk = self.ledger.own(
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("vd_lookup"),
                        layout: Some(&state.disk_layout),
                        module: &state.disk_module,
                        entry_point: Some("vd_lookup"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &disk_constants,
                            ..Default::default()
                        },
                        cache: None,
                    }),
                ResourceKind::ComputePipeline,
            );
            let map = |entry, layout: &wgpu::PipelineLayout| {
                self.ledger.own(
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: Some(layout),
                            module: &state.map_module,
                            entry_point: Some(entry),
                            compilation_options: wgpu::PipelineCompilationOptions {
                                constants: &map_constants,
                                ..Default::default()
                            },
                            cache: None,
                        }),
                    ResourceKind::ComputePipeline,
                )
            };
            state.pipelines.map = map("vmap_lookup", &state.map_layout);
            state.pipelines.queue_args = map("vmap_queue_args", &state.queue_layout);
        }
        if state.debug && state.pipelines.debug.is_none() || rebuild {
            let layout = &state.lookup_layout;
            let lookup = |label, debug: bool| {
                // The hybrid's own constant exists only in its module.
                let mut constants: Vec<_> = exact
                    .iter()
                    .copied()
                    .filter(|(name, _)| !name.starts_with("VSM_QUEUED"))
                    .collect();
                constants.push(("VSM_DEBUG", f64::from(u8::from(debug))));
                self.ledger.own(
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(label),
                            layout: Some(layout),
                            module: &state.module,
                            entry_point: Some("vsm_lookup"),
                            compilation_options: wgpu::PipelineCompilationOptions {
                                constants: &constants,
                                ..Default::default()
                            },
                            cache: None,
                        }),
                    ResourceKind::ComputePipeline,
                )
            };
            let debug = state.debug.then(|| lookup("vsm_lookup debug", true));
            if rebuild {
                state.pipelines.lookup = lookup("vsm_lookup", false);
                state.built = constants.clone();
                state.pipelines.queued = None;
            }
            state.pipelines.debug = debug;
        }
        if state.persistent_groups().1 && state.pipelines.order.is_none() {
            let order = |entry| {
                self.ledger.own(
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: Some(&state.lookup_layout),
                            module: &state.module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            cache: None,
                        }),
                    ResourceKind::ComputePipeline,
                )
            };
            state.pipelines.order = Some([order("vsm_order_count"), order("vsm_order_place")]);
        }
        // N8k hybrid: the exact lookup over the queue, with the exact lookup's constants.
        if state.settings.lookup == VirtualLookup::Hybrid && state.pipelines.queued.is_none() {
            let module = state.hybrid_module.get_or_insert_with(|| {
                self.ledger.own(
                    self.device
                        .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("N8k hybrid exact lookup"),
                            source: wgpu::ShaderSource::Wgsl(hybrid_source().into()),
                        }),
                    ResourceKind::ShaderModule,
                )
            });
            let mut constants = exact.clone();
            constants.push(("VSM_DEBUG", 0.0));
            state.pipelines.queued = Some(
                self.ledger.own(
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some("vsm_lookup_queued"),
                            layout: Some(&state.lookup_layout),
                            module,
                            entry_point: Some("vsm_lookup_queued"),
                            compilation_options: wgpu::PipelineCompilationOptions {
                                constants: &constants,
                                ..Default::default()
                            },
                            cache: None,
                        }),
                    ResourceKind::ComputePipeline,
                ),
            );
        }
        if state.settings.lookup == VirtualLookup::Surface {
            state.surface.as_mut().unwrap().ensure_pipelines(
                &self.device,
                &self.ledger,
                &state.lookup_layout,
                &constants,
            );
            if let Some(s4) = state.s4.as_mut() {
                s4.ensure_pipeline(&self.device, &self.ledger, &constants);
            }
        }
        let casters = state.casters.as_ref().unwrap();
        // Compare the uploaded view-projection's bits on the CPU: projection jitter,
        // aspect and depth planes can change receivers without changing the view/FOV.
        let mut key: Vec<u64> = camera
            .matrix(size.width, size.height)
            .to_cols_array()
            .iter()
            .map(|v| u64::from(v.to_bits()))
            .collect();
        key.extend([
            u64::from(size.width),
            u64::from(size.height),
            finest as u64,
            levels as u64,
            casters.generation,
        ]);
        key.extend(light_bits.map(u64::from));
        // The disk sets the march radius, so page requests depend on it too.
        key.push(slope.to_bits());
        // Receivers: the primary producer and its cut, so the rolling refresh never keeps
        // counts of other receivers.
        key.extend([
            self.primary_mode as u64,
            u64::from(self.lod_pixels.to_bits()),
            u64::from(self.software_extent.to_bits()),
            u64::from(self.lod_lift.to_bits()),
        ]);
        for i in 0..levels {
            let o = state.cache.origins[i].unwrap_or_default();
            key.extend([o[0] as u64, o[1] as u64, u64::from(state.cache.epochs[i])]);
        }
        // P-NSQ: a frame that rotates the directions never keeps counts of other directions
        // (TR1's lattice repeats its centre offset at k = 255 and 0, one matrix for both).
        key.push(u64::from(rotation));
        // N8f: moved casters alone (camera, sun basis, receivers and windows kept) still roll.
        let casters_only = !created && !rects.is_empty() && key == state.key;
        state.page_work = created || !rects.is_empty() || key != state.key;
        state.stats.page_work = state.page_work;
        state.key = key;
        // N8e rolling refresh: a frame that changes nothing but the sun (inside the map's
        // hysteresis) keeps the counts of the tiles it does not look up. The renderer clears
        // `rolling` whenever another pass writes the visibility target.
        // N8n: the surface cache keeps every cell itself; its frames never roll.
        let was_rolling = state.rolling;
        let rolling = state.rolling
            && (!state.page_work || casters_only)
            && !rebuild
            && s.refresh > 1
            // N-VSM1: the disk lookup's frames differ by TR1's jitter alone; it never rolls.
            && !matches!(s.lookup, VirtualLookup::Surface | VirtualLookup::Disk);
        state.casters_only = rolling && casters_only;
        state.history_frames = if rolling {
            state.history_frames.wrapping_add(1)
        } else {
            0
        };
        state.rolling = true;
        let schedule = if rolling {
            let [a, b] = refresh_block(s.refresh);
            let [x, y] = refresh_tile(s.refresh, state.history_frames);
            a | b << 4 | x << 8 | y << 12 | 1 << 16
        } else {
            0
        };
        state.schedule = schedule;
        if s.lookup == VirtualLookup::Surface {
            // N8n: the cells' epochs. A cell depends on the sun, the casters, the receiver
            // producer and its cut; the camera only chooses which cells are asked for. A still
            // frame (N8e's rolling condition, N8f's casters-only frames included) keeps the
            // target's counts where nothing changed them.
            let still = was_rolling && (!state.page_work || casters_only) && !rebuild;
            let producer = vec![
                self.primary_mode as u64,
                u64::from(self.lod_pixels.to_bits()),
                u64::from(self.software_extent.to_bits()),
                u64::from(self.lod_lift.to_bits()),
                state.casters.as_ref().unwrap().generation,
            ];
            let sun = (-travel).normalize().to_array().map(f32::to_bits);
            state.surface.as_mut().unwrap().advance(
                &self.queue,
                sun,
                !rects.is_empty(),
                surface_flush || sun_rebased || ladder_changed,
                producer,
                still,
            );
        }
        for value in [
            s.directions,
            schedule,
            s.walk_cap,
            u32::from(state.casters_only),
        ] {
            frame.extend_from_slice(&value.to_ne_bytes());
        }
        // P-NSQ: one batch and this frame's rotation; prepare_virtual_sampling raises the batches
        // once TR1 has chosen its budget.
        let exact = matches!(s.lookup, VirtualLookup::Exact | VirtualLookup::Surface);
        let truncated = u32::from(exact && s.truncated_walks_block);
        for value in [1, rotation, truncated, s.walk_steps] {
            frame.extend_from_slice(&u32::to_ne_bytes(value));
        }
        // P-RELIT: no restart zones until prepare_virtual_sampling has TR1's.
        frame.extend_from_slice(&[0; ZONES_BYTES]);
        debug_assert_eq!(frame.len(), FRAME_BYTES);
        let state = self.virtual_shadows.as_mut().unwrap();
        let casters = state.casters.as_ref().unwrap();
        let r = state.resources.as_mut().unwrap();
        self.queue.write_buffer(&r.frame, 0, &frame);
        if s.lookup == VirtualLookup::Disk {
            // N-VSM1 `VdFrame`: the receivers' own world-to-clip matrix, for the visible-surface
            // samples along the sun's screen direction.
            let mut bytes = Vec::with_capacity(DISK_FRAME_BYTES as usize);
            let clip = camera.matrix(size.width, size.height).to_cols_array();
            for value in clip
                .into_iter()
                .chain([size.width as f32, size.height as f32, disk_jitter, 0.0])
            {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            bytes.extend_from_slice(&disk_pattern());
            debug_assert_eq!(bytes.len() as u64, DISK_FRAME_BYTES);
            self.queue.write_buffer(&r.disk_frame, 0, &bytes);
            self.uploads.shadows += DISK_FRAME_BYTES;
        }
        let mut pass = vec![0u8; PASS_ENTRIES * PASS_STRIDE as usize];
        for i in 0..PASS_ENTRIES {
            pass[i * PASS_STRIDE as usize..i * PASS_STRIDE as usize + 4]
                .copy_from_slice(&(i as u32).to_ne_bytes());
        }
        self.queue.write_buffer(&r.pass, 0, &pass);
        self.uploads.shadows += (FRAME_BYTES + pass.len()) as u64;

        let primary = self.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let fresh = self.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let (arrays_models, _) = primary.virtual_geometry().unwrap();
        let key = [
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            (*fresh.world.texture).clone(),
            (*fresh.visibility.texture).clone(),
        ];
        let horizons = (*state.horizons.as_ref().unwrap().buffer).clone();
        if r.groups.as_ref().is_none_or(|g| {
            g.key != key
                || &g.models != arrays_models
                || g.horizons != horizons
                || g.casters != casters.generation
        }) {
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            let hit = view(&targets.hit.texture);
            let tiles = view(&targets.tiles.texture);
            let group =
                |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry<'_>]| {
                    self.ledger.own(
                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some(label),
                            layout,
                            entries,
                        }),
                        ResourceKind::BindGroup,
                    )
                };
            let l = &state.layouts;
            let clusters = &casters.instances.clusters.value;
            let groups = Groups {
                key,
                models: arrays_models.clone(),
                horizons: horizons.clone(),
                casters: casters.generation,
                mark: group(
                    "N8 page requests",
                    &l.mark,
                    &[
                        buffer(0, &r.frame),
                        buffer(1, &r.requests),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(14, &r.level_view),
                    ],
                ),
                compact: group(
                    "N8 page cache",
                    &l.compact,
                    &[
                        buffer(0, &r.frame),
                        buffer(1, &r.requests),
                        buffer(2, &r.state),
                        buffer(29, &r.render),
                        buffer(37, &r.cull_args),
                    ],
                ),
                cull: group(
                    "N8c caster cut and cull",
                    &l.cull,
                    &[
                        buffer(0, &r.frame),
                        buffer(25, clusters),
                        buffer(26, &casters.instances.groups.value),
                        buffer(27, &casters.work),
                        buffer(29, &r.render),
                    ],
                ),
                raster: group(
                    "N8c page raster",
                    &l.raster,
                    &[
                        buffer(0, &r.frame),
                        buffer(25, clusters),
                        buffer(29, &r.render),
                        buffer(31, &r.top),
                        buffer(32, &r.peel),
                        buffer(33, &casters.work),
                    ],
                ),
                mip: group(
                    "N8c max-mips",
                    &l.mip,
                    &[
                        buffer(0, &r.frame),
                        buffer(16, &r.top),
                        buffer(29, &r.render),
                        buffer(36, &r.mips),
                    ],
                ),
                pass: group(
                    "N8 page raster level",
                    &l.pass,
                    &[wgpu::BindGroupEntry {
                        binding: 30,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &r.pass,
                            offset: 0,
                            size: wgpu::BufferSize::new(16),
                        }),
                    }],
                ),
                lookup: group(
                    "N8 visibility",
                    &l.lookup,
                    &[
                        buffer(0, &r.frame),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(10, &fresh.normal.view),
                        texture(15, &fresh.visibility.view),
                        buffer(16, &r.top),
                        texture(18, &r.level_view),
                        buffer(19, &r.peel),
                        buffer(20, &r.debug),
                        buffer(24, &horizons),
                        buffer(35, &r.mips),
                    ],
                ),
                resolve: group(
                    "N8k texel planes",
                    &l.resolve,
                    &[
                        buffer(0, &r.frame),
                        buffer(16, &r.top),
                        buffer(19, &r.peel),
                        buffer(29, &r.render),
                        buffer(41, &r.planes),
                        buffer(43, &r.errors),
                    ],
                ),
                map: group(
                    "N8k map visibility",
                    &l.map,
                    &[
                        buffer(0, &r.frame),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(10, &fresh.normal.view),
                        texture(15, &fresh.visibility.view),
                        buffer(16, &r.top),
                        texture(18, &r.level_view),
                        buffer(19, &r.peel),
                        buffer(35, &r.mips),
                        buffer(40, &r.planes),
                        buffer(42, &r.errors),
                        buffer(45, &r.minmips),
                        buffer(47, &r.queue),
                    ],
                ),
                queue: group(
                    "N8k hybrid queue dispatch",
                    &l.queue,
                    &[buffer(47, &r.queue), buffer(48, &r.queue_dispatch)],
                ),
                disk: group(
                    "N-VSM1 disk visibility",
                    &l.disk,
                    &[
                        buffer(0, &r.frame),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(10, &fresh.normal.view),
                        texture(15, &fresh.visibility.view),
                        buffer(16, &r.top),
                        texture(18, &r.level_view),
                        buffer(19, &r.peel),
                        buffer(35, &r.mips),
                        buffer(50, &r.disk_frame),
                    ],
                ),
                queued: group(
                    "N8k hybrid exact lookup",
                    &l.lookup,
                    &[
                        buffer(0, &r.frame),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(10, &fresh.normal.view),
                        texture(15, &fresh.visibility.view),
                        buffer(16, &r.top),
                        texture(18, &r.level_view),
                        buffer(19, &r.peel),
                        buffer(20, &r.queue),
                        buffer(24, &horizons),
                        buffer(35, &r.mips),
                    ],
                ),
                surface: state
                    .surface
                    .as_ref()
                    .filter(|_| state.settings.lookup == VirtualLookup::Surface)
                    .map(|surface| {
                        surface.groups(
                            &self.device,
                            &self.ledger,
                            r,
                            [
                                &hit,
                                &tiles,
                                &fresh.world.view,
                                &fresh.normal.view,
                                &fresh.visibility.view,
                            ],
                        )
                    }),
                mip_dispatch: group(
                    "N8k mip dispatch arguments",
                    &l.mip_dispatch,
                    &[
                        buffer(0, &r.frame),
                        buffer(37, &r.cull_args),
                        buffer(46, &r.mip_args),
                    ],
                ),
                minmip: group(
                    "N8k min-mips",
                    &l.minmip,
                    &[
                        buffer(0, &r.frame),
                        buffer(16, &r.top),
                        buffer(29, &r.render),
                        buffer(44, &r.minmips),
                    ],
                ),
            };
            r.groups = Some(groups);
        }
        Ok(())
    }

    /// Page requests, compaction and the stale-page raster, then the 8-direction lookup
    /// into the fresh visibility target. `timestamps` brackets all of it; `mark` names its
    /// parts for the pass log (`sun_shadow.requests`, `.raster`, `.planes`, `.lookup`,
    /// `.queued`; src/pass_timeline.rs).
    pub(super) fn encode_virtual(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        let state = self.virtual_shadows.as_ref().unwrap();
        let r = state.resources.as_ref().unwrap();
        let g = r.groups.as_ref().unwrap();
        let p = &state.pipelines;
        let [width, height] = r.size;
        let (persist, order) = state.persistent_groups();
        if state.debug {
            encoder.clear_buffer(&r.debug, 0, None);
        } else if persist != 0 {
            encoder.clear_buffer(&r.debug, 0, Some(SCRATCH_HEADER));
        }
        if state.page_work {
            mark(encoder, "sun_shadow.requests");
            self.encode_virtual_pages(encoder, timestamps.as_ref(), mark);
        }
        let primary = self.primary.as_ref().unwrap();
        let (_, arrays) = primary.virtual_geometry().unwrap();
        let map = state.settings.lookup.map();
        let hybrid = state.settings.lookup == VirtualLookup::Hybrid;
        let disk = state.settings.lookup == VirtualLookup::Disk;
        if map && state.page_work {
            mark(encoder, "sun_shadow.planes");
            // N8k: the planes of the pages rasterized this frame, one workgroup per page slot.
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8k texel planes"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&p.resolve);
            pass.set_bind_group(0, &*g.resolve, &[]);
            pass.set_bind_group(1, arrays, &[]);
            pass.dispatch_workgroups(state.levels as u32 * r.pages * r.pages, 1, 1);
        }
        let exact_walk = state.settings.lookup.walks();
        if let Some(h) = state.horizons.as_ref().filter(|h| h.dispatch && exact_walk) {
            mark(encoder, "sun_shadow.records");
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8 ring horizons"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&p.horizon);
            pass.set_bind_group(1, arrays, &[]);
            for (d, &(_, triangles, _, _)) in h.key.iter().enumerate() {
                if !h.dirty[d] {
                    continue;
                }
                pass.set_bind_group(0, &*h.group, &[d as u32 * HORIZON_STRIDE as u32]);
                pass.dispatch_workgroups(triangles.div_ceil(64), 1, 1);
            }
            // N8g walk records, one pass per piece.
            pass.set_pipeline(&p.record);
            for (i, (count, batch, group)) in h.pieces.iter().enumerate() {
                if !h.dirty[*batch as usize] {
                    continue;
                }
                pass.set_bind_group(0, &**group, &[i as u32 * HORIZON_STRIDE as u32]);
                pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
            }
            let _ = (&h.params, &h.record_params, &h.segments);
        }
        if let Some(surface) = state
            .surface
            .as_ref()
            .filter(|_| state.settings.lookup == VirtualLookup::Surface)
        {
            mark(encoder, "sun_shadow.lookup");
            // N8n: probe, queued walk and insert; page work (if any) began the span.
            let timestamps = timestamps.map(|t| wgpu::ComputePassTimestampWrites {
                query_set: t.query_set,
                beginning_of_pass_write_index: if state.page_work {
                    None
                } else {
                    t.beginning_of_pass_write_index
                },
                end_of_pass_write_index: t.end_of_pass_write_index,
            });
            surface.encode(
                encoder,
                timestamps,
                r,
                g.surface.as_ref().unwrap(),
                &g.queued,
                arrays,
                &state.horizons.as_ref().unwrap().record_array,
                self.lighting.bind(),
                state.s4.as_ref().and_then(|s| s.dispatch()),
            );
            return;
        }
        if hybrid {
            encoder.clear_buffer(&r.queue, 0, Some(16));
        }
        mark(encoder, "sun_shadow.lookup");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N8 ring and map visibility"),
            timestamp_writes: timestamps
                .as_ref()
                .map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: if state.page_work {
                        None
                    } else {
                        t.beginning_of_pass_write_index
                    },
                    // The hybrid's exact pass ends the span instead.
                    end_of_pass_write_index: if hybrid {
                        None
                    } else {
                        t.end_of_pass_write_index
                    },
                })
                // A pass needs one of its timestamps (page work began the span).
                .filter(|t| {
                    t.beginning_of_pass_write_index.is_some() || t.end_of_pass_write_index.is_some()
                }),
        });
        if map {
            pass.set_pipeline(&p.map);
            pass.set_bind_group(0, &*g.map, &[]);
        } else if disk {
            pass.set_pipeline(&p.disk);
            pass.set_bind_group(0, &*g.disk, &[]);
        } else {
            pass.set_bind_group(0, &*g.lookup, &[]);
            pass.set_bind_group(1, arrays, &[]);
            pass.set_bind_group(2, &*state.horizons.as_ref().unwrap().record_array, &[]);
        }
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        // One workgroup per trace tile; a rolling frame only one tile of every block, unless
        // it moved casters (N8f), which visits every tile for the pixels they reach. N8m:
        // persistent workgroups take those tiles from a queue instead, and the order passes
        // first sort them heaviest first.
        let [a, b] = if state.schedule != 0 && !state.casters_only {
            refresh_block(state.settings.refresh)
        } else {
            [1, 1]
        };
        let grid = [
            width.div_ceil(8).div_ceil(a),
            height.div_ceil(4).div_ceil(b),
        ];
        let tiles = grid[0] * grid[1];
        if map || disk {
            pass.dispatch_workgroups(grid[0], grid[1], 1);
        } else {
            // Casters-only frames keep the grid's order (vsm_lookup).
            if let (true, Some([count, place])) = (order && !state.casters_only, &p.order) {
                for pipeline in [count, place] {
                    pass.set_pipeline(pipeline);
                    pass.dispatch_workgroups(tiles.div_ceil(256), 1, 1);
                }
            }
            match (&p.debug, state.debug) {
                (Some(debug), true) => pass.set_pipeline(debug),
                _ => pass.set_pipeline(&p.lookup),
            }
            if persist != 0 {
                pass.dispatch_workgroups(persist.min(tiles), 1, 1);
            } else {
                pass.dispatch_workgroups(grid[0], grid[1], 1);
            }
        }
        drop(pass);
        if hybrid {
            mark(encoder, "sun_shadow.queued");
            // N8k: the exact walk over the queued receivers, 32 lanes to a workgroup (N8m: a
            // small queue spreads each receiver over several, vmap_queue_args).
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8k hybrid exact visibility"),
                timestamp_writes: timestamps.map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: t.end_of_pass_write_index,
                }),
            });
            pass.set_pipeline(&p.queue_args);
            pass.set_bind_group(0, &*g.queue, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(p.queued.as_ref().unwrap());
            pass.set_bind_group(0, &*g.queued, &[]);
            pass.set_bind_group(1, arrays, &[]);
            pass.set_bind_group(2, &*state.horizons.as_ref().unwrap().record_array, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            pass.dispatch_workgroups_indirect(&r.queue_dispatch, 0);
        }
        let _ = &state.lookup_layout;
    }

    /// Tests: the triangles with walk records.
    #[cfg(test)]
    pub(super) fn test_walk_records(&self) -> Option<u64> {
        let h = self.virtual_shadows.as_ref()?.horizons.as_ref()?;
        Some(h.records.iter().map(|&[_, count]| u64::from(count)).sum())
    }

    /// Requests and compaction, then for the stale pages: clear, the per-level caster cut
    /// and cull, and the software and hardware rasters of the top layer, then the peel layer,
    /// then the rasterized pages' max-mips. Without a stale page (camera motion inside the
    /// level windows) the cull dispatches no group and the in-page mips return early.
    fn encode_virtual_pages(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<&wgpu::ComputePassTimestampWrites<'_>>,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        let state = self.virtual_shadows.as_ref().unwrap();
        let r = state.resources.as_ref().unwrap();
        let g = r.groups.as_ref().unwrap();
        let casters = state.casters.as_ref().unwrap();
        let p = &state.pipelines;
        let slots = state.levels as u64 * u64::from(r.pages * r.pages);
        let [width, height] = r.size;
        let (_, arrays) = self.primary.as_ref().unwrap().virtual_geometry().unwrap();
        encoder.clear_buffer(&r.requests, 0, Some(slots * 4));
        encoder.clear_buffer(&r.render, 0, Some(slots * 4));
        encoder.clear_buffer(&casters.work, 0, Some(MAX_LEVELS as u64 * ARGS_BYTES));
        encoder.clear_buffer(&r.cull_args, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8 page requests, cache and caster cull"),
                timestamp_writes: timestamps.map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                    end_of_pass_write_index: None,
                }),
            });
            // N-VSM1: the disk lookup requests its own windows' pages.
            pass.set_pipeline(if state.settings.lookup == VirtualLookup::Disk {
                &p.disk_mark
            } else {
                &p.mark
            });
            pass.set_bind_group(0, &*g.mark, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(4), 1);
            pass.set_pipeline(&p.compact);
            pass.set_bind_group(0, &*g.compact, &[]);
            pass.dispatch_workgroups(slots.div_ceil(64) as u32, 1, 1);
            pass.set_pipeline(&p.clear);
            pass.set_bind_group(0, &*g.raster, &[]);
            pass.dispatch_workgroups(slots as u32, 1, 1);
            pass.set_pipeline(&p.cull);
            pass.set_bind_group(0, &*g.cull, &[]);
            pass.set_bind_group(1, arrays, &[]);
            // N8f: no stale page (camera motion inside the windows) dispatches no group.
            pass.dispatch_workgroups_indirect(&r.cull_args, 0);
            pass.set_pipeline(&p.finish);
            pass.dispatch_workgroups(1, 1, 1);
        }
        mark(encoder, "sun_shadow.raster");
        for layer in 0..2 {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("N8c software page raster"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&p.software[layer]);
                pass.set_bind_group(0, &*g.raster, &[]);
                pass.set_bind_group(1, arrays, &[]);
                pass.set_bind_group(2, &*casters.data, &[]);
                for level in 0..state.levels {
                    pass.set_bind_group(3, &*g.pass, &[level as u32 * PASS_STRIDE as u32]);
                    pass.dispatch_workgroups_indirect(&casters.work, level as u64 * ARGS_BYTES);
                }
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("N8c hardware page raster"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &r.area,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&p.hardware[layer]);
            pass.set_bind_group(0, &*g.raster, &[]);
            pass.set_bind_group(1, arrays, &[]);
            pass.set_bind_group(2, &*casters.data, &[]);
            for level in 0..state.levels {
                pass.set_bind_group(3, &*g.pass, &[level as u32 * PASS_STRIDE as u32]);
                pass.draw_indirect(&casters.work, level as u64 * ARGS_BYTES + 16);
            }
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N8c clipmap max-mips"),
            timestamp_writes: None,
        });
        // N8k: the mips' dispatches are zero when no page was flagged (vsm_compact's cull
        // arguments), as camera motion inside the windows changes no top word.
        pass.set_pipeline(&p.mip_dispatch);
        pass.set_bind_group(0, &*g.mip_dispatch, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&p.mip);
        pass.set_bind_group(0, &*g.mip, &[]);
        let side = r.pages * PAGE;
        for mip in 1..=side.trailing_zeros() {
            pass.set_bind_group(3, &*g.pass, &[mip * PASS_STRIDE as u32]);
            pass.dispatch_workgroups_indirect(&r.mip_args, u64::from(mip - 1) * 12);
        }
        if state.settings.lookup.map() {
            pass.set_pipeline(&p.minmip);
            pass.set_bind_group(0, &*g.minmip, &[]);
            for mip in 1..=side.trailing_zeros() {
                pass.set_bind_group(3, &*g.pass, &[mip * PASS_STRIDE as u32]);
                pass.dispatch_workgroups_indirect(&r.mip_args, u64::from(mip - 1) * 12);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_shadow_module_validates_without_ray_queries() {
        for source in [
            shader_source(),
            records_source(),
            map_source(),
            hybrid_source(),
            surface_cache::surface_source(),
            disk_source(),
        ] {
            assert!(!source.contains("ray_query") && !source.contains("acceleration_structure"));
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
        // Verify the shader ABI rather than only duplicating Rust's size arithmetic.
        let module = naga::front::wgsl::parse_str(&shader_source()).unwrap();
        let frame = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("VsmFrame"))
            .unwrap()
            .1;
        let naga::TypeInner::Struct { members, span } = &frame.inner else {
            panic!("VsmFrame is not a struct")
        };
        assert_eq!(*span as usize, FRAME_BYTES);
        assert_eq!(
            members
                .iter()
                .find(|m| m.name.as_deref() == Some("nsq"))
                .unwrap()
                .offset as u64,
            SAMPLING_OFFSET
        );
        assert_eq!(
            FRAME_BYTES,
            928 + 12 * crate::TRACED_DRAWS as usize + ZONES_BYTES
        );
        // P-NSQ: the sampling uvec4 follows N8e's; P-RELIT: TR1's zones are the last member.
        assert_eq!(SAMPLING_OFFSET, (FRAME_BYTES - ZONES_BYTES) as u64 - 16);
        let zones = members.last().unwrap();
        assert_eq!(zones.name.as_deref(), Some("zones"));
        assert_eq!(u64::from(zones.offset), ZONES_OFFSET);
        let source = shader_source();
        let n8e = source.find("    n8e:vec4<u32>,").unwrap();
        let nsq = source.find("    nsq:vec4<u32>,").unwrap();
        assert!(n8e < nsq);
        // Both modules declare the same Tr1Zones.
        let declared = |text: &str| {
            let at = text.find("struct Tr1Zones {").unwrap();
            text[at..at + text[at..].find("};").unwrap()].to_owned()
        };
        assert_eq!(
            declared(&source),
            declared(include_str!("../shaders/primary_rays.wgsl"))
        );
        // The encoder reads the lookup schedule's defaults from the shader.
        assert_eq!(schedule_defaults(&shader_source()), (4096, true));
        assert_eq!(
            schedule_defaults("override VSM_PERSIST:u32=0u;"),
            (0, false)
        );
        assert_eq!(schedule_defaults(""), (0, false));
    }
    #[test]
    fn sun_basis_is_orthonormal_and_settings_validate() {
        for toward in [
            glam::DVec3::new(0.4, 0.8, 0.3),
            glam::DVec3::Y,
            -glam::DVec3::Y,
        ] {
            let [u, v, l] = basis(toward);
            for (a, b) in [(u, v), (v, l), (u, l)] {
                assert!(a.dot(b).abs() < 1e-12);
            }
            assert!((u.cross(v) - l).length() < 1e-12);
        }
        assert!(VirtualShadowSettings::default().validate().is_ok());
        let bad = VirtualShadowSettings {
            pages: 12,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        for bad in [
            VirtualShadowSettings {
                refresh: 3,
                ..Default::default()
            },
            VirtualShadowSettings {
                directions: 0,
                ..Default::default()
            },
            VirtualShadowSettings {
                sun_hysteresis: 0.1,
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }
    /// N-VSM1: the disk lookup's samples are a rook pattern inside the disk, at the marginal's
    /// quantiles, and the uploaded masks select by y rank.
    #[test]
    fn disk_pattern_is_a_rook_pattern_in_the_disk() {
        let mut ranks = DISK_Y_RANK;
        ranks.sort_unstable();
        assert!(ranks.iter().enumerate().all(|(k, &r)| usize::from(r) == k));
        let bytes = disk_pattern();
        assert_eq!(bytes.len() as u64, DISK_FRAME_BYTES - 80);
        let word = |i: usize| u32::from_ne_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
        let mask = |k: usize| u64::from(word(2 * k)) | u64::from(word(2 * k + 1)) << 32;
        assert_eq!(mask(0), u64::MAX);
        assert_eq!(mask(64), 0);
        for k in 0..64 {
            // Each rank removes exactly its own sample.
            let gone = mask(k) & !mask(k + 1);
            assert_eq!(gone.count_ones(), 1);
            assert_eq!(usize::from(DISK_Y_RANK[gone.trailing_zeros() as usize]), k);
        }
        let float = |i: usize| f32::from_ne_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
        let mut last = -1.0;
        for s in 0..64 {
            let (x, y) = (float(132 + 2 * s), float(133 + 2 * s));
            assert!(x > last && x * x + y * y <= 1.0);
            last = x;
        }
        // The marginal's share at a quantile is the quantile's rank.
        let share = |g: f64| 0.5 + (g * (1.0 - g * g).sqrt() + g.asin()) / std::f64::consts::PI;
        assert!((share(disk_quantile(0.3)) - 0.3).abs() < 1e-12);
    }
    /// N8e: every tile of a refresh block is looked up exactly once per `refresh` frames, so
    /// no kept count is older than `refresh - 1` frames.
    #[test]
    fn rolling_refresh_visits_every_tile_once_per_cycle() {
        for refresh in [1, 2, 4, 8, 16] {
            let [a, b] = refresh_block(refresh);
            assert_eq!(a * b, refresh);
            for start in [1, 5] {
                let mut seen: Vec<[u32; 2]> = (start..start + refresh)
                    .map(|f| refresh_tile(refresh, f))
                    .collect();
                assert!(seen.iter().all(|&[x, y]| x < a && y < b));
                seen.sort_unstable();
                seen.dedup();
                assert_eq!(seen.len() as u32, refresh);
            }
        }
    }
}
