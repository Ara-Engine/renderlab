// ---- T2 world-anchored sample records: shared encoding (docs/m6-motion-aa.md) ----
// Appended after the T1 sequence to the motion trace kernel module and to every program's
// motion shading module (film_records.rs). Binding-free.

// K, the records a pixel holds, set per pipeline (film_records.rs FilmMotion). Slot 0 is the
// pixel's fresh sample of the last moving frame (the fresh-record target); slots 1..K-1 are
// moved records, layers 0..K-2 of the moved-record array, valid ones first.
override FILM_RECORDS:u32=8u;
// The largest K - 1, which sizes register arrays.
const FILM_MOVED_MAX:u32=7u;

// A record is one exact camera-ray sample in 16 bytes (rgba32uint):
//   x: colour r, g as f16 (the program's exposed output after rl_finite)
//   y: colour b as f16 | (placement + 1) << 16, where zero is background. Background records
//      carry no colour: they take the frame's background, which never changes without a reset.
//   z: the hit's depth in the frame the record was last expressed in: near / view depth,
//      the reversed-Z depth of the view-projection with the far plane at infinity
//      (Camera::infinite_matrix), f32. Background samples are directions: depth 0.
//   w: position in the pixel holding it, 12 bits per axis in 1/4096 px (x | y << 12),
//      age << 24 (frames since it was traced, 4 bits, saturating), cold << 28, tile << 29,
//      side << 30, valid << 31.
// A record moves as its point (position, depth): it is re-expressed in each new frame's pixel
// grid and never interpolated. The tile bit is set on the first moved record of a trace tile's
// first pixel when the tile keeps explicit records; other tiles hold implied background
// records (film_records_kernel.wgsl film_implied). The cold bit marks, on an invalid first
// moved record, a pixel the validation left without a record although its fresh sample was
// jittered: the second trace pass retraces it at the pixel centre.
const FILM_VALID:u32=0x80000000u;
const FILM_SIDE:u32=0x40000000u;
const FILM_TILE_BIT:u32=0x20000000u;
const FILM_COLD:u32=0x10000000u;
// Test diagnostics only, on valid records (the cold bit's position): the sample is a suspect
// of the known receiver-origin defect at concave triangle edges (docs/m6-temporal-gpu.md
// section 0.3): its hit lies within 1e-4 barycentric of an edge and all 8 rays are blocked.
const FILM_BAND:u32=0x10000000u;
const FILM_AGE_MAX:u32=15u;

// A position in [0, 1)^2 to 12 bits per axis, rounded to nearest. Lattice positions
// (film_offset: odd multiples of 1/512) are exact.
fn film_quantize(position:vec2<f32>)->u32 {
    let q=min(vec2<u32>(round(position*4096.0)),vec2<u32>(4095u));
    return q.x|(q.y<<12u);
}
fn film_position(w:u32)->vec2<f32> {
    return vec2<f32>(f32(w&0xfffu),f32((w>>12u)&0xfffu))*(1.0/4096.0);
}
fn film_age(w:u32)->u32 {
    return (w>>24u)&0xfu;
}
fn film_placement(r:vec4<u32>)->u32 {
    return r.y>>16u;
}
// A record's colour, or `background` for a background record.
fn film_colour(r:vec4<u32>,background:vec3<f32>)->vec3<f32> {
    if film_placement(r)==0u {return background;}
    return vec3<f32>(unpack2x16float(r.x),unpack2x16float(r.y).x);
}
// The record of a sample with colour `c`, placement + 1, reversed-Z depth, quantized position
// and side. Its age is zero.
fn film_record(c:vec3<f32>,placement:u32,depth:f32,position:u32,side:bool)->vec4<u32> {
    return vec4<u32>(pack2x16float(c.rg),(pack2x16float(vec2<f32>(c.b,0.0))&0xffffu)|(placement<<16u),
        bitcast<u32>(depth),position|select(0u,FILM_SIDE,side)|FILM_VALID);
}

// A fresh sample's plane record (film_records_kernel.wgsl film_plane), one per pixel per moving
// frame: x = its record depth (0: background), y = its geometric normal, octahedral snorm16,
// z = placement + 1 in bits 0..22, side in bit 23, its sequence index k in bits 24..31.
fn film_plane_index(plane:vec4<u32>)->u32 {
    return plane.z>>24u;
}
fn film_plane_placement(plane:vec4<u32>)->u32 {
    return plane.z&0x7fffffu;
}
fn film_plane_side(plane:vec4<u32>)->bool {
    return ((plane.z>>23u)&1u)!=0u;
}
// ---- end of the T2 record encoding ----
