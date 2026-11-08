// ---- AT1 atmosphere kernels (src/atmosphere.rs composes shaders/atmosphere_common.wgsl first) ----
// Each kernel has its own derived layout and bind group; a texture is never sampled and
// written in one dispatch.
@group(0) @binding(0) var<uniform> atmosphere:AtmosphereUniform;
@group(0) @binding(1) var atmosphere_transmittance:texture_2d<f32>;
@group(0) @binding(2) var atmosphere_multiscatter:texture_2d<f32>;
@group(0) @binding(3) var atmosphere_sampler:sampler;
@group(0) @binding(4) var at_skyview:texture_2d<f32>;
@group(0) @binding(5) var at_history:texture_3d<f32>;
@group(0) @binding(6) var at_scatter:texture_3d<f32>;
@group(0) @binding(10) var at_transmittance_out:texture_storage_2d<rgba16float,write>;
@group(0) @binding(11) var at_multiscatter_out:texture_storage_2d<rgba16float,write>;
@group(0) @binding(12) var at_skyview_out:texture_storage_2d<rgba16float,write>;
@group(0) @binding(13) var at_volume_out:texture_storage_3d<rgba16float,write>;
@group(0) @binding(15) var at_scatter_out:texture_storage_3d<rgba16float,write>;

const AT_MULTISCATTER_SIZE:f32=32.0;

// Transmittance to the top of the atmosphere (paper section 4.1), 40 midpoint steps.
@compute @workgroup_size(8,8) fn at_transmittance_lut(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=AT_TRANSMITTANCE_SIZE;
    if any(id.xy>=size) { return; }
    let x_mu=at_uv_to_unit((f32(id.x)+0.5)/f32(size.x),f32(size.x));
    let x_r=at_uv_to_unit((f32(id.y)+0.5)/f32(size.y),f32(size.y));
    let bottom=at_bottom();let top=at_top();
    let h=sqrt(top*top-bottom*bottom);
    let rho=h*x_r;
    let r=sqrt(rho*rho+bottom*bottom);
    let d_min=top-r;let d_max=rho+h;
    let d=d_min+x_mu*(d_max-d_min);
    var mu=1.0;
    if d>0.0 { mu=clamp((h*h-rho*rho-d*d)/(2.0*r*d),-1.0,1.0); }
    let p=vec3<f32>(0.0,r,0.0);
    let dir=vec3<f32>(sqrt(max(1.0-mu*mu,0.0)),mu,0.0);
    let t=max(at_sphere(p,dir,top),0.0);
    var depth=vec3<f32>(0.0);
    for(var i=0u;i<40u;i+=1u) {
        let s=t*(f32(i)+0.5)/40.0;
        depth+=at_medium(length(p+dir*s)-bottom).extinction*(t/40.0);
    }
    textureStore(at_transmittance_out,vec2<i32>(id.xy),vec4<f32>(exp(-depth),1.0));
}

// The multiple-scattering transfer Psi_ms (paper section 5.5): per (sun zenith cosine,
// altitude), 64 directions of second-order luminance L2 and transfer f_ms, Psi = L2/(1-f_ms).
var<workgroup> at_ms_l:array<vec3<f32>,64>;
var<workgroup> at_ms_f:array<vec3<f32>,64>;
@compute @workgroup_size(64) fn at_multiscatter_lut(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let uv=(vec2<f32>(group.xy)+vec2<f32>(0.5))/AT_MULTISCATTER_SIZE;
    let mu_s=uv.x*2.0-1.0;
    let bottom=at_bottom();let top=at_top();
    let r=clamp(bottom+uv.y*(top-bottom),bottom+1e-3,top-1e-3);
    let p=vec3<f32>(0.0,r,0.0);
    let sun=vec3<f32>(sqrt(max(1.0-mu_s*mu_s,0.0)),mu_s,0.0);
    let a=(f32(li%8u)+0.5)/8.0;let b=(f32(li/8u)+0.5)/8.0;
    let cos_t=1.0-2.0*a;let sin_t=sqrt(max(1.0-cos_t*cos_t,0.0));
    let phi=2.0*AT_PI*b;
    let d=vec3<f32>(sin_t*cos(phi),cos_t,sin_t*sin(phi));
    let ground=at_sphere(p,d,bottom);
    let t_max=select(max(at_sphere(p,d,top),0.0),ground,ground>0.0);
    var l=vec3<f32>(0.0);var f=vec3<f32>(0.0);var throughput=vec3<f32>(1.0);
    let dt=t_max/20.0;
    for(var i=0u;i<20u;i+=1u) {
        let q=p+d*(dt*(f32(i)+0.5));
        let rq=length(q);
        let m=at_medium(rq-bottom);
        let segment=exp(-m.extinction*dt);
        let lit=select(1.0,0.0,at_sphere(q,sun,bottom)>0.0);
        let s=m.scattering*at_transmittance(rq,dot(q/rq,sun))*lit/(4.0*AT_PI);
        let inv=1.0/max(m.extinction,vec3<f32>(1e-7));
        l+=throughput*(s-s*segment)*inv;
        f+=throughput*(m.scattering-m.scattering*segment)*inv;
        throughput*=segment;
    }
    if ground>0.0 {
        let q=p+d*ground;let up=normalize(q);let mu=dot(up,sun);
        l+=throughput*at_transmittance(bottom,mu)*max(mu,0.0)*atmosphere.albedo.rgb/AT_PI;
    }
    at_ms_l[li]=l;at_ms_f[li]=f;
    workgroupBarrier();
    if li==0u {
        var sl=vec3<f32>(0.0);var sf=vec3<f32>(0.0);
        for(var k=0u;k<64u;k+=1u) { sl+=at_ms_l[k];sf+=at_ms_f[k]; }
        let psi=(sl/64.0)/max(vec3<f32>(1.0)-sf/64.0,vec3<f32>(1e-3));
        textureStore(at_multiscatter_out,vec2<i32>(group.xy),vec4<f32>(psi,1.0));
    }
}
fn at_multiscatter(r:f32, mu_s:f32) -> vec3<f32> {
    let u=clamp(mu_s*0.5+0.5,0.0,1.0);
    let v=clamp((r-at_bottom())/(at_top()-at_bottom()),0.0,1.0);
    let uv=vec2<f32>(at_unit_to_uv(u,AT_MULTISCATTER_SIZE),at_unit_to_uv(v,AT_MULTISCATTER_SIZE));
    return textureSampleLevel(atmosphere_multiscatter,atmosphere_sampler,uv,0.0).rgb;
}

// In-scattered luminance per unit sun illuminance along p + d t over [t0, t1] (km), with
// `steps` segments, plus the transmittance over it; single scattering with both phases and
// the multiple-scattering term (paper section 5.3).
struct AtSegment { luminance:vec3<f32>, transmittance:vec3<f32> };
fn at_integrate(p:vec3<f32>, d:vec3<f32>, t0:f32, t1:f32, steps:u32, start:AtSegment) -> AtSegment {
    var out=start;
    let sun=atmosphere.sun.xyz;
    let c=dot(d,sun);
    let pr=at_rayleigh_phase(c);let pm=at_mie_phase(atmosphere.mie_absorption.w,c);
    let dt=(t1-t0)/f32(steps);
    for(var i=0u;i<steps;i+=1u) {
        let q=p+d*(t0+dt*(f32(i)+0.5));
        let rq=length(q);
        let m=at_medium(rq-at_bottom());
        let segment=exp(-m.extinction*dt);
        let mu=dot(q/rq,sun);
        let lit=select(1.0,0.0,at_sphere(q,sun,at_bottom())>0.0);
        let s=(m.rayleigh*pr+m.mie*pm)*at_transmittance(rq,mu)*lit+m.scattering*at_multiscatter(rq,mu);
        out.luminance+=out.transmittance*(s-s*segment)/max(m.extinction,vec3<f32>(1e-7));
        out.transmittance*=segment;
    }
    return out;
}

// The sky-view LUT (paper section 5.2): luminance toward the camera per unit sun
// illuminance, latitude compressed about the horizon, longitude relative to the sun's
// azimuth (symmetric, so [0, pi]). Rays into the ground end at a Lambertian ground.
@compute @workgroup_size(8,8) fn at_skyview_lut(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(at_skyview_out);
    if any(id.xy>=size) { return; }
    let uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(size);
    let d=at_skyview_direction(uv);
    let p=at_eye_planet();
    let ground=at_sphere(p,d,at_bottom());
    let top=at_sphere(p,d,at_top());
    var segment=AtSegment(vec3<f32>(0.0),vec3<f32>(1.0));
    if top>0.0 {
        let t_max=select(top,ground,ground>0.0);
        // Squared spacing: short steps near the camera, where the medium is densest.
        let steps=30u;
        var t_prev=0.0;
        for(var i=0u;i<steps;i+=1u) {
            let x=(f32(i)+1.0)/f32(steps);
            let t=t_max*x*x;
            segment=at_integrate(p,d,t_prev,t,1u,segment);
            t_prev=t;
        }
        if ground>0.0 {
            let q=p+d*ground;let up=normalize(q);let mu=dot(up,atmosphere.sun.xyz);
            // Direct sun only, as the paper's ground.
            let irradiance=at_transmittance(at_bottom(),mu)*max(mu,0.0);
            segment.luminance+=segment.transmittance*irradiance*atmosphere.albedo.rgb/AT_PI;
        }
    }
    textureStore(at_skyview_out,vec2<i32>(id.xy),vec4<f32>(segment.luminance,1.0));
}
// The sky-view LUT's direction at uv, in world axes (y up; the sun's azimuth is fixed).
fn at_skyview_direction(uv:vec2<f32>) -> vec3<f32> {
    let r=at_bottom()+atmosphere.eye.w;
    let v_horizon=sqrt(max(r*r-at_bottom()*at_bottom(),0.0));
    let beta=acos(clamp(v_horizon/r,-1.0,1.0));
    let zenith_horizon=AT_PI-beta;
    var zenith:f32;
    if uv.y<0.5 {
        let c=1.0-2.0*uv.y;
        zenith=zenith_horizon*(1.0-c*c);
    } else {
        let c=uv.y*2.0-1.0;
        zenith=zenith_horizon+beta*c*c;
    }
    let cos_v=cos(zenith);let sin_v=sin(zenith);
    let light_view=-(uv.x*uv.x*2.0-1.0);
    let local=vec3<f32>(sin_v*light_view,cos_v,sin_v*sqrt(max(1.0-light_view*light_view,0.0)));
    // Local x is the sun's horizontal azimuth.
    let s=atmosphere.sun.xyz;
    let h=vec2<f32>(s.x,s.z);
    let a=select(vec2<f32>(1.0,0.0),normalize(h),dot(h,h)>1e-12);
    return vec3<f32>(a.x*local.x-a.y*local.z,local.y,a.y*local.x+a.x*local.z);
}

// The fog's ambient: the sky's mean radiance over the sphere, from the sky-view LUT,
// weighted by solid angle. Stored in the transmittance texture's extra row.
var<workgroup> at_ambient_sum:array<vec3<f32>,64>;
@compute @workgroup_size(64) fn at_ambient_mean(@builtin(local_invocation_index) li:u32) {
    let size=vec2<f32>(textureDimensions(at_skyview));
    var sum=vec3<f32>(0.0);
    // 16 x 16 directions on the sphere, each thread four of them.
    for(var k=0u;k<4u;k+=1u) {
        let i=li*4u+k;
        let a=(f32(i%16u)+0.5)/16.0;let b=(f32(i/16u)+0.5)/16.0;
        let cos_t=1.0-2.0*a;let sin_t=sqrt(max(1.0-cos_t*cos_t,0.0));
        let phi=2.0*AT_PI*b;
        let d=vec3<f32>(sin_t*cos(phi),cos_t,sin_t*sin(phi));
        sum+=textureSampleLevel(at_skyview,atmosphere_sampler,at_skyview_uv(d),0.0).rgb;
    }
    at_ambient_sum[li]=sum;
    workgroupBarrier();
    if li==0u {
        var total=vec3<f32>(0.0);
        for(var k=0u;k<64u;k+=1u) { total+=at_ambient_sum[k]; }
        textureStore(at_transmittance_out,vec2<i32>(0,i32(AT_TRANSMITTANCE_SIZE.y)),vec4<f32>(total/256.0,1.0));
    }
}

// The camera volume's aerial perspective (paper section 5.4), at slices 64..96: each column
// marches its camera ray, storing the in-scattered luminance and mean transmittance up to
// each slice's far boundary.
@compute @workgroup_size(8,8) fn at_aerial_volume(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=atmosphere.grid.xy;
    if any(id.xy>=grid) { return; }
    let uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(grid);
    let d=at_view_direction(at_uv_ndc(uv));
    let p=at_eye_planet();
    var segment=AtSegment(vec3<f32>(0.0),vec3<f32>(1.0));
    var t_prev=0.0;
    let top=max(at_sphere(p,d,at_top()),0.0);
    for(var k=0u;k<AT_AERIAL_SLICES;k+=1u) {
        let t=min(at_aerial_distance(f32(k+1u)/f32(AT_AERIAL_SLICES)),top);
        segment=at_integrate(p,d,t_prev,max(t,t_prev),2u,segment);
        t_prev=max(t,t_prev);
        let mean=dot(segment.transmittance,vec3<f32>(1.0/3.0));
        textureStore(at_volume_out,vec3<i32>(vec2<i32>(id.xy),i32(AT_FROXEL_SLICES+k)),vec4<f32>(segment.luminance,mean));
    }
}

// R2 sequence offset for frame k, in [0, 1)^3.
fn at_jitter(k:u32) -> vec3<f32> {
    let g=1.22074408460575947536;
    let a=vec3<f32>(1.0/g,1.0/(g*g),1.0/(g*g*g));
    return fract(vec3<f32>(0.5)+a*f32(k%4096u));
}

// Volumetric fog injection: each froxel's in-scattered radiance per unit sun illuminance
// and extinction (/m) at a jittered point, the sun shadowed by `at_sun_visible` (composed
// per path by src/atmosphere.rs), blended with its reprojected history.
@compute @workgroup_size(4,4,4) fn at_fog_inject(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=vec3<u32>(atmosphere.grid.xy,AT_FROXEL_SLICES);
    if any(id>=grid) { return; }
    let j=at_jitter(atmosphere.grid.z);
    let uv=(vec2<f32>(id.xy)+j.xy)/vec2<f32>(grid.xy);
    let d=at_view_direction(at_uv_ndc(uv));
    let distance=at_froxel_distance((f32(id.z)+j.z)/f32(grid.z));
    let world=atmosphere.eye.xyz+d*distance;
    let extinction=select(0.0,at_fog_density(world.y)+atmosphere.volume.y,distance>=atmosphere.fog.w);
    let sun=at_sun_transmittance()*at_hg(atmosphere.fog_albedo.w,dot(d,atmosphere.sun.xyz));
    let ambient=at_ambient();
    let scattering=atmosphere.fog_albedo.rgb*extinction;
    var value=vec4<f32>(scattering*(sun*at_sun_visible(world)+ambient),extinction);
    if atmosphere.previous_eye.w>0.0 {
        // Reproject the cell centre, not this frame's jittered integration sample.
        // Otherwise even a stationary camera repeatedly blurs history across cells.
        let centre_uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(grid.xy);
        let centre=atmosphere.eye.xyz+at_view_direction(at_uv_ndc(centre_uv))*at_froxel_distance((f32(id.z)+0.5)/f32(grid.z));
        let clip=atmosphere.previous_view_projection*vec4<f32>(centre,1.0);
        if clip.w>0.0 {
            let ndc=clip.xy/clip.w;
            let previous=vec3<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5,at_froxel_coordinate(length(centre-atmosphere.previous_eye.xyz)));
            let border=vec3<f32>(0.5)/vec3<f32>(grid);
            if all(previous>=border) && all(previous<=vec3<f32>(1.0)-border) {
                let history=textureSampleLevel(at_history,atmosphere_sampler,previous,0.0);
                // Reject density discontinuities; limit old lighting to this cell's
                // physical lit/shadowed range. Rapid visibility changes get less lag.
                let density_error=abs(history.a-extinction)/max(extinction,1e-6);
                let lo=scattering*ambient;let hi=scattering*(ambient+sun);
                let old=clamp(history.rgb,lo,hi);
                let delta=abs(old-value.rgb)/max(hi-lo,vec3<f32>(1e-6));
                let change=max(delta.x,max(delta.y,delta.z));
                let weight=atmosphere.volume.z*(1.0-smoothstep(0.1,0.5,density_error))*mix(1.0,0.3,smoothstep(0.25,0.75,change));
                value=vec4<f32>(mix(value.rgb,old,weight),extinction);
            }
        }
    }
    textureStore(at_scatter_out,vec3<i32>(id),value);
}

// Volumetric fog integration: each column accumulates in-scattering and transmittance
// front to back, storing them at each slice's far boundary (camera volume slices 0..64).
@compute @workgroup_size(8,8) fn at_fog_integrate(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=atmosphere.grid.xy;
    if any(id.xy>=grid) { return; }
    var luminance=vec3<f32>(0.0);var transmittance=1.0;
    var near=0.0;
    for(var k=0u;k<AT_FROXEL_SLICES;k+=1u) {
        let far=at_froxel_distance(f32(k+1u)/f32(AT_FROXEL_SLICES));
        let s=textureLoad(at_scatter,vec3<i32>(vec2<i32>(id.xy),i32(k)),0);
        let thickness=far-near;
        let segment=exp(-s.w*thickness);
        luminance+=transmittance*s.rgb*(1.0-segment)/max(s.w,1e-7);
        transmittance*=segment;
        near=far;
        textureStore(at_volume_out,vec3<i32>(vec2<i32>(id.xy),i32(k)),vec4<f32>(luminance,transmittance));
    }
}

// ---- Procedural environment source (Atmosphere::environment) ----
// The sky (no sun disk) as an environment in the legacy mip-chain layout scene.wgsl reads:
// a prefiltered specular chain whose level i holds GGX roughness i/(levels-1), and a diffuse
// irradiance cube, both unexposed (cd/m2 and lux) in the cube convention of
// environment_face_direction.
@group(0) @binding(16) var at_specular_out:texture_storage_2d_array<rgba32float,write>;
@group(0) @binding(17) var at_diffuse_out:texture_storage_2d_array<rgba32float,write>;
// src/atmosphere_gpu.rs ENVIRONMENT_SIZE and ENVIRONMENT_LEVELS must match.
const AT_ENVIRONMENT_SIZE:u32=128u;
const AT_ENVIRONMENT_LEVELS:u32=5u;
fn at_face_direction(face:u32, uv:vec2<f32>) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(1.0,-uv.y,-uv.x); }
        case 1u: { return vec3<f32>(-1.0,-uv.y,uv.x); }
        case 2u: { return vec3<f32>(uv.x,1.0,uv.y); }
        case 3u: { return vec3<f32>(uv.x,-1.0,-uv.y); }
        case 4u: { return vec3<f32>(uv.x,-uv.y,1.0); }
        default: { return vec3<f32>(-uv.x,-uv.y,-1.0); }
    }
}
// The sky's radiance (cd/m2) toward the camera from world direction d.
fn at_sky_radiance(d:vec3<f32>) -> vec3<f32> {
    let lux=atmosphere.illuminance.rgb/max(atmosphere.illuminance.w,1e-30);
    return textureSampleLevel(at_skyview,atmosphere_sampler,at_skyview_uv(d),0.0).rgb*lux;
}
fn at_hammersley(i:u32, n:u32) -> vec2<f32> {
    return vec2<f32>((f32(i)+0.5)/f32(n),f32(reverseBits(i))*2.3283064365386963e-10);
}
fn at_frame(n:vec3<f32>) -> mat3x3<f32> {
    let up=select(vec3<f32>(0.0,1.0,0.0),vec3<f32>(1.0,0.0,0.0),abs(n.y)>0.9);
    let t=normalize(cross(up,n));
    return mat3x3<f32>(t,cross(n,t),n);
}
fn at_texel_direction(id:vec3<u32>, size:u32) -> vec3<f32> {
    let uv=2.0*(vec2<f32>(id.xy)+vec2<f32>(0.5))/f32(size)-vec2<f32>(1.0);
    return normalize(at_face_direction(id.z,uv));
}
// One specular level: N=V=R GGX prefiltering by importance sampling, 64 samples.
@compute @workgroup_size(8,8,1) fn at_environment_specular(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(at_specular_out).x;
    if any(id.xy>=vec2<u32>(size)) || id.z>=6u { return; }
    let n=at_texel_direction(id,size);
    let level=log2(f32(AT_ENVIRONMENT_SIZE)/f32(size));
    let r=level/f32(AT_ENVIRONMENT_LEVELS-1u);
    var value=at_sky_radiance(n);
    if r>0.0 {
        let a=r*r;
        let frame=at_frame(n);
        var sum=vec3<f32>(0.0);var weight=0.0;
        for(var i=0u;i<64u;i+=1u) {
            let xi=at_hammersley(i,64u);
            let cos_t=sqrt((1.0-xi.y)/(1.0+(a*a-1.0)*xi.y));
            let sin_t=sqrt(max(1.0-cos_t*cos_t,0.0));
            let phi=2.0*AT_PI*xi.x;
            let h=frame*vec3<f32>(sin_t*cos(phi),sin_t*sin(phi),cos_t);
            let l=2.0*dot(n,h)*h-n;
            let nl=dot(n,l);
            if nl>0.0 { sum+=at_sky_radiance(l)*nl;weight+=nl; }
        }
        value=sum/max(weight,1e-6);
    }
    textureStore(at_specular_out,vec2<i32>(id.xy),i32(id.z),vec4<f32>(value,1.0));
}
// Diffuse irradiance (lux): cosine-weighted, 256 samples.
@compute @workgroup_size(8,8,1) fn at_environment_diffuse(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(at_diffuse_out).x;
    if any(id.xy>=vec2<u32>(size)) || id.z>=6u { return; }
    let n=at_texel_direction(id,size);
    let frame=at_frame(n);
    var sum=vec3<f32>(0.0);
    for(var i=0u;i<256u;i+=1u) {
        let xi=at_hammersley(i,256u);
        let r=sqrt(xi.y);let phi=2.0*AT_PI*xi.x;
        let l=frame*vec3<f32>(r*cos(phi),r*sin(phi),sqrt(max(1.0-xi.y,0.0)));
        sum+=at_sky_radiance(l);
    }
    textureStore(at_diffuse_out,vec2<i32>(id.xy),i32(id.z),vec4<f32>(AT_PI*sum/256.0,1.0));
}
