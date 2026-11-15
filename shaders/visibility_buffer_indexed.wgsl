// A triangle's first corner is unique and owns its flat ID. The other corners
// reuse mesh-local vertices; the hardware post-transform cache does their work once.
@group(0) @binding(8) var<storage,read_write> visibility:array<atomic<u64>>;
@group(0) @binding(9) var<storage,read> vertices:array<vec4<u32>>;
struct Out {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(flat) payload:u32,
    @location(1) @interpolate(flat) sides:u32,
};
@vertex fn vs(@builtin(vertex_index) index:u32)->Out {
    let vertex=vertices[index];let c=clusters[vertex.w>>7u];
    let placement=c.placement;let packed=placements[placement];
    var out:Out;
    out.position=config.matrix*(models[placement]*vec4<f32>(bitcast<vec3<f32>>(vertex.xyz),1.0));
    out.payload=vertex.w;
    out.sides=select(0u,select(2u,1u,normals[placement].info.x>0.0),(packed&0x40000000u)!=0u);
    return out;
}
@fragment fn fs(input:Out,@builtin(front_facing) front:bool) {
    if input.sides!=0u && front!=(input.sides==1u) {discard;}
    let p=vec2<u32>(input.position.xy);
    atomicMax(&visibility[p.y*image_size().x+p.x],(u64(bitcast<u32>(input.position.z))<<32u)|u64(input.payload));
}
@fragment fn fs_color(input:Out,@builtin(front_facing) front:bool)->@location(0) u32 {
    if input.sides!=0u && front!=(input.sides==1u) {discard;}
    return input.payload;
}
