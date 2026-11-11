// Conformance export only; transparent drawing requires caller material WGSL.
struct Parameters { gain:vec4<f32>, bias:vec4<f32> };
const DEFAULT_PARAMETERS:Parameters=Parameters(vec4<f32>(1.0),vec4<f32>(0.0));
struct Surface { rgb:vec3<f32> };
fn surface(input:SurfaceInput)->Surface {
    return Surface(sample_base_color(input).rgb*input.material.base_color.rgb*input.color.rgb);
}
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {
    return 0.125*s.rgb*params.gain.rgb+params.bias.rgb;
}
