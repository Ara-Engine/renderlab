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

// Perceptual-roughness bands: one mirror ray below RL_MIRROR; RL_GLOSSY_RAYS fixed VNDF
// strata to RL_TRACED; the traced fraction falls linearly to 0 at RL_UNTRACED, where the
// program's own untraced estimate (ES1's `indirect_radiance` and lobes) takes over.
// RL_TRACED and RL_UNTRACED are the defaults: the frame's ReflectionSettings
// (primary_frame.reflection, src/reflections.rs) set both, or switch tracing off.
const RL_MIRROR:f32=0.05;
const RL_TRACED:f32=0.35;
const RL_UNTRACED:f32=0.5;
const RL_GLOSSY_RAYS:u32=4u;
const RL_FAR:f32=1.0e6;

var<private> rl_specular:vec4<f32>;
// Shading a reflection hit: light visibility traces its own shadow ray (rl_hit_shadow),
// because the frame's visibility inputs belong to the primary receivers.
var<private> rl_reflection_hit:bool;

// Mean exposed radiance arriving over the declared lobe (rgb) and the traced fraction (w):
// 1 inside the traced bands, fading to 0 across the rough cutoff. w = 0 means untraced.
fn specular_incident(input:SurfaceInput)->vec4<f32> { return rl_specular; }

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
    let receiver=Receiver(receiver_world,geometric,vec2<i32>(pixel),info&0x1fffffffu,primary_epsilon(positions,hit.placement),(info&0x20000000u)!=0u);
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
    let count=select(RL_GLOSSY_RAYS,1u,mirror);
    let cone_alpha=select(alpha*inverseSqrt(f32(count)),0.0,mirror);
    let spread=pixel_angle+curvature+2.0*cone_alpha;
    let basis=rl_basis(n);
    let local_v=vec3<f32>(dot(v,basis[0]),dot(v,basis[1]),dot(v,basis[2]));
    // Fixed per-pixel azimuth; TR1 frames turn it by their rotation word (0 otherwise).
    let frame=primary_frame.sided.y;
    let seed=rl_hash((pixel.x*0x9e3779b1u)^rl_hash(pixel.y^(frame*0x85ebca6bu)));
    let rotation=f32(seed>>8u)*(1.0/16777216.0);
    let radial=select(0.5,fract(f32(frame)*0.6180339887),frame!=0u);
    var sum=vec3<f32>(0.0);
    var valid=0u;
    var shaded=false;
    // PP1/SK1 integration: reuse the function-scope query above. Naga 30 emits an
    // illegal opaque null for loop-local/conditionally initialized query variables.
    rl_reflection_hit=true;
    for(var k=0u;k<count;k+=1u) {
        var d=reflect(-v,n);
        if !mirror {
            let u=vec2<f32>((f32(k)+radial)/f32(count),fract(rotation+f32(k)*0.6180339887));
            d=reflect(-v,basis*rl_vndf(local_v,max(alpha,1e-6),u));
        }
        if dot(d,side)<=0.0 {continue;}
        rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,receiver.epsilon,RL_FAR,origin,d));
        while rayQueryProceed(&query) {}
        let hit=rayQueryGetCommittedIntersection(&query);
        var radiance=rl_sky(d,cone_alpha);
        if hit.kind!=0u {
            let h=rl_hit_input(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),origin,d,width,spread,pixel);
            rl_load_parameters(h.batch);
            let hs=surface(h.input);
            radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);
            shaded=true;
        }
        sum+=radiance;
        valid+=1u;
    }
    rl_reflection_hit=false;
    if shaded {rl_load_parameters(batch);}
    if valid==0u {return;}
    rl_specular=vec4<f32>(sum/f32(valid),traced);
}
// ---- end of M7-R1 traced reflections ----
