// MP1 (docs/m7-mp1.md): per-program tile lists for mixed-program traced and
// visibility-buffer frames. An engine kernel with no program code, compiled once.
// The dispatch keeps its own 4x4 list grid, independently of single-program shading:
// the region's list tiles in the first rows, then rows of
// 16x16 clear blocks over the image. Each region tile is appended to the list of every
// frame slot (program) whose pixels it holds; tiles holding misses or uncovered trace
// tiles, and every clear block, go to slot 0, which writes the clear colour. Each list
// is then shaded by its program's own pipeline in one indirect dispatch
// (program_traced_list.wgsl).
// Must match program_traced_list.wgsl RlList and primary_ray_gpu.rs ProgramLists.
struct RlList { slot:u32, base:u32, capacity:u32, flags:u32, programs:array<vec4<u32>,PRIMARY_BATCHES/4> };
@group(0) @binding(0) var bins_hit:texture_2d<u32>;
@group(0) @binding(1) var bins_coverage:texture_2d<u32>;
@group(0) @binding(2) var<storage,read> bins_region:vec4<u32>;
@group(0) @binding(3) var<storage,read> bins_placements:array<u32>;
@group(0) @binding(4) var<uniform> bins_list:RlList;
// Per slot at slot * capacity: the list's length, then its entries.
@group(0) @binding(5) var<storage,read_write> bins_entries:array<atomic<u32>>;
// Per slot: indirect dispatch arguments (x, y, z) and a pad word; cleared before this pass.
@group(0) @binding(6) var<storage,read_write> bins_args:array<atomic<u32>>;

// primary_ray_gpu.rs TRACE_TILE and CLEAR_TILE.
const BINS_TRACE_TILE=vec2<u32>(8u,4u);
const BINS_CLEAR_TILE=16u;
// Entry flags; program_traced_list.wgsl reads the same bits.
const BINS_CLEAR=0x40000000u;
const BINS_MISSES=0x80000000u;
// Workgroups per row of a list's indirect dispatch: wgpu's default per-dimension limit,
// which indirect validation enforces (a 1080p list holds up to about 139,000 tiles).
// program_traced_list.wgsl RL_LIST_ROW must match.
const BINS_ROW=65535u;

var<workgroup> bins_mask:atomic<u32>;

fn bins_append(slot:u32,entry:u32) {
    let base=slot*bins_list.capacity;
    let at=atomicAdd(&bins_entries[base],1u);
    // A list of n tiles dispatches min(n, ROW) x ceil(n / ROW) workgroups.
    atomicMax(&bins_args[slot*4u],min(at+1u,BINS_ROW));
    atomicMax(&bins_args[slot*4u+1u],at/BINS_ROW+1u);
    atomicStore(&bins_args[slot*4u+2u],1u);
    // Slot 0 writes misses and clear blocks.
    atomicStore(&bins_entries[base+1u+at],entry|select(0u,BINS_MISSES,slot==0u));
}

@compute @workgroup_size(4,4) fn program_bins(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let region=bins_region;
    let shaded=(region.zw-region.xy)/vec2<u32>(4u);
    let size=textureDimensions(bins_hit);
    if group.y>=shaded.y {
        let block=vec2<u32>(group.x,group.y-shaded.y);
        if lane==0u && all(block*BINS_CLEAR_TILE<size) {bins_append(0u,block.x|(block.y<<15u)|BINS_CLEAR);}
        return;
    }
    if group.x>=shaded.x {return;}
    if lane==0u {atomicStore(&bins_mask,0u);}
    workgroupBarrier();
    let id=region.xy+group.xy*4u+local.xy;
    var bit=0u;
    if all(id<size) {
        bit=1u;
        // Uncovered trace tiles hold stale hit records: their pixels are misses.
        if textureLoad(bins_coverage,id/BINS_TRACE_TILE,0).x!=0u {
            let record=textureLoad(bins_hit,vec2<i32>(id),0);
            if record.x!=0u {
                let batch=bins_placements[record.x-1u]&0x1fffffffu;
                bit=1u<<bins_list.programs[batch>>2u][batch&3u];
            }
        }
    }
    if bit!=0u {atomicOr(&bins_mask,bit);}
    workgroupBarrier();
    if lane==0u {
        var mask=atomicLoad(&bins_mask);
        let entry=group.x|(group.y<<15u);
        while mask!=0u {
            let slot=firstTrailingBit(mask);
            bins_append(slot,entry);
            mask&=mask-1u;
        }
    }
}

// The grouped binning (wgpu; `RENDERLAB_BINS_GROUPED=0` dispatches `program_bins` above):
// the same lists and entries, but one 16x16 group bins 4x4 list tiles (or 256 clear blocks)
// in workgroup memory and reserves each list's entries with one global atomic. Per-tile
// appends from about 130,000 groups queued on the lists' few counters: 0.3-0.4 ms at 1080p.
// Entries within a list come in another order, which the list shading does not depend on.
// Dispatch: ceil(region / 16) groups, then rows of groups of 16x16 clear blocks
// (program_lists.rs bins_groups).
// List entries per shading workgroup (program_lists.rs bin_source_for).
const BINS_PER:u32=1u;
// Per list tile of the group: the slots its pixels need.
var<workgroup> bins_tile_masks:array<atomic<u32>,16>;
// Per slot: the group's entries, then the next entry index once reserved.
var<workgroup> bins_counts:array<atomic<u32>,32>;
var<workgroup> bins_cursor:array<atomic<u32>,32>;

// Reserves `count` entries of `slot`'s list and raises its dispatch arguments over them.
fn bins_reserve(slot:u32,count:u32)->u32 {
    let at=atomicAdd(&bins_entries[slot*bins_list.capacity],count);
    let last=(at+count-1u)/BINS_PER;
    atomicMax(&bins_args[slot*4u],min(last+1u,BINS_ROW));
    atomicMax(&bins_args[slot*4u+1u],last/BINS_ROW+1u);
    atomicStore(&bins_args[slot*4u+2u],1u);
    return at;
}

@compute @workgroup_size(16,16) fn program_bins_grouped(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let region=bins_region;
    let shaded=(region.zw-region.xy)/vec2<u32>(4u);
    let rows=(shaded+vec2<u32>(3u))/vec2<u32>(4u);
    let size=textureDimensions(bins_hit);
    if lane<32u {atomicStore(&bins_counts[lane],0u);}
    if lane<16u {atomicStore(&bins_tile_masks[lane],0u);}
    workgroupBarrier();
    if group.y>=rows.y {
        // Clear blocks, one per invocation, all to slot 0.
        let blocks=(size+vec2<u32>(BINS_CLEAR_TILE-1u))/BINS_CLEAR_TILE;
        let block=vec2<u32>(group.x,group.y-rows.y)*16u+local.xy;
        let mine=all(block<blocks);
        if mine {atomicAdd(&bins_counts[0],1u);}
        workgroupBarrier();
        if lane==0u {
            let n=atomicLoad(&bins_counts[0]);
            if n>0u {atomicStore(&bins_cursor[0],bins_reserve(0u,n));}
        }
        workgroupBarrier();
        if mine {
            let at=atomicAdd(&bins_cursor[0],1u);
            atomicStore(&bins_entries[1u+at],block.x|(block.y<<15u)|BINS_CLEAR|BINS_MISSES);
        }
        return;
    }
    // This invocation's pixel in list tile `tile` (index `t` in the group).
    let tile=group.xy*4u+local.xy/4u;
    let t=(local.y/4u)*4u+local.x/4u;
    let id=region.xy+tile*4u+local.xy%4u;
    var bit=0u;
    if all(tile<shaded) && all(id<size) {
        bit=1u;
        // Uncovered trace tiles hold stale hit records: their pixels are misses.
        if textureLoad(bins_coverage,id/BINS_TRACE_TILE,0).x!=0u {
            let record=textureLoad(bins_hit,vec2<i32>(id),0);
            if record.x!=0u {
                let batch=bins_placements[record.x-1u]&0x1fffffffu;
                bit=1u<<bins_list.programs[batch>>2u][batch&3u];
            }
        }
    }
    if bit!=0u {atomicOr(&bins_tile_masks[t],bit);}
    workgroupBarrier();
    // One invocation per tile counts its slots; one per slot reserves them; the tiles append.
    var mask=0u;
    if lane<16u {mask=atomicLoad(&bins_tile_masks[lane]);}
    var m=mask;
    while m!=0u {
        atomicAdd(&bins_counts[firstTrailingBit(m)],1u);
        m&=m-1u;
    }
    workgroupBarrier();
    if lane<32u {
        let n=atomicLoad(&bins_counts[lane]);
        if n>0u {atomicStore(&bins_cursor[lane],bins_reserve(lane,n));}
    }
    workgroupBarrier();
    let entry_tile=group.xy*4u+vec2<u32>(lane%4u,lane/4u);
    let entry=entry_tile.x|(entry_tile.y<<15u);
    m=mask;
    while m!=0u {
        let slot=firstTrailingBit(m);
        let at=atomicAdd(&bins_cursor[slot],1u);
        atomicStore(&bins_entries[slot*bins_list.capacity+1u+at],entry|select(0u,BINS_MISSES,slot==0u));
        m&=m-1u;
    }
}
