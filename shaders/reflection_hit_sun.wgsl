// ---- reflection hits' sun from the Virtual clipmap (non-RT; src/reflection_hit_sun.rs) ----
// The hit pass's read-only view of the sun-shadow map, beside the fog's (VF1,
// shaders/atmosphere_shadow_virtual.wgsl): group 4 holds the map's frame (binding 0, composed
// from shaders/virtual_shadow.wgsl), its top layer (1) and its page state (2). A hit is a
// surface, not a froxel: its point moves off the surface along its viewed-side normal by a
// fraction of the level's texel before the depth test, and a point no resident level holds
// returns -1 so the caller traces its ray as before.
@group(4) @binding(1) var<storage,read> rl_hs_top:array<vec2<u32>>;
@group(4) @binding(2) var<storage,read> rl_hs_pages:array<u32>;
// The normal offset and the depth bias, in texels of the level read.
const RL_HS_NORMAL:f32=1.5;
const RL_HS_BIAS:f32=1.0;
fn rl_hit_sun_map(receiver:Receiver, l:vec3<f32>)->f32 {
    let side=select(-receiver.geometric,receiver.geometric,dot(receiver.geometric,l)>=0.0);
    let pages=vsm.counts.y;
    let size=pages*VSM_PAGE;
    // Finest valid resident page wins.
    for(var level=0u;level<vsm.counts.x;level+=1u) {
        let texel=vsm_texel(level);
        let p=vsm_light(fma(side,vec3<f32>(max(receiver.epsilon,RL_HS_NORMAL*texel)),receiver.world));
        if p.z>=vsm.depth.w { return 1.0; }
        let q=(p.xy-vsm.levels[level].origin.xy)/texel;
        if any(q<vec2<f32>(0.0)) || any(q>=vec2<f32>(f32(size))) { continue; }
        let cell=vec2<u32>(q);
        let page=cell/VSM_PAGE;
        var slot=(level*pages+page.y)*pages+page.x;
        if vsm_sparse() { slot=vsm_vpage(level,page); }
        if rl_hs_pages[slot]!=vsm.levels[level].info.x { continue; }
        var index=(level*size+cell.y)*size+cell.x;
        if vsm_sparse() {
            let physical=rl_hs_pages[vsm_section(2u)+slot];
            if physical==VSM_NONE || rl_hs_pages[vsm_section(1u)+slot]!=vsm_tag(level,page) { continue; }
            index=physical*VSM_PAGE*VSM_PAGE+(cell.y%VSM_PAGE)*VSM_PAGE+cell.x%VSM_PAGE;
        }
        let word=rl_hs_top[index];
        if word.x==0u { return 1.0; }
        let height=fma(bitcast<f32>(word.y),vsm.depth.z,vsm.depth.x);
        return select(0.0,1.0,p.z+RL_HS_BIAS*texel>=height);
    }
    return -1.0;
}
// The hit's visibility of `light`: the sun from the map where a resident level holds the
// point, every other light and every point the map lacks from the hit's own shadow ray.
fn rl_hit_sun(light:Light, receiver:Receiver, l:vec3<f32>)->f32 {
    if light.kind.x==0u && light.kind.z!=0u {
        let v=rl_hit_sun_map(receiver,l);
        if v>=0.0 { return v; }
    }
    return rl_hit_shadow(light,receiver,l);
}
// ---- end of reflection hits' sun ----
