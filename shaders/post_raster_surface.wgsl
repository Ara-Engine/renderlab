struct PostFrame { previous:mat4x4<f32>,current:mat4x4<f32>,inverse:mat4x4<f32>,view:mat4x4<f32>,options:vec4<f32> };
@group(0) @binding(0) var scene_depth:texture_depth_2d;
@group(0) @binding(1) var<uniform> frame:PostFrame;
@group(0) @binding(2) var surface:texture_storage_2d<rgba32float,write>;
@compute @workgroup_size(8,8) fn surface_raster(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(surface);if any(id.xy>=size) {return;}
    let z=textureLoad(scene_depth,vec2<i32>(id.xy),0);
    let uv=(vec2<f32>(id.xy)+0.5)/vec2<f32>(size);
    let h=frame.inverse*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),z,1.0);
    var point=vec4<f32>(h.xyz/h.w,1.0);
    var depth=max(0.001,-(frame.view*point).z);
    if z==0.0 {
        let near=frame.inverse*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),1.0,1.0);
        point=vec4<f32>(normalize(point.xyz-near.xyz/near.w),0.0);depth=1e8;
    }
    let a=frame.current*point;let b=frame.previous*point;
    var velocity=vec2<f32>(0.0);
    if b.w>0.0 && frame.options.z>0.5 {
        velocity=(a.xy/a.w-b.xy/b.w)*vec2<f32>(0.5,-0.5)*vec2<f32>(size)*frame.options.x;
        velocity*=min(1.0,frame.options.y/max(length(velocity),1e-5));
    }
    textureStore(surface,id.xy,vec4<f32>(velocity,depth,1.0));
}
