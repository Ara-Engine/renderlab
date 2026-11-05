// Reuses the footprint diagnostic's eight counters, with one centre ray per pixel.
@compute @workgroup_size(8,4) fn film_footprint(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=vec2<u32>(1920u,1080u);if any(id.xy>=size) {return;}
    film_sample=vec2<f32>(0.5);
    let trace=primary_raw_closest(id.xy,size);if trace.record.x==0u {return;}
    atomicAdd(&footprint_sums[0].accepted,1u);
    let v=trace.positions;let slot=trace.record.x-1u;
    let ray=primary_ray(id.xy,size);let direction=normalize(ray.far-ray.near);
    let back=dot(primary_geometric(v[0],v[1],v[2],slot),direction)>0.0;
    let epsilon=primary_epsilon(v,slot);
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(v[1]-v[0]);let ac=linear*(v[2]-v[0]);
    let margin=min(vec3<f32>(0.25),4.0*epsilon*vec3<f32>(length(ac-ab),length(ac),length(ab))/max(length(cross(ab,ac)),1e-30));
    let bary=bitcast<vec2<f32>>(trace.record.zw);let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let near=any(w<margin);
    if back {atomicAdd(&footprint_sums[0].visible,1u);}
    if back && !near {atomicAdd(&footprint_sums[0].opposite,1u);}
    if back && near {atomicAdd(&footprint_sums[0].corner,1u);}
    if near {atomicAdd(&footprint_sums[0].edge,1u);}
}
