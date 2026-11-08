// ---- AT1 atmosphere: the engine's camera-path application (docs/m7-at1.md) ----
// Composed into every camera-pixel shading module after the shared definitions. Programs
// never call these: the generated entries apply them to each program's exposed output, so
// every shading style gets the same sky, aerial perspective and fog. The world radiance
// cache's cell shading is not composed with them (bounce light carries no camera fog).
// Lighting group bindings 50-53 (src/lighting_gpu.rs); the sampler is the DFG table's
// bilinear clamp at binding 3.
@group(3) @binding(50) var<uniform> atmosphere:AtmosphereUniform;
@group(3) @binding(51) var atmosphere_transmittance:texture_2d<f32>;
@group(3) @binding(52) var atmosphere_skyview:texture_2d<f32>;
@group(3) @binding(53) var atmosphere_volume:texture_3d<f32>;

fn at_exposed(luminance:vec3<f32>) -> vec3<f32> {
    return min(luminance*atmosphere.illuminance.rgb,vec3<f32>(65504.0));
}
// The visible sky along world direction d, exposed: the sky-view LUT plus the sun's disk,
// through the height fog to infinity.
fn atmosphere_sky(d:vec3<f32>) -> vec3<f32> {
    var sky=textureSampleLevel(atmosphere_skyview,dfg_sampler,at_skyview_uv(d),0.0).rgb;
    let radius=atmosphere.albedo.w;
    if at_on(ATMOSPHERE_SUN_DISK) && dot(d,atmosphere.sun.xyz)>cos(radius) {
        // Limb darkening (Neckel and Labs' u=0.6 linear law), mean 1 over the disk.
        let x=acos(clamp(dot(d,atmosphere.sun.xyz),-1.0,1.0))/max(radius,1e-6);
        let mu=sqrt(max(1.0-x*x,0.0));
        let limb=(1.0-0.6*(1.0-mu))/0.8;
        let ground=at_sphere(at_eye_planet(),d,at_bottom());
        if ground<0.0 { sky+=at_sun_transmittance()*limb/(AT_PI*radius*radius); }
    }
    var exposed=at_exposed(sky);
    if at_on(ATMOSPHERE_HEIGHT_FOG) {
        let start=select(atmosphere.fog.w,max(atmosphere.fog.w,atmosphere.volume.x),at_on(ATMOSPHERE_VOLUMETRIC));
        let depth=at_fog_depth(atmosphere.eye.xyz,d,start,1e5);
        let t=exp(-depth);
        exposed=exposed*t+at_exposed(at_fog_source(d))*(1.0-t);
    }
    if at_on(ATMOSPHERE_VOLUMETRIC) {
        let clip=atmosphere.view_projection*vec4<f32>(atmosphere.eye.xyz+d,1.0);
        let ndc=clip.xy/max(clip.w,1e-6);
        let uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);
        let v=at_volume(uv,f32(AT_FROXEL_SLICES),0u,AT_FROXEL_SLICES);
        exposed=exposed*v.a+at_exposed(v.rgb);
    }
    return exposed;
}
// A primary miss: the visible sky when the atmosphere's sky is on, else `fallback`.
fn atmosphere_backdrop(d:vec3<f32>, fallback:vec4<f32>) -> vec4<f32> {
    if !at_on(ATMOSPHERE_SKY) { return fallback; }
    return vec4<f32>(atmosphere_sky(d),1.0);
}
// One camera-volume slice range's (in-scattering, transmittance) at uv and the continuous
// slice coordinate s (0 at the camera, `count` at the range's end), its values stored at
// slice far boundaries from `first`.
fn at_volume(uv:vec2<f32>, s:f32, first:u32, count:u32) -> vec4<f32> {
    let z=clamp(s-1.0,0.0,f32(count)-1.0);
    let w=(f32(first)+z+0.5)/f32(AT_VOLUME_SLICES);
    let v=textureSampleLevel(atmosphere_volume,dfg_sampler,vec3<f32>(uv,w),0.0);
    // Inside the first slice, fade from the camera's identity.
    let f=clamp(s,0.0,1.0);
    return vec4<f32>(v.rgb*f,mix(1.0,v.a,f));
}
// An exposed surface radiance seen from the camera at world point p: through the
// volumetric fog, the analytic height fog beyond it, then the aerial perspective.
fn atmosphere_surface(c:vec3<f32>, p:vec3<f32>) -> vec3<f32> {
    let flags=at_flags();
    if (flags&(ATMOSPHERE_AERIAL|ATMOSPHERE_HEIGHT_FOG|ATMOSPHERE_VOLUMETRIC))==0u { return c; }
    let offset=p-atmosphere.eye.xyz;
    let distance=length(offset);
    if distance<=0.0 { return c; }
    let d=offset/distance;
    let clip=atmosphere.view_projection*vec4<f32>(p,1.0);
    let ndc=clip.xy/max(clip.w,1e-6);
    let uv=clamp(vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5),vec2<f32>(0.0),vec2<f32>(1.0));
    var out=c;
    var start=atmosphere.fog.w;
    if (flags&ATMOSPHERE_VOLUMETRIC)!=0u {
        let v=at_volume(uv,at_froxel_coordinate(distance)*f32(AT_FROXEL_SLICES),0u,AT_FROXEL_SLICES);
        out=out*v.a+at_exposed(v.rgb);
        start=max(start,atmosphere.volume.x);
    }
    if (flags&ATMOSPHERE_HEIGHT_FOG)!=0u {
        let t=exp(-at_fog_depth(atmosphere.eye.xyz,d,start,distance));
        out=out*t+at_exposed(at_fog_source(d))*(1.0-t);
    }
    if (flags&ATMOSPHERE_AERIAL)!=0u {
        let s=sqrt(min(distance*0.001/AT_AERIAL_KM,1.0))*f32(AT_AERIAL_SLICES);
        let v=at_volume(uv,s,AT_FROXEL_SLICES,AT_AERIAL_SLICES);
        out=out*v.a+at_exposed(v.rgb);
    }
    return min(out,vec3<f32>(65504.0));
}
// ---- end of the atmosphere application ----
