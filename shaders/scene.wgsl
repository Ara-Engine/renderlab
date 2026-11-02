struct Camera { view_projection: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> models: array<mat4x4<f32>>;
// Frustum visibility assumes this camera * model * original bounded position.
// Position-displacing reloads must use Visibility::Unculled. Layout checks cannot certify bounds.
const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) slot: u32,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};
@vertex fn vs(input: VertexInput) -> VertexOutput {
    let model = models[input.slot];
    var output: VertexOutput;
    output.position = camera.view_projection * model * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    return output;
}
@fragment fn fs(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color * TINT, 1.0);
}
