// U-RTVIS reflection pass, shading side (docs/design-reflection-pass.md): each glossy receiver
// writes its lobe for rl_trace (shaders/reflections_trace.wgsl). Replaces the stubs in
// reflections.wgsl in the wgpu primary shading modules while RENDERLAB_RL_PASS=1
// (src/program.rs with_reflection_pass). The inline trace still shades the picture.
// L, per pixel: a = (ray origin, pack2x16float(cone width at the origin, spread per metre
// before the stratum's sub-cone)); b = (octahedral lobe normal, view direction, geometric side,
// GGX alpha as f16 (bits 0-15) | this frame's tag (bits 16-26) | the strata count, at most 31
// (bits 27-31)). A tag other than the frame's marks a stale texel.
@group(0) @binding(56) var rl_lobe_a:texture_storage_2d<rgba32float,write>;
@group(0) @binding(57) var rl_lobe_b:texture_storage_2d<rgba32uint,write>;
// x the frame counter, y the slot grid's width, z 1 while the pass runs, w 1 with the deferred
// composite (RENDERLAB_RL_DEFERRED=1: shading leaves glossy reflections to it).
@group(0) @binding(58) var<uniform> rl_pass_frame:vec4<u32>;
const RL_PASS:bool=true;
fn rl_composite()->bool { return rl_pass_frame.w!=0u; }
// The frame's tag, exact in f16.
fn rl_pass_tag()->u32 { return rl_pass_frame.x%2048u; }
fn rl_lobe_store(pixel:vec2<u32>,origin:vec3<f32>,n:vec3<f32>,v:vec3<f32>,side:vec3<f32>,alpha:f32,width:f32,spread:f32,count:u32) {
    if rl_pass_frame.z==0u {return;}
    let p=vec2<i32>(pixel);
    textureStore(rl_lobe_a,p,vec4<f32>(origin,bitcast<f32>(pack2x16float(vec2<f32>(width,spread)))));
    let word=(pack2x16float(vec2<f32>(alpha,0.0))&0xffffu)|(rl_pass_tag()<<16u)|(min(count,31u)<<27u);
    textureStore(rl_lobe_b,p,vec4<u32>(rl_oct_encode(n),rl_oct_encode(v),rl_oct_encode(side),word));
}
