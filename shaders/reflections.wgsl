// ---- M7-R1 traced reflections: engine-private, composed by src/program.rs ----
// Composed only into the traced shading modules of hardware renderers, and only for
// programs that declare `fn reflection_lobe(s:Surface, input:SurfaceInput) -> SpecularLobe`
// and call `specular_incident(input)` (docs/m7-r1.md). Every other composition gets the
// zero stub, so those programs keep their untraced `indirect_radiance` estimate.
//
// Each pixel traces its own rays from its own traced primary hit; its shading group then
// shares those shaded hits between neighbouring receivers of one surface (P-GLOSSY,
// rl_reflection_share below), with no history or denoiser. Traced frames admit one
// program, so every hit is shaded by that
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
const RL_GLOSSY_TRACED:u32=16u;
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
// `lane` is the pixel's slot in its shading group for rl_reflection_share.
fn rl_reflection_prepare(s:Surface,input:SurfaceInput,batch:u32,pixel:vec2<u32>,size:vec2<u32>,lane:u32) {
    var query:ray_query;
    rl_specular=vec4<f32>(0.0);
    rl_g=RlGather();
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
    // The footprint count sets every cone; the traced count is how many of those strata
    // this frame takes (P-GLOSSY, docs/p-glossy.md).
    let footprint=select(clamp(u32(ceil(RL_RAYS_PER_ROUGHNESS*rough)),RL_GLOSSY_RAYS,RL_GLOSSY_MAX),1u,mirror);
    let count=min(footprint,RL_GLOSSY_TRACED);
    let cone_alpha=select(alpha*inverseSqrt(f32(footprint)),0.0,mirror);
    let spread=pixel_angle+curvature+2.0*cone_alpha;
    let basis=rl_basis(n);
    let local_v=vec3<f32>(dot(v,basis[0]),dot(v,basis[1]),dot(v,basis[2]));
    // Fixed per-pixel azimuth; TR1 frames turn it by their rotation word (0 otherwise).
    let frame=primary_frame.sided.y;
    let seed=rl_hash((pixel.x*0x9e3779b1u)^rl_hash(pixel.y^(frame*0x85ebca6bu)));
    let rotation=f32(seed>>8u)*(1.0/16777216.0);
    let radial=select(0.5,fract(f32(frame)*0.6180339887),frame!=0u);
    rl_g=RlGather(origin,n,v,side,alpha,traced,width,0u,mirror,true);
    var sum=vec3<f32>(0.0);
    var valid=0u;
    var shaded=false;
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
        // Only a miss looks up the sky: the compiler does not sink a lookup the hit
        // branch overwrites (P-SKY, docs/p-sky.md).
        var radiance=vec3<f32>(0.0);
        var point=vec4<f32>(d,0.0);
        if hit.kind!=0u {
            let h=rl_hit_input(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),origin,d,width,spread,pixel);
            rl_load_parameters(h.batch);
            let hs=surface(h.input);
            radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);
            shaded=true;
            point=vec4<f32>(h.input.world,1.0);
        } else {
            radiance=rl_sky(d,cone_alpha);
        }
        sum+=radiance;
        if RL_SHARE && valid<RL_SHARE_SLOTS {
            rl_share_point[lane*RL_SHARE_SLOTS+valid]=point;
            rl_share_radiance[lane*RL_SHARE_SLOTS+valid]=vec4<f32>(radiance,rl_lobe_pdf(n,v,alpha,d));
        }
        valid+=1u;
    }
    rl_reflection_hit=false;
    rl_hit_variance=0.0;
    if shaded {rl_load_parameters(batch);}
    rl_g.count=valid;
    if valid==0u {return;}
    rl_specular=vec4<f32>(sum/f32(valid),traced);
}

// ---- P-GLOSSY shared strata (docs/p-glossy.md) ----
// Each pixel traces RL_GLOSSY_TRACED of its footprint's strata and leaves every sample in
// workgroup memory: the hit point (or, for a miss, the direction) with the radiance its own
// hit program shaded and the probability its own lobe drew it with. After one barrier each
// glossy receiver reconnects to its 3x3 neighbours' samples inside the shading group: a
// neighbour's hit is seen from this receiver's origin along a new direction, weighted by
// this receiver's lobe density over the sample's own (pairwise, bounded by 2) and
// self-normalized. The reflected image stays this receiver's own (each sample is placed by
// its hit point, not its ray), a rare bright hit spreads over the neighbourhood with its
// energy kept, and neighbours across a depth or normal edge, mirrors and below-plane
// directions are not reused. Hits are still shaded only by their own program; reuse
// changes which receiver averages a shaded value, never how it is shaded.
const RL_SHARE_SLOTS:u32=4u;
const RL_SHARE:bool=RL_GLOSSY_TRACED<=RL_SHARE_SLOTS;
// Rows of every shading group (8x4 tiles, 4x4 lists).
const RL_SHARE_ROWS:u32=4u;
struct RlGather {origin:vec3<f32>,n:vec3<f32>,v:vec3<f32>,side:vec3<f32>,alpha:f32,traced:f32,width:f32,count:u32,mirror:bool,active:bool};
var<private> rl_g:RlGather;
var<workgroup> rl_share_point:array<vec4<f32>,128>;
var<workgroup> rl_share_radiance:array<vec4<f32>,128>;
// Per lane: origin and sample count (-1: a mirror), then the receiver's side and footprint.
var<workgroup> rl_share_origin:array<vec4<f32>,32>;
var<workgroup> rl_share_side:array<vec4<f32>,32>;

// The density of direction w under the VNDF strata of lobe (n, v, alpha), per steradian.
fn rl_lobe_pdf(n:vec3<f32>,v:vec3<f32>,alpha:f32,w:vec3<f32>)->f32 {
    let h=normalize(v+w);
    let nh=dot(n,h);let nv=dot(n,v);
    if nh<=0.0 || nv<=0.0 || dot(v,h)<=0.0 {return 0.0;}
    let a2=max(alpha*alpha,1e-12);
    let k=nh*nh*(a2-1.0)+1.0;
    let g1=2.0*nv/(nv+sqrt(a2+(1.0-a2)*nv*nv));
    return a2*g1/(12.566370614359172*k*k*nv);
}

// Exchanges this group's samples and sets the receiver's `specular_incident` from them.
// Called by the traced entry in uniform control flow, after every lane's
// rl_reflection_prepare (or none, `live` false); `width` is the group's lanes per row.
fn rl_reflection_share(lane:u32,width:u32,live:bool) {
    let g=rl_g;
    let count=select(0.0,select(f32(g.count),-1.0,g.mirror),live && g.active && RL_SHARE);
    rl_share_origin[lane]=vec4<f32>(g.origin,count);
    rl_share_side[lane]=vec4<f32>(g.side,g.width);
    workgroupBarrier();
    if count<=0.0 {return;}
    let x=i32(lane%width);let y=i32(lane/width);
    var sum=vec3<f32>(0.0);
    var weights=0.0;
    for(var dy=-1;dy<=1;dy+=1) {
        for(var dx=-1;dx<=1;dx+=1) {
            let nx=x+dx;let ny=y+dy;
            if nx<0 || ny<0 || nx>=i32(width) || ny>=i32(RL_SHARE_ROWS) {continue;}
            let j=u32(ny)*width+u32(nx);
            let o=rl_share_origin[j];
            if o.w<=0.0 {continue;}
            let own=j==lane;
            if !own {
                // Same surface only: facing alike and within two footprints of the plane.
                let s=rl_share_side[j];
                if dot(s.xyz,g.side)<0.9 || abs(dot(g.side,o.xyz-g.origin))>2.0*max(g.width,s.w) {continue;}
            }
            for(var k=0u;k<u32(o.w);k+=1u) {
                let p=rl_share_point[j*RL_SHARE_SLOTS+k];
                let r=rl_share_radiance[j*RL_SHARE_SLOTS+k];
                var w=1.0;
                if !own {
                    var dir=p.xyz;
                    var jacobian=1.0;
                    if p.w>0.5 {
                        // Reconnect to the hit: solid angle scales by the squared distances.
                        let to=p.xyz-g.origin;let back=p.xyz-o.xyz;
                        let ri=dot(to,to);
                        jacobian=ri/max(dot(back,back),1e-30);
                        if jacobian<0.5 || jacobian>2.0 {continue;}
                        dir=to*inverseSqrt(max(ri,1e-30));
                    }
                    if dot(dir,g.side)<=0.0 {continue;}
                    let target_pdf=rl_lobe_pdf(g.n,g.v,g.alpha,dir);
                    w=2.0*target_pdf/max(target_pdf+r.w*jacobian,1e-30);
                }
                sum+=w*r.rgb;
                weights+=w;
            }
        }
    }
    if weights>0.0 {rl_specular=vec4<f32>(sum/weights,g.traced);}
}
// ---- end of P-GLOSSY shared strata ----
// ---- end of M7-R1 traced reflections ----
