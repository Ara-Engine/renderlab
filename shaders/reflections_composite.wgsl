// U-RTVIS deferred reflection composite (docs/design-reflection-pass.md), RENDERLAB_RL_DEFERRED=1
// with RENDERLAB_RL_PASS=1: after rl_trace_pixels and reflection_hits, each glossy pixel's strata
// samples (its range of R) are averaged as the inline loop averages them, with the same
// lone-stratum bound, and the HDR target gains w x reflectance x that mean (RL#79's binding 62:
// reflectance rgb times fog transmittance, w in alpha). Shading left rgb 0 with w, so nothing is
// subtracted (root, 2026-11-13). Additive blending; alpha (TR1's view hint) is kept.
@group(0) @binding(0) var rl_ranges:texture_2d<u32>;
@group(0) @binding(1) var rl_samples:texture_2d<f32>;
@group(0) @binding(2) var rl_reflectance:texture_2d<f32>;
// x the lone-stratum bound (primary_frame.reflection.z), y RL_GLOSSY_RAYS.
@group(0) @binding(3) var<uniform> rl_composite_frame:vec4<f32>;
const RL_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);

@vertex fn rl_composite_vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return vec4<f32>(uv*2.0-1.0,0.0,1.0);
}
@fragment fn rl_composite_fs(@builtin(position) at:vec4<f32>)->@location(0) vec4<f32> {
    let p=vec2<i32>(at.xy);
    // First slot << 5 | slot count (rl_trace_pixels).
    let word=textureLoad(rl_ranges,p,0).x;
    let range=vec2<u32>(word>>5u,word&31u);
    if range.y==0u {return vec4<f32>(0.0);}
    let reflectance=textureLoad(rl_reflectance,p,0);
    if !(reflectance.a>0.0) {return vec4<f32>(0.0);}
    let width=u32(textureDimensions(rl_samples).x);
    var sum=vec3<f32>(0.0);var top=vec4<f32>(0.0);var valid=0u;
    for(var k=0u;k<range.y;k+=1u) {
        let slot=range.x+k;
        let s=textureLoad(rl_samples,vec2<i32>(vec2<u32>(slot%width,slot/width)),0);
        if s.a!=1.0 {continue;}
        sum+=s.rgb;
        let level=dot(s.rgb,RL_LUMA);
        if level>top.w {top=vec4<f32>(s.rgb,level);}
        valid+=1u;
    }
    if valid==0u {return vec4<f32>(0.0);}
    let lone=rl_composite_frame.x;
    if lone>0.0 && valid>1u && f32(range.y)>=rl_composite_frame.y {
        let rest=max(dot(sum,RL_LUMA)-top.w,0.0)*lone;
        if top.w>rest {sum-=top.rgb*(1.0-rest/top.w);}
    }
    return vec4<f32>(reflectance.a*reflectance.rgb*(sum/f32(valid)),0.0);
}
