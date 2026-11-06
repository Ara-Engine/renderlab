// Interactive soft frames: each pixel's closest-hit camera ray and its 8-direction
// visibility in one dispatch. Appended with primary_trace.wgsl to the soft module, so
// fresh_estimate takes the exact receiver from registers instead of a target round trip.
// Counts equal primary_trace_receivers followed by 8-direction fresh_compute: same
// receiver, directions, flags and 0..256 output contract, misses and skips 256.
// Tile flags and the scene rectangle are primary_trace.wgsl's: only tiles with a camera
// hit write hit records and counts.
@compute @workgroup_size(8,4) fn primary_trace_interactive(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    let inside=all(id.xy<size);
    var trace:PrimaryTrace;
    let scene=primary_frame.scene;
    if inside && all(id.xy>=scene.xy) && all(id.xy<scene.zw) {trace=primary_closest(id.xy,size);}
    if trace.record.x!=0u {atomicOr(&primary_tile_hit,1u);}
    workgroupBarrier();
    let covered=atomicLoad(&primary_tile_hit);
    // A tile's first pixel is always inside the image.
    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}
    if !inside || covered==0u {return;}
    let p=vec2<i32>(id.xy);
    textureStore(primary_hit_output,p,trace.record);
    var visible=FRESH_SKIP;
    if trace.record.x!=0u {
        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
        visible=fresh_estimate(receiver,id.xy,size.x,0u);
    }
    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));
}

// Only raw hit records cross this boundary. Each listed pixel owns its record and
// visibility stores; atomic append order cannot change its inputs or arithmetic.
struct PrimaryRepairItem {pixel:u32,placement:u32,primitive:u32,u:u32,v:u32};
struct PrimaryRepairQueue {x:atomic<u32>,y:atomic<u32>,z:u32,count:atomic<u32>,items:array<PrimaryRepairItem>};
@group(0) @binding(24) var<storage,read_write> primary_repairs:PrimaryRepairQueue;
struct PrimaryRepairInput {x:u32,y:u32,z:u32,count:u32,items:array<PrimaryRepairItem>};
@group(0) @binding(26) var<storage,read> primary_repair_input:PrimaryRepairInput;
var<workgroup> primary_repair_mask:atomic<u32>;
var<workgroup> primary_repair_base:u32;

// A scalar conservative bound on all three error-scaled barycentric margins.
// Squared distances avoid normalizing or taking square roots on the common path.
fn primary_near_edge(trace:PrimaryTrace,epsilon:f32)->bool {
    let bary=bitcast<vec2<f32>>(trace.record.zw);
    let weights=vec3<f32>(1.0-bary.x-bary.y,bary);
    if all(weights>=vec3<f32>(0.25)) {return false;}
    let m=models[trace.record.x-1u];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let ab=linear*(trace.positions[1]-trace.positions[0]);let ac=linear*(trace.positions[2]-trace.positions[0]);
    let bc=ac-ab;let area=cross(ab,ac);
    let edges=vec3<f32>(dot(bc,bc),dot(ac,ac),dot(ab,ab));
    return any((weights<vec3<f32>(0.25)) & (weights*weights*dot(area,area)<(16.0*1.000004)*epsilon*epsilon*edges));
}

@compute @workgroup_size(8,4) fn primary_trace_interactive_deferred(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if all(id.xy==vec2<u32>(0u)) {primary_repairs.z=1u;}
    let size=textureDimensions(primary_hit_output);let in_bounds=all(id.xy<size);
    var trace:PrimaryTrace;let scene=primary_frame.scene;
    if in_bounds && all(id.xy>=scene.xy) && all(id.xy<scene.zw) {trace=primary_raw_closest(id.xy,size);}
    var receiver:Receiver;var near_edge=false;
    if trace.record.x!=0u {
        receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,vec2<i32>(id.xy));
        near_edge=primary_near_edge(trace,receiver.epsilon);
        atomicOr(&primary_tile_hit,1u);
        if near_edge {atomicOr(&primary_repair_mask,1u<<lane);}
    }
    workgroupBarrier();
    // One global append per tile. Lane rank is fixed even when tiles finish in a
    // different order; no pixel reads another pixel's result.
    let mask=atomicLoad(&primary_repair_mask);
    if lane==0u && mask!=0u {
        let count=countOneBits(mask);
        let base=atomicAdd(&primary_repairs.count,count);primary_repair_base=base;
        let block=(base+count-1u)/32u;
        if base%32u==0u || base/32u!=block {
            atomicMax(&primary_repairs.x,min(block+1u,256u));
            atomicMax(&primary_repairs.y,block/256u+1u);
        }
    }
    workgroupBarrier();
    let covered=atomicLoad(&primary_tile_hit);
    if all(id.xy == group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}
    if !in_bounds || covered==0u {return;}
    let p=vec2<i32>(id.xy);textureStore(primary_hit_output,p,trace.record);
    var visible=FRESH_SKIP;
    if trace.record.x!=0u {
        if near_edge {
            let index=primary_repair_base+countOneBits(mask&((1u<<lane)-1u));
            primary_repairs.items[index]=PrimaryRepairItem(id.x+id.y*size.x,trace.record.x,trace.record.y,trace.record.z,trace.record.w);
        }
        visible=fresh_estimate(receiver,id.xy,size.x,0u);
    }
    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));
}

@compute @workgroup_size(32) fn primary_repair_interactive(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=(group.x+group.y*256u)*32u+lane;
    if index>=primary_repair_input.count {return;}
    let size=textureDimensions(primary_hit_output);let flat=primary_repair_input.items[index].pixel;
    let id=vec2<u32>(flat%size.x,flat/size.x);let p=vec2<i32>(id);
    let record=vec4<u32>(primary_repair_input.items[index].placement,primary_repair_input.items[index].primitive,primary_repair_input.items[index].u,primary_repair_input.items[index].v);
    let slot=record.x-1u;let batch=primary_placements[slot]&0x1fffffffu;let base=record.y*3u;
    let v=primary_triangle_vertices(batch,record.y);
    let bary=bitcast<vec2<f32>>(record.zw);let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let point=world_position(receiver_point(v[0],v[1],v[2],w),slot).xyz;
    let ray=primary_ray(id,size);let delta=ray.far-ray.near;
    let trace=primary_repair(PrimaryTrace(record,point,v),ray.near,delta/length(delta));
    if all(trace.record==record) && all(trace.point==point) {return;}
    textureStore(primary_hit_output,p,trace.record);
    let receiver=primary_receiver(trace.point,trace.positions,slot,p);
    let visible=fresh_estimate(receiver,id,size.x,0u);
    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));
}
