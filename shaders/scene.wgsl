struct Camera { view_projection: mat4x4<f32>, view_mode: vec4<u32> };
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> models: array<mat4x4<f32>>;
struct NormalTransform { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, info:vec4<f32> };
@group(1) @binding(1) var<storage, read> normals: array<NormalTransform>;
struct Material { base_color:vec4<f32>, metallic:f32, roughness:f32, double_sided:u32, pad:u32 };
@group(2) @binding(0) var<uniform> material:Material;
@group(2) @binding(1) var base_color:texture_2d<f32>;
@group(2) @binding(2) var base_sampler:sampler;
@group(2) @binding(3) var metallic_roughness:texture_2d<f32>;
@group(2) @binding(4) var mr_sampler:sampler;
// Frustum visibility assumes this camera * model * original bounded position.
// Position-displacing reloads must use Visibility::Unculled. Layout checks cannot certify bounds.
const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) slot: u32,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) @interpolate(flat) winding:f32,
};
fn unit_normal(n:vec3<f32>) -> vec3<f32> {
    let scale=max(max(abs(n.x),abs(n.y)),abs(n.z));
    return normalize(n/max(scale,1e-30));
}
@vertex fn vs(input: VertexInput) -> VertexOutput {
    let model = models[input.slot];
    let normal = normals[input.slot];
    var output: VertexOutput;
    output.position = camera.view_projection * model * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.uv = input.uv;
    output.normal = unit_normal(mat3x3<f32>(normal.x.xyz,normal.y.xyz,normal.z.xyz)*unit_normal(input.normal));
    output.winding = normal.info.x;
    return output;
}
@fragment fn fs(input: VertexOutput, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {
    // Cull by source winding so mirrored slots remain in their original equal-depth order.
    if material.double_sided == 0u && front != (input.winding > 0.0) { discard; }
    let color=textureSample(base_color,base_sampler,input.uv);
    let mr=textureSample(metallic_roughness,mr_sampler,input.uv);
    var result=material.base_color.rgb*input.color.rgb*color.rgb*TINT;
    switch camera.view_mode.x {
        case 1u: {result=unit_normal(input.normal)*0.5+vec3<f32>(0.5);}
        case 2u: {result=vec3<f32>(material.metallic*mr.b);}
        case 3u: {result=vec3<f32>(material.roughness*mr.g);}
        default: {}
    }
    return vec4<f32>(result,1.0);
}
