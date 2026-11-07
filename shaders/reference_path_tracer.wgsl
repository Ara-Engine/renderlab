enable wgpu_ray_query;
// M7 reference path tracer (src/reference.rs): an unbiased estimate of the physical image
// for the GI and reflection gates. Sun disk (or delta) and sky are sampled with next-event
// estimation and combined with BSDF sampling by the power heuristic. Each sample draws its
// own pixel position (box filter) and shutter time. Measurement tooling, not a frame path.
//
// BSDF: the standard program's model (shaders/scene.wgsl brdf): Lambert * (1 - metal) plus
// GGX with Schlick F0 = 0.04 * specular * (1 - metal) + base * metal, alpha = max(r^2, 1e-4).
// Smith G is height-correlated (the program's) or separable (Mitsuba's, cross-check only).
// Every surface is shaded two-sided, and the sun disk replaces the sky inside its cone.
const PI:f32=3.14159265358979;

struct Frame {
    eye0:vec4<f32>, forward0:vec4<f32>, right0:vec4<f32>, up0:vec4<f32>,
    eye1:vec4<f32>, forward1:vec4<f32>, right1:vec4<f32>, up1:vec4<f32>,
    // xyz: unit direction to the sun; w: cos(angular radius), 1 for a delta sun.
    sun:vec4<f32>,
    // rgb: disk radiance (cd/m^2), or illuminance (lux) for a delta sun; w: 1 if a sun exists.
    sun_value:vec4<f32>,
    sky:vec4<f32>,
    environment:vec4<f32>, // cos(yaw), sin(yaw), intensity
    background:vec4<f32>, // rgb camera-only backdrop, w enabled
    // x width, y height, z first sample index of this dispatch, w samples this dispatch.
    size:vec4<u32>,
    // x max path depth (Mitsuba semantics: 1 = emitters only, 2 = direct), y seed, z smith (0 correlated, 1 separable).
    options:vec4<u32>,
};
struct Vertex {a:vec4<f32>, b:vec4<f32>, tangent:vec4<f32>, color:vec4<f32>};
struct Instance {offsets:vec4<u32>, n0:vec4<f32>, n1:vec4<f32>, n2:vec4<f32>};  // vertex, index, material
struct Material {base:vec4<f32>, emission:vec4<f32>, params:vec4<f32>, base_map:vec4<u32>, mr_map:vec4<u32>, normal_map:vec4<u32>};

@group(0) @binding(0) var<uniform> frame:Frame;
@group(0) @binding(1) var scene:acceleration_structure;
@group(0) @binding(2) var<storage,read> vertices:array<Vertex>;
@group(0) @binding(3) var<storage,read> indices:array<u32>;
@group(0) @binding(4) var<storage,read> instances:array<Instance>;
@group(0) @binding(5) var<storage,read> materials:array<Material>;
// Two running sums per pixel (even and odd samples) give the mean and a split-half noise estimate.
@group(0) @binding(6) var<storage,read_write> sums:array<vec4<f32>>;
@group(0) @binding(7) var<storage,read> texture_texels:array<u32>;
@group(0) @binding(8) var<storage,read> texture_info:array<vec4<u32>>;
@group(0) @binding(9) var<storage,read> environment_texels:array<vec4<f32>>;
struct Geometry {world:vec4<f32>, normal:vec4<f32>, ids:vec4<u32>, direct_even:vec4<f32>, direct_odd:vec4<f32>};
@group(0) @binding(10) var<storage,read_write> geometry:array<Geometry>;

fn address(i:i32, size:i32, wrap:u32)->i32 {
    if wrap==0u {return clamp(i,0,size-1);}
    if wrap==1u {let j=((i%(2*size))+2*size)%(2*size); return select(2*size-1-j,j,j<size);}
    return ((i%size)+size)%size;
}
fn texture_value(mip:vec4<u32>, p:vec2<i32>, binding:vec4<u32>, srgb:bool)->vec4<f32> {
    let xy=vec2<u32>(vec2<i32>(address(p.x,i32(mip.y),binding.y),address(p.y,i32(mip.z),binding.z)));
    let packed=texture_texels[mip.x+xy.y*mip.y+xy.x];
    var value=vec4<f32>(f32(packed&255u),f32((packed>>8u)&255u),f32((packed>>16u)&255u),f32(packed>>24u))/255.0;
    if srgb {value=vec4<f32>(select(pow((value.rgb+0.055)/1.055,vec3<f32>(2.4)),value.rgb/12.92,value.rgb<=vec3<f32>(0.04045)),value.a);}
    return value;
}
fn texture_sample_level(binding:vec4<u32>, uv:vec2<f32>, level:u32)->vec4<f32> {
    if binding.x==0u {return vec4<f32>(1.0);}
    let info=texture_info[binding.x-1u];
    // Level zero is integrated by the path samples' pixel jitter, not preblurred a second time.
    let mip=texture_info[info.x+min(level,info.y-1u)];
    let p=uv*vec2<f32>(mip.yz)-0.5;
    let xy=vec2<i32>(floor(p));
    if binding.w==0u {return texture_value(mip,vec2<i32>(floor(p+0.5)),binding,info.z!=0u);}
    let f=fract(p);
    return mix(mix(texture_value(mip,xy,binding,info.z!=0u),texture_value(mip,xy+vec2<i32>(1,0),binding,info.z!=0u),f.x),
               mix(texture_value(mip,xy+vec2<i32>(0,1),binding,info.z!=0u),texture_value(mip,xy+vec2<i32>(1,1),binding,info.z!=0u),f.x),f.y);
}
fn texture_sample(binding:vec4<u32>, uv:vec2<f32>)->vec4<f32> {
    let lod=max(bitcast<f32>(frame.options.w),0.0);
    return mix(texture_sample_level(binding,uv,u32(lod)),texture_sample_level(binding,uv,u32(lod)+1u),fract(lod));
}
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
// face, u, v in [-1,1]
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
fn environment_tap(face:u32, xy:vec2<i32>)->vec3<f32> {
    let n=frame.sky.w;
    // Project out-of-face bilinear taps onto their neighbouring face (seam-aware cube lookup).
    let d=environment_direction(face,(vec2<f32>(xy)+0.5)*(2.0/n)-1.0);
    return environment_texels[environment_index(environment_uv(d))].rgb;
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

// PCG hash (Jarzynski and Olano 2020): stateless, so any sample replays bit-identically.
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
    // Duff et al. 2017 branchless orthonormal basis.
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);
    let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}

struct Hit {hit:bool, p:vec3<f32>, n:vec3<f32>, ng:vec3<f32>, material:u32, uv:vec2<f32>, color:vec3<f32>, identity:vec2<u32>};

fn trace(origin:vec3<f32>, direction:vec3<f32>)->Hit {
    var query:ray_query;
    rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,0.0,1e30,origin,direction));
    while rayQueryProceed(&query) {}
    let h=rayQueryGetCommittedIntersection(&query);
    if h.kind==0u {return Hit(false,vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),0u,vec2<f32>(0.0),vec3<f32>(1.0),vec2<u32>(0u));}
    let inst=instances[h.instance_custom_data];
    let base=inst.offsets.y+3u*h.primitive_index;
    let v0=vertices[inst.offsets.x+indices[base]];
    let v1=vertices[inst.offsets.x+indices[base+1u]];
    let v2=vertices[inst.offsets.x+indices[base+2u]];
    let w=vec3<f32>(1.0-h.barycentrics.x-h.barycentrics.y,h.barycentrics);
    let local=v0.a.xyz*w.x+v1.a.xyz*w.y+v2.a.xyz*w.z;
    let p=h.object_to_world*vec4<f32>(local,1.0);
    let normal_matrix=mat3x3<f32>(inst.n0.xyz,inst.n1.xyz,inst.n2.xyz);
    var ng=normalize(normal_matrix*cross(v1.a.xyz-v0.a.xyz,v2.a.xyz-v0.a.xyz));
    let ns=vec3<f32>(v0.a.w,v0.b.xy)*w.x+vec3<f32>(v1.a.w,v1.b.xy)*w.y+vec3<f32>(v2.a.w,v2.b.xy)*w.z;
    var n=normalize(normal_matrix*ns);
    let uv=v0.b.zw*w.x+v1.b.zw*w.y+v2.b.zw*w.z;
    let color=v0.color.rgb*w.x+v1.color.rgb*w.y+v2.color.rgb*w.z;
    let m=materials[inst.offsets.z];
    if m.normal_map.x!=0u {
        let tangent=v0.tangent*w.x+v1.tangent*w.y+v2.tangent*w.z;
        let world_t=h.object_to_world*vec4<f32>(tangent.xyz,0.0);
        let projected=world_t-n*dot(n,world_t);
        var t=normalize(projected);
        if dot(projected,projected)<=1e-12 {
            let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(n.x)>0.9);
            t=normalize(axis-n*dot(n,axis));
        }
        let determinant=dot(cross(h.object_to_world[0],h.object_to_world[1]),h.object_to_world[2]);
        let b=tangent.w*sign(determinant)*cross(n,t);
        let sampled=texture_sample(m.normal_map,uv).xyz*2.0-1.0;
        let scale=max(abs(m.params.z),1.0);
        let decoded=vec3<f32>(sampled.xy*(m.params.z/scale),sampled.z/scale);
        var mapped=vec3<f32>(0.0,0.0,1.0);
        if length(decoded)>1e-6/scale {mapped=normalize(decoded);}
        n=normalize(t*mapped.x+b*mapped.y+n*mapped.z);
    }
    // Two-sided: shade the side the ray arrived from; keep the shading normal in that hemisphere.
    if dot(ng,direction)>0.0 {ng=-ng;}
    if dot(n,ng)<0.0 {n=-n;}
    return Hit(true,p,n,ng,inst.offsets.z,uv,color,vec2<u32>(h.instance_custom_data+1u,h.primitive_index+1u));
}
fn occluded(origin:vec3<f32>, direction:vec3<f32>)->bool {
    var query:ray_query;
    rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,0.0,1e30,origin,direction));
    while rayQueryProceed(&query) {}
    return rayQueryGetCommittedIntersection(&query).kind!=0u;
}
fn offset(p:vec3<f32>, ng:vec3<f32>)->vec3<f32> {
    let m=max(max(abs(p.x),abs(p.y)),abs(p.z));
    return p+ng*(1e-4+4e-7*m);
}

// Radiance arriving from an escaped direction: the sun disk inside its cone, else the sky.
fn in_sun(d:vec3<f32>)->bool {return frame.sun_value.w>0.0 && frame.sun.w<1.0 && dot(d,frame.sun.xyz)>=frame.sun.w;}
fn sun_pdf()->f32 {return 1.0/(2.0*PI*(1.0-frame.sun.w));}

struct Lobe {alpha:f32, a2:f32, f0:vec3<f32>, diffuse:vec3<f32>, spec:f32, p_spec:f32};
fn lobe(m:Material, nv:f32)->Lobe {
    let metal=m.base.w;
    let alpha=max(m.params.x*m.params.x,1e-4);
    let f0=vec3<f32>(0.04*m.params.y)*(1.0-metal)+m.base.rgb*metal;
    let spec=select(1.0,0.0,m.params.y==0.0 && metal==0.0);
    let diffuse=m.base.rgb*((1.0-metal)/PI);
    let fs=luminance(f0+(vec3<f32>(1.0)-f0)*pow(1.0-clamp(nv,0.0,1.0),5.0))*spec;
    let fd=luminance(diffuse)*PI;
    let p=select(fs/max(fs+fd,1e-12),0.0,fs<=0.0);
    return Lobe(alpha,alpha*alpha,f0,diffuse,spec,p);
}
fn ggx_d(l:Lobe, n:vec3<f32>, h:vec3<f32>)->f32 {
    let nh=max(dot(n,h),0.0);
    let c=cross(n,h);
    let denom=max(dot(c,c)+l.a2*nh*nh,1e-30);
    return l.a2/(PI*denom*denom);
}
fn g1(l:Lobe, x:f32)->f32 {return 2.0*x/(x+sqrt(l.a2+(1.0-l.a2)*x*x));}
// f(v, l) without the cosine.
fn bsdf(m:Material, l:Lobe, n:vec3<f32>, v:vec3<f32>, wi:vec3<f32>)->vec3<f32> {
    let nl=dot(n,wi);
    let nv=dot(n,v);
    if nl<=0.0 || nv<=0.0 {return vec3<f32>(0.0);}
    var result=l.diffuse;
    if l.spec>0.0 {
        let h=normalize(wi+v);
        let vh=clamp(dot(v,h),0.0,1.0);
        var vis:f32;
        if frame.options.z==0u {
            vis=0.5/max(nl*sqrt(l.a2+(1.0-l.a2)*nv*nv)+nv*sqrt(l.a2+(1.0-l.a2)*nl*nl),1e-30);
        } else {
            vis=g1(l,nl)*g1(l,nv)/(4.0*nl*nv);
        }
        let fresnel=l.f0+(vec3<f32>(1.0)-l.f0)*pow(1.0-vh,5.0);
        result+=ggx_d(l,n,h)*vis*fresnel;
    }
    return result;
}
// Combined solid-angle pdf of the lobe mixture (one-sample MIS over lobes).
fn bsdf_pdf(l:Lobe, n:vec3<f32>, v:vec3<f32>, wi:vec3<f32>)->f32 {
    let nl=dot(n,wi);
    let nv=dot(n,v);
    if nl<=0.0 || nv<=0.0 {return 0.0;}
    var pdf=(1.0-l.p_spec)*nl/PI;
    if l.p_spec>0.0 {
        let h=normalize(wi+v);
        // Visible-normal pdf: G1(v) D(h) / (4 n.v).
        pdf+=l.p_spec*g1(l,nv)*ggx_d(l,n,h)/(4.0*nv);
    }
    return pdf;
}
// Heitz 2018 visible-normal sampling, in the shading frame.
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
fn camera_ray(pixel:vec2<f32>)->array<vec3<f32>,2> {return camera_ray_time(pixel,rand());}
var<private> reference_centre:Hit;
var<private> sample_direct:vec3<f32>;
var<private> sample_visibility:f32;
var<private> sample_accepted:u32;

fn radiance(pixel:vec2<u32>)->vec3<f32> {
    let jitter=vec2<f32>(rand(),rand());
    let ray=camera_ray(vec2<f32>(pixel)+jitter);
    var origin=ray[0];
    var direction=ray[1];
    var throughput=vec3<f32>(1.0);
    var total=vec3<f32>(0.0);
    var pdf_prev=0.0;  // solid-angle pdf of the BSDF sample that produced `direction`; 0 = camera or delta
    let max_depth=frame.options.x;
    for (var depth=1u; depth<=max_depth; depth+=1u) {
        let hit=trace(origin,direction);
        if depth==1u && frame.environment.w>0.0 && hit.hit && reference_centre.hit {
            let dc=dot(reference_centre.p-frame.eye0.xyz,frame.forward0.xyz);
            let ds=dot(hit.p-frame.eye0.xyz,frame.forward0.xyz);
            // T1b's registered centre-surface condition: placement plus 2% camera-depth band.
            sample_accepted=select(0u,1u,hit.identity.x==reference_centre.identity.x && abs(ds-dc)<=0.02*dc);
        }
        if !hit.hit {
            if depth==1u && frame.background.w>0.0 {return frame.background.rgb;}
            if in_sun(direction) {
                let w=select(power(pdf_prev,sun_pdf()),1.0,pdf_prev==0.0);
                total+=throughput*frame.sun_value.rgb*w;
                if depth==2u {sample_direct+=throughput*frame.sun_value.rgb*w;}
            }
            if frame.sky.w>0.0 || !in_sun(direction) {
                // Constant-sky light sampling is cosine-weighted about the previous normal; its pdf
                // for this direction equals the diffuse part, so the weight uses the stored value.
                let w=select(power(pdf_prev,sky_pdf_prev),1.0,pdf_prev==0.0);
                total+=throughput*sky_radiance(direction)*w;
            }
            break;
        }
        var m=materials[hit.material];
        m.base=vec4<f32>(clamp(m.base.rgb*hit.color*texture_sample(m.base_map,hit.uv).rgb,vec3<f32>(0.0),vec3<f32>(1.0)),m.base.w);
        let mr=texture_sample(m.mr_map,hit.uv);
        m.base.w=clamp(m.base.w*mr.b,0.0,1.0);
        m.params.x=clamp(m.params.x*mr.g,0.0,1.0);
        total+=throughput*m.emission.rgb;
        if depth==max_depth {break;}
        let v=-direction;
        let nv=dot(hit.n,v);
        if nv<=0.0 {break;}
        let l=lobe(m,nv);
        let start=offset(hit.p,hit.ng);
        // Next-event estimation: sun.
        if frame.sun_value.w>0.0 {
            var wi:vec3<f32>;
            var light_pdf=0.0;
            if frame.sun.w>=1.0 {
                wi=frame.sun.xyz;
            } else {
                let z=1.0-rand()*(1.0-frame.sun.w);
                let phi=2.0*PI*rand();
                let r=sqrt(max(0.0,1.0-z*z));
                wi=basis(frame.sun.xyz)*vec3<f32>(r*cos(phi),r*sin(phi),z);
                light_pdf=sun_pdf();
            }
            let cos_l=dot(hit.n,wi);
            let visible=dot(hit.ng,wi)>0.0 && !occluded(start,wi);
            if depth==1u && sample_accepted!=0u {sample_visibility=select(0.0,1.0,visible);}
            if cos_l>0.0 && visible {
                let f=bsdf(m,l,hit.n,v,wi);
                if light_pdf==0.0 {
                    let direct=throughput*f*cos_l*frame.sun_value.rgb;
                    total+=direct;
                    if depth==1u {sample_direct+=direct;}
                } else {
                    let w=power(light_pdf,bsdf_pdf(l,hit.n,v,wi));
                    let direct=throughput*f*cos_l*frame.sun_value.rgb*w/light_pdf;
                    total+=direct;
                    if depth==1u {sample_direct+=direct;}
                }
            }
        }
        // Next-event estimation: constant sky, cosine-weighted about the shading normal.
        if frame.sky.w>0.0 || luminance(frame.sky.rgb)>0.0 {
            var wi:vec3<f32>;
            if frame.sky.w>0.0 {wi=sample_environment();} else {
                let r=sqrt(rand()); let phi=2.0*PI*rand();
                wi=basis(hit.n)*vec3<f32>(r*cos(phi),r*sin(phi),sqrt(max(0.0,1.0-r*r)));
            }
            let cos_l=dot(hit.n,wi);
            if cos_l>0.0 && dot(hit.ng,wi)>0.0 && (frame.sky.w>0.0 || !in_sun(wi)) && !occluded(start,wi) {
                var light_pdf=cos_l/PI;
                if frame.sky.w>0.0 {light_pdf=environment_pdf(wi);}
                let w=power(light_pdf,bsdf_pdf(l,hit.n,v,wi));
                total+=throughput*bsdf(m,l,hit.n,v,wi)*cos_l*sky_radiance(wi)*w/max(light_pdf,1e-30);
            }
        }
        // BSDF sampling for the continuation.
        let frame_n=basis(hit.n);
        var wi:vec3<f32>;
        if rand()<l.p_spec {
            let local_v=transpose(frame_n)*v;
            let h=frame_n*sample_vndf(l,local_v,vec2<f32>(rand(),rand()));
            wi=reflect(-v,h);
        } else {
            let r=sqrt(rand());
            let phi=2.0*PI*rand();
            wi=frame_n*vec3<f32>(r*cos(phi),r*sin(phi),sqrt(max(0.0,1.0-r*r)));
        }
        let cos_i=dot(hit.n,wi);
        if cos_i<=0.0 || dot(hit.ng,wi)<=0.0 {break;}
        let pdf=bsdf_pdf(l,hit.n,v,wi);
        if pdf<=0.0 {break;}
        throughput*=bsdf(m,l,hit.n,v,wi)*cos_i/pdf;
        pdf_prev=pdf;
        sky_pdf_prev=select(0.0,cos_i/PI,luminance(frame.sky.rgb)>0.0);
        if frame.sky.w>0.0 {sky_pdf_prev=environment_pdf(wi);}
        origin=start;
        direction=wi;
        // Russian roulette after three bounces keeps the estimator unbiased.
        if depth>=3u {
            let q=min(max(throughput.r,max(throughput.g,throughput.b)),0.95);
            if rand()>=q {break;}
            throughput/=q;
        }
    }
    return total;
}
var<private> sky_pdf_prev:f32;

@compute @workgroup_size(8,8)
fn trace_samples(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=frame.size.x || id.y>=frame.size.y {return;}
    let pixel=id.y*frame.size.x+id.x;
    var aux:Geometry;
    if frame.environment.w>0.0 {
        let ray=camera_ray_time(vec2<f32>(id.xy)+0.5,0.5);
        reference_centre=trace(ray[0],ray[1]);
        aux=geometry[pixel];
        aux.world=vec4<f32>(reference_centre.p,aux.world.w);
        aux.normal=vec4<f32>(reference_centre.ng,dot(reference_centre.ng,frame.sun.xyz));
        aux.ids=vec4<u32>(reference_centre.identity,reference_centre.material+1u,aux.ids.w);
    }
    var even=vec3<f32>(0.0);
    var odd=vec3<f32>(0.0);
    for (var k=0u; k<frame.size.w; k+=1u) {
        let s=frame.size.z+k;
        rng_state=pcg(pixel^pcg(s^pcg(frame.options.y)));
        sky_pdf_prev=0.0;
        sample_direct=vec3<f32>(0.0);sample_visibility=0.0;sample_accepted=0u;
        var value=radiance(id.xy);
        // A non-finite sample is a tracer bug; it is counted, not hidden.
        if any(value!=value) || any(abs(value)>vec3<f32>(3.0e38)) {value=vec3<f32>(-1.0e30);}
        if (s&1u)==0u {even+=value;} else {odd+=value;}
        if frame.environment.w>0.0 {
            aux.ids.w+=sample_accepted;
            if (s&1u)==0u {aux.direct_even+=vec4<f32>(sample_direct,sample_visibility);aux.world.w+=f32(sample_accepted);}
            else {aux.direct_odd+=vec4<f32>(sample_direct,sample_visibility);}
        }
    }
    sums[2u*pixel]+=vec4<f32>(even,0.0);
    sums[2u*pixel+1u]+=vec4<f32>(odd,0.0);
    if frame.environment.w>0.0 {geometry[pixel]=aux;}
}
