// ---- renderlab world radiance cache (M7 G1): cell keys, slots and SH, shared ----
// Shared by the engine's cache kernels (gi_cache.wgsl) and the program input
// `indirect_irradiance` (gi_api.wgsl). Each module declares `gi_table` itself: the
// kernels read and write it, shading only reads it. src/gi_cache.rs owns the layout.
//
// Cells are cubes of edge GI_MIN_EDGE*2^level at integer coordinates floor(p/edge),
// keyed by level, cube and a six-way normal bin (dominant axis of the receiver's
// geometric normal on its viewed side). Positions are never projected (C2), cells stay
// cubic (C3). A slot holds the cell's definition, its fixed query point, one visibility
// bit per fixed direction (the per-direction slots of W1) and the fixed-weight L2 SH
// projection of the occluded sky over those directions, replaced on every update.
// No field counts samples (W1, C8).

// Header words, then GI_SLOT_WORDS per slot.
const GI_HEADER:u32=64u;
const GI_SLOT_WORDS:u32=48u;
// Slots searched per key, from an offset the key picks.
const GI_BUCKET:u32=8u;
// Fixed directions per cell: a uniform Fibonacci hemisphere about the query normal.
// 64 or 128; src/gi_cache.rs substitutes the setting into this line.
const GI_DIRECTIONS:u32=64u;
// Visibility words per cell, and the most any setting uses.
const GI_WORDS:u32=GI_DIRECTIONS/32u;
const GI_MAX_WORDS:u32=4u;

// Header layout (written by the CPU each frame; src/gi_cache.rs Header).
const GI_H_ENABLED:u32=0u;
const GI_H_MASK:u32=1u;
const GI_H_FLAGS:u32=2u;
const GI_H_FRAME:u32=3u;
const GI_H_FOOTPRINT:u32=4u;
const GI_H_MIN_EDGE:u32=5u;
const GI_H_LEVELS:u32=6u;
const GI_H_EVICT:u32=7u;
const GI_H_EYE:u32=8u;
// Direction phases S: 1 replaces every direction on each update (W1 form 1); S > 1
// re-traces the directions k % S == frame % S and keeps the others (W1 form 2).
const GI_H_PHASES:u32=11u;
// Flag bits.
const GI_F_TRILINEAR:u32=1u;
const GI_F_FIELD:u32=2u;
// Rotate each cell's directions about its normal by an angle its key fixes.
const GI_F_ROTATE:u32=4u;

// Slot word offsets.
const GI_S_KEY_A:u32=0u;
const GI_S_KEY_B:u32=1u;
const GI_S_STATE:u32=2u;
const GI_S_SEEN:u32=3u;
const GI_S_DEF:u32=4u;
const GI_S_CELL:u32=5u;
const GI_S_POINT:u32=8u;
const GI_S_NORMAL:u32=11u;
const GI_S_BITS:u32=12u;
// Directions traced since the cell's last invalidation (GI_MAX_WORDS words).
const GI_S_KNOWN:u32=16u;
const GI_S_SH:u32=20u;
// State bits: the value is current; the cell has no canonical query point; some
// directions are still untraced (counted as open sky until they are).
const GI_TRACED:u32=1u;
const GI_ORPHAN:u32=2u;
const GI_PARTIAL:u32=4u;
// Kernel-only: queued for tracing this frame.
const GI_QUEUED:u32=8u;

fn gi_word(i:u32) -> u32 { return gi_table[i]; }
fn gi_header_f32(i:u32) -> f32 { return bitcast<f32>(gi_table[i]); }
fn gi_slot_base(slot:u32) -> u32 { return GI_HEADER+slot*GI_SLOT_WORDS; }

// Level whose edge is the smallest power-of-two multiple of the minimum edge at least
// `footprint` times the eye distance. `margin` widens the comparison for callers that
// must cover a neighbouring module's rounding (the kernels request both levels).
fn gi_level_real(world:vec3<f32>) -> f32 {
    let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
    let wanted=max(length(world-eye)*gi_header_f32(GI_H_FOOTPRINT),1e-30);
    return max(log2(wanted/gi_header_f32(GI_H_MIN_EDGE)),0.0);
}
fn gi_level(world:vec3<f32>) -> u32 {
    return min(u32(ceil(gi_level_real(world))),gi_word(GI_H_LEVELS)-1u);
}
fn gi_edge(level:u32) -> f32 { return gi_header_f32(GI_H_MIN_EDGE)*exp2(f32(level)); }

// Six bins: +x -x +y -y +z -z by the dominant component (ties to the lower axis).
fn gi_bin(n:vec3<f32>) -> u32 {
    let a=abs(n);
    if a.x>=a.y && a.x>=a.z { return select(1u,0u,n.x>=0.0); }
    if a.y>=a.z { return select(3u,2u,n.y>=0.0); }
    return select(5u,4u,n.z>=0.0);
}
fn gi_bin_normal(bin:u32) -> vec3<f32> {
    let s=select(1.0,-1.0,(bin&1u)!=0u);
    switch bin>>1u {
        case 0u: { return vec3<f32>(s,0.0,0.0); }
        case 1u: { return vec3<f32>(0.0,s,0.0); }
        default: { return vec3<f32>(0.0,0.0,s); }
    }
}
// The receiver's geometric normal on the side the camera sees.
fn gi_viewed_normal(world:vec3<f32>, geometric:vec3<f32>) -> vec3<f32> {
    let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
    return select(-geometric,geometric,dot(geometric,eye-world)>=0.0);
}

fn gi_mix(x:u32) -> u32 {
    var h=x;
    h^=h>>16u; h*=0x7feb352du; h^=h>>15u; h*=0x846ca68bu; h^=h>>16u;
    return h;
}
// Two independent 32-bit names of (level, bin, cube). Key a picks the bucket; key b
// the start offset within it. Zero marks an empty slot, so a is never zero.
fn gi_key(level:u32, bin:u32, cell:vec3<i32>) -> vec2<u32> {
    let c=bitcast<vec3<u32>>(cell);
    let d=level|(bin<<8u);
    let a=gi_mix(c.x^gi_mix(c.y^gi_mix(c.z^gi_mix(d^0x9e3779b9u))));
    let b=gi_mix(c.z^gi_mix(c.x^gi_mix(c.y^gi_mix(d^0x85ebca6bu))));
    return vec2<u32>(max(a,1u),b);
}
// The slot holding `key`, or 0xffffffff. Shading and the kernels search identically.
fn gi_find(key:vec2<u32>) -> u32 {
    let mask=gi_word(GI_H_MASK);
    let bucket=(key.x&mask)&~(GI_BUCKET-1u);
    for(var i=0u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((key.y+i)&(GI_BUCKET-1u));
        let base=gi_slot_base(slot);
        if gi_table[base+GI_S_KEY_A]==key.x && gi_table[base+GI_S_KEY_B]==key.y { return slot; }
    }
    return 0xffffffffu;
}

// Real L2 spherical harmonics, in the order 00, 1-1, 10, 11, 2-2, 2-1, 20, 21, 22.
struct GiSh { c:array<f32,9> };
fn gi_sh_basis(d:vec3<f32>) -> GiSh {
    var y:GiSh;
    y.c[0]=0.282095;
    y.c[1]=0.488603*d.y; y.c[2]=0.488603*d.z; y.c[3]=0.488603*d.x;
    y.c[4]=1.092548*d.x*d.y; y.c[5]=1.092548*d.y*d.z;
    y.c[6]=0.315392*(3.0*d.z*d.z-1.0);
    y.c[7]=1.092548*d.x*d.z; y.c[8]=0.546274*(d.x*d.x-d.y*d.y);
    return y;
}
// Clamped-cosine convolution weights per band (Ramamoorthi and Hanrahan 2001).
fn gi_band_weight(i:u32) -> f32 {
    if i==0u { return 3.141592653589793; }
    if i<4u { return 2.0943951023931953; }
    return 0.7853981633974483;
}

// ---- end of the shared cache definitions ----
