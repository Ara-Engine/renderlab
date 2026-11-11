// ---- Entity-ID glass raster (src/id_output.rs) ----
// Appended to the installed prelude. Transparent draws, rasterized with the unjittered
// camera (the prelude's `camera`, bound by this pass) and reversed-Z depth: the nearest
// transparent surface in front of the opaque view depth that is not fully clear writes its
// transparent instance + 1. Group 0 is this pass's own; groups 1 and 2 are the draw's
// instances and material, as on every raster pass.
struct IdGlassView { eye:vec4<f32>, forward:vec4<f32> };
@group(0) @binding(40) var<uniform> id_glass_view:IdGlassView;
// transparency_gpu.rs's opaque view depth: dot(world - eye, forward), or its miss value.
@group(0) @binding(41) var id_opaque_depth:texture_2d<f32>;
// x: the draw's first transparent instance in the tag table.
@group(0) @binding(42) var<uniform> id_glass_draw:vec4<u32>;
struct IdGlassVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) color:vec4<f32>,
    @location(1) uv:vec2<f32>,
    @location(2) @interpolate(flat) winding:f32,
    @location(3) world:vec3<f32>,
    @location(4) @interpolate(flat) slot:u32,
};
@vertex fn id_glass_vs(input:VertexInput) -> IdGlassVertex {
    let o=vertex_output(input);
    return IdGlassVertex(o.position,o.color,o.uv,o.winding,o.world,input.slot);
}
@fragment fn id_glass_fs(input:IdGlassVertex, @builtin(front_facing) front:bool) -> @location(0) u32 {
    // Before any discard: implicit derivatives need uniform control flow.
    let alpha=textureSample(base_color,base_sampler,input.uv).a;
    if material.double_sided==0u && front!=(input.winding>0.0) { discard; }
    let pixel=vec2<i32>(input.position.xy);
    let depth=dot(input.world-id_glass_view.eye.xyz,id_glass_view.forward.xyz);
    if depth>textureLoad(id_opaque_depth,pixel,0).x { discard; }
    // A pane is in front where its coverage (opacity after the material's and the texture's
    // alpha, as transparent_coverage) is at least a tenth. Opacity is stored as f16, where
    // the default 0.1 reads 0.09998: the threshold sits just below it.
    let opacity=select(1.0,unpack2x16float(material.pad).x,material.pad!=0u);
    let coverage=clamp(opacity*material.base_color.a*alpha*input.color.a,0.0,1.0);
    if coverage<0.0995 { discard; }
    return id_glass_draw.x+input.slot+1u;
}
// ---- end of entity-ID glass raster ----
