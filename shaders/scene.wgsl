enable wgpu_per_vertex;
struct Camera { view_projection: mat4x4<f32>, view_mode: vec4<u32> };
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> models: array<mat4x4<f32>>;
struct NormalTransform { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, info:vec4<f32> };
@group(1) @binding(1) var<storage, read> normals: array<NormalTransform>;
struct Material { base_color:vec4<f32>, metallic:f32, roughness:f32, double_sided:u32, thin:u32, normal_scale:f32, normal_mapped:u32, normal_variance:u32, pad:u32 };
@group(2) @binding(0) var<uniform> material:Material;
@group(2) @binding(1) var base_color:texture_2d<f32>;
@group(2) @binding(2) var base_sampler:sampler;
@group(2) @binding(3) var metallic_roughness:texture_2d<f32>;
@group(2) @binding(4) var mr_sampler:sampler;
@group(2) @binding(5) var normal_map:texture_2d_array<f32>;
@group(2) @binding(6) var normal_sampler:sampler;
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
    @location(5) tangent:vec3<f32>,
    @location(6) @interpolate(flat) handedness:f32,
    @location(7) receiver_vertex:vec3<f32>,
};
// Fragment interpolants omit location 7: RasterTriangle consumes it per vertex.
struct RasterInput {
    @builtin(position) position:vec4<f32>,
    @location(0) color:vec4<f32>,
    @location(1) uv:vec2<f32>,
    @location(2) normal:vec3<f32>,
    @location(3) @interpolate(flat) winding:f32,
    @location(4) world:vec3<f32>,
    @location(5) tangent:vec3<f32>,
    @location(6) @interpolate(flat) handedness:f32,
};
// Engine-private receiver of one shaded pixel; every visibility variant reads it.
// geometric: the hit triangle's unit plane normal (either orientation). pixel: the
// shaded pixel. slot: the traced material slot. Epsilon and thinness are geometry/
// material declarations shared by every shading program, independent of its BRDF.
struct Receiver { world:vec3<f32>, geometric:vec3<f32>, pixel:vec2<i32>, slot:u32, epsilon:f32, thin:bool };
// These are the rasterizer's hit triangle, not neighbouring pixels or mesh bounds.
struct RasterTriangle {
    @location(7) @interpolate(per_vertex) vertices:array<vec3<f32>,3>,
};
// Vertex-coordinate precision scale, including translated and transformed triangles.
fn receiver_epsilon(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>)->f32 {
    let extent=max(abs(a),max(abs(b),abs(c)));
    let m=max(extent.x,max(extent.y,extent.z));
    let ulp=bitcast<f32>(bitcast<u32>(m)+1u)-m;
    return max(1e-5,16.0*ulp);
}
// Pin the barycentric contraction order; camera-ray contraction is pinned separately.
fn receiver_point(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,w:vec3<f32>)->vec3<f32> {
    return fma(c,vec3<f32>(w.z),fma(b,vec3<f32>(w.y),a*w.x));
}
fn raster_receiver(world:vec3<f32>,triangle:RasterTriangle,pixel:vec2<i32>,thin:bool)->Receiver {
    let a=triangle.vertices[0];let b=triangle.vertices[1];let c=triangle.vertices[2];
    // The rasterizer interpolates this point on the winning triangle, including
    // sample interpolation at 4x. Per-vertex inputs supply its precision scale.
    let normal=unit_normal(cross(unit_normal(b-a),unit_normal(c-a)));
    return Receiver(world,normal,pixel,0u,receiver_epsilon(a,b,c),thin);
}
// Opaque receivers stay on the viewed side even under backlighting. Thinness is
// declared per material, not inferred from double-sided shading or the BRDF.
fn shadow_origin(receiver:Receiver,l:vec3<f32>)->vec3<f32> {
    let toward=select(lighting.eye-receiver.world,l,receiver.thin);
    let normal=select(-receiver.geometric,receiver.geometric,dot(receiver.geometric,toward)>=0.0);
    return fma(normal,vec3<f32>(receiver.epsilon),receiver.world);
}
fn unit_normal(n:vec3<f32>) -> vec3<f32> {
    let scale=max(max(abs(n.x),abs(n.y)),abs(n.z));
    let scaled=n/max(scale,1e-30);
    return scaled*inverseSqrt(max(dot(scaled,scaled),1e-30));
}
// renderlab: affine vertices begin
fn world_position(position:vec3<f32>, slot:u32) -> vec4<f32> {
    return models[slot] * vec4<f32>(position, 1.0);
}
fn vertex_output(input: VertexInput) -> VertexOutput {
    let normal = normals[input.slot];
    var output: VertexOutput;
    let world = world_position(input.position, input.slot);
    output.position = camera.view_projection * world;
    output.world = world.xyz;
    output.receiver_vertex = world.xyz;
    output.color = input.color;
    output.uv = input.uv;
    output.normal = unit_normal(mat3x3<f32>(normal.x.xyz,normal.y.xyz,normal.z.xyz)*unit_normal(input.normal));
    output.winding = normal.info.x;
    output.tangent = vec3<f32>(0.0);
    output.handedness = 0.0;
    return output;
}
@vertex fn vs(input:VertexInput) -> VertexOutput { return vertex_output(input); }
@vertex fn vs_tangent(input:VertexInput, @location(5) tangent:vec4<f32>) -> VertexOutput {
    var output=vertex_output(input);
    let m=models[input.slot];
    let a=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let scale=max(max(max(abs(m[0].x),abs(m[0].y)),max(abs(m[0].z),abs(m[1].x))),max(max(abs(m[1].y),abs(m[1].z)),max(max(abs(m[2].x),abs(m[2].y)),abs(m[2].z))));
    let t=unit_normal((a*(1.0/max(scale,1e-30)))*tangent.xyz);
    output.tangent=unit_normal(t-output.normal*dot(output.normal,t));
    output.handedness=tangent.w*output.winding;
    return output;
}

// renderlab: affine vertices end
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

// Only lit shading programs and the engine's visibility kernels use group 3.
struct LightingUniform { eye:vec3<f32>, count:u32, environment:vec3<f32>, exposure:f32, map:vec4<f32> };
struct Light { position_range:vec4<f32>, travel_outer:vec4<f32>, rgb_inner:vec4<f32>, kind:vec4<u32>, source:vec4<f32> };
@group(3) @binding(0) var<uniform> lighting:LightingUniform;
@group(3) @binding(1) var<storage,read> lights:array<Light>;
@group(3) @binding(2) var dfg:texture_2d<f32>;
@group(3) @binding(3) var dfg_sampler:sampler;
@group(3) @binding(6) var environment_diffuse:texture_2d_array<f32>;
@group(3) @binding(7) var environment_specular:texture_2d_array<f32>;
@group(3) @binding(9) var environment_specular_second:texture_2d_array<f32>;
struct ShadowView {
    world_to_clip:mat4x4<f32>,
    // Directional camera-depth interval or local radial interval; normalized and world bias.
    coverage_bias:vec4<f32>,
    camera_depth:vec4<f32>,
    // x: planner texel extent; optional hardware path uses y for depth/metre.
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
// renderlab: direct visibility begin
// Visibility of `light` from the receiver along l, toward the light. 1 is unshadowed.
fn hard_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    let world=receiver.world;
    let geometric=receiver.geometric;
    if light.kind.z==0u { return 1.0; }
    var layer=light.kind.y;
    // Shadow-map comparison at the ray's positive epsilon near point. Raster's
    // receiver-plane texel correction below remains a sampling precision guard.
    let biased_world=fma(l,vec3<f32>(receiver.epsilon),shadow_origin(receiver,l));
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
        reference+=dot(abs(plane.xy),2.0/vec2<f32>(extent))/(16.0*abs(plane.z));
    }
    return select(0.0,1.0,reference>=stored);
}
// renderlab: direct visibility end
// Presentation uses its own layout and entry points in this same atomic module.
@group(0) @binding(1) var exposed_hdr:texture_2d<f32>;
const PI:f32=3.141592653589793;
// Multiply in f32 mantissa/exponent form so large physical units paired with
// small exposure (or the reverse) do not overflow before a finite exposed result.
// All contributions are nonnegative: saturating each exposed contribution and
// their sum at the storage ceiling equals saturating the final exposed sum.
fn exposed_product(a:vec3<f32>, b:vec3<f32>) -> vec3<f32> {
    // P-NRT fast path, bit-identical to the form below: power-of-two scaling keeps round-to-
    // nearest unchanged inside the normal range, so when a*b is a finite normal and the
    // exposed product is at least normal, (a*b)*exposure equals ldexp of the mantissa product
    // at the unclamped exponent; past 2^17 both saturate to 65504. Exact zeros stay zero.
    let e=lighting.exposure;
    let ab=a*b;let p=ab*e;
    let lowest=vec3<f32>(1.17549435e-38);
    let normal=(ab>=lowest)&(ab<=vec3<f32>(3.0e38))&(p>=lowest);
    let zero=(ab==vec3<f32>(0.0))&((a==vec3<f32>(0.0))|(b==vec3<f32>(0.0)));
    if all(normal|zero) && e>=1.17549435e-38 && e<=3.0e38 {return min(p,vec3<f32>(65504.0));}
    let ap=frexp(a);
    let bp=frexp(b);
    let ep=frexp(vec3<f32>(lighting.exposure));
    let mantissa=ap.fract*bp.fract*ep.fract;
    // Three nonzero mantissas have product >= 1/8; exponent >=20 is
    // already above 65504. The low clamp is below binary16's minimum.
    let exponent=clamp(ap.exp+bp.exp+ep.exp,vec3<i32>(-126),vec3<i32>(20));
    return min(ldexp(mantissa,exponent),vec3<f32>(65504.0));
}
// Metadata and caller yaw both move source to world. Sample with the inverse +Y rotation.
fn environment_direction(world:vec3<f32>) -> vec3<f32> {
    return vec3<f32>(lighting.map.x*world.x-lighting.map.y*world.z,world.y,lighting.map.y*world.x+lighting.map.x*world.z);
}
// Identical face selection and three-face corner support to the accepted reconstruction probe.
fn environment_face_uv(d:vec3<f32>) -> vec3<f32> {
    let a=abs(d);
    if a.x>=a.y && a.x>=a.z {
        if d.x>0.0 { return vec3<f32>(0.0,-d.z/a.x,-d.y/a.x); }
        return vec3<f32>(1.0,d.z/a.x,-d.y/a.x);
    }
    if a.y>=a.z {
        if d.y>0.0 { return vec3<f32>(2.0,d.x/a.y,d.z/a.y); }
        return vec3<f32>(3.0,d.x/a.y,-d.z/a.y);
    }
    if d.z>0.0 { return vec3<f32>(4.0,d.x/a.z,-d.y/a.z); }
    return vec3<f32>(5.0,-d.x/a.z,-d.y/a.z);
}
fn environment_face_direction(face:i32, uv:vec2<f32>) -> vec3<f32> {
    switch face {
        case 0: { return vec3<f32>(1.0,-uv.y,-uv.x); }
        case 1: { return vec3<f32>(-1.0,-uv.y,uv.x); }
        case 2: { return vec3<f32>(uv.x,1.0,uv.y); }
        case 3: { return vec3<f32>(uv.x,-1.0,-uv.y); }
        case 4: { return vec3<f32>(uv.x,-uv.y,1.0); }
        default: { return vec3<f32>(-uv.x,-uv.y,-1.0); }
    }
}
fn environment_remap(tex:texture_2d_array<f32>, face:i32, p:vec2<i32>, size:i32, mip:i32, bank:i32) -> vec3<f32> {
    if all(p>=vec2<i32>(0)) && all(p<vec2<i32>(size)) { return textureLoad(tex,p,bank+face,mip).rgb; }
    let d=environment_face_direction(face,2.0*(vec2<f32>(p)+vec2<f32>(0.5))/f32(size)-vec2<f32>(1.0));
    let next=environment_face_uv(d);
    let q=clamp(vec2<i32>((next.yz+vec2<f32>(1.0))*f32(size)*0.5),vec2<i32>(0),vec2<i32>(size-1));
    return textureLoad(tex,q,bank+i32(next.x),mip).rgb;
}
fn environment_tap(tex:texture_2d_array<f32>, face:i32, p:vec2<i32>, size:i32, mip:i32, bank:i32) -> vec3<f32> {
    let outside_x=p.x<0 || p.x>=size;
    let outside_y=p.y<0 || p.y>=size;
    if !(outside_x && outside_y) { return environment_remap(tex,face,p,size,mip,bank); }
    let q=clamp(p,vec2<i32>(0),vec2<i32>(size-1));
    var sum=vec3<f32>(0.0);
    // Share corner remapping without putting ordinary taps through a sample loop.
    for (var i=0; i<3; i++) {
        let point=vec2<i32>(select(q.x,p.x,i==1),select(q.y,p.y,i==2));
        let value=environment_remap(tex,face,point,size,mip,bank);
        if i==0 { sum=value; } else { sum+=value; }
    }
    return sum/3.0;
}
fn environment_bilinear(tex:texture_2d_array<f32>, d:vec3<f32>, mip:i32, bank:i32) -> vec3<f32> {
    let size=i32(textureDimensions(tex,mip).x);
    let face=environment_face_uv(d);
    let xy=(face.yz+vec2<f32>(1.0))*f32(size)*0.5-vec2<f32>(0.5);
    let p=vec2<i32>(floor(xy));
    let t=fract(xy);
    let a=environment_tap(tex,i32(face.x),p,size,mip,bank);
    let b=environment_tap(tex,i32(face.x),p+vec2<i32>(1,0),size,mip,bank);
    let c=environment_tap(tex,i32(face.x),p+vec2<i32>(0,1),size,mip,bank);
    let e=environment_tap(tex,i32(face.x),p+vec2<i32>(1,1),size,mip,bank);
    return (a*(1.0-t.x)+b*t.x)*(1.0-t.y)+(c*(1.0-t.x)+e*t.x)*t.y;
}
fn environment_level(d:vec3<f32>, level:i32) -> vec3<f32> {
    let legacy=lighting.map.w>0.0;
    let second=!legacy && level>=42;
    let mip=select(0,level,legacy);
    let bank=select((level-select(0,42,second))*6,0,legacy);
    if second { return environment_bilinear(environment_specular_second,d,mip,bank); }
    return environment_bilinear(environment_specular,d,mip,bank);
}
fn environment_legacy_lod(d:vec3<f32>, lod:f32) -> vec3<f32> {
    let x=clamp(lod,0.0,lighting.map.w-1.0);
    let lo=i32(floor(x));
    let hi=min(lo+1,i32(lighting.map.w)-1);
    var values:array<vec3<f32>,2>;
    for (var i=0; i<2; i++) { values[i]=environment_level(d,select(lo,hi,i==1)); }
    return values[0]*(1.0-fract(x))+values[1]*fract(x);
}
fn environment_radiance(d:vec3<f32>, roughness:f32) -> vec3<f32> {
    let r=clamp(roughness,0.0,1.0);
    // Choose levels before filtering and share the lookup site. Keep each path's
    // arithmetic order, including the unblended fully rough return.
    var lo=64;
    var hi=64;
    var t=0.0;
    var single=false;
    if lighting.map.w>0.0 {
        let x=clamp(r*(lighting.map.w-1.0),0.0,lighting.map.w-1.0);
        lo=i32(floor(x));
        hi=min(lo+1,i32(lighting.map.w)-1);
        t=fract(x);
    } else if r>=1.0 {
        single=true;
    } else {
        lo=min(i32(floor(64.0*sqrt(r))),63);
        hi=lo+1;
        let a=f32(lo)/64.0;
        let b=f32(lo+1)/64.0;
        let a2=a*a; let b2=b*b;
        let a4=a2*a2; let b4=b2*b2;
        let a8=a4*a4; let b8=b4*b4;
        let r2=r*r;
        t=clamp((r2*r2-a8)/(b8-a8),0.0,1.0);
    }
    // P-NRT: a zero blend weight (every mirror lookup) makes the second level's finite
    // texels contribute exactly zero, so its four taps are skipped; the sum is unchanged.
    let levels=select(2,1,single || t==0.0);
    var values:array<vec3<f32>,2>;
    for (var i=0; i<levels; i++) {
        values[i]=environment_level(d,select(lo,hi,i==1));
    }
    if single { return values[0]; }
    return values[0]*(1.0-t)+values[1]*t;
}
fn environment_product(energy:vec3<f32>, response:vec3<f32>) -> vec3<f32> {
    // P-NRT fast path, bit-identical as exposed_product's: every partial product a nonnegative
    // finite normal, so ((energy*response)*exposure)*intensity rounds as the mantissa form at
    // the unclamped exponent; past 2^17 both saturate. Exact zeros stay zero.
    let ex=lighting.exposure;let gain=lighting.map.z;
    let ab=energy*response;let abe=ab*ex;let p=abe*gain;
    let lowest=vec3<f32>(1.17549435e-38);let highest=vec3<f32>(3.0e38);
    let normal=(ab>=lowest)&(ab<=highest)&(abe>=lowest)&(abe<=highest)&(p>=lowest);
    let zero=(ab==vec3<f32>(0.0))&((energy==vec3<f32>(0.0))|(response==vec3<f32>(0.0)));
    if all(normal|zero) && ex>=1.17549435e-38 && ex<=3.0e38 && gain>=1.17549435e-38 && gain<=3.0e38 {
        return min(p,vec3<f32>(65504.0));
    }
    let a=frexp(energy);
    let b=frexp(response);
    let e=frexp(vec3<f32>(lighting.exposure));
    let i=frexp(vec3<f32>(lighting.map.z));
    // Four nonzero mantissas have product >= 1/16. Keep dim values until final 16f storage.
    let exponent=clamp(a.exp+b.exp+e.exp+i.exp,vec3<i32>(-126),vec3<i32>(21));
    return min(ldexp(a.fract*b.fract*e.fract*i.fract,exponent),vec3<f32>(65504.0));
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
    let f0=vec3<f32>(0.04)*(1.0-metal)+base*metal;
    let fresnel=f0+(vec3<f32>(1.0)-f0)*pow(1.0-vh,5.0);
    // Declared initial diffuse model: plain nonmetallic Lambert.
    return (base*((1.0-metal)/PI)+d*visibility*fresnel)*nl;
}
@vertex fn vs_present(@builtin(vertex_index) id:u32) -> @builtin(position) vec4<f32> {
    let xy=vec2<f32>(f32((id<<1u)&2u),f32(id&2u));
    return vec4<f32>(xy*2.0-vec2<f32>(1.0),0.0,1.0);
}
@fragment fn fs_present(@builtin(position) position:vec4<f32>) -> @location(0) vec4<f32> {
    let value=textureLoad(exposed_hdr,vec2<i32>(position.xy),0).rgb;
    return vec4<f32>(value/(vec3<f32>(1.0)+value),1.0);
}
