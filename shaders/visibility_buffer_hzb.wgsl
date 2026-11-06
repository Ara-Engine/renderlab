// Min-depth (reversed-Z: farthest) pyramid. Level 0 texels are 2x2 pixels of the 64-bit
// visibility buffer's depth word; pixels outside the image are 0 and never occlude.
struct Size {width:u32,height:u32,pad0:u32,pad1:u32};
@group(0) @binding(0) var<storage,read> visibility:array<vec2<u32>>;
@group(0) @binding(1) var output:texture_storage_2d<r32float,write>;
@group(0) @binding(2) var source:texture_2d<f32>;
@group(0) @binding(3) var<uniform> image:Size;
@compute @workgroup_size(8,8) fn base(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    var z=1.0;
    for(var i=0u;i<4u;i+=1u) {
        let p=id.xy*2u+vec2<u32>(i&1u,i>>1u);
        var d=0.0;
        if p.x<image.width && p.y<image.height {d=bitcast<f32>(visibility[p.y*image.width+p.x].y);}
        z=min(z,d);
    }
    textureStore(output,id.xy,vec4<f32>(z));
}
@compute @workgroup_size(8,8) fn reduce(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    let p=vec2<i32>(id.xy*2u);let end=vec2<i32>(textureDimensions(source))-vec2<i32>(1);
    let z=min(min(textureLoad(source,min(p,end),0).x,textureLoad(source,min(p+vec2<i32>(1,0),end),0).x),min(textureLoad(source,min(p+vec2<i32>(0,1),end),0).x,textureLoad(source,min(p+vec2<i32>(1),end),0).x));
    textureStore(output,id.xy,vec4<f32>(z));
}
