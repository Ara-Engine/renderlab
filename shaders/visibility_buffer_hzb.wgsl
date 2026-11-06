@group(0) @binding(0) var depth:texture_depth_2d;
@group(0) @binding(1) var output:texture_storage_2d<r32float,write>;
@group(0) @binding(2) var source:texture_2d<f32>;
@compute @workgroup_size(8,8) fn base(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    var z=0.0;
    if all(id.xy<textureDimensions(depth)) {z=textureLoad(depth,vec2<i32>(id.xy),0);}
    textureStore(output,id.xy,vec4<f32>(z));
}
@compute @workgroup_size(8,8) fn reduce(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    let p=vec2<i32>(id.xy*2u);let end=vec2<i32>(textureDimensions(source))-vec2<i32>(1);
    let z=min(min(textureLoad(source,min(p,end),0).x,textureLoad(source,min(p+vec2<i32>(1,0),end),0).x),min(textureLoad(source,min(p+vec2<i32>(0,1),end),0).x,textureLoad(source,min(p+vec2<i32>(1),end),0).x));
    textureStore(output,id.xy,vec4<f32>(z));
}
