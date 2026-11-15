// Exact early-out predicate from primary_repair, including internal folds.
fn raster_needs_repair(trace:PrimaryTrace)->bool {
    let slot=trace.record.x-1u;let vertices=trace.positions;
    let a=world_position(vertices[0],slot).xyz;let b=world_position(vertices[1],slot).xyz;let c=world_position(vertices[2],slot).xyz;
    let epsilon=receiver_epsilon(a,b,c);
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(vertices[1]-vertices[0]);let ac=linear*(vertices[2]-vertices[0]);
    let margin=min(vec3<f32>(0.25),4.0*epsilon*vec3<f32>(length(ac-ab),length(ac),length(ab))/max(length(cross(ab,ac)),1e-30));
    let bary=bitcast<vec2<f32>>(trace.record.zw);let weights=vec3<f32>(1.0-bary.x-bary.y,bary);
    return !all(weights>=margin);
}
