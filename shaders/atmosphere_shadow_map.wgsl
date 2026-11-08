// VF1: the conventional raster path's already-rendered, reversed-Z sun maps.
struct AtShadowView {
    world_to_clip:mat4x4<f32>,coverage_bias:vec4<f32>,camera_depth:vec4<f32>,texel:vec4<f32>,
};
@group(0) @binding(21) var at_shadow_depth:texture_depth_2d_array;
@group(0) @binding(22) var<storage,read> at_shadow_views:array<AtShadowView>;
fn at_sun_visible(world:vec3<f32>) -> f32 {
    let first=u32(atmosphere.viewport.z);
    let count=u32(atmosphere.viewport.w);
    for(var i=0u;i<count;i+=1u) {
        let layer=first+i;
        let view=at_shadow_views[layer];
        let depth=dot(view.camera_depth,vec4<f32>(world,1.0));
        if depth<view.coverage_bias.x || depth>view.coverage_bias.y { continue; }
        let clip=view.world_to_clip*vec4<f32>(world,1.0);
        if clip.w<=0.0 { continue; }
        let ndc=clip.xyz/clip.w;
        if any(abs(ndc.xy)>vec2<f32>(1.0)) || ndc.z<0.0 || ndc.z>1.0 { continue; }
        let extent=vec2<i32>(textureDimensions(at_shadow_depth));
        let uv=ndc.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
        let pixel=clamp(vec2<i32>(uv*vec2<f32>(extent)),vec2<i32>(0),extent-vec2<i32>(1));
        let stored=textureLoad(at_shadow_depth,pixel,i32(layer),0);
        return select(0.0,1.0,ndc.z+view.coverage_bias.z>=stored);
    }
    return 1.0;
}
