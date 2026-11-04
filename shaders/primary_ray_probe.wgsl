// Phase A only. No production pipeline uses this module. Both samples are at 1x centres.
enable wgpu_ray_query;
struct Camera {
    view_projection:mat4x4<f32>,
    inverse_view_projection:mat4x4<f32>,
    size:vec2<u32>,
    pad:vec2<u32>,
};
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var scene:acceleration_structure;
// Full precision barycentrics, instance+1 (zero means miss), primitive index.
@group(0) @binding(2) var hits:texture_storage_2d<rgba32uint,write>;
@group(0) @binding(3) var depths:texture_storage_2d<r32float,write>;
// Validation-only world position, excluded from the timing pipeline by the override.
@group(0) @binding(4) var worlds:texture_storage_2d<rgba32float,write>;
override WRITE_WORLD:bool=false;
@compute @workgroup_size(8,4) fn primary_probe(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=camera.size) {return;}
    let ndc=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(camera.size)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let a=camera.inverse_view_projection*vec4<f32>(ndc,1.0,1.0);
    let b=camera.inverse_view_projection*vec4<f32>(ndc,0.0,1.0);
    let near=a.xyz/a.w;
    let far=b.xyz/b.w;
    let delta=far-near;
    let distance=length(delta);
    let direction=delta/distance;
    var query:ray_query;
    // Closest hit: deliberately no terminate-on-first-hit flag.
    rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,0.0,distance,near,direction));
    while rayQueryProceed(&query) {}
    let hit=rayQueryGetCommittedIntersection(&query);
    var record=vec4<u32>(0u);
    var world=vec4<f32>(0.0);
    if hit.kind!=0u {
        let p=near+direction*hit.t;
        let clip=camera.view_projection*vec4<f32>(p,1.0);
        world=vec4<f32>(p,clip.z/clip.w);
        record=vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics));
    }
    textureStore(hits,vec2<i32>(id.xy),record);
    textureStore(depths,vec2<i32>(id.xy),vec4<f32>(world.w));
    if WRITE_WORLD {textureStore(worlds,vec2<i32>(id.xy),world);}
}
