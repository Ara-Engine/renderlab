// ---- T1 temporal film accumulation: sample sequence and reprojection (docs/m6-temporal-primaries.md) ----
// Appended after primary_rays.wgsl to the temporal trace module and to every program's
// temporal shading head (film_accumulation.rs kernel_source, shading_head). Each frame
// traces one jittered primary per pixel with Interactive's eight disk directions, rotated by
// the pixel's sequence index; film_store.wgsl accumulates the program output.

// Frame state written by the CPU (src/film_accumulation.rs FilmState). It is storage, not a
// uniform (docs/m6-interactive-climb-2.md). reset: every pixel starts again and no history is
// read. frames: frames since the last reset. materialize (tests): keep history in every tile.
// moving: the camera changed since the previous frame, so history is reprojected and
// validated; otherwise each pixel's own history is exact. implied: the weight of an unkept
// tile's background history, the previous frames at the previous frame's camera (at most
// 256). previous and current are view-projections; previous_eye.w is the previous frame's
// pixel angle (radians per pixel at the image centre).
struct FilmFrame {
    reset:u32, frames:u32, materialize:u32, moving:u32,
    implied:u32, pad0:u32, pad1:u32, pad2:u32,
    previous:mat4x4<f32>,
    previous_inverse:mat4x4<f32>,
    current:mat4x4<f32>,
    previous_eye:vec4<f32>,
};
@group(0) @binding(24) var<storage,read> film_frame:FilmFrame;
// The previous frame's anchors (film_store.wgsl writes them). x: placement + 1 (zero is
// background) in bits 0..22, the thin-material viewed side in bit 23, the sequence index k
// in bits 24..31. y: the latest sample's view distance, f32 bits.
@group(0) @binding(25) var film_anchor_in:texture_2d<u32>;
// Per trace tile, nonzero while the tile keeps history (the previous frame's flags; see
// film_keep_tile). Every pixel of any other tile has seen nothing but background since the
// reset, so its history is implied: the background sample with weight film_frame.implied and
// index frames - 1.
@group(0) @binding(29) var film_tiles_in:texture_2d<u32>;

// The current sample's position inside its pixel and its disk-direction offset. The entry
// sets both before primary_ray and fresh_estimate read them (FILM_*_SEAM edits).
var<private> film_sample:vec2<f32>=vec2<f32>(0.5);
var<private> film_rotation:u32=0u;
// The sequence index whose sample the entry traces or shades.
var<private> film_k:u32=0u;

// Whether `pixel`'s tile kept history in the previous frame.
fn film_kept(pixel:vec2<i32>)->bool {
    return textureLoad(film_tiles_in,vec2<u32>(pixel)/PRIMARY_TILE,0).x!=0u;
}

// This frame's sequence index at `pixel`: zero after a reset, else the previous index + 1
// (mod 256). A reset frame reads no history.
fn film_index(pixel:vec2<i32>)->u32 {
    if film_frame.reset!=0u {return 0u;}
    if !film_kept(pixel) {return film_frame.frames&0xffu;}
    return ((textureLoad(film_anchor_in,pixel,0).x>>24u)+1u)&0xffu;
}

// src/film_accumulation.rs GENERATOR must match.
const FILM_GENERATOR:u32=71u;
// Position of sample k in its pixel, in (0, 1)^2. k = 0 is the pixel centre exactly, so a
// reset pixel's first sample is Interactive's ray. Other indices follow the 256-point rank-1
// lattice (j, 71 j mod 256) / 256 in bit-reversed order of j, moved (8.5, 8.5) / 256 px so no
// sample lies on a pixel edge: a lattice node at the pixel corner weighted a region of zero
// measure in the box filter at 1/256 (docs/m6-temporal-gpu.md). Every power-of-two prefix
// is the moved lattice with its centre point replaced by the exact centre, and the 16-point
// prefix is the T1 design's (71 = 7 mod 16). Indices 1-3 are the quarter pair and the former
// corner. Per pixel, one of four symmetries fixing the centre (identity, point reflection,
// transpose, both) decorrelates neighbours; it acts on the lattice before the move.
// Reflection maps every prefix lattice to itself; transpose does for prefixes up to 16.
fn film_offset(k:u32,pixel:vec2<u32>,width:u32)->vec2<f32> {
    if k==0u {return vec2<f32>(0.5);}
    var order=k;
    if k<4u {order=(0x0231u>>(4u*k))&0xfu;}
    let j=reverseBits(order)>>24u;
    var p=vec2<u32>(j,(FILM_GENERATOR*j)&0xffu);
    let s=fresh_hash((pixel.x+pixel.y*width)^0x2f6b1e5du);
    if (s&1u)!=0u {p=(vec2<u32>(256u)-p)&vec2<u32>(0xffu);}
    if (s&2u)!=0u {p=p.yx;}
    return (vec2<f32>((p+vec2<u32>(8u))&vec2<u32>(0xffu))+vec2<f32>(0.5))/256.0;
}

// Selects sample k at `pixel`: its in-pixel position and directions 8 (k mod 32) onward of
// the pixel's 256-direction Exact sequence, so index 0 traces Interactive's directions.
fn film_begin(k:u32,pixel:vec2<u32>,width:u32) {
    film_k=k;
    film_sample=film_offset(k,pixel,width);
    film_rotation=8u*(k&31u);
}

// History decisions use explicit multiply-adds throughout: long-lived pipelines may not
// re-contract them, so replays stay bit-identical (docs/determinism.md).
fn film_dot(a:vec3<f32>,b:vec3<f32>)->f32 {
    return fma(a.x,b.x,fma(a.y,b.y,a.z*b.z));
}
fn film_length(v:vec3<f32>)->f32 {
    return sqrt(film_dot(v,v));
}
// World position of the hit triangle's receiver point (R1's barycentric order).
fn film_world(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,w:vec3<f32>,placement:u32)->vec3<f32> {
    let p=receiver_point(a,b,c,w);let m=models[placement];
    return fma(m[0],vec4<f32>(p.x),fma(m[1],vec4<f32>(p.y),fma(m[2],vec4<f32>(p.z),m[3]))).xyz;
}

// The current sample's surface: world point (w = 1) or background direction (w = 0),
// placement + 1 (zero: background) and, for thin materials, the viewed side.
struct FilmSurface { point:vec4<f32>, placement:u32, side:u32 };

// The surface of a hit on triangle (a, b, c) (object space, index order) at barycentric
// weights w of placement slot `slot`. The trace and the shading compute it identically.
fn film_hit_surface(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,w:vec3<f32>,slot:u32)->FilmSurface {
    let x=film_world(a,b,c,w,slot);
    let side=film_dot(primary_geometric(a,b,c,slot),lighting.eye-x)>=0.0;
    return FilmSurface(vec4<f32>(x,1.0),slot+1u,u32(side));
}
// The background surface of the camera ray `primary_ray` builds for `pixel` (through
// film_sample).
fn film_miss_surface(pixel:vec2<u32>,size:vec2<u32>)->FilmSurface {
    let ray=primary_ray(pixel,size);
    let d=ray.far-ray.near;
    return FilmSurface(vec4<f32>(d/film_length(d),0.0),0u,0u);
}
// A surface's view distance from this frame's eye (zero for background).
fn film_distance(s:FilmSurface)->f32 {
    if s.placement==0u {return 0.0;}
    return film_length(s.point.xyz-lighting.eye);
}

// Continuous pixel position (pixel centres at + 0.5) of `p` under view-projection `m`: a point
// when p.w = 1, a direction when p.w = 0.
fn film_pixel(m:mat4x4<f32>,p:vec4<f32>,size:vec2<f32>)->vec2<f32> {
    let c=fma(m[0],vec4<f32>(p.x),fma(m[1],vec4<f32>(p.y),fma(m[2],vec4<f32>(p.z),m[3]*p.w)));
    let centre=0.5*size;
    return fma(c.xy/c.w,centre*vec2<f32>(1.0,-1.0),centre);
}

// Where `pixel`'s history lies in the previous frame, in continuous pixel coordinates: its
// centre moved by the motion of the current sample's surface point (instances are static in
// T1b; background moves by direction).
fn film_fetch(pixel:vec2<u32>,s:FilmSurface,size:vec2<f32>)->vec2<f32> {
    let motion=film_pixel(film_frame.previous,s.point,size)-film_pixel(film_frame.current,s.point,size);
    return vec2<f32>(pixel)+vec2<f32>(0.5)+motion;
}

// The previous frame's anchor at a tap inside the image whose tile flag is `kept`: stored, or
// implied (background, index frames - 1).
fn film_anchor_at(tap:vec2<i32>,kept:bool)->vec2<u32> {
    if kept {return textureLoad(film_anchor_in,tap,0).xy;}
    return vec2<u32>(((film_frame.frames-1u)&0xffu)<<24u,0u);
}
fn film_inside(tap:vec2<i32>,size:vec2<u32>)->bool {
    return all(tap>=vec2<i32>(0)) && all(tap<vec2<i32>(size));
}

// Forward test of a tap's anchor `a`: its last sample shows the current surface: the same
// placement (or both background), the same side for thin materials, and a view distance from
// the previous eye within four pixel footprints of the current point's.
fn film_forward(a:vec2<u32>,s:FilmSurface)->bool {
    if (a.x&0x7fffffu)!=s.placement {return false;}
    if s.placement==0u {return true;}
    if (primary_placements[s.placement-1u]&0x20000000u)!=0u && ((a.x>>23u)&1u)!=s.side {return false;}
    let d=film_length(s.point.xyz-film_frame.previous_eye.xyz);
    return abs(bitcast<f32>(a.y)-d)<=4.0*film_frame.previous_eye.w*d;
}

// Whether any inner bilinear tap of `pixel`'s fetch passes the forward test.
fn film_forward_inner(pixel:vec2<u32>,s:FilmSurface,size:vec2<u32>)->bool {
    let q=film_fetch(pixel,s,vec2<f32>(size))-vec2<f32>(0.5);
    let base=vec2<i32>(floor(q));
    for(var i=0;i<4;i+=1) {
        let tap=base+vec2<i32>(i&1,i>>1u);
        if film_inside(tap,size) && film_forward(film_anchor_at(tap,film_kept(tap)),s) {return true;}
    }
    return false;
}
// ---- end of the T1 sample sequence and reprojection helpers ----
