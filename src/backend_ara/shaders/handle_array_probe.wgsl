enable wgpu_binding_array;
@group(0) @binding(0) var colours: binding_array<texture_2d<f32>, 4>;
@group(0) @binding(1) var filters: binding_array<sampler, 4>;
@group(0) @binding(2) var depths: binding_array<texture_depth_2d, 4>;
@group(0) @binding(3) var comparisons: binding_array<sampler_comparison, 4>;
@group(0) @binding(4) var<storage,read_write> results: array<vec4<f32>>;
fn fetch(i:u32, uv:vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(colours[i], filters[(i+1u)%4u], uv, 0.0);
}
@compute @workgroup_size(4)
fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i >= 4u { return; }
    let uv=vec2<f32>(f32(i)*0.5-0.125, 0.375);
    results[i*4u]=fetch(i,uv);
    results[i*4u+1u]=textureSampleLevel(colours[2u],filters[0u],vec2(0.5),0.0);
    let d=textureSampleCompareLevel(depths[i],comparisons[i],uv,0.5);
    results[i*4u+2u]=vec4(d,d,d,d);
    results[i*4u+3u]=vec4<f32>(vec2<f32>(textureDimensions(colours[i])),f32(textureNumLevels(colours[i])),0.0);
}
