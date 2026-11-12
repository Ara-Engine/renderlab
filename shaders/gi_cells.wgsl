// ---- renderlab world radiance cache (M7 G1, G2): cell keys, records and SH, shared ----
// Shared by the engine's cache kernels (gi_cache.wgsl), the program inputs
// (`indirect_irradiance`, gi_api.wgsl) and the cell shading entry (gi_shade_entry.wgsl).
// Each module declares `gi_table` itself: the kernels read and write it, shading and cell
// shading only read it. src/gi_cache.rs owns the layout.
//
// Cells are cubes of edge GI_MIN_EDGE*2^level, keyed by level, integer cube and a
// six-way normal bin (dominant axis of the receiver's geometric normal on its viewed
// side). Positions are never projected (C2), cells stay cubic (C3). Each slot has a
// hot record that shading reads (key, valid bit, the fixed-weight L2 SH of the
// occluded sky, a constant sky's at unit radiance, in f16 with one f32 scale, then the
// L1 SH of the bounce light its blocked directions see, G2) and a cold record only the
// kernels touch (definition, fixed query point, one visibility bit per fixed direction:
// the per-direction slots of W1, the query point's material and light visibility). Values
// are replaced on update; no field counts samples (W1, C8).

// Header words, then every hot record, then every cold record.
const GI_HEADER:u32=64u;
const GI_HOT_WORDS:u32=24u;
// LC2 light-list plumbing: dynamic cold stride, including every light bit.
const GI_H_COLD_WORDS:u32=23u;
// Hot words 16-22 (G2): the bounce SH's f32 scale, then its 12 halves (L1, coefficient i*3+c
// for band index i, channel c), divided by the scale. The scale word is 0 until the cell's
// first gather and GI_BOUNCE_NONE (-0.0) after a gather that found no light, so the lookup
// can tell a cell without a bounce value yet from one without bounce (G2c). Word 23: the
// packed query normal (as GI_C_NORMAL), which shading's agreement weight reads in the same
// load.
const GI_HOT_BOUNCE:u32=16u;
const GI_BOUNCE_NONE:u32=0x80000000u;
const GI_HOT_NORMAL:u32=23u;
// Slots searched per key, from an offset the key picks.
const GI_BUCKET:u32=8u;
// Fixed directions per cell: a uniform Fibonacci hemisphere about the query normal.
// 64 or 128; src/gi_cache.rs substitutes the setting into this line.
const GI_DIRECTIONS:u32=64u;
// P-MEM: reserve only the words this direction count reads. The CPU and cell-shading
// pipeline use the same direction-specific cold layout (src/gi_cache.rs).
const GI_WORDS:u32=GI_DIRECTIONS/32u;
const GI_MAX_WORDS:u32=GI_WORDS;

// Header layout (written by the CPU each frame; src/gi_cache.rs prepare_gi).
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
// Level blend half-width (f32, in levels): within it of a level switch, the two levels
// mix with a continuous fixed weight, so moving eyes do not pop cells; 0 is off.
const GI_H_BLEND:u32=12u;
// Ray items in use: GI_RAY_ITEMS traces rays in parallel, 0 traces each cell's rays in
// setup (one thread per cell). The same rays and bits either way.
const GI_H_ITEMS:u32=13u;
// Cut refill groups S (0 or 1 off): on the frames after a cut, a cell traces the first
// group of directions k % S == g that is still untraced, one group a frame, so a cut's
// rays spread over S frames (src/gi_cache.rs IndirectSettings::cut_phases).
const GI_H_SPREAD:u32=14u;
// Rays a gather aims around each local light in range (U-SPECK, gi_near_gather; 0: none, and
// the fixed directions keep every hit): src/gi_cache.rs IndirectSettings::near_light_rays.
const GI_H_NEAR_RAYS:u32=15u;
// Software rays with cooked near-field masks (M7 G1c): their octahedral bins per side (0:
// none).
const GI_H_NEAR_BINS:u32=16u;
// Bounce light (M7 G2): bit 0 when this frame's program reflects light (declares `scatter`
// or `emission`) and the setting is on; the work-buffer offset of the cell shading queue;
// and the gathers an event schedules for each cell it reaches (G).
const GI_H_BOUNCE:u32=17u;
const GI_H_SHADE_QUEUE:u32=18u;
const GI_H_GATHERS:u32=19u;
// The most cells a frame gathers (G2b; 0: every queued cell). More wait by fixed slot groups
// (gi_gather), so a cut's gather rays spread over frames instead of one frame taking them all.
const GI_H_GATHER_CELLS:u32=20u;
// Gather stride S (G2b; 1, 2 or 4): a gather traces the blocked directions k with k % S == 0,
// each weighted S times a direction's solid angle: a fixed subset of the fixed directions.
const GI_H_GATHER_STRIDE:u32=21u;
// The most cells a frame creates (G2b; 0: every listed miss). When more are listed (a cut lists
// every visible cell), a frame claims those whose key phase gi_mix(key a) % P matches the
// frame, P the smallest power of two (up to 64) that brings the list under it on average; the
// others stay missing (pixels read coarser cells, which a cut's first frames create: src/
// gi_cache.rs COARSE_CUT_FRAMES) and are listed again next frame. A fixed rotation by key, never
// a ranking (W2).
const GI_H_CLAIM_CELLS:u32=22u;
// Cell-space filter (G3, gi_filter): bit 0 filters the sky SH, bit 1 the bounce SH (only while
// bounce runs: its shading queue lists the cells whose values changed); word 25 is the fixed
// tent's edge weight (f32; corners take its square).
const GI_H_FILTER:u32=24u;
const GI_H_FILTER_EDGE:u32=25u;
// Nonzero: a full bucket gives up its least recently seen cell to a miss (gi_claim_miss;
// src/gi_cache.rs IndirectSettings::pressure_eviction).
const GI_H_PRESSURE:u32=26u;
// A gather hit's cell edge over the hit's distance along the ray (f32; 0: the cell a pixel
// there would read): src/gi_cache.rs IndirectSettings::hit_cone_cells.
const GI_H_HIT_SPREAD:u32=27u;
// Nonzero: the cells pixels read this frame are gathered ahead of the rest, and the value is
// the most cells pixels read in a frame (the view's pixels over a cell's): gi_gather_now,
// src/gi_cache.rs IndirectSettings::gather_pixels_first.
const GI_H_PIXEL_CELLS:u32=28u;
const GI_FILTER_SKY:u32=1u;
const GI_FILTER_BOUNCE:u32=2u;
// Bit 2: contrast refine (gi_filter_cell), with the sky filter.
const GI_FILTER_CONTRAST:u32=4u;
// Bit 3: cells not gathered yet take their gathered neighbours' filtered bounce (gi_filter_cell).
const GI_FILTER_FILL:u32=8u;
// Split cells (M7 G1c): a cell whose fixed probe lines meet two surface layers (a slab over
// a floor) is read at finer levels, up to GI_REFINE below the pixel's own; its hot record's
// last word carries GI_SPLIT_HOT in the half no SH coefficient uses. One level keeps nearly
// all of three levels' quality on the quarry (map sky 30.85% against 30.76%) at a third of
// the probe's walk cost (about 0.9 against 3 ms a frame; docs/m7-g1c.md).
const GI_REFINE:u32=1u;
const GI_SPLIT_HOT:u32=0x10000u;
// The levels a point reads and the weight of the second: one level away from a switch
// (or with the blend off), two around it. `margin` widens the band for the kernels,
// which must also cover a shading module's rounding at the switch itself.
struct GiLevels { lo:u32, hi:u32, t:f32 };
fn gi_levels(world:vec3<f32>, margin:f32) -> GiLevels {
    let x=gi_level_real(world);
    let last=gi_word(GI_H_LEVELS)-1u;
    let level=gi_level(world);
    let blend=gi_header_f32(GI_H_BLEND);
    let k=round(x);
    let d=x-k;
    if abs(d)<max(blend,margin) {
        let t=select(select(0.0,1.0,d>=0.0),clamp((d+blend)/(2.0*blend),0.0,1.0),blend>0.0);
        return GiLevels(min(u32(max(k,0.0)),last),min(u32(max(k,0.0))+1u,last),t);
    }
    return GiLevels(level,level,0.0);
}
// Flag bits.
const GI_F_TRILINEAR:u32=1u;
const GI_F_FIELD:u32=2u;
// Rotate each cell's directions about its normal by an angle its key fixes.
const GI_F_ROTATE:u32=4u;
// Probe statistics (the work counters from GI_W_P_TILES), kept on profiled frames only.
const GI_F_STATS:u32=8u;
// A pixel inside a moved caster's swept bounds also requests the cell GI_MOVER_LEVELS coarser,
// and a receiver without gathered bounce at its level or the next reads that one (gi_probe,
// gi_prepare): src/gi_cache.rs IndirectSettings::mover_fill.
const GI_F_MOVER_FILL:u32=16u;
const GI_MOVER_LEVELS:u32=2u;

// Table layout after the header: the hot records, then the cold records.
// Hot record: word 0 is key a (31 bits) with GI_VALID set while the SH is current and
// the cell has a query point (0 for an empty slot); word 1 the f32 scale; words 2..15
// the 27 SH halves (coefficient i*3+c for band i, channel c), divided by the scale. A
// lookup's first bucket probe usually hits, so the key shares the record's first sector
// (a separate dense key array measured 0.011 ms slower in shading).
const GI_VALID:u32=0x80000000u;
const GI_KEY_BITS:u32=0x7fffffffu;
// Cold record word offsets.
const GI_C_KEY_B:u32=0u;
const GI_C_STATE:u32=1u;
const GI_C_SEEN:u32=2u;
const GI_C_DEF:u32=3u;
const GI_C_CELL:u32=4u;
const GI_C_NORMAL:u32=7u;
const GI_C_POINT:u32=8u;
// This frame's first ray item (valid while directions are pending).
const GI_C_BASE:u32=11u;
const GI_C_BITS:u32=12u;
// Directions traced since the cell's last invalidation (GI_MAX_WORDS words).
const GI_C_KNOWN:u32=GI_C_BITS+GI_MAX_WORDS;
// Directions with ray items this frame, between setup and finish (GI_MAX_WORDS words).
const GI_C_PENDING:u32=GI_C_KNOWN+GI_MAX_WORDS;
// Directions the cooked near-field masks block at the query point (GI_MAX_WORDS words; zero
// without masks): never traced, always blocked, whatever moves.
const GI_C_NEAR:u32=GI_C_PENDING+GI_MAX_WORDS;
// The query point's material (G2), fixed with the point: its placement's identity + 1 (0:
// none; a draw's stable id << 16 | instance, src/gi_cache.rs `draw_ids`), the hit triangle's
// primitive (0xffffffff when the ray source cannot name it: software rays against a cooked
// cut), its barycentrics (pack2x16unorm), then one visibility bit per light at the point.
const GI_C_MATERIAL:u32=GI_C_NEAR+GI_MAX_WORDS;
const GI_C_PRIMITIVE:u32=GI_C_MATERIAL+1u;
const GI_C_BARY:u32=GI_C_MATERIAL+2u;
const GI_C_LIGHTS:u32=GI_C_MATERIAL+3u;
const GI_NO_PRIMITIVE:u32=0xffffffffu;
// State bits: the value is current; the cell has no canonical query point; some
// directions are still untraced (counted as open sky until they are); queued for
// tracing this frame; the cell is split (read at finer levels), a property of its probe
// that keeps across re-traces until the next probe.
const GI_TRACED:u32=1u;
const GI_ORPHAN:u32=2u;
const GI_PARTIAL:u32=4u;
const GI_QUEUED:u32=8u;
const GI_SPLIT:u32=32u;
// Bounce (G2): queued for cell shading this frame; queued for a gather next frame; made for a
// gather's hit (it requests no further cells); and the gathers still scheduled (bits 12-14),
// set by the event that reached it: a schedule, never a count of samples (W1, C8).
const GI_SHADE:u32=64u;
const GI_BQUEUED:u32=128u;
const GI_SECONDARY:u32=256u;
const GI_GATHER_SHIFT:u32=12u;
const GI_GATHER_MASK:u32=0x7000u;
// A gather met hit cells without a value and was given one more.
const GI_RETRY:u32=512u;
// Made for a secondary cell's gather: requests nothing (GI_SECONDARY is set too).
const GI_TERTIARY:u32=1024u;
const GI_KEEP:u32=GI_SPLIT|GI_SHADE|GI_BQUEUED|GI_SECONDARY|GI_TERTIARY|GI_RETRY|GI_GATHER_MASK;

// Each module defines gi_word(i), the table's word i: the kernels index a u32 array,
// shading a vec4 array (gi_api.wgsl) so records load whole.
fn gi_header_f32(i:u32) -> f32 { return bitcast<f32>(gi_word(i)); }
fn gi_hot(slot:u32) -> u32 { return GI_HEADER+slot*GI_HOT_WORDS; }
fn gi_cold(slot:u32) -> u32 { return GI_HEADER+(gi_word(GI_H_MASK)+1u)*GI_HOT_WORDS+slot*max(24u,gi_word(GI_H_COLD_WORDS)); }

// Level whose edge is the smallest power-of-two multiple of the minimum edge at least
// `footprint` times the eye distance. The kernels request both levels near a boundary,
// where another module's rounding may choose either.
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
// The bin of n's second-largest component (ties to the lower axis), signed by it.
fn gi_second_bin(n:vec3<f32>) -> u32 {
    let a=abs(n);
    let first=gi_bin(n)>>1u;
    var axis=select(0u,1u,first==0u);
    for(var k=0u;k<3u;k+=1u) {
        if k!=first && a[k]>a[axis] { axis=k; }
    }
    return axis*2u+select(1u,0u,n[axis]>=0.0);
}
fn gi_bin_normal(bin:u32) -> vec3<f32> {
    let s=select(1.0,-1.0,(bin&1u)!=0u);
    switch bin>>1u {
        case 0u: { return vec3<f32>(s,0.0,0.0); }
        case 1u: { return vec3<f32>(0.0,s,0.0); }
        default: { return vec3<f32>(0.0,0.0,s); }
    }
}
// 1 on the bin's normal axis, 0 on its two tangent axes.
fn gi_axis_mask(bin:u32) -> vec3<f32> {
    let a=bin>>1u;
    return vec3<f32>(f32(a==0u),f32(a==1u),f32(a==2u));
}
// The normal a pixel's bin comes from: its interpolated vertex normal turned to the
// viewed side, or the viewed plane normal where that normal vanishes. On scans, triangles
// are about a pixel wide and their plane normals flip bins from pixel to pixel.
fn gi_bin_source(viewed:vec3<f32>, vertex:vec3<f32>) -> vec3<f32> {
    if dot(vertex,vertex)<=1e-12 { return viewed; }
    return select(-vertex,vertex,dot(vertex,viewed)>=0.0);
}
// The receiver's geometric normal on the side the camera sees.
fn gi_viewed_normal(world:vec3<f32>, geometric:vec3<f32>) -> vec3<f32> {
    let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
    return select(-geometric,geometric,dot(geometric,eye-world)>=0.0);
}

// Lattice coordinates of p in cells of `edge`. Along the bin's normal axis they are
// shifted a quarter cell against the bin, so a surface lying on a cell face (the
// ground at y = 0 lies on one at every level) sits well inside one layer whatever a
// module's rounding.
fn gi_lattice(p:vec3<f32>, edge:f32, bin:u32) -> vec3<f32> {
    let s=select(0.25,-0.25,(bin&1u)!=0u);
    return p/edge-gi_axis_mask(bin)*s;
}
// The lookup's cells: bilinear over four cells in the bin's tangent plane, in the
// layer containing the point along its normal axis (W3: fixed weights, no jitter).
struct GiCorners { base:vec3<i32>, f:vec3<f32> };
fn gi_corners(u:vec3<f32>, bin:u32) -> GiCorners {
    let tangent=vec3<f32>(1.0)-gi_axis_mask(bin);
    let shifted=u-tangent*0.5;
    let base=floor(shifted);
    return GiCorners(vec3<i32>(base),(shifted-base)*tangent);
}
// Corner c (0..3) of the bin's tangent plane: one step along its first tangent axis
// when c & 1, along its second when c & 2.
fn gi_corner_offset(bin:u32, c:u32) -> vec3<i32> {
    let a=bin>>1u;
    let t1=(a+1u)%3u;
    let t2=(a+2u)%3u;
    let o1=i32(c&1u);
    let o2=i32(c>>1u);
    return vec3<i32>(select(0,o1,t1==0u)+select(0,o2,t2==0u),
        select(0,o1,t1==1u)+select(0,o2,t2==1u),
        select(0,o1,t1==2u)+select(0,o2,t2==2u));
}
fn gi_corner_weight(f:vec3<f32>, bin:u32, c:u32) -> f32 {
    let a=bin>>1u;
    let f1=f[(a+1u)%3u];
    let f2=f[(a+2u)%3u];
    return select(1.0-f1,f1,(c&1u)!=0u)*select(1.0-f2,f2,(c&2u)!=0u);
}

fn gi_mix(x:u32) -> u32 {
    var h=x;
    h^=h>>16u; h*=0x7feb352du; h^=h>>15u; h*=0x846ca68bu; h^=h>>16u;
    return h;
}
// Two independent hashes of (level, bin, cube). Key a (31 bits, never zero) is the name
// lookups compare: its low bits pick the bucket and its top three bits the start offset
// (gi_start). Key b fixes the cell's direction rotation and is kept in the cold record.
fn gi_key(level:u32, bin:u32, cell:vec3<i32>) -> vec2<u32> {
    let c=bitcast<vec3<u32>>(cell);
    let d=level|(bin<<8u);
    let a=gi_mix(c.x^gi_mix(c.y^gi_mix(c.z^gi_mix(d^0x9e3779b9u))));
    let b=gi_mix(c.z^gi_mix(c.x^gi_mix(c.y^gi_mix(d^0x85ebca6bu))));
    return vec2<u32>(max(a&GI_KEY_BITS,1u),b);
}
// The slot holding `key`, or 0xffffffff: one hot word per probed slot, starting at the
// key's own offset (where its insert put it when free). Shading and the kernels search
// identically.
// A key's start offset in its bucket: key a's top bits, which the bucket index does not
// use (it takes at most 21 low bits), so a lookup hashes key a alone (measured 21 us
// cheaper in shading than starting from key b). Key b stays the cell's rotation.
fn gi_start(key:vec2<u32>) -> u32 { return key.x>>28u; }
// An empty slot ends a search (G2b): a key is inserted in the first free or reusable slot from
// its start (gi_claim), and a slot is never emptied again except by clearing the whole table,
// so no key lies past an empty slot in its probe order. A missing key then costs about two
// loads instead of the whole bucket's eight (Nsight: the bucket walk was the quarry shading
// pass's largest hotspot, docs/m7-g2b.md). The result is the same.
fn gi_find(key:vec2<u32>) -> u32 {
    let mask=gi_word(GI_H_MASK);
    let bucket=(key.x&mask)&~(GI_BUCKET-1u);
    for(var i=0u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((gi_start(key)+i)&(GI_BUCKET-1u));
        let word=gi_word(gi_hot(slot));
        if (word&GI_KEY_BITS)==key.x { return slot; }
        if word==0u { break; }
    }
    return 0xffffffffu;
}
// gi_find with the start slot's and the next slot's key words loaded together: a key whose
// start slot was taken usually sits one slot on. The same slot. For gather hits and the filter's
// neighbours, which meet more displaced keys than pixels do; in the probe the extra load cost
// about 10-15 us a frame (P-GI, docs/p-gi.md).
fn gi_find_pair(key:vec2<u32>) -> u32 {
    let mask=gi_word(GI_H_MASK);
    let bucket=(key.x&mask)&~(GI_BUCKET-1u);
    let s0=bucket+(gi_start(key)&(GI_BUCKET-1u));
    let s1=bucket+((gi_start(key)+1u)&(GI_BUCKET-1u));
    let w0=gi_word(gi_hot(s0));
    let w1=gi_word(gi_hot(s1));
    if (w0&GI_KEY_BITS)==key.x { return s0; }
    if w0==0u { return 0xffffffffu; }
    if (w1&GI_KEY_BITS)==key.x { return s1; }
    if w1==0u { return 0xffffffffu; }
    for(var i=2u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((gi_start(key)+i)&(GI_BUCKET-1u));
        let word=gi_word(gi_hot(slot));
        if (word&GI_KEY_BITS)==key.x { return slot; }
        if word==0u { break; }
    }
    return 0xffffffffu;
}
// SH coefficient j (band j / 3, channel j % 3) of a hot record, before its scale.
fn gi_half(hot:u32, j:u32) -> f32 {
    let pair=unpack2x16float(gi_word(hot+2u+(j>>1u)));
    return select(pair.x,pair.y,(j&1u)!=0u);
}

// Real L2 spherical harmonics of d, in the order 00, 1-1, 10, 11, 2-2, 2-1, 20, 21, 22,
// written to gi_y: a private array, never passed or returned by value (a by-value array
// costs a local-memory copy per call).
var<private> gi_y:array<f32,9>;
fn gi_sh_basis(d:vec3<f32>) {
    gi_y[0]=0.282095;
    gi_y[1]=0.488603*d.y; gi_y[2]=0.488603*d.z; gi_y[3]=0.488603*d.x;
    gi_y[4]=1.092548*d.x*d.y; gi_y[5]=1.092548*d.y*d.z;
    gi_y[6]=0.315392*(3.0*d.z*d.z-1.0);
    gi_y[7]=1.092548*d.x*d.z; gi_y[8]=0.546274*(d.x*d.x-d.y*d.y);
}
// Clamped-cosine convolution weights per band (Ramamoorthi and Hanrahan 2001).
fn gi_band_weight(i:u32) -> f32 {
    if i==0u { return 3.141592653589793; }
    if i<4u { return 2.0943951023931953; }
    return 0.7853981633974483;
}
// Bounce coefficient j (L1: band index j / 3, channel j % 3) of a hot record, before its scale.
fn gi_bounce_half(hot:u32, j:u32) -> f32 {
    let pair=unpack2x16float(gi_word(hot+GI_HOT_BOUNCE+1u+(j>>1u)));
    return select(pair.x,pair.y,(j&1u)!=0u);
}
// A cell's irradiance about n from its own hot record: the occluded sky in cached units
// (unit radiance for a constant sky, the map's radiance before intensity: apply
// gi_sky_scale's rule) and the bounce light in lux. Overwrites gi_y.
fn gi_cell_sky_irradiance(hot:u32, n:vec3<f32>) -> vec3<f32> {
    gi_sh_basis(n);
    var e=vec3<f32>(0.0);
    for(var i=0u;i<9u;i+=1u) {
        e+=gi_band_weight(i)*gi_y[i]*vec3<f32>(gi_half(hot,3u*i),gi_half(hot,3u*i+1u),gi_half(hot,3u*i+2u));
    }
    return max(e*bitcast<f32>(gi_word(hot+1u)),vec3<f32>(0.0));
}
fn gi_cell_bounce_irradiance(hot:u32, n:vec3<f32>) -> vec3<f32> {
    let scale=bitcast<f32>(gi_word(hot+GI_HOT_BOUNCE));
    if scale<=0.0 { return vec3<f32>(0.0); }
    gi_sh_basis(n);
    var e=vec3<f32>(0.0);
    for(var i=0u;i<4u;i+=1u) {
        e+=gi_band_weight(i)*gi_y[i]*vec3<f32>(gi_bounce_half(hot,3u*i),gi_bounce_half(hot,3u*i+1u),gi_bounce_half(hot,3u*i+2u));
    }
    return max(e*scale,vec3<f32>(0.0));
}
// Branchless orthonormal basis (Duff et al. 2017); the third column is n.
fn gi_basis(n:vec3<f32>) -> mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);
    let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
fn gi_unpack_normal(w:u32) -> vec3<f32> {
    let q=unpack2x16snorm(w);
    var n=vec3<f32>(q,1.0-abs(q.x)-abs(q.y));
    let t=max(-n.z,0.0);
    n=vec3<f32>(n.x+select(t,-t,n.x>=0.0),n.y+select(t,-t,n.y>=0.0),n.z);
    return normalize(n);
}
// Band-1 attenuation of a GGX lobe of perceptual roughness r about a direction, through its
// Phong equivalent cos^s (s = 2/alpha^2 - 2): (s + 1)/(s + 2), 1 for a mirror and 1/2 for a
// lobe as wide as the hemisphere. Used to read L1 bounce radiance in a specular lobe.
fn gi_lobe_band1(roughness:f32) -> f32 {
    let alpha=max(roughness*roughness,1e-4);
    let s=max(2.0/(alpha*alpha)-2.0,0.0);
    return (s+1.0)/(s+2.0);
}

// ---- end of the shared cache definitions ----
