// ---- renderlab shading-program API, raster fragment stage ----
// Mesh-uv samples use the hardware's implicit derivatives, as the fixed lit shader
// sampled. Call them in uniform control flow (the generated entry calls `surface` before
// any per-pixel branch); custom mappings pass their own footprint to the `_grad` forms.
fn sample_base_color(input:SurfaceInput) -> vec4<f32> { return textureSample(base_color,base_sampler,input.uv); }
fn sample_metallic_roughness(input:SurfaceInput) -> vec4<f32> { return textureSample(metallic_roughness,mr_sampler,input.uv); }
fn sample_normal_map(input:SurfaceInput) -> vec4<f32> { if input.material.normal_variance==2u { return vec4<f32>(unit_normal(textureSample(normal_map,normal_sampler,input.uv,1).yzw)*0.5+vec3<f32>(0.5),0.0); }
    let s=textureSample(normal_map,normal_sampler,input.uv,0);
    if input.material.normal_mapped!=2u { return s; }
    return rl_normal_texel(s,textureSampleGrad(normal_map,normal_sampler,input.uv,1,input.uv_dx,input.uv_dy).r); }
fn sample_base_color_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    return textureSampleGrad(base_color,base_sampler,uv,dx,dy);
}
fn sample_metallic_roughness_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    return textureSampleGrad(metallic_roughness,mr_sampler,uv,dx,dy);
}
fn sample_normal_layer_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>, layer:i32) -> vec4<f32> {
    let s=textureSampleGrad(normal_map,normal_sampler,uv,layer,dx,dy);
    if input.material.normal_mapped!=2u || layer!=0 { return s; }
    return rl_normal_texel(s,textureSampleGrad(normal_map,normal_sampler,uv,1,dx,dy).r);
}
// Split-sum (A, B) at (n.v, roughness). The table has one level, so every stage agrees.
fn environment_dfg(n_dot_v:f32, roughness:f32) -> vec2<f32> {
    let uv=(vec2<f32>(clamp(n_dot_v,0.0,1.0),roughness)*127.0+vec2<f32>(0.5))/128.0;
    return textureSample(dfg,dfg_sampler,uv).rg;
}
// ---- end of the raster program API ----
