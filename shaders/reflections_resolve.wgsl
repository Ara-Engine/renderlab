// U-RTVIS rl_resolve (docs/design-reflection-pass.md), RENDERLAB_RL_RESOLVE_PASS=1: one pass after
// the frame's shading over the reflection history texture (shaders/reflections_history.wgsl).
// Each glossy receiver shaded this frame left a sample (one rotated stratum, or its full set
// where it had no trusted history) and the history it read (layers 8-15). The resolve filters
// this frame's samples over an edge-aware neighbourhood (same placement, clip depth within 2%,
// binomial 3x3, or 5x5 from roughness RL_RESOLVE_WIDE), clamps the history to that
// neighbourhood's mean plus or minus RL_RESOLVE_CLAMP standard deviations, blends, and writes
// the result as the next frame's history (this frame's half). The next frame's shading shows it.
struct RlHistoryFrame { previous:mat4x4<f32>, words:vec4<u32> };
@group(0) @binding(55) var<uniform> rl_history_frame:RlHistoryFrame;
@group(0) @binding(54) var rl_history:texture_storage_2d_array<r32uint,read_write>;
const RL_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);
const RL_RESOLVE_WIDE:f32=0.3;
const RL_RESOLVE_DEPTH:f32=0.02;
const RL_RESOLVE_CLAMP:f32=1.5;
// The most frames a resolved history keeps.
const RL_RESOLVE_CAP:f32=16.0;

struct RlSample { rgb:vec3<f32>, m2:f32, slot:u32, rough:f32, depth:f32, full:bool, fresh:bool };
fn rl_sample(p:vec2<i32>)->RlSample {
    let fresh=textureLoad(rl_history,p,15).x==rl_history_frame.words.z+1u;
    let rg=unpack2x16float(textureLoad(rl_history,p,8).x);
    let bm=unpack2x16float(textureLoad(rl_history,p,9).x);
    let id=textureLoad(rl_history,p,10).x;
    let w=bitcast<f32>(textureLoad(rl_history,p,11).x);
    return RlSample(vec3<f32>(rg,bm.x),bm.y,id&0xffffffu,f32(id>>24u)/255.0,abs(w),w<0.0,fresh);
}

@compute @workgroup_size(8,8) fn rl_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(rl_history);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    let s=rl_sample(p);
    if !s.fresh {return;}
    let base=i32(4u*rl_history_frame.words.y);
    var mean=s.rgb;var m2=s.m2;var count=1.0;
    if s.full {
        count=f32(textureLoad(rl_history,p,14).x);
    } else {
        // This frame's samples around p: one surface only.
        let wide=s.rough>=RL_RESOLVE_WIDE;
        let r=select(1,2,wide);
        var sum=vec4<f32>(0.0);var moments=vec2<f32>(0.0);
        for(var y=-r;y<=r;y+=1) {
            for(var x=-r;x<=r;x+=1) {
                let q=p+vec2<i32>(x,y);
                if any(q<vec2<i32>(0)) || any(q>=vec2<i32>(size)) {continue;}
                let n=rl_sample(q);
                if !n.fresh || n.slot!=s.slot || abs(n.depth-s.depth)>RL_RESOLVE_DEPTH*s.depth {continue;}
                let kx=select(select(1.0,2.0,x==0),select(select(1.0,4.0,abs(x)==1),6.0,x==0),wide);
                let ky=select(select(1.0,2.0,y==0),select(select(1.0,4.0,abs(y)==1),6.0,y==0),wide);
                // A neighbour's full set stands for several strata.
                let w=kx*ky*select(1.0,4.0,n.full);
                sum+=vec4<f32>(n.rgb*w,w);
                let l=dot(n.rgb,RL_LUMA);
                moments+=vec2<f32>(l,select(l*l,n.m2,n.full))*w;
            }
        }
        let spatial=sum.rgb/sum.w;
        let mu=moments.x/sum.w;
        let sigma=sqrt(max(moments.y/sum.w-mu*mu,0.0));
        // The history the shading read, clamped to this frame's neighbourhood.
        let hrg=unpack2x16float(textureLoad(rl_history,p,12).x);
        let hbm=unpack2x16float(textureLoad(rl_history,p,13).x);
        let seen=f32(textureLoad(rl_history,p,14).x);
        var history=vec3<f32>(hrg,hbm.x);
        let l=dot(history,RL_LUMA);
        let lo=max(mu-RL_RESOLVE_CLAMP*sigma,0.0);let hi=mu+RL_RESOLVE_CLAMP*sigma+1e-4;
        if l>hi {history*=hi/max(l,1e-8);} else if l<lo {history=select(spatial,history*lo/l,l>0.0);}
        count=min(seen+1.0,RL_RESOLVE_CAP);
        mean=mix(history,spatial,1.0/count);
        m2=mix(hbm.y,moments.y/sum.w,1.0/count);
    }
    let m=min(mean,vec3<f32>(65504.0));
    textureStore(rl_history,p,base,vec4<u32>(pack2x16float(m.rg)));
    textureStore(rl_history,p,base+1,vec4<u32>(pack2x16float(vec2<f32>(m.b,min(m2,65504.0)))));
    textureStore(rl_history,p,base+2,vec4<u32>(((s.slot)&0xffffffu)|(min(u32(count),255u)<<24u)));
    textureStore(rl_history,p,base+3,vec4<u32>(bitcast<u32>(s.depth)));
}
