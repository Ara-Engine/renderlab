// Closest-hit camera rays. Appended after primary_rays.wgsl to the hardware composition
// and, for the Interactive trace, to the soft one; custom data is the placement index.
// Workgroups match primary_ray_gpu.rs TRACE_TILE.
// The shared TLAS with hit-triangle positions. Binding 2 stays the shadow rays' view
// (hardware_scene or fresh_scene), so one dispatch can bind both.
@group(0) @binding(19) var primary_scene:acceleration_structure<vertex_return>;
@group(0) @binding(13) var primary_hit_output:texture_storage_2d<rgba32uint,write>;
@group(0) @binding(16) var primary_world_output:texture_storage_2d<rgba32float,write>;
@group(0) @binding(17) var primary_normal_output:texture_storage_2d<rgba32float,write>;

// Object-space positions are the hit triangle's own, in index order.
struct PrimaryTrace {record:vec4<u32>,point:vec3<f32>,positions:array<vec3<f32>,3>};
// The full near-to-far segment with forced opaque geometry and no first-hit flag.
fn primary_closest(pixel:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;let distance=length(delta);
    let direction=delta/distance;
    var query:ray_query<vertex_return>;
    // Single-sided draws hide source-back faces as the raster discard does. Facing is
    // the object-space winding normal against the object-space ray, so mirrored
    // instances keep their source sides. Only scenes with such draws pay for it.
    let flags=select(RAY_FLAG_FORCE_OPAQUE,RAY_FLAG_FORCE_NO_OPAQUE,primary_frame.sided.x!=0u);
    rayQueryInitialize(&query,primary_scene,RayDesc(flags,255u,0.0,distance,ray.near,direction));
    while rayQueryProceed(&query) {
        let candidate=rayQueryGetCandidateIntersection(&query);
        var visible=(primary_placements[candidate.instance_custom_data]&0x40000000u)==0u;
        if !visible {
            let v=getCandidateHitVertexPositions(&query);
            visible=dot(cross(v[1]-v[0],v[2]-v[0]),candidate.world_to_object*vec4<f32>(direction,0.0))<0.0;
        }
        if visible {rayQueryConfirmIntersection(&query);}
    }
    let hit=rayQueryGetCommittedIntersection(&query);
    if hit.kind==0u {return PrimaryTrace(vec4<u32>(0u),vec3<f32>(0.0),array<vec3<f32>,3>());}
    return PrimaryTrace(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),
        ray.near+direction*hit.t,getCommittedHitVertexPositions(&query));
}
// Hard and unshadowed frames: the hit record only.
@compute @workgroup_size(8,4) fn primary_trace(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    if any(id.xy>=size) {return;}
    textureStore(primary_hit_output,vec2<i32>(id.xy),primary_closest(id.xy,size).record);
}
// Soft frames also publish exact fresh receivers at hits: the ray point, its depth and
// the hit triangle's plane normal from its returned positions (no vertex gather).
// Misses leave both receivers stale; traced fresh visibility reads coverage from the
// hit record instead (primary_ray_gpu.rs COVERAGE_SEAM).
@compute @workgroup_size(8,4) fn primary_trace_receivers(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    let trace=primary_closest(id.xy,size);
    textureStore(primary_hit_output,p,trace.record);
    if trace.record.x==0u {return;}
    let clip=camera.view_projection*vec4<f32>(trace.point,1.0);
    textureStore(primary_world_output,p,vec4<f32>(trace.point,clip.z/clip.w));
    let v=trace.positions;
    let normal=primary_geometric(v[0],v[1],v[2],trace.record.x-1u);
    textureStore(primary_normal_output,p,vec4<f32>(normal,1.0));
}
