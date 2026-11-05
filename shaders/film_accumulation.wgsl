// ---- T1 temporal film accumulation: sample sequence (docs/m6-temporal-primaries.md) ----
// Appended after primary_rays.wgsl to the temporal trace module and to every program's
// temporal shading head (primary_ray_gpu.rs film_kernel_source, film_shading_head). Each
// frame traces one jittered primary per pixel with Interactive's eight disk directions,
// rotated by the pixel's sequence index; film_store.wgsl accumulates the program output.

// Frame state written by the CPU (src/film_accumulation.rs). It is storage and never feeds
// addressing (docs/m6-interactive-climb-2.md). Nonzero reset discards every history.
struct FilmFrame { reset:u32, pad0:u32, pad1:u32, pad2:u32 };
@group(0) @binding(24) var<storage,read> film_frame:FilmFrame;
// The previous frame's anchors (film_store.wgsl writes them). x: placement + 1 (zero is
// background) in bits 0..22, the sequence index k in bits 24..31. y: the latest
// sample's view distance, f32 bits.
@group(0) @binding(25) var film_anchor_in:texture_2d<u32>;

// The current sample's position inside its pixel and its disk-direction offset. The entry
// sets both before primary_ray and fresh_estimate read them (FILM_*_SEAM edits).
var<private> film_sample:vec2<f32>=vec2<f32>(0.5);
var<private> film_rotation:u32=0u;

// This frame's sequence index at `pixel`: zero after a reset, else the previous index + 1
// (mod 256). A reset frame reads no history.
fn film_index(pixel:vec2<i32>)->u32 {
    if film_frame.reset!=0u {return 0u;}
    return ((textureLoad(film_anchor_in,pixel,0).x>>24u)+1u)&0xffu;
}

// src/film_accumulation.rs GENERATOR must match.
const FILM_GENERATOR:u32=71u;
// Position of sample k in its pixel, in [0, 1)^2: the 256-point rank-1 lattice
// (j, 71 j mod 256) / 256 in bit-reversed order of j, so every power-of-two prefix is a
// lattice and the 16-point prefix is the T1 design's (71 = 7 mod 16). The first four are
// reordered to centre, quarter pair, corner: k = 0 is the pixel centre exactly, so a reset
// pixel's first sample is Interactive's ray. Per pixel, one of four symmetries fixing the
// centre (identity, point reflection, transpose, both) decorrelates neighbours. Reflection
// maps every prefix lattice to itself; transpose does for prefixes up to 16 (7 * 7 = 1 mod 16).
fn film_offset(k:u32,pixel:vec2<u32>,width:u32)->vec2<f32> {
    var order=k;
    if k<4u {order=(0x0231u>>(4u*k))&0xfu;}
    let j=reverseBits(order)>>24u;
    var p=vec2<u32>(j,(FILM_GENERATOR*j)&0xffu);
    let s=fresh_hash((pixel.x+pixel.y*width)^0x2f6b1e5du);
    if (s&1u)!=0u {p=(vec2<u32>(256u)-p)&vec2<u32>(0xffu);}
    if (s&2u)!=0u {p=p.yx;}
    return vec2<f32>(p)/256.0;
}

// Selects sample k at `pixel`: its in-pixel position and directions 8 (k mod 32) onward of
// the pixel's 256-direction Exact sequence, so index 0 traces Interactive's directions.
fn film_begin(k:u32,pixel:vec2<u32>,width:u32) {
    film_sample=film_offset(k,pixel,width);
    film_rotation=8u*(k&31u);
}
// An anchor's view distance of the hit triangle's receiver point (R1's barycentric order),
// with every multiply-add explicit: the program-independent interpolation is not used, and
// long-lived pipelines cannot re-contract it (docs/determinism.md).
fn film_view_distance(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,w:vec3<f32>,placement:u32)->f32 {
    let p=receiver_point(a,b,c,w);let m=models[placement];
    let x=fma(m[0],vec4<f32>(p.x),fma(m[1],vec4<f32>(p.y),fma(m[2],vec4<f32>(p.z),m[3])));
    let d=x.xyz-lighting.eye;
    return sqrt(fma(d.x,d.x,fma(d.y,d.y,d.z*d.z)));
}
// ---- end of the T1 sample sequence ----
