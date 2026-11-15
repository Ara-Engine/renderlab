// Indirect x/y/z and count, then one pixel index per possible image pixel.
struct EdgeQueue {x:atomic<u32>,y:atomic<u32>,z:atomic<u32>,count:atomic<u32>,pixels:array<u32>};
@group(0) @binding(36) var<storage,read_write> edge_queue:EdgeQueue;
var<workgroup> edge_count:atomic<u32>;
var<workgroup> edge_base:u32;
var<private> edge_pixel:u32;
var<private> edge_slot:u32;
var<private> edge_queued:bool;
fn raster_enqueue(pixel:u32) {
    edge_pixel=pixel;edge_slot=atomicAdd(&edge_count,1u);edge_queued=true;
}
// All resolve invocations call this after publishing their workgroup count.
fn raster_flush_queue(first:bool) {
    if first {
        let n=atomicLoad(&edge_count);
        if n!=0u {
            let base=atomicAdd(&edge_queue.count,n);edge_base=base;
            let groups=(base+n+31u)/32u;
            atomicMax(&edge_queue.x,min(groups,65535u));
            atomicMax(&edge_queue.y,(groups+65534u)/65535u);
            atomicMax(&edge_queue.z,1u);
        }
    }
    workgroupBarrier();
    if edge_queued {edge_queue.pixels[edge_base+edge_slot]=edge_pixel;}
}
@compute @workgroup_size(1) fn reset_edge_queue() {
    atomicStore(&edge_queue.x,0u);atomicStore(&edge_queue.y,0u);
    atomicStore(&edge_queue.z,0u);atomicStore(&edge_queue.count,0u);
}
