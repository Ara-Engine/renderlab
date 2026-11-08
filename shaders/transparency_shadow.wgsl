// TP2: one nearest transparent sheet per draw, independently depth tested. Layers
// multiply when they lie between the receiver and the sun; opaque visibility stays
// with the existing shadow implementation. RGB is transmitted light, alpha depth.
struct TpTransmission { matrix:mat4x4<f32>, sun:vec4<f32>, info:vec4<f32> };
@group(3) @binding(26) var<uniform> tp_transmission:TpTransmission;
@group(3) @binding(27) var tp_transmission_map:texture_2d_array<f32>;
fn transparent_sun_transmission(world:vec3<f32>, direction:vec3<f32>) -> vec3<f32> {
    if tp_transmission.sun.w==0.0 || dot(direction,tp_transmission.sun.xyz)<0.9999 { return vec3<f32>(1.0); }
    let p=tp_transmission.matrix*vec4<f32>(world,1.0);
    let uv=p.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    if any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0)) { return vec3<f32>(1.0); }
    let size=vec2<i32>(textureDimensions(tp_transmission_map));
    let q=uv*vec2<f32>(size)-vec2<f32>(0.5);
    let base=vec2<i32>(floor(q));let f=fract(q);
    var result=vec3<f32>(1.0);
    for(var layer=0;layer<i32(tp_transmission.sun.w);layer+=1) {
        var filtered=vec3<f32>(0.0);
        for(var y=0;y<2;y+=1) { for(var x=0;x<2;x+=1) {
            let t=textureLoad(tp_transmission_map,clamp(base+vec2<i32>(x,y),vec2<i32>(0),size-vec2<i32>(1)),layer,0);
            let w=select(1.0-f.x,f.x,x==1)*select(1.0-f.y,f.y,y==1);
            filtered+=w*select(vec3<f32>(1.0),t.rgb,p.z>t.a+tp_transmission.info.x);
        } }
        result*=filtered;
    }
    return result;
}
