// ---- M7-R1 traced reflections: engine-private, composed by src/program.rs ----
// Composed only into the traced shading modules of hardware renderers, and only for
// programs that declare `fn reflection_lobe(s:Surface, input:SurfaceInput) -> SpecularLobe`
// and call `specular_incident(input)` (docs/m7-r1.md). Every other composition gets the
// zero stub, so those programs keep their untraced `indirect_radiance` estimate.
//
// Each pixel traces its own rays from its own traced primary hit and writes only its own
// value: no common projection, matching, reservoir, history or denoiser (P0's C1/C7,
// docs/m7-p0-claims.md). Traced frames admit one program, so every hit is shaded by that
// same program's `surface` and `shade`, with view vector -d and a ray-cone footprint; at a
// hit `specular_incident` is zero, so the hit takes its own untraced estimate (one bounce),
// and its indirect inputs read the open environment (no world radiance cache at hits).
// Temporal integration is TR1's: its frame rotation word turns the glossy strata.

// The program's specular lobe: GGX about `normal` with alpha (perceptual roughness
// squared), and `weight` > 0 to request rays. Non-GGX styles declare their nearest GGX
// lobe or none (m7-design 3.4, constraint 1).
struct SpecularLobe { normal:vec3<f32>, alpha:f32, weight:f32 };

// Perceptual-roughness bands: one mirror ray below RL_MIRROR; fixed VNDF strata to
// RL_TRACED; the traced fraction falls linearly to 0 at RL_UNTRACED, where the program's own
// untraced estimate (ES1's `indirect_radiance` and lobes) takes over.
// RL_TRACED and RL_UNTRACED are the defaults: the frame's ReflectionSettings
// (primary_frame.reflection, src/reflections.rs) set both, or switch tracing off.
const RL_MIRROR:f32=0.05;
const RL_TRACED:f32=0.35;
const RL_UNTRACED:f32=0.5;
// RQ1 (docs/m7-rq1.md): glossy strata grow with the lobe, ceil(RL_RAYS_PER_ROUGHNESS x
// roughness) clamped to [RL_GLOSSY_RAYS, RL_GLOSSY_MAX]: 4 to roughness 0.08, 12 at 0.25,
// 16 from 0.33. Wider lobes see more varied radiance; with four strata their per-frame grain
// widened TR1's motion box and left blocky smears on moving glossy metals. The count
// depends only on the declared lobe, so the estimate stays an unbiased stratified mean.
const RL_GLOSSY_RAYS:u32=4u;
const RL_GLOSSY_MAX:u32=16u;
const RL_RAYS_PER_ROUGHNESS:f32=48.0;
// U-RTVIS reflection history: without it (these stubs) every glossy receiver traces its full
// set each frame. The wgpu renderer's primary shading modules replace the block with
// shaders/reflections_history.wgsl when RENDERLAB_RL_HISTORY=1 (src/program.rs
// with_reflection_history; off by default). It reads history only under a still camera.
// ---- rl history stubs ----
const RL_HISTORY:bool=false;
var<private> rl_hist_m2:f32;
var<private> rl_hist_moved:bool;
fn rl_history_index()->u32 { return 0u; }
fn rl_history_lookup(receiver:Receiver,world:vec3<f32>,pixel:vec2<u32>,reflected:vec3<f32>)->vec4<f32> { return vec4<f32>(0.0); }
fn rl_history_store(pixel:vec2<u32>,slot:u32,world:vec3<f32>,mean:vec3<f32>,m2:f32,count:f32) {}
fn rl_history_resolve(mean:vec3<f32>,folded:f32,count:f32)->vec3<f32> { return mean; }
const RL_RESOLVE_PASS:bool=false;
fn rl_prepass()->bool { return false; }
fn rl_resolved(pixel:vec2<u32>)->vec4<f32> { return vec4<f32>(0.0); }
fn rl_sample_store(pixel:vec2<u32>,slot:u32,world:vec3<f32>,sample:vec3<f32>,m2:f32,full:bool,count:f32,rough:f32,seen:vec3<f32>,seen_m2:f32,dist:f32) {}
// ---- end of rl history stubs ----
// The receiver's perceptual roughness, for the history's resolve.
var<private> rl_hist_rough:f32;
// The lobe's footprint in screen pixels (about 2 alpha times the hit distance over the pixel's
// width at the receiver; 64 for a miss): the resolve's a-trous passes stay inside it.
var<private> rl_footprint:f32;
// U-RTVIS reflection pass: without it (these stubs) no lobe is written. The wgpu primary shading
// modules replace the block with shaders/reflections_lobe.wgsl while RENDERLAB_RL_PASS=1.
// ---- rl pass stubs ----
const RL_PASS:bool=false;
fn rl_lobe_store(pixel:vec2<u32>,origin:vec3<f32>,n:vec3<f32>,v:vec3<f32>,side:vec3<f32>,alpha:f32,width:f32,spread:f32) {}
// ---- end of rl pass stubs ----
// One closest-hit reflection ray: the hit record (x 0: a miss) and its distance (-1 for a miss).
// Software heads trace the same ray in software (src/program.rs SOFTWARE_REFLECTION_SEAMS).
struct RlCast {record:vec4<u32>,t:f32};
fn rl_cast(origin:vec3<f32>,d:vec3<f32>,tmin:f32)->RlCast {
    var query:ray_query;
    rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,tmin,RL_FAR,origin,d));
    while rayQueryProceed(&query) {}
    let hit=rayQueryGetCommittedIntersection(&query);
    if hit.kind==0u {return RlCast(vec4<u32>(0u),-1.0);}
    return RlCast(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),hit.t);
}
// A trusted receiver traces RL_HISTORY_STRATA strata of its set a frame (every
// count/RL_HISTORY_STRATA-th, from an offset turning with the frame) and folds them into a
// history of at most RL_HISTORY_CAP x count strata; one that moved over a pixel since the
// previous frame keeps at most that / RL_HISTORY_MOVING (its reflection moves with the
// parallax). src/program.rs RENDERLAB_RL_HISTORY_TUNE sets them for measurement.
const RL_HISTORY_STRATA:u32=4u;
// Rough lobes (perceptual roughness RL_HISTORY_ROUGH and up) trace RL_HISTORY_ROUGH_STRATA strata a
// frame instead: their reflection is noisier per stratum.
const RL_HISTORY_ROUGH_STRATA:u32=4u;
const RL_HISTORY_ROUGH:f32=0.3;
const RL_HISTORY_CAP:f32=1.0;
const RL_HISTORY_MOVING:f32=2.0;
// A stratum disagrees with the history when its luminance is more than RL_HISTORY_SIGMA
// standard deviations (the stored strata spread) plus RL_HISTORY_SLACK of the mean away.
const RL_HISTORY_SIGMA:f32=4.0;
const RL_HISTORY_SLACK:f32=0.1;
// A receiver the resolve restarts this frame (rl_restarts: a changed local light's restart
// sphere, a moving caster's swept shadow) shows this frame's strata alone, so its glossy
// lobe traces primary_frame.reflection.w times the strata (1 or less: the ordinary count;
// U-SPECK2, docs/speck2.md).
const RL_FAR:f32=1.0e6;
// Lone-stratum bound (U-SPECK2, docs/speck2.md). A glossy pixel's estimate is the mean of at
// most 16 strata, and temporal reconstruction averages some 16 frames of it: about 256
// samples. A target one ray in tens of thousands strikes (a lamp's bulb seen from across a
// yard, hundreds of times brighter than the rest of its lobe) cannot average out at that
// count; each strike is a white pixel that lasts as long as its history. So the brightest
// stratum counts for at most primary_frame.reflection.z times the luminance of all the
// others together (1: at most half the estimate; 0: no bound). Two strata on the same
// feature bound nothing, so a feature the lobe resolves keeps its energy; what the bound
// removes is the mean of targets too rare to draw (measured in docs/speck2.md). Mirror
// rays (one ray, the exact image) are never bound.
const RL_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);

var<private> rl_specular:vec4<f32>;
// Shading a reflection hit: light visibility traces its own shadow ray (rl_hit_shadow),
// because the frame's visibility inputs belong to the primary receivers.
var<private> rl_reflection_hit:bool;
// U-RTVIS: the stratum a reflection hit is shaded for, of this pixel's rl_hit_strata. Light
// visibility at hits splits its samples across the strata (src/program.rs RL_SPLIT).
var<private> rl_hit_stratum:u32;
var<private> rl_hit_strata:u32=1u;

// Mean exposed radiance arriving over the declared lobe (rgb) and the traced fraction (w):
// 1 inside the traced bands, fading to 0 across the rough cutoff. w = 0 means untraced.
fn specular_incident(input:SurfaceInput)->vec4<f32> { return rl_specular; }

// A unit direction as an octahedral snorm16x2 word, and back: reflection hit records' word 10
// (docs/design-reflection-pass.md), shared by rl_trace and the Mac's reflection_hits. The round
// trip is within the snorm16 step, about 1e-4 rad.
fn rl_oct_encode(d:vec3<f32>)->u32 {
    let p=d.xy/(abs(d.x)+abs(d.y)+abs(d.z));
    let sign=select(vec2<f32>(-1.0),vec2<f32>(1.0),p>=vec2<f32>(0.0));
    let folded=select((vec2<f32>(1.0)-abs(p.yx))*sign,p,d.z>=0.0);
    return pack2x16snorm(folded);
}
fn rl_oct_decode(word:u32)->vec3<f32> {
    let e=unpack2x16snorm(word);
    let z=1.0-abs(e.x)-abs(e.y);
    let sign=select(vec2<f32>(-1.0),vec2<f32>(1.0),e>=vec2<f32>(0.0));
    let xy=select((vec2<f32>(1.0)-abs(e.yx))*sign,e,z>=0.0);
    return normalize(vec3<f32>(xy,z));
}
fn rl_hash(input:u32)->u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
// Orthonormal tangent, bitangent and n (Duff et al. 2017).
fn rl_basis(n:vec3<f32>)->mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
// GGX visible-normal sample about +z for local view v (Heitz 2018), at stratum u.
fn rl_vndf(v:vec3<f32>,alpha:f32,u:vec2<f32>)->vec3<f32> {
    let vh=normalize(vec3<f32>(alpha*v.x,alpha*v.y,v.z));
    let lensq=vh.x*vh.x+vh.y*vh.y;
    let t1=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(-vh.y,vh.x,0.0)*inverseSqrt(max(lensq,1e-30)),lensq>1e-12);
    let t2=cross(vh,t1);
    let r=sqrt(u.x);let phi=6.283185307179586*u.y;
    let p1=r*cos(phi);let s=0.5*(1.0+vh.z);
    let p2=(1.0-s)*sqrt(max(1.0-p1*p1,0.0))+s*r*sin(phi);
    let nh=p1*t1+p2*t2+sqrt(max(1.0-p1*p1-p2*p2,0.0))*vh;
    return normalize(vec3<f32>(alpha*nh.x,alpha*nh.y,max(nh.z,1e-6)));
}

// The environment an escaped ray sees, exposed, prefiltered over its sub-cone.
fn rl_sky(d:vec3<f32>,alpha:f32)->vec3<f32> {
    if environment_enabled() {return environment_product(environment_reflection(d,sqrt(alpha)),vec3<f32>(1.0));}
    return exposed_product(environment_constant(),vec3<f32>(1.0));
}

// One hard shadow ray toward the light from a reflection hit, on the frame's TLAS.
fn rl_hit_shadow(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {
    // SK1 integration: function-scope queries avoid Naga 30 emitting an illegal
    // OpConstantNull for a query declared after conditional control flow.
    var query:ray_query;
    if light.kind.z==0u {return 1.0;}
    let side=select(-receiver.geometric,receiver.geometric,dot(receiver.geometric,l)>=0.0);
    let origin=fma(side,vec3<f32>(receiver.epsilon),receiver.world);
    var tmax=RL_FAR;
    if light.kind.x!=0u {tmax=max(distance(light.position_range.xyz,origin)-receiver.epsilon,0.0);}
    if tmax<=receiver.epsilon {return 1.0;}
    rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT|RAY_FLAG_FORCE_OPAQUE,255u,receiver.epsilon,tmax,origin,l));
    while rayQueryProceed(&query) {}
    return select(1.0,0.0,rayQueryGetCommittedIntersection(&query).kind!=0u);
}

struct RlHit {input:SurfaceInput,batch:u32,visible:bool};
// A reflection hit's receiver, built as the traced entry builds a camera hit's, with the
// reflection origin as its eye and a ray-cone footprint: `width` metres at the origin,
// growing `spread` metres per metre along the ray.
fn rl_hit_input(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,width:f32,spread:f32,pixel:vec2<u32>)->RlHit {
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let a=primary_vertex(hit,ta,hit.ia);let b=primary_vertex(hit,tb,hit.ib);let c=primary_vertex(hit,tc,hit.ic);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    let world=world_position(ta.position*w.x+tb.position*w.y+tc.position*w.z,hit.placement).xyz;
    let receiver_world=world_position(receiver_point(ta.position,tb.position,tc.position,w),hit.placement).xyz;
    let geometric=primary_geometric(ta.position,tb.position,tc.position,hit.placement);
    let source_front=dot(geometric,origin-world)>=0.0;
    let m=models[hit.placement];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let e1=linear*(tb.position-ta.position);let e2=linear*(tc.position-ta.position);
    let basis=rl_basis(d);
    let cone=PrimaryRay(origin,origin+d,basis[0]*width,basis[1]*width,basis[0]*(width+spread),basis[1]*(width+spread));
    let gradients=primary_gradients(cone,world,geometric,e1,e2,tb.uv-ta.uv,tc.uv-ta.uv);
    let bary_gradients=primary_gradients(cone,world,geometric,e1,e2,vec2<f32>(1.0,0.0),vec2<f32>(0.0,1.0));
    let normal_dx=(b.normal-a.normal)*bary_gradients[0].x+(c.normal-a.normal)*bary_gradients[0].y;
    let normal_dy=(b.normal-a.normal)*bary_gradients[1].x+(c.normal-a.normal)*bary_gradients[1].y;
    let front=source_front==(a.winding>0.0);
    let receiver_geometric=select(geometric,-geometric,source_front);
    let lit_front=front==(a.winding>0.0);
    let face=-receiver_geometric;
    let source_geometric=select(-face,face,lit_front);
    let material=primary_materials[hit.batch].value;
    let positions=array<vec3<f32>,3>(ta.position,tb.position,tc.position);
    let info=primary_placements[hit.placement];
    // The hit's shadow origin is offset toward each light (shadow_origin's `thin` side, as
    // rl_hit_shadow's): its viewed side is the reflection ray's, not the camera's, and the
    // camera-side offset started a hit's local-light rays inside every surface that faces
    // away from the camera (U-SPECK2, docs/speck2.md).
    let receiver=Receiver(receiver_world,geometric,vec2<i32>(pixel),info&0x1fffffffu,primary_epsilon(positions,hit.placement),true);
    // DC1's world-space decals project onto hits as onto camera receivers.
    let input=decal_prepare(SurfaceInput(world,e1*bary_gradients[0].x+e2*bary_gradients[0].y,e1*bary_gradients[1].x+e2*bary_gradients[1].y,source_geometric,
        a.normal*w.x+b.normal*w.y+c.normal*w.z,normal_dx,normal_dy,a.tangent*w.x+b.tangent*w.y+c.tangent*w.z,a.handedness,
        a.uv*w.x+b.uv*w.y+c.uv*w.z,gradients[0],gradients[1],a.color*w.x+b.color*w.y+c.color*w.z,lit_front,origin,pixel,material,receiver,decal_none()));
    // A single-sided draw's source-back face blocks the ray and sends nothing.
    return RlHit(input,hit.batch,material.double_sided!=0u || lit_front);
}

// Traces this receiver's declared lobe before its `shade` reads `specular_incident`.
// Called by the traced entry after `surface` and before the receiver's indirect inputs are
// prepared (src/program.rs); hits read the open environment state (`rl_indirect_open`: the
// cache-off values, no cache lookup per hit). Restores the receiver's parameters.
fn rl_reflection_prepare(s:Surface,input:SurfaceInput,batch:u32,pixel:vec2<u32>,size:vec2<u32>) {
    var query:ray_query;
    rl_specular=vec4<f32>(0.0);
    if !rl_term(RL_TERM_REFLECTIONS) {return;}
    let lobe=reflection_lobe(s,input);
    let alpha=clamp(lobe.alpha,0.0,1.0);
    let rough=sqrt(alpha);
    let bands=primary_frame.reflection;
    let traced=clamp((bands.x-rough)/max(bands.x-bands.y,1e-6),0.0,1.0);
    if traced<=0.0 || !(lobe.weight>0.0) {return;}
    let n=unit_normal(lobe.normal);
    let v=unit_normal(input.eye-input.world);
    if dot(n,n)<0.5 || dot(v,v)<0.5 || dot(n,v)<=1e-4 {return;}
    let receiver=input.receiver;
    let side=select(-receiver.geometric,receiver.geometric,dot(receiver.geometric,v)>=0.0);
    let origin=fma(side,vec3<f32>(receiver.epsilon),receiver.world);
    // Ray cone: the pixel's footprint at the receiver, widened by its angle, the normal's
    // curvature across the pixel (doubled on reflection) and, for glossy, the sub-cone.
    let camera=primary_ray(pixel,size);
    let pixel_angle=length(camera.dx_far-camera.dx_near)/max(length(camera.far-camera.near),1e-30);
    let width=pixel_angle*length(input.world-input.eye);
    let curvature=2.0*max(length(input.normal_dx),length(input.normal_dy))/max(length(input.normal),1e-30);
    let mirror=rough<RL_MIRROR;
    // The resolve's two shading dispatches: the pre-pass traces glossy lobes only (mirrors stay
    // in the shading pass); the shading pass shows this frame's resolved value where it exists.
    if RL_RESOLVE_PASS && mirror && rl_prepass() {return;}
    if RL_RESOLVE_PASS && !mirror && !rl_prepass() {
        let resolved=rl_resolved(pixel);
        if resolved.w>0.0 {rl_specular=vec4<f32>(resolved.rgb,traced);return;}
    }
    var count=select(clamp(u32(ceil(RL_RAYS_PER_ROUGHNESS*rough)),RL_GLOSSY_RAYS,RL_GLOSSY_MAX),1u,mirror);
    let restart_strata=primary_frame.reflection.w;
    let restarted=!mirror && restart_strata>1.0 && rl_restarts(input.world);
    if restarted {count*=u32(restart_strata);}
    let cone_alpha=select(alpha*inverseSqrt(f32(count)),0.0,mirror);
    let spread=pixel_angle+curvature+2.0*cone_alpha;
    let basis=rl_basis(n);
    let local_v=vec3<f32>(dot(v,basis[0]),dot(v,basis[1]),dot(v,basis[2]));
    // Fixed per-pixel azimuth; TR1 frames turn it by their rotation word (0 otherwise).
    let frame=primary_frame.sided.y;
    let seed=rl_hash((pixel.x*0x9e3779b1u)^rl_hash(pixel.y^(frame*0x85ebca6bu)));
    let rotation=f32(seed>>8u)*(1.0/16777216.0);
    let radial=select(0.5,fract(f32(frame)*0.6180339887),frame!=0u);
    // The reflection pass's lobe (RL_PASS): glossy receivers only; mirrors stay inline.
    if RL_PASS && !mirror {rl_lobe_store(pixel,origin,n,v,side,alpha,width,pixel_angle+curvature);}
    // Reflection history: a trusted glossy receiver traces one stratum, rotating with the
    // frame from a per-pixel offset, and the full set only when that stratum disagrees.
    var hist=vec4<f32>(0.0);
    rl_hist_rough=rough;
    // A restart sphere or swept shadow (rl_restarts) always traces the full set.
    if RL_HISTORY && !mirror && !restarted && frame!=0u && !rl_restarts(input.world) {hist=rl_history_lookup(receiver,input.world,pixel,reflect(-v,n));}
    var trusted=hist.w>0.0;
    // Fixed per-pixel offset, so `stride` consecutive frames visit every stratum once.
    let per_frame=select(RL_HISTORY_STRATA,RL_HISTORY_ROUGH_STRATA,rough>=RL_HISTORY_ROUGH);
    let stride=max(count/max(per_frame,1u),1u);
    let one=(rl_history_index()+(rl_hash((pixel.x*0x9e3779b1u)^rl_hash(pixel.y))&0xffffu))%stride;
    var sum=vec3<f32>(0.0);
    // The brightest stratum (rgb) and its luminance (w), for the lone-stratum bound.
    var top=vec4<f32>(0.0);
    var valid=0u;
    var shaded=false;
    var squares=0.0;
    // The traced strata's hit distances (software: the traversal's own), for the resolve's
    // reprojection by the reflected point.
    var rl_dist=0.0;var rl_dist_n=0.0;
    // PP1/SK1 integration: reuse the function-scope query above. Naga 30 emits an
    // illegal opaque null for loop-local/conditionally initialized query variables.
    rl_reflection_hit=true;
    // Path regularization (P-DEFECT, docs/p-defect.md): hits see their lights through this
    // ray's cone, of half-angle `spread`, which moves a hit's half-vector by up to spread/2.
    // The lobe widens by a half-vector slope disk of radius 4 x spread (per-axis slope
    // variance 4 spread^2, GGX alpha^2 8 spread^2): a glossy stratum's cone is about a quarter
    // of its lobe, so a hit's highlight then spans about all of the pixel's strata and a
    // moving or just-cut glossy surface sees a smooth reflected highlight. Narrower disks
    // left grain on moving glossy objects (radius 2 spread: about four strata). The reflected
    // highlight widens by about sqrt(2) of the receiver's own lobe; its energy is unchanged.
    // Mirror rays from flat receivers stay near exact.
    rl_hit_variance=8.0*spread*spread;
    for(var attempt=0u;attempt<2u;attempt+=1u) {
    let first=select(0u,one,trusted);
    let step=select(1u,stride,trusted);
    rl_hit_strata=select(count,(count-first+stride-1u)/stride,trusted);
    sum=vec3<f32>(0.0);top=vec4<f32>(0.0);valid=0u;squares=0.0;rl_dist=0.0;rl_dist_n=0.0;
    for(var k=first;k<count;k+=step) {
        rl_hit_stratum=select(k,k/stride,trusted);
        var d=reflect(-v,n);
        if !mirror {
            let u=vec2<f32>((f32(k)+radial)/f32(count),fract(rotation+f32(k)*0.6180339887));
            d=reflect(-v,basis*rl_vndf(local_v,max(alpha,1e-6),u));
        }
        if dot(d,side)<=0.0 {continue;}
        rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,receiver.epsilon,RL_FAR,origin,d));
        while rayQueryProceed(&query) {}
        let hit=rayQueryGetCommittedIntersection(&query);
        // Only a miss looks up the sky: the compiler does not sink a lookup the hit
        // branch overwrites (P-SKY, docs/p-sky.md).
        var radiance=vec3<f32>(0.0);
        if hit.kind!=0u {
            let record=vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics));
            rl_dist+=hit.t;rl_dist_n+=1.0;
            // A rough lobe's cone wider than the radiance cache's cell at the hit reads that
            // cell's outgoing radiance instead of shading the hit (reflection_cache_hits.wgsl).
            let cached=rl_cache_hit(record,origin,d,hit.t,width+spread*hit.t,rough);
            if cached.w>0.0 {
                radiance=rl_finite(expose(cached.rgb));
            } else {
                let h=rl_hit_input(record,origin,d,width,spread,pixel);
                // LC1 at the hit: its own cluster where that walk is exact (the same sum).
                rl_hit_lights_prepare(h.input.world);
                rl_load_parameters(h.batch);
                let hs=surface(h.input);
                radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);
                shaded=true;
            }
        } else {
            radiance=rl_sky(d,cone_alpha);
        }
        sum+=radiance;
        let level=dot(radiance,RL_LUMA);
        if level>top.w {top=vec4<f32>(radiance,level);}
        squares+=level*level;
        valid+=1u;
    }
    // The one stratum against the stored mean: within its noise it folds in; beyond it, the
    // history is not trusted and the full set traces now (never blend in doubt).
    if !trusted || valid==0u {break;}
    let mean_level=dot(hist.rgb,RL_LUMA);
    // The subset mean's noise: the strata spread over the subset's size.
    let sigma=sqrt(max(rl_hist_m2-mean_level*mean_level,0.0)/f32(valid));
    if abs(dot(sum,RL_LUMA)/f32(valid)-mean_level)<=RL_HISTORY_SIGMA*sigma+RL_HISTORY_SLACK*mean_level+1e-4 {break;}
    trusted=false;
    }
    rl_reflection_hit=false;
    rl_hit_lights_clear();
    rl_hit_strata=1u;
    rl_hit_variance=0.0;
    if shaded {rl_load_parameters(batch);}
    let slot=receiver.slot;
    if trusted && RL_RESOLVE_PASS {
        // rl_resolve folds this frame's sample; the receiver shows the resolved history.
        let fresh=select(hist.rgb,sum/f32(max(valid,1u)),valid>0u);
        let fresh_m2=select(rl_hist_m2,squares/f32(max(valid,1u)),valid>0u);
        rl_footprint=select(64.0,2.0*alpha*(rl_dist/max(rl_dist_n,1.0))/max(width,1e-6),rl_dist_n>0.0);
        rl_sample_store(pixel,slot,input.world,fresh,fresh_m2,false,hist.w,rough,hist.rgb,rl_hist_m2,select(0.0,rl_dist/rl_dist_n,rl_dist_n>0.0));
        rl_specular=vec4<f32>(hist.rgb,traced);
        return;
    }
    if trusted {
        var mean=hist.rgb;var m2=rl_hist_m2;var folded=hist.w;
        if valid>0u {
            // The lone-stratum bound, against the set the history stands for.
            let lone=primary_frame.reflection.z;
            // The brightest stratum against the set the history stands for.
            var fresh=sum;
            let most=lone*f32(count-1u)*mean_level_of(hist.rgb);
            if lone>0.0 && top.w>most {fresh-=top.rgb*(1.0-most/top.w);}
            let full=RL_HISTORY_CAP*f32(count);
            let cap=select(full,max(full/RL_HISTORY_MOVING,2.0),rl_hist_moved);
            let n=f32(valid);
            folded=min(folded+n,cap);
            mean=mix(mean,fresh/n,n/folded);
            m2=mix(m2,squares/n,n/folded);
        }
        rl_history_store(pixel,slot,input.world,mean,m2,folded);
        rl_specular=vec4<f32>(rl_history_resolve(mean,folded,f32(count)),traced);
        return;
    }
    if valid==0u {
        if RL_HISTORY && !mirror {rl_history_store(pixel,slot,input.world,vec3<f32>(0.0),0.0,0.0);}
        return;
    }
    let lone=primary_frame.reflection.z;
    if lone>0.0 && valid>1u {
        let rest=max(dot(sum,RL_LUMA)-top.w,0.0)*lone;
        if top.w>rest {sum-=top.rgb*(1.0-rest/top.w);}
    }
    let mean=sum/f32(valid);
    // The full set seeds the history as count strata, with their spread.
    if RL_RESOLVE_PASS && !mirror {
        rl_footprint=select(64.0,2.0*alpha*(rl_dist/max(rl_dist_n,1.0))/max(width,1e-6),rl_dist_n>0.0);
        rl_sample_store(pixel,slot,input.world,mean,squares/f32(valid),true,f32(count),rough,vec3<f32>(0.0),0.0,select(0.0,rl_dist/rl_dist_n,rl_dist_n>0.0));
    } else if RL_HISTORY && !mirror {rl_history_store(pixel,slot,input.world,mean,squares/f32(valid),f32(count));}
    rl_specular=vec4<f32>(mean,traced);
}
fn mean_level_of(c:vec3<f32>)->f32 { return dot(c,RL_LUMA); }
// ---- end of M7-R1 traced reflections ----
