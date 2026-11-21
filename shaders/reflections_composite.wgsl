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
// Edge-aware glossy denoise inputs (bindings 4-6, bound only by rl_filter_fs).
@group(0) @binding(4) var rl_glossy_raw:texture_2d<f32>;
@group(0) @binding(5) var rl_glossy_meta:texture_2d<u32>;
@group(0) @binding(6) var rl_primary_hit:texture_2d<u32>;
const RL_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);

fn rl_oct_decode(word:u32)->vec3<f32> {
    let e=unpack2x16snorm(word);
    let z=1.0-abs(e.x)-abs(e.y);
    let sign=select(vec2<f32>(-1.0),vec2<f32>(1.0),e>=vec2<f32>(0.0));
    let xy=select((vec2<f32>(1.0)-abs(e.yx))*sign,e,z>=0.0);
    return normalize(vec3<f32>(xy,z));
}

struct RlCompositePoint { @builtin(position) position:vec4<f32>, @location(0) @interpolate(flat) entry:u32 };
// An entry's point at its pixel's centre.
@vertex fn rl_composite_vs(@builtin(vertex_index) entry:u32)->RlCompositePoint {
    let pixel=rl_glossy.entries[entry*3u+2u].x;
    let size=vec2<f32>(textureDimensions(rl_reflectance));
    let centre=vec2<f32>(f32(pixel&0xffffu),f32(pixel>>16u))+0.5;
    let ndc=vec2<f32>(centre.x/size.x*2.0-1.0,1.0-centre.y/size.y*2.0);
    return RlCompositePoint(vec4<f32>(ndc,0.0,1.0),entry);
}
// Unweighted strata mean (rgb) and valid flag (w = 1 when valid, 0 otherwise) for one list entry.
fn rl_entry_mean(entry:u32)->vec4<f32> {
    let b=rl_glossy.entries[entry*3u+1u];
    let c=rl_glossy.entries[entry*3u+2u];
    let first=c.y;
    let count=(b.w>>16u)&31u;
    if count==0u {return vec4<f32>(0.0);}
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
    return vec4<f32>(sum/f32(valid),1.0);
}
@fragment fn rl_composite_fs(point:RlCompositePoint)->@location(0) vec4<f32> {
    let c=rl_glossy.entries[point.entry*3u+2u];
    let p=vec2<i32>(vec2<u32>(c.x&0xffffu,c.x>>16u));
    let reflectance=textureLoad(rl_reflectance,p,0);
    if !(reflectance.a>0.0) {return vec4<f32>(0.0);}
    let mean=rl_entry_mean(point.entry);
    if mean.a<=0.0 {return vec4<f32>(0.0);}
    return vec4<f32>(reflectance.a*reflectance.rgb*mean.rgb,0.0);
}
// Pass 1 of the denoised composite: scatter each glossy pixel's raw strata mean + GGX alpha and
// its octahedral lobe normal into screen-space scratch targets (cleared to 0 before draw).
struct RlScatterOut { @location(0) raw:vec4<f32>, @location(1) normal_oct:u32 };
@fragment fn rl_scatter_fs(point:RlCompositePoint)->RlScatterOut {
    let b=rl_glossy.entries[point.entry*3u+1u];
    let c=rl_glossy.entries[point.entry*3u+2u];
    let p=vec2<i32>(vec2<u32>(c.x&0xffffu,c.x>>16u));
    let reflectance=textureLoad(rl_reflectance,p,0);
    if !(reflectance.a>0.0) {return RlScatterOut(vec4<f32>(0.0),0u);}
    let mean=rl_entry_mean(point.entry);
    if mean.a<=0.0 {return RlScatterOut(vec4<f32>(0.0),0u);}
    let alpha=max(unpack2x16float(b.w&0xffffu).x,1e-4);
    return RlScatterOut(vec4<f32>(mean.rgb,alpha),b.x);
}
// Pass 2 of the denoised composite: edge-aware bilateral filter over listed glossy pixels only,
// locked to placement ID, lobe normal and roughness, then additively composited into HDR.
@fragment fn rl_filter_fs(point:RlCompositePoint)->@location(0) vec4<f32> {
    let b=rl_glossy.entries[point.entry*3u+1u];
    let c=rl_glossy.entries[point.entry*3u+2u];
    let p=vec2<i32>(vec2<u32>(c.x&0xffffu,c.x>>16u));
    let reflectance=textureLoad(rl_reflectance,p,0);
    if !(reflectance.a>0.0) {return vec4<f32>(0.0);}
    let center=textureLoad(rl_glossy_raw,p,0);
    if !(center.a>0.0) {return vec4<f32>(0.0);}
    let alpha=center.a;
    let rough=sqrt(max(alpha,0.0));
    if rough<0.08 {
        return vec4<f32>(reflectance.a*reflectance.rgb*center.rgb,0.0);
    }
    let size=vec2<i32>(textureDimensions(rl_glossy_raw));
    let id=textureLoad(rl_primary_hit,p,0).x;
    let n=rl_oct_decode(b.x);
    let wide=rough>=0.18;
    let rad=select(1,2,wide);
    let sigma=select(
        mix(0.65,1.15,clamp((rough-0.08)/0.10,0.0,1.0)),
        mix(1.20,1.80,clamp((rough-0.18)/0.22,0.0,1.0)),
        wide
    );
    let inv2s2=0.5/(sigma*sigma);
    let inv_alpha=2.0/max(alpha,0.02);
    var sum=center.rgb;
    var wsum=1.0;
    for(var y=-rad;y<=rad;y+=1) {
        for(var x=-rad;x<=rad;x+=1) {
            if x==0 && y==0 {continue;}
            let q=p+vec2<i32>(x,y);
            if any(q<vec2<i32>(0)) || any(q>=size) {continue;}
            let rq=textureLoad(rl_glossy_raw,q,0);
            if !(rq.a>0.0) {continue;}
            if textureLoad(rl_primary_hit,q,0).x!=id {continue;}
            let nq=rl_oct_decode(textureLoad(rl_glossy_meta,q,0).x);
            let ndot=dot(n,nq);
            if ndot<0.80 {continue;}
            let n2=ndot*ndot;
            let n4=n2*n2;
            let w=exp(-f32(x*x+y*y)*inv2s2-abs(rq.a-alpha)*inv_alpha)*n4*n4;
            sum+=rq.rgb*w;
            wsum+=w;
        }
    }
    return vec4<f32>(reflectance.a*reflectance.rgb*(sum/wsum),0.0);
}
