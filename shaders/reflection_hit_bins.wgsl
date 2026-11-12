// Reflection hit shading (docs/design-reflection-pass.md): bins H's records by the hit's
// program, so each program's hit module shades only its own hits in one indirect dispatch
// (shaders/reflection_hits.wgsl). An engine kernel with no program code, compiled once.
// Dispatched over H with rl_hit_args's arguments: rows of 64-lane groups, as the hit entry.
// The hit's program is its draw's frame slot (MP1's table, program_bins.wgsl RlList).
// Must match src/reflection_hits.rs HitBins.
struct RlHitBins { stride:u32, _pad0:u32, _pad1:u32, _pad2:u32, programs:array<vec4<u32>,PRIMARY_BATCHES/4> };
// H: a 16-byte header (count, dispatch), then three vec4<u32> per record.
@group(0) @binding(0) var<storage,read> hb_records:array<vec4<u32>>;
// Each placement's draw (batch) in its low 29 bits, as the primary trace reads it.
@group(0) @binding(1) var<storage,read> hb_placements:array<u32>;
@group(0) @binding(2) var<uniform> hb_bins:RlHitBins;
// Per slot at slot * stride: the bin's length, then its record indices.
@group(0) @binding(3) var<storage,read_write> hb_order:array<atomic<u32>>;
// Per slot: indirect dispatch arguments (x, y, z) and a pad word; cleared before this pass.
@group(0) @binding(4) var<storage,read_write> hb_args:array<atomic<u32>>;
// shaders/reflection_hits.wgsl RL_HITS_ROW and its lanes.
const HB_ROW=65535u;
const HB_LANES=64u;

@compute @workgroup_size(64) fn reflection_hit_bins(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=(group.x+group.y*HB_ROW)*HB_LANES+lane;
    if index>=hb_records[0].x {return;}
    let placement=hb_records[1u+index*3u].w-1u;
    let batch=hb_placements[placement]&0x1fffffffu;
    let slot=hb_bins.programs[batch>>2u][batch&3u];
    let base=slot*hb_bins.stride;
    let at=atomicAdd(&hb_order[base],1u);
    // A bin of n records dispatches min(g, ROW) x ceil(g / ROW) groups, g = ceil(n / 64).
    let groups=at/HB_LANES+1u;
    atomicMax(&hb_args[slot*4u],min(groups,HB_ROW));
    atomicMax(&hb_args[slot*4u+1u],(groups-1u)/HB_ROW+1u);
    atomicStore(&hb_args[slot*4u+2u],1u);
    atomicStore(&hb_order[base+1u+at],index);
}
