// N8g walk records (docs/non-rt-n8g.md) in N8h's layout (docs/non-rt-n8h.md): per recorded
// triangle of one traced draw, the bits of its three object-space corner positions, nine
// u32 (36 bytes). The lookup then reads a triangle's positions from one contiguous record
// instead of index -> vertex chains. Bits are copied, never computed, so walks over records
// equal walks over the index and vertex buffers. virtual_shadow_gpu.rs prepends
// PRIMARY_BATCHES and dispatches one pass per piece of a batch within one segment of 2^21
// records (one storage binding each): `count` triangles from `first`, written from record
// `at` of the segment.

struct VsmRecordPass {batch:u32,count:u32,first:u32,at:u32};
@group(0) @binding(22) var<uniform> vsm_record_pass:VsmRecordPass;
@group(0) @binding(23) var<storage,read_write> vsm_records_out:array<u32>;
struct VsmVec4s {data:array<vec4<f32>>};
struct VsmU32s {data:array<u32>};
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<VsmVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<VsmU32s,PRIMARY_BATCHES>;

@compute @workgroup_size(64) fn vsm_record(@builtin(global_invocation_id) id:vec3<u32>) {
    let local=id.x+id.y*65535u*64u;
    if local>=vsm_record_pass.count {return;}
    let t=vsm_record_pass.first+local;
    let b=vsm_record_pass.batch;
    let out=(vsm_record_pass.at+local)*9u;
    for(var k=0u;k<3u;k+=1u) {
        let index=primary_indices[b].data[3u*t+k];
        let p=bitcast<vec3<u32>>(primary_vertices[b].data[index*3u].xyz);
        vsm_records_out[out+3u*k]=p.x;
        vsm_records_out[out+3u*k+1u]=p.y;
        vsm_records_out[out+3u*k+2u]=p.z;
    }
}
