// ---- renderlab engine inputs: indirect light (M7) ----
// Composed only into modules whose program calls one of these functions
// (src/program.rs), so other programs compile exactly as before.
// Shading reads the table as 16-byte words, so a 64-byte hot record loads in four
// vector loads (src/gi_cache.rs owns the layout; it is the kernels' u32 table).
@group(3) @binding(20) var<storage,read> gi_table:array<vec4<u32>>;
fn gi_word(i:u32) -> u32 { return gi_table[i>>2u][i&3u]; }

// One hot record, loaded whole: key and valid bit, f32 scale, then the 27 SH halves.
struct GiHot { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32>, r3:vec4<u32> };
fn gi_hot_load(slot:u32) -> GiHot {
    let at=(GI_HEADER+slot*GI_HOT_WORDS)>>2u;
    return GiHot(gi_table[at],gi_table[at+1u],gi_table[at+2u],gi_table[at+3u]);
}
// The slot a key's insert tries first: its start offset in its bucket.
fn gi_hot_start(key:vec2<u32>) -> u32 {
    return ((key.x&gi_word(GI_H_MASK))&~(GI_BUCKET-1u))+(key.y&(GI_BUCKET-1u));
}
// The record of `key` given the record at its start slot, with r0.x zero when it is
// absent. The start record loads before its key is compared, so the usual case (the
// insert found its own start slot free) costs one memory round trip; otherwise the rest
// of the bucket is searched in the order gi_find uses.
fn gi_hot_settle(first:GiHot, key:vec2<u32>) -> GiHot {
    if (first.r0.x&GI_KEY_BITS)==key.x { return first; }
    let bucket=(key.x&gi_word(GI_H_MASK))&~(GI_BUCKET-1u);
    for(var i=1u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((key.y+i)&(GI_BUCKET-1u));
        if (gi_word(gi_hot(slot))&GI_KEY_BITS)==key.x { return gi_hot_load(slot); }
    }
    return GiHot(vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u));
}
fn gi_hot_find(key:vec2<u32>) -> GiHot {
    return gi_hot_settle(gi_hot_load(gi_hot_start(key)),key);
}
// A record's irradiance against gi_y, weighted by w: rgb, w. Zero for an absent or
// invalid cell. The same arithmetic, in the same order, as gi_half's per-coefficient sum.
fn gi_hot_eval(h:GiHot, w:f32) -> vec4<f32> {
    if w<=0.0 || (h.r0.x&GI_VALID)==0u { return vec4<f32>(0.0); }
    let p0=unpack2x16float(h.r0.z);let p1=unpack2x16float(h.r0.w);
    let p2=unpack2x16float(h.r1.x);let p3=unpack2x16float(h.r1.y);
    let p4=unpack2x16float(h.r1.z);let p5=unpack2x16float(h.r1.w);
    let p6=unpack2x16float(h.r2.x);let p7=unpack2x16float(h.r2.y);
    let p8=unpack2x16float(h.r2.z);let p9=unpack2x16float(h.r2.w);
    let p10=unpack2x16float(h.r3.x);let p11=unpack2x16float(h.r3.y);
    let p12=unpack2x16float(h.r3.z);let p13=unpack2x16float(h.r3.w);
    var e=vec3<f32>(0.0);
    e+=gi_y[0]*vec3<f32>(p0.x,p0.y,p1.x);
    e+=gi_y[1]*vec3<f32>(p1.y,p2.x,p2.y);
    e+=gi_y[2]*vec3<f32>(p3.x,p3.y,p4.x);
    e+=gi_y[3]*vec3<f32>(p4.y,p5.x,p5.y);
    e+=gi_y[4]*vec3<f32>(p6.x,p6.y,p7.x);
    e+=gi_y[5]*vec3<f32>(p7.y,p8.x,p8.y);
    e+=gi_y[6]*vec3<f32>(p9.x,p9.y,p10.x);
    e+=gi_y[7]*vec3<f32>(p10.y,p11.x,p11.y);
    e+=gi_y[8]*vec3<f32>(p12.x,p12.y,p13.x);
    return vec4<f32>(w*bitcast<f32>(h.r0.y)*e,w);
}

// The cells around `world` at `level` for `bin`, each evaluated against gi_y (the
// band-weighted basis of the shading normal): rgb is the weighted irradiance sum, w the
// weight sum. Bilinear over four cells in the bin's tangent plane, or the containing
// cell. Cells without a current value or query point get no weight; the caller
// renormalizes. No sub-cell jitter (W3). Each record loads whole in one round trip.
fn gi_sample(world:vec3<f32>, level:u32, bin:u32) -> vec4<f32> {
    let u=gi_lattice(world,gi_edge(level),bin);
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)==0u {
        return gi_hot_eval(gi_hot_find(gi_key(level,bin,vec3<i32>(floor(u)))),1.0);
    }
    let corners=gi_corners(u,bin);
    var out=vec4<f32>(0.0);
    for(var c=0u;c<4u;c+=1u) {
        let h=gi_hot_find(gi_key(level,bin,corners.base+gi_corner_offset(bin,c)));
        out+=gi_hot_eval(h,gi_corner_weight(corners.f,bin,c));
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
    gi_sh_basis(n);
    for(var i=0u;i<9u;i+=1u) { gi_y[i]*=gi_band_weight(i); }
    // Two levels around a switch (level blend), each normalized by its own weights.
    var e=vec3<f32>(0.0);
    var total=0.0;
    for(var j=0u;j<2u;j+=1u) {
        let w=select(1.0-pick.t,pick.t,j==1u);
        if w<=0.0 || (j==1u && pick.hi==pick.lo) { continue; }
        let s=gi_sample(world,select(pick.lo,pick.hi,j==1u),bin);
        if s.w<=1e-6 { continue; }
        e+=w*s.rgb/s.w;
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
        if s.w<=1e-6 { continue; }
        var level_e=max(s.rgb/s.w,vec3<f32>(0.0));
        if (gi_word(GI_H_FLAGS)&GI_F_FIELD)!=0u { level_e=gi_intensity(level_e); }
        return level_e;
    }
    return gi_open_irradiance(n);
}
// ---- end of the indirect-light inputs ----
