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
// Software counterpart of primary_trace.wgsl primary_raw_closest: the same pixel-centre
// segment, culling and outputs.
fn primary_raw_closest(pixel:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
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
fn primary_closest(pixel:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    let trace=primary_raw_closest(pixel,size);
    if trace.record.x==0u {return trace;}
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;
    return primary_repair(trace,ray.near,delta/length(delta));
}
// Probe candidates of primary_repair: every triangle of placement `slot` the object-space
// segment [0, reach] crosses (hardware: the unconfirmed non-opaque candidates of that
// instance), in BVH order, at most SOFTWARE_REPAIR_CANDIDATES.
const SOFTWARE_REPAIR_CANDIDATES:u32=16u;
fn software_candidates(slot:u32,origin:vec3<f32>,direction:vec3<f32>,reach:f32,list:ptr<function,array<u32,SOFTWARE_REPAIR_CANDIDATES>>)->u32 {
    let instance=software_instances[slot];
    let m=instance.rows;let p=vec4<f32>(origin,1.0);
    let o=vec3<f32>(dot(m[0],p),dot(m[1],p),dot(m[2],p));
    let d=vec3<f32>(dot(m[0].xyz,direction),dot(m[1].xyz,direction),dot(m[2].xyz,direction));
    let ray=software_ray(o,d);let mesh=instance.info.y;
    var stack:array<u32,SOFTWARE_STACK>;
    var sp=0u;var node=instance.info.x;var count=0u;
    loop {
        for(var c=0u;c<8u;c+=1u) {
            let child=software_nodes[mesh].data[node*8u+c];
            let kind=bitcast<u32>(child.hi.w);
            if kind==0u {break;}
            if software_box(ray,child.lo.xyz,child.hi.xyz,0.0,reach)>reach {continue;}
            let index=bitcast<u32>(child.lo.w);
            if kind==SOFTWARE_INNER {stack[sp]=index;sp+=1u;continue;}
            for(var i=0u;i<kind;i+=1u) {
                let primitive=software_prims[mesh].data[index+i];
                let v=software_positions(mesh,primitive);
                if count<SOFTWARE_REPAIR_CANDIDATES && software_triangle(ray,v[0],v[1],v[2],0.0,reach).x>=0.0 {(*list)[count]=primitive;count+=1u;}
            }
        }
        if sp==0u {return count;}
        sp-=1u;node=stack[sp];
    }
    return count;
}
// primary_trace.wgsl primary_edge_neighbour on the software index binding (the same buffer).
fn software_edge_neighbour(mesh:u32,primitive:u32,edge:u32)->u32 {
    let words=arrayLength(&software_indices[mesh].data);
    if words%3u!=1u {return 0xffffffffu;}
    return software_indices[mesh].data[(words-1u)/2u+primitive*3u+edge];
}
// Software copy of primary_trace.wgsl primary_repair (edge repair, d72b02c): the same
// arithmetic, with the probe's ray query replaced by software_candidates and the vertices
// read through the software bindings. Keep it in step with the production function.
fn primary_repair(trace:PrimaryTrace,origin:vec3<f32>,direction:vec3<f32>)->PrimaryTrace {
    var software_list:array<u32,SOFTWARE_REPAIR_CANDIDATES>;
    var software_count=0u;var software_next=0u;
    let slot=trace.record.x-1u;let vertices=trace.positions;
    let batch=software_instances[slot].info.y;
    let a=world_position(vertices[0],slot).xyz;let b=world_position(vertices[1],slot).xyz;let c=world_position(vertices[2],slot).xyz;
    let epsilon=receiver_epsilon(a,b,c);
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(vertices[1]-vertices[0]);let ac=linear*(vertices[2]-vertices[0]);
    let margin=min(vec3<f32>(0.25),4.0*epsilon*vec3<f32>(length(ac-ab),length(ac),length(ab))/max(length(cross(ab,ac)),1e-30));
    let bary=bitcast<vec2<f32>>(trace.record.zw);let weights=vec3<f32>(1.0-bary.x-bary.y,bary);
    if all(weights>=margin) {return trace;}
    let normal=primary_geometric(vertices[0],vertices[1],vertices[2],slot);
    let score=abs(primary_edge_dot(normal,direction));
    let radius=epsilon/max(score,0.0001);
    var result=trace;var best=-1.0;
    for(var edge=0u;edge<3u;edge+=1u) {
        if weights[edge]>=margin[edge] {continue;}
        var e0=vertices[(edge+1u)%3u];var e1=vertices[(edge+2u)%3u];
        if !primary_edge_order(e0,e1) {let swap=e0;e0=e1;e1=swap;}
        let p0=world_position(e0,slot).xyz;let p1=world_position(e1,slot).xyz;
        let axis=linear*(e1-e0);let centre=(p0+p1)*0.5;
        let local=fma(direction,vec3<f32>(primary_edge_dot(centre-origin,direction)),origin);
        let perpendicular=fma(-direction,vec3<f32>(primary_edge_dot(axis,direction)),axis);
        let denominator=primary_edge_dot(perpendicular,perpendicular);
        if denominator<1e-30 {continue;}
        let t=clamp(-primary_edge_dot(p0-local,perpendicular)/denominator,0.0,1.0);
        let point=fma(axis,vec3<f32>(t),p0);
        let edge_distance=length(point-trace.point);
        if edge_distance>4.0*epsilon {continue;}
        let side=unit_normal(cross(direction,axis));
        let neighbour=software_edge_neighbour(batch,trace.record.y,edge);
        for(var probe=0u;probe<2u;probe+=1u) {
            if probe==0u && weights[edge]>=margin[edge]*0.25 {continue;}
            if probe==1u {
                if best>=0.0 {continue;}
                let own_side=primary_edge_dot(linear*(vertices[edge]-e0),side);
                let viewed=select(normal,-normal,primary_edge_dot(normal,direction)>0.0);
                let projected=fma(epsilon,primary_edge_dot(viewed,side)*sign(own_side),weights[edge]*abs(own_side));
                if projected>=0.0 {continue;}
            }
            let own_side_direction=select(-epsilon,epsilon,primary_edge_dot(linear*(vertices[edge]-e0),side)>=0.0)*select(1.0,-1.0,probe!=0u);
            let start=fma(-direction,vec3<f32>(4.0*radius),fma(side,vec3<f32>(own_side_direction),point));
            let probe_direction=direction;let reach=8.0*radius;

            if neighbour==0xffffffffu {
                software_count=software_candidates(slot,start,probe_direction,reach,&software_list);software_next=0u;
            }
            var direct_pending=true;
            loop {
                var v:array<vec3<f32>,3>;var candidate_primitive=neighbour;
                var visible=(primary_placements[slot]&0x40000000u)==0u;
                if neighbour!=0xffffffffu {
                    if !direct_pending {break;}direct_pending=false;
                    v=software_positions(batch,neighbour);
                    if !primary_probe_intersects(v,slot,start,probe_direction,reach) {continue;}
                    if !visible {visible=primary_edge_dot(primary_geometric(v[0],v[1],v[2],slot),direction)<0.0;}
                } else {
                    if software_next>=software_count {break;}
                    candidate_primitive=software_list[software_next];software_next+=1u;
                    if candidate_primitive==trace.record.y {continue;}
                    v=software_positions(batch,candidate_primitive);
                    let r=software_instances[slot].rows;
                    if !visible {visible=dot(cross(v[1]-v[0],v[2]-v[0]),vec3<f32>(dot(r[0].xyz,direction),dot(r[1].xyz,direction),dot(r[2].xyz,direction)))<0.0;}
                }
                var common_vertices=0u;
                for(var i=0u;i<3u;i+=1u) {common_vertices+=u32(all(v[i]==e0) || all(v[i]==e1));}
                if common_vertices!=2u {continue;}
                if !visible {continue;}
                let n=primary_geometric(v[0],v[1],v[2],slot);
                let candidate_score=abs(primary_edge_dot(n,direction));
                let slack=8.0*1.1920928955078125e-7;
                var other=vec3<f32>(0.0);
                for(var j=0u;j<3u;j+=1u) {if !all(v[j]==e0) && !all(v[j]==e1) {other=linear*(v[j]-e0);}}
                let own=linear*(vertices[edge]-e0);
                let own_side=primary_edge_dot(own,side);let other_side=primary_edge_dot(other,side);
                let depth_axis=unit_normal(cross(axis,side));
                let own_depth=primary_edge_dot(own,depth_axis);let other_depth=primary_edge_dot(other,depth_axis);
                let order=fma(other_depth,abs(own_side),-own_depth*abs(other_side));
                let uncertainty=32.0*1.1920928955078125e-7*(abs(other_depth*own_side)+abs(own_depth*other_side));
                let overlap=own_side*other_side>0.0;
                if probe==1u {
                    let relative_other=other+linear*(e0-vertices[0]);
                    let inset=receiver_fold_weights(ab,ac,weights,edge,relative_other,-direction,epsilon);
                    if any(inset!=weights) && best<0.0 {
                        result=PrimaryTrace(vec4<u32>(trace.record.xy,bitcast<vec2<u32>>(inset.yz)),world_position(receiver_point(vertices[0],vertices[1],vertices[2],inset),slot).xyz,vertices);
                    }
                    continue;
                }
                if !overlap {continue;}
                if edge_distance>2.0*epsilon {continue;}
                let prefer=(overlap && order < -uncertainty) || ((!overlap || abs(order)<=uncertainty) && candidate_primitive<trace.record.y);
                let chosen=select(score,candidate_score,prefer);
                let primitive=select(trace.record.y,candidate_primitive,prefer);
                if prefer && (chosen>best+slack || (abs(chosen-best)<=slack && primitive<result.record.y)) {
                    var positions=vertices;if prefer {positions=v;}
                    let w=primary_edge_weights(positions,e0,e1,t);
                    result=PrimaryTrace(vec4<u32>(slot+1u,primitive,bitcast<vec2<u32>>(w.yz)),point,positions);
                    best=chosen;
                }
            }
        }
    }
    return result;
}
