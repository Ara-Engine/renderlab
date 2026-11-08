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
fn rs_f(i:u32)->f32 {return bitcast<f32>(rl_geometry[i]);}
fn rs_v(i:u32)->vec3<f32> {return vec3<f32>(rs_f(i),rs_f(i+1u),rs_f(i+2u));}
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
// The closest hit in (tmin, tmax), or with `any_hit` the first found; record.x = 0 is a miss.
fn rs_trace(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32,any_hit:bool)->RsHit {
    var best=RsHit(tmax,vec4<u32>(0u));
    var stack:array<u32,RS_STACK>;
    let world_inv=rs_inverse(direction);
    let count=rl_geometry[0];
    for(var i=0u;i<count;i+=1u) {
        let r=RS_HEADER_WORDS+i*RS_INSTANCE_WORDS;
        if rs_box(origin,world_inv,rs_v(r+12u),rs_v(r+15u),tmin,best.t)>best.t {continue;}
        let r0=vec4<f32>(rs_v(r),rs_f(r+3u));let r1=vec4<f32>(rs_v(r+4u),rs_f(r+7u));let r2=vec4<f32>(rs_v(r+8u),rs_f(r+11u));
        // Unnormalized object-space direction: t is the same parameter in both spaces.
        let o=vec3<f32>(dot(r0,vec4<f32>(origin,1.0)),dot(r1,vec4<f32>(origin,1.0)),dot(r2,vec4<f32>(origin,1.0)));
        let d=vec3<f32>(dot(r0.xyz,direction),dot(r1.xyz,direction),dot(r2.xyz,direction));
        let inv=rs_inverse(d);
        let nodes=rl_geometry[r+18u];let triangles=rl_geometry[r+19u];let slot=rl_geometry[r+21u];
        stack[0]=rl_geometry[r+20u];
        var sp=1u;
        while sp>0u {
            sp-=1u;
            var child=stack[sp];
            loop {
                if (child&RS_LEAF)!=0u {
                    let start=(child&~RS_LEAF)>>3u;
                    for(var k=0u;k<(child&7u);k+=1u) {
                        let x=triangles+(start+k)*10u;
                        let e1=rs_v(x+3u);let e2=rs_v(x+6u);
                        let p=cross(d,e2);let det=dot(e1,p);
                        if abs(det)<1e-30 {continue;}
                        let inv_det=1.0/det;let s=o-rs_v(x);
                        let u=dot(s,p)*inv_det;
                        if u< -RS_EDGE || u>1.0+RS_EDGE {continue;}
                        let q=cross(s,e1);let v=dot(d,q)*inv_det;
                        if v< -RS_EDGE || u+v>1.0+RS_EDGE {continue;}
                        let t=dot(e2,q)*inv_det;
                        if t>tmin && t<best.t {
                            best=RsHit(t,vec4<u32>(slot+1u,rl_geometry[x+9u],bitcast<u32>(u),bitcast<u32>(v)));
                            if any_hit {return best;}
                        }
                    }
                    break;
                }
                // Visit the nearer child next and push the farther one.
                let n=nodes+child*16u;
                let t0=rs_box(o,inv,rs_v(n),rs_v(n+3u),tmin,best.t);
                let t1=rs_box(o,inv,rs_v(n+6u),rs_v(n+9u),tmin,best.t);
                let c0=rl_geometry[n+12u];let c1=rl_geometry[n+13u];
                let h0=t0<=best.t;let h1=t1<=best.t;
                if h0 && h1 {
                    let near_first=t0<=t1;
                    if sp<RS_STACK {stack[sp]=select(c0,c1,near_first);sp+=1u;}
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
    }
    return best;
}
// R1's ray-query seams on the software path.
fn rs_closest(origin:vec3<f32>,d:vec3<f32>,tmin:f32,tmax:f32)->vec4<u32> {return rs_trace(origin,d,tmin,tmax,false).record;}
fn rs_blocked(origin:vec3<f32>,d:vec3<f32>,tmin:f32,tmax:f32)->bool {return rs_trace(origin,d,tmin,tmax,true).record.x!=0u;}
// ---- end of M7-R1n software reflection rays ----
