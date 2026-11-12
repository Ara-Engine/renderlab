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
// words: x 1 when this frame may read the history (a still camera, no cut or reset),
// y the half this frame writes, z the frame counter, w the exposure factor (f32 bits)
// carrying stored radiance to this frame.
@group(0) @binding(55) var<uniform> rl_history_frame:RlHistoryFrame;
@group(0) @binding(54) var rl_history:texture_storage_2d_array<r32uint,read_write>;
const RL_HISTORY:bool=true;
// The stored record's mean-square luminance, and whether the receiver moved over a pixel.
var<private> rl_hist_m2:f32;
var<private> rl_hist_moved:bool;
// RL_RESOLVE (src/program.rs RENDERLAB_RL_RESOLVE=1; the history then also reads under a moving
// camera): a trusted receiver's output averages its own new mean with its neighbours' previous
// means around the reprojected point, edge-aware (same placement, clip depth within
// RL_RESOLVE_DEPTH), binomial over 3x3, or 5x5 from roughness RL_RESOLVE_WIDE, each weighed by
// its history count. Its own history is first clamped to the neighbourhood's mean plus or minus
// RL_RESOLVE_CLAMP standard deviations. Records stay unfiltered, so the blur never accumulates.
const RL_RESOLVE:bool=false;
// RL_RESOLVE_PASS (RENDERLAB_RL_RESOLVE_PASS=1): the stored history is rl_resolve's output
// (shaders/reflections_resolve.wgsl). A trusted receiver shows it and leaves this frame's
// sample and the history it read for the resolve: layers 8 rg, 9 (b, mean luminance^2),
// 10 (slot+1) | roughness x 255 << 24, 11 clip w (negative: a full set), 12 the history's rg,
// 13 its (b, mean luminance^2), 14 its count (a full set's strata), 15 the frame counter + 1.
const RL_RESOLVE_PASS:bool=false;
// x: 0 for the pre-pass shading dispatch, 1 for the shading pass (set by a buffer copy between
// the two; src/reflection_pass.rs encode_resolved_shading).
@group(0) @binding(60) var<uniform> rl_phase:vec4<u32>;
fn rl_prepass()->bool { return RL_RESOLVE_PASS && rl_phase.x==0u; }
// This frame's resolved value at `pixel` (rgb, w 1), or w 0 where the pre-pass left no sample.
fn rl_resolved(pixel:vec2<u32>)->vec4<f32> {
    let p=vec2<i32>(pixel);
    if textureLoad(rl_history,p,15).x!=rl_history_frame.words.z+1u {return vec4<f32>(0.0);}
    // The a-trous filter's last pass (reflections_resolve.wgsl, layers 21-22).
    let rg=unpack2x16float(textureLoad(rl_history,p,21).x);
    let b=unpack2x16float(textureLoad(rl_history,p,22).x).x;
    return vec4<f32>(rg,b,1.0);
}
fn rl_sample_store(pixel:vec2<u32>,slot:u32,world:vec3<f32>,sample:vec3<f32>,m2:f32,full:bool,count:f32,rough:f32,seen:vec3<f32>,seen_m2:f32,dist:f32) {
    textureStore(rl_history,vec2<i32>(pixel),18,vec4<u32>(bitcast<u32>(dist)));
    let p=vec2<i32>(pixel);
    let w=(camera.view_projection*vec4<f32>(world,1.0)).w;
    let m=min(sample,vec3<f32>(65504.0));
    let h=min(seen,vec3<f32>(65504.0));
    textureStore(rl_history,p,8,vec4<u32>(pack2x16float(m.rg)));
    textureStore(rl_history,p,9,vec4<u32>(pack2x16float(vec2<f32>(m.b,min(m2,65504.0)))));
    textureStore(rl_history,p,10,vec4<u32>(((slot+1u)&0xffffffu)|(u32(clamp(rough,0.0,1.0)*255.0)<<24u)));
    textureStore(rl_history,p,11,vec4<u32>(bitcast<u32>(select(w,-w,full))));
    textureStore(rl_history,p,12,vec4<u32>(pack2x16float(h.rg)));
    textureStore(rl_history,p,13,vec4<u32>(pack2x16float(vec2<f32>(h.b,min(seen_m2,65504.0)))));
    // Layer 14: the count (low 8 bits) and the lobe's screen footprint in pixels (f16, high 16).
    textureStore(rl_history,p,14,vec4<u32>(min(u32(count),255u)|(pack2x16float(vec2<f32>(min(rl_footprint,65504.0),0.0))<<16u)));
    textureStore(rl_history,p,15,vec4<u32>(rl_history_frame.words.z+1u));
}
const RL_RESOLVE_WIDE:f32=0.3;
const RL_RESOLVE_DEPTH:f32=0.02;
const RL_RESOLVE_CLAMP:f32=2.0;
// The neighbours' weighted mean sum (rgb) and weight (w), and their luminance moments.
var<private> rl_hist_neigh:vec4<f32>;
var<private> rl_hist_neigh_moments:vec2<f32>;
// One neighbour record's mean (rgb) and its weight (w, 0: another surface or none).
fn rl_history_neighbour(q:vec2<i32>,base:i32,slot:u32,depth:f32,count:f32)->vec4<f32> {
    let size=vec2<i32>(textureDimensions(rl_history));
    if any(q<vec2<i32>(0)) || any(q>=size) {return vec4<f32>(0.0);}
    let id=textureLoad(rl_history,q,base+2).x;
    if (id&0xffffffu)!=slot+1u {return vec4<f32>(0.0);}
    let w=bitcast<f32>(textureLoad(rl_history,q,base+3).x);
    if abs(w-depth)>RL_RESOLVE_DEPTH*depth {return vec4<f32>(0.0);}
    let rg=unpack2x16float(textureLoad(rl_history,q,base).x);
    let b=unpack2x16float(textureLoad(rl_history,q,base+1).x).x;
    return vec4<f32>(rg,b,min(f32(id>>24u)/max(count,1.0),1.0));
}
// The neighbourhood of q (excluding q) into rl_hist_neigh and its moments.
fn rl_history_gather(q:vec2<i32>,base:i32,slot:u32,depth:f32,count:f32,wide:bool) {
    rl_hist_neigh=vec4<f32>(0.0);rl_hist_neigh_moments=vec2<f32>(0.0);
    let r=select(1,2,wide);
    let scale=bitcast<f32>(rl_history_frame.words.w);
    for(var y=-r;y<=r;y+=1) {
        for(var x=-r;x<=r;x+=1) {
            if x==0 && y==0 {continue;}
            let n=rl_history_neighbour(q+vec2<i32>(x,y),base,slot,depth,count);
            // Binomial taps: 1-2-1 over 3x3, 1-4-6-4-1 over 5x5.
            let kx=select(select(1.0,2.0,x==0),select(select(1.0,4.0,abs(x)==1),6.0,x==0),wide);
            let ky=select(select(1.0,2.0,y==0),select(select(1.0,4.0,abs(y)==1),6.0,y==0),wide);
            let w=kx*ky*n.w;
            let c=n.rgb*scale;
            rl_hist_neigh+=vec4<f32>(c*w,w);
            let l=dot(c,RL_LUMA);
            rl_hist_neigh_moments+=vec2<f32>(l,l*l)*w;
        }
    }
}

fn rl_history_index()->u32 { return rl_history_frame.words.z; }
// The receiver's placement moved since the previous frame (rtvis_held.wgsl rtvis_moved).
fn rl_history_placement_moved(slot:u32)->bool {
    let a=models[slot];let b=models[arrayLength(&models)/2u+slot];
    return any(bitcast<vec4<u32>>(a[0])!=bitcast<vec4<u32>>(b[0]))||any(bitcast<vec4<u32>>(a[1])!=bitcast<vec4<u32>>(b[1]))
        ||any(bitcast<vec4<u32>>(a[2])!=bitcast<vec4<u32>>(b[2]))||any(bitcast<vec4<u32>>(a[3])!=bitcast<vec4<u32>>(b[3]));
}
// The receiver's stored mean (rgb) and count (w, 0: no usable history).
// RL_VIRTUAL (with RL_RESOLVE_PASS): a lobe below RL_VIRTUAL_ROUGH reads its history where its
// reflected point (the receiver plus the mirror direction times the stored hit distance, both paths) was in
// the previous frame, if the same placement is there at a similar depth; otherwise, and for
// rougher lobes and misses, where its surface point was.
const RL_VIRTUAL:bool=true;
const RL_VIRTUAL_ROUGH:f32=0.3;
fn rl_history_lookup(receiver:Receiver,world:vec3<f32>,pixel:vec2<u32>,reflected:vec3<f32>)->vec4<f32> {
    rl_hist_m2=0.0;rl_hist_moved=false;rl_hist_neigh=vec4<f32>(0.0);
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
    var at_q=q;
    if RL_RESOLVE_PASS && RL_VIRTUAL && rl_hist_rough<RL_VIRTUAL_ROUGH {
        let dist=bitcast<f32>(textureLoad(rl_history,q,16+i32(1u-rl_history_frame.words.y)).x);
        if dist>0.0 {
            let v=rl_history_frame.previous*vec4<f32>(fma(reflected,vec3<f32>(dist),world),1.0);
            if v.w>0.0 {
                let qv=vec2<i32>(floor(fma(v.xy/v.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size)));
                if all(qv>=vec2<i32>(0)) && all(qv<size) && (textureLoad(rl_history,qv,base+2).x&0xffffffu)==slot+1u
                    && abs(bitcast<f32>(textureLoad(rl_history,qv,base+3).x)-c.w)<=0.1*c.w {at_q=qv;}
            }
        }
    }
    let rg=unpack2x16float(textureLoad(rl_history,at_q,base).x);
    let bm=unpack2x16float(textureLoad(rl_history,at_q,base+1).x);
    // Stored radiance is exposed: carry it to this frame's exposure.
    let scale=bitcast<f32>(rl_history_frame.words.w);
    rl_hist_m2=bm.y*scale*scale;
    var own=vec3<f32>(rg,bm.x)*scale;
    if RL_RESOLVE {
        let count=f32(id>>24u);
        rl_history_gather(q,base,slot,c.w,count,rl_hist_rough>=RL_RESOLVE_WIDE);
        let total=rl_hist_neigh.w;
        if total>0.0 {
            // Clamp the own history to the neighbourhood's spread (moving reflections).
            let mean=rl_hist_neigh_moments.x/total;
            let sigma=sqrt(max(rl_hist_neigh_moments.y/total-mean*mean,0.0));
            let l=dot(own,RL_LUMA);
            let lo=max(mean-RL_RESOLVE_CLAMP*sigma,0.0);let hi=mean+RL_RESOLVE_CLAMP*sigma+1e-4;
            if l>hi {own*=hi/l;} else if l<lo && l>0.0 {own*=lo/l;}
        }
    }
    return vec4<f32>(own,f32(textureLoad(rl_history,at_q,base+2).x>>24u));
}
// The receiver's output: its new mean with the neighbourhood gathered by the lookup
// (RL_RESOLVE), the centre tap weighed by its own history count; otherwise its mean.
fn rl_history_resolve(mean:vec3<f32>,folded:f32,count:f32)->vec3<f32> {
    if !RL_RESOLVE || rl_hist_neigh.w<=0.0 {return mean;}
    let centre=select(4.0,36.0,rl_hist_rough>=RL_RESOLVE_WIDE)*min(folded/max(count,1.0),1.0);
    return (mean*centre+rl_hist_neigh.rgb)/(centre+rl_hist_neigh.w);
}
// This frame's record at `pixel` (count 0: none).
fn rl_history_store(pixel:vec2<u32>,slot:u32,world:vec3<f32>,mean:vec3<f32>,m2:f32,count:f32) {
    let base=i32(4u*rl_history_frame.words.y);
    let p=vec2<i32>(pixel);
    let w=(camera.view_projection*vec4<f32>(world,1.0)).w;
    let m=min(mean,vec3<f32>(65504.0));
    textureStore(rl_history,p,base,vec4<u32>(pack2x16float(m.rg)));
    textureStore(rl_history,p,base+1,vec4<u32>(pack2x16float(vec2<f32>(m.b,min(m2,65504.0)))));
    textureStore(rl_history,p,base+2,vec4<u32>(select(0u,((slot+1u)&0xffffffu)|(min(u32(count),255u)<<24u),count>0.0)));
    textureStore(rl_history,p,base+3,vec4<u32>(bitcast<u32>(w)));
}
