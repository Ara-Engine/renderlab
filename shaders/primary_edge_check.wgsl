// Geometry checks only. The composer supplies reachable arithmetic helpers and
// rejects traversal instructions. A missing neighbour delegates the original hit.
@group(0) @binding(24) var<storage,read_write> primary_repairs:PrimaryRepairQueue;
@group(0) @binding(28) var<storage,read_write> primary_retries:PrimaryRetryQueue;
var<private> primary_check_fallback:bool;
@compute @workgroup_size(32) fn primary_check_interactive(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=(group.x+group.y*256u)*32u+lane;
    if index==0u {primary_retries.z=1u;}
    if index>=atomicLoad(&primary_repairs.count) {return;}
    let item=primary_repairs.items[index];let image=vec2<u32>(primary_repairs.x,primary_repairs.y);
    let id=vec2<u32>(item.pixel%image.x,item.pixel/image.x);
    let record=vec4<u32>(item.placement,item.primitive,item.u,item.v);let slot=record.x-1u;
    let v=primary_triangle_vertices(primary_placements[slot]&0x1fffffffu,record.y);
    let bary=bitcast<vec2<f32>>(record.zw);let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let point=world_position(receiver_point(v[0],v[1],v[2],w),slot).xyz;
    let ray=primary_ray(id,image);let delta=ray.far-ray.near;
    let trace=primary_repair(PrimaryTrace(record,point,v),ray.near,delta/length(delta));
    if !primary_check_fallback && all(trace.record==record) && all(trace.point==point) {return;}
    primary_repairs.items[index].fallback=u32(primary_check_fallback);
    if !primary_check_fallback {
        primary_repairs.items[index].primitive=trace.record.y;
        primary_repairs.items[index].u=trace.record.z;primary_repairs.items[index].v=trace.record.w;
        primary_repairs.items[index].px=bitcast<u32>(trace.point.x);primary_repairs.items[index].py=bitcast<u32>(trace.point.y);primary_repairs.items[index].pz=bitcast<u32>(trace.point.z);
    }
    let retry=atomicAdd(&primary_retries.count,1u);primary_retries.items[retry]=index;
    if retry%32u==0u {let block=retry/32u;atomicMax(&primary_retries.x,min(block+1u,256u));atomicMax(&primary_retries.y,block/256u+1u);}
}
