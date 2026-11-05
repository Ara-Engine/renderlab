// Interactive soft frames: each pixel's closest-hit camera ray and its 8-direction
// visibility in one dispatch. Appended with primary_trace.wgsl to the soft module, so
// fresh_estimate takes the exact receiver from registers instead of a target round trip.
// Counts equal primary_trace_receivers followed by 8-direction fresh_compute: same
// receiver, directions, flags and 0..256 output contract, misses and skips 256.
// Tile flags and the scene rectangle are primary_trace.wgsl's: only tiles with a camera
// hit write hit records and counts.
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
        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
        visible=fresh_estimate(receiver,id.xy,size.x,0u);
    }
    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));
}
