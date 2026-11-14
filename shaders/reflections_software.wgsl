// ---- M7-R1n software reflection rays (non-RT path), composed by src/program.rs ----
// R1's reflection trace (shaders/reflections.wgsl) with its two ray queries replaced by a
// software traversal of `rl_geometry`, which the non-RT shading head declares at group 0
// binding 36 (src/reflection_software.rs packs it). Per-mesh BVH2s over the drawn triangles
// behind a flat instance list: each ray is its own work item, traversed from the top, and
// writes only its own pixel's value; no cells, hit registration or shared projection (P0's
// W8/C7, docs/m7-p0-claims.md). Both faces block, as the hardware query's opaque rays.
// A hit is the hardware record (placement + 1, primitive, barycentrics), so `rl_hit_input`
// reconstructs and the program shades it exactly as on the RT path.
const RS_LEAF:u32=0x80000000u;
const RS_STACK:u32=32u;
// src/reflection_software.rs HEADER_WORDS and INSTANCE_WORDS must match.
const RS_HEADER_WORDS:u32=8u;
const RS_INSTANCE_WORDS:u32=24u;
// Barycentric slack on every edge test: Moller-Trumbore is not watertight, and rays through a
// shared edge otherwise slip between both triangles (sky seams on the R1n identity quad).
// Closest-hit order resolves the overlap; the hit point moves by at most this fraction.
const RS_EDGE:f32=4e-6;
// P-NRT: every instance record, node and triangle starts on a vec4 word
// (src/reflection_software.rs), so each is read as whole vec4s: the same words and
// arithmetic as single-word reads, in a quarter of the loads.
fn rs_q(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(rl_geometry[i]);}
fn rs_inverse(d:vec3<f32>)->vec3<f32> {
    let tiny=select(vec3<f32>(-1e-30),vec3<f32>(1e-30),d>=vec3<f32>(0.0));
    return vec3<f32>(1.0)/select(d,tiny,abs(d)<vec3<f32>(1e-30));
}
// The box's entry distance clamped to tmin, or a value above tmax when the ray misses it.
fn rs_box(o:vec3<f32>,inv:vec3<f32>,lo:vec3<f32>,hi:vec3<f32>,tmin:f32,tmax:f32)->f32 {
    let a=(lo-o)*inv;let b=(hi-o)*inv;
    let near=max(max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z)),tmin);
    let far=min(min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z)),tmax);
    return select(3.4028235e38,near,near<=far);
}
struct RsHit {t:f32,record:vec4<u32>};
// U-RTVIS: the bound instance i tests hits against: best.t in index order (the flat loop's), one
// ulp above it for an instance below the best hit's, whose equal-distance hit the ascending flat
// loop would have kept (src/instance_bvh.rs).
fn rs_limit(best:RsHit,best_i:u32,i:u32)->f32 {
    return select(best.t,bitcast<f32>(bitcast<u32>(best.t)+1u),best.record.x!=0u && i<best_i);
}
// One instance's part of rs_trace, updating best and best_i. True when an any-hit ray found a
// blocker.
fn rs_instance(i:u32,origin:vec3<f32>,direction:vec3<f32>,world_inv:vec3<f32>,tmin:f32,any_hit:bool,
    best:ptr<function,RsHit>,best_i:ptr<function,u32>,stack:ptr<function,array<u32,RS_STACK>>)->bool {
    // Record words 0-11 rows, 12-17 world lo/hi, 18 nodes, 19 triangles, 20 root, 21 slot,
    // 22 casts no shadow: any-hit (shadow) rays pass through it.
    let r=(RS_HEADER_WORDS+i*RS_INSTANCE_WORDS)>>2u;
    let tail=rl_geometry[r+5u];
    if any_hit && tail.z!=0u {return false;}
    let b0=rs_q(r+3u);let b1=rl_geometry[r+4u];
    let bound=rs_limit(*best,*best_i,i);
    if rs_box(origin,world_inv,b0.xyz,vec3<f32>(b0.w,bitcast<vec2<f32>>(b1.xy)),tmin,bound)>bound {return false;}
    let r0=rs_q(r);let r1=rs_q(r+1u);let r2=rs_q(r+2u);
    // Unnormalized object-space direction: t is the same parameter in both spaces.
    let o=vec3<f32>(dot(r0,vec4<f32>(origin,1.0)),dot(r1,vec4<f32>(origin,1.0)),dot(r2,vec4<f32>(origin,1.0)));
    let d=vec3<f32>(dot(r0.xyz,direction),dot(r1.xyz,direction),dot(r2.xyz,direction));
    let inv=rs_inverse(d);
    let nodes=b1.z>>2u;let triangles=b1.w>>2u;let slot=tail.y;
    (*stack)[0]=tail.x;
    var sp=1u;
    while sp>0u {
        sp-=1u;
        var child=(*stack)[sp];
        loop {
            if (child&RS_LEAF)!=0u {
                let start=(child&~RS_LEAF)>>3u;
                for(var k=0u;k<(child&7u);k+=1u) {
                    // Words 0-2 corner a, 3-5 edge b - a, 6-8 edge c - a, 9 primitive.
                    let x=triangles+(start+k)*3u;
                    let w0=rs_q(x);let w1=rs_q(x+1u);let w2=rl_geometry[x+2u];
                    let e1=vec3<f32>(w0.w,w1.xy);let e2=vec3<f32>(w1.zw,bitcast<f32>(w2.x));
                    let p=cross(d,e2);let det=dot(e1,p);
                    if abs(det)<1e-30 {continue;}
                    let inv_det=1.0/det;let s=o-w0.xyz;
                    let u=dot(s,p)*inv_det;
                    if u< -RS_EDGE || u>1.0+RS_EDGE {continue;}
                    let q=cross(s,e1);let v=dot(d,q)*inv_det;
                    if v< -RS_EDGE || u+v>1.0+RS_EDGE {continue;}
                    let t=dot(e2,q)*inv_det;
                    if t>tmin && t<rs_limit(*best,*best_i,i) {
                        *best=RsHit(t,vec4<u32>(slot+1u,w2.y,bitcast<u32>(u),bitcast<u32>(v)));*best_i=i;
                        if any_hit {return true;}
                    }
                }
                break;
            }
            // Visit the nearer child next and push the farther one.
            // Words 0-5 child 0 lo/hi, 6-11 child 1 lo/hi, 12-13 the tagged children.
            let n=nodes+child*4u;
            let n0=rs_q(n);let n1=rs_q(n+1u);let n2=rs_q(n+2u);let n3=rl_geometry[n+3u];
            let lim=rs_limit(*best,*best_i,i);
            let t0=rs_box(o,inv,n0.xyz,vec3<f32>(n0.w,n1.xy),tmin,lim);
            let t1=rs_box(o,inv,vec3<f32>(n1.zw,n2.x),n2.yzw,tmin,lim);
            let c0=n3.x;let c1=n3.y;
            let h0=t0<=lim;let h1=t1<=lim;
            if h0 && h1 {
                let near_first=t0<=t1;
                if sp<RS_STACK {(*stack)[sp]=select(c0,c1,near_first);sp+=1u;}
                child=select(c1,c0,near_first);
            } else if h0 {
                child=c0;
            } else if h1 {
                child=c1;
            } else {
                break;
            }
        }
    }
    return false;
}
const RS_TOP_STACK:u32=32u;
// The closest hit in (tmin, tmax), or with `any_hit` the first found; record.x = 0 is a miss.
// Through the top-level BVH over the instances' world boxes (src/instance_bvh.rs: header word
// 1 its base, 2 its root, nodes in the BLAS node layout), or the flat list without one.
fn rs_trace(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32,any_hit:bool)->RsHit {
    var best=RsHit(tmax,vec4<u32>(0u));
    var best_i=0xffffffffu;
    var stack:array<u32,RS_STACK>;
    let world_inv=rs_inverse(direction);
    let head=rl_geometry[0u];
    if head.y==0u {
        for(var i=0u;i<head.x;i+=1u) {
            if rs_instance(i,origin,direction,world_inv,tmin,any_hit,&best,&best_i,&stack) {return best;}
        }
        return best;
    }
    var nodes:array<u32,RS_TOP_STACK>;
    nodes[0]=head.z;
    var sp=1u;
    while sp>0u {
        sp-=1u;
        let item=nodes[sp];
        if (item&RS_LEAF)!=0u {
            if rs_instance(item&~RS_LEAF,origin,direction,world_inv,tmin,any_hit,&best,&best_i,&stack) {return best;}
            continue;
        }
        // Both children's boxes against a bound that keeps equal-distance ties (rs_limit).
        let n=(head.y>>2u)+item*4u;
        let n0=rs_q(n);let n1=rs_q(n+1u);let n2=rs_q(n+2u);let n3=rl_geometry[n+3u];
        let bound=select(best.t,bitcast<f32>(bitcast<u32>(best.t)+1u),best.record.x!=0u);
        let t0=rs_box(origin,world_inv,n0.xyz,vec3<f32>(n0.w,n1.xy),tmin,bound);
        let t1=rs_box(origin,world_inv,vec3<f32>(n1.zw,n2.x),n2.yzw,tmin,bound);
        // The nearer child is popped first.
        let near=t0<=t1;
        if select(t0,t1,near)<=bound && sp<RS_TOP_STACK {nodes[sp]=select(n3.x,n3.y,near);sp+=1u;}
        if select(t1,t0,near)<=bound && sp<RS_TOP_STACK {nodes[sp]=select(n3.y,n3.x,near);sp+=1u;}
    }
    return best;
}
// R1's ray-query seams on the software path.
fn rs_closest(origin:vec3<f32>,d:vec3<f32>,tmin:f32,tmax:f32)->vec4<u32> {return rs_trace(origin,d,tmin,tmax,false).record;}
fn rs_blocked(origin:vec3<f32>,d:vec3<f32>,tmin:f32,tmax:f32)->bool {return rs_trace(origin,d,tmin,tmax,true).record.x!=0u;}
// ---- end of M7-R1n software reflection rays ----
