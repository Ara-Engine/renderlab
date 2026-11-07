// Experimental RT1b: geometry-only footprint receivers. No material/program ABI
// changes. The centre hit remains the shading hit. MODE: 0 centre, 1 clipped
// centre triangle, 2 extended plane, 3 connected surface, 4 closest primary.
// Both ray budgets use the same origin distribution, with 8 or 256 samples.
override RT1B_MODE:u32=0u;
override RT1B_STRATIFIED:bool=false;
override RT1B_SUNFLOWER:bool=false;
var<private> rt1b_record:vec4<u32>;
var<private> rt1b_dx:vec3<f32>;
var<private> rt1b_dy:vec3<f32>;
var<private> rt1b_bx:vec2<f32>;
var<private> rt1b_by:vec2<f32>;
var<private> rt1b_view:vec3<f32>;
var<private> rt1b_vx:vec3<f32>;
var<private> rt1b_vy:vec3<f32>;
var<private> rt1b_sample_valid:bool;

// Diagnostic cost/quality control: reproduce the film reference's actual primary
// selection instead of pretending an extended or adjacent facet is visible. This
// adds a camera query per sample; it is deliberately not a production shortcut.
fn rt1b_closest(pixel:vec2<u32>,sample:vec2<f32>)->PrimaryTrace {
    let size=textureDimensions(primary_hit_output);
    let ndc=fma((vec2<f32>(pixel)+vec2<f32>(0.5)+sample)/vec2<f32>(size),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
    let inverse=primary_frame.inverse;
    let xy=fma(inverse[0],vec4<f32>(ndc.x),inverse[1]*ndc.y);
    let a=xy+inverse[2]+inverse[3];let b=xy+inverse[3];
    let near=a.xyz/a.w;let far=b.xyz/b.w;
    let delta=far-near;let distance=length(delta);let direction=delta/distance;
    var query:ray_query<vertex_return>;
    let flags=select(RAY_FLAG_FORCE_OPAQUE,RAY_FLAG_FORCE_NO_OPAQUE,primary_frame.sided.x!=0u);
    rayQueryInitialize(&query,primary_scene,RayDesc(flags,255u,0.0,distance,near,direction));
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
    return primary_repair(PrimaryTrace(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),point,v),near,direction);
}

fn rt1b_prepare(trace:PrimaryTrace,receiver:Receiver,pixel:vec2<u32>,width:u32) {
    if RT1B_MODE==0u {return;}
    rt1b_record=trace.record;
    if RT1B_MODE==4u {return;}
    let ray=primary_ray(pixel,textureDimensions(primary_hit_output));
    let direction=ray.far-ray.near;let denominator=dot(receiver.geometric,direction);
    rt1b_dx=vec3<f32>(0.0);rt1b_dy=vec3<f32>(0.0);
    rt1b_bx=vec2<f32>(0.0);rt1b_by=vec2<f32>(0.0);
    if abs(denominator)<1e-20 {return;}
    let t=dot(receiver.geometric,receiver.world-ray.near)/denominator;
    let qx=fma(vec3<f32>(t),ray.dx_far-ray.dx_near,ray.dx_near);
    let qy=fma(vec3<f32>(t),ray.dy_far-ray.dy_near,ray.dy_near);
    rt1b_dx=qx-direction*(dot(receiver.geometric,qx)/denominator);
    rt1b_dy=qy-direction*(dot(receiver.geometric,qy)/denominator);
    rt1b_view=normalize(direction);
    let vx=ray.dx_far-ray.dx_near;let vy=ray.dy_far-ray.dy_near;
    rt1b_vx=(vx-rt1b_view*dot(rt1b_view,vx))/length(direction);
    rt1b_vy=(vy-rt1b_view*dot(rt1b_view,vy))/length(direction);
    let m=models[trace.record.x-1u];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(trace.positions[1]-trace.positions[0]);let ac=linear*(trace.positions[2]-trace.positions[0]);
    let scale=max(max(max(abs(ab.x),abs(ab.y)),abs(ab.z)),max(max(abs(ac.x),abs(ac.y)),abs(ac.z)));
    let a=ab/max(scale,1e-30);let b=ac/max(scale,1e-30);
    let area=dot(cross(a,b),receiver.geometric);
    if abs(area)<1e-20 {return;}
    let u=cross(b,receiver.geometric)/area;let v=cross(receiver.geometric,a)/area;
    rt1b_bx=vec2<f32>(dot(u,rt1b_dx/scale),dot(v,rt1b_dx/scale));
    rt1b_by=vec2<f32>(dot(u,rt1b_dy/scale),dot(v,rt1b_dy/scale));
}
fn rt1b_position(index:u32,pixel:vec2<u32>,width:u32)->vec2<f32> {
    let seed=fresh_hash(pixel.x+pixel.y*width)^0x68bc21ebu;
    let shift=vec2<f32>(f32(seed>>8u),f32(fresh_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    var grid=vec2<u32>(16u,16u);
    if FRESH_DIRECTIONS==8u {grid=vec2<u32>(4u,2u);}
    let i=(index*5u)%FRESH_DIRECTIONS;
    return fract((vec2<f32>(f32(i%grid.x),f32(i/grid.x))+vec2<f32>(0.5))/vec2<f32>(grid)+shift)-vec2<f32>(0.5);
}
fn rt1b_sample(receiver:Receiver,index:u32,pixel:vec2<u32>,width:u32)->Receiver {
    rt1b_sample_valid=true;
    if RT1B_MODE==0u {return receiver;}
    let sample=rt1b_position(index,pixel,width);
    if RT1B_MODE==4u {
        let trace=rt1b_closest(pixel,sample);
        // The existing RF visibility reference conditions on centre placement and
        // two-percent camera depth. Do not count rejected samples as visible.
        let depth=dot(shadow_views[lights[0].kind.y].camera_depth,vec4<f32>(receiver.world,1.0));
        let sample_depth=dot(shadow_views[lights[0].kind.y].camera_depth,vec4<f32>(trace.point,1.0));
        if trace.record.x!=rt1b_record.x || abs(sample_depth-depth)>0.02*depth {
            rt1b_sample_valid=false;return receiver;
        }
        return primary_receiver(trace.point,trace.positions,trace.record.x-1u,receiver.pixel);
    }
    let uv=fma(rt1b_bx,vec2<f32>(sample.x),rt1b_by*sample.y);
    let bary=bitcast<vec2<f32>>(rt1b_record.zw);
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let delta=vec3<f32>(-uv.x-uv.y,uv);
    var factor=1.0;
    if RT1B_MODE==1u {
        for(var k=0u;k<3u;k+=1u) {if delta[k]<0.0 {factor=min(factor,w[k]/(-delta[k]));}}
    }
    var result=receiver;
    result.world=fma(rt1b_dx,vec3<f32>(sample.x*factor),fma(rt1b_dy,vec3<f32>(sample.y*factor),receiver.world));
    if RT1B_MODE!=3u || all(w+delta>=vec3<f32>(0.0)) {return result;}
    let base=result.world;
    let direction=normalize(fma(rt1b_vx,vec3<f32>(sample.x),fma(rt1b_vy,vec3<f32>(sample.y),rt1b_view)));
    let slot=rt1b_record.x-1u;let batch=primary_placements[slot]&0x1fffffffu;
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    var primitive=rt1b_record.y;var weights=w+delta;
    // Bounded experiment: unresolved/open topology retains the baseline receiver,
    // never turns an unknown surface into a clear shadow ray.
    for(var step=0u;step<16u;step+=1u) {
        var edge=0u;if weights.y<weights[edge] {edge=1u;}if weights.z<weights[edge] {edge=2u;}
        primitive=primary_edge_neighbour(batch,primitive,edge);
        if primitive==0xffffffffu {return receiver;}
        let vertices=primary_triangle_vertices(batch,primitive);
        let a=world_position(vertices[0],slot).xyz;
        let ab=linear*(vertices[1]-vertices[0]);let ac=linear*(vertices[2]-vertices[0]);
        let h=cross(direction,ac);let determinant=primary_edge_dot(ab,h);
        if determinant==0.0 {return receiver;}
        let s=base-a;let u=primary_edge_dot(s,h)/determinant;let q=cross(s,ab);
        let v=primary_edge_dot(direction,q)/determinant;let t=primary_edge_dot(ac,q)/determinant;
        weights=vec3<f32>(1.0-u-v,u,v);
        if all(weights>=vec3<f32>(0.0)) {
            result.world=world_position(receiver_point(vertices[0],vertices[1],vertices[2],weights),slot).xyz;
            result.geometric=primary_geometric(vertices[0],vertices[1],vertices[2],slot);
            result.epsilon=primary_epsilon(vertices,slot);
            return result;
        }
    }
    return receiver;
}
fn rt1b_direction(index:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32)->vec3<f32> {
    if (!RT1B_STRATIFIED && !RT1B_SUNFLOWER) || FRESH_DIRECTIONS!=8u {return fresh_direction(index,shift,l,u,v,slope);}
    var r=vec2<f32>((f32(index%2u)+shift.x)*0.5,(f32(index/2u)+shift.y)*0.25);
    if RT1B_SUNFLOWER {r=vec2<f32>((f32(index)+shift.x)*0.125,fract(fma(f32(index),0.6180339887498949,shift.y)));}
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}
fn rt1b_estimate(trace:PrimaryTrace,receiver:Receiver,pixel:vec2<u32>,width:u32,lane:u32)->u32 {
    rt1b_prepare(trace,receiver,pixel,width);
    return fresh_estimate(receiver,pixel,width,lane);
}
