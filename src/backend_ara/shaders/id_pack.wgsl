// One layer of the id texture packed bit-exact into an RGBA8 image (`id_output.wgsl`'s
// handoff, as a compute pass the backend runs into its own storage image): R = bits 0-7 ...
// A = bits 24-31, each byte k written as (k + 0.25) / 255 so any float-to-unorm8 rounding
// stores k.
@group(0) @binding(0) var pack_ids:texture_2d_array<u32>;
@group(0) @binding(1) var pack_out:texture_storage_2d<rgba8unorm,write>;
// x: the layer.
@group(0) @binding(2) var<uniform> pack_layer:vec4<u32>;
@compute @workgroup_size(8,8) fn id_pack(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(pack_out);
    if any(id.xy>=size) {return;}
    let v=textureLoad(pack_ids,vec2<i32>(id.xy),i32(pack_layer.x),0).x;
    let bytes=vec4<f32>(vec4<u32>(v&0xffu,(v>>8u)&0xffu,(v>>16u)&0xffu,v>>24u));
    textureStore(pack_out,id.xy,(bytes+vec4<f32>(0.25))/255.0);
}
