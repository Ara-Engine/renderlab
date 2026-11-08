// VF1: a depth-map test of the Virtual clipmap's top layer. No triangle walks.
@group(0) @binding(22) var<storage,read> at_shadow_top:array<vec2<u32>>;
@group(0) @binding(23) var<storage,read> at_shadow_pages:array<u32>;
fn at_sun_visible(world:vec3<f32>) -> f32 {
    let p=vsm_light(world);
    if p.z>=vsm.depth.w { return 1.0; }
    let pages=vsm.counts.y;
    let side=pages*VSM_PAGE;
    // Finest valid resident page wins. Froxels do not request pages: uncovered
    // volumes fall back through coarser levels, then to unoccluded sunlight.
    for(var level=0u;level<vsm.counts.x;level+=1u) {
        let q=(p.xy-vsm.levels[level].origin.xy)/vsm_texel(level);
        if any(q<vec2<f32>(0.0)) || any(q>=vec2<f32>(f32(side))) { continue; }
        let cell=vec2<u32>(q);
        let page=cell/VSM_PAGE;
        let slot=(level*pages+page.y)*pages+page.x;
        if at_shadow_pages[slot]!=vsm.levels[level].info.x { continue; }
        let word=at_shadow_top[(level*side+cell.y)*side+cell.x];
        if word.x==0u { return 1.0; }
        let height=fma(bitcast<f32>(word.y),vsm.depth.z,vsm.depth.x);
        // Froxels have no receiver surface and need no normal offset. A small
        // depth quantization guard only; jitter/reprojection integrates map edges.
        return select(0.0,1.0,p.z+max(vsm.depth.z*1e-6,1e-4)>=height);
    }
    return 1.0;
}
