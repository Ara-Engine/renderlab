@group(0) @binding(2) var<storage,read> visible:array<u32>;
@group(0) @binding(6) var<storage,read> source_triangles:array<u32>;
struct Out {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(flat) ids:vec2<u32>,
    @location(1) @interpolate(flat) winding:f32,
    @location(2) @interpolate(flat) single:u32,
};
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->Out {
    let c=clusters[visible[instance]];let local=vertex/3u;
    var out:Out;
    out.position=vec4<f32>(0.0,0.0,0.0,1.0);
    out.ids=vec2<u32>(c.placement+1u,c.first+local);
    out.winding=normals[c.placement].info.x;
    let packed=placements[c.placement];let batch=packed&0x1fffffffu;
    out.single=packed&0x40000000u;
    if local<c.count {
        let primitive=source_triangles[c.first+local];out.ids.y=primitive;
        let i=indices[batch].data[primitive*3u+vertex%3u];
        out.position=config.matrix*(models[c.placement]*vec4<f32>(vertices[batch].data[i*3u].xyz,1.0));
    }
    return out;
}
@fragment fn fs(input:Out,@builtin(front_facing) front:bool)->@location(0) vec2<u32> {
    if input.single!=0u && front!=(input.winding>0.0) {discard;}
    return input.ids;
}
