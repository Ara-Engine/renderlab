// ---- Reflection hits: local-light shadow cut (program.rs `hit_shadow_cut`) ----
// At a reflection hit the weakest shadowed local lights skip their shadow: each whose
// unshadowed illuminance at the hit (luminance x attenuation x n.l on the hit's plane) is
// at most RL_HIT_SHADOW_CUT of the hit's total light (every light, the sun included, plus
// the ambient) divided by the number of shadowed local lights, so the cut ones together
// stay within the fraction. A cut light shades fully visible, so the
// error is at most that fraction of the hit's light, the same every frame. The sun is never
// cut. Camera receivers are untouched.
const RL_HIT_SHADOW_CUT:f32=0.0625;
// The hit the cut was prepared for, and the illuminance below which a light is cut.
var<private> rl_hit_cut_ready:bool;
var<private> rl_hit_cut_at:vec3<f32>;
var<private> rl_hit_cut_below:f32;

// A local light with a shadow (map or rays); directional lights are never candidates.
fn rl_hit_cut_candidate(index:u32)->bool {
    let kind=lights[index].kind;
    return kind.x!=0u && kind.w!=0u;
}
fn rl_hit_light_lux(index:u32, l:vec3<f32>, attenuation:f32, n:vec3<f32>)->f32 {
    return dot(lights[index].rgb_inner.xyz,vec3<f32>(0.2126,0.7152,0.0722))*attenuation*max(dot(n,l),0.0);
}
// The hit's threshold, in one walk of its lights: each of the n candidates may take
// budget / n, so whichever are cut sum to at most the budget (the fraction of the hit's
// total), however many comparable lights there are. The ambient is the constant
// environment's: a texture lookup per hit cost more than the cut saved.
fn rl_hit_cut_prepare(input:SurfaceInput) {
    rl_hit_cut_ready=true;
    rl_hit_cut_at=input.world;
    let n=input.geometric;
    let count=rl_light_total();
    var total=PI*dot(lighting.environment,vec3<f32>(0.2126,0.7152,0.0722));
    var candidates=0u;
    for(var i=0u;i<count;i+=1u) {
        let s=light_sample_untinted(i,input);
        total+=rl_hit_light_lux(s.index,s.direction,s.attenuation,n);
        candidates+=u32(rl_hit_cut_candidate(s.index));
    }
    rl_hit_cut_below=RL_HIT_SHADOW_CUT*total/f32(max(candidates,1u));
}
// Whether light_visibility skips `s`'s shadow at this reflection hit.
fn rl_hit_shadow_cut(input:SurfaceInput, s:LightSample)->bool {
    if !rl_hit_cut_candidate(s.index) {return false;}
    if !rl_hit_cut_ready || any(input.world!=rl_hit_cut_at) {rl_hit_cut_prepare(input);}
    return rl_hit_light_lux(s.index,s.direction,s.attenuation,input.geometric)<rl_hit_cut_below;
}
