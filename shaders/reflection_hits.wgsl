// ---- reflection hit shading (docs/design-reflection-pass.md), after the traced module ----
// The reduced-rate reflection pass's hits: rl_trace appended one 48-byte record per hit to
// H; this entry shades the records of one program (a bin of src/reflection_hits.rs) with
// the same hit code the inline path runs (rl_hit_input, the program's surface and shade,
// its lights through light_visibility) and writes each one's exposed radiance into the
// sample image R at the record's slot. Hits read the open environment and walk every
// light, as inline hits do.
// H: a 16-byte header (count, dispatch x, y, z), then three vec4<u32> per record.
@group(0) @binding(64) var<storage,read> rl_hit_records:array<vec4<u32>>;
// R(N): one texel per slot (a 2x2 quad at quad rate); rgb the radiance, a = 1 written.
@group(0) @binding(65) var rl_hit_samples_out:texture_storage_2d<rgba16float,write>;
// This dispatch's bin: its base in rl_hit_order (the bin's length, then record indices).
struct RlHitBin { base:u32, frame:u32, _pad0:u32, _pad1:u32 };
@group(0) @binding(66) var<uniform> rl_hit_bin:RlHitBin;
@group(0) @binding(67) var<storage,read> rl_hit_order:array<u32>;
// Workgroups per dispatch row; src/reflection_hits.rs ROW must match.
const RL_HITS_ROW=65535u;

// The record's direction: octahedral, snorm16 x2 (rl_trace's encoding).
fn rl_hits_direction(word:u32)->vec3<f32> {
    let e=unpack2x16snorm(word);
    var v=vec3<f32>(e.x,e.y,1.0-abs(e.x)-abs(e.y));
    if v.z<0.0 { v=vec3<f32>((1.0-abs(v.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),v.xy>=vec2<f32>(0.0)),v.z); }
    return normalize(v);
}

@compute @workgroup_size(64) fn reflection_hits(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    // The dispatch is min(n, ROW) x ceil(n / ROW) groups of 64; the surplus lanes return.
    let index=(group.x+group.y*RL_HITS_ROW)*64u+lane;
    if index>=rl_hit_order[rl_hit_bin.base] {return;}
    let at=1u+rl_hit_order[rl_hit_bin.base+1u+index]*3u;
    let a=rl_hit_records[at];
    let b=rl_hit_records[at+1u];
    let c=rl_hit_records[at+2u];
    let pixel=vec2<u32>(a.x&0xffffu,a.x>>16u);
    let slot=a.y;
    let origin=bitcast<vec3<f32>>(vec3<u32>(b.w,c.x,c.y));
    let d=rl_hits_direction(c.z);
    let cone=unpack2x16float(c.w);
    // The inline loop's per-hit state (rl_reflection_prepare): this hit's share of its
    // pixel's strata, and its path regularization.
    rl_indirect_open();
    rl_reflection_hit=true;
    rl_hit_stratum=a.z&0xffffu;
    rl_hit_strata=max(a.z>>16u,1u);
    rl_hit_variance=8.0*cone.y*cone.y;
    let h=rl_hit_input(vec4<u32>(a.w,b.x,b.y,b.z),origin,d,cone.x,cone.y,pixel);
    rl_load_parameters(h.batch);
    let hs=surface(h.input);
    let radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);
    let width=textureDimensions(rl_hit_samples_out).x;
    textureStore(rl_hit_samples_out,vec2<i32>(vec2<u32>(slot%width,slot/width)),vec4<f32>(radiance,1.0));
}
