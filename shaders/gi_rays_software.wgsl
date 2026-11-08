// ---- world radiance cache ray source: software rays (non-RT path) ----
// Per-placement object-space BVH4s in the R0c cluster-BVH layout (docs/m7-r0c.md):
// 20-word nodes (origin, step, four tagged children, axis-major byte planes, errors),
// 8-word leaves, vec4 positions and leaf-local indices, packed by src/gi_bvh.rs into
// one buffer behind a flat instance list (a top level rebuilt on the CPU when
// transforms change). Each ray is its own work item, traversed from the top; no hits
// are registered per node and no rays are launched per node (W8). Both faces block.
@group(0) @binding(35) var<storage,read> gi_geometry:array<u32>;
struct GiHit { hit:bool, t:f32, normal:vec3<f32> };
const GI_BVH_LEAF:u32=0x80000000u;
const GI_BVH_EMPTY:u32=0xffffffffu;
const GI_BVH_STACK:u32=48u;
// Instance records follow a 16-word header; src/gi_bvh.rs INSTANCE_WORDS must match.
const GI_INSTANCE_WORDS:u32=32u;
fn gi_gf(i:u32) -> f32 { return bitcast<f32>(gi_geometry[i]); }
fn gi_gv(i:u32) -> vec3<f32> { return vec3<f32>(gi_gf(i),gi_gf(i+1u),gi_gf(i+2u)); }
fn gi_safe_inverse(d:vec3<f32>) -> vec3<f32> {
    let tiny=select(vec3<f32>(-1e-30),vec3<f32>(1e-30),d>=vec3<f32>(0.0));
    return vec3<f32>(1.0)/select(d,tiny,abs(d)<vec3<f32>(1e-30));
}
fn gi_slab(o:vec3<f32>, inv:vec3<f32>, lo:vec3<f32>, hi:vec3<f32>, tmin:f32, tmax:f32) -> bool {
    return gi_slab_near(o,inv,lo,hi,tmin,tmax)<=tmax;
}
// The box's entry distance in [tmin, tmax], or a value above tmax when the ray misses it.
fn gi_slab_near(o:vec3<f32>, inv:vec3<f32>, lo:vec3<f32>, hi:vec3<f32>, tmin:f32, tmax:f32) -> f32 {
    let a=(lo-o)*inv;let b=(hi-o)*inv;
    let near=max(max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z)),tmin);
    let far=min(min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z)),tmax);
    return select(3.4028235e38,near,near<=far);
}
// Moller-Trumbore, both faces; t in (tmin, tmax).
fn gi_triangle(o:vec3<f32>, d:vec3<f32>, a:vec3<f32>, b:vec3<f32>, c:vec3<f32>, tmin:f32, tmax:f32) -> f32 {
    let e1=b-a;let e2=c-a;
    let p=cross(d,e2);
    let det=dot(e1,p);
    if abs(det)<1e-30 { return -1.0; }
    let inv=1.0/det;
    let s=o-a;
    let u=dot(s,p)*inv;
    if u<0.0 || u>1.0 { return -1.0; }
    let q=cross(s,e1);
    let v=dot(d,q)*inv;
    if v<0.0 || u+v>1.0 { return -1.0; }
    let t=dot(e2,q)*inv;
    return select(-1.0,t,t>tmin && t<tmax);
}
// Closest hit (or, with `any`, the first found) in (tmin, tmax) over every instance.
fn gi_traverse(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32, any_hit:bool) -> GiHit {
    var best=GiHit(false,tmax,vec3<f32>(0.0));
    var stack:array<u32,GI_BVH_STACK>;
    let world_inv=gi_safe_inverse(direction);
    let instances=gi_geometry[0];
    for(var i=0u;i<instances;i+=1u) {
        let r=16u+i*GI_INSTANCE_WORDS;
        if !gi_slab(origin,world_inv,gi_gv(r+12u),gi_gv(r+15u),tmin,best.t) { continue; }
        let r0=vec4<f32>(gi_gv(r),gi_gf(r+3u));let r1=vec4<f32>(gi_gv(r+4u),gi_gf(r+7u));let r2=vec4<f32>(gi_gv(r+8u),gi_gf(r+11u));
        // Unnormalized object-space direction: t is the same parameter in both spaces.
        let o=vec3<f32>(dot(r0,vec4<f32>(origin,1.0)),dot(r1,vec4<f32>(origin,1.0)),dot(r2,vec4<f32>(origin,1.0)));
        let d=vec3<f32>(dot(r0.xyz,direction),dot(r1.xyz,direction),dot(r2.xyz,direction));
        let inv=gi_safe_inverse(d);
        let nodes=gi_geometry[r+18u];let leaves=gi_geometry[r+19u];
        let positions=gi_geometry[r+20u];let indices=gi_geometry[r+21u];
        stack[0]=gi_geometry[r+22u];
        var sp=1u;
        while sp>0u {
            sp-=1u;
            let child=stack[sp];
            if (child&GI_BVH_LEAF)!=0u {
                let l=leaves+(child&~GI_BVH_LEAF)*8u;
                let first=gi_geometry[l+4u];let count=gi_geometry[l+5u];
                for(var k=0u;k<count;k+=3u) {
                    let x=indices+first+k;
                    let a=gi_gv(positions+gi_geometry[x]*4u);
                    let b=gi_gv(positions+gi_geometry[x+1u]*4u);
                    let c=gi_gv(positions+gi_geometry[x+2u]*4u);
                    let t=gi_triangle(o,d,a,b,c,tmin,best.t);
                    if t>=0.0 {
                        // The source-winding normal through the inverse transpose. r0-r2 are
                        // the world-to-object rows, so the matrix with them as columns is it.
                        let n=mat3x3<f32>(r0.xyz,r1.xyz,r2.xyz)*cross(b-a,c-a);
                        best=GiHit(true,t,unit_normal(n));
                        if any_hit { return best; }
                    }
                }
                continue;
            }
            let n=nodes+child*20u;
            let origin_q=gi_gv(n);let step=gi_gv(n+3u);
            // The four children's entry distances (misses above best.t), then pushed far to
            // near so the nearest is visited first: closest-hit rays shrink best.t early and
            // first-hit rays reach a blocker sooner. The order is fixed by the distances.
            var t0=3.4028235e38;var t1=3.4028235e38;var t2=3.4028235e38;var t3=3.4028235e38;
            let c0=gi_geometry[n+6u];let c1=gi_geometry[n+7u];let c2=gi_geometry[n+8u];let c3=gi_geometry[n+9u];
            let lx=gi_geometry[n+10u];let ly=gi_geometry[n+11u];let lz=gi_geometry[n+12u];
            let hx=gi_geometry[n+13u];let hy=gi_geometry[n+14u];let hz=gi_geometry[n+15u];
            for(var s=0u;s<4u;s+=1u) {
                let next=select(select(select(c3,c2,s==2u),c1,s==1u),c0,s==0u);
                if next==GI_BVH_EMPTY { continue; }
                let shift=8u*s;
                let lq=vec3<f32>(vec3<u32>(lx>>shift,ly>>shift,lz>>shift)&vec3<u32>(0xffu));
                let hq=vec3<f32>(vec3<u32>(hx>>shift,hy>>shift,hz>>shift)&vec3<u32>(0xffu));
                let t=gi_slab_near(o,inv,origin_q+step*lq,origin_q+step*hq,tmin,best.t);
                if s==0u { t0=t; } else if s==1u { t1=t; } else if s==2u { t2=t; } else { t3=t; }
            }
            // Four-element sorting network on (distance, child), descending. Distances are
            // nonnegative, so their bits order like the floats; children stay integers.
            var a0=vec2<u32>(bitcast<u32>(t0),c0);var a1=vec2<u32>(bitcast<u32>(t1),c1);
            var a2=vec2<u32>(bitcast<u32>(t2),c2);var a3=vec2<u32>(bitcast<u32>(t3),c3);
            if a0.x<a1.x { let x=a0;a0=a1;a1=x; }
            if a2.x<a3.x { let x=a2;a2=a3;a3=x; }
            if a0.x<a2.x { let x=a0;a0=a2;a2=x; }
            if a1.x<a3.x { let x=a1;a1=a3;a3=x; }
            if a1.x<a2.x { let x=a1;a1=a2;a2=x; }
            for(var s=0u;s<4u;s+=1u) {
                let e=select(select(select(a3,a2,s==2u),a1,s==1u),a0,s==0u);
                if bitcast<f32>(e.x)<=best.t && sp<GI_BVH_STACK {
                    stack[sp]=e.y;
                    sp+=1u;
                }
            }
        }
    }
    return best;
}
fn gi_closest(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32) -> GiHit {
    return gi_traverse(origin,direction,tmin,tmax,false);
}
fn gi_visible(origin:vec3<f32>, direction:vec3<f32>) -> bool {
    return !gi_traverse(origin,direction,0.0,1e20,true).hit;
}
