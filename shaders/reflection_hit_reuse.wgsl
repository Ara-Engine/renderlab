// ---- S4a reflection hits from the previous frame (docs/design-hit-reuse.md) ----
// RENDERLAB_RL_HIT_REUSE (src/program.rs `with_hit_reuse`): composed into traced modules ahead
// of the cache-hit hook, which both the inline strata loop and the compacted pass's hit entry
// call before shading a hit. A hit that passes every test below takes TR1's resolved colour
// where its own surface point was last frame, instead of `surface` and `shade`; any failure
// shades it as before.
// The prefix of TR1's frame record (temporal_resolve.wgsl Tr1Frame) this test reads.
struct RlReuseFrame {
    previous:mat4x4<f32>, current:mat4x4<f32>, previous_inverse:mat4x4<f32>,
    previous_eye:vec4<f32>, current_eye:vec4<f32>,
    // z: the factor carrying exposed history to this frame's exposure (0: no TR1 frame bound).
    jitter:vec4<f32>,
    background:vec4<f32>, windows:vec4<f32>, starts:vec4<f32>,
    // x: nonzero when this frame restarts every pixel (cut, first frame, reset events).
    flags:vec4<u32>,
};
@group(0) @binding(66) var<storage,read> rl_reuse_frame:RlReuseFrame;
// The history TR1 reads this frame: exposed mean and sample weight; placement + 1 (bits 0-22),
// viewed side (bit 23), MQ2's view hint code (bits 24-31, temporal_resolve.wgsl
// tr1_view_code) and view distance (f32 bits in y).
// TR1's two history sides (67/68 side 0, 73/74 side 1) and which one its resolve reads this
// frame (75 x): parity-free bindings for cached groups. A backend that binds the read side to
// both pairs leaves 75 at zero.
@group(0) @binding(67) var rl_reuse_colour:texture_2d<f32>;
@group(0) @binding(68) var rl_reuse_anchor:texture_2d<u32>;
@group(0) @binding(73) var rl_reuse_colour_1:texture_2d<f32>;
@group(0) @binding(74) var rl_reuse_anchor_1:texture_2d<u32>;
@group(0) @binding(75) var<uniform> rl_reuse_side:vec4<u32>;
fn rl_reuse_colour_at(q:vec2<i32>)->vec4<f32> {
    if rl_reuse_side.x==0u {return textureLoad(rl_reuse_colour,q,0);}
    return textureLoad(rl_reuse_colour_1,q,0);
}
fn rl_reuse_anchor_at(q:vec2<i32>)->vec4<u32> {
    if rl_reuse_side.x==0u {return textureLoad(rl_reuse_anchor,q,0);}
    return textureLoad(rl_reuse_anchor_1,q,0);
}
// Tunables (RENDERLAB_RL_HIT_REUSE_TUNE): the same-surface test's relative and absolute view
// distance tolerance, the history weight floor, the fallback view test's roughness floor, the
// atmosphere's transmittance floor, and the measurement mode (0 reuse; 1 reused hits 1 and
// shaded 0; 2 every hit 1; 3 every hit 0: the reused share is (1 - 3) / (2 - 3)).
const RL_REUSE_DEPTH:f32=0.01;
const RL_REUSE_SLACK:f32=0.02;
const RL_REUSE_WEIGHT:f32=4.0;
const RL_REUSE_ROUGH:f32=0.5;
const RL_REUSE_TRANSMIT:f32=0.5;
const RL_REUSE_MODE:u32=0u;
// The view test: with RL_REUSE_HINT, each texel's MQ2 hint bounds how far the view-dependent
// part slides between last frame's camera direction and this ray's (under RL_REUSE_VIEW
// pixels of its own image); without it, only non-metal hits at least RL_REUSE_ROUGH rough.
const RL_REUSE_HINT:bool=true;
const RL_REUSE_VIEW:f32=0.5;
// RL_REUSE_FOOT > 0: on a known non-metal hit, the bound grows to RL_REUSE_FOOT times the
// receiving cone's footprint at the hit in last frame's pixels there (a glossy reflection blurs
// the hit over that footprint), never below RL_REUSE_VIEW: a mirror-like receiver's cone is
// near zero, so it keeps the half-pixel bound, since a sharp reflection shows a wrong hit exactly.
const RL_REUSE_FOOT:f32=0.0;
// The hit's material class: 0 non-metal, 1 metal (never reused), 2 unknown (the trace kernel
// binds no material group: no footprint scaling, the hint's bound alone).
fn rl_reuse_metal(batch:u32)->u32 {
    return select(0u,1u,primary_materials[batch].value.metallic>=0.1);
}
// MQ2's hint - 1 from its anchor code: 0 for code 0 (view-independent), else ten codes an
// octave about code 81 (temporal_resolve.wgsl tr1_view_code; never below the stored value).
fn rl_reuse_view(code:u32)->f32 {
    return select(exp2((f32(code)-81.0)/10.0),0.0,code==0u);
}
// 6 without the hint: only non-metal hits at least RL_REUSE_ROUGH rough, whose own view
// dependence is small. With the hint, src/program.rs composes `return true;` here, so modules
// without the material group (the trace kernel) never name it.
fn rl_reuse_material(batch:u32)->bool {
    let material=primary_materials[batch].value;
    return material.metallic<0.1 && material.roughness>=RL_REUSE_ROUGH;
}
// The placement's previous transform (object_motion.wgsl: the model table's second half).
fn rl_reuse_previous_model(slot:u32)->mat4x4<f32> {
    return models[arrayLength(&models)/2u+slot];
}
// The hit's radiance from last frame's resolved colour (unexposed, as the cache hook returns
// it), with w 1; w 0 when the hit must be shaded.
fn rl_hit_reuse(record:vec4<u32>,d:vec3<f32>,footprint:f32)->vec4<f32> {
    // 1. A bound TR1 frame that does not restart (cuts, first frames and reset events never reuse).
    if !(rl_reuse_frame.jitter.z>0.0) || rl_reuse_frame.flags.x!=0u {return vec4<f32>(0.0);}
    let hit=primary_decode(record);
    // Skinned hits shade (their previous surface needs SK1's previous vertices).
    if batch_skinned(primary_batch_info(hit.batch)) {return vec4<f32>(0.0);}
    if !rl_reuse_material(hit.batch) {return vec4<f32>(0.0);}
    let metal=rl_reuse_metal(hit.batch);
    if metal==1u {return vec4<f32>(0.0);}
    let ta=primary_input(hit.batch,hit.ia,hit.placement);
    let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    let object=ta.position*w.x+tb.position*w.y+tc.position*w.z;
    let world=world_position(object,hit.placement).xyz;
    // 2. Outside this frame's restart zones: changed lights, a moving caster's swept shadow.
    if tr1_relit(world) || tr1_caster_shadow(world) {return vec4<f32>(0.0);}
    // 3. The hit's own previous position: the reflected surface's motion, not the receiver's.
    let previous=(rl_reuse_previous_model(hit.placement)*vec4<f32>(object,1.0)).xyz;
    // 4. In last frame's view.
    let clip=rl_reuse_frame.previous*vec4<f32>(previous,1.0);
    if clip.w<=0.0 {return vec4<f32>(0.0);}
    let size=vec2<f32>(textureDimensions(rl_reuse_colour));
    let ndc=clip.xy/clip.w;
    let at=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5)*size-0.5;
    if any(at< vec2<f32>(0.0)) || any(at>size-1.0) {return vec4<f32>(0.0);}
    let base=vec2<i32>(floor(at));
    let f=at-floor(at);
    let distance=length(rl_reuse_frame.previous_eye.xyz-previous);
    // 6 with the hint: the turn between last frame's view of the point and this ray's.
    let turn=acos(clamp(dot(-d,(rl_reuse_frame.previous_eye.xyz-previous)/max(distance,1e-6)),-1.0,1.0));
    // The footprint in last frame's pixels at the hit (previous_eye.w: radians per pixel).
    let pixels=footprint/max(distance*rl_reuse_frame.previous_eye.w,1e-6);
    let bound=select(RL_REUSE_VIEW,max(RL_REUSE_VIEW,RL_REUSE_FOOT*pixels),RL_REUSE_FOOT>0.0 && metal==0u);
    // 5. The same surface at each bilinear texel (placement and view distance); 7. a history
    // old enough.
    var sum=vec3<f32>(0.0);var weight=0.0;var valid=0u;
    for(var k=0u;k<4u;k+=1u) {
        let o=vec2<i32>(i32(k&1u),i32(k>>1u));
        let q=base+o;
        let anchor=rl_reuse_anchor_at(q);
        let seen=bitcast<f32>(anchor.y);
        if (anchor.x&0x7fffffu)!=hit.placement+1u || abs(seen-distance)>distance*RL_REUSE_DEPTH+RL_REUSE_SLACK {continue;}
        if RL_REUSE_HINT && turn*rl_reuse_view(anchor.x>>24u)>bound {continue;}
        let colour=rl_reuse_colour_at(q);
        if !(colour.w>=RL_REUSE_WEIGHT) {continue;}
        let b=select(1.0-f.x,f.x,o.x==1)*select(1.0-f.y,f.y,o.y==1);
        sum+=colour.rgb*b;weight+=b;valid+=1u;
    }
    if valid<2u || !(weight>0.0) {return vec4<f32>(0.0);}
    // Into this frame's exposure, then out of the camera's atmosphere toward that surface:
    // atmosphere_surface is affine in its colour, c' = c T + S.
    let carried=sum/weight*rl_reuse_frame.jitter.z;
    let scatter=atmosphere_surface(vec3<f32>(0.0),world);
    let transmit=atmosphere_surface(vec3<f32>(1.0),world)-scatter;
    if any(transmit<vec3<f32>(RL_REUSE_TRANSMIT)) {return vec4<f32>(0.0);}
    let exposed=max(carried-scatter,vec3<f32>(0.0))/transmit;
    if RL_REUSE_MODE==1u || RL_REUSE_MODE==2u {return vec4<f32>(vec3<f32>(1.0/lighting.exposure),1.0);}
    return vec4<f32>(exposed/lighting.exposure,1.0);
}
// Whether the last rl_cache_hit call reused the hit (the hit kernel counts it).
var<private> rl_reused:bool;
// The cache-hit hook, reuse first (RL_REUSE_MODE's measurement forms short-circuit it).
fn rl_cache_hit(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,t:f32,cone:f32,rough:f32)->vec4<f32> {
    let reused=rl_hit_reuse(record,d,cone);
    rl_reused=reused.w>0.0;
    if RL_REUSE_MODE==2u {return vec4<f32>(vec3<f32>(1.0/lighting.exposure),1.0);}
    if RL_REUSE_MODE==3u {return vec4<f32>(0.0,0.0,0.0,1.0);}
    if reused.w>0.0 {return reused;}
    if RL_REUSE_MODE==1u {return vec4<f32>(0.0,0.0,0.0,1.0);}
    return rl_cache_hit_inner(record,origin,d,t,cone,rough);
}
// ---- end of S4a reflection hits from the previous frame ----
