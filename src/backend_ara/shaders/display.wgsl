// The swapchain is UNORM: encode the display transfer on the GPU before presenting.
struct Settings { bloom_motion:vec4<f32>,lens:vec4<f32>,grade:vec4<f32>,white:vec4<f32>,display:vec4<f32> };
@group(0) @binding(0) var colour:texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
    let p=vec2<f32>(f32((id<<1u)&2u),f32(id&2u));return vec4<f32>(p*2.0-1.0,0.0,1.0);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    let c=clamp(textureLoad(colour,vec2<i32>(p.xy),0).rgb,vec3<f32>(0.0),vec3<f32>(1.0));
    let encoded=select(1.055*pow(c,vec3<f32>(1.0/2.4))-0.055,c*12.92,c<=vec3<f32>(0.0031308));
    return vec4<f32>(encoded,1.0);
}
