// U-RTVIS held sun-shadow budget (docs/rtvis.md), appended to the reconstruction's trace
// module by temporal_reconstruction.rs `trace_source_held`. Each trace entry asks
// `rtvis_hold` before its receiver's estimate and records the result with `rtvis_store`.
// History word per pixel: class << 8 | streak, class 1 every direction blocked, 2 every
// direction clear, 0 anything else (fractional, a miss, a skip, a moving receiver's start).
struct RtvisHeld { previous:mat4x4<f32>, words:vec4<u32>, lo:vec4<f32>, hi:vec4<f32>, light:vec4<f32> };
@group(0) @binding(50) var<uniform> rtvis_held:RtvisHeld;
@group(0) @binding(51) var rtvis_history_in:texture_2d<u32>;
@group(0) @binding(52) var rtvis_history_out:texture_storage_2d<r32uint,write>;
// HELD_STOP: the probe count at which an agreeing receiver stops (never by default, and in
// every entry that does not call rtvis_hold), and the class it must agree with.
var<private> tr1_held_probes:u32=0xffffffffu;
var<private> tr1_held_lit:bool;
// The reprojected centre texel's history word (0: none), for this frame's streak.
var<private> rtvis_centre:u32;

// Whether point x may sit in the sun shadow of a placement that moved or is skinned
// (temporal_zones.wgsl tr1_caster_shadow's test over RtvisHeld's swept bounds).
fn rtvis_swept(x:vec3<f32>)->bool {
    if rtvis_held.lo.w==0.0 {return false;}
    let lo=rtvis_held.lo.xyz;let hi=rtvis_held.hi.xyz;
    let reach=length(max(abs(x-lo),abs(x-hi)));
    let grow=vec3<f32>(reach*rtvis_held.light.w);
    let rcp=vec3<f32>(1.0)/rtvis_held.light.xyz;
    let a=(lo-grow-x)*rcp;let b=(hi+grow-x)*rcp;
    let t0=min(a,b);let t1=max(a,b);
    return min(min(t1.x,t1.y),t1.z)>=max(max(max(t0.x,t0.y),t0.z),0.0);
}
// The receiver's placement moved since the previous frame (object_motion.wgsl motion_rigid).
fn rtvis_moved(slot:u32)->bool {
    let a=models[slot];let b=models[arrayLength(&models)/2u+slot];
    return any(bitcast<vec4<u32>>(a[0])!=bitcast<vec4<u32>>(b[0]))||any(bitcast<vec4<u32>>(a[1])!=bitcast<vec4<u32>>(b[1]))
        ||any(bitcast<vec4<u32>>(a[2])!=bitcast<vec4<u32>>(b[2]))||any(bitcast<vec4<u32>>(a[3])!=bitcast<vec4<u32>>(b[3]));
}
// Sets HELD_STOP for this receiver: a still, unskinned receiver outside every moving
// caster's swept shadow whose reprojected 3x3 history is one class for HELD_STREAK frames.
fn rtvis_hold(receiver:Receiver,slot:u32) {
    tr1_held_probes=0xffffffffu;rtvis_centre=0u;
    let batch=primary_placements[slot]&0x1fffffffu;
    if batch_skinned(primary_batch_info(batch)) || rtvis_moved(slot) {return;}
    let c=rtvis_held.previous*vec4<f32>(receiver.world,1.0);
    if c.w<=0.0 {return;}
    let size=vec2<i32>(textureDimensions(rtvis_history_in));
    let q=vec2<i32>(floor(fma(c.xy/c.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size)));
    if any(q<vec2<i32>(1)) || any(q>=size-vec2<i32>(1)) {return;}
    rtvis_centre=textureLoad(rtvis_history_in,q,0).x;
    let cls=rtvis_centre>>8u;
    if rtvis_held.words.x==0u || cls==0u || rtvis_swept(receiver.world) {return;}
    for(var k=0;k<9;k+=1) {
        let h=textureLoad(rtvis_history_in,q+vec2<i32>(k%3-1,k/3-1),0).x;
        if (h>>8u)!=cls || (h&0xffu)<rtvis_held.words.z {return;}
    }
    tr1_held_probes=rtvis_held.words.y;tr1_held_lit=cls==2u;
}
// This frame's history word at p from the receiver's count over tr1_rays directions.
fn rtvis_store(p:vec2<i32>,visible:u32) {
    var h=0u;
    if visible!=FRESH_SKIP && (visible==0u || visible==tr1_rays) {
        let cls=select(2u,1u,visible==0u);
        h=(cls<<8u)|select(1u,min((rtvis_centre&0xffu)+1u,255u),(rtvis_centre>>8u)==cls);
    }
    textureStore(rtvis_history_out,p,vec4<u32>(h));
}
