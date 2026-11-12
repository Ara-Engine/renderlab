// U-RTVIS reflection history (docs/rtvis.md). Replaces the stubs in reflections.wgsl in
// the wgpu renderer's primary shading modules (src/program.rs rl_history_source). A glossy
// receiver whose reprojected history is its own surface traces one rotated stratum of its
// fixed set a frame and folds it into the stored mean, up to the set's count. A cut, a
// reset, a moved or skinned placement, a restart sphere, no matching record, or a stratum
// that disagrees with the stored mean beyond its noise traces the full set, as without it.
// Per pixel, two halves of four r32uint layers (written half: RlHistoryFrame.words.y):
// 0 pack2x16float(r,g), 1 pack2x16float(b, mean luminance^2), 2 (slot+1) | count << 24,
// 3 the receiver's clip w (f32 bits).
struct RlHistoryFrame { previous:mat4x4<f32>, words:vec4<u32> };
// words: x 1 when this frame may read the history (TR1 continues it: no cut or reset),
// y the half this frame writes, z the frame counter, w the exposure factor (f32 bits)
// carrying stored radiance to this frame.
@group(0) @binding(53) var<uniform> rl_history_frame:RlHistoryFrame;
@group(0) @binding(54) var rl_history:texture_storage_2d_array<r32uint,read_write>;
const RL_HISTORY:bool=true;
// The stored record's mean-square luminance, and whether the receiver moved over a pixel.
var<private> rl_hist_m2:f32;
var<private> rl_hist_moved:bool;

fn rl_history_index()->u32 { return rl_history_frame.words.z; }
// The receiver's placement moved since the previous frame (rtvis_held.wgsl rtvis_moved).
fn rl_history_placement_moved(slot:u32)->bool {
    let a=models[slot];let b=models[arrayLength(&models)/2u+slot];
    return any(bitcast<vec4<u32>>(a[0])!=bitcast<vec4<u32>>(b[0]))||any(bitcast<vec4<u32>>(a[1])!=bitcast<vec4<u32>>(b[1]))
        ||any(bitcast<vec4<u32>>(a[2])!=bitcast<vec4<u32>>(b[2]))||any(bitcast<vec4<u32>>(a[3])!=bitcast<vec4<u32>>(b[3]));
}
// The receiver's stored mean (rgb) and count (w, 0: no usable history).
fn rl_history_lookup(receiver:Receiver,world:vec3<f32>,pixel:vec2<u32>)->vec4<f32> {
    rl_hist_m2=0.0;rl_hist_moved=false;
    if rl_history_frame.words.x==0u {return vec4<f32>(0.0);}
    let slot=receiver.slot;
    let batch=primary_placements[slot]&0x1fffffffu;
    if batch_skinned(primary_batch_info(batch)) || rl_history_placement_moved(slot) {return vec4<f32>(0.0);}
    let c=rl_history_frame.previous*vec4<f32>(world,1.0);
    if c.w<=0.0 {return vec4<f32>(0.0);}
    let size=vec2<i32>(textureDimensions(rl_history));
    let at=fma(c.xy/c.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size);
    let q=vec2<i32>(floor(at));
    if any(q<vec2<i32>(0)) || any(q>=size) {return vec4<f32>(0.0);}
    rl_hist_moved=distance(at,vec2<f32>(pixel)+vec2<f32>(0.5))>0.5;
    let base=i32(4u*(1u-rl_history_frame.words.y));
    let id=textureLoad(rl_history,q,base+2).x;
    if (id&0xffffffu)!=slot+1u {return vec4<f32>(0.0);}
    let w=bitcast<f32>(textureLoad(rl_history,q,base+3).x);
    if abs(w-c.w)>0.02*c.w {return vec4<f32>(0.0);}
    let rg=unpack2x16float(textureLoad(rl_history,q,base).x);
    let bm=unpack2x16float(textureLoad(rl_history,q,base+1).x);
    // Stored radiance is exposed: carry it to this frame's exposure.
    let scale=bitcast<f32>(rl_history_frame.words.w);
    rl_hist_m2=bm.y*scale*scale;
    return vec4<f32>(vec3<f32>(rg,bm.x)*scale,f32(id>>24u));
}
// This frame's record at `pixel` (count 0: none).
fn rl_history_store(pixel:vec2<u32>,slot:u32,world:vec3<f32>,mean:vec3<f32>,m2:f32,count:f32) {
    let base=i32(4u*rl_history_frame.words.y);
    let p=vec2<i32>(pixel);
    let w=(camera.view_projection*vec4<f32>(world,1.0)).w;
    let m=min(mean,vec3<f32>(65504.0));
    textureStore(rl_history,p,base,vec4<u32>(pack2x16float(m.rg)));
    textureStore(rl_history,p,base+1,vec4<u32>(pack2x16float(vec2<f32>(m.b,min(m2,65504.0)))));
    textureStore(rl_history,p,base+2,vec4<u32>(select(0u,((slot+1u)&0xffffffu)|(u32(count)<<24u),count>0.0)));
    textureStore(rl_history,p,base+3,vec4<u32>(bitcast<u32>(w)));
}
