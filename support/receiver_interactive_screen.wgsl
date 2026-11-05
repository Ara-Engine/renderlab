// Test-only scheduling screen: one lane traces each pixel, eight share its shadow
// directions. The existing 8x4 coverage tile and production estimator are unchanged.
var<workgroup> screen_traces:array<PrimaryTrace,32>;
var<workgroup> screen_counts:array<atomic<u32>,32>;
@compute @workgroup_size(256) fn primary_trace_interactive(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) index:u32) {
    let slot=index/8u;let lane=index%8u;
    let id=group.xy*PRIMARY_TILE+vec2<u32>(slot%PRIMARY_TILE.x,slot/PRIMARY_TILE.x);
    let size=textureDimensions(primary_hit_output);let inside=all(id<size);
    if lane==0u {
        let trace=primary_scene_trace(id,size);
        screen_traces[slot]=trace;
        if trace.record.x!=0u {atomicOr(&primary_tile_hit,1u);}
    }
    workgroupBarrier();
    let covered=atomicLoad(&primary_tile_hit);
    if index==0u {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}
    if covered==0u {return;}
    let p=vec2<i32>(id);let trace=screen_traces[slot];
    if inside && lane==0u {textureStore(primary_hit_output,p,trace.record);}
    var partial=FRESH_SKIP;
    if trace.record.x!=0u {
        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
        partial=fresh_estimate_lanes(receiver,id,size.x,lane,8u);
    }
    atomicAdd(&screen_counts[slot],select(partial,0u,partial==FRESH_SKIP));
    workgroupBarrier();
    if inside && lane==0u {
        textureStore(fresh_output,p,vec4<u32>(select(32u*atomicLoad(&screen_counts[slot]),256u,partial==FRESH_SKIP)));
    }
}
