// ---- MP1: per-program tile-list shading entry, after the traced entry ----
// Mixed-program frames (docs/m7-mp1.md): program_bins.wgsl lists each frame slot's tiles,
// and this module's program shades its own slot's list in one indirect dispatch. Pixels
// of other programs return after reading their hit record; slot 0 also writes misses and
// the clear blocks outside primary_region. Each pixel is written by exactly one dispatch.
// Single-program frames never compose this entry: they keep primary_shade_tiles.
// Must match program_bins.wgsl RlList and primary_ray_gpu.rs ProgramLists.
struct RlList { slot:u32, base:u32, capacity:u32, flags:u32, programs:array<vec4<u32>,16> };
// This dispatch's slot (a dynamic offset per slot) and every draw's slot.
@group(0) @binding(38) var<uniform> rl_list:RlList;
// Per slot at base: the list's length, then its entries.
@group(0) @binding(39) var<storage,read> rl_entries:array<u32>;
const RL_LIST_CLEAR=0x40000000u;
const RL_LIST_MISSES=0x80000000u;
// Workgroups per dispatch row; program_bins.wgsl BINS_ROW must match.
const RL_LIST_ROW=65535u;

// The frame slot (program) of draw `batch`.
fn rl_slot_of(batch:u32)->u32 { return rl_list.programs[batch>>2u][batch&3u]; }

@compute @workgroup_size(4,4) fn primary_shade_list(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    // The dispatch is min(n, ROW) x ceil(n / ROW); the last row's surplus groups return.
    let index=group.x+group.y*RL_LIST_ROW;
    if index>=rl_entries[rl_list.base] {return;}
    let entry=rl_entries[rl_list.base+1u+index];
    let size=textureDimensions(primary_hit);
    let tile=vec2<u32>(entry&0x7fffu,(entry>>15u)&0x7fffu);
    let region=primary_region;
    if (entry&RL_LIST_CLEAR)!=0u {
        // As primary_shade_tiles: each lane stores one column of a clear block outside the region.
        let base=tile*PRIMARY_CLEAR_TILE+vec2<u32>(lane,0u);
        for(var k=0u;k<PRIMARY_CLEAR_TILE;k+=1u) {
            let p=base+vec2<u32>(0u,k);
            if all(p<size) && !(all(p>=region.xy) && all(p<region.zw)) {textureStore(primary_hdr,vec2<i32>(p),primary_miss(p,size,primary_frame.clear));}
        }
        return;
    }
    let id=region.xy+tile*4u+local.xy;
    if any(id>=size) {return;}
    let misses=(entry&RL_LIST_MISSES)!=0u;
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {
        if misses {textureStore(primary_hdr,vec2<i32>(id),primary_miss(id,size,primary_frame.clear));}
        return;
    }
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {
        if misses {textureStore(primary_hdr,vec2<i32>(id),primary_miss(id,size,primary_frame.clear));}
        return;
    }
    if rl_slot_of(primary_placements[record.x-1u]&0x1fffffffu)!=rl_list.slot {return;}
    primary_shade_pixel(id,size);
}
