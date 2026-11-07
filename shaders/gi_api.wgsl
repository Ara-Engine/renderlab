// ---- renderlab engine inputs: indirect light (M7) ----
// Composed only into modules whose program calls one of these functions
// (src/program.rs), so other programs compile exactly as before.
@group(3) @binding(20) var<storage,read> gi_table:array<u32>;

// Weighted sum of the slot SH of one cell around `world` at `level`, for `bin`:
// trilinear over the eight cells whose centres surround the point, or the cell
// containing it. Cells without a current value (or without a query point) get no
// weight; the caller renormalizes. No sub-cell jitter (W3).
struct GiSample { weight:f32, sh:array<vec3<f32>,9> };
fn gi_sample(world:vec3<f32>, level:u32, bin:u32) -> GiSample {
    var out:GiSample;
    out.weight=0.0;
    let edge=gi_edge(level);
    let trilinear=(gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u;
    let scaled=world/edge;
    var base=vec3<i32>(floor(scaled));
    var f=vec3<f32>(0.0);
    var corners=1u;
    if trilinear {
        let shifted=scaled-vec3<f32>(0.5);
        base=vec3<i32>(floor(shifted));
        f=shifted-floor(shifted);
        corners=8u;
    }
    for(var c=0u;c<corners;c+=1u) {
        let o=vec3<u32>(c&1u,(c>>1u)&1u,(c>>2u)&1u);
        var w=1.0;
        if trilinear {
            let t=select(vec3<f32>(1.0)-f,f,o==vec3<u32>(1u));
            w=t.x*t.y*t.z;
            if w<=0.0 { continue; }
        }
        let slot=gi_find(gi_key(level,bin,base+vec3<i32>(o)));
        if slot==0xffffffffu { continue; }
        let s=gi_slot_base(slot);
        if (gi_table[s+GI_S_STATE]&(GI_TRACED|GI_ORPHAN))!=GI_TRACED { continue; }
        out.weight+=w;
        for(var i=0u;i<9u;i+=1u) {
            let k=s+GI_S_SH+i*3u;
            out.sh[i]+=w*vec3<f32>(bitcast<f32>(gi_table[k]),bitcast<f32>(gi_table[k+1u]),bitcast<f32>(gi_table[k+2u]));
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
    for(var level=gi_level(world);level<levels;level+=1u) {
        let s=gi_sample(world,level,bin);
        if s.weight<=1e-6 { continue; }
        let y=gi_sh_basis(n);
        var e=vec3<f32>(0.0);
        for(var i=0u;i<9u;i+=1u) { e+=gi_band_weight(i)*y.c[i]*s.sh[i]; }
        e=max(e/s.weight,vec3<f32>(0.0));
        if (gi_word(GI_H_FLAGS)&GI_F_FIELD)!=0u { e=gi_intensity(e); }
        return e;
    }
    return gi_open_irradiance(n);
}
// ---- end of the indirect-light inputs ----
