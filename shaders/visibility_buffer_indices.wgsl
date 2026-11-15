// One compacted stream per phase. Gather current cluster positions after skinning;
// no static vertex copy, padding triangles, or per-mesh draw commands.
@group(0) @binding(3) var<storage,read> lists:array<u32>;
@group(0) @binding(5) var<storage,read> cull_args:array<u32>;
struct DrawArgs {
    count:atomic<u32>,instances:u32,first:u32,base:i32,first_instance:u32,
    vertices:atomic<u32>,overflow:atomic<u32>,pad:u32,
    old_vertices:u32,old_instances:u32,old_first:u32,old_instance:u32,
};
@group(0) @binding(7) var<storage,read_write> draw:DrawArgs;
@group(0) @binding(8) var<storage,read_write> indices:array<u32>;
@group(0) @binding(9) var<storage,read_write> vertices:array<vec4<u32>>;
@group(2) @binding(8) var<storage,read> cluster_data:binding_array<U32s,PRIMARY_BATCHES>;
var<workgroup> output_index:u32;
var<workgroup> output_vertex:u32;
var<workgroup> selected:u32;
var<workgroup> valid:u32;
@compute @workgroup_size(128) fn build_indices(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let instance=group.x+group.y*65535u;
    if li==0u {
        selected=0xffffffffu;
        if instance<cull_args[5] {selected=lists[config.info.x+instance];}
    }
    let index=workgroupUniformLoad(&selected);
    if index==0xffffffffu {return;}
    let c=clusters[index];let nv=(c.counts>>8u)&255u;let nt=c.counts&255u;
    if li==0u {
        output_index=atomicAdd(&draw.count,nt*3u);
        output_vertex=atomicAdd(&draw.vertices,nv+nt);
        valid=u32(output_index+nt*3u<=arrayLength(&indices) && output_vertex+nv+nt<=arrayLength(&vertices));
        if valid==0u {atomicStore(&draw.overflow,1u);}
    }
    if workgroupUniformLoad(&valid)==0u {return;}
    let batch=c.pad1;let d=&cluster_data[batch].data;
    if li<nv {
        let o=c.vertices+3u*li;
        vertices[output_vertex+li]=vec4<u32>((*d)[o],(*d)[o+1u],(*d)[o+2u],index<<7u);
    }
    if li<nt {
        let t=(*d)[c.triangles+li];let o=c.vertices+3u*(t&255u);
        vertices[output_vertex+nv+li]=vec4<u32>((*d)[o],(*d)[o+1u],(*d)[o+2u],(index<<7u)|li);
        let at=output_index+li*3u;
        indices[at]=output_vertex+nv+li;
        indices[at+1u]=output_vertex+((t>>8u)&255u);
        indices[at+2u]=output_vertex+((t>>16u)&255u);
    }
}
// An overflow draws the complete original hardware list. Never clamp geometry to
// the stream capacity; the fallback is decided before either draw reads the counts.
@compute @workgroup_size(1) fn finish_indices() {
    let overflow=atomicLoad(&draw.overflow)!=0u;
    draw.old_instances=select(0u,cull_args[5],overflow);
    if overflow {atomicStore(&draw.count,0u);}
}
@compute @workgroup_size(1) fn reset_indices() {
    atomicStore(&draw.count,0u);atomicStore(&draw.vertices,0u);atomicStore(&draw.overflow,0u);
    draw.old_instances=0u;
}
