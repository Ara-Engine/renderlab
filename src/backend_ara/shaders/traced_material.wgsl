// Conformance export only; runtime shading requires caller material WGSL.
struct Parameters { gain:vec4<f32>, bias:vec4<f32> };
const DEFAULT_PARAMETERS:Parameters=Parameters(vec4<f32>(1.0),vec4<f32>(0.0));
struct Surface { rgb:vec3<f32> };
fn surface(input:SurfaceInput)->Surface {
    let base=sample_base_color(input).rgb;
    let mr=sample_metallic_roughness(input).rgb;
    let normal=sample_normal_layer_grad(input,input.uv,input.uv_dx,input.uv_dy,1).rgb;
    return Surface((base+0.125*mr+0.25*normal)*input.material.base_color.rgb*input.color.rgb);
}
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {
    return s.rgb*params.gain.rgb+params.bias.rgb+abs(input.world)*params.gain.w;
}
