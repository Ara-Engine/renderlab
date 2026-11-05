// Experimental primary-identity replacement. The caller supplies the committed
// hardware t and facing; failed or out-of-bound retries preserve the raw hit.
fn primary_backface_retry(trace:PrimaryTrace,origin:vec3<f32>,direction:vec3<f32>,distance:f32,front:bool)->PrimaryTrace {
    if front {return trace;}
    let slot=trace.record.x-1u;let vertices=trace.positions;
    let a=world_position(vertices[0],slot).xyz;let b=world_position(vertices[1],slot).xyz;let c=world_position(vertices[2],slot).xyz;
    let epsilon=receiver_epsilon(a,b,c);
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(vertices[1]-vertices[0]);let ac=linear*(vertices[2]-vertices[0]);
    let margin=min(vec3<f32>(0.25),4.0*epsilon*vec3<f32>(length(ac-ab),length(ac),length(ab))/max(length(cross(ab,ac)),1e-30));
    let bary=bitcast<vec2<f32>>(trace.record.zw);let weights=vec3<f32>(1.0-bary.x-bary.y,bary);
    if all(weights>=margin) {return trace;}
    let normal=primary_geometric(vertices[0],vertices[1],vertices[2],slot);
    let bound=4.0*epsilon/max(abs(primary_edge_dot(normal,direction)),0.0001);
    var retry:ray_query<vertex_return>;
    let flags=RAY_FLAG_FORCE_NO_OPAQUE|RAY_FLAG_CULL_BACK_FACING;
    rayQueryInitialize(&retry,primary_scene,RayDesc(flags,255u,0.0,distance+bound,origin,direction));
    while rayQueryProceed(&retry) {
        let candidate=rayQueryGetCandidateIntersection(&retry);
        var visible=(primary_placements[candidate.instance_custom_data]&0x40000000u)==0u;
        if !visible {
            let v=getCandidateHitVertexPositions(&retry);
            visible=dot(cross(v[1]-v[0],v[2]-v[0]),candidate.world_to_object*vec4<f32>(direction,0.0))<0.0;
        }
        if visible {rayQueryConfirmIntersection(&retry);}
    }
    let hit=rayQueryGetCommittedIntersection(&retry);
    if hit.kind==0u || abs(hit.t-distance)>bound {return trace;}
    let v=getCommittedHitVertexPositions(&retry);
    let w=vec3<f32>(1.0-hit.barycentrics.x-hit.barycentrics.y,hit.barycentrics);
    let point=world_position(receiver_point(v[0],v[1],v[2],w),hit.instance_custom_data).xyz;
    return PrimaryTrace(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),point,v);
}
