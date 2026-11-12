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
// Disk rays per soft local light for this receiver on ray-traced stages (0: the ordinary
// four). rl_local_budget raises it where the resolve restarts the receiver's history;
// reflection hits are shaded before it is set and keep four.
var<private> rl_local_rays:u32;

fn rl_cluster_word(c:u32, k:u32) -> u32 {
    return textureLoad(rl_cluster_lists,vec2<u32>((c%LC_ROW)*LC_STRIDE+k,c/LC_ROW),0).x;
}
// The generated entries call this once per receiver, before `shade`.
fn rl_lights_prepare(world:vec3<f32>) {
    rl_local_budget(world);
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
// U-RTVIS: a PCSS filter tap whose 2x2 texels lie inside its face compares them inline, against
// the same receiver-plane reference rl_local_tap computes and in its order: the same visibility
// bits, without rl_local_tap's per-call matrix, tile and cube-edge work. src/light_cluster.rs turns these off for measurement
// (RENDERLAB_LOCAL_GATHER=0: four rl_local_tap calls a tap; RENDERLAB_LOCAL_PCF=skip: no filter).
const RL_LOCAL_GATHER:bool=true;
const RL_LOCAL_SKIP:bool=false;
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
    let extent=vec2<f32>(rl_light_frame.tiles[slot].z);
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
        let far=max(select(1000.0,light.position_range.w,light.position_range.w>0.0),2.0*near);
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
        if RL_LOCAL_SKIP { return 0.5; }
        let blocker=blocker_sum/blockers;
        let penumbra=min(source*max(clip.w-blocker,0.0)/max(blocker*clip.w,1e-6),vec2<f32>(96.0));
        var soft=0.0;
        for(var k=0u;k<24u;k+=1u) {
            let tap=t+rl_local_disk(k,24u)*max(penumbra,vec2<f32>(0.5));
            let base=floor(tap);
            let fraction=tap-base;
            let corner=vec2<i32>(base);
            if RL_LOCAL_GATHER && all(corner>=vec2<i32>(0)) && all(corner+vec2<i32>(1)<vec2<i32>(extent)) {
                // The 2x2 texels lie inside this face: rl_local_tap's comparison without its
                // per-call matrix, tile and cube-edge work.
                let origin=corner+vec2<i32>(rl_light_frame.tiles[slot].xy);
                for(var j=0u;j<2u;j+=1u) {
                    for(var i=0u;i<2u;i+=1u) {
                        let weight=select(1.0-fraction.x,fraction.x,i==1u)*select(1.0-fraction.y,fraction.y,j==1u);
                        let coordinate=corner+vec2<i32>(i32(i),i32(j));
                        let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
                        let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
                        let stored=textureLoad(rl_local_depth,origin+vec2<i32>(i32(i),i32(j)),i32(layer%6u),0);
                        soft+=weight*select(0.0,1.0,reference>=stored);
                    }
                }
                continue;
            }
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
    let near=rl_light_frame.far.y;
    let far=max(select(1000.0,light.position_range.w,light.position_range.w>0.0),2.0*near);
    // Four gathers read the same 4x4 hard-PCF texels. Compare each against its own
    // receiver-plane depth and keep the original row-major accumulation order.
    // Face/tile edges retain the original reprojecting path below.
    let first=origin-vec2<i32>(1);
    if all(first>=vec2<i32>(0)) && all(first+vec2<i32>(3)<vec2<i32>(extent)) {
        let atlas=vec2<f32>(textureDimensions(rl_local_depth));
        let tile=rl_light_frame.tiles[slot];
        // Fixed components keep the 4x4 PCF block in registers. Preserve the
        // original row-major comparisons and accumulation (including each weight).
        var gathered=0.0;
        let centre0=(vec2<f32>(first+vec2<i32>(0,0))+tile.xy+vec2<f32>(1.0))/atlas;
        let left0=textureGather(rl_local_depth,dfg_sampler,centre0,i32(layer%6u));
        let right0=textureGather(rl_local_depth,dfg_sampler,centre0+vec2<f32>(2.0/atlas.x,0.0),i32(layer%6u));
        {
            let coordinate=origin+vec2<i32>(0-1,0-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left0.w);
            gathered+=visibility*rl_pcf_weight(0,f.x)*rl_pcf_weight(0,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(1-1,0-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left0.z);
            gathered+=visibility*rl_pcf_weight(1,f.x)*rl_pcf_weight(0,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(2-1,0-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right0.w);
            gathered+=visibility*rl_pcf_weight(2,f.x)*rl_pcf_weight(0,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(3-1,0-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right0.z);
            gathered+=visibility*rl_pcf_weight(3,f.x)*rl_pcf_weight(0,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(0-1,1-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left0.x);
            gathered+=visibility*rl_pcf_weight(0,f.x)*rl_pcf_weight(1,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(1-1,1-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left0.y);
            gathered+=visibility*rl_pcf_weight(1,f.x)*rl_pcf_weight(1,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(2-1,1-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right0.x);
            gathered+=visibility*rl_pcf_weight(2,f.x)*rl_pcf_weight(1,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(3-1,1-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right0.y);
            gathered+=visibility*rl_pcf_weight(3,f.x)*rl_pcf_weight(1,f.y);
        }
        let centre2=(vec2<f32>(first+vec2<i32>(0,2))+tile.xy+vec2<f32>(1.0))/atlas;
        let left2=textureGather(rl_local_depth,dfg_sampler,centre2,i32(layer%6u));
        let right2=textureGather(rl_local_depth,dfg_sampler,centre2+vec2<f32>(2.0/atlas.x,0.0),i32(layer%6u));
        {
            let coordinate=origin+vec2<i32>(0-1,2-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left2.w);
            gathered+=visibility*rl_pcf_weight(0,f.x)*rl_pcf_weight(2,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(1-1,2-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left2.z);
            gathered+=visibility*rl_pcf_weight(1,f.x)*rl_pcf_weight(2,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(2-1,2-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right2.w);
            gathered+=visibility*rl_pcf_weight(2,f.x)*rl_pcf_weight(2,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(3-1,2-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right2.z);
            gathered+=visibility*rl_pcf_weight(3,f.x)*rl_pcf_weight(2,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(0-1,3-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left2.x);
            gathered+=visibility*rl_pcf_weight(0,f.x)*rl_pcf_weight(3,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(1-1,3-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=left2.y);
            gathered+=visibility*rl_pcf_weight(1,f.x)*rl_pcf_weight(3,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(2-1,3-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right2.x);
            gathered+=visibility*rl_pcf_weight(2,f.x)*rl_pcf_weight(3,f.y);
        }
        {
            let coordinate=origin+vec2<i32>(3-1,3-1);
            let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
            let reference=ndc.z+dot(slope,tap_ndc-ndc.xy)+bias;
            let visibility=select(0.0,1.0,reference>=right2.y);
            gathered+=visibility*rl_pcf_weight(3,f.x)*rl_pcf_weight(3,f.y);
        }
        return gathered/9.0;
    }
    var lit=0.0;
    for(var j=0;j<4;j+=1) {
        for(var i=0;i<4;i+=1) {
            let tap=vec2<f32>(origin+vec2<i32>(i-1,j-1));
            let visibility=rl_local_tap(light,receiver,biased,layer,ndc,slope,bias,tap,near,far).x;
            lit+=visibility*rl_pcf_weight(i,f.x)*rl_pcf_weight(j,f.y);
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
    let tile=rl_light_frame.tiles[layer/6u];
    let extent=vec2<f32>(tile.z);
    let size=vec2<i32>(extent);
    let m=rl_local_matrix(layer);
    // Compare on the texel actually loaded, including fractional blocker-search taps.
    // Otherwise a sloping receiver can be mistaken for its own blocker.
    var coordinate=vec2<i32>(floor(tap+vec2<f32>(0.5)));
    let tap_ndc=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
    var use_layer=layer;
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
        coordinate=clamp(coordinate,vec2<i32>(0),size-vec2<i32>(1));
        let centre=((vec2<f32>(coordinate)+vec2<f32>(0.5))/extent-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
        let other_forward=vec3<f32>(neighbour[0].w,neighbour[1].w,neighbour[2].w);
        let other_right=vec3<f32>(neighbour[0].x,neighbour[1].x,neighbour[2].x);
        let other_up=vec3<f32>(neighbour[0].y,neighbour[1].y,neighbour[2].y);
        let ray=other_forward+other_right*(centre.x/dot(other_right,other_right))+other_up*(centre.y/dot(other_up,other_up));
        let denom=dot(normal,ray);
        if abs(denom)<1e-6 { return vec2<f32>(1.0,0.0); }
        let hit_t=dot(normal,biased-light.position_range.xyz)/denom;
        if hit_t<=0.0 { return vec2<f32>(1.0,0.0); }
        let hit=neighbour*vec4<f32>(light.position_range.xyz+ray*hit_t,1.0);
        reference=hit.z/hit.w+bias;
        depth_scale=dot(forward,ray)/dot(other_forward,ray);
    }
    coordinate=clamp(coordinate,vec2<i32>(0),size-vec2<i32>(1));
    let stored=textureLoad(rl_local_depth,coordinate+vec2<i32>(tile.xy),i32(use_layer%6u),0);
    let depth=near*far/max(near+stored*(far-near),1e-10);
    return vec2<f32>(select(0.0,1.0,reference>=stored),depth*depth_scale);
}
