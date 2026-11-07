// ---- renderlab engine inputs: indirect light (M7) ----
// Composed only into modules whose program calls one of these functions
// (src/program.rs), so other programs compile exactly as before.
@group(3) @binding(20) var<storage,read> gi_table:array<u32>;

// The scale-weighted sum of the SH of the cells around `world` at `level` for `bin`:
// bilinear over four cells in the bin's tangent plane, or the containing cell. Cells
// without a current value or query point get no weight; the caller renormalizes. No
// sub-cell jitter (W3).
struct GiSample { weight:f32, sh:array<vec3<f32>,9> };
fn gi_sample(world:vec3<f32>, level:u32, bin:u32) -> GiSample {
    var out:GiSample;
    out.weight=0.0;
    let u=gi_lattice(world,gi_edge(level),bin);
    let trilinear=(gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u;
    var corners=GiCorners(vec3<i32>(floor(u)),vec3<f32>(0.0));
    var count=1u;
    if trilinear {
        corners=gi_corners(u,bin);
        count=4u;
    }
    for(var c=0u;c<count;c+=1u) {
        var w=1.0;
        var cell=corners.base;
        if trilinear {
            w=gi_corner_weight(corners.f,bin,c);
            if w<=0.0 { continue; }
            cell+=gi_corner_offset(bin,c);
        }
        let slot=gi_find(gi_key(level,bin,cell));
        if slot==0xffffffffu { continue; }
        let hot=gi_hot(slot);
        if (gi_table[hot]&GI_VALID)==0u { continue; }
        out.weight+=w;
        let scale=w*bitcast<f32>(gi_table[hot+1u]);
        for(var i=0u;i<9u;i+=1u) {
            out.sh[i]+=scale*vec3<f32>(gi_half(hot,i*3u),gi_half(hot,i*3u+1u),gi_half(hot,i*3u+2u));
        }
    }
    return out;
}

// Unoccluded environment irradiance in lux around n, as `ambient` computes it.
// Intensity products saturate at the f32 maximum instead of overflowing.
fn gi_open_irradiance(n:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 {
        return gi_intensity(environment_bilinear(environment_diffuse,environment_direction(n),0,0));
    }
    return PI*lighting.environment;
}
fn gi_intensity(e:vec3<f32>) -> vec3<f32> {
    return min(e*lighting.map.z,vec3<f32>(3.4028235e38));
}

// Incident irradiance in lux, unexposed, around `n` at the engine's receiver: the
// occluded sky from the world radiance cache (M7 G1; bounce arrives with G2). The
// engine computes it identically for every program. Without a current cache value
// (cache off, non-traced frames, overflow) it is the unoccluded environment.
fn indirect_irradiance(input:SurfaceInput, n:vec3<f32>) -> vec3<f32> {
    if gi_word(GI_H_ENABLED)==0u { return gi_open_irradiance(n); }
    let world=input.receiver.world;
    let bin=gi_bin(gi_viewed_normal(world,input.receiver.geometric));
    let levels=gi_word(GI_H_LEVELS);
    let pick=gi_levels(world,0.0);
    let y=gi_sh_basis(n);
    // Two levels around a switch (level blend), each normalized by its own weights.
    var e=vec3<f32>(0.0);
    var total=0.0;
    for(var j=0u;j<2u;j+=1u) {
        let w=select(1.0-pick.t,pick.t,j==1u);
        if w<=0.0 || (j==1u && pick.hi==pick.lo) { continue; }
        let s=gi_sample(world,select(pick.lo,pick.hi,j==1u),bin);
        if s.weight<=1e-6 { continue; }
        var level_e=vec3<f32>(0.0);
        for(var i=0u;i<9u;i+=1u) { level_e+=gi_band_weight(i)*y.c[i]*s.sh[i]; }
        e+=w*level_e/s.weight;
        total+=w;
    }
    if total>0.0 {
        e=max(e/total,vec3<f32>(0.0));
        if (gi_word(GI_H_FLAGS)&GI_F_FIELD)!=0u { e=gi_intensity(e); }
        return e;
    }
    // Neither level has a value here: coarser levels, then the open environment.
    for(var level=max(pick.lo,pick.hi)+1u;level<levels;level+=1u) {
        let s=gi_sample(world,level,bin);
        if s.weight<=1e-6 { continue; }
        var level_e=vec3<f32>(0.0);
        for(var i=0u;i<9u;i+=1u) { level_e+=gi_band_weight(i)*y.c[i]*s.sh[i]; }
        level_e=max(level_e/s.weight,vec3<f32>(0.0));
        if (gi_word(GI_H_FLAGS)&GI_F_FIELD)!=0u { level_e=gi_intensity(level_e); }
        return level_e;
    }
    return gi_open_irradiance(n);
}
// ---- end of the indirect-light inputs ----
