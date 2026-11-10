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
