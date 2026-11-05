// A traversal-free slice of primary_ray and primary_closest. No history, sampled
// textures, atomics, shared memory or prior output is read.
struct Frame { inverse:mat4x4<f32> };
@group(0) @binding(0) var<uniform> frame:Frame;
@group(0) @binding(1) var output:texture_storage_2d<rgba32uint,write>;
@compute @workgroup_size(8,4) fn ray_probe(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output);
    if any(id.xy>=size) {return;}
    let pixel=id.xy;
    let ndc=(vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let inverse=frame.inverse;
    let a=inverse*vec4<f32>(ndc,1.0,1.0);
    let b=inverse*vec4<f32>(ndc,0.0,1.0);
    let near=a.xyz/a.w;let far=b.xyz/b.w;
    let delta=far-near;let distance=length(delta);
    let direction=delta/distance;
    textureStore(output,vec2<i32>(id.xy),bitcast<vec4<u32>>(vec4<f32>(near,direction.x)));
}
