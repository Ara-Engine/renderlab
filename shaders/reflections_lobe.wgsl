// U-RTVIS reflection pass, shading side (docs/design-reflection-pass.md). Replaces the stubs in
// reflections.wgsl in the wgpu primary shading modules while the pass runs (src/program.rs
// pass_block). A reporting program's glossy receiver that fits the composite's room appends its
// lobe to the frame's glossy-pixel list, which rl_trace_pixels (shaders/reflections_trace.wgsl)
// and the composite walk; every other receiver keeps its inline estimate.
// The list: its length (a 16-byte header, cleared each frame), then three vec4<u32> an entry
// (48 bytes):
//   0: the ray origin (f32 bits) | pack2x16float(cone width at the origin, spread per metre
//      before the stratum's sub-cone);
//   1: octahedral lobe normal, view direction, geometric side | GGX alpha as f16 (bits 0-15) and
//      the strata count, at most 31 (bits 16-20);
//   2: the pixel (x | y << 16) | its first slot in R and H's room | 0 | 0.
struct RlGlossy { count:atomic<u32>, header:array<u32,3>, entries:array<vec4<u32>> };
@group(0) @binding(56) var<storage,read_write> rl_glossy:RlGlossy;
// x the frame counter, y the composite's room (R's slots and H's records), z the list's capacity
// in entries, w 1 while the pass runs.
@group(0) @binding(58) var<uniform> rl_pass_frame:vec4<u32>;
// The trace counters (reflection_pass.rs STATS; bound at ROOM_STATS here, since the primary
// shading layout holds the resolve's phase at 60), cleared each frame before shading: word 2 sums
// every glossy receiver's requested strata, so the room grows to the content; word 6 counts the
// receivers that did not fit (inline that frame). Words 4 and 5 are the Mac thread's S4a.
// Measurement runs (RL_ROOM_HISTOGRAM, src/program.rs) add: 7 and 8, the strata and receivers the
// floor keeps inline (traced fraction under RL_COMPOSITE_FLOOR, 5 %); 9 and 10, the listed ones
// whose traced fraction x reflectance weight is under 0.005.
@group(0) @binding(69) var<storage,read_write> rl_room_stats:array<atomic<u32>,16>;
const RL_ROOM_HISTOGRAM:bool=false;
const RL_PASS:bool=true;
// A receiver whose traced fraction is under this keeps today's inline estimate (no reservation, no
// entry, no composite): it renders as the default without the pass, and near-cutoff bands stay out
// of the room.
const RL_COMPOSITE_FLOOR:f32=0.05;
// Whether any of the receiver's `count` strata leaves above its geometric side, with the
// directions inline's loop draws (reflections.wgsl: the frame's radial offset, the pixel's
// rotation, the same VNDF sample and side test; no rays). A receiver with none is never listed:
// inline returns it with w = 0 (its untraced estimate), and the composite, averaging no valid
// stratum, would have added nothing for a traced share shading had already zeroed (the frame-18
// pulse on low-strata glossy receivers).
fn rl_any_stratum(count:u32,radial:f32,rotation:f32,basis:mat3x3<f32>,local_v:vec3<f32>,v:vec3<f32>,alpha:f32,side:vec3<f32>)->bool {
    for(var k=0u;k<count;k+=1u) {
        let u=vec2<f32>((f32(k)+radial)/f32(count),fract(rotation+f32(k)*0.6180339887));
        let d=reflect(-v,basis*rl_vndf(local_v,max(alpha,1e-6),u));
        if dot(d,side)>0.0 {return true;}
    }
    return false;
}
fn rl_composite_floor(traced:f32,count:u32)->bool {
    if traced>=RL_COMPOSITE_FLOOR {return true;}
    if RL_ROOM_HISTOGRAM {atomicAdd(&rl_room_stats[7],count);atomicAdd(&rl_room_stats[8],1u);}
    return false;
}
// Whether this module's program reports its reflectance for the deferred composite (it declares
// `const RL_DEFERRED`; src/program.rs `pass_block` sets it for deferred_variant modules). Only
// such programs leave their glossy lobes to the pass; every other program keeps its inline
// estimate.
const RL_DEFERRED_PROGRAM:bool=false;
// The receiver's first slot, from its reservation, for its list entry.
var<private> rl_room_first:u32;
// Inline overflow: a glossy receiver reserves its strata in the composite's room; one that does
// not fit keeps its inline estimate this frame (no list entry), so no frame is ever scaled. Which
// receivers overflow on such a frame follows the GPU's order; both estimators have the same mean.
fn rl_room_reserve(count:u32,traced:f32,weight:f32)->bool {
    if RL_ROOM_HISTOGRAM && traced*weight<0.005 {atomicAdd(&rl_room_stats[9],count);atomicAdd(&rl_room_stats[10],1u);}
    let first=atomicAdd(&rl_room_stats[2],count);
    if first+count<=rl_pass_frame.y {rl_room_first=first;return true;}
    atomicAdd(&rl_room_stats[6],1u);
    return false;
}
// The receiver's list entry. Entries never outnumber reservations, which fit the room, and the
// list holds the room's slots in entries, so every reserved receiver has one.
fn rl_lobe_store(pixel:vec2<u32>,origin:vec3<f32>,n:vec3<f32>,v:vec3<f32>,side:vec3<f32>,alpha:f32,width:f32,spread:f32,count:u32) {
    let entry=atomicAdd(&rl_glossy.count,1u);
    if entry>=rl_pass_frame.z {return;}
    let at=entry*3u;
    rl_glossy.entries[at]=vec4<u32>(bitcast<vec3<u32>>(origin),pack2x16float(vec2<f32>(width,spread)));
    let word=(pack2x16float(vec2<f32>(alpha,0.0))&0xffffu)|(min(count,31u)<<16u);
    rl_glossy.entries[at+1u]=vec4<u32>(rl_oct_encode(n),rl_oct_encode(v),rl_oct_encode(side),word);
    rl_glossy.entries[at+2u]=vec4<u32>(pixel.x|(pixel.y<<16u),rl_room_first,0u,0u);
}
