// Closest-hit camera rays. Appended after primary_rays.wgsl to the hardware composition
// and, for the Interactive trace, to the soft one; custom data is the placement index.
// Workgroups and PRIMARY_TILE match primary_ray_gpu.rs TRACE_TILE.
// The shared TLAS with hit-triangle positions. Binding 2 stays the shadow rays' view
// (hardware_scene or fresh_scene), so one dispatch can bind both.
@group(0) @binding(19) var primary_scene:acceleration_structure<vertex_return>;
@group(0) @binding(13) var primary_hit_output:texture_storage_2d<rgba32uint,write>;
@group(0) @binding(16) var primary_world_output:texture_storage_2d<rgba32float,write>;
@group(0) @binding(17) var primary_normal_output:texture_storage_2d<rgba32float,write>;
// Every trace tile (PRIMARY_TILE, primary_rays.wgsl) writes its coverage flag; only
// tiles with a camera hit write hit records (and receivers or counts), so shading and
// fresh visibility read the flag first. Pixels outside primary_frame.scene are misses
// without a ray.
@group(0) @binding(20) var primary_tile_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(23) var primary_epsilon_output:texture_storage_2d<r32float,write>;
var<workgroup> primary_tile_hit:atomic<u32>;

// Object-space positions are the hit triangle's own, in index order.
struct PrimaryTrace {record:vec4<u32>,point:vec3<f32>,positions:array<vec3<f32>,3>};
struct PrimaryVertices {data:array<vec4<f32>>};
struct PrimaryIndices {data:array<u32>};
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<PrimaryVertices,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<PrimaryIndices,PRIMARY_BATCHES>;
fn primary_triangle_vertices(batch:u32,primitive:u32)->array<vec3<f32>,3> {
    let base=primitive*3u;
    return array<vec3<f32>,3>(primary_vertices[batch].data[primary_indices[batch].data[base]*3u].xyz,
        primary_vertices[batch].data[primary_indices[batch].data[base+1u]*3u].xyz,
        primary_vertices[batch].data[primary_indices[batch].data[base+2u]*3u].xyz);
}
// Original triangle indices, then one neighbour per opposite-vertex edge, then
// padding: 6*T+1 words. Original-only buffers have 3*T words and use the probe.
fn primary_edge_neighbour(batch:u32,primitive:u32,edge:u32)->u32 {
    let words=arrayLength(&primary_indices[batch].data);
    if words%3u!=1u {return 0xffffffffu;}
    return primary_indices[batch].data[(words-1u)/2u+primitive*3u+edge];
}
// renderlab: fold inset begin
// A neighbouring facet must lie above the viewed-side plane before an inset
// is allowed. Convex folds, flat triangulation edges and isolated silhouettes
// retain their hit point. The move is at most four normal-clearance epsilons.
fn receiver_fold_weights(ab:vec3<f32>,ac:vec3<f32>,w:vec3<f32>,edge:u32,other:vec3<f32>,toward:vec3<f32>,epsilon:f32)->vec3<f32> {
    let area_vector=cross(ab,ac);let n=unit_normal(area_vector);
    let viewed=select(-n,n,dot(n,toward)>=0.0);
    let distance=fma(viewed.x,other.x,fma(viewed.y,other.y,viewed.z*other.z));
    let error=32.0*1.1920928955078125e-7*dot(abs(viewed),abs(other));
    if distance<=error {return w;}
    var a=ab;var b=ac;var opposite=vec3<f32>(0.0);
    if edge==1u {a=ac;b=vec3<f32>(0.0);opposite=ab;}
    if edge==2u {a=vec3<f32>(0.0);b=ab;opposite=ac;}
    let neighbour=unit_normal(cross(b-a,other-a));
    let outside=select(-neighbour,neighbour,dot(neighbour,toward)>=0.0);
    let point=fma(ac,vec3<f32>(w.z),ab*w.y);
    let clearance=fma(epsilon,dot(outside,viewed),dot(outside,point-a));
    let clearance_error=32.0*1.1920928955078125e-7*(epsilon+dot(abs(outside),abs(point-a)));
    if clearance>=-clearance_error {return w;}
    let altitude=length(area_vector)/max(length(b-a),1e-30);
    let margin=min(0.25,4.0*epsilon/max(altitude,1e-30));
    if w[edge]>=margin {return w;}
    let room=1.0-margin;
    let start=fma(opposite,vec3<f32>(margin),a*room);let axis=(b-a)*room;
    let t=clamp(dot(point-start,axis)/max(dot(axis,axis),1e-30),0.0,1.0);
    var nearest=vec3<f32>(0.0);nearest[edge]=margin;
    nearest[(edge+1u)%3u]=room*(1.0-t);nearest[(edge+2u)%3u]=room*t;
    let offset_delta=fma(axis,vec3<f32>(t),start)-point;
    return fma(nearest-w,vec3<f32>(min(1.0,4.0*epsilon/max(length(offset_delta),1e-30))),w);
}
// renderlab: fold inset end

// Traversal can return the farther facet where two projected triangles overlap
// at a fold. Query only the receiver's error-width edge strip, rebasing beside
// the shared edge. Canonical endpoint order and local facet slopes make the
// ambiguous choice independent of the triangle traversal happened to return.
// These probes never become shadow origins; the receiver displacement is bounded.
fn primary_edge_order(a:vec3<f32>,b:vec3<f32>)->bool {
    return a.x<b.x || (a.x==b.x && (a.y<b.y || (a.y==b.y && a.z<b.z)));
}
fn primary_edge_dot(a:vec3<f32>,b:vec3<f32>)->f32 {return fma(a.x,b.x,fma(a.y,b.y,a.z*b.z));}
// The retained camera-relative probe, intersected analytically when topology is
// unambiguous. Its f32 origin is the same local, epsilon-separated search point.
fn primary_probe_intersects(v:array<vec3<f32>,3>,slot:u32,start:vec3<f32>,direction:vec3<f32>,reach:f32)->bool {
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(v[1]-v[0]);let ac=linear*(v[2]-v[0]);
    let h=cross(direction,ac);let determinant=primary_edge_dot(ab,h);
    if determinant==0.0 {return false;}
    let s=start-world_position(v[0],slot).xyz;
    let u=primary_edge_dot(s,h)/determinant;let q=cross(s,ab);
    let w=primary_edge_dot(direction,q)/determinant;
    let t=primary_edge_dot(ac,q)/determinant;
    return u>=0.0 && w>=0.0 && u+w<=1.0 && t>=0.0 && t<=reach;
}
fn primary_edge_weights(v:array<vec3<f32>,3>,a:vec3<f32>,b:vec3<f32>,t:f32)->vec3<f32> {
    var w=vec3<f32>(0.0);
    for(var i=0u;i<3u;i+=1u) {
        if all(v[i]==a) {w[i]=1.0-t;}
        if all(v[i]==b) {w[i]=t;}
    }
    return w;
}
// The full near-to-far segment with forced opaque geometry and no first-hit flag.
fn primary_raw_closest(pixel:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;let distance=length(delta);
    let direction=delta/distance;
    var query:ray_query<vertex_return>;
    // Single-sided draws hide source-back faces as the raster discard does. Facing is
    // the object-space winding normal against the object-space ray, so mirrored
    // instances keep their source sides. Only scenes with such draws pay for it.
    let flags=select(RAY_FLAG_FORCE_OPAQUE,RAY_FLAG_FORCE_NO_OPAQUE,primary_frame.sided.x!=0u);
    rayQueryInitialize(&query,primary_scene,RayDesc(flags,255u,0.0,distance,ray.near,direction));
    while rayQueryProceed(&query) {
        let candidate=rayQueryGetCandidateIntersection(&query);
        var visible=(primary_placements[candidate.instance_custom_data]&0x40000000u)==0u;
        if !visible {
            let v=getCandidateHitVertexPositions(&query);
            visible=dot(cross(v[1]-v[0],v[2]-v[0]),candidate.world_to_object*vec4<f32>(direction,0.0))<0.0;
        }
        if visible {rayQueryConfirmIntersection(&query);}
    }
    let hit=rayQueryGetCommittedIntersection(&query);
    if hit.kind==0u {return PrimaryTrace(vec4<u32>(0u),vec3<f32>(0.0),array<vec3<f32>,3>());}
    let v=getCommittedHitVertexPositions(&query);
    let w=vec3<f32>(1.0-hit.barycentrics.x-hit.barycentrics.y,hit.barycentrics);
    let point=world_position(receiver_point(v[0],v[1],v[2],w),hit.instance_custom_data).xyz;
    return PrimaryTrace(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),point,v);
}
fn primary_closest(pixel:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    let trace=primary_raw_closest(pixel,size);
    if trace.record.x==0u {return trace;}
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;
    return primary_repair(trace,ray.near,delta/length(delta));
}
fn primary_repair(trace:PrimaryTrace,origin:vec3<f32>,direction:vec3<f32>)->PrimaryTrace {
    var query:ray_query<vertex_return>;
    let slot=trace.record.x-1u;let vertices=trace.positions;
    let batch=primary_placements[slot]&0x1fffffffu;
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
        // Rebase the ray close to the common edge before solving for its closest
        // point. The same canonical edge gives the same point from either face.
        let local=fma(direction,vec3<f32>(primary_edge_dot(centre-origin,direction)),origin);
        let perpendicular=fma(-direction,vec3<f32>(primary_edge_dot(axis,direction)),axis);
        let denominator=primary_edge_dot(perpendicular,perpendicular);
        if denominator<1e-30 {continue;}
        let t=clamp(-primary_edge_dot(p0-local,perpendicular)/denominator,0.0,1.0);
        let point=fma(axis,vec3<f32>(t),p0);
        let edge_distance=length(point-trace.point);
        if edge_distance>4.0*epsilon {continue;}
        let side=unit_normal(cross(direction,axis));
        let neighbour=primary_edge_neighbour(batch,trace.record.y,edge);
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
                rayQueryInitialize(&query,primary_scene,RayDesc(RAY_FLAG_FORCE_NO_OPAQUE,255u,0.0,reach,start,probe_direction));
            }
            var direct_pending=true;
            loop {
                var v:array<vec3<f32>,3>;var candidate_primitive=neighbour;
                var visible=(primary_placements[slot]&0x40000000u)==0u;
                if neighbour!=0xffffffffu {
                    if !direct_pending {break;}direct_pending=false;
                    v=primary_triangle_vertices(batch,neighbour);
                    if !primary_probe_intersects(v,slot,start,probe_direction,reach) {continue;}
                    if !visible {visible=primary_edge_dot(primary_geometric(v[0],v[1],v[2],slot),direction)<0.0;}
                } else {
                    if !rayQueryProceed(&query) {break;}
                    let candidate=rayQueryGetCandidateIntersection(&query);
                    if candidate.instance_custom_data!=slot || candidate.primitive_index==trace.record.y {continue;}
                    candidate_primitive=candidate.primitive_index;
                    v=getCandidateHitVertexPositions(&query);
                    if !visible {visible=dot(cross(v[1]-v[0],v[2]-v[0]),candidate.world_to_object*vec4<f32>(direction,0.0))<0.0;}
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
// The pixel's camera ray, or a miss without a ray outside the image or primary_frame.scene.
fn primary_scene_trace(id:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    var trace:PrimaryTrace;
    let scene=primary_frame.scene;
    if all(id<size) && all(id>=scene.xy) && all(id<scene.zw) {trace=primary_closest(id,size);}
    return trace;
}
// ORs the workgroup's hits and publishes the tile's flag. Uniform control flow only.
fn primary_tile_covered(hit:bool,id:vec2<u32>,group:vec2<u32>)->bool {
    if hit {atomicOr(&primary_tile_hit,1u);}
    workgroupBarrier();
    let covered=atomicLoad(&primary_tile_hit);
    // A tile's first pixel is always inside the image.
    if all(id==group*PRIMARY_TILE) {textureStore(primary_tile_output,group,vec4<u32>(covered));}
    return covered!=0u;
}
// Hard and unshadowed frames: hit record and geometric receiver precision.
// Compute the edge correction here, outside programmable hard shading's dataflow.
@compute @workgroup_size(8,4) fn primary_trace(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    let trace=primary_scene_trace(id.xy,size);let record=trace.record;
    if !primary_tile_covered(record.x!=0u,id.xy,group.xy) || any(id.xy>=size) {return;}
    textureStore(primary_hit_output,vec2<i32>(id.xy),record);
    if record.x!=0u {textureStore(primary_epsilon_output,vec2<i32>(id.xy),vec4<f32>(primary_epsilon(trace.positions,record.x-1u)));}
}
// Exact soft frames also publish exact fresh receivers at hits: the triangle point, epsilon
// and the hit triangle's plane normal from its returned positions (no vertex gather).
// Misses leave both receivers stale; traced fresh visibility reads coverage from the
// tile flag and hit record instead (primary_ray_gpu.rs COVERAGE_SEAM).
@compute @workgroup_size(8,4) fn primary_trace_receivers(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    let trace=primary_scene_trace(id.xy,size);
    if !primary_tile_covered(trace.record.x!=0u,id.xy,group.xy) || any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    textureStore(primary_hit_output,p,trace.record);
    if trace.record.x==0u {return;}
    let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
    textureStore(primary_world_output,p,vec4<f32>(receiver.world,receiver.epsilon));
    let normal=receiver.geometric;
    textureStore(primary_normal_output,p,vec4<f32>(normal,select(1.0,-1.0,receiver.thin)));
}
