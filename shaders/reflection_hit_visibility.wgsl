// ---- reflection-hit light visibility from the radiance cache (RENDERLAB_RL_HIT_VIS, src/program.rs) ----
// Composed after shaders/reflections.wgsl on traced stages whose program reads the world radiance
// cache, only while the switch names a minimum roughness; otherwise nothing here is composed and
// hits trace their visibility as before.
// A hit from a lobe at least RL_HIT_VIS_ROUGH rough takes each shadowed light's visibility from
// the bit the radiance cache's cell there holds for it (gi_light: one ray from the cell's query
// point per light, re-traced when a moved caster's swept box meets it) instead of tracing: the
// sun's hard ray, and the local lights' disk rays, which hits from lobes past
// LocalShadows::unshadowed_hits_from otherwise skip (so those hits get their lamp shadows back).
// The cell is found as rl_cache_hit finds it: at the level the hit's ray cone covers, in the bin
// of the hit's geometric normal turned toward the ray. RL_HIT_VIS_CONE also requires the cone to
// cover the hit's own pixel-level cell (cache-terminated hits' rule); without it any glossy hit
// reads its cell, at cell resolution. A missing or orphaned cell, one whose lights were never
// traced (GI_LIT), and a light the cell's query point does not face (gi_light's own test) keep
// the hit's ray. Visibility is the engine's input, so programs shade the hit as before.
const RL_HIT_VIS_ROUGH:f32=RL_HIT_VIS_ROUGH_MIN;
const RL_HIT_VIS_CONE:bool=RL_HIT_VIS_CONE_ON;
// The cell's cold record for the hit being shaded, or 0xffffffff to trace.
var<private> rl_hit_vis_cold:u32=0xffffffffu;

// Before a hit's `shade`: finds its cell. `world` is the hit point, `cone` the ray cone's width
// there (metres), `rough` the reflecting lobe's perceptual roughness.
fn rl_hit_vis_prepare(world:vec3<f32>,origin:vec3<f32>,cone:f32,rough:f32,geometric:vec3<f32>) {
    rl_hit_vis_cold=0xffffffffu;
    if rough<RL_HIT_VIS_ROUGH || gi_word(GI_H_ENABLED)==0u { return; }
    let base=gi_level(world);
    if RL_HIT_VIS_CONE && cone<gi_edge(base) { return; }
    let last=gi_word(GI_H_LEVELS)-1u;
    let level=min(max(base,u32(ceil(log2(max(cone,1e-30)/gi_header_f32(GI_H_MIN_EDGE))))),last);
    let bin=gi_bin(select(-geometric,geometric,dot(geometric,origin-world)>=0.0));
    let slot=gi_find_pair(gi_key(level,bin,vec3<i32>(floor(gi_lattice(world,gi_edge(level),bin)))));
    if slot==0xffffffffu { return; }
    let cold=gi_cold(slot);
    let state=gi_word(cold+GI_C_STATE);
    if (state&GI_LIT)==0u || (state&GI_ORPHAN)!=0u { return; }
    rl_hit_vis_cold=cold;
}

// The cached visibility of light `index` at the prepared hit (0 or 1), or -1 to trace: no cell,
// or a light the cell's query point does not face (gi_light traced no ray for it).
fn rl_hit_vis(index:u32) -> f32 {
    let cold=rl_hit_vis_cold;
    if cold==0xffffffffu { return -1.0; }
    let q=vec3<f32>(bitcast<f32>(gi_word(cold+GI_C_POINT)),bitcast<f32>(gi_word(cold+GI_C_POINT+1u)),bitcast<f32>(gi_word(cold+GI_C_POINT+2u)));
    let n=gi_unpack_normal(gi_word(cold+GI_C_NORMAL));
    // gi_light_affects (gi_cache.wgsl) at the query point.
    let light=lights[index];
    if all(light.rgb_inner.xyz==vec3<f32>(0.0)) { return -1.0; }
    var l=-light.travel_outer.xyz;
    if light.kind.x!=0u {
        let delta=light.position_range.xyz-q;
        let distance=length(delta);
        l=delta/max(distance,1e-30);
        if light.position_range.w>0.0 && distance>=light.position_range.w { return -1.0; }
        if light.kind.x==2u && !(dot(-l,light.travel_outer.xyz)>light.travel_outer.w) { return -1.0; }
    }
    if dot(n,l)<=0.0 { return -1.0; }
    return select(0.0,1.0,((gi_word(cold+GI_C_LIGHTS+(index>>5u))>>(index&31u))&1u)!=0u);
}
// ---- end of reflection-hit light visibility ----
