// Read-only while this buffer also supplies the indirect dispatch arguments.
struct EdgeQueue {x:u32,y:u32,z:u32,count:u32,pixels:array<u32>};
@group(0) @binding(36) var<storage,read> edge_queue:EdgeQueue;
// The unselected resolve entry remains valid in this module; repair never enqueues.
fn raster_enqueue(pixel:u32) {}
fn raster_flush_queue(first:bool) {}
@compute @workgroup_size(32) fn repair_edges(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let index=(group.x+group.y*65535u)*32u+li;
    if index>=edge_queue.count {return;}
    let size=textureDimensions(primary_hit_output);
    let pixel=edge_queue.pixels[index];let xy=vec2<u32>(pixel%size.x,pixel/size.x);
    let hit=raster_hit(xy,size.x);
    if hit.ids.x==0u {return;}
    let trace=raster_trace(xy,size,hit.ids,hit.positions);
    let p=vec2<i32>(xy);textureStore(primary_hit_output,p,trace.record);
    var receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
    // edge material store
    let lift=hit.error*LOD_LIFT;
    if lift>0.0 {
        let ray=primary_ray(xy,size);let toward=ray.near-receiver.world;
        receiver.world+=select(-receiver.geometric,receiver.geometric,dot(receiver.geometric,toward)>=0.0)*lift;
    }
    textureStore(primary_epsilon_output,p,vec4<f32>(receiver.epsilon));
    if WRITE_RECEIVERS {
        textureStore(primary_world_output,p,vec4<f32>(receiver.world,receiver.epsilon));
        textureStore(primary_normal_output,p,vec4<f32>(receiver.geometric,select(1.0,-1.0,receiver.thin)));
    }
}
