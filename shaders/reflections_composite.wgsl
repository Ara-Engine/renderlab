// U-RTVIS deferred reflection composite (docs/design-reflection-pass.md): after rl_trace_pixels
// and reflection_hits, each entry of the frame's glossy-pixel list draws one point at its pixel.
// Its strata samples (its slots of R) are averaged as the inline loop averages them, with the same
// lone-stratum bound, and the HDR target gains w x reflectance x that mean (RL#79's binding 62:
// reflectance rgb times fog transmittance, w in alpha). Shading left rgb 0 with w, so nothing is
// subtracted (root, 2026-11-13). Additive blending; alpha (TR1's view hint) is kept. The draw's
// vertex count is the list's length (rl_list_args), so a view with no glossy pixels draws none.
struct RlGlossyEntries { count:u32, header:array<u32,3>, entries:array<vec4<u32>> };
@group(0) @binding(0) var<storage,read> rl_glossy:RlGlossyEntries;
@group(0) @binding(1) var rl_samples:texture_2d<f32>;
@group(0) @binding(2) var rl_reflectance:texture_2d<f32>;
// x the lone-stratum bound (primary_frame.reflection.z), y RL_GLOSSY_RAYS.
@group(0) @binding(3) var<uniform> rl_composite_frame:vec4<f32>;
const RL_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);

struct RlCompositePoint { @builtin(position) position:vec4<f32>, @location(0) @interpolate(flat) entry:u32 };
// An entry's point at its pixel's centre.
@vertex fn rl_composite_vs(@builtin(vertex_index) entry:u32)->RlCompositePoint {
    let pixel=rl_glossy.entries[entry*3u+2u].x;
    let size=vec2<f32>(textureDimensions(rl_reflectance));
    let centre=vec2<f32>(f32(pixel&0xffffu),f32(pixel>>16u))+0.5;
    let ndc=vec2<f32>(centre.x/size.x*2.0-1.0,1.0-centre.y/size.y*2.0);
    return RlCompositePoint(vec4<f32>(ndc,0.0,1.0),entry);
}
@fragment fn rl_composite_fs(point:RlCompositePoint)->@location(0) vec4<f32> {
    let b=rl_glossy.entries[point.entry*3u+1u];
    let c=rl_glossy.entries[point.entry*3u+2u];
    let p=vec2<i32>(vec2<u32>(c.x&0xffffu,c.x>>16u));
    let first=c.y;
    let count=(b.w>>16u)&31u;
    let reflectance=textureLoad(rl_reflectance,p,0);
    if count==0u || !(reflectance.a>0.0) {return vec4<f32>(0.0);}
    let width=u32(textureDimensions(rl_samples).x);
    var sum=vec3<f32>(0.0);var top=vec4<f32>(0.0);var valid=0u;
    for(var k=0u;k<count;k+=1u) {
        let slot=first+k;
        let s=textureLoad(rl_samples,vec2<i32>(vec2<u32>(slot%width,slot/width)),0);
        if s.a!=1.0 {continue;}
        sum+=s.rgb;
        let level=dot(s.rgb,RL_LUMA);
        if level>top.w {top=vec4<f32>(s.rgb,level);}
        valid+=1u;
    }
    if valid==0u {return vec4<f32>(0.0);}
    let lone=rl_composite_frame.x;
    if lone>0.0 && valid>1u && f32(count)>=rl_composite_frame.y {
        let rest=max(dot(sum,RL_LUMA)-top.w,0.0)*lone;
        if top.w>rest {sum-=top.rgb*(1.0-rest/top.w);}
    }
    return vec4<f32>(reflectance.a*reflectance.rgb*(sum/f32(valid)),0.0);
}
