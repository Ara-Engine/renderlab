// ---- TR1 temporal reconstruction (docs/m7-tr1.md, docs/m7-tr1b.md) ----
// Appended to the non-RT engine head (primary_ray_gpu::non_rt_heads), so it compiles on
// devices without ray queries. One pass after the frame's shading: each pixel's history is
// fetched where its current sample's surface point was in the previous frame, and accepted
// only by surface identity (placement, viewed side, tangent plane) and engine state. No
// colour, luminance or neighbourhood statistic decides acceptance or weight: every program,
// however it shades, converges to the footprint integral of its own output.
//
// Every colour history is one surface's (TR1b): a silhouette pixel keeps two slots, its
// sample's surface and the other surface of its 3x3, and the occluder's coverage of its
// footprint, which moves with the occluder. Its film is coverage-weighted.
//
// Patent notes (docs/m7-p0-claims.md §4.3, docs/m7-tr1b.md): no neighbourhood colour box or
// variance (US 10,116,916 / 10,362,289); no fast history or retained list of previous values
// (a second slot is another surface's single history, not a second list of this surface's);
// no lighting difference or own-pixel moments (US 12,437,371, pending 2026/0162231). This
// pass's weights, coverages, anchors and slots are read by this pass alone, next frame (W5).

// Frame state written by the CPU (temporal_reconstruction.rs). Storage, not uniform: it
// feeds addressing (docs/m6-interactive-climb-2.md).
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
    // x: static window; y: engine-event window cap; z: coverage window under motion; w: blur
    // budget.
    windows:vec4<f32>,
    // x: Catmull-Rom motion window; y: spatial-start window cap; z: spatial-start tent radius
    // (0: the pixel's own sample).
    starts:vec4<f32>,
    // x: nonzero restarts every pixel (cut, first frame, reset events); y: TR1_* options;
    // z: nonzero when nothing moved (bit-equal unjittered view-projections; moved placements
    // restart), decided on the CPU: the GPU need not round two projections alike.
    flags:vec4<u32>,
};
const TR1_CATMULL_ROM:u32=1u;
const TR1_SPATIAL_START:u32=2u;
const TR1_REVERSE_SEARCH:u32=4u;
const TR1_COVERAGE_SHARE:u32=8u;
@group(0) @binding(40) var<storage,read> tr1:Tr1Frame;
// This frame's shaded samples (a copy of the HDR target before this pass).
@group(0) @binding(41) var tr1_sample:texture_2d<f32>;
// The previous frame's first slot: exposed mean and sample weight, and its anchor.
@group(0) @binding(42) var tr1_colour_in:texture_2d<f32>;
@group(0) @binding(43) var tr1_anchor_in:texture_2d<u32>;
@group(0) @binding(44) var tr1_colour_out:texture_storage_2d<rgba32float,write>;
// Anchor of the first slot. x: placement + 1 (zero is background) in bits 0..22, the
// viewed side in bit 23; y: the latest sample's view distance, f32 bits.
@group(0) @binding(45) var tr1_anchor_out:texture_storage_2d<rg32uint,write>;
// The reconstructed image (the frame's HDR target).
@group(0) @binding(46) var tr1_output:texture_storage_2d<rgba16float,write>;
// The second slot (silhouettes; TR1_NONE elsewhere). x: placement + 1 and side as the
// anchor's, bit 24 when the first slot's surface is the occluder; y, z: f16 exposed mean and
// weight; w: f16 coverage of the first slot's surface and its weight.
@group(0) @binding(47) var tr1_layer_in:texture_2d<u32>;
@group(0) @binding(48) var tr1_layer_out:texture_storage_2d<rgba32uint,write>;
struct Tr1Vec4s { data:array<vec4<f32>> };
struct Tr1U32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> tr1_vertices:binding_array<Tr1Vec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> tr1_indices:binding_array<Tr1U32s,PRIMARY_BATCHES>;

const TR1_ID:u32=0x7fffffu;
const TR1_NONE:u32=0xffffffffu;
const TR1_FIRST_OCCLUDES:u32=0x1000000u;

fn tr1_position(batch:u32,index:u32)->vec3<f32> {
    return tr1_vertices[batch].data[tr1_indices[batch].data[index]*3u].xyz;
}
fn tr1_dot(a:vec3<f32>,b:vec3<f32>)->f32 {return fma(a.x,b.x,fma(a.y,b.y,a.z*b.z));}
fn tr1_inside(t:vec2<i32>,size:vec2<u32>)->bool {
    return all(t>=vec2<i32>(0)) && all(t<vec2<i32>(size));
}

// The hit record's placement + 1 at a pixel, or zero: uncovered trace tiles keep stale
// records, and every pixel there is background.
fn tr1_id(p:vec2<i32>)->u32 {
    if textureLoad(primary_tiles,vec2<u32>(p)/PRIMARY_TILE,0).x==0u {return 0u;}
    return textureLoad(primary_hit,p,0).x;
}

// A sample's surface: world point (w = 1) or background direction (w = 0), its identity,
// view distance and unit geometric normal.
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
// Where a surface point was in the previous frame, as the continuous position of this
// pixel's footprint there (pixel centres at + 0.5, minus 0.5 for bilinear taps).
fn tr1_fetch_position(p:vec2<i32>,s:Tr1Surface,size:vec2<f32>)->vec2<f32> {
    if tr1.flags.z!=0u {return vec2<f32>(p);}
    return vec2<f32>(p)+tr1_pixel(tr1.previous,s.point,size)-tr1_pixel(tr1.current,s.point,size);
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

// The second slot of a previous-frame tap.
struct Tr1Second { id:u32, side:u32, colour:vec3<f32>, weight:f32, coverage:f32, coverage_weight:f32, first_occludes:bool };
fn tr1_second(t:vec2<i32>)->Tr1Second {
    let l=textureLoad(tr1_layer_in,t,0);
    var s:Tr1Second;
    s.id=TR1_NONE;
    if l.x==TR1_NONE {return s;}
    let rg=unpack2x16float(l.y);let bw=unpack2x16float(l.z);let cw=unpack2x16float(l.w);
    return Tr1Second(l.x&TR1_ID,(l.x>>23u)&1u,vec3<f32>(rg,bw.x),bw.y,cw.x,cw.y,(l.x&TR1_FIRST_OCCLUDES)!=0u);
}

// The history surface `s` left at previous-frame tap `t`, if any: (mean, weight), weight
// zero when none. The first slot shows the surface when its placement and viewed side match
// and, moving, its sample point (rebuilt from its own ray) lies within four pixel footprints
// of the current point's tangent plane: a fold or ridge of the same mesh in front of or
// behind the surface fails at any viewing angle. The second slot (a silhouette's other
// surface) shows it by placement and side. Still frames read the pixel's own slots.
fn tr1_tap(t:vec2<i32>,s:Tr1Surface,expected:f32,size:vec2<u32>)->vec4<f32> {
    if !tr1_inside(t,size) {return vec4<f32>(0.0);}
    let thin=s.id!=0u && (primary_placements[s.id-1u]&0x20000000u)!=0u;
    let a=textureLoad(tr1_anchor_in,t,0).xy;
    if (a.x&TR1_ID)==s.id && !(thin && ((a.x>>23u)&1u)!=s.side) {
        var ok=s.id==0u || tr1.flags.z!=0u;
        if !ok {
            let point=tr1_anchor_point(t,bitcast<f32>(a.y),vec2<f32>(size));
            ok=abs(tr1_dot(s.normal,point-s.point.xyz))<=4.0*tr1.previous_eye.w*expected;
        }
        if ok {return textureLoad(tr1_colour_in,t,0);}
    }
    let second=tr1_second(t);
    if second.id==s.id && !(thin && second.side!=s.side) && second.weight>0.0 {
        return vec4<f32>(second.colour,second.weight);
    }
    return vec4<f32>(0.0);
}

// Catmull-Rom weights of taps -1, 0, 1, 2 at fractional offset t.
fn tr1_catmull_rom(t:f32)->vec4<f32> {
    let t2=t*t;
    return vec4<f32>(t*fma(t,2.0-t,-1.0),fma(t2,fma(3.0,t,-5.0),2.0),
        t*fma(t,fma(-3.0,t,4.0),1.0),t2*(t-1.0))*0.5;
}

// Surface `s`'s continued history at this pixel: exposed mean (rgb) and weight (w), zero
// weight when no tap shows it. Bilinear over the taps that show it, renormalised, with the
// bilinear weighted sum of their weights; Catmull-Rom over 4x4 when every tap shows it,
// limited to the inner taps' range (history only: the undershoot fix, not a current-frame
// box). `window` is the window the fetch allows.
struct Tr1History { colour:vec3<f32>, weight:f32, window:f32 };
fn tr1_history(p:vec2<i32>,s:Tr1Surface,size:vec2<u32>)->Tr1History {
    let q=tr1_fetch_position(p,s,vec2<f32>(size));
    let base=vec2<i32>(floor(q));
    let f=q-floor(q);
    var expected=0.0;
    if s.id!=0u {
        let v=s.point.xyz-tr1.previous_eye.xyz;
        expected=sqrt(tr1_dot(v,v));
    }
    let scale=tr1.jitter.z;
    var h:Tr1History;
    var total=0.0;
    var colour=vec3<f32>(0.0);
    var lo=vec3<f32>(3.4e38);
    var hi=vec3<f32>(-3.4e38);
    var shown=0u;
    for(var i=0;i<4;i+=1) {
        let o=vec2<i32>(i&1,i>>1u);
        let x=tr1_tap(base+o,s,expected,size);
        if x.w>0.0 {
            let w=select(1.0-f.x,f.x,o.x!=0)*select(1.0-f.y,f.y,o.y!=0);
            let c=x.xyz*scale;
            total+=w;
            colour=fma(c,vec3<f32>(w),colour);
            h.weight=fma(x.w,w,h.weight);
            lo=min(lo,c);hi=max(hi,c);
            shown+=1u;
        }
    }
    if (shown==0u || total<=0.0) && (tr1.flags.y&TR1_REVERSE_SEARCH)!=0u && tr1.flags.z==0u {
        // The +-1 px search: no bilinear tap shows the surface, but a tap of the 4x4 ring
        // around them may (a revealed pixel beside where it was hidden). Tent-weighted by
        // distance from the fetch point, at half their weight, with the bilinear window.
        var near=vec3<f32>(0.0);
        var nw=0.0;
        var nn=0.0;
        for(var i=0;i<16;i+=1) {
            let o=vec2<i32>(i&3,i>>2u)-vec2<i32>(1);
            if all(o>=vec2<i32>(0)) && all(o<=vec2<i32>(1)) {continue;}
            let x=tr1_tap(base+o,s,expected,size);
            if x.w>0.0 {
                let d=abs(vec2<f32>(o)-f);
                let w=max(1.0-0.5*d.x,0.0)*max(1.0-0.5*d.y,0.0);
                near=fma(x.xyz*scale,vec3<f32>(w),near);nw+=w;nn=fma(x.w,w,nn);
            }
        }
        if nw>0.0 {
            h.colour=near/nw;
            h.weight=0.5*nn/nw;
            h.window=min(tr1.windows.x,1.0+tr1.windows.w/max(fma(f.x,1.0-f.x,f.y*(1.0-f.y)),0.25));
            h.window=min(h.window,tr1.windows.y);
        }
        return h;
    }
    if shown==0u || total<=0.0 {return h;}
    h.colour=colour/total;
    // The window: the static one at whole-pixel offsets, the resampling blur budget under
    // bilinear motion (T1b's W = 1 + v_max / sum f(1-f)), the Catmull-Rom motion window, and
    // engine-event caps.
    let blur=fma(f.x,1.0-f.x,f.y*(1.0-f.y));
    h.window=tr1.windows.x;
    if blur>0.0 {h.window=min(h.window,1.0+tr1.windows.w/blur);}
    if shown==4u && (tr1.flags.y&TR1_CATMULL_ROM)!=0u && blur>0.0 {
        let wx=tr1_catmull_rom(f.x);let wy=tr1_catmull_rom(f.y);
        var sharp=vec3<f32>(0.0);
        var outer=true;
        for(var i=0;i<16 && outer;i+=1) {
            let o=vec2<i32>(i&3,i>>2u)-vec2<i32>(1);
            let x=tr1_tap(base+o,s,expected,size);
            outer=x.w>0.0;
            sharp=fma(x.xyz*scale,vec3<f32>(wx[i&3]*wy[i>>2u]),sharp);
        }
        if outer {
            h.colour=clamp(sharp,lo,hi);
            h.window=min(tr1.windows.x,tr1.starts.x);
        }
    }
    h.window=min(h.window,tr1.windows.y);
    return h;
}

// The occluder's coverage of this pixel's footprint in the previous frame, fetched where the
// occluder's point was (the edge moves with the occluder): (coverage, weight), weight zero
// when no tap holds the pair. A tap of one surface alone covers 1 or 0; a silhouette tap
// reports its stored coverage. Also the window its resampling allows.
fn tr1_coverage(p:vec2<i32>,o:Tr1Surface,u:u32,size:vec2<u32>)->vec3<f32> {
    let q=tr1_fetch_position(p,o,vec2<f32>(size));
    let base=vec2<i32>(floor(q));
    let f=q-floor(q);
    var total=0.0;var coverage=0.0;var weight=0.0;
    for(var i=0;i<4;i+=1) {
        let d=vec2<i32>(i&1,i>>1u);
        let t=base+d;
        if !tr1_inside(t,size) {continue;}
        let first=textureLoad(tr1_anchor_in,t,0).x&TR1_ID;
        let second=tr1_second(t);
        var c=-1.0;var n=0.0;
        if first==o.id {
            if second.id==TR1_NONE {c=1.0;n=textureLoad(tr1_colour_in,t,0).w;} else {c=second.coverage;n=second.coverage_weight;}
        } else if first==u {
            if second.id==TR1_NONE {c=0.0;n=textureLoad(tr1_colour_in,t,0).w;} else if second.id==o.id {c=1.0-second.coverage;n=second.coverage_weight;} else {c=0.0;n=second.coverage_weight;}
        } else if second.id==o.id {
            c=1.0-second.coverage;n=second.coverage_weight;
        }
        if c>=0.0 && n>0.0 {
            let w=select(1.0-f.x,f.x,d.x!=0)*select(1.0-f.y,f.y,d.y!=0);
            total+=w;coverage=fma(c,w,coverage);weight=fma(n,w,weight);
        }
    }
    if total<=0.0 {return vec3<f32>(0.0);}
    let blur=fma(f.x,1.0-f.x,f.y*(1.0-f.y));
    var window=tr1.windows.x;
    if blur>0.0 {window=min(min(window,1.0+tr1.windows.w/blur),tr1.windows.z);}
    return vec3<f32>(coverage/total,weight,min(window,tr1.windows.y));
}

// A start from this frame's samples only: the tent-weighted mean of the 3x3 samples showing
// `id`, each weighted at its jittered position, and that estimate's effective sample count.
fn tr1_spatial(p:vec2<i32>,id:u32,size:vec2<u32>)->vec4<f32> {
    let radius=tr1.starts.z;
    if radius<=0.0 && tr1_id(p)==id {return vec4<f32>(textureLoad(tr1_sample,p,0).xyz,1.0);}
    var sum=vec3<f32>(0.0);
    var w1=0.0;
    var w2=0.0;
    for(var i=0;i<9;i+=1) {
        let o=vec2<i32>(i%3,i/3)-vec2<i32>(1);
        let t=p+o;
        if !tr1_inside(t,size) || tr1_ring[i]!=id {continue;}
        // At radius 2 and zero jitter, the 3x3 [1,2,1] filter.
        let d=abs(vec2<f32>(o)+tr1.jitter.xy)/max(radius,2.0);
        let w=max(1.0-d.x,0.0)*max(1.0-d.y,0.0);
        sum=fma(textureLoad(tr1_sample,t,0).xyz,vec3<f32>(w),sum);
        w1+=w;w2=fma(w,w,w2);
    }
    if w1<=0.0 {return vec4<f32>(0.0);}
    return vec4<f32>(sum/w1,w1*w1/w2);
}
// The tent-weighted share of this frame's 3x3 samples showing `id` at the pixel centre: a
// one-frame coverage estimate.
fn tr1_share(id:u32)->f32 {
    var hit=0.0;var all=0.0;
    for(var i=0;i<9;i+=1) {
        let d=abs(vec2<f32>(vec2<i32>(i%3,i/3)-vec2<i32>(1))+tr1.jitter.xy);
        let w=max(1.0-d.x,0.0)*max(1.0-d.y,0.0);
        all+=w;
        if tr1_ring[i]==id {hit+=w;}
    }
    return hit/max(all,1e-6);
}

// One surface's history continued with this frame's sample (the pixel's own surface), or
// started from this frame's samples when nothing continues it.
fn tr1_continue(p:vec2<i32>,s:Tr1Surface,sample:vec3<f32>,size:vec2<u32>)->vec4<f32> {
    if s.id==0u {return vec4<f32>(quantizeToF16(tr1.background.xyz),tr1.windows.x);}
    var h:Tr1History;
    if tr1.flags.x==0u {h=tr1_history(p,s,size);}
    if h.weight>0.0 {
        let n=clamp(h.weight+1.0,1.0,max(h.window,1.0));
        // m' = m + (x - m) / n', with the multiply-add pinned so replays are bit-identical.
        return vec4<f32>(fma(sample-h.colour,vec3<f32>(1.0/n),h.colour),n);
    }
    if (tr1.flags.y&TR1_SPATIAL_START)!=0u {
        let start=tr1_spatial(p,s.id,size);
        return vec4<f32>(start.xyz,clamp(start.w,1.0,max(tr1.starts.y,1.0)));
    }
    return vec4<f32>(sample,1.0);
}

fn tr1_store_single(p:vec2<i32>,s:Tr1Surface,film:vec4<f32>) {
    textureStore(tr1_anchor_out,p,vec4<u32>((s.id&TR1_ID)|((s.side&1u)<<23u),bitcast<u32>(s.distance),0u,0u));
    textureStore(tr1_colour_out,p,film);
    textureStore(tr1_layer_out,p,vec4<u32>(TR1_NONE,0u,0u,0u));
    textureStore(tr1_output,p,vec4<f32>(film.xyz,1.0));
}

@compute @workgroup_size(8,8) fn tr1_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(tr1_output);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    let s=tr1_surface(p,size);
    // A silhouette pixel: some 3x3 sample of this frame shows another surface. The other
    // surface is the nearest such sample's (edge neighbours before corners).
    var other=-1;
    for(var i=0;i<9;i+=1) {
        let t=p+vec2<i32>(i%3,i/3)-vec2<i32>(1);
        tr1_ring[i]=s.id;
        if tr1_inside(t,size) {
            tr1_ring[i]=tr1_id(t);
            if tr1_ring[i]!=s.id && (other<0 || (i%2==1 && other%2==0)) {other=i;}
        }
    }
    let sample=textureLoad(tr1_sample,p,0).xyz;
    // On a still view a silhouette stays one while its own pixel holds the other surface:
    // whether this frame's 3x3 shows it depends on the jitter, not on the footprint.
    var kept:Tr1Second;
    kept.id=TR1_NONE;
    if other<0 && tr1.flags.z!=0u && tr1.flags.x==0u {
        let own=tr1_second(p);
        let first=textureLoad(tr1_anchor_in,p,0).x&TR1_ID;
        // Kept only while this sample shows one of the two stored surfaces.
        if own.id!=TR1_NONE && (first==s.id || own.id==s.id) {
            kept=own;
            if own.id==s.id {
                // Last frame's sample showed the other surface: the first slot is the other.
                kept.id=first;
                kept.side=(textureLoad(tr1_anchor_in,p,0).x>>23u)&1u;
            }
        }
    }
    if other<0 && kept.id==TR1_NONE {
        // One surface: open background is the background exactly; an interior continues
        // its surface's history from either slot of the previous frame.
        tr1_store_single(p,s,tr1_continue(p,s,sample,size));
        return;
    }
    // A silhouette: the other surface's sample point gives its motion and depth. A kept
    // still silhouette reads its own pixel (no motion) and its stored occluder.
    var u:Tr1Surface;
    var occluder_is_s:bool;
    if other>=0 {
        let t=p+vec2<i32>(other%3,other/3)-vec2<i32>(1);
        u=tr1_surface(t,size);
        // The occluder: never the background; between two hits, the nearer.
        occluder_is_s=u.id==0u || (s.id!=0u && s.distance<=u.distance);
    } else {
        u=Tr1Surface(s.point,kept.id,kept.side,s.distance,s.normal);
        let first=textureLoad(tr1_anchor_in,p,0).x&TR1_ID;
        occluder_is_s=select(!kept.first_occludes,kept.first_occludes,first==s.id);
    }
    var o=u;
    if occluder_is_s {o=s;}
    let mine=tr1_continue(p,s,sample,size);
    // The other surface keeps its history (no sample of it this frame), or starts from this
    // frame's samples of it.
    var theirs=vec4<f32>(quantizeToF16(tr1.background.xyz),tr1.windows.x);
    if u.id!=0u {
        var h:Tr1History;
        if tr1.flags.x==0u {h=tr1_history(p,u,size);}
        if h.weight>0.0 {
            theirs=vec4<f32>(h.colour,min(h.weight,max(h.window,1.0)));
        } else {
            let start=tr1_spatial(p,u.id,size);
            theirs=vec4<f32>(start.xyz,clamp(start.w,1.0,max(tr1.starts.y,1.0)));
        }
    }
    // The occluder's coverage: continued where the occluder was, with this frame's sample
    // as the new observation (1 when it shows the occluder); a new silhouette starts from the
    // tent-weighted share of this frame's 3x3 samples.
    var c=vec3<f32>(0.0);
    if tr1.flags.x==0u {c=tr1_coverage(p,o,select(s.id,u.id,occluder_is_s),size);}
    var coverage=tr1_share(o.id);
    var coverage_weight=1.0;
    if c.y>0.0 {
        coverage_weight=clamp(c.y+1.0,1.0,max(c.z,1.0));
        var observed=select(0.0,1.0,occluder_is_s);
        if (tr1.flags.y&TR1_COVERAGE_SHARE)!=0u {observed=tr1_share(o.id);}
        coverage=fma(observed-c.x,1.0/coverage_weight,c.x);
    }
    var front=theirs.xyz;var back=mine.xyz;
    if occluder_is_s {front=mine.xyz;back=theirs.xyz;}
    let film=mix(back,front,coverage);
    textureStore(tr1_anchor_out,p,vec4<u32>((s.id&TR1_ID)|((s.side&1u)<<23u),bitcast<u32>(s.distance),0u,0u));
    textureStore(tr1_colour_out,p,mine);
    let first_coverage=select(1.0-coverage,coverage,occluder_is_s);
    textureStore(tr1_layer_out,p,vec4<u32>((u.id&TR1_ID)|((u.side&1u)<<23u)|select(0u,TR1_FIRST_OCCLUDES,occluder_is_s),pack2x16float(theirs.xy),
        pack2x16float(vec2<f32>(theirs.z,theirs.w)),pack2x16float(vec2<f32>(first_coverage,coverage_weight))));
    textureStore(tr1_output,p,vec4<f32>(film,1.0));
}
// ---- end of TR1 ----
