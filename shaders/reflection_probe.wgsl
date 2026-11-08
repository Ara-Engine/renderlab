// R0b test-only stages. Each secondary ray belongs to its traced primary pixel.
// No projections, hit matching, reservoirs, caches, denoising or history.
struct ProbeSettings { mode:u32, rays:u32, sample:u32, exposure:f32 };
@group(0) @binding(11) var<uniform> probe:ProbeSettings;
struct ProbeRecord {
    world:vec4<f32>, normal:vec4<f32>, geometric:vec4<f32>, uv:vec4<f32>,
    color:vec4<f32>, identity:vec4<u32>, direction:vec4<f32>, weight:vec4<f32>,
};
@group(0) @binding(12) var<storage,read_write> primary:array<ProbeRecord>;
@group(0) @binding(13) var<storage,read_write> secondary:array<ProbeRecord>;
@group(0) @binding(14) var<storage,read_write> incident:array<vec4<f32>>;
@group(0) @binding(10) var<storage,read> dfg_values:array<vec2<f32>>;

// Adapter services execute the existing user sources below. The probe exposes
// constant sky only; no production environment/GI implementation is replaced.
struct Material { base_color:vec4<f32>, metallic:f32, roughness:f32, double_sided:u32,
    thin:u32, normal_scale:f32, normal_mapped:u32, normal_variance:u32, pad:u32 };
struct Receiver { world:vec3<f32>, geometric:vec3<f32>, pixel:vec2<i32>, slot:u32, epsilon:f32, thin:bool };
struct LightingUniform {eye:vec3<f32>,count:u32,environment:vec3<f32>,exposure:f32,map:vec4<f32>};
struct Light {position_range:vec4<f32>,travel_outer:vec4<f32>,rgb_inner:vec4<f32>,kind:vec4<u32>};
var<private> lighting:LightingUniform;
var<private> lights:array<Light,1>;
var<private> suppress_specular:bool;
const TINT=vec3<f32>(1.0);
fn unit_normal(n:vec3<f32>)->vec3<f32> {
    let scale=max(max(abs(n.x),abs(n.y)),abs(n.z));let q=n/max(scale,1e-30);
    return q*inverseSqrt(max(dot(q,q),1e-30));
}
fn setup() {
    lighting=LightingUniform(frame.eye0.xyz,u32(frame.sun_value.w>0.0),frame.sky.rgb,probe.exposure,vec4<f32>(0.0));
    lights[0]=Light(vec4<f32>(0.0),vec4<f32>(-frame.sun.xyz,0.0),
        vec4<f32>(frame.sun_value.rgb*select(PI*sun_height()*(2.0-sun_height()),1.0,sun_height()==0.0),0.0),vec4<u32>(0u));
}
fn hard_shadow(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {
    let start=offset(receiver.world,receiver.geometric);var sum=0.0;
    let disk=basis(l);
    // Eight fixed disk nodes, identical at primary and secondary program calls.
    for(var k=0u;k<8u;k+=1u) {
        let h=(f32(k)+0.5)/8.0*sun_height();let phi=f32(k)*2.39996323;
        let r=sqrt(h*(2.0-h));let d=disk*vec3<f32>(r*cos(phi),r*sin(phi),1.0-h);
        sum+=select(1.0,0.0,occluded(start,d));
    }
    return sum/8.0;
}
fn ambient(n:vec3<f32>)->vec3<f32> {return PI*frame.sky.rgb;}
fn indirect_irradiance(input:SurfaceInput,n:vec3<f32>)->vec3<f32> {return ambient(n);}
fn environment_enabled()->bool {return false;}
fn environment_constant()->vec3<f32> {return select(frame.sky.rgb,vec3<f32>(0.0),suppress_specular);}
fn environment_reflection(d:vec3<f32>,rough:f32)->vec3<f32> {return environment_constant();}
// M7-ES1's inputs as the open environment above: the probe has no world radiance cache,
// so radiance is unoccluded and no environment lobes are separated (docs/m7-es1.md).
fn indirect_radiance(input:SurfaceInput,d:vec3<f32>,rough:f32)->vec3<f32> {return environment_constant();}
struct EnvironmentLobe { direction:vec3<f32>, sharpness:f32, illuminance:vec3<f32> };
fn environment_lobe_count()->u32 {return 0u;}
fn environment_lobe(input:SurfaceInput,i:u32)->EnvironmentLobe {return EnvironmentLobe(vec3<f32>(0.0,0.0,1.0),1.0,vec3<f32>(0.0));}
fn environment_lobe_reflection(input:SurfaceInput,i:u32,d:vec3<f32>,rough:f32)->vec3<f32> {return vec3<f32>(0.0);}
// M7-R1's inputs untraced: the probe traces its own rays (docs/m7-r1.md).
struct SpecularLobe { normal:vec3<f32>, alpha:f32, weight:f32 };
fn specular_incident(input:SurfaceInput)->vec4<f32> {return vec4<f32>(0.0);}
fn environment_product(a:vec3<f32>,b:vec3<f32>)->vec3<f32> {return exposed_product(a,b);}
fn environment_dfg(nv:f32,rough:f32)->vec2<f32> {
    let p=vec2<f32>(clamp(nv,0.0,1.0),rough)*127.0;
    let a=vec2<u32>(floor(p));let b=min(a+1u,vec2<u32>(127u));let f=fract(p);
    return mix(mix(dfg_values[a.y*128u+a.x],dfg_values[a.y*128u+b.x],f.x),
        mix(dfg_values[b.y*128u+a.x],dfg_values[b.y*128u+b.x],f.x),f.y);
}
fn sample_base_color(input:SurfaceInput)->vec4<f32> {return texture_sample(materials[input.receiver.slot].base_map,input.uv);}
fn sample_metallic_roughness(input:SurfaceInput)->vec4<f32> {return texture_sample(materials[input.receiver.slot].mr_map,input.uv);}
// trace() already applied the source normal map. Supply its result as the source
// normal with normal_mapped=0, avoiding a second application in material_normal.
fn sample_normal_map(input:SurfaceInput)->vec4<f32> {return vec4<f32>(0.5,0.5,1.0,1.0);}
fn specular_brdf(input:SurfaceInput,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32)->vec3<f32> {
    return brdf(material_normal(input),unit_normal(input.eye-input.world),l,base,metal,rough);
}

fn pack(h:Hit,d:vec3<f32>)->ProbeRecord {
    return ProbeRecord(vec4<f32>(h.p,select(0.0,1.0,h.hit)),vec4<f32>(h.n,0.0),
        vec4<f32>(h.ng,select(-1.0,1.0,h.source_front)),vec4<f32>(h.uv,0.0,0.0),vec4<f32>(h.color,1.0),
        vec4<u32>(h.identity,h.material,0u),vec4<f32>(d,0.0),vec4<f32>(0.0));
}
fn unpack(r:ProbeRecord)->Hit {
    return Hit(r.world.w!=0.0,r.world.xyz,r.normal.xyz,r.geometric.xyz,r.identity.z,r.uv.xy,r.color.rgb,r.identity.xy,r.geometric.w>0.0);
}
fn input_at(r:ProbeRecord,id:vec2<u32>)->SurfaceInput {
    let m=materials[r.identity.z];
    let material=Material(vec4<f32>(m.base.rgb,1.0),m.base.w,m.params.x,1u,0u,m.params.z,0u,0u,0u);
    // R0b uses level-zero samples and zero derivatives; RF fixtures have no
    // textures. R1 must retain source frames and full ray differentials for LOD.
    return SurfaceInput(r.world.xyz,vec3<f32>(0.0),vec3<f32>(0.0),r.geometric.xyz*r.geometric.w,
        r.normal.xyz*r.geometric.w,vec3<f32>(0.0),vec3<f32>(0.0),basis(r.normal.xyz)[0],1.0,
        r.uv.xy,vec2<f32>(0.0),vec2<f32>(0.0),r.color,r.geometric.w>0.0,r.world.xyz-r.direction.xyz,id,
        material,Receiver(r.world.xyz,r.geometric.xyz,vec2<i32>(id),r.identity.z,1e-4,false));
}
fn own_program(r:ProbeRecord,id:vec2<u32>)->vec3<f32> {
    let input=input_at(r,id);
    if materials[r.identity.z].emission.w!=0.0 {return toon_shade(toon_surface(input),input);}
    return std_shade(std_surface(input),input);
}
fn eligible(r:ProbeRecord)->bool {return r.world.w!=0.0 && materials[r.identity.z].emission.w==0.0;}
fn material_at_hit(hit:Hit)->RefMaterial {
    var m=materials[hit.material];
    m.base=vec4<f32>(clamp(m.base.rgb*hit.color*texture_sample(m.base_map,hit.uv).rgb,vec3<f32>(0.0),vec3<f32>(1.0)),m.base.w);
    let mr=texture_sample(m.mr_map,hit.uv);
    m.base.w=clamp(m.base.w*mr.b,0.0,1.0);m.params.x=clamp(m.params.x*mr.g,0.0,1.0);
    return m;
}
fn primary_ray_probe(id:vec2<u32>)->array<vec3<f32>,2> {
    var ray=camera_ray_time(vec2<f32>(id)+0.5,0.0);
    if probe.mode==3u {
        // Independent mirrored camera: reflect the complete camera basis. This
        // retains pixel handedness; Camera::look_at would reverse the right axis.
        ray[0].y=-ray[0].y;ray[1].y=-ray[1].y;
        // Analytic plane clip, no reflector depth map. This is an oracle only.
        let t=-ray[0].y/ray[1].y;ray[0]=ray[0]+ray[1]*(t+0.0002);
    }
    return ray;
}
@compute @workgroup_size(8,8) fn probe_primary(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=frame.size.xy) {return;}
    let ray=primary_ray_probe(id.xy);let h=trace(ray[0],ray[1]);
    primary[id.y*frame.size.x+id.x]=pack(h,ray[1]);
}
@compute @workgroup_size(8,8) fn probe_primary_shade(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=frame.size.xy) {return;}setup();
    let i=id.y*frame.size.x+id.x;let r=primary[i];var value=frame.background.rgb;
    suppress_specular=probe.mode==1u && eligible(r);
    if r.world.w!=0.0 {value=own_program(r,id.xy);}
    if probe.mode==2u {value=vec3<f32>(0.0);}
    sums[i]=vec4<f32>(value,1.0);
}
@compute @workgroup_size(8,8) fn probe_reflect(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=frame.size.xy) {return;}
    let i=id.y*frame.size.x+id.x;let r=primary[i];var result:ProbeRecord;
    if !eligible(r) {secondary[i]=result;return;}
    let h=unpack(r);let m=material_at_hit(h);let v=-r.direction.xyz;
    var l=lobe(m,dot(h.n,v));l.p_spec=1.0;
    var d=reflect(-v,h.n);var weight=vec3<f32>(0.0);
    let mirror=m.params.x<0.001 || probe.mode==2u;
    if mirror {
        if probe.sample!=0u {secondary[i]=result;return;}
        weight=l.f0+(1.0-l.f0)*pow(1.0-clamp(dot(h.n,v),0.0,1.0),5.0);
    } else {
        // K fixed equal-probability sub-cones of the visible GGX normal lobe.
        // Per-pixel rotation is fixed forever; no frame seed or resampling.
        let rotation=f32(pcg(i)>>8u)/16777216.0;
        let u=vec2<f32>((f32(probe.sample)+0.5)/f32(probe.rays),fract(rotation+f32(probe.sample)*0.618033989));
        let b=basis(h.n);let micro=b*sample_vndf(l,transpose(b)*v,u);d=reflect(-v,micro);
        let pdf=bsdf_pdf(l,h.n,v,d);
        if pdf>0.0 {weight=max(bsdf(m,l,h.n,v,d)-l.diffuse,vec3<f32>(0.0))*max(dot(h.n,d),0.0)/(pdf*f32(probe.rays));}
    }
    if dot(h.ng,d)>0.0 {
        result=pack(trace(offset(h.p,h.ng),d),d);result.weight=vec4<f32>(weight,1.0);
        // A miss still carries a valid ray and its throughput.
        result.direction.w=1.0;
    }
    secondary[i]=result;
}
@compute @workgroup_size(8,8) fn probe_hit_shade(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=frame.size.xy) {return;}setup();
    let i=id.y*frame.size.x+id.x;let r=secondary[i];var value=vec3<f32>(0.0);
    if r.direction.w!=0.0 {
        if r.world.w!=0.0 {value=own_program(r,id.xy);} else {value=expose(sky_radiance(r.direction.xyz));}
    }
    incident[i]=vec4<f32>(value,1.0);
}
@compute @workgroup_size(8,8) fn probe_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=frame.size.xy) {return;}
    let i=id.y*frame.size.x+id.x;let r=secondary[i];
    let weight=select(r.weight.rgb,vec3<f32>(1.0),probe.mode==2u && probe.sample==0u);
    sums[i]+=vec4<f32>(incident[i].rgb*weight,0.0);
}
