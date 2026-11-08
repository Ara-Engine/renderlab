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
// LC2 deterministic sampling helpers, shared by the ray and map variants.
fn rl_local_hash(value:u32) -> u32 {
    var x=value; x=(x^(x>>16u))*0x7feb352du; x=(x^(x>>15u))*0x846ca68bu;
    return x^(x>>16u);
}
fn rl_local_disk(i:u32,n:u32) -> vec2<f32> {
    let r=sqrt((f32(i)+0.5)/f32(n));
    let phi=f32(i)*2.399963229728653;
    return r*vec2<f32>(cos(phi),sin(phi));
}
fn rl_local_face(d:vec3<f32>) -> u32 {
    let a=abs(d);
    if a.x>=a.y && a.x>=a.z { return select(1u,0u,d.x>=0.0); }
    if a.y>=a.z { return select(3u,2u,d.y>=0.0); }
    return select(5u,4u,d.z>=0.0);
}
// Pooled map `layer`'s world-to-clip matrix (reversed depth: near 1, far 0).
fn rl_local_matrix(layer:u32) -> mat4x4<f32> {
    return rl_light_frame.shadows[layer];
}
// A shadowed local light's visibility from its pooled maps: a spot light's one view or a
// point light's cube face, compared on the receiver's own plane (as hard_shadow does) and
// filtered with blocker-search PCF for positive radii, 3x3 bilinear PCF (16 loads)
// at zero radius. Outside every map is lit.
fn rl_local_shadow_map(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    // kind.w: pooled slot + 1 in the low 16 bits; bit 16 marks a six-face (cube) slot.
    let slot=(light.kind.w&0xffffu)-1u;
    if slot>=LC_SLOTS { return 1.0; }
    let biased=fma(l,vec3<f32>(receiver.epsilon),shadow_origin(receiver,l));
    var layer=slot*6u;
    if (light.kind.w&0x10000u)!=0u {
        let d=biased-light.position_range.xyz;
        layer+=rl_local_face(d);
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
    // LC2 PCSS: blocker search in linear light-view depth, then contact-hardening PCF.
    // The 96-texel footprint bound caps map support, never drops a light. Zero radius
    // retains LC1's deterministic hard-edge antialiasing below.
    if light.source.x>0.0 {
        let near=rl_light_frame.far.y;
        let far=max(light.position_range.w,select(1000.0,2.0*near,light.position_range.w>0.0));
        let projection=vec2<f32>(length(vec3<f32>(m[0].x,m[1].x,m[2].x)),length(vec3<f32>(m[0].y,m[1].y,m[2].y)));
        let source=0.5*projection*light.source.x*extent;
        let search=min(source*max(1.0/near-1.0/clip.w,0.0),vec2<f32>(96.0));
        var blocker_sum=0.0;
        var blockers=0.0;
        for(var k=0u;k<12u;k+=1u) {
            var offset=vec2<f32>(0.0);
            if k>0u { offset=rl_local_disk(k-1u,11u)*search; }
            let tap=t+offset;
            let result=rl_local_tap(light,receiver,biased,layer,ndc,slope,bias,tap,near,far);
            if result.x<0.5 && result.y>0.0 { blocker_sum+=result.y; blockers+=1.0; }
        }
        if blockers==0.0 { return 1.0; }
        let blocker=blocker_sum/blockers;
        let penumbra=min(source*max(clip.w-blocker,0.0)/max(blocker*clip.w,1e-6),vec2<f32>(96.0));
        var soft=0.0;
        for(var k=0u;k<24u;k+=1u) {
            let tap=t+rl_local_disk(k,24u)*max(penumbra,vec2<f32>(0.5));
            let base=floor(tap);
            let fraction=tap-base;
            for(var j=0u;j<2u;j+=1u) {
                for(var i=0u;i<2u;i+=1u) {
                    let weight=select(1.0-fraction.x,fraction.x,i==1u)*select(1.0-fraction.y,fraction.y,j==1u);
                    soft+=weight*rl_local_tap(light,receiver,biased,layer,ndc,slope,bias,base+vec2<f32>(f32(i),f32(j)),near,far).x;
                }
            }
        }
        return soft/24.0;
    }
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

// A PCSS tap's visibility and blocker depth along the original face's forward axis.
// Cube taps crossing a face edge reproject into their neighbour, rather than clamp and
// smear the face edge. The comparison stays on the same world receiver plane.
fn rl_local_tap(light:Light,receiver:Receiver,biased:vec3<f32>,layer:u32,ndc:vec3<f32>,slope:vec2<f32>,bias:f32,tap:vec2<f32>,near:f32,far:f32) -> vec2<f32> {
    let extent=vec2<f32>(textureDimensions(rl_local_depth));
    let size=vec2<i32>(extent);
    let m=rl_local_matrix(layer);
    let tap_ndc=((tap+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
    var use_layer=layer;
    var coordinate=vec2<i32>(tap);
    var reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
    var depth_scale=1.0;
    if (light.kind.w&0x10000u)!=0u && any(abs(tap_ndc)>vec2<f32>(1.0)) {
        let right=vec3<f32>(m[0].x,m[1].x,m[2].x);
        let up=vec3<f32>(m[0].y,m[1].y,m[2].y);
        let forward=vec3<f32>(m[0].w,m[1].w,m[2].w);
        let direction=forward+right*(tap_ndc.x/dot(right,right))+up*(tap_ndc.y/dot(up,up));
        use_layer=((light.kind.w&0xffffu)-1u)*6u+rl_local_face(direction);
        let neighbour=rl_local_matrix(use_layer);
        let normal=receiver.geometric;
        let denominator=dot(normal,direction);
        if abs(denominator)<1e-6 { return vec2<f32>(1.0,0.0); }
        let distance=dot(normal,biased-light.position_range.xyz)/denominator;
        if distance<=0.0 { return vec2<f32>(1.0,0.0); }
        let world=light.position_range.xyz+direction*distance;
        let clip=neighbour*vec4<f32>(world,1.0);
        let n=clip.xyz/clip.w;
        coordinate=vec2<i32>(floor((n.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5))*extent));
        reference=n.z+bias;
        let other_forward=vec3<f32>(neighbour[0].w,neighbour[1].w,neighbour[2].w);
        depth_scale=dot(forward,direction)/dot(other_forward,direction);
    }
    coordinate=clamp(coordinate,vec2<i32>(0),size-vec2<i32>(1));
    let stored=textureLoad(rl_local_depth,coordinate,i32(use_layer),0);
    let depth=near*far/max(near+stored*(far-near),1e-10);
    return vec2<f32>(select(0.0,1.0,reference>=stored),depth*depth_scale);
}
