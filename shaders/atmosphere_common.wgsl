// ---- AT1 atmosphere and fog: shared definitions (docs/m7-at1.md) ----
// A Hillaire-2020 sky (EGSR 2020, "A Scalable and Production Ready Sky and Atmosphere
// Rendering Technique"), written from the paper: a transmittance LUT, a multiple-scattering
// LUT, a per-frame sky-view LUT and a camera volume holding the aerial perspective and the
// volumetric fog froxels. Every LUT stores values per unit sun illuminance outside the
// atmosphere; `atmosphere.illuminance` scales them to exposed radiance. Atmosphere lengths
// are kilometres about the planet centre; fog lengths are world metres. The kernels
// (shaders/atmosphere.wgsl) and the shading API (shaders/atmosphere_api.wgsl) each declare
// `atmosphere`, `atmosphere_transmittance` and the sampler these functions read.
struct AtmosphereUniform {
    // rgb Rayleigh scattering (/km), w 1/scale height (/km).
    rayleigh:vec4<f32>,
    // rgb Mie scattering (/km), w 1/scale height (/km).
    mie:vec4<f32>,
    // rgb Mie absorption (/km), w Mie phase g.
    mie_absorption:vec4<f32>,
    // rgb ozone absorption (/km) at its peak, w peak altitude (km).
    ozone:vec4<f32>,
    // Ground radius (km), top radius (km), the ground's world y (m), ozone half width (km).
    planet:vec4<f32>,
    // Ground albedo, w sun angular radius (rad).
    albedo:vec4<f32>,
    // Unit direction toward the sun, w flag bits (ATMOSPHERE_*) as f32 bits.
    sun:vec4<f32>,
    // Sun illuminance outside the atmosphere times exposure: LUT values to exposed HDR;
    // w the exposure.
    illuminance:vec4<f32>,
    // Camera (m), w camera altitude (km, clamped above the ground).
    eye:vec4<f32>,
    // Height fog: extinction (/m) at the base height, falloff (/m), base world y (m), start (m).
    fog:vec4<f32>,
    // Fog single-scattering albedo, w its Henyey-Greenstein g.
    fog_albedo:vec4<f32>,
    // Volumetric fog: range (m), extra uniform extinction (/m), history weight, unused.
    volume:vec4<f32>,
    // The previous frame's camera (m), w 1 when history may be read.
    previous_eye:vec4<f32>,
    view_projection:mat4x4<f32>,
    inverse_view_projection:mat4x4<f32>,
    previous_view_projection:mat4x4<f32>,
    // Camera volume froxel columns (x, y), the frame index and the sun-visibility source.
    grid:vec4<u32>,
    // The target's width and height; conventional-map sun view first/count (VF1).
    viewport:vec4<f32>,
};
const ATMOSPHERE_SKY:u32=1u;
const ATMOSPHERE_SUN_DISK:u32=2u;
const ATMOSPHERE_AERIAL:u32=4u;
const ATMOSPHERE_HEIGHT_FOG:u32=8u;
const ATMOSPHERE_VOLUMETRIC:u32=16u;
const AT_PI:f32=3.141592653589793;
// Camera volume slices: froxels first, then the aerial perspective (src/atmosphere.rs).
const AT_FROXEL_SLICES:u32=64u;
const AT_AERIAL_SLICES:u32=32u;
const AT_VOLUME_SLICES:u32=96u;
// Aerial perspective reach (km) and the froxel distribution's exponent.
const AT_AERIAL_KM:f32=32.0;
const AT_FROXEL_EXPONENT:f32=5.0;
// Transmittance LUT rows; one more row holds the sky's mean radiance (fog ambient).
const AT_TRANSMITTANCE_SIZE=vec2<u32>(256u,64u);

fn at_flags() -> u32 { return bitcast<u32>(atmosphere.sun.w); }
fn at_on(flag:u32) -> bool { return (at_flags()&flag)!=0u; }
fn at_bottom() -> f32 { return atmosphere.planet.x; }
fn at_top() -> f32 { return atmosphere.planet.y; }

struct AtMedium { scattering:vec3<f32>, extinction:vec3<f32>, rayleigh:vec3<f32>, mie:vec3<f32> };
// Scattering and extinction (/km) at altitude h (km).
fn at_medium(h:f32) -> AtMedium {
    let dr=exp(-max(h,0.0)*atmosphere.rayleigh.w);
    let dm=exp(-max(h,0.0)*atmosphere.mie.w);
    let dozone=max(0.0,1.0-abs(h-atmosphere.ozone.w)/atmosphere.planet.w);
    let r=atmosphere.rayleigh.xyz*dr;
    let m=atmosphere.mie.xyz*dm;
    return AtMedium(r+m,r+m+atmosphere.mie_absorption.xyz*dm+atmosphere.ozone.xyz*dozone,r,m);
}
// The nearest nonnegative distance from p along unit d to the sphere of `radius` about the
// planet centre, or -1 when the ray misses it.
fn at_sphere(p:vec3<f32>, d:vec3<f32>, radius:f32) -> f32 {
    let b=dot(p,d);
    let c=dot(p,p)-radius*radius;
    let disc=b*b-c;
    if disc<0.0 { return -1.0; }
    let s=sqrt(disc);
    if -b-s>=0.0 { return -b-s; }
    if -b+s>=0.0 { return -b+s; }
    return -1.0;
}
fn at_rayleigh_phase(c:f32) -> f32 { return 3.0/(16.0*AT_PI)*(1.0+c*c); }
// Cornette-Shanks, the paper's Mie phase.
fn at_mie_phase(g:f32, c:f32) -> f32 {
    let k=3.0/(8.0*AT_PI)*(1.0-g*g)/(2.0+g*g);
    return k*(1.0+c*c)/pow(max(1.0+g*g-2.0*g*c,1e-6),1.5);
}
fn at_hg(g:f32, c:f32) -> f32 {
    let d=max(1.0+g*g-2.0*g*c,1e-6);
    return (1.0-g*g)/(4.0*AT_PI*d*sqrt(d));
}
// Texel-centre mapping of a unit coordinate (Bruneton's GetTextureCoordFromUnitRange).
fn at_unit_to_uv(x:f32, size:f32) -> f32 { return 0.5/size+x*(1.0-1.0/size); }
fn at_uv_to_unit(u:f32, size:f32) -> f32 { return (u-0.5/size)/(1.0-1.0/size); }
// Transmittance LUT coordinates of a ray from radius r (km) with zenith cosine mu, toward
// the top of the atmosphere (Bruneton 2017's parameterization).
fn at_transmittance_uv(r:f32, mu:f32) -> vec2<f32> {
    let bottom=at_bottom();let top=at_top();
    let h=sqrt(max(top*top-bottom*bottom,0.0));
    let rho=sqrt(max(r*r-bottom*bottom,0.0));
    let disc=r*r*(mu*mu-1.0)+top*top;
    let d=max(0.0,-r*mu+sqrt(max(disc,0.0)));
    let d_min=top-r;let d_max=rho+h;
    let x_mu=(d-d_min)/max(d_max-d_min,1e-6);
    let x_r=rho/max(h,1e-6);
    let size=vec2<f32>(AT_TRANSMITTANCE_SIZE);
    // The LUT occupies rows 0..64 of a 65-row texture: v is scaled into them.
    return vec2<f32>(at_unit_to_uv(x_mu,size.x),at_unit_to_uv(x_r,size.y)*size.y/(size.y+1.0));
}
// Transmittance from radius r with zenith cosine mu to the top of the atmosphere; zero once
// the ray enters the ground.
fn at_transmittance(r:f32, mu:f32) -> vec3<f32> {
    let horizon=-sqrt(max(1.0-at_bottom()*at_bottom()/(r*r),0.0));
    if mu<horizon { return vec3<f32>(0.0); }
    return textureSampleLevel(atmosphere_transmittance,atmosphere_sampler,at_transmittance_uv(r,mu),0.0).rgb;
}
// The sky's mean radiance over the sphere at the camera, per unit sun illuminance.
fn at_ambient() -> vec3<f32> {
    return textureLoad(atmosphere_transmittance,vec2<i32>(0,i32(AT_TRANSMITTANCE_SIZE.y)),0).rgb;
}
// The camera's position in kilometres about the planet centre (world y up, no curvature
// beneath the camera).
fn at_eye_planet() -> vec3<f32> { return vec3<f32>(0.0,at_bottom()+atmosphere.eye.w,0.0); }
// Transmittance toward the sun from the camera's altitude.
fn at_sun_transmittance() -> vec3<f32> {
    return at_transmittance(at_bottom()+atmosphere.eye.w,atmosphere.sun.y);
}

// Height fog extinction (/m) at world height y.
fn at_fog_density(y:f32) -> f32 {
    return atmosphere.fog.x*exp(clamp(-atmosphere.fog.y*(y-atmosphere.fog.z),-80.0,80.0));
}
// The height fog's optical depth from o along unit d over [t0, t1] metres, in closed form.
fn at_fog_depth(o:vec3<f32>, d:vec3<f32>, t0:f32, t1:f32) -> f32 {
    if t1<=t0 || atmosphere.fog.x<=0.0 { return 0.0; }
    let k=atmosphere.fog.y*d.y;
    let base=at_fog_density(o.y);
    if abs(k*(t1-t0))<1e-4 { return min(base*(t1-t0)*exp(clamp(-k*0.5*(t0+t1),-80.0,80.0)),1e4); }
    return min(base*(exp(clamp(-k*t0,-80.0,80.0))-exp(clamp(-k*t1,-80.0,80.0)))/k,1e4);
}
// The fog's in-scattered radiance per unit sun illuminance toward the camera along d: the
// sun through the atmosphere (unshadowed) and the sky's mean radiance.
fn at_fog_source(d:vec3<f32>) -> vec3<f32> {
    let sun=at_sun_transmittance()*at_hg(atmosphere.fog_albedo.w,dot(-d,-atmosphere.sun.xyz));
    return atmosphere.fog_albedo.rgb*(sun+at_ambient());
}

// Froxel slice boundary distance (m) at unit slice coordinate w, and its inverse.
fn at_froxel_distance(w:f32) -> f32 {
    return atmosphere.volume.x*(exp2(AT_FROXEL_EXPONENT*w)-1.0)/(exp2(AT_FROXEL_EXPONENT)-1.0);
}
fn at_froxel_coordinate(d:f32) -> f32 {
    return log2(max(d,0.0)/max(atmosphere.volume.x,1e-6)*(exp2(AT_FROXEL_EXPONENT)-1.0)+1.0)/AT_FROXEL_EXPONENT;
}
// Aerial perspective slice boundary distance (km) at unit slice coordinate w.
fn at_aerial_distance(w:f32) -> f32 { return AT_AERIAL_KM*w*w; }
// A camera ray's direction through the jittered view's NDC xy.
fn at_view_direction(ndc:vec2<f32>) -> vec3<f32> {
    let p=atmosphere.inverse_view_projection*vec4<f32>(ndc,1.0,1.0);
    return normalize(p.xyz/p.w-atmosphere.eye.xyz);
}
fn at_uv_ndc(uv:vec2<f32>) -> vec2<f32> { return vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0); }
// The sky-view LUT coordinates of world direction d (the inverse of at_skyview_direction).
fn at_skyview_uv(d:vec3<f32>) -> vec2<f32> {
    let r=at_bottom()+atmosphere.eye.w;
    let v_horizon=sqrt(max(r*r-at_bottom()*at_bottom(),0.0));
    let beta=acos(clamp(v_horizon/r,-1.0,1.0));
    let zenith_horizon=AT_PI-beta;
    let zenith=acos(clamp(d.y,-1.0,1.0));
    var v:f32;
    if zenith<zenith_horizon {
        let c=sqrt(max(1.0-zenith/zenith_horizon,0.0));
        v=(1.0-c)*0.5;
    } else {
        v=sqrt(clamp((zenith-zenith_horizon)/max(beta,1e-6),0.0,1.0))*0.5+0.5;
    }
    let s=vec2<f32>(atmosphere.sun.x,atmosphere.sun.z);
    let h=vec2<f32>(d.x,d.z);
    var light_view=1.0;
    if dot(s,s)>1e-12 && dot(h,h)>1e-12 { light_view=dot(normalize(s),normalize(h)); }
    return vec2<f32>(sqrt(clamp(-light_view*0.5+0.5,0.0,1.0)),v);
}
// ---- end of the shared atmosphere definitions ----
