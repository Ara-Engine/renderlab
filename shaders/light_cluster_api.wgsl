// ---- LC1 clustered lights and local shadows (engine-private; docs/m7-lc1.md) ----
// The camera's froxel grid (shaders/light_cluster.wgsl writes it each lit frame) and the
// pooled local shadow maps. Programs never read these: `light_count`, `light_sample` and
// `light_visibility` walk the receiver's cluster through them. No storage buffers: the
// header is a uniform and the lists a u32 texture (cluster c's word k at texel
// (c % LC_ROW * LC_STRIDE + k, c / LC_ROW)).
@group(3) @binding(23) var<uniform> rl_light_frame:LcFrame;
@group(3) @binding(24) var rl_cluster_lists:texture_2d<u32>;
@group(3) @binding(25) var rl_local_depth:texture_depth_2d_array;
// x: this receiver's cluster, y: its light count, z: 1 once the entry has prepared the
// receiver. Unprepared stages (cell shading, film accumulation) and receivers outside the
// grid walk every light in order.
var<private> rl_light_list:vec3<u32>;

fn rl_cluster_word(c:u32, k:u32) -> u32 {
    return textureLoad(rl_cluster_lists,vec2<u32>((c%LC_ROW)*LC_STRIDE+k,c/LC_ROW),0).x;
}
// The generated entries call this once per receiver, before `shade`.
fn rl_lights_prepare(world:vec3<f32>) {
    if rl_light_frame.info.y==0u { return; }
    let proj=rl_light_frame.proj;
    let v=rl_light_frame.view*vec4<f32>(world,1.0);
    let depth=-v.z;
    if !(depth>proj.z) { return; }
    let ndc=vec2<f32>(proj.x*v.x,proj.y*v.y)/depth;
    if any(abs(ndc)>vec2<f32>(1.0)) { return; }
    let tx=min(u32((ndc.x*0.5+0.5)*f32(LC_X)),LC_X-1u);
    let ty=min(u32((0.5-ndc.y*0.5)*f32(LC_Y)),LC_Y-1u);
    let tz=min(u32(max(log2(depth/proj.z)*proj.w,0.0)),LC_Z-1u);
    let c=(tz*LC_Y+ty)*LC_X+tx;
    rl_light_list=vec3<u32>(c,rl_cluster_word(c,0u)&0xffffu,1u);
}
fn rl_light_total() -> u32 {
    return select(lighting.count,rl_light_list.y,rl_light_list.z!=0u);
}
// The global light index of the receiver's i-th light.
fn rl_light_index(i:u32) -> u32 {
    if rl_light_list.z==0u { return i; }
    return (rl_cluster_word(rl_light_list.x,1u+(i>>1u))>>((i&1u)*16u))&0xffffu;
}
// Pooled map `layer`'s world-to-clip matrix (reversed depth: near 1, far 0).
fn rl_local_matrix(layer:u32) -> mat4x4<f32> {
    return rl_light_frame.shadows[layer];
}
// A shadowed local light's visibility from its pooled maps: a spot light's one view or a
// point light's cube face, compared on the receiver's own plane (as hard_shadow does) and
// filtered by a 3x3 bilinear PCF (16 loads). Outside every map is lit.
fn rl_local_shadow_map(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    // kind.w: pooled slot + 1 in the low 16 bits; bit 16 marks a six-face (cube) slot.
    let slot=(light.kind.w&0xffffu)-1u;
    if slot>=LC_SLOTS { return 1.0; }
    let biased=fma(l,vec3<f32>(receiver.epsilon),shadow_origin(receiver,l));
    var layer=slot*6u;
    if (light.kind.w&0x10000u)!=0u {
        let d=biased-light.position_range.xyz;
        let a=abs(d);
        if a.x>=a.y && a.x>=a.z { layer+=select(1u,0u,d.x>=0.0); }
        else if a.y>=a.z { layer+=select(3u,2u,d.y>=0.0); }
        else { layer+=select(5u,4u,d.z>=0.0); }
    }
    let m=rl_local_matrix(layer);
    let clip=m*vec4<f32>(biased,1.0);
    if clip.w<=0.0 { return 1.0; }
    let ndc=clip.xyz/clip.w;
    if any(abs(ndc.xy)>vec2<f32>(1.0)) || ndc.z<0.0 || ndc.z>1.0 { return 1.0; }
    let extent=vec2<f32>(textureDimensions(rl_local_depth));
    let geometric=receiver.geometric;
    let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(geometric.x)>0.9);
    let tangent=unit_normal(cross(geometric,axis));
    let bitangent=cross(geometric,tangent);
    let dx=m*vec4<f32>(tangent,0.0);
    let dy=m*vec4<f32>(bitangent,0.0);
    let plane=cross(dx.xyz-ndc*dx.w,dy.xyz-ndc*dy.w);
    // Depth change per NDC unit on the receiver's plane, limited at grazing incidence to
    // a 16:1 slope so PCF taps never compare against a near-vertical extrapolation.
    var slope=vec2<f32>(0.0);
    if plane.z!=0.0 { slope=clamp(-plane.xy/plane.z,vec2<f32>(-16.0),vec2<f32>(16.0)); }
    // Half a texel of the plane's own slope bounds the stored depth's rasterization offset.
    let texel=2.0/extent;
    let bias=0.5*dot(abs(slope),texel)+1e-6;
    let t=(ndc.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5))*extent-vec2<f32>(0.5);
    let origin=vec2<i32>(floor(t));
    let f=t-floor(t);
    let size=vec2<i32>(extent);
    var lit=0.0;
    for(var j=0;j<4;j+=1) {
        for(var i=0;i<4;i+=1) {
            let p=clamp(origin+vec2<i32>(i-1,j-1),vec2<i32>(0),size-vec2<i32>(1));
            let centre=((vec2<f32>(p)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,centre-ndc.xy)+bias;
            let stored=textureLoad(rl_local_depth,p,i32(layer),0);
            lit+=select(0.0,rl_pcf_weight(i,f.x)*rl_pcf_weight(j,f.y),reference>=stored);
        }
    }
    return lit/9.0;
}
// Separable 3x3 bilinear PCF over a 4x4 texel block: (1-f, 1, 1, f), summing to 3.
fn rl_pcf_weight(i:i32, f:f32) -> f32 {
    if i==0 { return 1.0-f; }
    if i==3 { return f; }
    return 1.0;
}
