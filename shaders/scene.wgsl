struct Camera { view_projection: mat4x4<f32>, view_mode: vec4<u32> };
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> models: array<mat4x4<f32>>;
struct NormalTransform { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, info:vec4<f32> };
@group(1) @binding(1) var<storage, read> normals: array<NormalTransform>;
struct Material { base_color:vec4<f32>, metallic:f32, roughness:f32, double_sided:u32, pad:u32 };
@group(2) @binding(0) var<uniform> material:Material;
@group(2) @binding(1) var base_color:texture_2d<f32>;
@group(2) @binding(2) var base_sampler:sampler;
@group(2) @binding(3) var metallic_roughness:texture_2d<f32>;
@group(2) @binding(4) var mr_sampler:sampler;
// Frustum visibility assumes this camera * model * original bounded position.
// Position-displacing reloads must use Visibility::Unculled. Layout checks cannot certify bounds.
const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) slot: u32,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) @interpolate(flat) winding:f32,
    @location(4) world:vec3<f32>,
};
fn unit_normal(n:vec3<f32>) -> vec3<f32> {
    let scale=max(max(abs(n.x),abs(n.y)),abs(n.z));
    let scaled=n/max(scale,1e-30);
    return scaled*inverseSqrt(max(dot(scaled,scaled),1e-30));
}
fn world_position(position:vec3<f32>, slot:u32) -> vec4<f32> {
    return models[slot] * vec4<f32>(position, 1.0);
}
@vertex fn vs(input: VertexInput) -> VertexOutput {
    let normal = normals[input.slot];
    var output: VertexOutput;
    let world = world_position(input.position, input.slot);
    output.position = camera.view_projection * world;
    output.world = world.xyz;
    output.color = input.color;
    output.uv = input.uv;
    output.normal = unit_normal(mat3x3<f32>(normal.x.xyz,normal.y.xyz,normal.z.xyz)*unit_normal(input.normal));
    output.winding = normal.info.x;
    return output;
}
@fragment fn fs(input: VertexOutput, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {
    // Cull by source winding so mirrored slots remain in their original equal-depth order.
    if material.double_sided == 0u && front != (input.winding > 0.0) { discard; }
    let color=textureSample(base_color,base_sampler,input.uv);
    let mr=textureSample(metallic_roughness,mr_sampler,input.uv);
    var result=material.base_color.rgb*input.color.rgb*color.rgb*TINT;
    switch camera.view_mode.x {
        case 1u: {result=unit_normal(input.normal)*0.5+vec3<f32>(0.5);}
        case 2u: {result=vec3<f32>(material.metallic*mr.b);}
        case 3u: {result=vec3<f32>(material.roughness*mr.g);}
        default: {}
    }
    return vec4<f32>(result,1.0);
}

// Group 0-2 retain the diagnostic ABI. Only fs_lit uses group 3.
struct LightingUniform { eye:vec3<f32>, count:u32, environment:vec3<f32>, exposure:f32 };
struct Light { position_range:vec4<f32>, travel_outer:vec4<f32>, rgb_inner:vec4<f32>, kind:vec4<u32> };
@group(3) @binding(0) var<uniform> lighting:LightingUniform;
@group(3) @binding(1) var<storage,read> lights:array<Light>;
@group(3) @binding(2) var dfg:texture_2d<f32>;
@group(3) @binding(3) var dfg_sampler:sampler;
struct ShadowView {
    world_to_clip:mat4x4<f32>,
    // Directional camera-depth interval or local radial interval; normalized and world bias.
    coverage_bias:vec4<f32>,
    camera_depth:vec4<f32>,
    // Reserved for texel-scaled bias experiments; nearest lookup does not read this.
    texel:vec4<f32>,
};
@group(3) @binding(4) var shadow_depth:texture_depth_2d_array;
@group(3) @binding(5) var<storage,read> shadow_views:array<ShadowView>;
@group(0) @binding(0) var<uniform> shadow_view:ShadowView;
struct ShadowVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(flat) winding:f32,
};
@vertex fn vs_shadow(@location(0) position:vec3<f32>, @builtin(instance_index) slot:u32) -> ShadowVertex {
    var output:ShadowVertex;
    output.position=shadow_view.world_to_clip*world_position(position,slot);
    output.winding=normals[slot].info.x;
    return output;
}
@fragment fn fs_shadow(input:ShadowVertex, @builtin(front_facing) front:bool) {
    if material.double_sided==0u && front!=(input.winding>0.0) { discard; }
}
fn hard_shadow(light:Light, world:vec3<f32>, geometric:vec3<f32>, l:vec3<f32>) -> f32 {
    if light.kind.z==0u { return 1.0; }
    var layer=light.kind.y;
    let normal=select(-geometric,geometric,dot(geometric,l)>=0.0);
    let biased_world=world+normal*shadow_views[layer].coverage_bias.w;
    if light.kind.x==0u {
        let depth=dot(shadow_views[layer].camera_depth,vec4<f32>(world,1.0));
        var found=false;
        for(var view=0u;view<light.kind.z;view+=1u) {
            let coverage=shadow_views[light.kind.y+view].coverage_bias;
            if depth>=coverage.x && depth<=coverage.y { layer=light.kind.y+view; found=true; break; }
        }
        if !found { return 1.0; }
    } else {
        let delta=biased_world-light.position_range.xyz;
        let radial=length(world-light.position_range.xyz);
        let coverage=shadow_views[layer].coverage_bias;
        if radial<coverage.x || radial>coverage.y { return 1.0; }
        if light.kind.x==1u {
            let a=abs(delta);
            if a.x>=a.y && a.x>=a.z { layer+=select(1u,0u,delta.x>=0.0); }
            else if a.y>=a.z { layer+=select(3u,2u,delta.y>=0.0); }
            else { layer+=select(5u,4u,delta.z>=0.0); }
        }
    }
    let view=shadow_views[layer];
    let clip=view.world_to_clip*vec4<f32>(biased_world,1.0);
    if clip.w<=0.0 { return 1.0; }
    let ndc=clip.xyz/clip.w;
    if any(abs(ndc.xy)>vec2<f32>(1.0)) || ndc.z<0.0 || ndc.z>1.0 { return 1.0; }
    let extent=vec2<i32>(textureDimensions(shadow_depth));
    let uv=ndc.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    let pixel=clamp(vec2<i32>(uv*vec2<f32>(extent)),vec2<i32>(0),extent-vec2<i32>(1));
    let stored=textureLoad(shadow_depth,pixel,i32(layer),0);
    // Compare at the sampled texel centre on this receiver's geometric plane.
    // A constant bias alone cannot cover the nearest texel's slope-dependent depth error.
    let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(geometric.x)>0.9);
    let tangent=unit_normal(cross(geometric,axis));
    let bitangent=cross(geometric,tangent);
    let dx=view.world_to_clip*vec4<f32>(tangent,0.0);
    let dy=view.world_to_clip*vec4<f32>(bitangent,0.0);
    let plane=cross(dx.xyz-ndc*dx.w,dy.xyz-ndc*dy.w);
    let sample_ndc=((vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(extent)-vec2<f32>(0.5))*vec2<f32>(2.0,-2.0);
    var reference=ndc.z;
    if plane.z!=0.0 {
        reference-=dot(plane.xy,sample_ndc-ndc.xy)/plane.z;
        // Bounds subpixel rounding of the receiver's own plane (at least 4 subpixel bits),
        // not depth differences from non-planar neighbouring triangles.
        reference-=dot(abs(plane.xy),2.0/vec2<f32>(extent))/(16.0*abs(plane.z));
    }
    return select(0.0,1.0,reference-view.coverage_bias.z<=stored);
}
// Presentation uses its own layout and entry points in this same atomic module.
@group(0) @binding(1) var exposed_hdr:texture_2d<f32>;
const PI:f32=3.141592653589793;
// Multiply in f32 mantissa/exponent form so large physical units paired with
// small exposure (or the reverse) do not overflow before a finite exposed result.
// All contributions are nonnegative: saturating each exposed contribution and
// their sum at the storage ceiling equals saturating the final exposed sum.
fn exposed_product(a:vec3<f32>, b:vec3<f32>) -> vec3<f32> {
    let ap=frexp(a);
    let bp=frexp(b);
    let ep=frexp(vec3<f32>(lighting.exposure));
    let mantissa=ap.fract*bp.fract*ep.fract;
    // Three nonzero mantissas have product >= 1/8; exponent >=20 is
    // already above 65504. The low clamp is below binary16's minimum.
    let exponent=clamp(ap.exp+bp.exp+ep.exp,vec3<i32>(-126),vec3<i32>(20));
    return min(ldexp(mantissa,exponent),vec3<f32>(65504.0));
}
fn brdf(n:vec3<f32>, v:vec3<f32>, l:vec3<f32>, base:vec3<f32>, metal:f32, rough:f32) -> vec3<f32> {
    let nl=max(dot(n,l),0.0);
    let nv=max(dot(n,v),1e-7);
    if dot(l+v,l+v)<1e-20 {return base*((1.0-metal)/PI)*nl;}
    let h=unit_normal(l+v);
    let nh=max(dot(n,h),0.0);
    let vh=clamp(dot(v,h),0.0,1.0);
    let alpha=max(rough*rough,1e-4);
    let a2=alpha*alpha;
    // |N x H|^2 + alpha^2 (N dot H)^2 preserves the peak at alpha=1e-4 in f32.
    let c=cross(n,h);
    let denom=max(dot(c,c)+a2*nh*nh,1e-20);
    let d=a2/(PI*denom*denom);
    let visibility=0.5/max(nl*sqrt(a2+(1.0-a2)*nv*nv)+nv*sqrt(a2+(1.0-a2)*nl*nl),1e-20);
    let f0=mix(vec3<f32>(0.04),base,metal);
    let fresnel=f0+(vec3<f32>(1.0)-f0)*pow(1.0-vh,5.0);
    // Declared initial diffuse model: plain nonmetallic Lambert.
    return (base*((1.0-metal)/PI)+d*visibility*fresnel)*nl;
}
@fragment fn fs_lit(input:VertexOutput, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {
    let source_front=front==(input.winding>0.0);
    // Derivatives evaluated uniformly before any source-winding discard.
    let geometric=unit_normal(cross(unit_normal(dpdx(input.world)),unit_normal(dpdy(input.world))));
    let sampled=textureSample(base_color,base_sampler,input.uv);
    let mr=textureSample(metallic_roughness,mr_sampler,input.uv);
    if material.double_sided==0u && !source_front { discard; }
    var n=unit_normal(input.normal);
    if dot(n,n)<0.5 { n=geometric; }
    if dot(n,n)<0.5 { n=vec3<f32>(0.0,0.0,1.0); }
    if material.double_sided!=0u && !source_front { n=-n; }
    var v=unit_normal(lighting.eye-input.world);
    if dot(v,v)<0.5 { v=n; }
    let base=clamp(material.base_color.rgb*input.color.rgb*sampled.rgb*TINT,vec3<f32>(0.0),vec3<f32>(1.0));
    let metal=clamp(material.metallic*mr.b,0.0,1.0);
    let rough=clamp(material.roughness*mr.g,0.0,1.0);
    let uv=(vec2<f32>(clamp(dot(n,v),0.0,1.0),rough)*127.0+vec2<f32>(0.5))/128.0;
    let ab=textureSample(dfg,dfg_sampler,uv).rg;
    let f0=mix(vec3<f32>(0.04),base,metal);
    var exposed=exposed_product(lighting.environment,base*(1.0-metal)+f0*ab.x+vec3<f32>(ab.y));
    for(var i=0u;i<lighting.count;i+=1u) {
        let light=lights[i];
        var l=-light.travel_outer.xyz;
        var attenuation=1.0;
        if light.kind.x!=0u {
            // Scale coordinates before subtraction, avoiding overflow for opposite finite inputs.
            let half_delta=light.position_range.xyz*0.5-input.world*0.5;
            let scale=max(max(abs(half_delta.x),abs(half_delta.y)),abs(half_delta.z));
            l=unit_normal(half_delta);
            let scaled=half_delta/max(scale,1e-30);
            let distance=scale*min(2.0*sqrt(dot(scaled,scaled)),1e37/max(scale,1.0));
            let inverse=1.0/max(distance,0.01);
            attenuation=inverse*inverse;
            if light.position_range.w>0.0 {
                let ratio=min(distance/light.position_range.w,1.0);
                let window=max(1.0-ratio*ratio*ratio*ratio,0.0);
                attenuation*=window*window;
            }
            if light.kind.x==2u {
                let cone=clamp((dot(-l,light.travel_outer.xyz)-light.travel_outer.w)/max(light.rgb_inner.w-light.travel_outer.w,1e-4),0.0,1.0);
                attenuation*=cone*cone;
            }
        }
        attenuation*=hard_shadow(light,input.world,geometric,l);
        exposed=min(exposed+exposed_product(light.rgb_inner.xyz,brdf(n,v,l,base,metal,rough)*attenuation),vec3<f32>(65504.0));
    }
    return vec4<f32>(exposed,1.0);
}
@vertex fn vs_present(@builtin(vertex_index) id:u32) -> @builtin(position) vec4<f32> {
    let xy=vec2<f32>(f32((id<<1u)&2u),f32(id&2u));
    return vec4<f32>(xy*2.0-vec2<f32>(1.0),0.0,1.0);
}
@fragment fn fs_present(@builtin(position) position:vec4<f32>) -> @location(0) vec4<f32> {
    let value=textureLoad(exposed_hdr,vec2<i32>(position.xy),0).rgb;
    return vec4<f32>(value/(vec3<f32>(1.0)+value),1.0);
}
