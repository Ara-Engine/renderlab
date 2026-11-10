enable wgpu_binding_array;
struct Words { header: vec4<u32>, data: array<u32> };
struct Counters { data: array<atomic<u32>> };
@group(0) @binding(0) var<storage,read> reads: binding_array<Words,4>;
@group(0) @binding(1) var<storage,read_write> output: array<u32>;
@group(0) @binding(2) var<storage,read_write> writes: binding_array<Counters,4>;
fn fetch(which: u32, at: u32) -> u32 { return reads[which].data[at]; }
@compute @workgroup_size(4)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= 4u { return; }
    let value = fetch(i, i * 7u + 2u);
    output[i * 4u] = value;
    output[i * 4u + 1u] = arrayLength(&reads[i].data);
    output[i * 4u + 2u] = reads[2u].data[i + 1u];
    output[i * 4u + 3u] = reads[i].header.x;
    atomicAdd(&writes[i].data[0], value);
}
