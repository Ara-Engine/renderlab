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
    // xyz: the exposed background (primary-miss) colour; stored samples are its f16 value.
    // w: 1 when every miss shows it; 0 when misses show the visible sky
    // (Renderer::set_visible_environment) and reconstruct like a surface keyed by direction.
    background:vec4<f32>,
    // x: static window; y: engine-event window cap; z: coverage window under motion; w: blur
    // budget.
    windows:vec4<f32>,
    // x: Catmull-Rom motion window; y: spatial-start window cap; z: spatial-start tent radius
    // (0: the pixel's own sample); w: MQ2's view budget (0: off).
    starts:vec4<f32>,
    // x: nonzero restarts every pixel (cut, first frame, reset events); y: TR1_* options;
    // z: nonzero when the camera did not move (bit-equal unjittered view-projections),
    // decided on the CPU: the GPU need not round two projections alike. Surfaces whose vertices
    // or placement moved are not still (tr1_still).
    flags:vec4<u32>,
    // Where static receivers' histories restart this frame (Tr1Zones, primary_rays.wgsl):
    // a moving caster's swept shadow (SK1) and changed local lights' restart spheres (P-RELIT).
    zones:Tr1Zones,
    // x: Reconstruction::sharp_blur_budget.
    resample:vec4<f32>,
};
const TR1_CATMULL_ROM:u32=1u;
const TR1_SPATIAL_START:u32=2u;
// MQ1 (docs/m7-mq1.md): some placement's transform changed since the previous frame, and
// continued histories are clamped to the plain min/max box of this frame's 3x3 samples (all
// nine samples, no mean or variance: P0's box, docs/m7-p0-claims.md).
const TR1_BOX:u32=8u;
// TR2 (docs/m7-tr2.md): upscaled young history interpolates the internal grid
// (Reconstruction::upscale_interpolation).
const TR1_UPSCALE_INTERPOLATION:u32=16u;
@group(0) @binding(40) var<storage,read> tr1:Tr1Frame;
// This frame's shaded samples (a copy of the HDR target before this pass).
@group(0) @binding(41) var tr1_sample:texture_2d<f32>;
// The previous frame's first slot: exposed mean and sample weight, and its anchor.
@group(0) @binding(42) var tr1_colour_in:texture_2d<f32>;
@group(0) @binding(43) var tr1_anchor_in:texture_2d<u32>;
@group(0) @binding(44) var tr1_colour_out:texture_storage_2d<rgba32float,write>;
// Anchor of the first slot. x: placement + 1 (zero is background) in bits 0..22, the
// viewed side in bit 23, MQ2's view hint as tr1_view_code in bits 24..31 (for the history's
// readers; TR1 itself reads it from the sample); y: the latest sample's view distance, f32 bits.
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
// SK1: previous submitted vertices. Static draws alias the current stream; a skinned
// draw binds its own previous stream, and the batch word says which (batch_skinned).
var<private> tr1_near_skin:bool;
fn tr1_skinned(batch:u32)->bool {
    return batch_skinned(primary_batch_info(batch));
}
// MV1: only the motion source changes; moving rigid silhouettes follow SK1's path.
fn tr1_moving(slot:u32)->bool {
    return motion_rigid(slot)||tr1_skinned(primary_placements[slot]&0x1fffffffu);
}

const TR1_ID:u32=0x7fffffu;
const TR1_NONE:u32=0xffffffffu;
const TR1_FIRST_OCCLUDES:u32=0x1000000u;

fn tr1_position(batch:u32,index:u32)->vec3<f32> {
    return tr1_vertices[batch].data[tr1_indices[batch].data[index]*3u].xyz;
}
fn tr1_previous_position(batch:u32,index:u32)->vec3<f32> {
    return motion_previous_vertex(batch,tr1_indices[batch].data[index]);
}
fn tr1_dot(a:vec3<f32>,b:vec3<f32>)->f32 {return fma(a.x,b.x,fma(a.y,b.y,a.z*b.z));}
fn tr1_inside(t:vec2<i32>,size:vec2<u32>)->bool {
    return all(t>=vec2<i32>(0)) && all(t<vec2<i32>(size));
}

// Upscaling (RenderScale): this frame's samples (tr1_sample, primary_hit, primary_tiles) may
// lie on a coarser grid than the output and the histories (tr1_output). An output pixel takes
// the sample nearest its centre. tr1_ratio is the sample grid's size over the output's, per
// axis; at 1 (native) every mapping below is the identity and every weight 1.
var<private> tr1_ratio:vec2<f32>;
// Whether the sample grid is the output grid, decided from the two integer sizes and never
// from tr1_ratio: a float quotient of equal sizes need not be exactly 1 (Vulkan allows
// OpFDiv 2.5 ULP), and RADV's native frames then took the upscaling path, with tent weights
// just under 1 and the nearest-sample clamp at the image border.
var<private> tr1_same_grid:bool;
// The output pixel this invocation resolves.
var<private> tr1_here:vec2<i32>;
fn tr1_native()->bool {return tr1_same_grid;}
// The sample grid pixel whose sample (offset `jitter`) is nearest output pixel p's centre.
fn tr1_nearest(p:vec2<i32>,jitter:vec2<f32>)->vec2<i32> {
    let c=fma(vec2<f32>(p)+vec2<f32>(0.5),tr1_ratio,-jitter);
    return clamp(vec2<i32>(floor(c)),vec2<i32>(0),vec2<i32>(textureDimensions(tr1_sample))-vec2<i32>(1));
}
fn tr1_map(p:vec2<i32>)->vec2<i32> {
    if tr1_native() {return p;}
    return tr1_nearest(p,tr1.jitter.xy);
}
// Output pixel p's sample offset from its centre in output pixels for a frame's jitter.
fn tr1_sample_offset(p:vec2<i32>,jitter:vec2<f32>)->vec2<f32> {
    if tr1_native() {return jitter;}
    return (vec2<f32>(tr1_nearest(p,jitter))+vec2<f32>(0.5)+jitter)/tr1_ratio-vec2<f32>(p)-vec2<f32>(0.5);
}
fn tr1_offset(p:vec2<i32>)->vec2<f32> {return tr1_sample_offset(p,tr1.jitter.xy);}
// This frame's sample weight at output pixel p: 1 at native resolution; upscaled, a tent of
// one output pixel around p's centre (zero: p keeps its history this frame).
fn tr1_weight(p:vec2<i32>)->f32 {
    if tr1_native() {return 1.0;}
    let o=abs(tr1_offset(p));
    return max(1.0-o.x,0.0)*max(1.0-o.y,0.0);
}

// The hit record's placement + 1 at a pixel's sample, or zero: uncovered trace tiles keep
// stale records, and every pixel there is background.
fn tr1_sample_id(q:vec2<i32>)->u32 {
    if textureLoad(primary_tiles,vec2<u32>(q)/PRIMARY_TILE,0).x==0u {return 0u;}
    return textureLoad(primary_hit,q,0).x;
}
fn tr1_id(p:vec2<i32>)->u32 {return tr1_sample_id(tr1_map(p));}

// TR2 (upscaled frames): this frame's samples of surface `id` around output pixel p, each
// weighted by a separable tent of radius `r` output pixels at its jittered position:
// (weighted mean, total weight). At r = 1 it is TR1b's nearest-sample tent, widened to every
// sample inside it; at r = the internal sample spacing it interpolates the internal grid
// (weights sum to 1 where every sample shows the surface). The 3x3 internal samples around
// the nearest one cover any r up to the spacing.
fn tr1_gather(p:vec2<i32>,id:u32,r:vec2<f32>)->vec4<f32> {
    let dims=vec2<i32>(textureDimensions(tr1_sample));
    let base=vec2<i32>(floor(fma(vec2<f32>(p)+vec2<f32>(0.5),tr1_ratio,-tr1.jitter.xy)));
    var sum=vec3<f32>(0.0);
    var total=0.0;
    for(var i=0;i<9;i+=1) {
        let q=base+vec2<i32>(i%3,i/3)-vec2<i32>(1);
        if any(q<vec2<i32>(0)) || any(q>=dims) {continue;}
        let d=abs((vec2<f32>(q)+vec2<f32>(0.5)+tr1.jitter.xy)/tr1_ratio-vec2<f32>(p)-vec2<f32>(0.5))/r;
        let w=max(1.0-d.x,0.0)*max(1.0-d.y,0.0);
        if w<=0.0 || tr1_sample_id(q)!=id {continue;}
        sum=fma(textureLoad(tr1_sample,q,0).xyz,vec3<f32>(w),sum);
        total+=w;
    }
    if total<=0.0 {return vec4<f32>(0.0);}
    return vec4<f32>(sum/total,total);
}

// A sample's surface: world point (w = 1) or background direction (w = 0), its identity,
// view distance and unit geometric normal; the same point and plane in the previous frame
// (skinned vertices' previous positions, SK1; a moved placement's previous transform, MQ1),
// whether either moved it, and its program's view dependence (MQ2: the sample's alpha
// minus 1, shaders/temporal_view.wgsl; 0 when view-independent).
struct Tr1Surface { point:vec4<f32>, id:u32, side:u32, distance:f32, normal:vec3<f32>, previous_point:vec4<f32>, previous_normal:vec3<f32>, moved:bool, view:f32 };
// MV1: MQ1/MQ2 surface classification reads the shared submitted snapshot.
fn tr1_moved(id:u32)->bool {return id!=0u && id!=TR1_NONE && motion_rigid((id-1u)&TR1_ID);}
// MQ2: the angle (radians) the reflected view direction turned at a surface since the
// previous frame: the view direction's turn from the eye, plus twice its plane's turn
// (moved placements and skinned vertices). Zero for a still camera and a still surface.
fn tr1_view_turn(s:Tr1Surface)->f32 {
    let a=tr1.previous_eye.xyz-s.previous_point.xyz;
    let b=tr1.current_eye.xyz-s.point.xyz;
    let turn=a*inverseSqrt(max(tr1_dot(a,a),1e-30))-b*inverseSqrt(max(tr1_dot(b,b),1e-30));
    let plane=s.normal-s.previous_normal;
    return sqrt(tr1_dot(turn,turn))+2.0*sqrt(tr1_dot(plane,plane));
}
// The surface neither the camera nor its vertices or placement moved: its history is its
// own pixel's.
fn tr1_still(s:Tr1Surface)->bool {return tr1.flags.z!=0u && !s.moved;}
// MQ1's box over this frame's 3x3 samples (TR1_BOX frames only).
var<private> tr1_box_lo:vec3<f32>;
var<private> tr1_box_hi:vec3<f32>;
// Placement + 1 of this frame's samples over the workgroup's 8x8 pixels and a one-pixel
// border (TR1_NONE outside the image), loaded once per workgroup by tr1_resolve.
var<workgroup> tr1_tile:array<u32,100>;
// This invocation's tile coordinates and its own sample's placement + 1.
var<private> tr1_local:vec2<i32>;
var<private> tr1_self:u32;
// Placement + 1 of 3x3 sample i (row-major from the top left); outside the image, the
// pixel's own.
fn tr1_ring(i:i32)->u32 {
    let v=tr1_tile[(tr1_local.y+i/3)*10+tr1_local.x+i%3];
    return select(v,tr1_self,v==TR1_NONE);
}
fn tr1_surface(p:vec2<i32>,size:vec2<u32>)->Tr1Surface {
    let id=tr1_id(p);
    let q=tr1_map(p);
    if id==0u {
        // The sample's own camera ray: primary_ray inverts this frame's jittered projection.
        let ray=primary_ray(vec2<u32>(q),textureDimensions(tr1_sample));
        let d=ray.far-ray.near;
        let point=vec4<f32>(d*inverseSqrt(tr1_dot(d,d)),0.0);
        return Tr1Surface(point,0u,0u,0.0,vec3<f32>(0.0),point,vec3<f32>(0.0),false,0.0);
    }
    let record=textureLoad(primary_hit,q,0);
    let slot=id-1u;
    let batch=primary_placements[slot]&0x1fffffffu;
    let i=record.y*3u;
    let a=tr1_position(batch,i);let b=tr1_position(batch,i+1u);let c=tr1_position(batch,i+2u);
    let uv=bitcast<vec2<f32>>(record.zw);
    var pa=a;var pb=b;var pc=c;
    if tr1_skinned(batch) {pa=tr1_previous_position(batch,i);pb=tr1_previous_position(batch,i+1u);pc=tr1_previous_position(batch,i+2u);}
    // Preserve MQ1/SK1's exact static arithmetic; both moving consumers read the
    // same previous transform in the shared placement table (object_motion.wgsl).
    var px=world_position(receiver_point(pa,pb,pc,vec3<f32>(1.0-uv.x-uv.y,uv)),slot);
    var pn=cross(world_position(pb,slot).xyz-world_position(pa,slot).xyz,world_position(pc,slot).xyz-world_position(pa,slot).xyz);
    let placed=motion_rigid(slot);
    if placed {
        px=motion_previous_world(receiver_point(pa,pb,pc,vec3<f32>(1.0-uv.x-uv.y,uv)),slot);
        let qa=motion_previous_world(pa,slot).xyz;
        pn=cross(motion_previous_world(pb,slot).xyz-qa,motion_previous_world(pc,slot).xyz-qa);
    }
    let x=world_position(receiver_point(a,b,c,vec3<f32>(1.0-uv.x-uv.y,uv)),slot).xyz;
    let wa=world_position(a,slot).xyz;
    let n=cross(world_position(b,slot).xyz-wa,world_position(c,slot).xyz-wa);
    let view=x-tr1.current_eye.xyz;
    let distance=sqrt(tr1_dot(view,view));
    let facing=tr1_dot(n,view);
    let unit=n*inverseSqrt(max(tr1_dot(n,n),1e-30));
    return Tr1Surface(vec4<f32>(x,1.0),id,u32(facing<=0.0),distance,unit,px,pn*inverseSqrt(max(tr1_dot(pn,pn),1e-30)),
        placed||any(a!=pa)||any(b!=pb)||any(c!=pc),max(textureLoad(tr1_sample,q,0).w-1.0,0.0));
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
    if tr1_still(s) {return vec2<f32>(p);}
    let q=vec2<f32>(p)+tr1_pixel(tr1.previous,s.previous_point,size)-tr1_pixel(tr1.current,s.point,size);
    // Rounding in two projections of one point is not motion: the sky under a camera
    // translation lands within 1e-6 px of its pixel, and must not be resampled (nor get the
    // Catmull-Rom window).
    let r=round(q);
    return select(q,r,abs(q-r)<vec2<f32>(1e-3));
}

// A previous-frame tap's latest sample point, rebuilt from its own camera ray (its pixel
// and the previous frame's sample offset) and its anchor distance.
fn tr1_anchor_point(t:vec2<i32>,distance:f32,size:vec2<f32>)->vec3<f32> {
    let o=tr1_sample_offset(t,vec2<f32>(tr1.current_eye.w,tr1.jitter.w));
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
        var ok=s.id==0u || tr1_still(s);
        if !ok {
            let point=tr1_anchor_point(t,bitcast<f32>(a.y),vec2<f32>(size));
            ok=abs(tr1_dot(s.previous_normal,point-s.previous_point.xyz))<=4.0*tr1.previous_eye.w*expected;
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
// Whether surface s restarts this frame (shaders/temporal_zones.wgsl): a static receiver
// whose light a moving caster's swept shadow can change (SK1), or any surface point in a
// changed local light's restart sphere (P-RELIT). It starts again from this frame's samples
// (tr1_continue), as on a frame that restarts every pixel.
fn tr1_restarts(s:Tr1Surface)->bool {
    if s.id==0u {return false;}
    // The zone tests first: both are off on most frames, and tr1_moving reads the placement.
    return (tr1_caster_shadow(s.point.xyz) && !tr1_moving(s.id-1u)) || tr1_relit(s.point.xyz);
}

fn tr1_history(p:vec2<i32>,s:Tr1Surface,size:vec2<u32>)->Tr1History {
    if tr1_restarts(s) {
        var rejected:Tr1History;return rejected;
    }
    let q=tr1_fetch_position(p,s,vec2<f32>(size));
    let base=vec2<i32>(floor(q));
    let f=q-floor(q);
    var expected=0.0;
    if s.id!=0u {
        let v=s.previous_point.xyz-tr1.previous_eye.xyz;
        expected=sqrt(tr1_dot(v,v));
    }
    let scale=tr1.jitter.z;
    var h:Tr1History;
    var total=0.0;
    var colour=vec3<f32>(0.0);
    var lo=vec3<f32>(3.4e38);
    var hi=vec3<f32>(-3.4e38);
    var shown=0u;
    // The four bilinear taps' colours, for Catmull-Rom's inner taps.
    var inner=array<vec3<f32>,4>();
    for(var i=0;i<4;i+=1) {
        let o=vec2<i32>(i&1,i>>1u);
        let x=tr1_tap(base+o,s,expected,size);
        if x.w>0.0 {
            let w=select(1.0-f.x,f.x,o.x!=0)*select(1.0-f.y,f.y,o.y!=0);
            let c=x.xyz*scale;
            inner[i]=c;
            total+=w;
            colour=fma(c,vec3<f32>(w),colour);
            h.weight=fma(x.w,w,h.weight);
            lo=min(lo,c);hi=max(hi,c);
            shown+=1u;
        }
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
            var c=vec3<f32>(0.0);
            if all(o>=vec2<i32>(0)) && all(o<=vec2<i32>(1)) {
                c=inner[o.y*2+o.x];
            } else {
                let x=tr1_tap(base+o,s,expected,size);
                outer=x.w>0.0;
                c=x.xyz*scale;
            }
            sharp=fma(c,vec3<f32>(wx[i&3]*wy[i>>2u]),sharp);
        }
        if outer {
            h.colour=clamp(sharp,lo,hi);
            h.window=min(tr1.windows.x,tr1.starts.x);
            // P-FLICKER: Catmull-Rom's own blur still varies with f, so its window follows the
            // same rule as bilinear's, with its own budget (0: the fixed window).
            if tr1.resample.x>0.0 {h.window=min(h.window,1.0+tr1.resample.x/blur);}
        }
    }
    h.window=min(h.window,tr1.windows.y);
    // MQ2: view-dependent output (reflections) moves across its surface as the view
    // direction turns, so surface-identity history lags it. Cap the window so the lag, in
    // pixels of the reflected image weighted by the program's declared share, stays within
    // the view budget: 1 + budget / drift, as the blur budget above.
    if s.view>0.0 && tr1.starts.w>0.0 {
        let drift=s.view*tr1_view_turn(s);
        if drift>0.0 {
            let cap=1.0+tr1.starts.w/drift;
            h.window=min(h.window,cap);
        }
    }
    // MQ1: while placements move, light they change on other surfaces (moving shadows,
    // reflections of moving objects) is held to this frame's 3x3 range.
    if (tr1.flags.y&TR1_BOX)!=0u {h.colour=clamp(h.colour,tr1_box_lo,tr1_box_hi);}
    return h;
}

// The area of a unit pixel box on the occluder side of a straight edge, n the edge's unit
// normal into the occluder and s the box centre's signed distance from the edge (positive
// inside), and its inverse.
fn tr1_box_area(s:f32,n:vec2<f32>)->f32 {
    let a=max(abs(n.x),abs(n.y));let b=min(abs(n.x),abs(n.y));
    let hi=0.5*(a+b);let lo=0.5*(a-b);
    if s>=hi {return 1.0;}
    if s<=-hi {return 0.0;}
    if abs(s)<=lo || b<1e-4 {return clamp(0.5+s/a,0.0,1.0);}
    let r=hi-abs(s);
    let corner=r*r/(2.0*a*b);
    return select(corner,1.0-corner,s>0.0);
}
fn tr1_box_distance(c:f32,n:vec2<f32>)->f32 {
    let a=max(abs(n.x),abs(n.y));let b=min(abs(n.x),abs(n.y));
    let hi=0.5*(a+b);
    let corner=0.5*b/a;
    if b<1e-4 || (c>=corner && c<=1.0-corner) {return (c-0.5)*a;}
    let k=2.0*a*b;
    if c<corner {return sqrt(k*max(c,0.0))-hi;}
    return hi-sqrt(k*max(1.0-c,0.0));
}

// The occluder's coverage of this pixel's footprint in the previous frame, fetched where the
// occluder's point was (the edge moves with the occluder): (coverage, weight), weight zero
// when no tap holds the pair. A tap of one surface alone covers 1 or 0; a silhouette tap
// reports its stored coverage. Also the window its resampling allows. Under motion the
// partly covered taps' coverages are carried as a straight edge (its normal from the 2x2
// taps' coverage gradient): each tap's edge distance is moved to the fetch point and the box
// area taken there, so a moving straight edge is not resampled (its window is then the
// coverage window at any sub-pixel offset).
fn tr1_coverage(p:vec2<i32>,o:Tr1Surface,u:u32,size:vec2<u32>)->vec3<f32> {
    let q=tr1_fetch_position(p,o,vec2<f32>(size));
    let base=vec2<i32>(floor(q));
    let f=q-floor(q);
    var total=0.0;var coverage=0.0;var weight=0.0;
    var taps=vec4<f32>(-1.0);
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
            taps[i]=c;
        }
    }
    if total<=0.0 {return vec3<f32>(0.0);}
    let blur=fma(f.x,1.0-f.x,f.y*(1.0-f.y));
    var window=tr1.windows.x;
    if blur>0.0 {window=min(min(window,1.0+tr1.windows.w/blur),tr1.windows.z);}
    if blur>0.0 {
        // The edge normal: the coverage gradient over the rows and columns of valid taps.
        var g=vec2<f32>(0.0);
        if taps.x>=0.0 && taps.y>=0.0 {g.x+=taps.y-taps.x;}
        if taps.z>=0.0 && taps.w>=0.0 {g.x+=taps.w-taps.z;}
        if taps.x>=0.0 && taps.z>=0.0 {g.y+=taps.z-taps.x;}
        if taps.y>=0.0 && taps.w>=0.0 {g.y+=taps.w-taps.y;}
        let gg=tr1_dot(vec3<f32>(g,0.0),vec3<f32>(g,0.0));
        if gg>=0.0025 {
            let n=g*inverseSqrt(gg);
            var sd=0.0;var sw=0.0;
            for(var i=0;i<4;i+=1) {
                let c=taps[i];
                if c<=0.001 || c>=0.999 {continue;}
                let d=vec2<i32>(i&1,i>>1u);
                let w=max(select(1.0-f.x,f.x,d.x!=0)*select(1.0-f.y,f.y,d.y!=0),1e-6);
                // This tap's edge distance at its centre, moved to the fetch point.
                sd=fma(tr1_box_distance(c,n)+tr1_dot(vec3<f32>(n,0.0),vec3<f32>(f-vec2<f32>(d),0.0)),w,sd);
                sw+=w;
            }
            if sw>0.0 {
                return vec3<f32>(tr1_box_area(sd/sw,n),weight,min(min(tr1.windows.x,tr1.windows.z),tr1.windows.y));
            }
        }
    }
    return vec3<f32>(coverage/total,weight,min(window,tr1.windows.y));
}

// A start from this frame's samples only: the tent-weighted mean of the 3x3 samples showing
// `id`, each weighted at its jittered position, and that estimate's effective sample count.
// At radius 2 and zero jitter the tent is the 3x3 [1,2,1] filter; at radius 1 it is bilinear
// interpolation to the pixel centre (the own sample alone at zero jitter). A surface the
// tent misses (shown only farther out in the 3x3) starts from the radius-2 tent.
fn tr1_spatial(p:vec2<i32>,id:u32,size:vec2<u32>)->vec4<f32> {
    let radius=tr1.starts.z;
    if radius<=0.0 && tr1_id(p)==id {return vec4<f32>(textureLoad(tr1_sample,tr1_map(p),0).xyz,1.0);}
    var sum=vec3<f32>(0.0);
    var w1=0.0;
    var w2=0.0;
    var wide=vec3<f32>(0.0);
    var v1=0.0;
    var v2=0.0;
    for(var i=0;i<9;i+=1) {
        let o=vec2<i32>(i%3,i/3)-vec2<i32>(1);
        let t=p+o;
        if !tr1_inside(t,size) || tr1_ring(i)!=id {continue;}
        let a=abs(vec2<f32>(o)+tr1_offset(t));
        let c=textureLoad(tr1_sample,tr1_map(t),0).xyz;
        let d=a/radius;
        let w=max(1.0-d.x,0.0)*max(1.0-d.y,0.0);
        sum=fma(c,vec3<f32>(w),sum);
        w1+=w;w2=fma(w,w,w2);
        let e=0.5*a;
        let v=max(1.0-e.x,0.0)*max(1.0-e.y,0.0);
        wide=fma(c,vec3<f32>(v),wide);
        v1+=v;v2=fma(v,v,v2);
    }
    if w1>0.0 {return vec4<f32>(sum/w1,w1*w1/w2);}
    if v1<=0.0 {return vec4<f32>(0.0);}
    return vec4<f32>(wide/v1,v1*v1/v2);
}
// The tent-weighted share of this frame's 3x3 samples showing `id` at the pixel centre: a
// one-frame coverage estimate.
fn tr1_share(id:u32)->f32 {
    var hit=0.0;var all=0.0;
    for(var i=0;i<9;i+=1) {
        let o=vec2<i32>(i%3,i/3)-vec2<i32>(1);
        let d=abs(vec2<f32>(o)+tr1_offset(tr1_here+o));
        let w=max(1.0-d.x,0.0)*max(1.0-d.y,0.0);
        all+=w;
        if tr1_ring(i)==id {hit+=w;}
    }
    return hit/max(all,1e-6);
}

// One surface's history continued with this frame's sample (the pixel's own surface), or
// started from this frame's samples when nothing continues it.
fn tr1_continue(p:vec2<i32>,s:Tr1Surface,sample:vec3<f32>,size:vec2<u32>)->vec4<f32> {
    if s.id==0u && tr1.background.w!=0.0 {return vec4<f32>(quantizeToF16(tr1.background.xyz),tr1.windows.x);}
    var h:Tr1History;
    if tr1.flags.x==0u {h=tr1_history(p,s,size);}
    if h.weight>0.0 {
        // This frame's sample weight: 1 at native resolution; upscaled, its tent weight.
        var w=tr1_weight(p);
        var x=sample;
        if !tr1_native() && (tr1.flags.y&TR1_UPSCALE_INTERPOLATION)!=0u {
            // TR2: young history interpolates the internal grid; the tent narrows to one output
            // pixel as the history reaches the sharp window.
            let young=clamp(1.0-h.weight/max(tr1.starts.x,1.0),0.0,1.0);
            let g=tr1_gather(p,s.id,mix(vec2<f32>(1.0),max(1.0/tr1_ratio,vec2<f32>(1.0)),young));
            x=g.xyz;
            w=min(g.w,1.0);
        }
        let n=clamp(h.weight+w,1.0,max(h.window,1.0));
        // m' = m + w (x - m) / n', with the multiply-add pinned so replays are bit-identical.
        return vec4<f32>(fma(x-h.colour,vec3<f32>(w/n),h.colour),n);
    }
    if (tr1.flags.y&TR1_SPATIAL_START)!=0u {
        let start=tr1_spatial(p,s.id,size);
        return vec4<f32>(start.xyz,clamp(start.w,1.0,max(tr1.starts.y,1.0)));
    }
    return vec4<f32>(sample,1.0);
}

// MQ2's view dependence (Tr1Surface.view: the hint minus 1, shaders/temporal_view.wgsl) in 8
// bits, for readers of the history that shade no sample of their own (reflection-hit reuse,
// docs/design-hit-reuse.md): 0 when view-independent (view 0), otherwise
// k = clamp(ceil(10 log2(view)) + 81, 1, 255): ten codes an octave, view 2^-8 at k 1, 1 at
// k 81, MQ2's 60000 cap at k 240. Read back as view = exp2((k - 81) / 10): never below the
// stored value (a smaller view reads as 2^-8), at most 2^0.1 (7%) above it, so a reader's
// view test errs toward shading.
fn tr1_view_code(view:f32)->u32 {
    if !(view>0.0) {return 0u;}
    return u32(clamp(ceil(10.0*log2(view))+81.0,1.0,255.0));
}
// The first slot's anchor word x for surface s.
fn tr1_anchor_word(s:Tr1Surface)->u32 {
    return (s.id&TR1_ID)|((s.side&1u)<<23u)|(tr1_view_code(s.view)<<24u);
}
fn tr1_store_single(p:vec2<i32>,s:Tr1Surface,film:vec4<f32>) {
    textureStore(tr1_anchor_out,p,vec4<u32>(tr1_anchor_word(s),bitcast<u32>(s.distance),0u,0u));
    textureStore(tr1_colour_out,p,film);
    textureStore(tr1_layer_out,p,vec4<u32>(TR1_NONE,0u,0u,0u));
    textureStore(tr1_output,p,vec4<f32>(film.xyz,1.0));
}

@compute @workgroup_size(8,8) fn tr1_resolve(@builtin(global_invocation_id) id:vec3<u32>,
        @builtin(local_invocation_index) lane:u32,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(tr1_output);
    let samples=textureDimensions(tr1_sample);
    tr1_same_grid=all(samples==size);
    tr1_ratio=select(vec2<f32>(samples)/vec2<f32>(size),vec2<f32>(1.0),tr1_same_grid);
    tr1_here=vec2<i32>(id.xy);
    // This frame's placements over the 10x10 tile, two loads per invocation at most.
    let corner=vec2<i32>(group.xy*8u)-vec2<i32>(1);
    for(var k=lane;k<100u;k+=64u) {
        let t=corner+vec2<i32>(i32(k%10u),i32(k/10u));
        tr1_tile[k]=select(TR1_NONE,tr1_id(t),tr1_inside(t,size));
    }
    workgroupBarrier();
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    tr1_local=p-corner-vec2<i32>(1);
    let s=tr1_surface(p,size);
    tr1_self=s.id;
    // A silhouette pixel: some 3x3 sample of this frame shows another surface. The other
    // surface is the nearest such sample's (edge neighbours before corners).
    var other=-1;
    // P-NRT: tr1_moving is a pure function of the placement and tr1_near_skin only ORs, so a
    // placement equal to the last one tested (`tested`, placement + 1) adds nothing; a ring
    // is nearly always one or two placements.
    var tested=0u;
    for(var i=0;i<9;i+=1) {
        let v=tr1_ring(i);
        if v!=0u && v!=tested {tested=v;tr1_near_skin=tr1_near_skin || tr1_moving(v-1u);}
        if v!=s.id && (other<0 || (i%2==1 && other%2==0)) {other=i;}
    }
    let sample=textureLoad(tr1_sample,tr1_map(p),0).xyz;
    if (tr1.flags.y&TR1_BOX)!=0u {
        tr1_box_lo=vec3<f32>(3.4e38);tr1_box_hi=vec3<f32>(-3.4e38);
        for(var i=0;i<9;i+=1) {
            let t=p+vec2<i32>(i%3,i/3)-vec2<i32>(1);
            if !tr1_inside(t,size) {continue;}
            let c=textureLoad(tr1_sample,tr1_map(t),0).xyz;
            tr1_box_lo=min(tr1_box_lo,c);tr1_box_hi=max(tr1_box_hi,c);
        }
    }
    var kept:Tr1Second;
    kept.id=TR1_NONE;
    // SK1: a character that left this pixel is not a still silhouette missed by
    // this frame's sample. Retire its retained slot, even outside the current ring.
    if tr1.flags.x==0u && tr1.flags.z!=0u {
        let first=textureLoad(tr1_anchor_in,p,0).x&TR1_ID;
        let second=tr1_second(p).id;
        if first!=0u && first!=tested {tr1_near_skin=tr1_near_skin || tr1_moving(first-1u);}
        if second!=0u && second!=TR1_NONE && second!=tested {tr1_near_skin=tr1_near_skin || tr1_moving(second-1u);}
    }
    // P-RELIT: not in a changed light's restart sphere, where the third-surface case below
    // would carry both stored colours at the light's old intensity.
    if tr1.flags.z!=0u && tr1.flags.x==0u && !tr1_near_skin && !s.moved && !(s.id!=0u && tr1_relit(s.point.xyz)) {
        // A still silhouette keeps its two surfaces whatever this frame's 3x3 shows: whether
        // the jittered 3x3 shows the other one depends on the jitter, not on the footprint,
        // and at a corner of three surfaces the jitter alternates which other one it shows;
        // replacing the second slot would discard that surface's history and the coverage.
        let own=tr1_second(p);
        let a=textureLoad(tr1_anchor_in,p,0).xy;
        let first=a.x&TR1_ID;
        // MQ1: a pair with a moved placement is a moving silhouette (below).
        if own.id!=TR1_NONE && !tr1_moved(own.id) && !tr1_moved(first) {
            if first==s.id || own.id==s.id {
                kept=own;
                if own.id==s.id {
                    // Last frame's sample showed the other surface: the first slot is the other.
                    kept.id=first;
                    kept.side=(a.x>>23u)&1u;
                }
                other=-1;
            } else {
                // This sample shows a third surface: the pixel carries its two surfaces'
                // state unchanged (the two-layer limit; docs/m7-tr1b.md).
                let scale=tr1.jitter.z;
                let c0=textureLoad(tr1_colour_in,p,0);
                let mine=vec4<f32>(c0.xyz*scale,c0.w);
                let theirs=own.colour*scale;
                textureStore(tr1_anchor_out,p,vec4<u32>(a,0u,0u));
                textureStore(tr1_colour_out,p,mine);
                let l=textureLoad(tr1_layer_in,p,0);
                textureStore(tr1_layer_out,p,vec4<u32>(l.x,pack2x16float(theirs.xy),pack2x16float(vec2<f32>(theirs.z,own.weight)),l.w));
                textureStore(tr1_output,p,vec4<f32>(mix(theirs,mine.xyz,own.coverage),1.0));
                return;
            }
        }
    }
    if other<0 && kept.id==TR1_NONE {
        // One surface: open constant background is the background exactly; an interior (or
        // the visible sky) continues its surface's history from either slot of the previous
        // frame.
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
        u=Tr1Surface(s.point,kept.id,kept.side,s.distance,s.normal,s.previous_point,s.previous_normal,s.moved,0.0);
        let first=textureLoad(tr1_anchor_in,p,0).x&TR1_ID;
        occluder_is_s=select(!kept.first_occludes,kept.first_occludes,first==s.id);
    }
    var o=u;
    if occluder_is_s {o=s;}
    let mine=tr1_continue(p,s,sample,size);
    // The other surface keeps its history (no sample of it this frame), or starts from this
    // frame's samples of it.
    var theirs=vec4<f32>(quantizeToF16(tr1.background.xyz),tr1.windows.x);
    if u.id!=0u || tr1.background.w==0.0 {
        var h:Tr1History;
        if tr1.flags.x==0u {h=tr1_history(p,u,size);}
        if h.weight>0.0 {
            theirs=vec4<f32>(h.colour,min(h.weight,max(h.window,1.0)));
            // A hidden surface's history gets no sample: under motion it is only resampled,
            // so it carries at most the start window once revealed.
            if !tr1_still(u) {theirs.w=min(theirs.w,max(tr1.starts.y,1.0));}
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
        let w=tr1_weight(p);
        coverage_weight=clamp(c.y+w,1.0,max(c.z,1.0));
        coverage=fma(select(0.0,1.0,occluder_is_s)-c.x,w/coverage_weight,c.x);
    }
    var front=theirs.xyz;var back=mine.xyz;
    if occluder_is_s {front=mine.xyz;back=theirs.xyz;}
    // Whole coverage shows one layer exactly: mix's arithmetic is not exact at its ends when
    // the layers differ by orders of magnitude (a bright sky behind rock).
    let film=select(select(mix(back,front,coverage),back,coverage<=0.0),front,coverage>=1.0);
    textureStore(tr1_anchor_out,p,vec4<u32>(tr1_anchor_word(s),bitcast<u32>(s.distance),0u,0u));
    textureStore(tr1_colour_out,p,mine);
    let first_coverage=select(1.0-coverage,coverage,occluder_is_s);
    textureStore(tr1_layer_out,p,vec4<u32>((u.id&TR1_ID)|((u.side&1u)<<23u)|select(0u,TR1_FIRST_OCCLUDES,occluder_is_s),pack2x16float(theirs.xy),
        pack2x16float(vec2<f32>(theirs.z,theirs.w)),pack2x16float(vec2<f32>(first_coverage,coverage_weight))));
    textureStore(tr1_output,p,vec4<f32>(film,1.0));
}
// ---- end of TR1 ----
