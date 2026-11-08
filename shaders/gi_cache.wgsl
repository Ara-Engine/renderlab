// ---- renderlab world radiance cache kernels (M7 G1: sky visibility) ----
// Composed after the built-in prelude, primary_rays.wgsl, gi_cells.wgsl and one ray
// source (gi_rays_hardware.wgsl or gi_rays_software.wgsl); src/gi_cache.rs owns the
// buffers and the per-frame order:
//   gi_probe    covered pixels list the cells they read (bin from the interpolated
//               vertex normal), deduplicated per 8x8 tile; each tile then finds its
//               unique cells once, marks them seen, and lists the missing ones in the
//               frame's request table (once per frame); then each pixel walks what its
//               lookup will read: the coarser level's cells where its level has no value
//               but an orphan (up to two levels), and the finer level's where its cells
//               are split (G1c, G1d);
//   gi_claim    each miss picks a free slot as the table stands and raises the slot's
//               claim word to its claim value (atomicMax, C4): its generation band, then
//               a rank from its key that no other key of its bucket shares, so exactly one
//               miss wins a slot whatever the thread order (gi_miss_claim);
//   gi_resolve  the highest claim writes the cell and queues it for tracing; claim and
//               resolve repeat in rounds, so collisions retry within the frame; the
//               last round (gi_resolve_last) also clears each miss's request entry;
//   gi_scan     on event frames only: moved casters invalidate the direction slots
//               whose fixed rays they reach (the whole cell when they reach its
//               cube) and re-queue it, the fixed rotation re-queues
//               (slot+frame) mod P == 0, an environment change re-integrates, and
//               partially traced cells (W1 form 2) re-queue;
//   gi_setup    eight lanes per queued cell: the canonical query point when it has
//               none (five fixed probe lines in parallel, which also decide whether the
//               cell is split), the directions cooked near-field masks block where it
//               has them (G1c), then one ray item per unknown (or this phase's) direction
//               they leave open;
//   gi_rays     one thread per ray item: that direction's visibility bit;
//   gi_finish   four lanes per queued cell: the bits into its direction slots and the
//               SH over every slot, replacing the value (W1); after a miss-list
//               overflow it also clears the whole request table;
//   gi_integrate the SH again from the stored bits, with no rays (environment frames).
// With bounce light (M7 G2, docs/m7-g2.md), when the frame's program declares `scatter` or
// `emission`:
//   gi_gather   after the probe, eight lanes per cell of last frame's gather queue: one
//               closest-hit ray along each of its blocked fixed directions, reading the
//               hit cell's outgoing radiance (the cache-terminated path: that radiance
//               carries the hit's own bounce, so bounces add up over gathers) and requesting
//               a hit cell that does not exist yet; the L1 SH of those radiances replaces
//               the cell's bounce value (W1). Cells it reaches go to cell shading;
//   gi_light    one thread per cell to shade: one visibility ray per light at its query
//               point, through this module's ray source;
//   then each program's cell shading (gi_shade_entry.wgsl) writes the cells' outgoing
//   radiance from the program's own `surface`, `scatter` and `emission`.
// A gather is scheduled for each cell an event reaches (new or re-traced cells, lighting
// and environment steps, a fixed slot group per frame while lighting, the environment or
// casters keep changing), a fixed number of times: unconditional, never ranked (W2).
// Every pass is a direct dispatch of a fixed grid that reads its count in the kernel and
// loops (src/gi_cache.rs *_GROUPS): no indirect dispatch, so no argument passes and no
// indirect-call validation. src/gi_cache.rs skips scan and integrate without an event.
// Every new or invalidated cell is traced in the frame it appears: no ranking decides
// which cells are traced (W2). No reservoirs or resampling anywhere (C1).

struct GiFrame {
    // The traced primaries' shading region [x0, y0, x1, y1).
    region:vec4<u32>,
    // x: moved caster boxes, y: rotation period (0 off), z: scan flags, w: miss capacity.
    counts:vec4<u32>,
    // x: slots, y: miss list offset, z: trace queue offset, w: integrate queue offset.
    offsets:vec4<u32>,
    // x: request table offset, y: its entry mask (two words per entry), z: ray item
    // list offset (GI_RAY_ITEMS words, then the staged visibility bits, one per item), w:
    // the environment's slot group: period (bits 0-7; 0: every slot), phase (bits 8-15),
    // and bit 31 when the scan visits only that group (environment-only frames).
    requests:vec4<u32>,
    // Bounce (G2): x this frame's gather queue (its count, then slots), y next frame's,
    // z the relight event (GI_RELIGHT_*: bit 31 every cell, else a slot group's period in
    // bits 0-7 and phase in bits 8-15; bit 29 when gathers may request cells, which is
    // whenever the claim rounds run), w the cell shading queue (its count is GI_W_SHADES).
    bounce:vec4<u32>,
    // Swept world bounds of moved placements: lo.xyz then hi.xyz.
    boxes:array<vec4<f32>,64>,
};
const GI_SCAN_CASTER:u32=1u;
const GI_SCAN_ROTATION:u32=2u;
const GI_SCAN_ENVIRONMENT:u32=4u;
const GI_SCAN_PARTIAL:u32=8u;
// Lighting, environment or program changed (G2): cells relight (bounce.z says which).
const GI_SCAN_RELIGHT:u32=16u;
const GI_RELIGHT_ALL:u32=0x80000000u;
const GI_RELIGHT_REQUESTS:u32=0x20000000u;
// Work counters: misses, traces, integrations, then statistics.
const GI_W_MISSES:u32=0u;
const GI_W_TRACES:u32=1u;
const GI_W_INTEGRATES:u32=2u;
const GI_W_MISS_OVERFLOW:u32=3u;
const GI_W_FULL:u32=4u;
const GI_W_ORPHANS:u32=5u;
const GI_W_LOCAL_OVERFLOW:u32=6u;
const GI_W_CREATED:u32=7u;
const GI_W_INVALIDATED:u32=8u;
const GI_W_REFRESHED:u32=9u;
const GI_W_LOST:u32=10u;
const GI_W_RAYS:u32=11u;
// Ray items allocated this frame (may pass the capacity: those cells trace in setup).
const GI_W_ITEMS:u32=12u;
// Orphans by cause: no candidate qualified (no hit, another bin or outside the cube; 13),
// or every qualifying candidate lay inside a solid (14).
const GI_W_ORPHAN_NONE:u32=13u;
const GI_W_ORPHAN_INSIDE:u32=14u;
// Cells probed this frame that are split.
const GI_W_SPLITS:u32=15u;
// Probe statistics on profiled frames (GI_F_STATS; G1d): tiles with a covered pixel, covered
// pixels, tile-table insert calls and their probe steps, unique cells from pixels, added for
// orphans' parents and by refinement, finds and how many found a cell, refinement walks
// (pixel, bin and level), tiles that refined, the most unique cells in one tile, and tiles
// by unique cells (< 32, < 64, < GI_LOCAL, full).
const GI_W_P_TILES:u32=16u;
const GI_W_P_PIXELS:u32=17u;
const GI_W_P_INSERTS:u32=18u;
const GI_W_P_STEPS:u32=19u;
const GI_W_P_CELLS:u32=20u;
const GI_W_P_PARENTS:u32=21u;
const GI_W_P_REFINED:u32=22u;
const GI_W_P_FINDS:u32=23u;
const GI_W_P_FOUND:u32=24u;
const GI_W_P_WALKS:u32=25u;
const GI_W_P_REFINE_TILES:u32=26u;
const GI_W_P_MAX:u32=27u;
const GI_W_P_HIST:u32=28u;
// Bounce (G2): cells queued for cell shading (gi_shade_entry.wgsl reads this word), cells
// gathered, their rays, rays whose hit cell had no value yet, cells they requested, light
// visibility rays, and appends to next frame's gather queue.
const GI_W_SHADES:u32=32u;
const GI_W_GATHERS:u32=33u;
const GI_W_GATHER_RAYS:u32=34u;
const GI_W_GATHER_MISSES:u32=35u;
const GI_W_GATHER_REQUESTS:u32=36u;
const GI_W_LIGHT_RAYS:u32=37u;
const GI_W_BQUEUE:u32=38u;
// Gather rays without a value, by cause: no cell at the hit's level (requested), a cell not
// shaded yet, an orphan whose coarser levels had none either.
const GI_W_GATHER_ABSENT:u32=39u;
const GI_W_GATHER_UNSHADED:u32=40u;
const GI_W_GATHER_ORPHANS:u32=41u;
// Queued cells a capped gather frame left for next frame (gi_gather).
const GI_W_GATHER_DEFERRED:u32=42u;
// Listed misses a capped claim left for a later frame (gi_claim, GI_H_CLAIM_CELLS).
const GI_W_CLAIM_DEFERRED:u32=43u;
// Ray item capacity, a multiple of 32 (src/gi_cache.rs RAY_ITEMS).
const GI_RAY_ITEMS:u32=2097152u;
// Per-slot claim words, after the counters (src/gi_cache.rs COUNTER_WORDS).
const GI_W_CLAIMS:u32=44u;
// A miss: definition (4 words), request-table entry, claimed slot.
const GI_MISS_WORDS:u32=6u;
const GI_UNCLAIMED:u32=0xffffffffu;
const GI_CREATED:u32=0xfffffffeu;
// Prefiltered environment roughness whose lobe is about one direction's solid angle.
const GI_ENV_ROUGHNESS:f32=0.35;

@group(0) @binding(30) var<storage,read_write> gi_table:array<u32>;
fn gi_word(i:u32) -> u32 { return gi_table[i]; }
@group(0) @binding(31) var<storage,read_write> gi_work:array<atomic<u32>>;
@group(0) @binding(32) var<uniform> gi_frame:GiFrame;
// Bounce (G2): each slot's outgoing radiance (rgb; w 1 once cell shading wrote it, 0 before),
// written by cell shading, read by gathers, cleared when a slot takes a new cell.
@group(0) @binding(33) var<storage,read_write> gi_outgoing:array<vec4<f32>>;
// Per draw (batch) of this frame: its stable id (0: none), first placement and instance
// count, so a cell's material names its placement whatever the draw order (src/gi_cache.rs).
@group(0) @binding(36) var<storage,read> gi_draws:array<vec4<u32>>;
struct GiVec4s { data:array<vec4<f32>> };
struct GiU32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> gi_vertices:binding_array<GiVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> gi_indices:binding_array<GiU32s,PRIMARY_BATCHES>;

// ---- the shared receiver of one covered pixel ----
struct GiReceiver { world:vec3<f32>, geometric:vec3<f32>, vertex:vec3<f32> };
// The same receiver the traced shading entry builds (program_traced_entry.wgsl):
// receiver_point on the hit triangle in the placement's frame, its plane normal, and the
// interpolated vertex normal it passes as SurfaceInput.normal (the pixel's bin source).
fn gi_receiver(record:vec4<u32>) -> GiReceiver {
    let placement=record.x-1u;
    let batch=primary_placements[placement]&0x1fffffffu;
    let i=record.y*3u;
    let ia=gi_indices[batch].data[i];let ib=gi_indices[batch].data[i+1u];let ic=gi_indices[batch].data[i+2u];
    let a=gi_vertices[batch].data[ia*3u].xyz;let b=gi_vertices[batch].data[ib*3u].xyz;let c=gi_vertices[batch].data[ic*3u].xyz;
    let bary=bitcast<vec2<f32>>(record.zw);
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let world=world_position(receiver_point(a,b,c,w),placement).xyz;
    // The interpolated vertex normal, as the traced shading entry forms SurfaceInput.normal.
    let m=normals[placement];
    let basis=mat3x3<f32>(m.x.xyz,m.y.xyz,m.z.xyz);
    let na=unit_normal(basis*unit_normal(vec3<f32>(gi_vertices[batch].data[ia*3u].w,gi_vertices[batch].data[ia*3u+1u].xy)));
    let nb=unit_normal(basis*unit_normal(vec3<f32>(gi_vertices[batch].data[ib*3u].w,gi_vertices[batch].data[ib*3u+1u].xy)));
    let nc=unit_normal(basis*unit_normal(vec3<f32>(gi_vertices[batch].data[ic*3u].w,gi_vertices[batch].data[ic*3u+1u].xy)));
    return GiReceiver(world,primary_geometric(a,b,c,placement),na*w.x+nb*w.y+nc*w.z);
}

// ---- probe: per-tile unique cells, then one lookup each ----
// Pixels per probe tile side (src/gi_cache.rs PROBE_TILE). 16x16 tiles found each unique cell
// once per tile with about 40% fewer finds than 8x8 on SV (G1b), but on the quarry the probe
// waits mostly at its barriers (G1d's trace: about a third of its samples), and 8x8 tiles sync
// two warps instead of eight: the quarry probe is about 8% faster at 8x8 with 256 entries,
// and SV unchanged (docs/m7-g1d.md).
const GI_PROBE_TILE:u32=8u;
const GI_PROBE_LANES:u32=GI_PROBE_TILE*GI_PROBE_TILE;
// Entries of the tile's cell table (GI_W_LOCAL_OVERFLOW counts any cell it drops). An 8x8 tile
// on scanned rock lists up to about 150 cells (its normals spread over several bins); 128
// entries overflowed on the quarry, 256 do not.
const GI_LOCAL:u32=256u;
var<workgroup> gi_local_key:array<atomic<u32>,GI_LOCAL>;
var<workgroup> gi_local_def:array<vec4<u32>,GI_LOCAL>;
// The tile's unique cells as hash slots, in insertion order, so the find loop visits
// only occupied entries (one pass for up to GI_PROBE_LANES cells).
var<workgroup> gi_local_list:array<u32,GI_LOCAL>;
var<workgroup> gi_local_count:atomic<u32>;
// The end of this round's entries (lane 0 reads the count after the barrier).
var<workgroup> gi_local_end:u32;
// Per entry: its GI_LS_* state once found this frame (written by the one lane that finds it);
// and whether any entry so far is split or an orphan, which starts the pixels' walks.
var<workgroup> gi_local_state:array<u32,GI_LOCAL>;
var<workgroup> gi_local_any_flag:u32;
const GI_LS_SPLIT:u32=1u;
const GI_LS_VALID:u32=2u;
const GI_LS_ORPHAN:u32=4u;
// Rounds of finds: each finds what the last added. Pixels' walks add coarser levels where a
// level has no value but an orphan (up to GI_PARENT_ROUNDS levels, one a round), and the finer
// level under split cells (GI_REFINE levels, round 0). Four parent levels (G2; two before):
// pixels whose cells and two coarser levels were all orphans fell back to the open sky, bright
// specks in the quarry's hollows (196 pixels at pose 0 against 38, and none in the hollow
// under the central slab, docs/m7-g2.md); rounds end early where a tile adds nothing.
const GI_PROBE_ROUNDS:u32=6u;
const GI_PARENT_ROUNDS:u32=4u;
// Probe statistics (GI_F_STATS): each lane counts in private scalars and adds them to the
// workgroup's once; lane 0 adds the tile's to the work counters. gi_cause says which kind of
// request the current inserts are (0 pixels, 1 orphans' parents, 2 refinement).
var<private> gi_stats:bool;
var<private> gi_cause:u32;
var<private> gi_n_inserts:u32;
var<private> gi_n_steps:u32;
var<private> gi_n_pixel:u32;
var<private> gi_n_parent:u32;
var<private> gi_n_refine:u32;
var<private> gi_n_finds:u32;
var<private> gi_n_found:u32;
var<private> gi_n_walks:u32;
var<workgroup> gi_wg_stats:array<atomic<u32>,9>;
var<workgroup> gi_wg_stats_on:u32;

// Adds one cell to the tile's set.
fn gi_local_insert(level:u32, bin:u32, cell:vec3<i32>) {
    let key=gi_key(level,bin,cell);
    var h=key.x%GI_LOCAL;
    if gi_stats { gi_n_inserts+=1u; }
    for(var i=0u;i<GI_LOCAL;i+=1u) {
        if gi_stats { gi_n_steps+=1u; }
        let r=atomicCompareExchangeWeak(&gi_local_key[h],0u,key.x);
        if r.exchanged {
            gi_local_def[h]=vec4<u32>(level|(bin<<8u),bitcast<vec3<u32>>(cell));
            gi_local_list[atomicAdd(&gi_local_count,1u)]=h;
            if gi_stats {
                if gi_cause==0u { gi_n_pixel+=1u; } else if gi_cause==1u { gi_n_parent+=1u; } else { gi_n_refine+=1u; }
            }
            return;
        }
        if r.old_value==key.x { return; }
        // A weak exchange may fail spuriously with the slot still empty: retry it.
        if r.old_value!=0u { h=(h+1u)%GI_LOCAL; }
    }
    atomicAdd(&gi_work[GI_W_LOCAL_OVERFLOW],1u);
}
// The cells a receiver reads at one level and bin, as a descriptor: the four bilinear
// cells of its layer, or the containing cell. Where the point lies within rounding of a
// face that decides the cell, the neighbour is requested too, so a shading module's own
// arithmetic always finds its cell. (Bilinear weights vanish at the faces of the
// tangent axes.) x: level, bin<<8, the near-face flags from bit 11 (bilinear: below and
// above along the normal axis; nearest: below xyz, above xyz); yzw: the base cell.
fn gi_request_desc(world:vec3<f32>, level:u32, bin:u32) -> vec4<u32> {
    let u=gi_lattice(world,gi_edge(level),bin);
    let f=u-floor(u);
    let margin=1e-4+4e-7*max(max(abs(u.x),abs(u.y)),abs(u.z));
    let near_low=f<vec3<f32>(margin);
    let near_high=f>vec3<f32>(1.0-margin);
    var flags=0u;
    var base=vec3<i32>(floor(u));
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u {
        let normal=gi_axis_mask(bin)>vec3<f32>(0.5);
        base=gi_corners(u,bin).base;
        flags=select(0u,1u,any(near_low&normal))|select(0u,2u,any(near_high&normal));
    } else {
        flags=select(0u,1u,near_low.x)|select(0u,2u,near_low.y)|select(0u,4u,near_low.z)
            |select(0u,8u,near_high.x)|select(0u,16u,near_high.y)|select(0u,32u,near_high.z);
    }
    return vec4<u32>(level|(bin<<8u)|(flags<<11u),bitcast<vec3<u32>>(base));
}
// Adds a descriptor's cells to the tile's set.
fn gi_request_cells(desc:vec4<u32>) {
    let level=desc.x&0xffu;
    let bin=(desc.x>>8u)&7u;
    let flags=(desc.x>>11u)&63u;
    let base=bitcast<vec3<i32>>(desc.yzw);
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u {
        // Layers along the normal axis: the containing one, and a neighbour near a face.
        let axis=vec3<i32>(gi_axis_mask(bin)>vec3<f32>(0.5));
        for(var layer=-1;layer<=1;layer+=1) {
            if (layer==-1 && (flags&1u)==0u) || (layer==1 && (flags&2u)==0u) { continue; }
            for(var k=0u;k<4u;k+=1u) {
                gi_local_insert(level,bin,base+gi_corner_offset(bin,k)+axis*layer);
            }
        }
        return;
    }
    let lo=-vec3<i32>((vec3<u32>(flags)>>vec3<u32>(0u,1u,2u))&vec3<u32>(1u));
    let hi=vec3<i32>((vec3<u32>(flags)>>vec3<u32>(3u,4u,5u))&vec3<u32>(1u));
    for(var x=lo.x;x<=hi.x;x+=1) {
        for(var y=lo.y;y<=hi.y;y+=1) {
            for(var z=lo.z;z<=hi.z;z+=1) {
                gi_local_insert(level,bin,base+vec3<i32>(x,y,z));
            }
        }
    }
}
// A listed cell's GI_LS_* state (0 when unlisted, or listed but not found).
fn gi_local_lookup(key:u32) -> u32 {
    var h=key%GI_LOCAL;
    for(var i=0u;i<GI_LOCAL;i+=1u) {
        let k=atomicLoad(&gi_local_key[h]);
        if k==key { return gi_local_state[h]; }
        if k==0u { return 0u; }
        h=(h+1u)%GI_LOCAL;
    }
    return 0u;
}
// The cells the lookup reads at one level (the four bilinear corners of the point's layer, or
// its containing cell), from the tile's set: x the bilinear weight of those found with a value,
// y their GI_LS_SPLIT and GI_LS_ORPHAN bits ORed.
fn gi_local_state_at(world:vec3<f32>, level:u32, bin:u32) -> vec2<f32> {
    let u=gi_lattice(world,gi_edge(level),bin);
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)==0u {
        let st=gi_local_lookup(gi_key(level,bin,vec3<i32>(floor(u))).x);
        return vec2<f32>(select(0.0,1.0,(st&GI_LS_VALID)!=0u),f32(st&(GI_LS_SPLIT|GI_LS_ORPHAN)));
    }
    let corners=gi_corners(u,bin);
    var w=0.0;
    var bits=0u;
    for(var c=0u;c<4u;c+=1u) {
        let st=gi_local_lookup(gi_key(level,bin,corners.base+gi_corner_offset(bin,c)).x);
        if (st&GI_LS_VALID)!=0u { w+=gi_corner_weight(corners.f,bin,c); }
        bits|=st&(GI_LS_SPLIT|GI_LS_ORPHAN);
    }
    return vec2<f32>(w,f32(bits));
}
// A pixel's walk after round `round` along one chain (one of its levels and bins), as shading's
// lookup reads (gi_api.wgsl indirect_irradiance):
// - at its own level, a split cell lists the next finer level's cells (round 0; GI_REFINE 1),
//   which the refined lookup reads;
// - a level whose corners have no value but include an orphan lists the next coarser level's
//   cells around the point (up to GI_PARENT_ROUNDS levels), which the fallback reads.
// Round r checks level `level`+r: the levels before it had no value and an orphan (otherwise the
// chain ended), and found cells keep their state across rounds. Returns whether the chain ended:
// a level with a value, cells only unlisted (new this frame, traced before shading), or no
// deeper level. Pixels request parents only where shading falls back to them: four parents per
// found orphan (G1b-G1c) listed about as many parents as pixel cells on scans and overflowed
// tile tables (docs/m7-g1d.md).
fn gi_pixel_walk(world:vec3<f32>, level:u32, bin:u32, round:u32, levels:u32) -> bool {
    let l=level+round;
    if l>=levels { return true; }
    if gi_stats { gi_n_walks+=1u; }
    let st=gi_local_state_at(world,l,bin);
    let bits=u32(st.y);
    if round<GI_REFINE && (bits&GI_LS_SPLIT)!=0u && l>0u {
        gi_cause=2u;
        gi_request_cells(gi_request_desc(world,l-1u,bin));
    }
    if st.x>1e-4 || (bits&GI_LS_ORPHAN)==0u || round>=GI_PARENT_ROUNDS || l+1u>=levels { return true; }
    gi_cause=1u;
    gi_request_cells(gi_request_desc(world,l+1u,bin));
    return false;
}
@compute @workgroup_size(8,8) fn gi_probe(@builtin(global_invocation_id) grid:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    for(var i=lane;i<GI_LOCAL;i+=GI_PROBE_LANES) {
        atomicStore(&gi_local_key[i],0u);
        gi_local_state[i]=0u;
    }
    if lane==0u {
        atomicStore(&gi_local_count,0u);
        gi_local_any_flag=0u;
        gi_wg_stats_on=gi_word(GI_H_FLAGS)&GI_F_STATS;
    }
    if lane<9u { atomicStore(&gi_wg_stats[lane],0u); }
    gi_cause=0u;
    // Uniform (the load carries the barrier), so the statistics' barrier is legal.
    let stats=workgroupUniformLoad(&gi_wg_stats_on)!=0u;
    gi_stats=stats;
    let p=grid.xy+gi_frame.region.xy;
    let size=textureDimensions(primary_hit);
    var record=vec4<u32>(0u);
    if all(p<gi_frame.region.zw) && all(p<size) && textureLoad(primary_tiles,p/PRIMARY_TILE,0).x!=0u {
        record=textureLoad(primary_hit,vec2<i32>(p),0);
    }
    // The pixel's receiver, bins and levels, kept for its walks, and the chains (bin, low or
    // high level) whose walk has ended.
    var world=vec3<f32>(0.0);
    var bins=0u;
    var walked=0u;
    var pick=GiLevels(0u,0u,0.0);
    if record.x!=0u {
        let r=gi_receiver(record);
        world=r.world;
        let n=gi_bin_source(gi_viewed_normal(r.world,r.geometric),r.vertex);
        // Both levels around a switch: inside the blend band, and within rounding of
        // the switch itself, where the shading module's ceil may go either way.
        pick=gi_levels(r.world,1e-3);
        // Bins on either side of a near tie, and both sides of a near-grazing view.
        let a=abs(n);
        bins=1u<<gi_bin(n);
        let eps=1e-4;
        if abs(a.x-a.y)<eps || abs(a.x-a.z)<eps || abs(a.y-a.z)<eps {
            bins|=1u<<gi_bin(n+vec3<f32>(eps*sign(n.x),0.0,0.0));
            bins|=1u<<gi_bin(n+vec3<f32>(0.0,eps*sign(n.y),0.0));
            bins|=1u<<gi_bin(n+vec3<f32>(0.0,0.0,eps*sign(n.z)));
        }
        let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
        let toward=eye-r.world;
        if abs(dot(r.geometric,toward))<1e-4*length(toward) { bins|=1u<<gi_bin(-n); }
        // Every pixel inserts its own requests. (Skipping pixels whose tile neighbour
        // requests the identical cells is exact but measured only ~5 us faster.)
        for(var b=0u;b<6u;b+=1u) {
            if (bins&(1u<<b))==0u { continue; }
            gi_request_cells(gi_request_desc(r.world,pick.lo,b));
            if pick.hi!=pick.lo { gi_request_cells(gi_request_desc(r.world,pick.hi,b)); }
        }
    }
    workgroupBarrier();
    // Each unique cell once, in rounds: found cells are marked seen and missing ones are
    // listed. After each round the pixels walk their chains (gi_pixel_walk), adding the finer
    // and coarser cells their lookups will read, which the next round finds or lists.
    let capacity=gi_frame.counts.w;
    let levels=gi_word(GI_H_LEVELS);
    var begin=0u;
    gi_cause=1u;
    for(var round=0u;round<GI_PROBE_ROUNDS;round+=1u) {
        if lane==0u { gi_local_end=min(atomicLoad(&gi_local_count),GI_LOCAL); }
        // Uniform across the workgroup (the load carries the barrier), so a round that adds
        // nothing ends the loop for every lane: tiles without orphans pay one round.
        let to=workgroupUniformLoad(&gi_local_end);
        if to==begin { break; }
        for(var k=begin+lane;k<to;k+=GI_PROBE_LANES) {
            let i=gi_local_list[k];
            let def=gi_local_def[i];
            // Key a is the tile table's own key; finds need nothing else (a request computes
            // key b, which picks one cell among those sharing key a: gi_request_list).
            let key=vec2<u32>(atomicLoad(&gi_local_key[i]),0u);
            let slot=gi_find(key);
            if gi_stats {
                gi_n_finds+=1u;
                if slot!=0xffffffffu { gi_n_found+=1u; }
            }
            if slot!=0xffffffffu {
                // Tiles sharing a cell store the same frame: the race is benign. The store
                // is unconditional: reading the stamp first stalled the probe on a second
                // dependent load per cell.
                gi_table[gi_cold(slot)+GI_C_SEEN]=gi_word(GI_H_FRAME);
                // Split (its hot record's last word, in the line the key read), and valid or an
                // orphan: a found cell was traced the frame it was made, so one without a value
                // has no query point.
                var st=select(GI_LS_ORPHAN,GI_LS_VALID,(gi_word(gi_hot(slot))&GI_VALID)!=0u);
                if (gi_word(gi_hot(slot)+15u)&GI_SPLIT_HOT)!=0u { st|=GI_LS_SPLIT; }
                gi_local_state[i]=st;
                if (st&(GI_LS_SPLIT|GI_LS_ORPHAN))!=0u { gi_local_any_flag=1u; }
                continue;
            }
            // Tiles share cells: the frame's request table lists each missing cell once and
            // keeps the highest generation band that wants it (pixels' over gathers'; the
            // cleared table's zero is no request). Any listing order gives the same set.
            let entry=gi_request_entry(key.x);
            if entry.x==0xffffffffu { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); continue; }
            atomicMax(&gi_work[gi_frame.requests.x+entry.x*GI_REQUEST_WORDS+1u],GI_BAND_PIXEL);
            if !gi_request_list(entry.x,gi_key(def.x&0xffu,def.x>>8u,bitcast<vec3<i32>>(def.yzw)).y) { continue; }
            let at=atomicAdd(&gi_work[GI_W_MISSES],1u);
            if at>=capacity { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); continue; }
            let base=gi_frame.offsets.y+at*GI_MISS_WORDS;
            atomicStore(&gi_work[base],def.x);
            atomicStore(&gi_work[base+1u],def.y);
            atomicStore(&gi_work[base+2u],def.z);
            atomicStore(&gi_work[base+3u],def.w);
            atomicStore(&gi_work[base+4u],entry.x);
            atomicStore(&gi_work[base+5u],GI_UNCLAIMED);
        }
        begin=to;
        workgroupBarrier();
        // Pixels walk their open chains. Tiles with no split or orphan cell skip this (a
        // uniform flag).
        if round+1u<GI_PROBE_ROUNDS && workgroupUniformLoad(&gi_local_any_flag)!=0u {
            if gi_stats && lane==0u && round==0u { atomicAdd(&gi_work[GI_W_P_REFINE_TILES],1u); }
            for(var b=0u;b<6u;b+=1u) {
                if (bins&(1u<<b))==0u { continue; }
                if (walked&(1u<<(2u*b)))==0u && gi_pixel_walk(world,pick.lo,b,round,levels) {
                    walked|=1u<<(2u*b);
                }
                if pick.hi!=pick.lo && (walked&(2u<<(2u*b)))==0u && gi_pixel_walk(world,pick.hi,b,round,levels) {
                    walked|=2u<<(2u*b);
                }
            }
            workgroupBarrier();
        }
    }
    if stats { gi_probe_stats(lane,record.x!=0u); }
}
// Adds a tile's probe statistics to the work counters (GI_F_STATS frames only, a uniform
// condition: the header flag).
fn gi_probe_stats(lane:u32, covered:bool) {
    atomicAdd(&gi_wg_stats[0],gi_n_inserts);
    atomicAdd(&gi_wg_stats[1],gi_n_steps);
    atomicAdd(&gi_wg_stats[2],gi_n_pixel);
    atomicAdd(&gi_wg_stats[3],gi_n_parent);
    atomicAdd(&gi_wg_stats[4],gi_n_refine);
    atomicAdd(&gi_wg_stats[5],gi_n_finds);
    atomicAdd(&gi_wg_stats[6],gi_n_found);
    atomicAdd(&gi_wg_stats[7],gi_n_walks);
    if covered { atomicAdd(&gi_wg_stats[8],1u); }
    workgroupBarrier();
    if lane!=0u { return; }
    let cells=atomicLoad(&gi_local_count);
    if cells==0u { return; }
    atomicAdd(&gi_work[GI_W_P_TILES],1u);
    atomicAdd(&gi_work[GI_W_P_PIXELS],atomicLoad(&gi_wg_stats[8]));
    atomicAdd(&gi_work[GI_W_P_INSERTS],atomicLoad(&gi_wg_stats[0]));
    atomicAdd(&gi_work[GI_W_P_STEPS],atomicLoad(&gi_wg_stats[1]));
    atomicAdd(&gi_work[GI_W_P_CELLS],atomicLoad(&gi_wg_stats[2]));
    atomicAdd(&gi_work[GI_W_P_PARENTS],atomicLoad(&gi_wg_stats[3]));
    atomicAdd(&gi_work[GI_W_P_REFINED],atomicLoad(&gi_wg_stats[4]));
    atomicAdd(&gi_work[GI_W_P_FINDS],atomicLoad(&gi_wg_stats[5]));
    atomicAdd(&gi_work[GI_W_P_FOUND],atomicLoad(&gi_wg_stats[6]));
    atomicAdd(&gi_work[GI_W_P_WALKS],atomicLoad(&gi_wg_stats[7]));
    atomicMax(&gi_work[GI_W_P_MAX],cells);
    let bucket=select(select(select(3u,2u,cells<GI_LOCAL),1u,cells<64u),0u,cells<32u);
    atomicAdd(&gi_work[GI_W_P_HIST+bucket],1u);
}
// Request-table entries: key a, the highest generation band that wants it, and the highest
// inverted key b among the cells requested under it (gi_request_list).
const GI_REQUEST_WORDS:u32=3u;
// The request-table entry of key a (linear probing; zero is empty), and whether this
// call inserted it. (0xffffffff, 0) when the table is full.
fn gi_request_entry(key:u32) -> vec2<u32> {
    let mask=gi_frame.requests.y;
    var h=key&mask;
    for(var j=0u;j<64u;j+=1u) {
        let r=atomicCompareExchangeWeak(&gi_work[gi_frame.requests.x+h*GI_REQUEST_WORDS],0u,key);
        if r.exchanged { return vec2<u32>(h,1u); }
        if r.old_value==key { return vec2<u32>(h,0u); }
        if r.old_value!=0u { h=(h+1u)&mask; }
    }
    return vec2<u32>(0xffffffffu,0u);
}
// Whether a request of the cell with key b `key_b` under `entry` lists a miss. Different cells
// can share key a (31 bits: a cut's ~90k new keys meet about twice a frame), and the table
// holds one of them. The entry keeps the highest inverted key b of the cells requested under
// it, and a request lists a miss only when it raises that value; the claim takes only the miss
// whose key b is the final value (gi_miss_chosen). So the same cell is inserted whatever the
// thread order (G2's listing-order choice made tables, and once an image, differ across
// processes: docs/m7-g2b.md). The same cell requested again lists nothing.
fn gi_request_list(entry:u32, key_b:u32) -> bool {
    let v=max(~key_b,1u);
    return atomicMax(&gi_work[gi_frame.requests.x+entry*GI_REQUEST_WORDS+2u],v)<v;
}
// Generation bands of a request (the request table keeps the highest that wants a cell):
// pixels' cells, cells a pixel-made cell's gather requests (made secondary), and cells a
// secondary cell's gather requests (made tertiary; they request nothing). 0 is no request.
const GI_BAND_PIXEL:u32=3u;
const GI_BAND_SECONDARY:u32=2u;
const GI_BAND_TERTIARY:u32=1u;
// A listed miss's generation band.
fn gi_miss_band(base:u32) -> u32 {
    return atomicLoad(&gi_work[gi_frame.requests.x+atomicLoad(&gi_work[base+4u])*GI_REQUEST_WORDS+1u]);
}
// Whether a listed miss is its entry's chosen cell (gi_request_list).
fn gi_miss_chosen(base:u32) -> bool {
    let v=max(~gi_miss_key(base).y,1u);
    return v==atomicLoad(&gi_work[gi_frame.requests.x+atomicLoad(&gi_work[base+4u])*GI_REQUEST_WORDS+2u]);
}
// A listed miss's claim value (higher wins a slot): its band in the top two bits, then the
// bits of key a that do not pick its bucket (those above the slot mask, and the three that
// pick the start offset within it). Keys of one bucket differ in exactly those bits, so no
// two misses that can meet at a slot tie, and exactly one wins it whatever the thread order.
// (G2's gather requests from one cell shared a priority, and two could both write a slot:
// tables then differed across processes, docs/m7-g2b.md.) The rank is fixed by the key
// alone, as the start slot is: it orders inserts within a frame's claim, never which rays are
// cast (W2). It fits below bit 30 for any table of at least 16 slots.
fn gi_miss_claim(base:u32) -> u32 {
    let key=gi_miss_key(base).x;
    let s=countOneBits(gi_word(GI_H_MASK));
    return (gi_miss_band(base)<<30u)|((key>>s)<<3u)|(key&(GI_BUCKET-1u));
}

// ---- claim and resolve: deterministic inserts, in rounds ----
fn gi_miss_key(base:u32) -> vec2<u32> {
    let def=atomicLoad(&gi_work[base]);
    let cell=bitcast<vec3<i32>>(vec3<u32>(atomicLoad(&gi_work[base+1u]),atomicLoad(&gi_work[base+2u]),atomicLoad(&gi_work[base+3u])));
    return gi_key(def&0xffu,def>>8u,cell);
}
fn gi_evictable(slot:u32) -> bool {
    if gi_table[gi_hot(slot)]==0u { return true; }
    return gi_word(GI_H_FRAME)-gi_table[gi_cold(slot)+GI_C_SEEN]>gi_word(GI_H_EVICT);
}
// Listed misses, as a grid-stride loop count.
fn gi_miss_count() -> u32 { return min(atomicLoad(&gi_work[GI_W_MISSES]),gi_frame.counts.w); }
@compute @workgroup_size(64) fn gi_claim(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=gi_miss_count();
    for(var i=id.x;i<count;i+=groups.x*64u) { gi_claim_miss(i); }
}
// The claim period P for this frame's listed misses (1 without a cap; gi_claim).
fn gi_claim_period() -> u32 {
    let cap=gi_word(GI_H_CLAIM_CELLS);
    var p=1u;
    if cap==0u { return p; }
    let count=gi_miss_count();
    while p<64u && count>cap*p { p*=2u; }
    return p;
}
fn gi_claim_miss(index:u32) {
    let base=gi_frame.offsets.y+index*GI_MISS_WORDS;
    if atomicLoad(&gi_work[base+5u])==GI_CREATED { return; }
    // Outside this frame's key phase (GI_H_CLAIM_CELLS): left for a later frame.
    let period=gi_claim_period();
    if period>1u && gi_mix(gi_miss_key(base).x)%period!=gi_word(GI_H_FRAME)%period {
        atomicStore(&gi_work[base+5u],GI_CREATED);
        atomicAdd(&gi_work[GI_W_CLAIM_DEFERRED],1u);
        return;
    }
    // A cell another cell with the same key a superseded this frame: never claims.
    if !gi_miss_chosen(base) {
        atomicStore(&gi_work[base+5u],GI_CREATED);
        return;
    }
    let key=gi_miss_key(base);
    let mask=gi_word(GI_H_MASK);
    let bucket=(key.x&mask)&~(GI_BUCKET-1u);
    for(var i=0u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((gi_start(key)+i)&(GI_BUCKET-1u));
        if gi_evictable(slot) {
            atomicStore(&gi_work[base+5u],slot);
            atomicMax(&gi_work[GI_W_CLAIMS+slot],gi_miss_claim(base));
            return;
        }
    }
    atomicStore(&gi_work[base+5u],GI_UNCLAIMED);
    atomicAdd(&gi_work[GI_W_FULL],1u);
}
@compute @workgroup_size(64) fn gi_resolve(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=gi_miss_count();
    for(var i=id.x;i<count;i+=groups.x*64u) { gi_resolve_miss(i); }
}
// The last round also empties the frame's request table: each listed miss clears its
// own entry once it no longer reads it, so the table is clean for the next frame
// without clearing all of it. (Entries a full miss list could not list are cleared by
// gi_finish.)
@compute @workgroup_size(64) fn gi_resolve_last(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=gi_miss_count();
    for(var i=id.x;i<count;i+=groups.x*64u) {
        gi_resolve_miss(i);
        let entry=gi_frame.requests.x+atomicLoad(&gi_work[gi_frame.offsets.y+i*GI_MISS_WORDS+4u])*GI_REQUEST_WORDS;
        atomicStore(&gi_work[entry],0u);
        atomicStore(&gi_work[entry+1u],0u);
        atomicStore(&gi_work[entry+2u],0u);
    }
}
fn gi_resolve_miss(index:u32) {
    let base=gi_frame.offsets.y+index*GI_MISS_WORDS;
    let slot=atomicLoad(&gi_work[base+5u]);
    if slot>=GI_CREATED { return; }
    if atomicLoad(&gi_work[GI_W_CLAIMS+slot])!=gi_miss_claim(base) {
        atomicAdd(&gi_work[GI_W_LOST],1u);
        return;
    }
    // Losers read this clear as a lower claim, so clearing first is safe.
    atomicStore(&gi_work[GI_W_CLAIMS+slot],0u);
    atomicStore(&gi_work[base+5u],GI_CREATED);
    let key=gi_miss_key(base);
    let cold=gi_cold(slot);
    gi_table[gi_hot(slot)]=key.x;
    // No split flag, bounce value or outgoing radiance from the slot's last occupant.
    gi_table[gi_hot(slot)+15u]=0u;
    gi_table[gi_hot(slot)+GI_HOT_BOUNCE]=0u;
    gi_outgoing[slot]=vec4<f32>(0.0);
    gi_table[cold+GI_C_KEY_B]=key.y;
    // Cells only gathers requested (bands below the pixels') are secondary, or tertiary
    // when a secondary cell's gather requested them (gi_gather).
    let band=gi_miss_band(base);
    let generation=select(select(0u,GI_SECONDARY,band<GI_BAND_PIXEL),GI_SECONDARY|GI_TERTIARY,band<GI_BAND_SECONDARY);
    gi_table[cold+GI_C_STATE]=GI_QUEUED|generation;
    gi_table[cold+GI_C_SEEN]=gi_word(GI_H_FRAME);
    gi_table[cold+GI_C_DEF]=atomicLoad(&gi_work[base]);
    for(var i=0u;i<3u;i+=1u) { gi_table[cold+GI_C_CELL+i]=atomicLoad(&gi_work[base+1u+i]); }
    for(var w=0u;w<GI_MAX_WORDS;w+=1u) {
        gi_table[cold+GI_C_KNOWN+w]=0u;
        gi_table[cold+GI_C_NEAR+w]=0u;
    }
    gi_table[cold+GI_C_MATERIAL]=0u;
    gi_table[cold+GI_C_PRIMITIVE]=GI_NO_PRIMITIVE;
    gi_table[cold+GI_C_BARY]=0u;
    gi_table[cold+GI_C_LIGHTS]=0u;
    gi_queue_trace(slot);
    atomicAdd(&gi_work[GI_W_CREATED],1u);
}
// Queue appends stay within the slot-sized queues: a slot is queued at most once per
// frame (GI_QUEUED), so neither can overflow.
fn gi_queue_trace(slot:u32) {
    let at=atomicAdd(&gi_work[GI_W_TRACES],1u);
    if at<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.offsets.z+at],slot); }
}
fn gi_queue_integrate(slot:u32) {
    let at=atomicAdd(&gi_work[GI_W_INTEGRATES],1u);
    if at<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.offsets.w+at],slot); }
}
// ---- bounce scheduling (G2) ----
// Whether this frame's program reflects light, so gathers and cell shading run.
fn gi_bounce_on() -> bool { return gi_word(GI_H_BOUNCE)!=0u; }
// Queues a cell for this frame's cell shading once (GI_SHADE, cleared by gi_light). Every
// caller handles the cell in one thread.
fn gi_queue_shade(slot:u32) {
    let cold=gi_cold(slot);
    let state=gi_table[cold+GI_C_STATE];
    if (state&GI_SHADE)!=0u { return; }
    gi_table[cold+GI_C_STATE]=state|GI_SHADE;
    let at=atomicAdd(&gi_work[GI_W_SHADES],1u);
    if at<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.bounce.w+at],slot); }
}
// Schedules at least `gathers` gathers of a cell on the next frames, queueing it for next
// frame's once (GI_BQUEUED). An event reaching the cell sets the schedule; it is not a count
// of samples (W1, C8), and no cell is compared with another (W2).
fn gi_schedule(slot:u32, gathers:u32) {
    if gathers==0u { return; }
    let cold=gi_cold(slot);
    let state=gi_table[cold+GI_C_STATE];
    let left=max((state&GI_GATHER_MASK)>>GI_GATHER_SHIFT,min(gathers,7u));
    var next=(state&~GI_GATHER_MASK)|(left<<GI_GATHER_SHIFT);
    if (state&GI_BQUEUED)==0u {
        next|=GI_BQUEUED;
        let at=atomicAdd(&gi_work[gi_frame.bounce.y],1u);
        if at<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.bounce.y+1u+at],slot); }
        atomicAdd(&gi_work[GI_W_BQUEUE],1u);
    }
    gi_table[cold+GI_C_STATE]=next;
}

// ---- the fixed directions of one cell ----
struct GiCell { level:u32, bin:u32, cell:vec3<i32>, centre:vec3<f32>, edge:f32, key:vec2<u32> };
fn gi_cell(slot:u32) -> GiCell {
    let cold=gi_cold(slot);
    let def=gi_table[cold+GI_C_DEF];
    let cell=bitcast<vec3<i32>>(vec3<u32>(gi_table[cold+GI_C_CELL],gi_table[cold+GI_C_CELL+1u],gi_table[cold+GI_C_CELL+2u]));
    let level=def&0xffu;
    let bin=def>>8u;
    let edge=gi_edge(level);
    // The cube's lattice corner sits a quarter cell against the bin (gi_lattice).
    let shift=gi_axis_mask(bin)*select(0.25,-0.25,(bin&1u)!=0u);
    return GiCell(level,bin,cell,(vec3<f32>(cell)+vec3<f32>(0.5)+shift)*edge,edge,
        vec2<u32>(gi_table[gi_hot(slot)]&GI_KEY_BITS,gi_table[cold+GI_C_KEY_B]));
}
// Direction k of GI_DIRECTIONS: equal solid angles of the hemisphere about the
// query normal, optionally rotated about it by an angle the key fixes.
fn gi_direction(frame:mat3x3<f32>, key:vec2<u32>, k:u32) -> vec3<f32> {
    let z=1.0-(f32(k)+0.5)/f32(GI_DIRECTIONS);
    let r=sqrt(max(1.0-z*z,0.0));
    let rotation=select(0.0,f32(key.y>>8u)*(6.283185307179586/16777216.0),(gi_word(GI_H_FLAGS)&GI_F_ROTATE)!=0u);
    let phi=f32(k)*2.399963229728653+rotation;
    return frame*vec3<f32>(r*cos(phi),r*sin(phi),z);
}
fn gi_pack_normal(n:vec3<f32>) -> u32 {
    let p=n.xy/(abs(n.x)+abs(n.y)+abs(n.z));
    let q=select((vec2<f32>(1.0)-abs(p.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),p>=vec2<f32>(0.0)),p,n.z>=0.0);
    return pack2x16snorm(q);
}
// How far a cell's rays start off its query point along its axis: at least the reference
// path tracer's offset (1e-4 + 4e-7 |p|max, reference_path_tracer.wgsl), which scan micro-relief
// needs (G2: the smaller ulp-scaled offset blocked rays the reference's escape; quarry TR1 RMS
// 7.56 to 7.35 F0 codes).
fn gi_origin_offset(q:vec3<f32>, edge:f32) -> f32 {
    let m=max(max(abs(q.x),abs(q.y)),abs(q.z));
    let ulp=bitcast<f32>(bitcast<u32>(max(m,1e-30))+1u)-max(m,1e-30);
    return max(max(1e-5,32.0*ulp)+1e-4*edge,1e-4+4e-7*m);
}

// ---- SH of the occluded sky over the fixed directions ----
// A prepared map's radiance toward d; a constant sky caches unit radiance, and the lookup
// scales by the current constant, so a constant-sky change needs no re-integration.
fn gi_sky(d:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 { return environment_radiance(environment_direction(d),GI_ENV_ROUGHNESS); }
    return vec3<f32>(1.0);
}
// One lane's or cell's SH sums (gi_lane_add, gi_lane_reduce); entry 9 is the same
// directions' unoccluded sum, as L2 irradiance at the cell normal.
var<private> gi_cell_sh:array<vec3<f32>,10>;
// The sum replaces the slot's SH, stored as halves of one f32 scale: no blend with
// any earlier value (W1).
// A map's SH is first scaled per channel so that its unoccluded value at the cell normal is
// the prepared diffuse irradiance there: 64 hemisphere directions of a map's prefiltered
// radiance miss or blur a bright sun's lobe (G1c: open quarry ground read 11% dark with every
// direction visible), while the prepared irradiance is exact. A constant sky needs no
// scale: the directions integrate the L2 kernel to pi exactly.
fn gi_store_sh(slot:u32) {
    if lighting.map.w!=0.0 {
        let n=gi_unpack_normal(gi_table[gi_cold(slot)+GI_C_NORMAL]);
        let exact=environment_bilinear(environment_diffuse,environment_direction(n),0,0);
        let open=gi_cell_sh[9];
        // Guarded: a near-black or degenerate sum keeps its own value.
        let s=select(vec3<f32>(1.0),clamp(exact/max(open,vec3<f32>(1e-30)),vec3<f32>(0.25),vec3<f32>(4.0)),
            open>vec3<f32>(1e-20));
        for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]*=s; }
    }
    var m=0.0;
    for(var i=0u;i<9u;i+=1u) {
        let c=abs(gi_cell_sh[i]);
        m=max(m,max(c.x,max(c.y,c.z)));
    }
    let scale=max(m,1e-30);
    let hot=gi_hot(slot);
    for(var w=0u;w<14u;w+=1u) {
        let a=gi_sh_component(2u*w);
        let b=select(0.0,gi_sh_component(min(2u*w+1u,26u)),2u*w+1u<27u);
        gi_table[hot+2u+w]=pack2x16float(vec2<f32>(a,b)/scale);
    }
    // The last word's free half carries the split flag.
    let split=(gi_table[gi_cold(slot)+GI_C_STATE]&GI_SPLIT)!=0u;
    gi_table[hot+15u]=gi_table[hot+15u]|select(0u,GI_SPLIT_HOT,split);
    gi_table[hot+1u]=bitcast<u32>(scale);
    // The query normal beside the value, for shading's agreement weight (G2).
    gi_table[hot+GI_HOT_NORMAL]=gi_table[gi_cold(slot)+GI_C_NORMAL];
}
fn gi_sh_component(j:u32) -> f32 {
    let v=gi_cell_sh[j/3u];
    let c=j%3u;
    return select(select(v.z,v.y,c==1u),v.x,c==0u);
}

// ---- trace: canonical query point, fixed directions, replace ----
const GI_PROBES:u32=5u;
// Hits a probe line follows through its cube.
const GI_LINE_HITS:u32=4u;
// A deterministic probe from the key alone (W2): five fixed lines from fixed points on the
// cube's face along its bin normal, straight down through the cube. A line's first hit is
// its candidate when its viewed normal lies within about 70 degrees of the bin normal (not
// only in the bin itself: scan triangles are about a pixel wide and their plane normals
// scatter across bins), it lies in the cube, and a ray from it along its normal does not
// leave through the inside of a closed surface. When that first hit lies inside a solid
// (the face starts inside a slab), the line's next qualifying hit is tried (the floor
// beneath the slab).
// A candidate: its hit point (q.w 1), viewed normal (n.w 1: inside a solid), the hit
// triangle's positions (tri.xyz) and instance, with bit 31 when it is seen from behind its
// source winding (tri.w), and its barycentrics: the near field's inputs (G1c).
struct GiCandidate { q:vec4<f32>, n:vec4<f32>, tri:vec4<u32>, bary:vec2<f32> };
fn gi_no_candidate() -> GiCandidate {
    return GiCandidate(vec4<f32>(0.0),vec4<f32>(0.0),vec4<u32>(0u),vec2<f32>(0.0));
}
// A line's first two qualifying hits and whether it is split: it meets two surface layers,
// two faces toward the bin (by source winding) with open space between them, more than an
// eighth of an edge apart (a slab over a floor, a sheet over the ground). A box resting on
// the ground is one layer: its bottom and the ground coincide.
struct GiLineHits { a:GiCandidate, b:GiCandidate, split:bool };
fn gi_probe_line(c:GiCell, j:u32) -> GiLineHits {
    let nb=gi_bin_normal(c.bin);
    let axis=c.bin>>1u;
    let t1=gi_bin_normal(((axis+1u)%3u)*2u);
    let t2=gi_bin_normal(((axis+2u)%3u)*2u);
    var o=vec2<f32>(0.0);
    if j>0u { o=vec2<f32>(select(-0.25,0.25,(j&1u)!=0u),select(-0.25,0.25,j<3u)); }
    // The segment overhangs both faces by a margin, so a surface lying exactly on a
    // face is found by both cells beside it.
    let margin=0.01*c.edge;
    let start=c.centre+nb*(0.5*c.edge+margin)+(t1*o.x+t2*o.y)*c.edge;
    let length=c.edge+2.0*margin;
    var out=GiLineHits(gi_no_candidate(),gi_no_candidate(),false);
    var found=0u;
    var layers=0u;
    var last=0.0;
    var tmin=0.0;
    for(var i=0u;i<GI_LINE_HITS;i+=1u) {
        let h=gi_closest(start,-nb,tmin,length);
        if !h.hit { break; }
        let facing=dot(h.normal,nb);
        // Candidates: the first hit when it qualifies, then the next hit facing the bin past
        // an open gap (the floor beneath a slab whose inside the line started in).
        let second=found==1u && facing>=0.34 && h.t-last>0.01*c.edge;
        if (i==0u && abs(facing)>=0.34) || second {
            let n=select(-h.normal,h.normal,facing>=0.0);
            let side=select(0u,0x80000000u,facing<0.0);
            let r=GiCandidate(vec4<f32>(start-nb*h.t,1.0),vec4<f32>(n,0.0),vec4<u32>(h.tri,h.inst|side),h.bary);
            if found==0u { out.a=r; } else { out.b=r; }
            found+=1u;
        }
        if facing>=0.34 && (layers==0u || h.t-last>0.125*c.edge) { layers+=1u; }
        last=h.t;
        if layers>=2u && (found==2u || out.a.q.w==0.0) { break; }
        tmin=h.t+1e-4*c.edge;
    }
    out.split=layers>=2u;
    return out;
}
// The line's candidate: its first when it is not inside a solid, else its second when
// that is not; otherwise its first, marked inside (n.w 1), or none. One inside-test site in
// a loop: every inlined ray query adds to gi_setup's registers (G1c measured each extra
// site costing setup time even where it never ran).
fn gi_line_candidate(h:GiLineHits, edge:f32) -> GiCandidate {
    if h.a.q.w==0.0 { return h.a; }
    for(var k=0u;k<2u;k+=1u) {
        var r=h.a;
        if k==1u { r=h.b; }
        if r.q.w==0.0 { break; }
        if !gi_probe_inside(r,edge) { return r; }
    }
    var r=h.a;
    r.n.w=1.0;
    return r;
}
// Whether a candidate lies inside a solid: a ray from it along its normal first meets a
// hit seen from the back of its source winding. A candidate the line met from behind (the
// line started inside a solid) is inside on that alone, as at G1b. One it met from the
// front is inside when that back face lies within an edge (a scan buried just under an
// overlapping one, gravel under a stone), or further when each of four fixed rays 80
// degrees off the normal meets a back face too: a point inside a closed solid (the ground
// under a block resting on it) is enclosed on every side, while a floor under an open,
// single-sided scan sheet sees the sheet's back overhead but has a way out to the side, and
// it is a receiver (the quarry's cavity, G1c). Where two coincident faces tie (a block's
// bottom on the ground), hardware and software rays may report either first; both give the
// same answer, enclosed.
fn gi_probe_inside(r:GiCandidate, edge:f32) -> bool {
    let q=r.q.xyz;
    let n=r.n.xyz;
    let o=q+n*gi_origin_offset(q,edge);
    let f=gi_basis(n);
    // Ray 0 along the normal, rays 1-4 at 80 degrees off it, through one query site.
    for(var i=0u;i<5u;i+=1u) {
        let a=f32(i)*1.5707963267948966;
        let d=select(f*vec3<f32>(0.9848078*cos(a),0.9848078*sin(a),0.17364818),n,i==0u);
        let h=gi_closest(o,d,0.0,1e20);
        if !h.hit || dot(h.normal,d)<=0.0 { return false; }
        if i==0u && ((r.tri.w&0x80000000u)!=0u || h.t<=edge) { return true; }
    }
    return true;
}
// Directions a queued cell traces this frame, in word w: every unknown one (replace
// mode: new cells, and the directions a moved caster reached), or this frame's phase
// (W1 form 2).
fn gi_wanted(known:u32, w:u32, phases:u32, phase:u32) -> u32 {
    if phases==1u { return ~known; }
    return gi_group_mask(w,phases,phase);
}
// Word w of the direction group k % groups == g.
fn gi_group_mask(w:u32, groups:u32, g:u32) -> u32 {
    var m=0u;
    for(var b=0u;b<32u;b+=1u) {
        if (w*32u+b)%groups==g { m|=1u<<b; }
    }
    return m;
}
// Direction k of the unrotated set in the cell's own frame (the key's rotation turns
// every direction alike, so it keeps which directions are nearest).
fn gi_local_direction(k:u32) -> vec3<f32> {
    let z=1.0-(f32(k)+0.5)/f32(GI_DIRECTIONS);
    let r=sqrt(max(1.0-z*z,0.0));
    let phi=f32(k)*2.399963229728653;
    return vec3<f32>(r*cos(phi),r*sin(phi),z);
}
// The traced direction nearest the untraced direction k (lowest index on a tie), given
// the cell's traced masks, or GI_DIRECTIONS when none is traced. An untraced direction
// takes that direction's visibility bit: a fixed fill of the per-direction slots (W1
// form 2), never a count or an average.
// Each direction's eight nearest directions of the unrotated set, nearest first (ties to
// the lower index), four 8-bit indices a word; src/gi_cache.rs neighbour_table writes it
// for GI_DIRECTIONS. The fill tries them before searching every direction.
const GI_NEIGHBOURS:array<u32,256>=array<u32,256>();
fn gi_known_bit(j:u32, k0:u32, k1:u32, k2:u32, k3:u32) -> bool {
    let w=j/32u;
    return (select(select(select(k3,k2,w==2u),k1,w==1u),k0,w==0u)&(1u<<(j%32u)))!=0u;
}
fn gi_nearest_known(k:u32, k0:u32, k1:u32, k2:u32, k3:u32) -> u32 {
    for(var i=0u;i<8u;i+=1u) {
        let at=k*8u+i;
        let j=(GI_NEIGHBOURS[at/4u]>>(8u*(at%4u)))&0xffu;
        if gi_known_bit(j,k0,k1,k2,k3) { return j; }
    }
    let d=gi_local_direction(k);
    var best=GI_DIRECTIONS;
    var best_dot=-2.0;
    for(var j=0u;j<GI_DIRECTIONS;j+=1u) {
        let w=j/32u;
        let words=select(select(select(k3,k2,w==2u),k1,w==1u),k0,w==0u);
        if (words&(1u<<(j%32u)))==0u { continue; }
        let c=dot(d,gi_local_direction(j));
        if c>best_dot { best_dot=c; best=j; }
    }
    return best;
}
// Whether direction k is open given the cell's traced masks k0..k3 and bits b0..b3: its
// own bit when traced, else the nearest traced direction's bit, else open sky.
fn gi_open(k:u32, k0:u32, k1:u32, k2:u32, k3:u32, b0:u32, b1:u32, b2:u32, b3:u32) -> bool {
    var j=k;
    let w=k/32u;
    if (select(select(select(k3,k2,w==2u),k1,w==1u),k0,w==0u)&(1u<<(k%32u)))==0u {
        j=gi_nearest_known(k,k0,k1,k2,k3);
        if j==GI_DIRECTIONS { return true; }
    }
    let v=j/32u;
    return (select(select(select(b3,b2,v==2u),b1,v==1u),b0,v==0u)&(1u<<(j%32u)))!=0u;
}
fn gi_point(cold:u32) -> vec3<f32> {
    return vec3<f32>(bitcast<f32>(gi_table[cold+GI_C_POINT]),bitcast<f32>(gi_table[cold+GI_C_POINT+1u]),bitcast<f32>(gi_table[cold+GI_C_POINT+2u]));
}
/// Eight lanes per queued cell, eight cells per workgroup: the canonical query point
// when the cell has none, its near-field mask bits, then one ray item per direction to trace
// (gi_rays), contiguous per cell in direction order. A fresh cell's lanes 0-4 walk the
// five probe lines and resolve their candidates together, each with its own inside tests
// (every line also decides the split). Lane 0 takes the first point in line order (the same
// point as trying them in turn). A point on an instance with cooked near-field masks then has each
// lane mark the directions the masks block. The lanes then write the items, or past the item
// capacity trace the cell's directions themselves: the same rays and bits either way.
const GI_SETUP_LANES:u32=8u;
const GI_SETUP_CELLS:u32=8u;
var<workgroup> gi_wg_candidates:array<GiCandidate,64>;
// Per cell: the setup state (GI_SETUP_*), the first item, the pending words and the bits
// traced past the capacity.
const GI_SETUP_NONE:u32=0u;
const GI_SETUP_PROBE:u32=1u;
const GI_SETUP_ITEMS:u32=2u;
const GI_SETUP_TRACE:u32=3u;
const GI_SETUP_NEAR:u32=5u;
var<workgroup> gi_wg_state:array<u32,GI_SETUP_CELLS>;
var<workgroup> gi_wg_base:array<u32,GI_SETUP_CELLS>;
var<workgroup> gi_wg_pending:array<u32,GI_SETUP_CELLS*GI_MAX_WORDS>;
var<workgroup> gi_wg_traced:array<atomic<u32>,GI_SETUP_CELLS*GI_MAX_WORDS>;
// Per cell: some probe line is split; and the directions the near-field masks block.
var<workgroup> gi_wg_split:array<atomic<u32>,GI_SETUP_CELLS>;
var<workgroup> gi_wg_near:array<atomic<u32>,GI_SETUP_CELLS*GI_MAX_WORDS>;
// A cell's chosen candidate sits in lane 7's slot, which no probe line uses.
const GI_SETUP_CHOSEN:u32=7u;
@compute @workgroup_size(64) fn gi_setup(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) index:u32) {
    let cell=index/GI_SETUP_LANES;
    let lane=index%GI_SETUP_LANES;
    let count=gi_uniform_count(index,GI_W_TRACES);
    let bins=gi_word(GI_H_NEAR_BINS);
    if lane==0u {
        atomicStore(&gi_wg_split[cell],0u);
        for(var w=0u;w<GI_MAX_WORDS;w+=1u) { atomicStore(&gi_wg_near[cell*GI_MAX_WORDS+w],0u); }
    }
    workgroupBarrier();
    for(var first=group.x*GI_SETUP_CELLS;first<count;first+=groups.x*GI_SETUP_CELLS) {
        let live=first+cell<count;
        var slot=0u;
        if live { slot=atomicLoad(&gi_work[gi_frame.offsets.z+first+cell]); }
        let c=gi_cell(slot);
        let cold=gi_cold(slot);
        // A fresh cell (nothing known) has no query point yet.
        var fresh=live;
        for(var w=0u;w<GI_WORDS;w+=1u) { fresh=fresh && gi_table[cold+GI_C_KNOWN+w]==0u; }
        // Stage 1: lines 0-4 walk their cube and resolve their candidates together.
        if fresh && lane<GI_PROBES {
            let line=gi_probe_line(c,lane);
            if line.split { atomicOr(&gi_wg_split[cell],1u); }
            gi_wg_candidates[index]=gi_line_candidate(line,c.edge);
        }
        if live && lane==0u {
            gi_wg_state[cell]=select(GI_SETUP_ITEMS,GI_SETUP_PROBE,fresh);
        }
        workgroupBarrier();
        // Stage 3: lane 0 takes the first point in line order, or marks an orphan.
        if live && lane==0u {
            var state=gi_wg_state[cell];
            if state==GI_SETUP_PROBE {
                let split=atomicLoad(&gi_wg_split[cell])!=0u;
                var qualified=false;
                state=GI_SETUP_NONE;
                for(var j=0u;j<2u*GI_PROBES;j+=1u) {
                    let r=gi_wg_candidates[cell*GI_SETUP_LANES+j%GI_PROBES];
                    qualified=qualified || r.q.w!=0.0;
                    if r.q.w==0.0 || r.n.w!=0.0 { continue; }
                    // Front faces first, in line order; back faces only when no line has one
                    // (G2: a back face's cell sees under its sheet, and the pixels near a
                    // bin tie that read it showed as dark pinholes; docs/m7-g2.md). The point
                    // still comes from the key alone, in a fixed order (W2).
                    if j<GI_PROBES && (r.tri.w&0x80000000u)!=0u { continue; }
                    let near=bins!=0u && gi_near_base(r.tri.w&0x7fffffffu)!=0u;
                    gi_setup_point(slot,r,split);
                    gi_wg_candidates[cell*GI_SETUP_LANES+GI_SETUP_CHOSEN]=r;
                    state=select(GI_SETUP_ITEMS,GI_SETUP_NEAR,near);
                    break;
                }
                if state==GI_SETUP_NONE {
                    gi_table[gi_hot(slot)]=c.key.x;
                    gi_table[gi_hot(slot)+15u]=select(0u,GI_SPLIT_HOT,split);
                    gi_table[gi_hot(slot)+GI_HOT_BOUNCE]=0u;
                    gi_table[cold+GI_C_STATE]=GI_TRACED|GI_ORPHAN|select(0u,GI_SPLIT,split)
                        |(gi_table[cold+GI_C_STATE]&(GI_BQUEUED|GI_SECONDARY|GI_TERTIARY|GI_SHADE));
                    atomicAdd(&gi_work[GI_W_ORPHANS],1u);
                    atomicAdd(&gi_work[select(GI_W_ORPHAN_NONE,GI_W_ORPHAN_INSIDE,qualified)],1u);
                }
                if split { atomicAdd(&gi_work[GI_W_SPLITS],1u); }
            }
            gi_wg_state[cell]=state;
        }
        workgroupBarrier();
        // Stage 4: a new point on cooked near-field masks: each lane marks its directions
        // (k = lane, lane+8, ...) that the masks block (G1c).
        if live && gi_wg_state[cell]==GI_SETUP_NEAR {
            let r=gi_wg_candidates[cell*GI_SETUP_LANES+GI_SETUP_CHOSEN];
            let base=gi_near_base(r.tri.w&0x7fffffffu);
            let back=(r.tri.w&0x80000000u)!=0u;
            let frame=gi_basis(r.n.xyz);
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_SETUP_LANES) {
                if gi_near_blocked(base,r.tri.xyz,r.bary,back,gi_direction(frame,c.key,k),bins) {
                    atomicOr(&gi_wg_near[cell*GI_MAX_WORDS+k/32u],1u<<(k%32u));
                }
            }
        }
        workgroupBarrier();
        // Stage 5: lane 0 stores the mask bits, then the pending directions and items.
        if live && lane==0u {
            var state=gi_wg_state[cell];
            if state==GI_SETUP_NEAR {
                for(var w=0u;w<GI_MAX_WORDS;w+=1u) {
                    gi_table[cold+GI_C_NEAR+w]=atomicLoad(&gi_wg_near[cell*GI_MAX_WORDS+w]);
                }
                state=GI_SETUP_ITEMS;
            }
            if state==GI_SETUP_ITEMS {
                let base=gi_setup_directions(slot,cell);
                gi_wg_base[cell]=base;
                if base==0xffffffffu { state=GI_SETUP_TRACE; }
            }
            gi_wg_state[cell]=state;
            for(var w=0u;w<GI_MAX_WORDS;w+=1u) { atomicStore(&gi_wg_traced[cell*GI_MAX_WORDS+w],0u); }
        }
        workgroupBarrier();
        let state=gi_wg_state[cell];
        if live && (state==GI_SETUP_ITEMS || state==GI_SETUP_TRACE) {
            let base=gi_wg_base[cell];
            let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
            let q=gi_point(cold);
            let frame=gi_basis(n);
            let origin=q+n*gi_origin_offset(q,c.edge);
            // This lane's directions k = lane, lane+8, ...
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_SETUP_LANES) {
                let w=k/32u;
                let bit=1u<<(k%32u);
                let pending=gi_wg_pending[cell*GI_MAX_WORDS+w];
                if (pending&bit)==0u { continue; }
                if state==GI_SETUP_ITEMS {
                    // Item base + (pending directions before k).
                    var rank=countOneBits(pending&(bit-1u));
                    for(var v=0u;v<w;v+=1u) { rank+=countOneBits(gi_wg_pending[cell*GI_MAX_WORDS+v]); }
                    atomicStore(&gi_work[gi_frame.requests.z+base+rank],(slot<<7u)|k);
                } else if gi_visible(origin,gi_direction(frame,c.key,k)) {
                    atomicOr(&gi_wg_traced[cell*GI_MAX_WORDS+w],bit);
                }
            }
        }
        workgroupBarrier();
        // Past the capacity: the traced bits into the slots; nothing pending. Lane 0 also
        // clears the cell's split and near words for the next cells.
        if live && lane==0u && state==GI_SETUP_TRACE {
            for(var w=0u;w<GI_WORDS;w+=1u) {
                let m=gi_wg_pending[cell*GI_MAX_WORDS+w];
                let bits=gi_table[cold+GI_C_BITS+w];
                gi_table[cold+GI_C_BITS+w]=(bits&~m)|(atomicLoad(&gi_wg_traced[cell*GI_MAX_WORDS+w])&m);
                gi_table[cold+GI_C_KNOWN+w]=gi_table[cold+GI_C_KNOWN+w]|m;
                gi_table[cold+GI_C_PENDING+w]=0u;
            }
        }
        if lane==0u {
            atomicStore(&gi_wg_split[cell],0u);
            for(var w=0u;w<GI_MAX_WORDS;w+=1u) { atomicStore(&gi_wg_near[cell*GI_MAX_WORDS+w],0u); }
        }
        // The next cells overwrite the workgroup state only after every lane is done.
        workgroupBarrier();
    }
}
// A fresh cell's canonical query point and normal, whether it is split, and the point's
// material (G2: the placement and, where the ray source names it, the hit triangle). Its near
// words are cleared until stage 5 stores them. The bounce bits survive a re-probe.
// The normal, the axis of the cell's fixed directions, is the interpolated vertex normal at
// the point where the ray source names the triangle (hardware rays), turned to the
// candidate's side; otherwise the hit facet's plane normal. On scans a facet's plane tilts
// from the surface around it, and a hemisphere about it dips into the rock: such cells read
// near-black and made isolated dark pixels (G2: quarry all-covered RMS 10.35 -> 9.76 codes,
// pinholes 1,754 -> 1,523 at 1x; docs/m7-g2.md). Fixed by the key alone, as before (W2).
fn gi_setup_point(slot:u32, r:GiCandidate, split:bool) {
    let cold=gi_cold(slot);
    for(var i=0u;i<3u;i+=1u) { gi_table[cold+GI_C_POINT+i]=bitcast<u32>(r.q[i]); }
    var axis=r.n.xyz;
    let primitive=gi_hit_primitive(r.tri.xyz);
    if primitive!=GI_NO_PRIMITIVE {
        let rec=gi_receiver(vec4<u32>((r.tri.w&0x7fffffffu)+1u,primitive,bitcast<vec2<u32>>(r.bary)));
        if dot(rec.vertex,rec.vertex)>1e-12 {
            let v=normalize(rec.vertex);
            axis=select(-v,v,dot(v,r.n.xyz)>=0.0);
        }
    }
    gi_table[cold+GI_C_NORMAL]=gi_pack_normal(axis);
    gi_table[cold+GI_C_STATE]=GI_QUEUED|select(0u,GI_SPLIT,split)
        |(gi_table[cold+GI_C_STATE]&(GI_BQUEUED|GI_SECONDARY|GI_TERTIARY|GI_SHADE));
    for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[cold+GI_C_NEAR+w]=0u; }
    let inst=r.tri.w&0x7fffffffu;
    gi_table[cold+GI_C_MATERIAL]=gi_hit_identity(inst,r.tri.xyz);
    gi_table[cold+GI_C_PRIMITIVE]=gi_hit_primitive(r.tri.xyz);
    gi_table[cold+GI_C_BARY]=pack2x16unorm(r.bary);
}
// Lane 0's part of setup for a cell with a query point: the pending directions (into the
// workgroup's copy too) and the item allocation. Directions its near-field masks block become
// known and blocked, and are never traced. Returns the first item, or 0xffffffff past the
// item capacity (the lanes then trace the directions themselves).
fn gi_setup_directions(slot:u32, cell:u32) -> u32 {
    let cold=gi_cold(slot);
    let phases=max(gi_word(GI_H_PHASES),1u);
    let phase=gi_word(GI_H_FRAME)%phases;
    // Near-blocked directions: static geometry within the near field blocks them whatever
    // moves, so they are known and blocked, before any group or phase is chosen.
    for(var w=0u;w<GI_WORDS;w+=1u) {
        let near=gi_table[cold+GI_C_NEAR+w];
        gi_table[cold+GI_C_KNOWN+w]=gi_table[cold+GI_C_KNOWN+w]|near;
        gi_table[cold+GI_C_BITS+w]=gi_table[cold+GI_C_BITS+w]&~near;
    }
    // Cut refill: the first direction group with untraced directions, one a frame.
    let spread=gi_word(GI_H_SPREAD);
    var group=0xffffffffu;
    if spread>1u {
        for(var g=0u;g<spread && group==0xffffffffu;g+=1u) {
            for(var w=0u;w<GI_WORDS;w+=1u) {
                if (~gi_table[cold+GI_C_KNOWN+w]&gi_group_mask(w,spread,g))!=0u { group=g; }
            }
        }
    }
    var rays=0u;
    for(var w=0u;w<GI_MAX_WORDS;w+=1u) {
        var m=0u;
        if w<GI_WORDS {
            let known=gi_table[cold+GI_C_KNOWN+w];
            m=gi_wanted(known,w,phases,phase)&~gi_table[cold+GI_C_NEAR+w];
            if group!=0xffffffffu { m=~known&gi_group_mask(w,spread,group); }
            gi_table[cold+GI_C_PENDING+w]=m;
        }
        gi_wg_pending[cell*GI_MAX_WORDS+w]=m;
        rays+=countOneBits(m);
    }
    atomicAdd(&gi_work[GI_W_RAYS],rays);
    // Items past the capacity are never written; the rays pass stops at the capacity.
    let capacity=min(gi_word(GI_H_ITEMS),GI_RAY_ITEMS);
    var base=capacity;
    if capacity!=0u { base=atomicAdd(&gi_work[GI_W_ITEMS],rays); }
    if base<capacity && rays<=capacity-base {
        gi_table[cold+GI_C_BASE]=base;
        return base;
    }
    return 0xffffffffu;
}
// One thread per ray item: one direction's visibility from its cell's query point,
// kept as the bit at the item's index (atomicOr: the same bits in any order).
@compute @workgroup_size(64) fn gi_rays(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_ITEMS]),GI_RAY_ITEMS);
    for(var i=id.x;i<count;i+=groups.x*64u) { gi_ray(i); }
}
fn gi_ray(index:u32) {
    let item=atomicLoad(&gi_work[gi_frame.requests.z+index]);
    let slot=item>>7u;
    let cold=gi_cold(slot);
    let edge=gi_edge(gi_table[cold+GI_C_DEF]&0xffu);
    let key=vec2<u32>(gi_table[gi_hot(slot)]&GI_KEY_BITS,gi_table[cold+GI_C_KEY_B]);
    let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
    let q=gi_point(cold);
    let origin=q+n*gi_origin_offset(q,edge);
    if gi_visible(origin,gi_direction(gi_basis(n),key,item&127u)) {
        let staged=gi_frame.requests.z+GI_RAY_ITEMS;
        atomicOr(&gi_work[staged+index/32u],1u<<(index%32u));
    }
}
// ---- finish and integrate: 4 lanes per cell, 16 cells per workgroup ----
// Each lane forms the SH terms of its directions (lane, lane+4, ...) in order; a fixed
// binary tree in workgroup memory sums the 4 lanes (deterministic, no atomics on values),
// and the cell's lane 0 stores the result. The value replaces the stored one (W1).
const GI_SH_LANES:u32=4u;
const GI_SH_CELLS:u32=16u;
// Per cell in the workgroup: 30 components (gi_cell_sh) of GI_SH_LANES lanes.
var<workgroup> gi_wg_sh:array<f32,GI_SH_CELLS*30u*GI_SH_LANES>;
var<workgroup> gi_wg_bits:array<atomic<u32>,GI_SH_CELLS*GI_MAX_WORDS>;
var<workgroup> gi_wg_count:u32;
// Adds direction k's term to this lane's sums (gi_cell_sh): to the SH when it is open, and
// with a map, to the unoccluded sum either way.
fn gi_lane_add(c:GiCell, frame:mat3x3<f32>, k:u32, open:bool) {
    let map=lighting.map.w!=0.0;
    if !open && !map { return; }
    let d=gi_direction(frame,c.key,k);
    let radiance=gi_sky(d)*(6.283185307179586/f32(GI_DIRECTIONS));
    // The unoccluded L2 irradiance at the cell normal: the clamped-cosine kernel through
    // band 2 (gi_band_weight's bands) at z = d.n, 3/32 + z/2 + 15z^2/32 (gi_store_sh).
    if map {
        let z=1.0-(f32(k)+0.5)/f32(GI_DIRECTIONS);
        gi_cell_sh[9]+=radiance*(0.09375+0.5*z+0.46875*z*z);
    }
    if !open { return; }
    gi_sh_basis(d);
    for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]+=radiance*gi_y[i]; }
}
// Sums each cell's lanes' gi_cell_sh into its lane 0, in a fixed tree order.
// Every invocation of the workgroup calls it (it holds barriers).
fn gi_lane_reduce(cell:u32, lane:u32) {
    let at=cell*30u*GI_SH_LANES+lane;
    for(var i=0u;i<10u;i+=1u) {
        gi_wg_sh[at+(i*3u)*GI_SH_LANES]=gi_cell_sh[i].x;
        gi_wg_sh[at+(i*3u+1u)*GI_SH_LANES]=gi_cell_sh[i].y;
        gi_wg_sh[at+(i*3u+2u)*GI_SH_LANES]=gi_cell_sh[i].z;
    }
    workgroupBarrier();
    for(var stride=GI_SH_LANES/2u;stride>0u;stride/=2u) {
        if lane<stride {
            for(var j=0u;j<30u;j+=1u) {
                gi_wg_sh[at+j*GI_SH_LANES]+=gi_wg_sh[at+j*GI_SH_LANES+stride];
            }
        }
        workgroupBarrier();
    }
    if lane==0u {
        for(var i=0u;i<10u;i+=1u) {
            gi_cell_sh[i]=vec3<f32>(gi_wg_sh[at+(i*3u)*GI_SH_LANES],gi_wg_sh[at+(i*3u+1u)*GI_SH_LANES],gi_wg_sh[at+(i*3u+2u)*GI_SH_LANES]);
        }
    }
}
// A queue length as a uniform value: the workgroup's first invocation reads it.
fn gi_uniform_count(index:u32, counter:u32) -> u32 {
    if index==0u { gi_wg_count=min(atomicLoad(&gi_work[counter]),gi_frame.offsets.x); }
    return workgroupUniformLoad(&gi_wg_count);
}
// Each queued cell: its items' bits into the direction slots, then the SH over every
// slot, replacing the value (W1). An untraced direction (W1 form 2, or the cut refill)
// takes the bit of the nearest traced direction until it is traced itself. A fixed grid of workgroups loops over the queue.
@compute @workgroup_size(64) fn gi_finish(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) index:u32) {
    let cell=index/GI_SH_LANES;
    let lane=index%GI_SH_LANES;
    let count=gi_uniform_count(index,GI_W_TRACES);
    let staged=gi_frame.requests.z+GI_RAY_ITEMS;
    for(var first=group.x*GI_SH_CELLS;first<count;first+=groups.x*GI_SH_CELLS) {
        for(var w=lane;w<GI_MAX_WORDS;w+=GI_SH_LANES) { atomicStore(&gi_wg_bits[cell*GI_MAX_WORDS+w],0u); }
        workgroupBarrier();
        var slot=0u;
        var live=first+cell<count;
        if live {
            slot=atomicLoad(&gi_work[gi_frame.offsets.z+first+cell]);
            live=(gi_table[gi_cold(slot)+GI_C_STATE]&GI_ORPHAN)==0u;
        }
        let cold=gi_cold(slot);
        for(var i=0u;i<10u;i+=1u) { gi_cell_sh[i]=vec3<f32>(0.0); }
        // The bits first, every direction's, so untraced directions can read them.
        if live {
            let base=gi_table[cold+GI_C_BASE];
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_SH_LANES) {
                let w=k/32u;
                let bit=1u<<(k%32u);
                let pending=gi_table[cold+GI_C_PENDING+w];
                var visible=(gi_table[cold+GI_C_BITS+w]&bit)!=0u;
                if (pending&bit)!=0u {
                    // This direction's item: the pending directions before it, in order.
                    var rank=countOneBits(pending&(bit-1u));
                    for(var v=0u;v<w;v+=1u) { rank+=countOneBits(gi_table[cold+GI_C_PENDING+v]); }
                    let at=base+rank;
                    visible=(atomicLoad(&gi_work[staged+at/32u])&(1u<<(at%32u)))!=0u;
                }
                if visible { atomicOr(&gi_wg_bits[cell*GI_MAX_WORDS+w],bit); }
            }
        }
        workgroupBarrier();
        if live {
            let c=gi_cell(slot);
            let frame=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
            let at=cell*GI_MAX_WORDS;
            let k0=gi_table[cold+GI_C_KNOWN]|gi_table[cold+GI_C_PENDING];
            let k1=select(0u,gi_table[cold+GI_C_KNOWN+1u]|gi_table[cold+GI_C_PENDING+1u],GI_WORDS>1u);
            let k2=select(0u,gi_table[cold+GI_C_KNOWN+2u]|gi_table[cold+GI_C_PENDING+2u],GI_WORDS>2u);
            let k3=select(0u,gi_table[cold+GI_C_KNOWN+3u]|gi_table[cold+GI_C_PENDING+3u],GI_WORDS>3u);
            let b0=atomicLoad(&gi_wg_bits[at]);let b1=atomicLoad(&gi_wg_bits[at+1u]);
            let b2=atomicLoad(&gi_wg_bits[at+2u]);let b3=atomicLoad(&gi_wg_bits[at+3u]);
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_SH_LANES) {
                gi_lane_add(c,frame,k,gi_open(k,k0,k1,k2,k3,b0,b1,b2,b3));
            }
        }
        gi_lane_reduce(cell,lane);
        if live && lane==0u {
            var all_known=true;
            for(var w=0u;w<GI_WORDS;w+=1u) {
                let known=gi_table[cold+GI_C_KNOWN+w]|gi_table[cold+GI_C_PENDING+w];
                all_known=all_known && known==0xffffffffu;
                gi_table[cold+GI_C_BITS+w]=atomicLoad(&gi_wg_bits[cell*GI_MAX_WORDS+w]);
                gi_table[cold+GI_C_KNOWN+w]=known;
                gi_table[cold+GI_C_PENDING+w]=0u;
            }
            gi_store_sh(slot);
            // The item base depends on allocation order: cleared, so the table is deterministic.
            gi_table[cold+GI_C_BASE]=0u;
            gi_table[cold+GI_C_STATE]=select(GI_TRACED|GI_PARTIAL,GI_TRACED,all_known)|(gi_table[cold+GI_C_STATE]&GI_KEEP);
            gi_table[gi_hot(slot)]=gi_table[gi_hot(slot)]|GI_VALID;
            // Bounce (G2): a traced cell's sky changed, so it is shaded again; once every
            // direction is known its blocked directions are gathered GI_H_GATHERS times.
            if gi_bounce_on() {
                gi_queue_shade(slot);
                if all_known { gi_schedule(slot,gi_word(GI_H_GATHERS)); }
            }
        }
        // Lane 0 of each cell has read its bits before the next cells clear them.
        workgroupBarrier();
    }
    // The last resolve round cleared every listed miss's request entry; after a miss-list
    // overflow, entries nobody listed remain, so the whole request table is cleared (no
    // later pass reads it this frame).
    if atomicLoad(&gi_work[GI_W_MISS_OVERFLOW])!=0u {
        let words=(gi_frame.requests.y+1u)*GI_REQUEST_WORDS;
        for(var i=group.x*64u+index;i<words;i+=groups.x*64u) { atomicStore(&gi_work[gi_frame.requests.x+i],0u); }
    }
}
// Each listed cell's SH again from its stored bits, with no rays (environment changes).
@compute @workgroup_size(64) fn gi_integrate(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) index:u32) {
    let cell=index/GI_SH_LANES;
    let lane=index%GI_SH_LANES;
    let count=gi_uniform_count(index,GI_W_INTEGRATES);
    for(var first=group.x*GI_SH_CELLS;first<count;first+=groups.x*GI_SH_CELLS) {
        let live=first+cell<count;
        var slot=0u;
        if live { slot=atomicLoad(&gi_work[gi_frame.offsets.w+first+cell]); }
        let cold=gi_cold(slot);
        for(var i=0u;i<10u;i+=1u) { gi_cell_sh[i]=vec3<f32>(0.0); }
        if live {
            let c=gi_cell(slot);
            let frame=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
            let k0=gi_table[cold+GI_C_KNOWN];
            let k1=select(0u,gi_table[cold+GI_C_KNOWN+1u],GI_WORDS>1u);
            let k2=select(0u,gi_table[cold+GI_C_KNOWN+2u],GI_WORDS>2u);
            let k3=select(0u,gi_table[cold+GI_C_KNOWN+3u],GI_WORDS>3u);
            let b0=gi_table[cold+GI_C_BITS];
            let b1=select(0u,gi_table[cold+GI_C_BITS+1u],GI_WORDS>1u);
            let b2=select(0u,gi_table[cold+GI_C_BITS+2u],GI_WORDS>2u);
            let b3=select(0u,gi_table[cold+GI_C_BITS+3u],GI_WORDS>3u);
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_SH_LANES) {
                gi_lane_add(c,frame,k,gi_open(k,k0,k1,k2,k3,b0,b1,b2,b3));
            }
        }
        gi_lane_reduce(cell,lane);
        if live && lane==0u { gi_store_sh(slot); }
        workgroupBarrier();
    }
}

// ---- event scan ----
fn gi_box_hit(o:vec3<f32>, d:vec3<f32>, lo:vec3<f32>, hi:vec3<f32>) -> bool {
    let inv=vec3<f32>(1.0)/d;
    let a=(lo-o)*inv;let b=(hi-o)*inv;
    let near=max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z));
    let far=min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z));
    return far>=max(near,0.0);
}
// What moved casters reach in one cell: its cube (the canonical probe: the whole cell
// is re-probed) or some of its fixed rays, followed without bound (only those
// directions' slots are re-traced). Returns whether anything was reached.
fn gi_reach(slot:u32, c:GiCell) -> bool {
    let cold=gi_cold(slot);
    let orphan=(gi_table[cold+GI_C_STATE]&GI_ORPHAN)!=0u;
    let q=vec3<f32>(bitcast<f32>(gi_table[cold+GI_C_POINT]),bitcast<f32>(gi_table[cold+GI_C_POINT+1u]),bitcast<f32>(gi_table[cold+GI_C_POINT+2u]));
    let frame=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
    var reached:array<u32,GI_MAX_WORDS>;
    var any_ray=false;
    // The rays start a small offset along the normal (gi_origin_offset); the box test
    // runs from that same origin, and the box grows by a margin several times larger.
    let origin=q+gi_unpack_normal(gi_table[cold+GI_C_NORMAL])*gi_origin_offset(q,c.edge);
    for(var b=0u;b<gi_frame.counts.x;b+=1u) {
        let pad=vec3<f32>(1e-3*c.edge+1e-4);
        let lo=gi_frame.boxes[b*2u].xyz-pad;let hi=gi_frame.boxes[b*2u+1u].xyz+pad;
        // The cube, grown by a quarter edge for the probe's start points.
        if all(c.centre+vec3<f32>(0.75*c.edge)>=lo) && all(c.centre-vec3<f32>(0.75*c.edge)<=hi) {
            for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[cold+GI_C_KNOWN+w]=0u; }
            return true;
        }
        // An orphan has no rays; only a moved cube can give it a query point.
        if orphan { continue; }
        for(var k=0u;k<GI_DIRECTIONS;k+=1u) {
            if gi_box_hit(origin,gi_direction(frame,c.key,k),lo,hi) {
                reached[k/32u]|=1u<<(k%32u);
            }
        }
    }
    // Directions the near field blocks stay blocked whatever moves: never re-traced.
    for(var w=0u;w<GI_WORDS;w+=1u) {
        let m=reached[w]&~gi_table[cold+GI_C_NEAR+w];
        any_ray=any_ray || m!=0u;
        gi_table[cold+GI_C_KNOWN+w]=gi_table[cold+GI_C_KNOWN+w]&~m;
    }
    return any_ray;
}
@compute @workgroup_size(64) fn gi_scan(@builtin(global_invocation_id) id:vec3<u32>) {
    // Environment-only frames (requests.w bit 31) visit only this frame's slot group.
    let group=gi_frame.requests.w;
    let period=group&0xffu;
    var slot=id.x;
    if (group&0x80000000u)!=0u {
        slot=id.x*period+(period-((group>>8u)&0xffu)%period)%period;
    }
    if slot>=gi_frame.offsets.x { return; }
    if gi_table[gi_hot(slot)]==0u || gi_evictable(slot) { return; }
    let cold=gi_cold(slot);
    let state=gi_table[cold+GI_C_STATE];
    if (state&GI_QUEUED)!=0u { return; }
    let flags=gi_frame.counts.z;
    var trace=(state&GI_TRACED)==0u || ((flags&GI_SCAN_PARTIAL)!=0u && (state&GI_PARTIAL)!=0u);
    if !trace && (flags&GI_SCAN_CASTER)!=0u && gi_reach(slot,gi_cell(slot)) {
        atomicAdd(&gi_work[GI_W_INVALIDATED],1u);
        trace=true;
    }
    if !trace && (flags&GI_SCAN_ROTATION)!=0u && gi_frame.counts.y!=0u && (slot+gi_word(GI_H_FRAME))%gi_frame.counts.y==0u {
        // A fixed-rotation refresh re-probes and re-traces the whole cell.
        for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[cold+GI_C_KNOWN+w]=0u; }
        atomicAdd(&gi_work[GI_W_REFRESHED],1u);
        trace=true;
    }
    if trace {
        gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]|GI_QUEUED;
        gi_queue_trace(slot);
        return;
    }
    // Bounce (G2), for cells with a value: a lighting, environment or program change relights
    // every cell (a step after still frames: shaded now, then GI_H_GATHERS gathers) or one
    // fixed slot group a frame while it keeps changing (one gather each); a moved caster
    // crossing a light's ray from the query point reshades it. Fixed rotations by slot index,
    // never a ranking (W2).
    if gi_bounce_on() && (state&(GI_TRACED|GI_ORPHAN|GI_PARTIAL))==GI_TRACED {
        if (flags&GI_SCAN_RELIGHT)!=0u {
            let relight=gi_frame.bounce.z;
            let all=(relight&GI_RELIGHT_ALL)!=0u;
            let p=relight&0xffu;
            if all || p==0u || (slot+((relight>>8u)&0xffu))%p==0u {
                gi_queue_shade(slot);
                gi_schedule(slot,select(1u,gi_word(GI_H_GATHERS),all));
            }
        }
        if (flags&GI_SCAN_CASTER)!=0u && gi_light_reached(slot) { gi_queue_shade(slot); }
    }
    // A changing map re-integrates one fixed slot group a frame (src/gi_cache.rs
    // ENVIRONMENT_PERIOD): a fixed rotation, never a ranking (W2).
    if (flags&GI_SCAN_ENVIRONMENT)!=0u && (state&GI_ORPHAN)==0u
        && (period==0u || (slot+((group>>8u)&0xffu))%period==0u) {
        gi_queue_integrate(slot);
    }
}

// ---- bounce light (G2): gathers and light visibility ----
// The direction a cell's query point sees light l in, and how far the light is (1e20 for a
// directional light), as light_sample computes it from a receiver.
fn gi_light_direction(l:u32, q:vec3<f32>) -> vec4<f32> {
    let light=lights[l];
    if light.kind.x==0u { return vec4<f32>(-light.travel_outer.xyz,1e20); }
    let delta=light.position_range.xyz-q;
    let distance=length(delta);
    return vec4<f32>(delta/max(distance,1e-30),distance);
}
// Whether a moved caster's swept box meets a light's ray from a cell's query point.
fn gi_light_reached(slot:u32) -> bool {
    let cold=gi_cold(slot);
    let q=gi_point(cold);
    let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
    let edge=gi_edge(gi_table[cold+GI_C_DEF]&0xffu);
    let origin=q+n*gi_origin_offset(q,edge);
    let lights_n=min(lighting.count,32u);
    for(var b=0u;b<gi_frame.counts.x;b+=1u) {
        let pad=vec3<f32>(1e-3*edge+1e-4);
        let lo=gi_frame.boxes[b*2u].xyz-pad;let hi=gi_frame.boxes[b*2u+1u].xyz+pad;
        for(var l=0u;l<lights_n;l+=1u) {
            if gi_box_hit(origin,gi_light_direction(l,q).xyz,lo,hi) { return true; }
        }
    }
    return false;
}
// One thread per cell queued for shading: one visibility ray per light (up to 32) from its
// query point, as bits for cell shading; clears its shading flag for the next frame.
@compute @workgroup_size(64) fn gi_light(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_SHADES]),gi_frame.offsets.x);
    for(var i=id.x;i<count;i+=groups.x*64u) {
        let slot=atomicLoad(&gi_work[gi_frame.bounce.w+i]);
        let cold=gi_cold(slot);
        gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]&~GI_SHADE;
        let q=gi_point(cold);
        let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
        let edge=gi_edge(gi_table[cold+GI_C_DEF]&0xffu);
        let origin=q+n*gi_origin_offset(q,edge);
        var bits=0u;
        let lights_n=min(lighting.count,32u);
        for(var l=0u;l<lights_n;l+=1u) {
            let d=gi_light_direction(l,q);
            if dot(n,d.xyz)<=0.0 { continue; }
            if gi_visible_within(origin,d.xyz,d.w) { bits|=1u<<l; }
        }
        atomicAdd(&gi_work[GI_W_LIGHT_RAYS],lights_n);
        gi_table[cold+GI_C_LIGHTS]=bits;
    }
}
// Requests the cell (level, bin, cell) for this frame's claim rounds, as the probe lists a
// miss, in a generation band below the pixels' (resolve marks it secondary or tertiary).
fn gi_request_one(level:u32, bin:u32, cell:vec3<i32>, band:u32) {
    let key=gi_key(level,bin,cell);
    let entry=gi_request_entry(key.x);
    if entry.x==0xffffffffu { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); return; }
    atomicMax(&gi_work[gi_frame.requests.x+entry.x*GI_REQUEST_WORDS+1u],band);
    if !gi_request_list(entry.x,key.y) { return; }
    let at=atomicAdd(&gi_work[GI_W_MISSES],1u);
    if at>=gi_frame.counts.w { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); return; }
    let base=gi_frame.offsets.y+at*GI_MISS_WORDS;
    let c=bitcast<vec3<u32>>(cell);
    atomicStore(&gi_work[base],level|(bin<<8u));
    atomicStore(&gi_work[base+1u],c.x);
    atomicStore(&gi_work[base+2u],c.y);
    atomicStore(&gi_work[base+3u],c.z);
    atomicStore(&gi_work[base+4u],entry.x);
    atomicStore(&gi_work[base+5u],GI_UNCLAIMED);
    atomicAdd(&gi_work[GI_W_GATHER_REQUESTS],1u);
}
// The outgoing radiance a gather ray meets at its hit: the containing cell of the hit's
// point, bin and level (the same cell a pixel there reads first), or else the first of the
// next coarser levels' containing cells with a value (a deterministic coarser fallback, as
// the lookup's, W3). A hit level without a cell is requested in generation band `band` when
// that is nonzero, for the claim rounds that follow this pass. w says what another gather
// could change (GC1, docs/m7-gc1.md):
// - GI_HIT_FINAL (1): a final value, the hit level's own or, where the hit level's cell is an
//   orphan (it never gets a value), a coarser one: cell shading wrote it with no gathers left
//   (gi_shade_entry.wgsl marks values still changing below 1);
// - GI_HIT_CHANGING (0.75): the same, but the cell still has gathers scheduled;
// - GI_HIT_MISSING (0.5): a coarser level's value while the hit level's cell is absent or not
//   shaded yet, which a later gather reads instead (the gather is then repeated once);
// - GI_HIT_NONE (0.25): no value, and none can come: the hit level's cell is an orphan, or
//   absent and not requested by this gather, and no coarser level has one (the ray adds
//   nothing, finally);
// - 0: no value yet (the ray adds nothing; counted as missing too).
// An absent hit cell a tertiary cell's gather meets (tertiary cells request none) counts as an
// orphan does: the coarser value it reads is the one a repeated gather would read.
const GI_HIT_FINAL:f32=1.0;
const GI_HIT_CHANGING:f32=0.75;
const GI_HIT_MISSING:f32=0.5;
const GI_HIT_NONE:f32=0.25;
fn gi_hit_radiance(origin:vec3<f32>, d:vec3<f32>, h:GiHit, band:u32, tertiary:bool) -> vec4<f32> {
    let r=gi_hit_receiver(origin,d,h);
    let world=r.world;
    let bin=gi_bin(r.vertex);
    let levels=gi_word(GI_H_LEVELS);
    let base=gi_level(world);
    var cause=0u;
    // The hit level's cell is an orphan: the levels above it are its final fallback.
    var orphan=false;
    for(var l=base;l<min(base+1u+GI_PARENT_ROUNDS,levels);l+=1u) {
        let cell=vec3<i32>(floor(gi_lattice(world,gi_edge(l),bin)));
        let slot=gi_find(gi_key(l,bin,cell));
        if slot==0xffffffffu {
            if l==base && band!=0u { gi_request_one(l,bin,cell,band); }
            if l==base { cause=GI_W_GATHER_ABSENT; orphan=tertiary; }
            continue;
        }
        // A gather keeps the cells it reads alive, like a pixel (the race is benign).
        gi_table[gi_cold(slot)+GI_C_SEEN]=gi_word(GI_H_FRAME);
        let o=gi_outgoing[slot];
        if o.w>0.0 {
            let settled=select(GI_HIT_CHANGING,GI_HIT_FINAL,o.w>=1.0);
            return vec4<f32>(o.rgb,select(GI_HIT_MISSING,settled,l==base || orphan));
        }
        if l==base {
            orphan=(gi_table[gi_cold(slot)+GI_C_STATE]&GI_ORPHAN)!=0u;
            if !orphan { cause=GI_W_GATHER_UNSHADED; }
        }
    }
    atomicAdd(&gi_work[select(GI_W_GATHER_ORPHANS,cause,cause!=0u)],1u);
    return vec4<f32>(0.0,0.0,0.0,select(0.0,GI_HIT_NONE,orphan));
}
// One workgroup per queued cell and one lane per fixed direction (k = lane, and lane + 64 with
// 128 directions): each lane traces its direction when it is blocked, with a closest-hit ray,
// and adds its hit's outgoing radiance times the fixed weight 2 pi / K to the L1 SH; a fixed
// tree sums the lanes, and the sum replaces the cell's bounce value (W1). (G2 ran eight lanes a
// cell, each tracing about six rays one after another; the pass waited on those chains, most
// of all on software rays: docs/m7-g2b.md.) Cells read their own and other cells' last
// outgoing radiance, which cell shading rewrites only after this pass, so the result does not
// depend on thread order.
// A frame gathers at most GI_H_GATHER_CELLS cells (0: every queued cell). When more are queued,
// it gathers those with slot % P == frame % P, for the smallest power of two P (up to
// GI_GATHER_PERIOD_MAX) that brings the queue under that count on average, and the rest wait
// in next frame's queue with their schedule unchanged. A fixed rotation by slot (W2), never a
// ranking: it bounds a cut's gather rays per frame in time rather than in frames.
const GI_GATHER_LANES:u32=64u;
const GI_GATHER_PERIOD_MAX:u32=64u;
var<workgroup> gi_wg_bounce:array<f32,12u*GI_GATHER_LANES>;
var<workgroup> gi_wg_missing:atomic<u32>;
var<workgroup> gi_wg_changing:atomic<u32>;
var<workgroup> gi_wg_rays:atomic<u32>;
var<private> gi_gather_sh:array<vec3<f32>,4>;
// The gather period P for `count` queued cells (1 without a cap).
fn gi_gather_period(count:u32) -> u32 {
    let cap=gi_word(GI_H_GATHER_CELLS);
    var p=1u;
    if cap==0u { return p; }
    while p<GI_GATHER_PERIOD_MAX && count>cap*p { p*=2u; }
    return p;
}
@compute @workgroup_size(64) fn gi_gather(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u { gi_wg_count=min(atomicLoad(&gi_work[gi_frame.bounce.x]),gi_frame.offsets.x); }
    let count=workgroupUniformLoad(&gi_wg_count);
    let requests=(gi_frame.bounce.z&GI_RELIGHT_REQUESTS)!=0u;
    let period=gi_gather_period(count);
    let phase=gi_word(GI_H_FRAME)%period;
    for(var at=group.x;at<count;at+=groups.x) {
        if lane==0u {
            atomicStore(&gi_wg_missing,0u);
            atomicStore(&gi_wg_changing,0u);
            atomicStore(&gi_wg_rays,0u);
        }
        let slot=atomicLoad(&gi_work[gi_frame.bounce.x+1u+at]);
        let cold=gi_cold(slot);
        let state=gi_table[cold+GI_C_STATE];
        let queued=gi_table[gi_hot(slot)]!=0u && (state&GI_BQUEUED)!=0u
            && (state&(GI_TRACED|GI_ORPHAN|GI_PARTIAL))==GI_TRACED;
        let live=queued && slot%period==phase;
        workgroupBarrier();
        for(var i=0u;i<4u;i+=1u) { gi_gather_sh[i]=vec3<f32>(0.0); }
        if live {
            let c=gi_cell(slot);
            let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
            let q=gi_point(cold);
            let frame=gi_basis(n);
            let origin=q+n*gi_origin_offset(q,c.edge);
            // Cells pixels made request in the secondary band, cells their gathers made in the
            // tertiary band, and tertiary cells request none: two generations of hit cells
            // beyond what pixels see, all below the pixels' band (gi_miss_claim).
            let secondary=(state&GI_SECONDARY)!=0u;
            let tertiary=(state&GI_TERTIARY)!=0u;
            let band=select(select(GI_BAND_SECONDARY,GI_BAND_TERTIARY,secondary),0u,!requests || tertiary);
            var missing=0u;
            var changing=0u;
            var rays=0u;
            let stride=max(gi_word(GI_H_GATHER_STRIDE),1u);
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_GATHER_LANES) {
                if k%stride!=0u || (gi_table[cold+GI_C_BITS+k/32u]&(1u<<(k%32u)))!=0u { continue; }
                let d=gi_direction(frame,c.key,k);
                let h=gi_closest(origin,d,0.0,1e20);
                rays+=1u;
                if !h.hit { continue; }
                let o=gi_hit_radiance(origin,d,h,band,tertiary);
                if o.w==GI_HIT_MISSING || o.w==0.0 { missing+=1u; }
                if o.w==GI_HIT_CHANGING { changing+=1u; }
                if o.w<=GI_HIT_NONE { continue; }
                gi_sh_basis(d);
                let radiance=o.rgb*(6.283185307179586*f32(stride)/f32(GI_DIRECTIONS));
                for(var i=0u;i<4u;i+=1u) { gi_gather_sh[i]+=radiance*gi_y[i]; }
            }
            if missing!=0u { atomicAdd(&gi_wg_missing,missing); }
            if changing!=0u { atomicAdd(&gi_wg_changing,changing); }
            if rays!=0u { atomicAdd(&gi_wg_rays,rays); }
        }
        // The lanes' sums into lane 0 in a fixed tree order (no atomics on values).
        for(var i=0u;i<4u;i+=1u) {
            gi_wg_bounce[(i*3u)*GI_GATHER_LANES+lane]=gi_gather_sh[i].x;
            gi_wg_bounce[(i*3u+1u)*GI_GATHER_LANES+lane]=gi_gather_sh[i].y;
            gi_wg_bounce[(i*3u+2u)*GI_GATHER_LANES+lane]=gi_gather_sh[i].z;
        }
        workgroupBarrier();
        for(var stride=GI_GATHER_LANES/2u;stride>0u;stride/=2u) {
            if lane<stride {
                for(var j=0u;j<12u;j+=1u) {
                    gi_wg_bounce[j*GI_GATHER_LANES+lane]+=gi_wg_bounce[j*GI_GATHER_LANES+lane+stride];
                }
            }
            workgroupBarrier();
        }
        if live && lane==0u {
            atomicAdd(&gi_work[GI_W_GATHERS],1u);
            atomicAdd(&gi_work[GI_W_GATHER_RAYS],atomicLoad(&gi_wg_rays));
            let missing=atomicLoad(&gi_wg_missing);
            if missing!=0u { atomicAdd(&gi_work[GI_W_GATHER_MISSES],missing); }
            var m=0.0;
            for(var j=0u;j<12u;j+=1u) { m=max(m,abs(gi_wg_bounce[j*GI_GATHER_LANES])); }
            let hot=gi_hot(slot);
            if m<=0.0 {
                gi_table[hot+GI_HOT_BOUNCE]=0u;
            } else {
                for(var w=0u;w<6u;w+=1u) {
                    let a=gi_wg_bounce[(2u*w)*GI_GATHER_LANES];
                    let b=gi_wg_bounce[(2u*w+1u)*GI_GATHER_LANES];
                    gi_table[hot+GI_HOT_BOUNCE+1u+w]=pack2x16float(vec2<f32>(a,b)/m);
                }
                gi_table[hot+GI_HOT_BOUNCE]=bitcast<u32>(m);
            }
            // The schedule: one gather done. A gather that met hit cells without a value yet
            // (new this frame, or requested now) is repeated once more.
            var left=(state&GI_GATHER_MASK)>>GI_GATHER_SHIFT;
            left=select(0u,left-1u,left>0u);
            // GC1: a gather whose every hit value was final ends the schedule. The cell's fixed
            // rays meet the same cells, whose values no longer change, so a further gather would
            // replace this value with itself (W1 form 1); an event that changes them schedules
            // its cells again. Cells reached while their hits still change keep the G gathers.
            // Moving cameras create cells among settled ones, which then gather once, not G
            // times (docs/m7-gc1.md).
            if missing==0u && atomicLoad(&gi_wg_changing)==0u { left=0u; }
            var retry=state&GI_RETRY;
            if missing!=0u {
                if left==0u && retry==0u { left=1u; retry=GI_RETRY; }
            } else {
                retry=0u;
            }
            gi_table[cold+GI_C_STATE]=(state&~(GI_GATHER_MASK|GI_BQUEUED|GI_RETRY))|retry;
            gi_queue_shade(slot);
            gi_schedule(slot,left);
        }
        // Outside this frame's slot group: next frame's queue, the schedule unchanged.
        if queued && !live && lane==0u {
            atomicAdd(&gi_work[GI_W_GATHER_DEFERRED],1u);
            gi_table[cold+GI_C_STATE]=state&~GI_BQUEUED;
            gi_schedule(slot,max((state&GI_GATHER_MASK)>>GI_GATHER_SHIFT,1u));
        }
        workgroupBarrier();
    }
}
