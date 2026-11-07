// ---- renderlab shading-program API, traced compute stage ----
// Draw geometry and materials are descriptor-indexed by batch (one per draw), so one
// dispatch shades every pixel. Samples use the hit's ray-differential footprint; compute
// stages have no implicit derivatives, so naga rejects textureSample and dpdx here.
@group(0) @binding(18) var primary_hdr:texture_storage_2d<rgba16float,write>;
struct PrimaryVec4s { data:array<vec4<f32>> };
struct PrimaryU32s { data:array<u32> };
struct PrimaryMaterial { value:Material };
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<PrimaryVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<PrimaryU32s,PRIMARY_BATCHES>;
@group(1) @binding(4) var<storage,read> primary_tangents:binding_array<PrimaryVec4s,PRIMARY_BATCHES>;
@group(2) @binding(10) var<storage,read> primary_materials:binding_array<PrimaryMaterial,PRIMARY_BATCHES>;
@group(2) @binding(11) var primary_base_color:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(12) var primary_base_sampler:binding_array<sampler,PRIMARY_BATCHES>;
@group(2) @binding(13) var primary_metallic_roughness:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(14) var primary_mr_sampler:binding_array<sampler,PRIMARY_BATCHES>;
@group(2) @binding(15) var primary_normal_map:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(16) var primary_normal_sampler:binding_array<sampler,PRIMARY_BATCHES>;

fn sample_base_color(input:SurfaceInput) -> vec4<f32> {
    return sample_base_color_grad(input,input.uv,input.uv_dx,input.uv_dy);
}
fn sample_metallic_roughness(input:SurfaceInput) -> vec4<f32> {
    return sample_metallic_roughness_grad(input,input.uv,input.uv_dx,input.uv_dy);
}
fn sample_normal_map(input:SurfaceInput) -> vec4<f32> {
    return sample_normal_map_grad(input,input.uv,input.uv_dx,input.uv_dy);
}
fn sample_base_color_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    let i=input.receiver.slot;
    return textureSampleGrad(primary_base_color[i],primary_base_sampler[i],uv,dx,dy);
}
fn sample_metallic_roughness_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    let i=input.receiver.slot;
    return textureSampleGrad(primary_metallic_roughness[i],primary_mr_sampler[i],uv,dx,dy);
}
fn sample_normal_map_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    let i=input.receiver.slot;
    return textureSampleGrad(primary_normal_map[i],primary_normal_sampler[i],uv,dx,dy);
}
// The DFG table has one level, so this equals the raster sample.
fn environment_dfg(n_dot_v:f32, roughness:f32) -> vec2<f32> {
    let uv=(vec2<f32>(clamp(n_dot_v,0.0,1.0),roughness)*127.0+vec2<f32>(0.5))/128.0;
    return textureSampleLevel(dfg,dfg_sampler,uv,0.0).rg;
}
// ---- end of the traced program API ----
