struct Camera { view_projection: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;
const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};
@vertex fn vs(input: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    var output: VertexOutput;
    output.position = camera.view_projection * model * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    return output;
}
@fragment fn fs(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color * TINT, 1.0);
}
