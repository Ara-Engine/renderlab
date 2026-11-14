struct FogSunMap { clips:array<mat4x4<f32>,2>, params:vec4<f32> };
@group(0) @binding(24) var fog_sun_depth:texture_depth_2d_array;
@group(0) @binding(25) var<uniform> fog_sun_map:FogSunMap;
fn at_sun_visible(world:vec3<f32>)->f32 {
    for(var layer=0u;layer<2u;layer+=1u) {
        let p=(fog_sun_map.clips[layer]*vec4<f32>(world,1.0)).xyz;
        if any(abs(p.xy)>=vec2<f32>(1.0)) || p.z<0.0 || p.z>1.0 {continue;}
        let size=vec2<i32>(textureDimensions(fog_sun_depth));
        let uv=p.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
        let pixel=clamp(vec2<i32>(uv*vec2<f32>(size)),vec2<i32>(0),size-vec2<i32>(1));
        let depth=textureLoad(fog_sun_depth,pixel,i32(layer),0);
        return select(0.0,1.0,p.z+fog_sun_map.params.x>=depth);
    }
    // CPU preparation contains the complete radial fog volume in level 1.
    return 1.0;
}
