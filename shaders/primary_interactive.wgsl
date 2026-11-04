// Interactive soft frames: each pixel's closest-hit camera ray and its 8-direction
// visibility in one dispatch. Appended with primary_trace.wgsl to the soft module, so
// fresh_estimate takes the exact receiver from registers instead of a target round trip.
// Counts equal primary_trace_receivers followed by 8-direction fresh_compute: same
// receiver, directions, flags and 0..256 output contract, misses and skips 256.
// Every tile writes its coverage flag; only tiles with a camera hit write hit records
// and counts, so Interactive shading reads the flag first. Pixels outside
// primary_frame.scene are misses without a ray.
// Workgroups and PRIMARY_TILE match primary_ray_gpu.rs TRACE_TILE.
const PRIMARY_TILE=vec2<u32>(8u,4u);
@group(0) @binding(20) var primary_tile_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(21) var primary_tiles:texture_2d<u32>;
var<workgroup> primary_tile_hit:atomic<u32>;
@compute @workgroup_size(8,4) fn primary_trace_interactive(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    let inside=all(id.xy<size);
    var trace:PrimaryTrace;
    let scene=primary_frame.scene;
    if inside && all(id.xy>=scene.xy) && all(id.xy<scene.zw) {trace=primary_closest(id.xy,size);}
    if trace.record.x!=0u {atomicOr(&primary_tile_hit,1u);}
    workgroupBarrier();
    let covered=atomicLoad(&primary_tile_hit);
    // A tile's first pixel is always inside the image.
    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}
    if !inside || covered==0u {return;}
    let p=vec2<i32>(id.xy);
    textureStore(primary_hit_output,p,trace.record);
    var visible=FRESH_SKIP;
    if trace.record.x!=0u {
        let v=trace.positions;
        let normal=primary_geometric(v[0],v[1],v[2],trace.record.x-1u);
        visible=fresh_estimate(trace.point,normal,id.xy,size.x,0u);
    }
    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));
}
// primary_shade for Interactive frames: uncovered tiles take the clear colour without
// reading their stale hit records. Workgroups match primary_ray_gpu.rs SHADE_TILE.
@compute @workgroup_size(4,4) fn primary_shade_tiles(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    if any(id.xy>=size) {return;}
    if textureLoad(primary_tiles,id.xy/PRIMARY_TILE,0).x==0u {
        textureStore(primary_hdr,vec2<i32>(id.xy),primary_frame.clear);
        return;
    }
    primary_shade_pixel(id.xy,size);
}
