// N8g walk records (docs/non-rt-n8g.md): per addressed triangle of one traced draw, its three
// object-space positions with its edge-adjacency neighbours, one vec4<u32> per k = 0..2: xyz
// the bits of corner k's position, w the adjacency word of edge k (vsm_neighbour's) or
// 0xffffffff without adjacency. The lookup then reads a triangle and its neighbours from 48
// contiguous bytes instead of index -> vertex -> adjacency chains. Bits are copied, never
// computed, so walks over records equal walks over the index, vertex and adjacency buffers.
// virtual_shadow_gpu.rs prepends PRIMARY_BATCHES and dispatches one pass per segment of
// 2^21 triangles (one storage binding each): `count` triangles from `first`.

struct VsmRecordPass {batch:u32,count:u32,first:u32,pad:u32};
@group(0) @binding(22) var<uniform> vsm_record_pass:VsmRecordPass;
@group(0) @binding(23) var<storage,read_write> vsm_records_out:array<vec4<u32>>;
struct VsmVec4s {data:array<vec4<f32>>};
struct VsmU32s {data:array<u32>};
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<VsmVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<VsmU32s,PRIMARY_BATCHES>;

@compute @workgroup_size(64) fn vsm_record(@builtin(global_invocation_id) id:vec3<u32>) {
    let local=id.x+id.y*65535u*64u;
    if local>=vsm_record_pass.count {return;}
    let t=vsm_record_pass.first+local;
    let b=vsm_record_pass.batch;
    // The adjacency suffix of the index buffer (receiver_adjacency.rs): 6T+1 words.
    let words=arrayLength(&primary_indices[b].data);
    let adjacency=words%3u==1u;
    for(var k=0u;k<3u;k+=1u) {
        let index=primary_indices[b].data[3u*t+k];
        let p=bitcast<vec3<u32>>(primary_vertices[b].data[index*3u].xyz);
        var n=0xffffffffu;
        if adjacency {n=primary_indices[b].data[(words-1u)/2u+3u*t+k];}
        vsm_records_out[3u*local+k]=vec4<u32>(p,n);
    }
}
