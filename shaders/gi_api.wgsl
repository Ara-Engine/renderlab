// ---- renderlab engine inputs: indirect light (M7) ----
// Composed only into modules whose program calls one of these functions
// (src/program.rs), so other programs compile exactly as before.
// Shading reads the table as 16-byte words, so a 96-byte hot record loads in six vector
// loads (src/gi_cache.rs owns the layout; it is the kernels' u32 table).
@group(3) @binding(20) var<storage,read> gi_table:array<vec4<u32>>;
fn gi_word(i:u32) -> u32 { return gi_table[i>>2u][i&3u]; }

// One hot record, loaded whole: key and valid bit, f32 scale, the 27 sky SH halves, then the
// bounce scale and its 12 L1 halves (G2).
struct GiHot { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32>, r3:vec4<u32>, r4:vec4<u32>, r5:vec4<u32> };
fn gi_hot_load(slot:u32) -> GiHot {
    let at=(GI_HEADER+slot*GI_HOT_WORDS)>>2u;
    return GiHot(gi_table[at],gi_table[at+1u],gi_table[at+2u],gi_table[at+3u],gi_table[at+4u],gi_table[at+5u]);
}
// The slot a key's insert tries first: its start offset in its bucket.
fn gi_hot_start(key:vec2<u32>) -> u32 {
    return ((key.x&gi_word(GI_H_MASK))&~(GI_BUCKET-1u))+(gi_start(key)&(GI_BUCKET-1u));
}
// The record of `key` given the record at its start slot, with r0.x zero when it is
// absent. The start record loads before its key is compared, so the usual case (the
// insert found its own start slot free) costs one memory round trip; otherwise the rest
// of the bucket is searched in the order gi_find uses.
fn gi_hot_settle(first:GiHot, key:vec2<u32>) -> GiHot {
    if (first.r0.x&GI_KEY_BITS)==key.x { return first; }
    let bucket=(key.x&gi_word(GI_H_MASK))&~(GI_BUCKET-1u);
    for(var i=1u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((gi_start(key)+i)&(GI_BUCKET-1u));
        if (gi_word(gi_hot(slot))&GI_KEY_BITS)==key.x { return gi_hot_load(slot); }
    }
    return GiHot(vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u));
}
fn gi_hot_find(key:vec2<u32>) -> GiHot {
    return gi_hot_settle(gi_hot_load(gi_hot_start(key)),key);
}

// One level's lookup, as weighted coefficient sums (gi_sample): the sky's L2 in cached units
// and the bounce light's L1 in absolute units. Private, never passed by value (a by-value
// array costs a local-memory copy per call).
var<private> gi_lv_sky:array<vec3<f32>,9>;
var<private> gi_lv_bounce:array<vec3<f32>,4>;
// The receiver's bin-source normal (its viewed vertex normal), against which each cell's query
// normal is weighed (gi_prepare).
var<private> gi_rx_normal:vec3<f32>;
// How far a cell's surface agrees with the receiver's: full weight within about 75 degrees,
// none for a surface facing away (G2). A cell whose fixed query point lies on another side
// (the back of the receiver's own sheet, a facet across a crease) describes light the
// receiver does not get; reading it made isolated near-black pixels where a receiver's
// normal sat near a bin tie (docs/m7-g2.md). A fixed geometric weight, no jitter (W3).
fn gi_agreement(h:GiHot) -> f32 {
    return clamp(dot(gi_unpack_normal(h.r5.w),gi_rx_normal)*4.0,0.0,1.0);
}
// Adds a record's coefficients with weight w to the level sums; returns the weight it added,
// or 0 for an absent or invalid cell.
fn gi_hot_add(h:GiHot, w_in:f32) -> f32 {
    if w_in<=0.0 || (h.r0.x&GI_VALID)==0u { return 0.0; }
    let w=w_in*gi_agreement(h);
    if w<=0.0 { return 0.0; }
    let s=w*bitcast<f32>(h.r0.y);
    let p0=unpack2x16float(h.r0.z);let p1=unpack2x16float(h.r0.w);
    let p2=unpack2x16float(h.r1.x);let p3=unpack2x16float(h.r1.y);
    let p4=unpack2x16float(h.r1.z);let p5=unpack2x16float(h.r1.w);
    let p6=unpack2x16float(h.r2.x);let p7=unpack2x16float(h.r2.y);
    let p8=unpack2x16float(h.r2.z);let p9=unpack2x16float(h.r2.w);
    let p10=unpack2x16float(h.r3.x);let p11=unpack2x16float(h.r3.y);
    let p12=unpack2x16float(h.r3.z);let p13=unpack2x16float(h.r3.w);
    gi_lv_sky[0]+=s*vec3<f32>(p0.x,p0.y,p1.x);
    gi_lv_sky[1]+=s*vec3<f32>(p1.y,p2.x,p2.y);
    gi_lv_sky[2]+=s*vec3<f32>(p3.x,p3.y,p4.x);
    gi_lv_sky[3]+=s*vec3<f32>(p4.y,p5.x,p5.y);
    gi_lv_sky[4]+=s*vec3<f32>(p6.x,p6.y,p7.x);
    gi_lv_sky[5]+=s*vec3<f32>(p7.y,p8.x,p8.y);
    gi_lv_sky[6]+=s*vec3<f32>(p9.x,p9.y,p10.x);
    gi_lv_sky[7]+=s*vec3<f32>(p10.y,p11.x,p11.y);
    gi_lv_sky[8]+=s*vec3<f32>(p12.x,p12.y,p13.x);
    let b=w*bitcast<f32>(h.r4.x);
    if b>0.0 {
        let q0=unpack2x16float(h.r4.y);let q1=unpack2x16float(h.r4.z);
        let q2=unpack2x16float(h.r4.w);let q3=unpack2x16float(h.r5.x);
        let q4=unpack2x16float(h.r5.y);let q5=unpack2x16float(h.r5.z);
        gi_lv_bounce[0]+=b*vec3<f32>(q0.x,q0.y,q1.x);
        gi_lv_bounce[1]+=b*vec3<f32>(q1.y,q2.x,q2.y);
        gi_lv_bounce[2]+=b*vec3<f32>(q3.x,q3.y,q4.x);
        gi_lv_bounce[3]+=b*vec3<f32>(q4.y,q5.x,q5.y);
    }
    return w;
}

// Whether the last gi_sample read a split cell with weight (G1c): its pixels then read
// the next finer level, where the slab and the floor beneath it are separate cells.
var<private> gi_sampled_split:bool;
fn gi_note_split(h:GiHot, w:f32) {
    gi_sampled_split=gi_sampled_split || (w>0.0 && (h.r3.w&GI_SPLIT_HOT)!=0u && (h.r0.x&GI_VALID)!=0u && gi_agreement(h)>0.0);
}
// The cells around `world` at `level` for `bin` into the level sums (gi_lv_*), returning
// their total weight; the caller normalizes. Bilinear over four cells in the bin's tangent
// plane, or the containing cell. Cells without a current value or query point get no
// weight. No sub-cell jitter (W3). Each record loads whole in one round trip.
fn gi_sample(world:vec3<f32>, level:u32, bin:u32) -> f32 {
    gi_sampled_split=false;
    for(var i=0u;i<9u;i+=1u) { gi_lv_sky[i]=vec3<f32>(0.0); }
    for(var i=0u;i<4u;i+=1u) { gi_lv_bounce[i]=vec3<f32>(0.0); }
    let u=gi_lattice(world,gi_edge(level),bin);
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)==0u {
        let h=gi_hot_find(gi_key(level,bin,vec3<i32>(floor(u))));
        gi_note_split(h,1.0);
        return gi_hot_add(h,1.0);
    }
    let corners=gi_corners(u,bin);
    var total=0.0;
    for(var c=0u;c<4u;c+=1u) {
        let h=gi_hot_find(gi_key(level,bin,corners.base+gi_corner_offset(bin,c)));
        let w=gi_corner_weight(corners.f,bin,c);
        gi_note_split(h,w);
        total+=gi_hot_add(h,w);
    }
    return total;
}
// gi_sample, read at the finer level where it meets split cells (GI_REFINE 1), keeping the
// finer level when it has a value: the same walk gi_probe requests. The level sums hold the
// level whose weight it returns.
fn gi_sample_refined(world:vec3<f32>, level:u32, bin:u32) -> f32 {
    let s=gi_sample(world,level,bin);
    if GI_REFINE==0u || !gi_sampled_split || level==0u { return s; }
    let f=gi_sample(world,level-1u,bin);
    if f>1e-6 { return f; }
    return gi_sample(world,level,bin);
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
// Cached values in lux: a map's cells hold its radiance (intensity applies here); a
// constant sky's cells hold unit radiance, scaled here by the current constant.
fn gi_sky_scale(e:vec3<f32>) -> vec3<f32> {
    if (gi_word(GI_H_FLAGS)&GI_F_FIELD)!=0u { return gi_intensity(e); }
    return min(e*lighting.environment,vec3<f32>(3.4028235e38));
}

// The receiver's blended cache value: the sky's L2 and the bounce light's L1 coefficients,
// each normalized by the lookup's fixed weights (the levels around a switch, a split cell's
// finer level, then coarser levels where the receiver's have no value). The generated entry
// computes it once per receiver (gi_prepare through rl_indirect_prepare, src/program.rs), so
// the diffuse input, ES1's specular visibility and the bounce lobe share one lookup.
var<private> gi_rx_sky:array<vec3<f32>,9>;
var<private> gi_rx_bounce:array<vec3<f32>,4>;
var<private> gi_rx_valid:bool;
fn gi_rx_add(w:f32) {
    for(var i=0u;i<9u;i+=1u) { gi_rx_sky[i]+=w*gi_lv_sky[i]; }
    for(var i=0u;i<4u;i+=1u) { gi_rx_bounce[i]+=w*gi_lv_bounce[i]; }
}
fn gi_prepare(input:SurfaceInput) {
    gi_rx_valid=false;
    for(var i=0u;i<9u;i+=1u) { gi_rx_sky[i]=vec3<f32>(0.0); }
    for(var i=0u;i<4u;i+=1u) { gi_rx_bounce[i]=vec3<f32>(0.0); }
    if gi_word(GI_H_ENABLED)==0u { return; }
    let world=input.receiver.world;
    // The bin comes from the interpolated vertex normal, as gi_probe requests it.
    let source=gi_bin_source(gi_viewed_normal(world,input.receiver.geometric),input.normal);
    gi_rx_normal=unit_normal(source);
    let bin=gi_bin(source);
    let levels=gi_word(GI_H_LEVELS);
    let pick=gi_levels(world,0.0);
    // Two levels around a switch (level blend), each normalized by its own weights.
    var total=0.0;
    for(var j=0u;j<2u;j+=1u) {
        let w=select(1.0-pick.t,pick.t,j==1u);
        if w<=0.0 || (j==1u && pick.hi==pick.lo) { continue; }
        let level=select(pick.lo,pick.hi,j==1u);
        let s=gi_sample_refined(world,level,bin);
        if s<=1e-6 { continue; }
        // Corners holding part of the weight (orphans, absent or disagreeing cells) keep it;
        // the next coarser level fills the rest, instead of the corners left taking all of
        // it (G2: renormalizing over one dark corner made isolated pinholes after TR1).
        total+=w;
        if s>=0.999 || level+1u>=levels {
            gi_rx_add(w/s);
            continue;
        }
        gi_rx_add(w);
        let c=gi_sample(world,level+1u,bin);
        if c>1e-6 {
            gi_rx_add(w*(1.0-s)/c);
            continue;
        }
        // No coarser value: the level's own corners take the rest, as without the fill.
        gi_rx_add(w*(1.0-s)/gi_sample_refined(world,level,bin));
    }
    if total>0.0 {
        let inv=1.0/total;
        for(var i=0u;i<9u;i+=1u) { gi_rx_sky[i]*=inv; }
        for(var i=0u;i<4u;i+=1u) { gi_rx_bounce[i]*=inv; }
        gi_rx_valid=true;
        return;
    }
    // No agreeing value at the receiver's level in its bin: its second bin there (the next
    // axis of its normal, which a receiver near a bin tie shares with its neighbours), then
    // coarser levels, then the open environment.
    let s2=gi_sample_refined(world,pick.lo,gi_second_bin(source));
    if s2>1e-6 {
        gi_rx_add(1.0/s2);
        gi_rx_valid=true;
        return;
    }
    for(var level=max(pick.lo,pick.hi)+1u;level<levels;level+=1u) {
        let s=gi_sample(world,level,bin);
        if s<=1e-6 { continue; }
        gi_rx_add(1.0/s);
        gi_rx_valid=true;
        return;
    }
}
// The prepared receiver's occluded sky irradiance about n in lux, or the open environment's
// where the cache has no value.
fn gi_rx_sky_irradiance(n:vec3<f32>) -> vec3<f32> {
    if !gi_rx_valid { return gi_open_irradiance(n); }
    gi_sh_basis(n);
    var e=vec3<f32>(0.0);
    for(var i=0u;i<9u;i+=1u) { e+=gi_band_weight(i)*gi_y[i]*gi_rx_sky[i]; }
    return gi_sky_scale(max(e,vec3<f32>(0.0)));
}
// The prepared receiver's bounce light in lux about n (G2), or radiance in a GGX lobe of
// perceptual roughness about a direction; zero without a value.
fn gi_rx_bounce_irradiance(n:vec3<f32>) -> vec3<f32> {
    if !gi_rx_valid { return vec3<f32>(0.0); }
    gi_sh_basis(n);
    var e=vec3<f32>(0.0);
    for(var i=0u;i<4u;i+=1u) { e+=gi_band_weight(i)*gi_y[i]*gi_rx_bounce[i]; }
    return max(e,vec3<f32>(0.0));
}
fn gi_rx_bounce_radiance(direction:vec3<f32>, roughness:f32) -> vec3<f32> {
    if !gi_rx_valid { return vec3<f32>(0.0); }
    gi_sh_basis(direction);
    let a1=gi_lobe_band1(roughness);
    let e=gi_y[0]*gi_rx_bounce[0]+a1*(gi_y[1]*gi_rx_bounce[1]+gi_y[2]*gi_rx_bounce[2]+gi_y[3]*gi_rx_bounce[3]);
    return max(e,vec3<f32>(0.0));
}

// Incident irradiance in lux, unexposed, around `n` at the engine's receiver: the occluded
// sky plus the bounce light from the world radiance cache (M7 G1, G2). The engine computes
// it identically for every program. Without a current cache value (cache off, non-traced
// frames, overflow) it is the unoccluded environment.
// A normal facing away from the receiver's viewed side (a scan micro-fold whose interpolated
// vertex normal points into the surface the camera sees) is read mirrored into that side: the
// cells hold only the viewed hemisphere's light, so the light beyond it reads as none, and such
// pixels went near-black where the uncached image does not (G2; IQ1's cache pinholes,
// docs/m7-g2.md). Continuous at the plane; normals on the viewed side are unchanged.
fn gi_viewed_side(n:vec3<f32>) -> vec3<f32> {
    let d=dot(n,gi_rx_normal);
    if d>=0.0 { return n; }
    return n-2.0*d*gi_rx_normal;
}
fn indirect_irradiance(input:SurfaceInput, n:vec3<f32>) -> vec3<f32> {
    if gi_word(GI_H_ENABLED)==0u || !gi_rx_valid { return gi_open_irradiance(n); }
    let m=gi_viewed_side(n);
    return gi_rx_sky_irradiance(m)+gi_rx_bounce_irradiance(m);
}
// ---- end of the indirect-light inputs ----
