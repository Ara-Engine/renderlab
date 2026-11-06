// Software BVH8 traversal: the non-RT ray backend probe (docs/non-rt-path.md, N0).
// Test-only. src/software_bvh.rs builds the nodes, keeps a CPU reference with the same
// traversal and triangle test, and appends this file (after SOFTWARE_MESHES,
// SOFTWARE_INSTANCES and SOFTWARE_STACK) to the traced soft module with its ray queries
// removed, or alone to fixture ray kernels.

// One BVH8 child: lo.w holds its index bits (a node, or a leaf's first software_prims
// entry), hi.w its kind (0 empty, SOFTWARE_INNER, else the leaf's triangle count). Node n
// is children 8n..8n+7, packed first. Boxes are padded at build time.
struct SoftwareChild {lo:vec4<f32>,hi:vec4<f32>};
struct SoftwareChildren {data:array<SoftwareChild>};
// rows: world-to-object 3x4 rows. lo, hi: padded world bounds. info: root node, mesh slot,
// single-sided flag.
struct SoftwareInstance {rows:array<vec4<f32>,3>,lo:vec4<f32>,hi:vec4<f32>,info:vec4<u32>};
struct SoftwareVec4s {data:array<vec4<f32>>};
struct SoftwareU32s {data:array<u32>};
// Per mesh, with mesh-local indices: nodes, and triangle indices in leaf order.
@group(2) @binding(40) var<storage,read> software_nodes:binding_array<SoftwareChildren,SOFTWARE_MESHES>;
@group(2) @binding(41) var<storage,read> software_prims:binding_array<SoftwareU32s,SOFTWARE_MESHES>;
// One per placement, in placement order.
@group(2) @binding(42) var<storage,read> software_instances:array<SoftwareInstance>;
// The meshes' own vertex (three vec4 per vertex, position first) and index buffers.
@group(2) @binding(43) var<storage,read> software_vertices:binding_array<SoftwareVec4s,SOFTWARE_MESHES>;
@group(2) @binding(44) var<storage,read> software_indices:binding_array<SoftwareU32s,SOFTWARE_MESHES>;

const SOFTWARE_INNER:u32=0xffffffffu;
const SOFTWARE_NONE:u32=0xffffffffu;
// Closest hit; closest hit culling single-sided back faces (camera rays); first hit.
const SOFTWARE_CLOSEST:u32=0u;
const SOFTWARE_CULL:u32=1u;
const SOFTWARE_ANY:u32=2u;
// Relative widening of slab distances: rounding in the slab arithmetic never culls a box
// the ray reaches (robust traversal, Ize 2013). software_bvh.rs WIDEN must match.
const SOFTWARE_WIDEN:f32=1e-6;

// The per-ray constants of the watertight test (Woop, Benthin and Wald 2013).
struct SoftwareRay {origin:vec3<f32>,direction:vec3<f32>,inverse:vec3<f32>,shear:vec3<f32>,axes:vec3<u32>};
// t bounds the search (tmax until a hit); placement and primitive are SOFTWARE_NONE on a
// miss. bary: weights of the hit triangle's second and third vertices, as hardware reports.
// First-hit queries report the placement only.
struct SoftwareHit {t:f32,placement:u32,primitive:u32,bary:vec2<f32>};

fn software_ray(origin:vec3<f32>,direction:vec3<f32>)->SoftwareRay {
    let a=abs(direction);
    var z=2u;
    if a.x>=a.y && a.x>=a.z {z=0u;} else if a.y>=a.z {z=1u;}
    var x=(z+1u)%3u;var y=(z+2u)%3u;
    // Swapping keeps the winding, so edge function signs mean the same on both faces.
    if direction[z]<0.0 {let s=x;x=y;y=s;}
    let dz=direction[z];
    let tiny=select(vec3<f32>(-1e-30),vec3<f32>(1e-30),direction>=vec3<f32>(0.0));
    let safe=select(direction,tiny,a<vec3<f32>(1e-30));
    return SoftwareRay(origin,direction,vec3<f32>(1.0)/safe,vec3<f32>(direction[x]/dz,direction[y]/dz,1.0/dz),vec3<u32>(x,y,z));
}
// Widened entry distance of a box within [tmin, tmax], or infinity when it is missed.
fn software_box(ray:SoftwareRay,lo:vec3<f32>,hi:vec3<f32>,tmin:f32,tmax:f32)->f32 {
    let a=(lo-ray.origin)*ray.inverse;let b=(hi-ray.origin)*ray.inverse;
    let n=min(a,b);let f=max(a,b);
    let near=max(max(n.x,n.y),n.z);let far=min(min(f.x,f.y),f.z);
    let entry=max(near-abs(near)*SOFTWARE_WIDEN,tmin);
    let exit=min(far+abs(far)*SOFTWARE_WIDEN,tmax);
    return select(bitcast<f32>(0x7f800000u),entry,entry<=exit);
}
// a*b-c*d with an exact sign (Kahan, with fma): neighbours sharing an edge get edge
// functions of opposite sign or both zero, so no ray passes between them.
fn software_difference(a:f32,b:f32,c:f32,d:f32)->f32 {
    let w=c*d;
    return fma(a,b,-w)+fma(-c,d,w);
}
// Watertight ray/triangle test in f32: (t, b1, b2) for a hit within [tmin, tmax], else t=-1.
fn software_triangle(ray:SoftwareRay,p0:vec3<f32>,p1:vec3<f32>,p2:vec3<f32>,tmin:f32,tmax:f32)->vec3<f32> {
    let k=ray.axes;let s=ray.shear;
    let a=p0-ray.origin;let b=p1-ray.origin;let c=p2-ray.origin;
    let ax=fma(-s.x,a[k.z],a[k.x]);let ay=fma(-s.y,a[k.z],a[k.y]);
    let bx=fma(-s.x,b[k.z],b[k.x]);let by=fma(-s.y,b[k.z],b[k.y]);
    let cx=fma(-s.x,c[k.z],c[k.x]);let cy=fma(-s.y,c[k.z],c[k.y]);
    let u=software_difference(cx,by,cy,bx);
    let v=software_difference(ax,cy,ay,cx);
    let w=software_difference(bx,ay,by,ax);
    let miss=vec3<f32>(-1.0,0.0,0.0);
    if (u<0.0 || v<0.0 || w<0.0) && (u>0.0 || v>0.0 || w>0.0) {return miss;}
    let det=u+v+w;
    if det==0.0 {return miss;}
    let t=fma(u,s.z*a[k.z],fma(v,s.z*b[k.z],w*(s.z*c[k.z])))/det;
    if !(t>=tmin && t<=tmax) {return miss;}
    return vec3<f32>(t,v/det,w/det);
}
// Object-space positions of a mesh triangle, in index order.
fn software_positions(mesh:u32,primitive:u32)->array<vec3<f32>,3> {
    let i=3u*primitive;
    let a=software_indices[mesh].data[i];let b=software_indices[mesh].data[i+1u];
    let c=software_indices[mesh].data[i+2u];
    return array<vec3<f32>,3>(software_vertices[mesh].data[3u*a].xyz,
        software_vertices[mesh].data[3u*b].xyz,software_vertices[mesh].data[3u*c].xyz);
}
// Closest hit in one mesh's BVH8 with an object-space ray, updating `hit`. Children are
// visited in slot order and leaves tested on sight; internal children are pushed nearest on
// top, popped entries beyond the current hit are skipped, and ties go to the lower
// placement, then primitive.
fn software_blas(ray:SoftwareRay,root:u32,mesh:u32,placement:u32,cull:bool,tmin:f32,hit:ptr<function,SoftwareHit>) {
    var stack:array<u32,SOFTWARE_STACK>;
    var entry:array<f32,SOFTWARE_STACK>;
    var sp=0u;
    var node=root;
    loop {
        let base=sp;
        for(var c=0u;c<8u;c+=1u) {
            let child=software_nodes[mesh].data[node*8u+c];
            let kind=bitcast<u32>(child.hi.w);
            if kind==0u {break;}
            let near=software_box(ray,child.lo.xyz,child.hi.xyz,tmin,(*hit).t);
            if near>(*hit).t {continue;}
            let index=bitcast<u32>(child.lo.w);
            if kind==SOFTWARE_INNER {
                var j=sp;
                while j>base && entry[j-1u]<near {stack[j]=stack[j-1u];entry[j]=entry[j-1u];j-=1u;}
                stack[j]=index;entry[j]=near;sp+=1u;
                continue;
            }
            if SOFTWARE_DIAG_NO_TRIANGLES {continue;}
            for(var i=0u;i<kind;i+=1u) {
                let primitive=software_prims[mesh].data[index+i];
                let v=software_positions(mesh,primitive);
                let r=software_triangle(ray,v[0],v[1],v[2],tmin,(*hit).t);
                if r.x<0.0 {continue;}
                // Source-back faces of single-sided draws, as primary_trace.wgsl culls them.
                if cull && dot(cross(v[1]-v[0],v[2]-v[0]),ray.direction)>=0.0 {continue;}
                if r.x<(*hit).t || placement<(*hit).placement || (placement==(*hit).placement && primitive<(*hit).primitive) {
                    (*hit)=SoftwareHit(r.x,placement,primitive,r.yz);
                }
            }
        }
        loop {
            if sp==0u {return;}
            sp-=1u;
            if entry[sp]<=(*hit).t {node=stack[sp];break;}
        }
    }
}
// Diagnostic switches of the N0 limiter screen; they change results. No triangle tests, and
// shadow rays capped at tmin + SOFTWARE_DIAG_SHORT when it is positive.
override SOFTWARE_DIAG_NO_TRIANGLES:bool=false;
override SOFTWARE_DIAG_SHORT:f32=0.0;
// First hit in one mesh's BVH8: children in slot order with no entry distances (the
// retained hillclimb step: 0.85x of the shared closest-hit loop, same answers).
// True at the first hit within [tmin, tmax]. Returning the hit primitive out of these loops
// instead made NVIDIA 617.14's shadow kernel 4.4x slower (40-44 against 9.1 ms, same answers).
fn software_blas_any(ray:SoftwareRay,root:u32,mesh:u32,tmin:f32,tmax:f32)->bool {
    var stack:array<u32,SOFTWARE_STACK>;
    var sp=0u;
    var node=root;
    loop {
        for(var c=0u;c<8u;c+=1u) {
            let child=software_nodes[mesh].data[node*8u+c];
            let kind=bitcast<u32>(child.hi.w);
            if kind==0u {break;}
            if software_box(ray,child.lo.xyz,child.hi.xyz,tmin,tmax)>tmax {continue;}
            let index=bitcast<u32>(child.lo.w);
            if kind==SOFTWARE_INNER {stack[sp]=index;sp+=1u;continue;}
            if SOFTWARE_DIAG_NO_TRIANGLES {continue;}
            for(var i=0u;i<kind;i+=1u) {
                let v=software_positions(mesh,software_prims[mesh].data[index+i]);
                if software_triangle(ray,v[0],v[1],v[2],tmin,tmax).x>=0.0 {return true;}
            }
        }
        if sp==0u {return false;}
        sp-=1u;
        node=stack[sp];
    }
    return false;
}
// Top level: a brute-force loop over the placements' world boxes; hit rays are moved to
// object space, as hardware instances do.
fn software_trace(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32,mode:u32)->SoftwareHit {
    var hit=SoftwareHit(tmax,SOFTWARE_NONE,SOFTWARE_NONE,vec2<f32>(0.0));
    let world=software_ray(origin,direction);
    for(var i=0u;i<SOFTWARE_INSTANCES;i+=1u) {
        let instance=software_instances[i];
        if software_box(world,instance.lo.xyz,instance.hi.xyz,tmin,hit.t)>hit.t {continue;}
        let m=instance.rows;let p=vec4<f32>(origin,1.0);
        let o=vec3<f32>(dot(m[0],p),dot(m[1],p),dot(m[2],p));
        let d=vec3<f32>(dot(m[0].xyz,direction),dot(m[1].xyz,direction),dot(m[2].xyz,direction));
        let cull=mode==SOFTWARE_CULL && instance.info.z!=0u;
        if mode==SOFTWARE_ANY {
            if software_blas_any(software_ray(o,d),instance.info.x,instance.info.y,tmin,tmax) {
                return SoftwareHit(tmin,i,0u,vec2<f32>(0.0));
            }
            continue;
        }
        software_blas(software_ray(o,d),instance.info.x,instance.info.y,i,cull,tmin,&hit);
    }
    return hit;
}
// Opaque any-hit with first-hit termination: the shadow rays' query.
fn software_occluded(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32)->bool {
    let end=select(tmax,min(tmax,tmin+SOFTWARE_DIAG_SHORT),SOFTWARE_DIAG_SHORT>0.0);
    return software_trace(origin,direction,tmin,end,SOFTWARE_ANY).placement!=SOFTWARE_NONE;
}

// ---- composed traced modules only ----
// Software counterpart of primary_trace.wgsl primary_closest: the same pixel-centre
// segment, culling and outputs.
fn primary_closest(pixel:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;let distance=length(delta);
    let direction=delta/distance;
    let mode=select(SOFTWARE_CLOSEST,SOFTWARE_CULL,primary_frame.sided.x!=0u);
    let hit=software_trace(ray.near,direction,0.0,distance,mode);
    if hit.placement==SOFTWARE_NONE {return PrimaryTrace(vec4<u32>(0u),vec3<f32>(0.0),array<vec3<f32>,3>());}
    let v=software_positions(software_instances[hit.placement].info.y,hit.primitive);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    // Reconstruct on the hit triangle rather than the rounded primary-ray segment.
    let point=world_position(receiver_point(v[0],v[1],v[2],w),hit.placement).xyz;
    return PrimaryTrace(vec4<u32>(hit.placement+1u,hit.primitive,bitcast<vec2<u32>>(hit.bary)),point,v);
}
