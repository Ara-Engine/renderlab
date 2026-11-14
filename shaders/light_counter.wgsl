// Measurement only: independent SSBO, no counter values are written to HDR.
struct LcCounter { header:vec4<u32>, bins:array<atomic<u32>> };
@group(0) @binding(80) var<storage,read_write> lc_counter:LcCounter;
var<private> lc_primary:bool;
var<private> lc_current:bool;
var<private> lc_seen:u32;
var<private> lc_range:u32;
var<private> lc_source:u32;
var<private> lc_view:u32;
var<private> lc_face_pass:u32;
var<private> lc_disagree:u32;
var<private> lc_protected:u32;
var<private> lc_view_any:u32;
var<private> lc_calls:u32;
var<private> lc_rays:u32;
var<private> lc_without_rgb:vec3<f32>;
fn lc_capture(input:SurfaceInput,n:vec3<f32>,s:LightSample) {
    if !lc_primary || s.index!=lc_counter.header.y {return;}
    lc_seen=1u;
    lc_range=u32(s.attenuation==0.0);
    let source=dot(input.geometric,s.direction)>0.0;
    let viewed=dot(n,s.direction)>0.0;
    lc_protected=u32(input.material.double_sided!=0u || input.receiver.thin);
    if lc_range==0u {
        lc_source=u32(!source);lc_view=u32(source && !viewed);
        lc_face_pass=u32(source && viewed);lc_view_any=u32(!viewed);
        lc_disagree=u32(lc_protected!=0u && source!=viewed);
    }
}
// Same CC0 Narkowicz fit as post_tone.wgsl; grading/bloom/TR1 intentionally excluded.
fn lc_display(c:vec3<f32>)->vec3<f32> {
    let x=max(c,vec3<f32>(0.0));
    let t=clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14),vec3<f32>(0.0),vec3<f32>(1.0));
    return select(t*12.92,1.055*pow(t,vec3<f32>(1.0/2.4))-0.055,t>vec3<f32>(0.0031308));
}
fn lc_commit(full:vec3<f32>,without:vec3<f32>,input:SurfaceInput) {
    let delta=abs(lc_display(atmosphere_surface(rl_finite(full),input.world))-lc_display(atmosphere_surface(rl_finite(without),input.world)))*255.0;
    let codes=max(max(delta.x,delta.y),delta.z);
    let visible=u32(codes>=1.0);let traced=u32(lc_rays>0u);
    let base=((LC_COUNTER_PROGRAM*32u+lc_counter.header.y)*2u)*20u;
    atomicAdd(&lc_counter.bins[base],1u);
    atomicAdd(&lc_counter.bins[base+1u],lc_seen);
    atomicAdd(&lc_counter.bins[base+2u],lc_range);
    atomicAdd(&lc_counter.bins[base+3u],lc_source);
    atomicAdd(&lc_counter.bins[base+4u],lc_view);
    atomicAdd(&lc_counter.bins[base+5u],lc_face_pass);
    atomicAdd(&lc_counter.bins[base+6u],lc_disagree);
    atomicAdd(&lc_counter.bins[base+7u],lc_calls);
    atomicAdd(&lc_counter.bins[base+8u],lc_rays);
    atomicAdd(&lc_counter.bins[base+9u],traced*(1u-visible));
    atomicAdd(&lc_counter.bins[base+10u],traced*visible);
    atomicAdd(&lc_counter.bins[base+11u],u32(lc_calls>0u && lc_rays==0u));
    atomicAdd(&lc_counter.bins[base+12u],traced*lc_source);
    atomicAdd(&lc_counter.bins[base+13u],traced*lc_view);
    atomicAdd(&lc_counter.bins[base+14u],lc_view_any);
    atomicAdd(&lc_counter.bins[base+15u],lc_protected*lc_seen);
    atomicAdd(&lc_counter.bins[base+17u],lc_seen*(1u-visible));
    atomicAdd(&lc_counter.bins[base+18u],lc_seen*visible);
    atomicMax(&lc_counter.bins[base+19u],bitcast<u32>(codes));
}
fn lc_primary_shade(s:Surface,input:SurfaceInput)->vec3<f32> {
    lc_primary=lc_counter.header.x!=0u;
    let full=shade(s,input);
    if lc_primary {lc_commit(full,lc_without_rgb,input);}
    return full;
}
