// P-RTS penumbra list (docs/p-rts.md). Standalone module, run after TR1's probe trace
// and its edge repair when the frame's shadow budget exceeds its probe count.
// The probe pass wrote each receiver's sampling.y probe rays (sampling.z of the budget's
// directions, the rest over a cone sampling.w times the sun's) as 0..256. A pixel is listed for the full budget when its own receiver exists and
// its 3x3 window holds both a blocked and a visible probe ray: a mixed pixel, or unanimous
// 0 and 256 pixels side by side. Listed pixels get exactly production's budget count.
@group(0) @binding(6) var probe_visibility:texture_2d<u32>;
@group(0) @binding(7) var primary_hit:texture_2d<u32>;
@group(0) @binding(21) var primary_tiles:texture_2d<u32>;
struct PenumbraQueue {x:atomic<u32>,y:atomic<u32>,z:u32,count:atomic<u32>,items:array<u32>};
@group(0) @binding(29) var<storage,read_write> penumbra:PenumbraQueue;
// primary_rays.wgsl PRIMARY_TILE: uncovered tiles keep stale hit records and counts.
const PRIMARY_TILE=vec2<u32>(8u,4u);
const PENUMBRA_RADIUS:i32=1;
// Listed pixels per primary_penumbra_trace workgroup (primary_penumbra.wgsl).
const PENUMBRA_PIXELS:u32=4u;
var<workgroup> penumbra_mask:atomic<u32>;
var<workgroup> penumbra_base:u32;

// Bit 0: some probe ray was blocked; bit 1: some was visible. Zero without a receiver.
fn penumbra_class(p:vec2<i32>,size:vec2<i32>)->u32 {
    if any(p<vec2<i32>(0)) || any(p>=size) {return 0u;}
    if textureLoad(primary_tiles,vec2<u32>(p)/PRIMARY_TILE,0).x==0u {return 0u;}
    if textureLoad(primary_hit,p,0).x==0u {return 0u;}
    let v=textureLoad(probe_visibility,p,0).x;
    return select(0u,1u,v<256u)|select(0u,2u,v>0u);
}

@compute @workgroup_size(8,4) fn primary_penumbra_list(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if all(id.xy==vec2<u32>(0u)) {penumbra.z=1u;}
    let size=vec2<i32>(textureDimensions(primary_hit));
    let p=vec2<i32>(id.xy);
    var listed=false;
    if penumbra_class(p,size)!=0u {
        var seen=0u;
        for(var dy=-PENUMBRA_RADIUS;dy<=PENUMBRA_RADIUS;dy+=1) {
            for(var dx=-PENUMBRA_RADIUS;dx<=PENUMBRA_RADIUS;dx+=1) {
                seen|=penumbra_class(p+vec2<i32>(dx,dy),size);
            }
        }
        listed=seen==3u;
    }
    if listed {atomicOr(&penumbra_mask,1u<<lane);}
    workgroupBarrier();
    // One global append per tile; lane rank fixes each pixel's slot within it.
    let mask=atomicLoad(&penumbra_mask);
    if lane==0u && mask!=0u {
        let count=countOneBits(mask);
        let base=atomicAdd(&penumbra.count,count);penumbra_base=base;
        let block=(base+count-1u)/PENUMBRA_PIXELS;
        atomicMax(&penumbra.x,min(block+1u,256u));
        atomicMax(&penumbra.y,block/256u+1u);
    }
    workgroupBarrier();
    if listed {
        penumbra.items[penumbra_base+countOneBits(mask&((1u<<lane)-1u))]=id.x+id.y*u32(size.x);
    }
}
