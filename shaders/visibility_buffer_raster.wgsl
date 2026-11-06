// Hardware raster of large or near-plane clusters: one instance per cluster, 128 triangle
// slots. Unused slots collapse to a point. The depth attachment only rejects fragments
// behind other hardware fragments; the 64-bit max decides final visibility.
// Hardware clusters from the cluster count (visibility_buffer_cull.wgsl).
@group(0) @binding(3) var<storage,read> lists:array<u32>;
@group(0) @binding(8) var<storage,read_write> visibility:array<atomic<u64>>;
// Its own group: wgpu forbids binding arrays beside uniform buffers.
@group(2) @binding(8) var<storage,read> cluster_data:binding_array<U32s,64>;
struct Out {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(flat) payload:u32,
    // 0: both sides, 1: front faces only, 2: back faces only (mirrored single-sided).
    @location(1) @interpolate(flat) sides:u32,
};
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->Out {
    let index=lists[config.info.x+instance];let c=clusters[index];let local=vertex/3u;
    var out:Out;
    out.position=vec4<f32>(0.0,0.0,0.0,1.0);
    out.payload=(index<<7u)|local;
    let packed=placements[c.placement];
    out.sides=select(0u,select(2u,1u,normals[c.placement].info.x>0.0),(packed&0x40000000u)!=0u);
    if local<(c.counts&255u) {
        let batch=packed&0x1fffffffu;
        let t=cluster_data[batch].data[c.triangles+local];
        let o=c.vertices+3u*((t>>(8u*(vertex%3u)))&255u);let d=&cluster_data[batch].data;
        let p=vec3<f32>(bitcast<f32>((*d)[o]),bitcast<f32>((*d)[o+1u]),bitcast<f32>((*d)[o+2u]));
        out.position=config.matrix*(models[c.placement]*vec4<f32>(p,1.0));
    }
    return out;
}
@fragment fn fs(input:Out,@builtin(front_facing) front:bool) {
    if input.sides!=0u && front!=(input.sides==1u) {discard;}
    let p=vec2<u32>(input.position.xy);
    atomicMax(&visibility[p.y*image_size().x+p.x],(u64(bitcast<u32>(input.position.z))<<32u)|u64(input.payload));
}
