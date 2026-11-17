enable wgpu_ray_query;
enable wgpu_ray_query_vertex_return;
enable wgpu_binding_array;
// Live reference (docs/design-reference-live.md): the path tracer's program-free kernels. Its
// estimator is src/reference.rs's (shaders/reference_path_tracer.wgsl), unchanged in its
// mathematics, split into wavefront stages so every material program can run between them as
// the engine runs it (Stage::Reference, shaders/reference_live_entry.wgsl):
//   rf_begin   one camera ray per pixel for sample `frame.size.z` (pixel and shutter jitter);
//   rf_trace   closest hits on the engine's TLAS with the camera trace's single-sided rule
//              (primary_trace.wgsl), then a free flight through the fog: a flight shorter than
//              the segment scatters in the fog there (rf_scatter), else a hit queues its record
//              and a miss collects the sun and sky (power-heuristic weights against the previous
//              BSDF or phase sample) and ends the path;
//   (programs) each program stores the material records of its own draws' hits;
//   rf_shade   the record's emission, next-event estimation for the sun, local lights and sky,
//              then a BSDF sample of the standard model (Lambert plus GGX, height-correlated
//              Smith) and Russian roulette after three bounces;
//   rf_end     each pixel's sample into its even or odd sum.
// Every surface is shaded two-sided toward the arriving ray, as src/reference.rs does.
const PI:f32=3.14159265358979;
const RF_HIT:u32=1u;
// rf_paths[i].state: the path still continues (cleared when it ends).
const RF_ALIVE:u32=1u;

struct Frame {
    eye0:vec4<f32>, forward0:vec4<f32>, right0:vec4<f32>, up0:vec4<f32>,
    eye1:vec4<f32>, forward1:vec4<f32>, right1:vec4<f32>, up1:vec4<f32>,
    // xyz: unit direction to the sun; w: 1 if a sun exists.
    sun:vec4<f32>,
    // rgb: disk radiance (cd/m^2), or illuminance (lux) for a delta sun; w: 1 - cos(angular radius), 0 for a delta sun.
    sun_value:vec4<f32>,
    // rgb: constant sky radiance; w: the environment cube's face size, 0 for a constant sky.
    sky:vec4<f32>,
    environment:vec4<f32>, // cos(yaw), sin(yaw), intensity
    // x width, y height, z this sample's index, w local lights.
    size:vec4<u32>,
    // x max path depth (1 = emitters only, 2 = direct), y seed.
    options:vec4<u32>,
    // The fog (src/reference_live.rs Fog): height-fog extinction (/m) at its base height, falloff
    // (/m), base world y (m), and the volumetric fog's extra extinction (/m); 0 and 0: no fog.
    fog:vec4<f32>,
    // rgb single-scattering albedo, w Henyey-Greenstein g.
    fog_albedo:vec4<f32>,
    // x the extra extinction's radius about the eye (m), y the planet ground's world y (m), z its radius (km).
    fog_space:vec4<f32>,
};
// A path: its ray, throughput, radiance so far and the pdfs of the BSDF sample that produced
// the ray (0: the camera's), its random state and depth.
struct Path { origin:vec3<f32>, pdf_prev:f32, direction:vec3<f32>, sky_pdf_prev:f32, throughput:vec3<f32>, rng:u32, total:vec3<f32>, depth:u32, state:u32, pad0:u32, pad1:u32, pad2:u32 };
// Must match shaders/reference_live_entry.wgsl and src/reference_live.rs.
struct RfHit { placement:u32, primitive:u32, bary:vec2<f32>, origin:vec3<f32>, flags:u32 };
struct RfRecord { normal_alpha:vec4<f32>, base_metal:vec4<f32>, emission_rough:vec4<f32>, world_model:vec4<f32>, geometric:vec4<f32> };
// inner: x cos(inner cone), y source radius (0: a point).
struct LocalLight {position_range:vec4<f32>, candela_kind:vec4<f32>, travel_outer:vec4<f32>, inner:vec4<f32>};

@group(0) @binding(0) var<uniform> frame:Frame;
@group(0) @binding(1) var scene:acceleration_structure<vertex_return>;
@group(0) @binding(2) var<storage,read_write> paths:array<Path>;
@group(0) @binding(3) var<storage,read_write> hits:array<RfHit>;
@group(0) @binding(4) var<storage,read> records:array<RfRecord>;
// Two running sums per pixel (even and odd samples): the mean and a split-half noise estimate.
@group(0) @binding(5) var<storage,read_write> sums:array<vec4<f32>>;
// The environment cube's texels (rgb) with the running sum of their sampling mass (w).
@group(0) @binding(6) var<storage,read> environment_texels:array<vec4<f32>>;
@group(0) @binding(7) var<storage,read> local_lights:array<LocalLight>;
// x: paths still alive after this bounce's shading (the host stops once it reads zero).
@group(0) @binding(8) var<storage,read_write> counters:array<atomic<u32>,4>;
// Per draw: bit 0 marks a see-through (transparent) material.
@group(0) @binding(9) var<storage,read> rf_batches:array<u32>;
// The engine's placement table (primary_rays.wgsl): bit 30 marks single-sided draws.
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;
// The frame's traced geometry and materials (program_traced.wgsl's group 1 and 2), for the coverage of
// see-through shadow casters.
struct RfVec4s { data:array<vec4<f32>> };
struct RfU32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<RfVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<RfU32s,PRIMARY_BATCHES>;
struct Material { base_color:vec4<f32>, metallic:f32, roughness:f32, double_sided:u32, thin:u32, normal_scale:f32, normal_mapped:u32, normal_variance:u32, pad:u32 };
struct RfMaterial { value:Material };
@group(2) @binding(10) var<storage,read> primary_materials:binding_array<RfMaterial,PRIMARY_BATCHES>;
@group(2) @binding(11) var primary_base_color:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(12) var primary_base_sampler:binding_array<sampler,PRIMARY_BATCHES>;

// ---- src/reference.rs's environment, random numbers, basis and BSDF (reference_path_tracer.wgsl) ----
fn environment_direction(face:u32, uv:vec2<f32>)->vec3<f32> {
    switch face {
        case 0u: {return normalize(vec3<f32>(1.0,-uv.y,-uv.x));}
        case 1u: {return normalize(vec3<f32>(-1.0,-uv.y,uv.x));}
        case 2u: {return normalize(vec3<f32>(uv.x,1.0,uv.y));}
        case 3u: {return normalize(vec3<f32>(uv.x,-1.0,-uv.y));}
        case 4u: {return normalize(vec3<f32>(uv.x,-uv.y,1.0));}
        default: {return normalize(vec3<f32>(-uv.x,-uv.y,-1.0));}
    }
}
fn environment_rotate(d:vec3<f32>, inverse:bool)->vec3<f32> {
    let c=frame.environment.x; let s=select(-frame.environment.y,frame.environment.y,inverse);
    return vec3<f32>(c*d.x+s*d.z,d.y,-s*d.x+c*d.z);
}
fn environment_uv(d:vec3<f32>)->vec3<f32> {
    let a=abs(d);
    if a.x>=a.y && a.x>=a.z {
        if d.x>=0.0 {return vec3<f32>(0.0,-d.z/a.x,-d.y/a.x);}
        return vec3<f32>(1.0,d.z/a.x,-d.y/a.x);
    }
    if a.y>=a.z {
        if d.y>=0.0 {return vec3<f32>(2.0,d.x/a.y,d.z/a.y);}
        return vec3<f32>(3.0,d.x/a.y,-d.z/a.y);
    }
    if d.z>=0.0 {return vec3<f32>(4.0,d.x/a.z,-d.y/a.z);}
    return vec3<f32>(5.0,-d.x/a.z,-d.y/a.z);
}
fn environment_index(uv:vec3<f32>)->u32 {
    let n=u32(frame.sky.w);
    let p=vec2<u32>(clamp((uv.yz*0.5+0.5)*f32(n),vec2<f32>(0.0),vec2<f32>(f32(n)-1.0)));
    return u32(uv.x)*n*n+p.y*n+p.x;
}
fn environment_remap(face:u32, xy:vec2<i32>)->vec3<f32> {
    let n=frame.sky.w;
    let d=environment_direction(face,(vec2<f32>(xy)+0.5)*(2.0/n)-1.0);
    return environment_texels[environment_index(environment_uv(d))].rgb;
}
fn environment_tap(face:u32, xy:vec2<i32>)->vec3<f32> {
    let n=i32(frame.sky.w);
    if (xy.x<0 || xy.x>=n) && (xy.y<0 || xy.y>=n) {
        let q=clamp(xy,vec2<i32>(0),vec2<i32>(n-1));
        return (environment_remap(face,q)+environment_remap(face,vec2<i32>(xy.x,q.y))+environment_remap(face,vec2<i32>(q.x,xy.y)))/3.0;
    }
    return environment_remap(face,xy);
}
fn sky_radiance(d:vec3<f32>)->vec3<f32> {
    if frame.sky.w==0.0 {return frame.sky.rgb;}
    let uv=environment_uv(environment_rotate(d,false));
    let p=(uv.yz*0.5+0.5)*frame.sky.w-0.5;
    let xy=vec2<i32>(floor(p)); let f=fract(p); let face=u32(uv.x);
    return frame.environment.z*mix(mix(environment_tap(face,xy),environment_tap(face,xy+vec2<i32>(1,0)),f.x),
        mix(environment_tap(face,xy+vec2<i32>(0,1)),environment_tap(face,xy+vec2<i32>(1,1)),f.x),f.y);
}
fn environment_pdf(d:vec3<f32>)->f32 {
    let uv=environment_uv(environment_rotate(d,false));
    let i=environment_index(uv);
    var prev=0.0; if i>0u {prev=environment_texels[i-1u].w;}
    let mass=environment_texels[i].w-prev;
    return mass*frame.sky.w*frame.sky.w*0.25*pow(1.0+dot(uv.yz,uv.yz),1.5);
}
fn sample_environment()->vec3<f32> {
    let u=rand(); var lo=0u; var hi=arrayLength(&environment_texels)-1u;
    while lo<hi {let mid=(lo+hi)/2u; if u<environment_texels[mid].w {hi=mid;} else {lo=mid+1u;}}
    let n=u32(frame.sky.w); let p=lo%(n*n);
    let uv=(vec2<f32>(f32(p%n),f32(p/n))+vec2<f32>(rand(),rand()))*(2.0/f32(n))-1.0;
    return environment_rotate(environment_direction(lo/(n*n),uv),true);
}
fn pcg(v:u32)->u32 {
    let state=v*747796405u+2891336453u;
    let word=((state>>((state>>28u)+4u))^state)*277803737u;
    return (word>>22u)^word;
}
var<private> rng_state:u32;
fn rand()->f32 {
    rng_state=pcg(rng_state);
    return f32(rng_state>>8u)*(1.0/16777216.0);
}
fn luminance(c:vec3<f32>)->f32 {return dot(c,vec3<f32>(0.2126,0.7152,0.0722));}
fn basis(n:vec3<f32>)->mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);
    let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
fn offset(p:vec3<f32>, ng:vec3<f32>)->vec3<f32> {
    let m=max(max(abs(p.x),abs(p.y)),abs(p.z));
    return p+ng*(1e-4+4e-7*m);
}
fn sun_height()->f32 {return frame.sun_value.w;}
fn sample_sun()->vec3<f32> {
    if sun_height()==0.0 {return frame.sun.xyz;}
    let h=rand()*sun_height();let phi=2.0*PI*rand();
    let r=sqrt(h*(2.0-h));
    return basis(frame.sun.xyz)*vec3<f32>(r*cos(phi),r*sin(phi),1.0-h);
}
fn in_sun(d:vec3<f32>)->bool {
    let s=cross(d,frame.sun.xyz);
    return frame.sun.w>0.0 && sun_height()>0.0 && dot(d,frame.sun.xyz)>0.0
        && dot(s,s)<=sun_height()*(2.0-sun_height());
}
fn sun_pdf()->f32 {return 1.0/(2.0*PI*sun_height());}
// The standard model on a material record: specular F0 0.04 (dielectric) or the base (metal).
struct Lobe {alpha:f32, a2:f32, f0:vec3<f32>, diffuse:vec3<f32>, spec:f32, p_spec:f32};
fn lobe(base:vec3<f32>, metal:f32, roughness:f32, nv:f32)->Lobe {
    let alpha=max(roughness*roughness,1e-4);
    let f0=vec3<f32>(0.04)*(1.0-metal)+base*metal;
    let diffuse=base*((1.0-metal)/PI);
    let fs=luminance(f0+(vec3<f32>(1.0)-f0)*pow(1.0-clamp(nv,0.0,1.0),5.0));
    let fd=luminance(diffuse)*PI;
    let p=select(fs/max(fs+fd,1e-12),0.0,fs<=0.0);
    return Lobe(alpha,alpha*alpha,f0,diffuse,1.0,p);
}
fn ggx_d(l:Lobe, n:vec3<f32>, h:vec3<f32>)->f32 {
    let nh=max(dot(n,h),0.0);
    let c=cross(n,h);
    let denom=max(dot(c,c)+l.a2*nh*nh,1e-30);
    return l.a2/(PI*denom*denom);
}
fn g1(l:Lobe, x:f32)->f32 {return 2.0*x/(x+sqrt(l.a2+(1.0-l.a2)*x*x));}
fn bsdf(l:Lobe, n:vec3<f32>, v:vec3<f32>, wi:vec3<f32>)->vec3<f32> {
    let nl=dot(n,wi);
    let nv=dot(n,v);
    if nl<=0.0 || nv<=0.0 {return vec3<f32>(0.0);}
    let h=normalize(wi+v);
    let vh=clamp(dot(v,h),0.0,1.0);
    let vis=0.5/max(nl*sqrt(l.a2+(1.0-l.a2)*nv*nv)+nv*sqrt(l.a2+(1.0-l.a2)*nl*nl),1e-30);
    let fresnel=l.f0+(vec3<f32>(1.0)-l.f0)*pow(1.0-vh,5.0);
    return l.diffuse+ggx_d(l,n,h)*vis*fresnel;
}
fn bsdf_pdf(l:Lobe, n:vec3<f32>, v:vec3<f32>, wi:vec3<f32>)->f32 {
    let nl=dot(n,wi);
    let nv=dot(n,v);
    if nl<=0.0 || nv<=0.0 {return 0.0;}
    var pdf=(1.0-l.p_spec)*nl/PI;
    if l.p_spec>0.0 {
        let h=normalize(wi+v);
        pdf+=l.p_spec*g1(l,nv)*ggx_d(l,n,h)/(4.0*nv);
    }
    return pdf;
}
fn sample_vndf(l:Lobe, v:vec3<f32>, u:vec2<f32>)->vec3<f32> {
    let vh=normalize(vec3<f32>(l.alpha*v.x,l.alpha*v.y,v.z));
    let len2=vh.x*vh.x+vh.y*vh.y;
    let t1=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(-vh.y,vh.x,0.0)*inverseSqrt(len2),len2>0.0);
    let t2=cross(vh,t1);
    let r=sqrt(u.x);
    let phi=2.0*PI*u.y;
    let a=r*cos(phi);
    var b=r*sin(phi);
    let s=0.5*(1.0+vh.z);
    b=(1.0-s)*sqrt(max(0.0,1.0-a*a))+s*b;
    let nh=a*t1+b*t2+sqrt(max(0.0,1.0-a*a-b*b))*vh;
    return normalize(vec3<f32>(l.alpha*nh.x,l.alpha*nh.y,max(0.0,nh.z)));
}
fn power(a:f32, b:f32)->f32 {return a*a/max(a*a+b*b,1e-30);}
// ---- end of src/reference.rs's functions ----

// ---- The fog: the engine's height and volumetric fog as one grey participating medium ----
fn fog_on()->bool {return frame.fog.x>0.0 || frame.fog.w>0.0;}
// The height fog's extinction (/m) at world height y; along o + t d it is fog_base(o.y) * exp(-falloff d.y t).
fn fog_base(y:f32)->f32 {return frame.fog.x*exp(clamp(-frame.fog.y*(y-frame.fog.z),-80.0,80.0));}
// The ray's interval [x, y] inside the extra extinction's sphere about the eye (empty: y <= x).
fn fog_sphere(o:vec3<f32>, d:vec3<f32>)->vec2<f32> {
    if frame.fog.w<=0.0 {return vec2<f32>(0.0);}
    let q=o-frame.eye0.xyz;
    let b=dot(q,d);
    let disc=b*b-(dot(q,q)-frame.fog_space.x*frame.fog_space.x);
    if disc<=0.0 {return vec2<f32>(0.0);}
    let r=sqrt(disc);
    return vec2<f32>(max(-b-r,0.0),max(-b+r,0.0));
}
// How far a ray that leaves the scene meets fog: to the planet's ground, or 100 km (the engine's
// backdrop fog, atmosphere_api.wgsl at_fog_backdrop_distance, from the ray's own altitude).
fn fog_escape(o:vec3<f32>, d:vec3<f32>)->f32 {
    let bottom=frame.fog_space.z;
    let h=max((o.y-frame.fog_space.y)*1e-3,1e-4);
    let b=(bottom+h)*d.y;
    let c=h*(2.0*bottom+h);
    let disc=b*b-c;
    if b>=0.0 || disc<0.0 {return 1e5;}
    return min(1000.0*c/(-b+sqrt(disc)),1e5);
}
// The fraction of light the fog passes along o + t d over [0, tmax], in closed form.
fn fog_transmittance(o:vec3<f32>, d:vec3<f32>, tmax:f32)->f32 {
    if !fog_on() {return 1.0;}
    var depth=0.0;
    let base=fog_base(o.y);
    if base>0.0 {
        let k=frame.fog.y*d.y;
        let x=k*tmax;
        if abs(x)<1e-4 {depth=base*tmax*(1.0-0.5*x);} else {depth=base*(1.0-exp(clamp(-x,-80.0,80.0)))/k;}
    }
    let s=fog_sphere(o,d);
    depth+=frame.fog.w*max(min(s.y,tmax)-s.x,0.0);
    return exp(-min(depth,1e4));
}
// A free-flight distance through the fog from o along d: the nearer of the height fog's and the
// sphere's own (exact for the sum of the two media), each by inverting its optical depth; 1e30
// when neither scatters.
fn fog_flight(o:vec3<f32>, d:vec3<f32>)->f32 {
    var t=1e30;
    let base=fog_base(o.y);
    if base>0.0 {
        let xi=-log(1.0-rand());
        let k=frame.fog.y*d.y;
        let x=xi*k/base;
        if abs(x)<1e-4 {t=xi/base*(1.0+0.5*x);} else if x<1.0 {t=-log(1.0-x)/k;}
    }
    let s=fog_sphere(o,d);
    if s.y>s.x {
        let te=s.x-log(1.0-rand())/frame.fog.w;
        if te<s.y {t=min(t,te);}
    }
    return t;
}
// Henyey-Greenstein's phase for the cosine c between the travel direction and the scattered one.
fn hg(g:f32, c:f32)->f32 {
    let d=1.0+g*g-2.0*g*c;
    return (1.0-g*g)/(4.0*PI*d*sqrt(d));
}
fn sample_hg(g:f32, travel:vec3<f32>)->vec3<f32> {
    let u=rand();
    var c=1.0-2.0*u;
    if abs(g)>=1e-3 {
        let q=(1.0-g*g)/(1.0-g+2.0*g*u);
        c=clamp((1.0+g*g-q*q)/(2.0*g),-1.0,1.0);
    }
    let s=sqrt(max(1.0-c*c,0.0));
    let phi=2.0*PI*rand();
    return basis(travel)*vec3<f32>(s*cos(phi),s*sin(phi),c);
}
// ---- end of the fog ----

// A see-through hit's base colour (rgb) and coverage (a), as the standard program resolves them: the
// material's factor, vertex colour and base-colour texture at level 0, coverage through
// transparent_coverage (program_api.wgsl): the opacity times the alpha product.
fn rf_seethrough(batch:u32, primitive:u32, bary:vec2<f32>)->vec4<f32> {
    let m=primary_materials[batch].value;
    let i=primitive*3u;
    let ia=primary_indices[batch].data[i];let ib=primary_indices[batch].data[i+1u];let ic=primary_indices[batch].data[i+2u];
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let uv=primary_vertices[batch].data[ia*3u+1u].zw*w.x+primary_vertices[batch].data[ib*3u+1u].zw*w.y+primary_vertices[batch].data[ic*3u+1u].zw*w.z;
    let color=primary_vertices[batch].data[ia*3u+2u]*w.x+primary_vertices[batch].data[ib*3u+2u]*w.y+primary_vertices[batch].data[ic*3u+2u]*w.z;
    let sampled=textureSampleLevel(primary_base_color[batch],primary_base_sampler[batch],uv,0.0);
    let base=clamp(m.base_color.rgb*color.rgb*sampled.rgb,vec3<f32>(0.0),vec3<f32>(1.0));
    let opacity=select(1.0,unpack2x16float(m.pad).x,m.pad!=0u);
    let coverage=select(1.0,clamp(opacity*m.base_color.a*sampled.a*color.a,0.0,1.0),m.pad!=0u);
    return vec4<f32>(base,coverage);
}
// Schlick's dielectric Fresnel reflectance at cosine c for index n.
fn rf_fresnel(n:f32, c:f32)->f32 {
    let f0=((n-1.0)/(n+1.0))*((n-1.0)/(n+1.0));
    return f0+(1.0-f0)*pow(1.0-clamp(c,0.0,1.0),5.0);
}
// The light that reaches along the ray before `tmax`: zero behind an opaque shadow caster; a see-through
// caster passes 1 - its coverage, glass (IOR above 1) also its Fresnel transmission tinted by its base colour.
fn transmitted_within(origin:vec3<f32>, direction:vec3<f32>, tmax:f32)->vec3<f32> {
    var query:ray_query<vertex_return>;
    rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_NO_OPAQUE,SHADOW_CASTER_MASK,0.0,tmax,origin,direction));
    var t=vec3<f32>(1.0);
    while rayQueryProceed(&query) {
        let c=rayQueryGetCandidateIntersection(&query);
        let batch=primary_placements[c.instance_custom_data]&0x1fffffffu;
        if (rf_batches[batch]&1u)==0u {
            rayQueryConfirmIntersection(&query);
            continue;
        }
        let s=rf_seethrough(batch,c.primitive_index,c.barycentrics);
        let m=primary_materials[batch].value;
        let ior=unpack2x16float(m.pad).y;
        var share=vec3<f32>(1.0-s.a);
        if ior>1.0 {
            let v=getCandidateHitVertexPositions(&query);
            let ng=normalize(cross(v[1]-v[0],v[2]-v[0])*mat3x3<f32>(c.world_to_object[0],c.world_to_object[1],c.world_to_object[2]));
            share*=(1.0-rf_fresnel(ior,abs(dot(ng,direction))))*s.rgb;
        }
        t*=share;
    }
    if rayQueryGetCommittedIntersection(&query).kind!=0u {return vec3<f32>(0.0);}
    return t;
}
// Local light `light` seen from p, with the engine's attenuation. A light with a source radius r is
// a sphere of uniform radiance I / (pi r^2) (I its candela), sampled uniformly over the cone it
// subtends: its weight, the cone's solid angle over pi r^2, tends to the point light's 1 / d^2 far
// away. Range window and spot cone are the engine's, at the centre. `reach` ends the shadow ray.
struct LightSample {wi:vec3<f32>, reach:f32, attenuation:f32};
fn sample_light(light:LocalLight, p:vec3<f32>)->LightSample {
    let delta=light.position_range.xyz-p;
    let distance=length(delta);
    let radius=light.inner.y;
    var wi=delta/max(distance,1e-30);
    var reach=distance*0.999;
    var attenuation=0.0;
    if radius>0.0 && distance>radius*1.0001 {
        let sin_max=radius/distance;
        let cos_max=sqrt(max(1.0-sin_max*sin_max,0.0));
        let z=1.0-rand()*(1.0-cos_max);
        let phi=2.0*PI*rand();
        let s=sqrt(max(1.0-z*z,0.0));
        wi=basis(wi)*vec3<f32>(s*cos(phi),s*sin(phi),z);
        // The sampled ray meets the sphere at the near root of |p + t wi - c| = r.
        let b=dot(wi,delta);
        reach=(b-sqrt(max(b*b-(distance*distance-radius*radius),0.0)))*0.999;
        attenuation=2.0*PI*(1.0-cos_max)/(PI*radius*radius);
    } else {
        let inverse=1.0/max(distance,0.01);
        attenuation=inverse*inverse;
    }
    if light.position_range.w>0.0 {
        let ratio=min(distance/light.position_range.w,1.0);
        let window=max(1.0-ratio*ratio*ratio*ratio,0.0);
        attenuation*=window*window;
    }
    if light.candela_kind.w>1.5 {
        let cone=clamp((dot(-wi,light.travel_outer.xyz)-light.travel_outer.w)/max(light.inner.x-light.travel_outer.w,1e-4),0.0,1.0);
        attenuation*=cone*cone;
    }
    return LightSample(wi,reach,attenuation);
}
// What reaches p from direction wi out of the scene: see-through casters and the fog.
fn sky_share(p:vec3<f32>, wi:vec3<f32>)->vec3<f32> {
    return transmitted_within(p,wi,1e30)*fog_transmittance(p,wi,fog_escape(p,wi));
}
// A scattering in the fog at p: the albedo's share goes on, next-event estimation collects the
// sun, local lights and sky through the fog and its see-through casters (power heuristic against
// the phase sample, as at surfaces), and the phase function samples the continuation.
fn rf_scatter(path:ptr<function,Path>, p:vec3<f32>) {
    let travel=(*path).direction;
    let g=frame.fog_albedo.w;
    (*path).throughput*=frame.fog_albedo.rgb;
    var alive=(*path).depth<frame.options.x;
    if alive {
        let beta=(*path).throughput;
        if frame.sun.w>0.0 {
            let wi=sample_sun();
            let phase=hg(g,dot(travel,wi));
            let f=phase*sky_share(p,wi);
            if sun_height()==0.0 {
                (*path).total+=beta*f*frame.sun_value.rgb;
            } else {
                let light_pdf=sun_pdf();
                (*path).total+=beta*f*frame.sun_value.rgb*power(light_pdf,phase)/light_pdf;
            }
        }
        for (var k=0u; k<frame.size.w; k+=1u) {
            let s=sample_light(local_lights[k],p);
            if s.attenuation<=0.0 {continue;}
            let share=transmitted_within(p,s.wi,s.reach)*fog_transmittance(p,s.wi,s.reach);
            (*path).total+=beta*hg(g,dot(travel,s.wi))*share*local_lights[k].candela_kind.rgb*s.attenuation;
        }
        if frame.sky.w>0.0 || luminance(frame.sky.rgb)>0.0 {
            var wi:vec3<f32>;
            var light_pdf=1.0/(4.0*PI);
            if frame.sky.w>0.0 {
                wi=sample_environment();
                light_pdf=environment_pdf(wi);
            } else {
                let z=1.0-2.0*rand();
                let phi=2.0*PI*rand();
                let r=sqrt(max(1.0-z*z,0.0));
                wi=vec3<f32>(r*cos(phi),r*sin(phi),z);
            }
            if frame.sky.w>0.0 || !in_sun(wi) {
                let phase=hg(g,dot(travel,wi));
                (*path).total+=beta*phase*sky_share(p,wi)*sky_radiance(wi)*power(light_pdf,phase)/max(light_pdf,1e-30);
            }
        }
        // The phase sample: its pdf is the phase, so the throughput keeps its value.
        let wi=sample_hg(g,travel);
        (*path).pdf_prev=hg(g,dot(travel,wi));
        (*path).sky_pdf_prev=select(0.0,1.0/(4.0*PI),luminance(frame.sky.rgb)>0.0);
        if frame.sky.w>0.0 {(*path).sky_pdf_prev=environment_pdf(wi);}
        (*path).origin=p;
        (*path).direction=wi;
        // Russian roulette after three bounces, as at surfaces.
        if (*path).depth>=3u {
            let t=(*path).throughput;
            let q=min(max(t.r,max(t.g,t.b)),0.95);
            if rand()>=q {alive=false;} else {(*path).throughput/=q;}
        }
    }
    (*path).depth+=1u;
    (*path).rng=rng_state;
    if alive {atomicAdd(&counters[0],1u);} else {(*path).state=0u;}
}
fn camera_ray_time(pixel:vec2<f32>, t:f32)->array<vec3<f32>,2> {
    let eye=mix(frame.eye0.xyz,frame.eye1.xyz,t);
    let forward=normalize(mix(frame.forward0.xyz,frame.forward1.xyz,t));
    let right=normalize(mix(frame.right0.xyz,frame.right1.xyz,t));
    let up=normalize(mix(frame.up0.xyz,frame.up1.xyz,t));
    let size=vec2<f32>(frame.size.xy);
    let ndc=vec2<f32>(pixel.x/size.x*2.0-1.0,1.0-pixel.y/size.y*2.0);
    let tan_y=frame.eye0.w;
    let tan_x=tan_y*size.x/size.y;
    return array<vec3<f32>,2>(eye,normalize(forward+ndc.x*tan_x*right+ndc.y*tan_y*up));
}
fn pixel_count()->u32 {return frame.size.x*frame.size.y;}

@compute @workgroup_size(64) fn rf_begin(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i>=pixel_count() {return;}
    let s=frame.size.z;
    rng_state=pcg(i^pcg(s^pcg(frame.options.y)));
    let pixel=vec2<u32>(i%frame.size.x,i/frame.size.x);
    let jitter=vec2<f32>(rand(),rand());
    let ray=camera_ray_time(vec2<f32>(pixel)+jitter,rand());
    paths[i]=Path(ray[0],0.0,ray[1],0.0,vec3<f32>(1.0),rng_state,vec3<f32>(0.0),1u,RF_ALIVE,0u,0u,0u);
    hits[i].flags=0u;
}

@compute @workgroup_size(64) fn rf_trace(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i>=pixel_count() {return;}
    hits[i].flags=0u;
    var path=paths[i];
    if (path.state&RF_ALIVE)==0u {return;}
    rng_state=path.rng;
    var query:ray_query<vertex_return>;
    rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_NO_OPAQUE,255u,0.0,1e30,path.origin,path.direction));
    while rayQueryProceed(&query) {
        // primary_trace.wgsl: single-sided draws hide source-back faces, by the object-space
        // winding against the object-space ray.
        let candidate=rayQueryGetCandidateIntersection(&query);
        var visible=(primary_placements[candidate.instance_custom_data]&0x40000000u)==0u;
        if !visible {
            let v=getCandidateHitVertexPositions(&query);
            visible=dot(cross(v[1]-v[0],v[2]-v[0]),candidate.world_to_object*vec4<f32>(path.direction,0.0))<0.0;
        }
        if visible {rayQueryConfirmIntersection(&query);}
    }
    let hit=rayQueryGetCommittedIntersection(&query);
    // The fog: a free flight shorter than the segment (to the hit, or out of the scene) scatters there.
    if fog_on() {
        let reach=select(fog_escape(path.origin,path.direction),hit.t,hit.kind!=0u);
        let t=fog_flight(path.origin,path.direction);
        if t<reach {
            rf_scatter(&path,path.origin+t*path.direction);
            paths[i]=path;
            return;
        }
        path.rng=rng_state;
        paths[i].rng=rng_state;
    }
    if hit.kind!=0u {
        hits[i]=RfHit(hit.instance_custom_data,hit.primitive_index,hit.barycentrics,path.origin,RF_HIT);
        return;
    }
    // A miss: the sun disk inside its cone, else the sky, then the path ends.
    if in_sun(path.direction) {
        let w=select(power(path.pdf_prev,sun_pdf()),1.0,path.pdf_prev==0.0);
        path.total+=path.throughput*frame.sun_value.rgb*w;
    }
    if frame.sky.w>0.0 || !in_sun(path.direction) {
        let w=select(power(path.pdf_prev,path.sky_pdf_prev),1.0,path.pdf_prev==0.0);
        path.total+=path.throughput*sky_radiance(path.direction)*w;
    }
    path.state=0u;
    paths[i]=path;
}

@compute @workgroup_size(64) fn rf_shade(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i>=pixel_count() {return;}
    var path=paths[i];
    if (path.state&RF_ALIVE)==0u || (hits[i].flags&RF_HIT)==0u {return;}
    rng_state=path.rng;
    let r=records[i];
    let n=r.normal_alpha.xyz;
    let ng=r.geometric.xyz;
    let p=r.world_model.xyz;
    let base=r.base_metal.rgb;
    let metal=r.base_metal.w;
    let roughness=r.emission_rough.w;
    // A see-through surface (TP1): over its coverage it is the surface below; elsewhere a film (IOR 1)
    // passes the path straight on, and glass is a thin slab, a Fresnel mirror or a straight
    // transmission tinted by its base colour (no lateral shift: the slab's thickness is ignored).
    let ior=r.geometric.w;
    if ior>0.0 && rand()>=r.normal_alpha.w {
        var through=true;
        if ior>1.0 {
            if rand()<rf_fresnel(ior,abs(dot(n,path.direction))) {
                through=false;
                path.direction=reflect(path.direction,select(-n,n,dot(n,path.direction)<0.0));
                path.origin=offset(p,ng);
                path.pdf_prev=0.0;
            } else {
                path.throughput*=base;
            }
        }
        if through {
            // The path goes on in its own direction: its BSDF pdf still weighs what it meets next.
            path.origin=offset(p,-ng);
        }
        path.depth+=1u;
        path.rng=rng_state;
        let alive=path.depth<=frame.options.x+64u && any(path.throughput>vec3<f32>(0.0));
        if alive {atomicAdd(&counters[0],1u);} else {path.state=0u;}
        paths[i]=path;
        return;
    }
    // Emitters are found only by hitting them (src/reference.rs): no light sampling, no weight.
    path.total+=path.throughput*r.emission_rough.rgb;
    let v=-path.direction;
    let nv=dot(n,v);
    var alive=path.depth<frame.options.x && nv>0.0;
    if alive {
        let l=lobe(base,metal,roughness,nv);
        let start=offset(p,ng);
        // Next-event estimation: sun.
        if frame.sun.w>0.0 {
            let wi=sample_sun();
            let cos_l=dot(n,wi);
            if cos_l>0.0 && dot(ng,wi)>0.0 {
                let f=bsdf(l,n,v,wi)*sky_share(start,wi);
                if sun_height()==0.0 {
                    path.total+=path.throughput*f*cos_l*frame.sun_value.rgb;
                } else {
                    let light_pdf=sun_pdf();
                    path.total+=path.throughput*f*cos_l*frame.sun_value.rgb*power(light_pdf,bsdf_pdf(l,n,v,wi))/light_pdf;
                }
            }
        }
        // Next-event estimation: local lights with the engine's attenuation (sample_light).
        for (var k=0u; k<frame.size.w; k+=1u) {
            let s=sample_light(local_lights[k],p);
            let cos_l=dot(n,s.wi);
            if cos_l<=0.0 || dot(ng,s.wi)<=0.0 || s.attenuation<=0.0 {continue;}
            let share=transmitted_within(start,s.wi,s.reach)*fog_transmittance(start,s.wi,s.reach);
            path.total+=path.throughput*bsdf(l,n,v,s.wi)*share*cos_l*local_lights[k].candela_kind.rgb*s.attenuation;
        }
        // Next-event estimation: sky (the environment's importance sampling, or cosine-weighted
        // about the shading normal for a constant sky).
        if frame.sky.w>0.0 || luminance(frame.sky.rgb)>0.0 {
            var wi:vec3<f32>;
            if frame.sky.w>0.0 {wi=sample_environment();} else {
                let u=sqrt(rand()); let phi=2.0*PI*rand();
                wi=basis(n)*vec3<f32>(u*cos(phi),u*sin(phi),sqrt(max(0.0,1.0-u*u)));
            }
            let cos_l=dot(n,wi);
            if cos_l>0.0 && dot(ng,wi)>0.0 && (frame.sky.w>0.0 || !in_sun(wi)) {
                let share=sky_share(start,wi);
                var light_pdf=cos_l/PI;
                if frame.sky.w>0.0 {light_pdf=environment_pdf(wi);}
                let w=power(light_pdf,bsdf_pdf(l,n,v,wi));
                path.total+=path.throughput*bsdf(l,n,v,wi)*share*cos_l*sky_radiance(wi)*w/max(light_pdf,1e-30);
            }
        }
        // BSDF sampling for the continuation.
        let frame_n=basis(n);
        var wi:vec3<f32>;
        if rand()<l.p_spec {
            let local_v=transpose(frame_n)*v;
            let h=frame_n*sample_vndf(l,local_v,vec2<f32>(rand(),rand()));
            wi=reflect(-v,h);
        } else {
            let u=sqrt(rand());
            let phi=2.0*PI*rand();
            wi=frame_n*vec3<f32>(u*cos(phi),u*sin(phi),sqrt(max(0.0,1.0-u*u)));
        }
        let cos_i=dot(n,wi);
        let pdf=bsdf_pdf(l,n,v,wi);
        alive=cos_i>0.0 && dot(ng,wi)>0.0 && pdf>0.0;
        if alive {
            path.throughput*=bsdf(l,n,v,wi)*cos_i/pdf;
            path.pdf_prev=pdf;
            path.sky_pdf_prev=select(0.0,cos_i/PI,luminance(frame.sky.rgb)>0.0);
            if frame.sky.w>0.0 {path.sky_pdf_prev=environment_pdf(wi);}
            path.origin=start;
            path.direction=wi;
            // Russian roulette after three bounces keeps the estimator unbiased.
            if path.depth>=3u {
                let q=min(max(path.throughput.r,max(path.throughput.g,path.throughput.b)),0.95);
                if rand()>=q {alive=false;} else {path.throughput/=q;}
            }
        }
    }
    path.depth+=1u;
    path.rng=rng_state;
    if alive {atomicAdd(&counters[0],1u);} else {path.state=0u;}
    paths[i]=path;
}

@compute @workgroup_size(64) fn rf_end(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i>=pixel_count() {return;}
    var value=paths[i].total;
    // A non-finite sample is a tracer bug; it is counted, not hidden (src/reference.rs).
    if any(value!=value) || any(abs(value)>vec3<f32>(3.0e38)) {value=vec3<f32>(-1.0e30);}
    // A path still alive here was cut at the bounce limit: counted in counters[1].
    if (paths[i].state&RF_ALIVE)!=0u {atomicAdd(&counters[1],1u);}
    let odd=frame.size.z&1u;
    sums[2u*i+odd]+=vec4<f32>(value,0.0);
}
