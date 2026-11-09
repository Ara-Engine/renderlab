// P-RTS visibility for listed penumbra pixels (docs/p-rts.md). Appended to TR1's soft
// trace module after primary_interactive.wgsl. temporal_reconstruction.rs derives
// fresh_estimate_budget from that module's own fresh_estimate text (directions j <
// sampling.x, strided by PENUMBRA_LANES), so each listed pixel's count is production's:
// the same receiver, directions, flags and integer sum; only lanes share the loop.
struct PrimaryPenumbraInput {x:u32,y:u32,z:u32,count:u32,items:array<u32>};
@group(0) @binding(29) var<storage,read> primary_penumbra_input:PrimaryPenumbraInput;
const PENUMBRA_LANES:u32=8u;
var<workgroup> penumbra_counts:array<atomic<u32>,4>;
var<workgroup> penumbra_world:array<vec4<f32>,4>;
var<workgroup> penumbra_normal:array<vec4<f32>,4>;
var<workgroup> penumbra_slot:array<u32,4>;
var<workgroup> penumbra_hit:array<u32,4>;

// The probe pass's receiver for the hit `trace`, then primary_repair_interactive's
// correction of it: the same near-edge test and repair inputs, so the same f32 receiver.
fn primary_penumbra_receiver(trace:PrimaryTrace,id:vec2<u32>,size:vec2<u32>)->Receiver {
    let p=vec2<i32>(id);
    let slot=trace.record.x-1u;
    var receiver=primary_receiver(trace.point,trace.positions,slot,p);
    if primary_near_edge(trace,receiver.epsilon) {
        let record=trace.record;
        let v=primary_triangle_vertices(primary_placements[slot]&0x1fffffffu,record.y);
        let bary=bitcast<vec2<f32>>(record.zw);let w=vec3<f32>(1.0-bary.x-bary.y,bary);
        let point=world_position(receiver_point(v[0],v[1],v[2],w),slot).xyz;
        let ray=primary_ray(id,size);let delta=ray.far-ray.near;
        let repaired=primary_repair(PrimaryTrace(record,point,v),ray.near,delta/length(delta));
        if any(repaired.record!=record) || any(repaired.point!=point) {
            receiver=primary_receiver(repaired.point,repaired.positions,slot,p);
        }
    }
    return receiver;
}

// Four listed pixels per group, eight lanes each; lane 0 traces the camera ray.
@compute @workgroup_size(32) fn primary_penumbra_trace(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) index:u32) {
    let slot=index/PENUMBRA_LANES;let lane=index%PENUMBRA_LANES;
    let item=(group.x+group.y*256u)*4u+slot;
    let live=item<primary_penumbra_input.count;
    let size=textureDimensions(fresh_output);
    var id=vec2<u32>(0u);
    if live {let flat=primary_penumbra_input.items[item];id=vec2<u32>(flat%size.x,flat/size.x);}
    if live && lane==0u {
        // Listed pixels had a probe-pass hit; the same ray returns it (a miss keeps the
        // probe count rather than reading past the placements).
        let trace=primary_raw_closest(id,size);
        penumbra_hit[slot]=trace.record.x;
        if trace.record.x!=0u {
            let r=primary_penumbra_receiver(trace,id,size);
            penumbra_world[slot]=vec4<f32>(r.world,r.epsilon);
            penumbra_normal[slot]=vec4<f32>(r.geometric,select(1.0,-1.0,r.thin));
            penumbra_slot[slot]=r.slot;
        }
    }
    workgroupBarrier();
    let traced=live && penumbra_hit[slot]!=0u;
    var partial=FRESH_SKIP;
    if traced {
        let w=penumbra_world[slot];let n=penumbra_normal[slot];
        let receiver=Receiver(w.xyz,n.xyz,vec2<i32>(id),penumbra_slot[slot],w.w,n.w<0.0);
        partial=fresh_estimate_budget(receiver,id,size.x,lane);
    }
    atomicAdd(&penumbra_counts[slot],select(partial,0u,partial==FRESH_SKIP));
    workgroupBarrier();
    if traced && lane==0u {
        textureStore(fresh_output,vec2<i32>(id),vec4<u32>(select((256u/primary_frame.sampling.x)*atomicLoad(&penumbra_counts[slot]),256u,partial==FRESH_SKIP)));
    }
}
