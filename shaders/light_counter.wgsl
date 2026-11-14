// Measurement only: independent SSBO, no counter values are written to HDR.
struct LcCounter { header:vec4<u32>, bins:array<atomic<u32>> };
@group(0) @binding(80) var<storage,read_write> lc_counter:LcCounter;
@group(0) @binding(81) var<storage,read_write> lc_pass_records:array<vec4<u32>>;
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
var<private> lc_unshadowed_rgb:vec3<f32>;
var<private> lc_distance_bin:u32;
var<private> lc_normal_bin:u32;
var<private> lc_shadow_slots:u32;
var<private> lc_cached:bool;
var<private> lc_remaining:f32;
var<private> lc_skip:bool;
var<private> lc_invalid:bool;
// Diagnostic simulation: 6.443 global display slope, E=0.5 shared by locals.
// Charge the smooth window even when it still traces; only w=0 skips a ray.
fn lc_cutoff(s:LightSample,c:vec3<f32>) {
    if !lc_primary || lights[s.index].kind.x==0u {return;}
    let valid=all(c>=vec3<f32>(0.0)) && all(c<=vec3<f32>(3.402823e38));
    let b=255.0*6.443*max(max(c.x,c.y),c.z)*(1.0+0.000001);
    let finite=valid && b<=3.402823e38;
    var skip=false;
    if finite && lc_remaining>0.0 {
        let w=smoothstep(lc_remaining*0.5,lc_remaining,b);
        skip=b<=lc_remaining*0.5;
        lc_remaining=max(lc_remaining-b*(1.0-w),0.0);
    }
    if s.index==lc_counter.header.y {lc_skip=skip;lc_invalid=!finite;}
}
fn lc_capture(input:SurfaceInput,n:vec3<f32>,s:LightSample) {
    if !lc_primary {return;}
    let capable=lights[s.index].kind.w!=0u;
    let slot=lc_shadow_slots;
    if capable {lc_shadow_slots+=1u;}
    if s.index!=lc_counter.header.y {return;}
    lc_cached=capable && slot<16u;
    lc_seen=1u;
    lc_range=u32(s.attenuation==0.0);
    let source=dot(input.geometric,s.direction)>0.0;
    let viewed=dot(n,s.direction)>0.0;
    let light=lights[s.index];
    let fraction=length(light.position_range.xyz-input.world)/max(light.position_range.w,1e-20);
    lc_distance_bin=select(6u,5u,light.position_range.w>0.0);
    if light.position_range.w>0.0 {
        if fraction<0.25 {lc_distance_bin=0u;} else if fraction<0.5 {lc_distance_bin=1u;} else if fraction<0.75 {lc_distance_bin=2u;} else if fraction<0.9 {lc_distance_bin=3u;} else if fraction<1.0 {lc_distance_bin=4u;}
    }
    let nl=dot(n,s.direction);
    lc_normal_bin=5u;
    if nl<0.0 {lc_normal_bin=0u;} else if nl<0.1 {lc_normal_bin=1u;} else if nl<0.25 {lc_normal_bin=2u;} else if nl<0.5 {lc_normal_bin=3u;} else if nl<0.75 {lc_normal_bin=4u;}
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
fn lc_base(stage:u32)->u32 {return ((LC_COUNTER_PROGRAM*32u+lc_counter.header.y)*4u+stage)*96u;}
fn lc_accumulate(stage:u32,codes:f32,upper:f32) {
    let visible=u32(codes>=1.0);let traced=u32(lc_rays>0u);
    let under1=u32(upper<1.0);let under_half=u32(upper<0.5);let under_quarter=u32(upper<0.25);
    let base=lc_base(stage);
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
    atomicAdd(&lc_counter.bins[base+16u],lc_rays*(1u-visible));
    atomicAdd(&lc_counter.bins[base+17u],lc_seen*(1u-visible));
    atomicAdd(&lc_counter.bins[base+18u],lc_seen*visible);
    atomicMax(&lc_counter.bins[base+19u],bitcast<u32>(codes));
    atomicAdd(&lc_counter.bins[base+20u],lc_rays*lc_source);
    atomicAdd(&lc_counter.bins[base+21u],lc_rays*lc_view);
    atomicAdd(&lc_counter.bins[base+22u],lc_rays*lc_disagree);
    atomicAdd(&lc_counter.bins[base+23u],lc_rays*lc_disagree*visible);
    atomicAdd(&lc_counter.bins[base+24u],lc_seen*under1);
    atomicAdd(&lc_counter.bins[base+25u],lc_rays*under1);
    atomicAdd(&lc_counter.bins[base+26u],traced*under1);
    atomicMax(&lc_counter.bins[base+27u],bitcast<u32>(upper));
    atomicAdd(&lc_counter.bins[base+92u],lc_seen*u32(lc_skip));
    atomicAdd(&lc_counter.bins[base+93u],lc_rays*u32(lc_skip));
    atomicAdd(&lc_counter.bins[base+94u],lc_rays*u32(codes<0.5));
    atomicAdd(&lc_counter.bins[base+95u],lc_seen*u32(lc_invalid));
    atomicAdd(&lc_counter.bins[base+28u],lc_seen*under_half);
    atomicAdd(&lc_counter.bins[base+29u],lc_rays*under_half);
    atomicAdd(&lc_counter.bins[base+30u],traced*under_half);
    atomicAdd(&lc_counter.bins[base+31u],lc_seen*under_quarter);
    atomicAdd(&lc_counter.bins[base+32u],lc_rays*under_quarter);
    atomicAdd(&lc_counter.bins[base+33u],traced*under_quarter);
    if lc_rays>0u {
        let distance=base+34u+lc_distance_bin*4u;
        atomicAdd(&lc_counter.bins[distance],lc_rays);atomicAdd(&lc_counter.bins[distance+1u],lc_rays*under1);atomicAdd(&lc_counter.bins[distance+2u],lc_rays*under_half);atomicAdd(&lc_counter.bins[distance+3u],lc_rays*under_quarter);
        let normal=base+62u+lc_normal_bin*4u;
        atomicAdd(&lc_counter.bins[normal],lc_rays);atomicAdd(&lc_counter.bins[normal+1u],lc_rays*under1);atomicAdd(&lc_counter.bins[normal+2u],lc_rays*under_half);atomicAdd(&lc_counter.bins[normal+3u],lc_rays*under_quarter);
        atomicAdd(&lc_counter.bins[base+86u],u32(upper+0.001<codes));
    }
}
fn lc_commit(full:vec3<f32>,without:vec3<f32>,input:SurfaceInput) {
    let absent=lc_display(atmosphere_surface(rl_finite(without),input.world));
    let delta=abs(lc_display(atmosphere_surface(rl_finite(full),input.world))-absent)*255.0;
    let codes=max(max(delta.x,delta.y),delta.z);
    // Visibility=0/1 endpoints retain every other lit term. No producer normal is invented.
    let upper_delta=abs(lc_display(atmosphere_surface(rl_finite(lc_unshadowed_rgb),input.world))-absent)*255.0;
    let upper=max(max(upper_delta.x,upper_delta.y),upper_delta.z);
    lc_accumulate(0u,codes,upper);
    if lc_cached {lc_accumulate(1u,codes,upper);} else {atomicAdd(&lc_counter.bins[lc_base(1u)],1u);}
    if LC_COUNTER_LVIS {
        let pixel=input.pixel.y*textureDimensions(primary_hit).x+input.pixel.x;
        let record=lc_pass_records[pixel];
        let matched=record.x!=0u && lc_seen!=0u && lc_cached && lc_range==((record.w>>9u)&1u);
        if (record.x!=0u)!=lc_cached {atomicAdd(&lc_counter.bins[lc_base(0u)+87u],1u);}
        if record.x!=0u && lc_range!=((record.w>>9u)&1u) {atomicAdd(&lc_counter.bins[lc_base(0u)+91u],1u);}
        if matched {
            lc_calls=record.y;lc_rays=record.z;
            lc_accumulate(2u,codes,upper);
            atomicAdd(&lc_counter.bins[lc_base(2u)+89u],1u);
            atomicAdd(&lc_counter.bins[lc_base(2u)+90u],(record.w>>8u)&1u);
        } else {
            atomicAdd(&lc_counter.bins[lc_base(2u)],1u);
            atomicAdd(&lc_counter.bins[lc_base(0u)+88u],record.z);
        }
    }
}
fn lc_primary_shade(s:Surface,input:SurfaceInput)->vec3<f32> {
    lc_primary=lc_counter.header.x!=0u;
    lc_remaining=0.5;
    let full=shade(s,input);
    if lc_primary {lc_commit(full,lc_without_rgb,input);}
    return full;
}
