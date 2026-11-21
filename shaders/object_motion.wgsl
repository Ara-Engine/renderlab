// MV1: shared surface-motion source for TR1 and PP1 on traced/visibility hits.
// The ordinary model ABI remains indexed by placement; its second half stores
// the previous submitted placement. No material/shading-model assumptions.
struct MotionVec4s { data:array<vec4<f32>> };
@group(1) @binding(6) var<storage,read> motion_previous_vertices:binding_array<MotionVec4s,PRIMARY_BATCHES>;
fn motion_previous_model(slot:u32)->mat4x4<f32> {
    return models[arrayLength(&models)/2u+slot];
}
fn motion_rigid(slot:u32)->bool {
    let a=models[slot];let b=motion_previous_model(slot);
    let eps=vec4<f32>(1e-5);
    return any(abs(a[0]-b[0])>eps)||any(abs(a[1]-b[1])>eps)
        ||any(abs(a[2]-b[2])>eps)||any(abs(a[3]-b[3])>eps);
}
fn motion_previous_vertex(batch:u32,vertex:u32)->vec3<f32> {
    return motion_previous_vertices[batch].data[vertex*3u].xyz;
}
fn motion_previous_world(position:vec3<f32>,slot:u32)->vec4<f32> {
    return motion_previous_model(slot)*vec4<f32>(position,1.0);
}
