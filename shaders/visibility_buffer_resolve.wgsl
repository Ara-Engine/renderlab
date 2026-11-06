@group(0) @binding(29) var raster_ids:texture_2d<u32>;
@group(0) @binding(13) var primary_hit_output:texture_storage_2d<rgba32uint,write>;
@group(0) @binding(16) var primary_world_output:texture_storage_2d<rgba32float,write>;
@group(0) @binding(17) var primary_normal_output:texture_storage_2d<rgba32float,write>;
@group(0) @binding(20) var primary_tile_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(23) var primary_epsilon_output:texture_storage_2d<r32float,write>;
var<workgroup> covered:atomic<u32>;
override WRITE_RECEIVERS:bool=true;
// Reintersect the pixel-centre ray with the selected source triangle. Rebase beside
// the facet before the barycentric solve; raster-interpolated world positions and
// quantized depth are never receiver origins. Preserve receiver_point's FMA order.
fn raster_trace(pixel:vec2<u32>,size:vec2<u32>,ids:vec2<u32>)->PrimaryTrace {
    let slot=ids.x-1u;let batch=primary_placements[slot]&0x1fffffffu;
    let v=primary_triangle_vertices(batch,ids.y);
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let a=world_position(v[0],slot).xyz;let ab=linear*(v[1]-v[0]);let ac=linear*(v[2]-v[0]);
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;let distance=length(delta);let direction=delta/distance;
    let local=fma(direction,vec3<f32>(primary_edge_dot(a-ray.near,direction)),ray.near)-a;
    let h=cross(direction,ac);let determinant=primary_edge_dot(ab,h);
    let u=primary_edge_dot(local,h)/determinant;let q=cross(local,ab);
    let w=primary_edge_dot(direction,q)/determinant;
    let weights=vec3<f32>(1.0-u-w,u,w);
    let point=world_position(receiver_point(v[0],v[1],v[2],weights),slot).xyz;
    return primary_repair(PrimaryTrace(vec4<u32>(ids,bitcast<vec2<u32>>(weights.yz)),point,v),ray.near,direction);
}
@compute @workgroup_size(8,4) fn resolve(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(raster_ids);
    var trace:PrimaryTrace;
    if all(id.xy<size) {
        let ids=textureLoad(raster_ids,id.xy,0).xy;
        if ids.x!=0u {trace=raster_trace(id.xy,size,ids);atomicOr(&covered,1u);}
    }
    workgroupBarrier();
    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(atomicLoad(&covered)));}
    if atomicLoad(&covered)==0u || any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);textureStore(primary_hit_output,p,trace.record);
    if trace.record.x==0u {return;}
    let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
    textureStore(primary_epsilon_output,p,vec4<f32>(receiver.epsilon));
    if WRITE_RECEIVERS {
        textureStore(primary_world_output,p,vec4<f32>(receiver.world,receiver.epsilon));
        textureStore(primary_normal_output,p,vec4<f32>(receiver.geometric,select(1.0,-1.0,receiver.thin)));
    }
}
