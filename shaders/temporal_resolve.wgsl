// ---- TR1 temporal reconstruction (docs/m7-tr1.md) ----
// Appended to the non-RT engine head (primary_ray_gpu::non_rt_heads), so it compiles on
// devices without ray queries. One pass after the frame's shading: each pixel's history is
// fetched where its current sample's surface point was in the previous frame, and accepted
// only by surface identity (placement, viewed side, view distance) and engine state. No
// colour, luminance or neighbourhood statistic decides acceptance or weight: every program,
// however it shades, converges to the footprint integral of its own output.
//
// Patent notes (docs/m7-p0-claims.md §4.3, docs/m7-tr1.md): no neighbourhood colour box or
// variance (US 10,116,916 / 10,362,289), one history value per pixel (no fast history or
// retained list), no lighting difference or own-pixel moments (US 12,437,371, pending
// 2026/0162231). This pass's weights, ages, anchors and masks are read by this pass alone,
// next frame (W5): no later stage reads them.

// Frame state written by the CPU (temporal_reconstruction.rs Frame). Storage, not uniform:
// it feeds addressing (docs/m6-interactive-climb-2.md).
struct Tr1Frame {
    // Unjittered view-projections of the previous and this frame, and the inverse of the
    // previous one.
    previous:mat4x4<f32>,
    current:mat4x4<f32>,
    previous_inverse:mat4x4<f32>,
    // xyz: the previous eye; w: radians per pixel at the image centre.
    previous_eye:vec4<f32>,
    // xyz: this frame's eye; w: the previous frame's sample offset x.
    current_eye:vec4<f32>,
    // xy: this frame's sample offset from the pixel centre in pixels (y down); z: the
    // factor that carries exposed history to this frame's exposure; w: the previous
    // frame's sample offset y.
    jitter:vec4<f32>,
    // The exposed background (primary-miss) colour; stored samples are its f16 value.
    background:vec4<f32>,
    // x: static window; y: engine-event window cap; z: silhouette window; w: blur budget.
    windows:vec4<f32>,
    // x: Catmull-Rom motion window; y: spatial-start window cap; z, w: spatial-start tent
    // radius inside a surface and at silhouettes (0: the pixel's own sample).
    starts:vec4<f32>,
    // x: nonzero restarts every pixel (cut, first frame, reset events); y: TR1_* options;
    // z: nonzero when nothing moved (bit-equal unjittered view-projections; moved placements
    // restart), decided on the CPU: the GPU need not round two projections alike.
    flags:vec4<u32>,
};
const TR1_CATMULL_ROM:u32=1u;
const TR1_SPATIAL_START:u32=2u;
const TR1_SILHOUETTE_MIX:u32=4u;
const TR1_BOX:u32=8u;
const TR1_SILHOUETTE_SPATIAL:u32=16u;
@group(0) @binding(40) var<storage,read> tr1:Tr1Frame;
// This frame's shaded samples (a copy of the HDR target before this pass).
@group(0) @binding(41) var tr1_sample:texture_2d<f32>;
// The previous frame's history: exposed mean and sample weight; and its anchors.
@group(0) @binding(42) var tr1_colour_in:texture_2d<f32>;
@group(0) @binding(43) var tr1_anchor_in:texture_2d<u32>;
@group(0) @binding(44) var tr1_colour_out:texture_storage_2d<rgba32float,write>;
// Anchor of a pixel's history. x: placement + 1 (zero is background) in bits 0..22, the
// viewed side in bit 23, bit 24 when the history mixes surfaces (a silhouette pixel's);
// y: the latest sample's view distance, f32 bits.
@group(0) @binding(45) var tr1_anchor_out:texture_storage_2d<rg32uint,write>;
// The reconstructed image (the frame's HDR target).
@group(0) @binding(46) var tr1_output:texture_storage_2d<rgba16float,write>;
struct Tr1Vec4s { data:array<vec4<f32>> };
struct Tr1U32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> tr1_vertices:binding_array<Tr1Vec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> tr1_indices:binding_array<Tr1U32s,PRIMARY_BATCHES>;

const TR1_ID:u32=0x7fffffu;
const TR1_MIXED:u32=0x1000000u;

fn tr1_position(batch:u32,index:u32)->vec3<f32> {
    return tr1_vertices[batch].data[tr1_indices[batch].data[index]*3u].xyz;
}
fn tr1_dot(a:vec3<f32>,b:vec3<f32>)->f32 {return fma(a.x,b.x,fma(a.y,b.y,a.z*b.z));}

// The hit record's placement + 1 at a pixel, or zero: uncovered trace tiles keep stale
// records, and every pixel there is background.
fn tr1_id(p:vec2<i32>)->u32 {
    if textureLoad(primary_tiles,vec2<u32>(p)/PRIMARY_TILE,0).x==0u {return 0u;}
    return textureLoad(primary_hit,p,0).x;
}

// The current sample's surface: world point (w = 1) or background direction (w = 0), its
// identity, view distance and unit geometric normal.
struct Tr1Surface { point:vec4<f32>, id:u32, side:u32, distance:f32, normal:vec3<f32> };
// Placement + 1 of each 3x3 sample of this frame (set once per pixel by tr1_resolve).
var<private> tr1_ring:array<u32,9>;
fn tr1_surface(p:vec2<i32>,size:vec2<u32>)->Tr1Surface {
    let id=tr1_id(p);
    if id==0u {
        // The sample's own camera ray: primary_ray inverts this frame's jittered projection.
        let ray=primary_ray(vec2<u32>(p),size);
        let d=ray.far-ray.near;
        return Tr1Surface(vec4<f32>(d*inverseSqrt(tr1_dot(d,d)),0.0),0u,0u,0.0,vec3<f32>(0.0));
    }
    let record=textureLoad(primary_hit,p,0);
    let slot=id-1u;
    let batch=primary_placements[slot]&0x1fffffffu;
    let i=record.y*3u;
    let a=tr1_position(batch,i);let b=tr1_position(batch,i+1u);let c=tr1_position(batch,i+2u);
    let uv=bitcast<vec2<f32>>(record.zw);
    let x=world_position(receiver_point(a,b,c,vec3<f32>(1.0-uv.x-uv.y,uv)),slot).xyz;
    let wa=world_position(a,slot).xyz;
    let n=cross(world_position(b,slot).xyz-wa,world_position(c,slot).xyz-wa);
    let view=x-tr1.current_eye.xyz;
    let distance=sqrt(tr1_dot(view,view));
    let facing=tr1_dot(n,view);
    let unit=n*inverseSqrt(max(tr1_dot(n,n),1e-30));
    return Tr1Surface(vec4<f32>(x,1.0),id,u32(facing<=0.0),distance,unit);
}

// Continuous pixel position (pixel centres at + 0.5) of a point (w = 1) or direction (w = 0).
fn tr1_pixel(m:mat4x4<f32>,p:vec4<f32>,size:vec2<f32>)->vec2<f32> {
    let c=fma(m[0],vec4<f32>(p.x),fma(m[1],vec4<f32>(p.y),fma(m[2],vec4<f32>(p.z),m[3]*p.w)));
    let centre=0.5*size;
    return fma(c.xy/c.w,centre*vec2<f32>(1.0,-1.0),centre);
}

// A previous-frame tap's latest sample point, rebuilt from its own camera ray (its pixel
// and the previous frame's sample offset) and its anchor distance.
fn tr1_anchor_point(t:vec2<i32>,distance:f32,size:vec2<f32>)->vec3<f32> {
    let o=vec2<f32>(tr1.current_eye.w,tr1.jitter.w);
    let ndc=fma((vec2<f32>(t)+vec2<f32>(0.5)+o)/size,vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
    let m=tr1.previous_inverse;
    let xy=fma(m[0],vec4<f32>(ndc.x),m[1]*ndc.y);
    let near=xy+m[2]+m[3];
    let far=xy+m[3];
    let d=far.xyz/far.w-near.xyz/near.w;
    return fma(d,vec3<f32>(distance*inverseSqrt(tr1_dot(d,d))),tr1.previous_eye.xyz);
}

// Whether the previous-frame tap `t` holds history this pixel's surface may continue.
//
// Still (nothing moved: the tap is the pixel itself): the pixel's history is the running
// mean over its own footprint, continued while the tap's surface is still one of the 3x3
// samples' (an edge's surfaces, or the pixel's own); a surface that left the neighbourhood
// restarts it.
//
// Moving: the tap shows the surface when its latest sample has the same placement and
// viewed side and, for a hit, its sample point (rebuilt from its own ray) lies within four
// pixel footprints of the current point's tangent plane: a fold or ridge of the same mesh
// in front of or behind the surface fails, at any viewing angle. History that mixed
// surfaces (a silhouette pixel's) is continued only by a silhouette pixel, so it never
// trails into interiors or open background; another surface's only with TR1_SILHOUETTE_MIX.
fn tr1_accepts(t:vec2<i32>,s:Tr1Surface,silhouette:bool,still:bool,expected:f32,size:vec2<u32>)->bool {
    if any(t<vec2<i32>(0)) || any(t>=vec2<i32>(size)) {return false;}
    let a=textureLoad(tr1_anchor_in,t,0).xy;
    let id=a.x&TR1_ID;
    if still {
        for(var i=0;i<9;i+=1) {if tr1_ring[i]==id {return true;}}
        return false;
    }
    let mixed=(a.x&TR1_MIXED)!=0u;
    if id!=s.id {return mixed && silhouette && (tr1.flags.y&TR1_SILHOUETTE_MIX)!=0u;}
    if mixed && !silhouette {return false;}
    if s.id==0u {return true;}
    if (primary_placements[s.id-1u]&0x20000000u)!=0u && ((a.x>>23u)&1u)!=s.side {return false;}
    let point=tr1_anchor_point(t,bitcast<f32>(a.y),vec2<f32>(size));
    return abs(tr1_dot(s.normal,point-s.point.xyz))<=4.0*tr1.previous_eye.w*expected;
}

// Catmull-Rom weights of taps -1, 0, 1, 2 at fractional offset t.
fn tr1_catmull_rom(t:f32)->vec4<f32> {
    let t2=t*t;
    return vec4<f32>(t*fma(t,2.0-t,-1.0),fma(t2,fma(3.0,t,-5.0),2.0),
        t*fma(t,fma(-3.0,t,4.0),1.0),t2*(t-1.0))*0.5;
}

// The pixel's continued history: exposed mean (rgb) and weight (w), zero weight when no
// tap is accepted. Bilinear over the accepted inner taps, renormalised, with the bilinear
// weighted sum of their weights; Catmull-Rom over 4x4 when every tap is accepted, limited
// to the inner taps' range (history only: the undershoot fix, not a current-frame box).
struct Tr1History { colour:vec3<f32>, weight:f32, blur:f32, sharp:bool, still:bool };
fn tr1_history(p:vec2<i32>,s:Tr1Surface,silhouette:bool,size:vec2<u32>)->Tr1History {
    let fsize=vec2<f32>(size);
    let still=tr1.flags.z!=0u;
    var motion=vec2<f32>(0.0);
    if !still {motion=tr1_pixel(tr1.previous,s.point,fsize)-tr1_pixel(tr1.current,s.point,fsize);}
    let q=vec2<f32>(p)+motion;
    let base=vec2<i32>(floor(q));
    let f=q-floor(q);
    var expected=0.0;
    if s.id!=0u {
        let v=s.point.xyz-tr1.previous_eye.xyz;
        expected=sqrt(tr1_dot(v,v));
    }
    let scale=tr1.jitter.z;
    var h:Tr1History;
    h.still=still;
    var total=0.0;
    var colour=vec3<f32>(0.0);
    var lo=vec3<f32>(3.4e38);
    var hi=vec3<f32>(-3.4e38);
    var accepted=0u;
    for(var i=0;i<4;i+=1) {
        let o=vec2<i32>(i&1,i>>1u);
        let t=base+o;
        if tr1_accepts(t,s,silhouette,still,expected,size) {
            let w=select(1.0-f.x,f.x,o.x!=0)*select(1.0-f.y,f.y,o.y!=0);
            let x=textureLoad(tr1_colour_in,t,0);
            let c=x.xyz*scale;
            total+=w;
            colour=fma(c,vec3<f32>(w),colour);
            h.weight=fma(x.w,w,h.weight);
            lo=min(lo,c);hi=max(hi,c);
            accepted+=1u;
        }
    }
    if accepted==0u || total<=0.0 {return h;}
    h.colour=colour/total;
    h.blur=fma(f.x,1.0-f.x,f.y*(1.0-f.y));
    if accepted==4u && (tr1.flags.y&TR1_CATMULL_ROM)!=0u && h.blur>0.0 {
        let wx=tr1_catmull_rom(f.x);let wy=tr1_catmull_rom(f.y);
        var sharp=vec3<f32>(0.0);
        var outer=true;
        for(var i=0;i<16 && outer;i+=1) {
            let o=vec2<i32>(i&3,i>>2u)-vec2<i32>(1);
            let t=base+o;
            if all(o>=vec2<i32>(0)) && all(o<=vec2<i32>(1)) {
                sharp=fma(textureLoad(tr1_colour_in,t,0).xyz*scale,vec3<f32>(wx[i&3]*wy[i>>2u]),sharp);
            } else if tr1_accepts(t,s,silhouette,still,expected,size) {
                sharp=fma(textureLoad(tr1_colour_in,t,0).xyz*scale,vec3<f32>(wx[i&3]*wy[i>>2u]),sharp);
            } else {
                outer=false;
            }
        }
        if outer {
            h.colour=clamp(sharp,lo,hi);
            h.sharp=true;
        }
    }
    return h;
}

// A revealed pixel's start from this frame's samples only: the tent-weighted mean of the
// 3x3 samples showing its surface (every surface at a silhouette), each weighted at its
// jittered position, and that estimate's effective sample count.
fn tr1_spatial(p:vec2<i32>,id:u32,silhouette:bool,size:vec2<u32>)->vec4<f32> {
    let radius=select(tr1.starts.z,tr1.starts.w,silhouette);
    if radius<=0.0 {return vec4<f32>(textureLoad(tr1_sample,p,0).xyz,1.0);}
    var sum=vec3<f32>(0.0);
    var w1=0.0;
    var w2=0.0;
    for(var i=0;i<9;i+=1) {
        let o=vec2<i32>(i%3,i/3)-vec2<i32>(1);
        let t=p+o;
        if any(t<vec2<i32>(0)) || any(t>=vec2<i32>(size)) {continue;}
        if !silhouette && tr1_id(t)!=id {continue;}
        // At radius 2 and zero jitter, the 3x3 [1,2,1] filter; at radius 1 it interpolates
        // the samples bilinearly at the pixel centre.
        let d=abs(vec2<f32>(o)+tr1.jitter.xy)/radius;
        let w=max(1.0-d.x,0.0)*max(1.0-d.y,0.0);
        sum=fma(textureLoad(tr1_sample,t,0).xyz,vec3<f32>(w),sum);
        w1+=w;w2=fma(w,w,w2);
    }
    return vec4<f32>(sum/w1,w1*w1/w2);
}

@compute @workgroup_size(8,8) fn tr1_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(tr1_output);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    let s=tr1_surface(p,size);
    // A silhouette pixel: some 3x3 sample of this frame shows another surface.
    var silhouette=false;
    for(var i=0;i<9;i+=1) {
        let t=p+vec2<i32>(i%3,i/3)-vec2<i32>(1);
        tr1_ring[i]=s.id;
        if all(t>=vec2<i32>(0)) && all(t<vec2<i32>(size)) {
            tr1_ring[i]=tr1_id(t);
            if tr1_ring[i]!=s.id {silhouette=true;}
        }
    }
    let anchor=vec2<u32>((s.id&TR1_ID)|((s.side&1u)<<23u)|select(0u,TR1_MIXED,silhouette),bitcast<u32>(s.distance));
    textureStore(tr1_anchor_out,p,vec4<u32>(anchor,0u,0u));
    // Open background: every sample of the footprint's neighbourhood misses, so the film is
    // the background exactly and its history needs no fetch.
    if s.id==0u && !silhouette {
        let background=quantizeToF16(tr1.background.xyz);
        textureStore(tr1_colour_out,p,vec4<f32>(background,tr1.windows.x));
        textureStore(tr1_output,p,vec4<f32>(background,1.0));
        return;
    }
    var sample=textureLoad(tr1_sample,p,0).xyz;
    var h:Tr1History;
    if tr1.flags.x==0u {h=tr1_history(p,s,silhouette,size);}
    var mean=sample;
    var n=1.0;
    if h.weight>0.0 {
        // The window: the static one at whole-pixel offsets, the resampling blur budget
        // under bilinear motion (T1b's W = 1 + v_max / sum f(1-f)), the Catmull-Rom motion
        // window, engine-event caps and the silhouette cap.
        var window=tr1.windows.x;
        if h.blur>0.0 {
            window=min(window,select(1.0+tr1.windows.w/h.blur,tr1.starts.x,h.sharp));
        }
        window=min(window,tr1.windows.y);
        // Moving silhouettes cap the window: their coverage drifts by the parallax.
        if silhouette && !h.still {window=min(window,tr1.windows.z);}
        n=clamp(h.weight+1.0,1.0,max(window,1.0));
        if silhouette && !h.still && (tr1.flags.y&TR1_SILHOUETTE_SPATIAL)!=0u {
            sample=tr1_spatial(p,s.id,true,size).xyz;
        }
        var history=h.colour;
        if (tr1.flags.y&TR1_BOX)!=0u {
            // The plain min/max box of every 3x3 sample of this frame: no statistic, and no
            // sample of the neighbourhood lies outside it.
            var lo=sample;
            var hi=sample;
            for(var i=0;i<9;i+=1) {
                let t=clamp(p+vec2<i32>(i%3,i/3)-vec2<i32>(1),vec2<i32>(0),vec2<i32>(size)-vec2<i32>(1));
                let c=textureLoad(tr1_sample,t,0).xyz;
                lo=min(lo,c);hi=max(hi,c);
            }
            history=clamp(history,lo,hi);
        }
        // m' = m + (x - m) / n', with the multiply-add pinned so replays are bit-identical.
        mean=fma(sample-history,vec3<f32>(1.0/n),history);
    } else if (tr1.flags.y&TR1_SPATIAL_START)!=0u {
        let start=tr1_spatial(p,s.id,silhouette,size);
        mean=start.xyz;
        n=clamp(start.w,1.0,max(tr1.starts.y,1.0));
    }
    textureStore(tr1_colour_out,p,vec4<f32>(mean,n));
    textureStore(tr1_output,p,vec4<f32>(mean,1.0));
}
// ---- end of TR1 ----
