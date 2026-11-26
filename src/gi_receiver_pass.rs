//! Optional geometry-only GI receiver preparation, before either shading path and its lists.
use super::*;
use crate::program::{self, Stage};
pub(crate) const BINDING: u32 = 68;
const BYTES: u64 = 48;
const SOURCE: &str = include_str!("../shaders/gi_receiver_pass.wgsl");
const ENTRY_SEAM: &str = "// ---- GI receiver compute entry ----";
#[path = "gi_lookup_probe.rs"]
mod lookup_probe;
/// Defaults are approved separately for each path after game-scene picture and memory checks.
/// Both on: without ray hardware the inline lookup ran at the whole shading kernel's register
/// count, and moving it out also retires the GI sun lookups booked to `sun_shadow`.
/// `RENDERLAB_GI_PASS=0` restores the inline lookup on either path.
pub(crate) const RT_DEFAULT: bool = true;
pub(crate) const NON_RT_DEFAULT: bool = true;
const HEAD_MARK: &str = "\n// renderlab: this renderer prepares GI receivers before shading\n";
fn env_switch(value: Option<&str>) -> Option<bool> {
    match value {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    }
}
fn requested() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| env_switch(std::env::var("RENDERLAB_GI_PASS").ok().as_deref()))
}
/// Explicit opt-in only. The port uses this for its existing refusal and never adopts
/// the wgpu renderer's path defaults.
pub(crate) fn on() -> bool {
    requested() == Some(true)
}
fn path_default(hardware: bool) -> bool {
    if hardware { RT_DEFAULT } else { NON_RT_DEFAULT }
}
fn default_on(device: &wgpu::Device) -> bool {
    path_default(
        device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
    )
}
pub(in crate::renderer) fn enabled(device: &wgpu::Device) -> bool {
    requested().unwrap_or_else(|| default_on(device))
}
fn default_head_for(head: &mut String, default: bool, override_on: Option<bool>) {
    if default && override_on != Some(false) {
        head.push_str(HEAD_MARK);
    }
}
/// Carry this renderer's default through every program composed on its shading heads.
/// Unmarked heads (including the port) retain inline lookup unless explicitly enabled.
pub(crate) fn default_head(head: &mut String, device: &wgpu::Device) {
    default_head_for(head, default_on(device), requested());
}
fn source_enabled(source: &str, override_on: Option<bool>) -> bool {
    override_on.unwrap_or_else(|| source.contains(HEAD_MARK))
}
pub(crate) fn for_source(source: &str) -> bool {
    source_enabled(source, requested())
}
pub(crate) fn entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: BINDING,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(BYTES),
        },
        count: None,
    }
}
/// The lighting cache's emitter list readers the final gather samples (shaders/gi_emitters.wgsl,
/// read through gi_word): its constants, the pick's struct and the functions that need no cache
/// binding. The build kernels in that file bind the cache's own buffers and stay out.
fn emitter_readers() -> String {
    const SOURCE: &str = include_str!("../shaders/gi_emitters.wgsl");
    let block = |start: usize| {
        let brace = start + SOURCE[start..].find('{').unwrap();
        let mut depth = 1;
        let mut end = brace + 1;
        while depth != 0 {
            depth += i32::from(SOURCE.as_bytes()[end] == b'{') - i32::from(SOURCE.as_bytes()[end] == b'}');
            end += 1;
        }
        format!("{}\n", &SOURCE[start..end])
    };
    let mut out: String = SOURCE.lines().filter(|l| l.starts_with("const GI_")).map(|l| format!("{l}\n")).collect();
    out.push_str(&block(SOURCE.find("struct GiEmitterPick").unwrap()));
    for name in [
        "gi_emitter_draws_at",
        "gi_emitter_cdf_at",
        "gi_emitter_vec",
        "gi_emitter_power",
        "gi_emitter_pick",
        "gi_batch_emission",
        "gi_triangle_emission",
        "gi_emitter_sh",
    ] {
        out.push_str(&block(SOURCE.find(&format!("fn {name}(")).unwrap()));
    }
    out
}
/// The GI receiver kernel's module on a traced shading `head`, exactly as the engine compiles
/// it: the standard program's traced composition (which carries the shading-side definitions
/// `shade_source` appends), the emitter list's readers and the kernel half of
/// gi_receiver_pass.wgsl.
pub(crate) fn kernel_module(head: &str) -> program::Composed {
    let mut composed = program::compose(head, program::STANDARD_PROGRAM, Stage::Traced, false);
    if program::final_gather() {
        composed.source = program::with_cache_lookup(composed.source);
    }
    // The gather's kernels are in the kernel half whether or not it runs.
    composed.source.push_str(&emitter_readers());
    let mut kernel = SOURCE.split_once(ENTRY_SEAM).unwrap().1.to_string();
    // On the non-RT software-BVH path, tracing 32 rays/probe every single frame costs ~21 ms at
    // 1080p. Use 16 rays/probe (1 warp = 32 lanes for 2 slots/blocks) plus adaptive half/quarter
    // update rate (full rate on cuts/disocclusions/movers, half rate in motion, quarter rate once
    // settled), keeping RT at full-rate 32 rays untouched.
    if head.contains(program::SOFTWARE_REFLECTION_GEOMETRY) {
        for (old, new) in [
            ("const FG_RAYS:u32=32u;", "const FG_RAYS:u32=16u;"),
            ("const FG_HALF_RATE:bool=false;", "const FG_HALF_RATE:bool=true;"),
            ("const FG_HALF_MIN:f32=8.0;", "const FG_HALF_MIN:f32=8.0;"),
            ("const FG_STABLE_RATE:u32=4u;", "const FG_STABLE_RATE:u32=8u;"),
            (
                "var<workgroup> fg_lane_sh:array<array<f32,40>,64>;",
                "var<workgroup> fg_lane_sh:array<array<f32,40>,32>;",
            ),
            (
                "var<workgroup> fg_nee_sh:array<array<f32,12>,64>;",
                "var<workgroup> fg_nee_sh:array<array<f32,12>,32>;",
            ),
            (
                "@compute @workgroup_size(64) fn fg_trace(",
                "@compute @workgroup_size(32) fn fg_trace(",
            ),
            (
                "@compute @workgroup_size(64) fn fg_adapt(",
                "@compute @workgroup_size(32) fn fg_adapt(",
            ),
        ] {
            assert_eq!(kernel.matches(old).count(), 1, "non-RT final gather seam: {old}");
            kernel = kernel.replacen(old, new, 1);
        }
    }
    // Measurement only (`RENDERLAB_GI_RX_PROBE=nolookup`): no cache lookup, so the pass costs
    // its decode and the record's stores alone. Shading then sees no GI value. The pattern holds
    // no line break and is applied to the kernel text alone: the earlier two-line pattern on the
    // composed source did not match in the game, so the switch did nothing there.
    if std::env::var("RENDERLAB_GI_RX_PROBE").as_deref() == Ok("nolookup") {
        kernel = kernel.replacen("gi_prepare(input);", "", 1);
    }
    if reuse() {
        kernel = with_reuse(&kernel);
    }
    // Measurement only: RENDERLAB_SHADE_EDITS reaches the kernel half too (program::shade_edits).
    for (old, new) in program::shade_edits() {
        if kernel.matches(old.as_str()).count() == 1 {
            kernel = kernel.replacen(old.as_str(), new, 1);
        }
    }
    composed.source.push_str(&kernel);
    // Measurement only (`RENDERLAB_GI_RX_PROBE=firstslot`): every find reads its start record
    // alone, as a find given its slot would; keys past it read as absent (a wrong picture).
    if std::env::var("RENDERLAB_GI_RX_PROBE").as_deref() == Ok("firstslot") {
        let from = "    if (first.r0.x&GI_KEY_BITS)==key.x { return first; }\n    if first.r0.x==0u { return none; }\n";
        assert_eq!(composed.source.matches(from).count(), 1, "firstslot seam");
        composed.source = composed.source.replacen(
            from,
            "    if (first.r0.x&GI_KEY_BITS)==key.x { return first; }\n    if true { return none; }\n",
            1,
        );
    }
    composed.source = lookup_probe::source(composed.source, lookup_probe::on());
    if count() {
        composed.source = with_count(composed.source);
    }
    composed
}
/// Measurement only (`RENDERLAB_GI_RX_COUNT=1`, read with `RENDERLAB_RL_ROOM_LOG`): the kernel
/// counts its lookup's paths into the reflection pass counters' spare words (logged as
/// trace = [4, 11, 12, 13, 14, 15] and tail = [5, 7, 8]): 4 receivers prepared, 5 levels sampled
/// (gi_sample), 7 cell finds, 8 slot reads past a find's first record, 11 level blends (the
/// second level of a switch), 12 refinements (a split cell's finer level), 13 coarser fills,
/// 14 second-bin lookups, 15 fallback levels sampled. The picture is unchanged.
/// Receiver reuse: on a frame where the camera is still (TR1's planner) and the lighting cache
/// has run no work since this view's records were read (`GiState::table_generation`: with two
/// views, the other view's frame may have run it), a pixel still on last frame's placement in the same normal
/// bin keeps its prepared record: the same cells of the same table, read a jitter step away. Every other pixel, and every pixel
/// on frames the cache runs, looks up as before. The keys (one word a pixel, a hash of the
/// placement and bin, 0 for misses) live in the pass's own group 2, so reuse needs the lean layout.
/// `RENDERLAB_GI_RX_REUSE=0` looks up every pixel every frame.
pub(in crate::renderer) fn reuse() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        lean() && !program::final_gather() && std::env::var("RENDERLAB_GI_RX_REUSE").map_or(true, |v| v != "0")
    })
}
/// Bytes of the keys buffer ahead of its per-pixel words: the frame's reuse word and padding.
const KEYS_HEAD: u64 = 16;
fn with_reuse(kernel: &str) -> String {
    const MISS: &str = "    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return;}\n    let record=textureLoad(primary_hit,vec2<i32>(id),0);\n    if record.x==0u {return;}\n";
    const LOOKUP: &str = "    gi_prepare(input);\n";
    assert_eq!(kernel.matches(MISS).count(), 1, "GI receiver reuse seam");
    assert_eq!(kernel.matches(LOOKUP).count(), 1, "GI receiver reuse lookup seam");
    // Misses write key 0, so a pixel covered again looks up.
    let missed = "    let at_key=id.y*size.x+id.x;\n    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {gi_rx_keys.keys[at_key]=0u;return;}\n    let record=textureLoad(primary_hit,vec2<i32>(id),0);\n    if record.x==0u {gi_rx_keys.keys[at_key]=0u;return;}\n";
    // The key: placement and normal bin, which choose the cells (the triangle changes under
    // the jitter on finely tessellated meshes: Glint, two thirds of its pixels a frame).
    let keyed = "    let key=((record.x*0x9e3779b1u)^(gi_bin(gi_bin_source(gi_viewed_normal(input.receiver.world,input.receiver.geometric),input.normal))*0x85ebca6bu))|1u;\n    if gi_rx_keys.reuse!=0u && gi_rx_keys.keys[at_key]==key {return;}\n    gi_rx_keys.keys[at_key]=key;\n    gi_prepare(input);\n";
    let declaration = "struct GiRxKeys { reuse:u32, pad0:u32, pad1:u32, pad2:u32, keys:array<u32> };\n@group(2) @binding(0) var<storage,read_write> gi_rx_keys:GiRxKeys;\n";
    format!("{declaration}{}", kernel.replacen(MISS, missed, 1).replacen(LOOKUP, keyed, 1))
}
fn count() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_RX_COUNT").is_ok_and(|v| v == "1" || v == "2"))
}
fn with_count(source: String) -> String {
    let stats = if source.contains("var<storage,read_write> rl_room_stats") {
        String::new()
    } else {
        "@group(0) @binding(69) var<storage,read_write> rl_room_stats:array<atomic<u32>,16>;\n".to_owned()
    };
    let seams: [(&str, String); 11] = [
        (
            "fn gi_prepare(input:SurfaceInput) {\n",
            format!("{stats}var<private> gi_cn:array<u32,9>;\nfn gi_prepare(input:SurfaceInput) {{\n"),
        ),
        (
            "    let world=input.receiver.world;\n    // The bin comes from",
            "    gi_cn[0]+=1u;\n    let world=input.receiver.world;\n    // The bin comes from".into(),
        ),
        (
            "    gi_lv_bw=0.0;\n    let u=gi_lattice(world,gi_edge(level),bin);\n",
            "    gi_lv_bw=0.0;\n    gi_cn[1]+=1u;\n    let u=gi_lattice(world,gi_edge(level),bin);\n".into(),
        ),
        (
            "    let none=GiHot(vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u));\n",
            "    let none=GiHot(vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u));\n    gi_cn[2]+=1u;\n".into(),
        ),
        (
            "    if first.r0.x==0u { return none; }\n",
            "    if first.r0.x==0u { return none; }\n    gi_cn[3]+=1u;\n".into(),
        ),
        (
            "        let word=gi_word(gi_hot(slot));\n        if (word&GI_KEY_BITS)==key.x { return gi_hot_load(slot); }",
            "        let word=gi_word(gi_hot(slot));\n        gi_cn[3]+=1u;\n        if (word&GI_KEY_BITS)==key.x { return gi_hot_load(slot); }".into(),
        ),
        (
            "        let level=select(pick.lo,pick.hi,j==1u);\n",
            "        let level=select(pick.lo,pick.hi,j==1u);\n        if j==1u { gi_cn[4]+=1u; }\n".into(),
        ),
        ("            sample_level=level-1u;\n", "            gi_cn[5]+=1u;\n            sample_level=level-1u;\n".into()),
        ("        let c=gi_sample(world,level+1u,bin);\n", "        gi_cn[6]+=1u;\n        let c=gi_sample(world,level+1u,bin);\n".into()),
        (
            "    let s2=gi_sample_refined(world,pick.lo,gi_second_bin(source));\n",
            "    gi_cn[7]+=1u;\n    let s2=gi_sample_refined(world,pick.lo,gi_second_bin(source));\n".into(),
        ),
        (
            "    for(var level=max(pick.lo,pick.hi)+1u;level<levels;level+=1u) {\n        let s=gi_sample(world,level,bin);\n",
            "    for(var level=max(pick.lo,pick.hi)+1u;level<levels;level+=1u) {\n        gi_cn[8]+=1u;\n        let s=gi_sample(world,level,bin);\n".into(),
        ),
    ];
    let mut source = source;
    // `=2` (with RENDERLAB_GI_BUCKET=0, so finds walk): words 11, 14 and 15 count the finds that
    // found nothing, those that found their key past the second slot, and the misses that read a
    // whole bucket with no empty slot, in place of blends, second bins and fallback levels.
    let finds = std::env::var("RENDERLAB_GI_RX_COUNT").as_deref() == Ok("2");
    for (k, (from, to)) in seams.into_iter().enumerate() {
        if finds && [6, 9, 10].contains(&k) {
            continue;
        }
        assert_eq!(source.matches(from).count(), 1, "GI receiver count seam {from}");
        source = source.replacen(from, &to, 1);
    }
    if finds {
        for (from, to) in [
            (
                "    if first.r0.x==0u { return none; }\n",
                "    if first.r0.x==0u { gi_cn[4]+=1u; return none; }\n",
            ),
            ("    if next==0u { return none; }\n", "    if next==0u { gi_cn[4]+=1u; return none; }\n"),
            (
                "        if (word&GI_KEY_BITS)==key.x { return gi_hot_load(slot); }\n        if word==0u { break; }\n    }\n    return none;\n",
                "        if (word&GI_KEY_BITS)==key.x { gi_cn[7]+=1u; return gi_hot_load(slot); }\n        if word==0u { break; }\n        if i==GI_BUCKET-1u { gi_cn[8]+=1u; }\n    }\n    gi_cn[4]+=1u;\n    return none;\n",
            ),
        ] {
            assert_eq!(source.matches(from).count(), 1, "GI receiver find count seam {from}");
            source = source.replacen(from, to, 1);
        }
    }
    let words = [4, 5, 7, 8, 11, 12, 13, 14, 15];
    let flush: String = std::iter::once("    gi_prepare(input);\n".to_owned())
        .chain(words.iter().enumerate().map(|(k, w)| {
            format!("    if gi_cn[{k}]!=0u {{ atomicAdd(&rl_room_stats[{w}],gi_cn[{k}]); }}\n")
        }))
        .collect();
    assert_eq!(source.matches("    gi_prepare(input);\n").count(), 1, "GI receiver count flush");
    source.replacen("    gi_prepare(input);\n", &flush, 1)
}
pub(crate) fn shade_source(source: String, eligible: bool, specular: bool) -> String {
    let enabled = for_source(&source);
    shade_source_for(source, eligible, specular, enabled)
}
fn shade_source_for(mut source: String, eligible: bool, specular: bool, enabled: bool) -> String {
    if !enabled || !eligible {
        return source;
    }
    source = source.replace(
        "    rl_indirect_prepare(surface_input);",
        if specular {
            "    gi_pass_load(surface_input,id,size); es_prepare(surface_input);"
        } else {
            "    gi_pass_load(surface_input,id,size);"
        },
    );
    source.push_str(SOURCE.split_once(ENTRY_SEAM).unwrap().0);
    source
}
/// What a receiver pipeline set is built for: the `Shading` index, the prelude's hash
/// (`primary_hash`), whether group 2 holds the reuse keys, and the view's shade and array
/// layouts.
pub(in crate::renderer) type ReceiverKey = (usize, [u8; 32], bool, wgpu::BindGroupLayout, wgpu::BindGroupLayout);

/// The receiver's pipelines for one `ReceiverKey`. The renderer keeps them
/// (`Renderer::gi_receiver_pipelines`) apart from the size-dependent `Targets`, so a resize
/// compiles nothing and `prepare_view_pipelines` builds them before a frame.
pub(in crate::renderer) struct ReceiverPipelines {
    key: ReceiverKey,
    pipeline: Owned<wgpu::ComputePipeline>,
    /// Whether `pipeline` is on the lean layout (`lean`), binding `group_layout` as group 2:
    /// empty, or the reuse keys (`reuse`).
    lean: bool,
    group_layout: Owned<wgpu::BindGroupLayout>,
    /// The final gather's five kernels and their group 2 layout (`program::final_gather`).
    gather: Option<[Owned<wgpu::ComputePipeline>; 5]>,
    gather_group: Option<Owned<wgpu::BindGroupLayout>>,
}

pub(super) struct Targets {
    pub values: Buffer,
    args: Buffer,
    /// Per `Shading`, the pipelines this target last bound, and its group 2 for them.
    pipelines: [Option<std::sync::Arc<ReceiverPipelines>>; 2],
    empty: Option<Owned<wgpu::BindGroup>>,
    /// Reuse's keys (`reuse`), and whether a lookup has written them and the records.
    keys: Option<Buffer>,
    computed: bool,
    /// The cache generation (`GiState::table_generation`) the records were last read at.
    table: Option<u64>,
    /// The shading region the keys were written over; pixels outside it keep stale keys, so a
    /// new region clears them (`clear_keys`, encoded with the pass) and does not reuse.
    region: [u32; 4],
    clear_keys: bool,
    /// The final gather (`program::final_gather`; its kernels are in `ReceiverPipelines`): its
    /// probes, FG_SLOTS per FG_TILE tile, one per FG_BLOCK block and the tile probes' filtered
    /// copies, follow the pixels' records in `values` (gi_receiver_pass.wgsl FgProbe). Its
    /// group 2 binds the cache's external request list (binding 30) and its own probe state
    /// (binding 31) instead of the materials group: its probes read cache cells and sample no
    /// material, and hits without a cell value ask for their cells there. Not in the shared lighting group: one more writable
    /// buffer there put the reflection hits' pipeline over the per-stage storage limit.
    /// Binding 30's stand-in while the cache has no resources (its own stand-in is bound
    /// read-only in the lighting group): four words, no room for an entry.
    requests_stand_in: Option<Buffer>,
    /// The gather's own probe state (binding 31, gi_receiver_pass.wgsl fg_state), written each
    /// frame: 20-word header (recently_changed, gather_frame, bank parity, has_prev, previous VP),
    /// then two words a probe, zero at creation.
    probe_state: Option<Buffer>,
    /// Per-pixel linear camera ray distance written by `fg_classify` and sampled by `fg_resolve`
    /// for short-range sub-probe corner contact AO (binding 32, zero extra geometry decode pass).
    fg_depth: Option<Buffer>,
    gather_frame: u32,
    previous_vp: Option<glam::Mat4>,
    size: [u32; 2],
    tiles: [u32; 2],
    blocks: [u32; 2],
}
/// Header words at the start of `probe_state` (gi_receiver_pass.wgsl FG_STATE_HEAD).
const FG_STATE_HEAD: u64 = 20;
/// The final gather's probe tile in pixels and probe size in bytes (gi_receiver_pass.wgsl
/// FG_TILE, FgProbe).
const FG_TILE: u32 = 16;
const FG_PROBE_BYTES: u64 = 192;
/// Probes per tile (gi_receiver_pass.wgsl FG_SLOTS): a second where the tile holds a second plane.
const FG_SLOTS: u64 = 2;
/// The adaptive probes' block in pixels (gi_receiver_pass.wgsl FG_BLOCK): one probe each.
const FG_BLOCK: u32 = 8;
/// The receiver kernel samples no material, so it is built on the shading layout with an empty
/// group 2 when it validates there, and binds the empty group: every dispatch merges each
/// resource of its bound groups into wgpu's usage tracking when the command buffer finishes, and
/// group 2 holds every draw's material buffers, textures and samplers. The same kernel and
/// results. `RENDERLAB_GI_RX_LEAN=0` binds the materials group.
fn lean() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_RX_LEAN").map_or(true, |v| v != "0"))
}
impl Renderer {
    pub(super) fn gi_receiver_targets(&self, size: wgpu::Extent3d) -> Targets {
        Targets {
            values: Buffer::new(
                self,
                "GI prepared receiver",
                u64::from(size.width) * u64::from(size.height) * BYTES
                    + if program::final_gather() {
                        (u64::from(size.width.div_ceil(FG_TILE)) * u64::from(size.height.div_ceil(FG_TILE)) * FG_SLOTS * 3
                            + u64::from(size.width.div_ceil(FG_BLOCK)) * u64::from(size.height.div_ceil(FG_BLOCK)) * 2)
                            * FG_PROBE_BYTES
                    } else {
                        0
                    },
                wgpu::BufferUsages::STORAGE
                    | if lookup_probe::on() {
                        wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST
                    } else {
                        wgpu::BufferUsages::empty()
                    },
            ),
            args: Buffer::new(
                self,
                "GI receiver dispatch",
                12,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            ),
            pipelines: [None, None],
            empty: None,
            keys: reuse().then(|| {
                Buffer::new(
                    self,
                    "GI receiver reuse keys",
                    KEYS_HEAD + u64::from(size.width) * u64::from(size.height) * 4,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                )
            }),
            computed: false,
            table: None,
            region: [0; 4],
            clear_keys: false,
            requests_stand_in: program::final_gather()
                .then(|| Buffer::new(self, "final gather requests stand-in", 16, wgpu::BufferUsages::STORAGE)),
            probe_state: program::final_gather().then(|| {
                let probes = u64::from(size.width.div_ceil(FG_TILE)) * u64::from(size.height.div_ceil(FG_TILE)) * FG_SLOTS
                    + u64::from(size.width.div_ceil(FG_BLOCK)) * u64::from(size.height.div_ceil(FG_BLOCK));
                Buffer::new(
                    self,
                    "final gather probe state",
                    FG_STATE_HEAD * 4 + probes * 8,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                )
            }),
            fg_depth: program::final_gather().then(|| {
                Buffer::new(
                    self,
                    "final gather linear depth",
                    (u64::from(size.width) * u64::from(size.height) * 4).max(4),
                    wgpu::BufferUsages::STORAGE,
                )
            }),
            gather_frame: 0,
            previous_vp: None,
            size: [size.width, size.height],
            tiles: [size.width.div_ceil(FG_TILE), size.height.div_ceil(FG_TILE)],
            blocks: [size.width.div_ceil(FG_BLOCK), size.height.div_ceil(FG_BLOCK)],
        }
    }
    pub(crate) fn ensure_gi_receiver_pass(&mut self, shading: Shading, camera: &Camera) -> Result<(), Error> {
        if !enabled(&self.device) {
            return Ok(());
        }
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        let Some(target) = state.targets.as_ref().and_then(|t| t.gi_receiver.as_ref()) else {
            return Ok(());
        };
        let [x, y] = state.region_groups([8, 4]);
        // The gather's probe hits append cells without a value to the cache's external list,
        // which the cache turns into requests from its next encode on.
        let gathers = program::final_gather();
        self.queue.write_buffer(
            &target.args.value,
            0,
            &[x, y, 1u32]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect::<Vec<_>>(),
        );
        let key = self.gi_receiver_key(shading, target.keys.is_some());
        let current = target.pipelines[shading as usize].as_ref().filter(|p| p.key == key);
        let built = current.is_some();
        let lean = current.is_some_and(|p| p.lean);
        let region = state.region;
        let moved = target.region != region;
        let table = self.gi.table_generation();
        if let Some(keys) = &target.keys {
            // Still: TR1's planner saw this frame's camera equal the last one's (no turn, no
            // rigid motion); the cache idles during smooth camera motion too.
            let still = self
                .view
                .reconstruction_state
                .as_ref()
                .is_some_and(|s| s.plan.held_frames() > 0);
            let reuse = built && !moved && still && lean && target.computed && table.is_some() && target.table == table;
            self.queue.write_buffer(&keys.value, 0, &u32::from(reuse).to_ne_bytes());
        }
        let current_vp = camera.matrix(target.size[0], target.size[1]);
        if gathers {
            self.gi.request_external();
            self.gi.request_emitters();
            if let Some(state) = &target.probe_state {
                let reset = self
                    .view
                    .reconstruction_state
                    .as_ref()
                    .is_some_and(|s| s.plan.reset);
                let prev = target.previous_vp.filter(|_| !reset && !moved);
                let mut words = [0u32; FG_STATE_HEAD as usize];
                words[0] = u32::from(self.gi.recently_changed());
                words[1] = target.gather_frame;
                words[2] = target.gather_frame & 1;
                words[3] = u32::from(prev.is_some());
                let m = prev.unwrap_or(current_vp).to_cols_array();
                for (i, f) in m.iter().enumerate() {
                    words[4 + i] = f.to_bits();
                }
                self.queue.write_buffer(&state.value, 0, &words.iter().flat_map(|w| w.to_ne_bytes()).collect::<Vec<_>>());
            }
        }
        let target = self.view.primary.as_mut().unwrap().targets.as_mut().unwrap().gi_receiver.as_mut().unwrap();
        target.computed = true;
        target.gather_frame = target.gather_frame.wrapping_add(1);
        target.previous_vp = Some(current_vp);
        target.table = table;
        target.clear_keys = moved && target.keys.is_some();
        target.region = region;
        if built {
            return Ok(());
        }
        let pipelines = self.gi_receiver_pipelines(shading, key)?;
        let target = self.view.primary.as_ref().unwrap().targets.as_ref().unwrap().gi_receiver.as_ref().unwrap();
        let mut group = Vec::new();
        if let Some(k) = &target.keys {
            group.push(wgpu::BindGroupEntry { binding: 0, resource: k.value.as_entire_binding() });
        }
        let empty = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("GI receiver, group 2"),
                layout: &pipelines.group_layout,
                entries: &group,
            }),
            ResourceKind::BindGroup,
        );
        let target = self.view.primary.as_mut().unwrap().targets.as_mut().unwrap().gi_receiver.as_mut().unwrap();
        target.empty = Some(empty);
        target.pipelines[shading as usize] = Some(pipelines);
        Ok(())
    }

    /// The selected view's receiver key for `shading`, with or without the reuse keys.
    pub(in crate::renderer) fn gi_receiver_key(&self, shading: Shading, keys: bool) -> ReceiverKey {
        let state = self.view.primary.as_ref().unwrap();
        (
            shading as usize,
            self.primary_hash,
            keys,
            (*state.shade_bindings[shading as usize]).clone(),
            (*state.array_bindings).clone(),
        )
    }

    /// The receiver's pipelines for `key`, built the first time it is seen and kept: frames
    /// bind them through their targets, and `prepare_view_pipelines` builds them before the
    /// first frame. A new prelude hash replaces the entry it supersedes.
    pub(in crate::renderer) fn gi_receiver_pipelines(
        &mut self,
        shading: Shading,
        key: ReceiverKey,
    ) -> Result<std::sync::Arc<ReceiverPipelines>, Error> {
        if let Some(p) = self.gi_receiver_pipelines.iter().find(|p| p.key == key) {
            return Ok(p.clone());
        }
        let state = self.view.primary.as_ref().unwrap();
        let head = &self.primary_sources.as_ref().unwrap().1[shading as usize];
        let composed = kernel_module(head);
        let abi = self.program_abi();
        let scopes = OperationScopes::new(&self.device);
        let module = super::program_gpu::module(&abi, "GI receiver", &composed);
        let start = std::time::Instant::now();
        let build_entry = |layout: &wgpu::PipelineLayout, entry: &str| {
            self.ledger.own(
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: abi.cache,
                    }),
                ResourceKind::ComputePipeline,
            )
        };
        let build = |layout: &wgpu::PipelineLayout| build_entry(layout, "gi_receiver_pass");
        let storage = |binding: u32, least: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(least),
            },
            count: None,
        };
        let mut group_entries = Vec::new();
        if key.2 {
            group_entries.push(storage(0, KEYS_HEAD + 4));
        }
        let empty_layout = self.ledger.own(
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("GI receiver, group 2"),
                entries: &group_entries,
            }),
            ResourceKind::BindGroupLayout,
        );
        let mut lean_built = None;
        let mut gather = None;
        let mut gather_group = None;
        if lean() {
            let lean_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("GI receiver ABI, lean"),
                bind_group_layouts: &[
                    Some(&state.shade_bindings[shading as usize]),
                    Some(&state.array_bindings),
                    Some(&empty_layout),
                    Some(&self.lighting.bindings),
                ],
                immediate_size: 0,
            });
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipeline = build(&lean_layout);
            if pollster::block_on(scope.pop()).is_none() {
                lean_built = Some(pipeline);
            }
        }
        let is_lean = lean_built.is_some();
        if program::final_gather() {
            let group = self.ledger.own(
                self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("final gather, group 2"),
                    entries: &[storage(30, 16), storage(31, FG_STATE_HEAD * 4), storage(32, 4)],
                }),
                ResourceKind::BindGroupLayout,
            );
            let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("final gather ABI"),
                bind_group_layouts: &[
                    Some(&state.shade_bindings[shading as usize]),
                    Some(&state.array_bindings),
                    Some(&group),
                    Some(&self.lighting.bindings),
                ],
                immediate_size: 0,
            });
            gather = Some([
                build_entry(&layout, "fg_trace"),
                build_entry(&layout, "fg_filter"),
                build_entry(&layout, "fg_classify"),
                build_entry(&layout, "fg_adapt"),
                build_entry(&layout, "fg_resolve"),
            ]);
            gather_group = Some(group);
        }
        let pipeline = lean_built.unwrap_or_else(|| build(&state.shade_layouts[shading as usize]));
        eprintln!(
            "gi_receiver_compile_seconds={:.3}",
            start.elapsed().as_secs_f64()
        );
        if let Some(error) = scopes.finish() {
            return Err(super::program_gpu::scope_error(
                "GI receiver",
                error,
                &[composed],
            ));
        }
        self.pipelines.created += 1;
        let p = std::sync::Arc::new(ReceiverPipelines {
            key,
            pipeline,
            lean: is_lean,
            group_layout: empty_layout,
            gather,
            gather_group,
        });
        self.gi_receiver_pipelines.retain(|q| {
            (q.key.0, q.key.2, &q.key.3, &q.key.4) != (p.key.0, p.key.2, &p.key.3, &p.key.4)
        });
        self.gi_receiver_pipelines.push(p.clone());
        Ok(p)
    }
    /// The receiver kernel, or with the final gather its five kernels, each in its own compute
    /// pass behind a `mark` of its name, so the pass log times them apart.
    pub(crate) fn encode_gi_receiver_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        if !enabled(&self.device) {
            return;
        }
        let state = self.view.primary.as_ref().unwrap();
        let Some(target) = state.targets.as_ref().and_then(|t| t.gi_receiver.as_ref()) else {
            return;
        };
        let Some(pipelines) = &target.pipelines[shading as usize] else {
            return;
        };
        let pipeline = &pipelines.pipeline;
        if lookup_probe::on() {
            encoder.clear_buffer(&target.values.value, 0, None);
        }
        if let Some(keys) = target.keys.as_ref().filter(|_| target.clear_keys) {
            encoder.clear_buffer(&keys.value, KEYS_HEAD, None);
        }
        let bindings = state.bindings.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let shade = &**bindings.shade[shading as usize].as_ref().unwrap();
        if let (Some([trace, filter, classify, adapt, resolve]), Some(layout), Some(stand_in), Some(probe_state), Some(fg_depth)) = (
            pipelines.gather.as_ref(),
            pipelines.gather_group.as_ref(),
            target.requests_stand_in.as_ref(),
            target.probe_state.as_ref(),
            target.fg_depth.as_ref(),
        ) {
            // The cache's list is replaced when its table grows, so the group follows it each frame.
            let requests = self.gi.external_requests();
            let requests = if std::ptr::eq(requests, &*self.gi.dummy) { &*stand_in.value } else { requests };
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("final gather, group 2"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 30, resource: requests.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 31, resource: probe_state.value.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 32, resource: fg_depth.value.as_entire_binding() },
                ],
            });
            // Grids: probe tiles, the filter's 8x4 tiles a group, blocks, two blocks a group,
            // and the resolve's indirect pixel grid (None).
            let steps: [(&'static str, &wgpu::ComputePipeline, Option<[u32; 2]>); 5] = [
                ("fg_trace", trace, Some(target.tiles)),
                ("fg_filter", filter, Some([target.tiles[0].div_ceil(8), target.tiles[1].div_ceil(4)])),
                ("fg_classify", classify, Some(target.blocks)),
                ("fg_adapt", adapt, Some([target.blocks[0].div_ceil(2), target.blocks[1]])),
                ("fg_resolve", resolve, None),
            ];
            for (name, kernel, grid) in steps {
                mark(encoder, name);
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(name),
                    timestamp_writes: None,
                });
                pass.set_pipeline(kernel);
                pass.set_bind_group(0, shade, &[]);
                pass.set_bind_group(1, &*g.arrays, &[]);
                pass.set_bind_group(2, &group, &[]);
                pass.set_bind_group(3, self.lighting.bind(), &[]);
                match grid {
                    Some([x, y]) => pass.dispatch_workgroups(x, y, 1),
                    None => pass.dispatch_workgroups_indirect(&target.args.value, 0),
                }
            }
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GI receiver"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, shade, &[]);
        pass.set_bind_group(1, &*g.arrays, &[]);
        match target.empty.as_ref().filter(|_| pipelines.lean) {
            Some(empty) => pass.set_bind_group(2, &**empty, &[]),
            None => pass.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]),
        }
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups_indirect(&target.args.value, 0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_defaults_and_overrides_agree_with_composition() {
        for default in [false, true] {
            for (value, expected) in [
                (None, default),
                (Some(""), default),
                (Some("0"), false),
                (Some("1"), true),
                (Some("other"), default),
            ] {
                let requested = env_switch(value);
                let mut head = "unmodified head".to_owned();
                default_head_for(&mut head, default, requested);
                assert_eq!(source_enabled(&head, requested), expected);
                assert_eq!(requested.unwrap_or(default), expected);
                if !default || requested == Some(false) {
                    assert_eq!(head, "unmodified head");
                }
            }
        }
        // A renderer's default mark never enables an unrelated, unmarked backend head.
        assert!(!source_enabled("port head", None));
    }
    #[test]
    fn disabled_composition_is_byte_identical() {
        for eligible in [false, true] {
            for specular in [false, true] {
                let source = "    rl_indirect_prepare(surface_input);\n".to_owned();
                assert_eq!(
                    shade_source_for(source.clone(), eligible, specular, false),
                    source
                );
            }
        }
        assert_eq!(BYTES, 48);
    }
    #[test]
    fn receiver_and_shading_validate_both_paths() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let hardware = super::super::shading_sources(prelude, program::STANDARD_PROGRAM).unwrap();
        let (_, software_heads) = super::super::non_rt_sources(prelude).unwrap();
        let software = software_heads.map(|head| {
            program::compose(&head, program::STANDARD_PROGRAM, Stage::Traced, false).source
        });
        for source in hardware.into_iter().chain(software) {
            let mut source = if source.contains("struct GiPrepared") {
                source
            } else {
                shade_source_for(source, true, true, true)
            };
            source.push_str(&emitter_readers());
            source.push_str(SOURCE.split_once(ENTRY_SEAM).unwrap().1);
            // The final gather's module carries the cache lookup on both paths.
            let source = program::with_cache_lookup(source);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            let record = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some("GiPrepared"))
                .unwrap()
                .1;
            let naga::TypeInner::Struct { members, span } = &record.inner else {
                panic!("GI record must be a struct")
            };
            assert_eq!(*span, BYTES as u32);
            assert_eq!(
                members.iter().map(|m| m.offset).collect::<Vec<_>>(),
                [0, 16, 32]
            );
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            // Vulkan's backend writes it too (overrides at their defaults, as wgpu does first):
            // some modules validate and still fail there.
            let (module, info) =
                naga::back::pipeline_constants::process_overrides(&module, &info, None, &Default::default())
                    .unwrap();
            // The kernels this file owns (the shading entries are checked where they are built).
            for entry in module.entry_points.iter().filter(|e| e.name.starts_with("fg_") || e.name == "gi_receiver_pass") {
                let pipeline = naga::back::spv::PipelineOptions {
                    shader_stage: naga::ShaderStage::Compute,
                    entry_point: entry.name.clone(),
                };
                let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), Some(&pipeline))
                }));
                assert!(matches!(written, Ok(Ok(_))), "SPIR-V of {}", entry.name);
            }
        }
    }
    #[test]
    fn kernel_module_validates_both_rt_and_non_rt_paths() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let mut rt_head = super::super::shading_heads(prelude).unwrap()[0].clone();
        let mut non_rt_head = super::super::non_rt_sources(prelude).unwrap().1[0].clone();
        default_head_for(&mut rt_head, true, None);
        default_head_for(&mut non_rt_head, true, None);
        for (label, head, expect_half_rate) in [
            ("rt", &rt_head, false),
            ("non_rt", &non_rt_head, true),
        ] {
            let composed = kernel_module(head);
            assert_eq!(
                composed.source.contains("const FG_HALF_RATE:bool=true;"),
                expect_half_rate,
                "{label} FG_HALF_RATE mismatch"
            );
            let module = naga::front::wgsl::parse_str(&composed.source)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&composed.source)));
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            let (module, info) = naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                None,
                &Default::default(),
            )
            .unwrap();
            for entry in module
                .entry_points
                .iter()
                .filter(|e| e.name.starts_with("fg_") || e.name == "gi_receiver_pass")
            {
                let pipeline = naga::back::spv::PipelineOptions {
                    shader_stage: naga::ShaderStage::Compute,
                    entry_point: entry.name.clone(),
                };
                let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    naga::back::spv::write_vec(
                        &module,
                        &info,
                        &naga::back::spv::Options::default(),
                        Some(&pipeline),
                    )
                }));
                assert!(matches!(written, Ok(Ok(_))), "{label} SPIR-V of {}", entry.name);
            }
        }
    }
}
