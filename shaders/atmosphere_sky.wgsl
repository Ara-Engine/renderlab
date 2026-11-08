// ---- AT1 raster sky backdrop: a full-screen triangle at the reversed-Z far plane, drawn
// last in the lit mesh pass with depth GreaterEqual, so it lands only on samples no
// surface covered (src/atmosphere.rs composes the shared definitions and the API first).
@group(3) @binding(3) var dfg_sampler:sampler;
@vertex fn vs_sky(@builtin(vertex_index) id:u32) -> @builtin(position) vec4<f32> {
    let xy=vec2<f32>(f32((id<<1u)&2u),f32(id&2u));
    return vec4<f32>(xy*2.0-vec2<f32>(1.0),0.0,1.0);
}
@fragment fn fs_sky(@builtin(position) position:vec4<f32>) -> @location(0) vec4<f32> {
    let uv=position.xy/atmosphere.viewport.xy;
    return vec4<f32>(atmosphere_sky(at_view_direction(at_uv_ndc(uv))),1.0);
}
