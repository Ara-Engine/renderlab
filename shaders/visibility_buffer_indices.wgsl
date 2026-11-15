// Compact actual visible triangles, including their original provoking corner.
@group(0) @binding(3) var<storage,read> lists:array<u32>;
@group(0) @binding(5) var<storage,read> cull_args:array<u32>;
struct MeshDraw {first:u32,vertices:u32,cluster_base:u32,placement_base:u32,clusters:u32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(6) var<storage,read> meshes:array<MeshDraw>;
struct DrawArgs {count:atomic<u32>,instances:u32,first:u32,base:i32,first_instance:u32};
@group(0) @binding(7) var<storage,read_write> draws:array<DrawArgs>;
@group(0) @binding(8) var<storage,read_write> indices:array<u32>;
@group(2) @binding(8) var<storage,read> cluster_data:binding_array<U32s,PRIMARY_BATCHES>;
var<workgroup> output_base:u32;
var<workgroup> selected:u32;
@compute @workgroup_size(128) fn build_indices(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let instance=group.x+group.y*65535u;
    if li==0u {
        selected=0xffffffffu;
        if instance<cull_args[5] {selected=lists[config.info.x+instance];}
    }
    let index=workgroupUniformLoad(&selected);
    if index==0xffffffffu {return;}
    let c=clusters[index];
    let batch=placements[c.placement]&0x1fffffffu;let mesh=meshes[batch];
    let n=c.counts&255u;
    if li==0u {output_base=mesh.first+atomicAdd(&draws[batch].count,n*3u);}
    workgroupBarrier();
    if li>=n {return;}
    let t=cluster_data[batch].data[c.triangles+li];
    let base=(c.placement-mesh.placement_base)*mesh.vertices;
    let at=output_base+li*3u;
    indices[at]=base+c.pad2+li;
    indices[at+1u]=base+c.pad1+((t>>8u)&255u);
    indices[at+2u]=base+c.pad1+((t>>16u)&255u);
}
@compute @workgroup_size(64) fn reset_indices(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x<arrayLength(&draws) {atomicStore(&draws[id.x].count,0u);}
}
