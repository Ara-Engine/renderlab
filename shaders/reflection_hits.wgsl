// ---- reflection hit shading (docs/design-reflection-pass.md), after the traced module ----
// The reduced-rate reflection pass's hits: rl_trace appended one 48-byte record per hit to
// H; this entry shades them with the same hit code the inline path runs (rl_hit_input, the
// program's surface and shade, its lights through its own cluster and light_visibility) and
// writes each one's exposed radiance into the sample image R at the record's slot. Hits read
// the open environment, as inline hits do. One indirect dispatch over all of H with
// rl_hit_args's arguments per frame program; in a mixed-program frame each program's module
// keeps its own draws' hits (rl_hits_mine).
// H: a 16-byte header (count, dispatch x, y, z), then three vec4<u32> per record.
@group(0) @binding(64) var<storage,read> rl_hit_records:array<vec4<u32>>;
// R: one texel per slot (a 2x2 quad); rgb the radiance, a = 1 written.
@group(0) @binding(65) var rl_hit_samples_out:texture_storage_2d<rgba16float,write>;
// Workgroups per dispatch row (rl_hit_args's RL_HIT_GROUP-record groups, rows of 65535).
const RL_HITS_ROW=65535u;
// Whether this module shades the hit on `placement`: every hit in a single-program frame. A
// mixed-program frame runs each program's module over all of H, and each keeps the hits on
// its own draws (program::compose_hits with lists replaces this function).
fn rl_hits_mine(placement:u32)->bool { return true; }

@compute @workgroup_size(64) fn reflection_hits(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    // The dispatch is min(g, ROW) x ceil(g / ROW) groups of 64; the surplus lanes return.
    let index=(group.x+group.y*RL_HITS_ROW)*64u+lane;
    if index>=rl_hit_records[0].x {return;}
    let at=1u+index*3u;
    let a=rl_hit_records[at];
    if !rl_hits_mine(a.w-1u) {return;}
    let b=rl_hit_records[at+1u];
    let c=rl_hit_records[at+2u];
    let pixel=vec2<u32>(a.x&0xffffu,a.x>>16u);
    let slot=a.y;
    let origin=bitcast<vec3<f32>>(vec3<u32>(b.w,c.x,c.y));
    let d=rl_oct_decode(c.z);
    let cone=unpack2x16float(c.w);
    let record=vec4<u32>(a.w,b.x,b.y,b.z);
    let width=textureDimensions(rl_hit_samples_out).x;
    let at_slot=vec2<i32>(vec2<u32>(slot%width,slot/width));
    // Word 2: stratum (bits 0-7), strata (8-15), the lobe's GGX alpha as f16 (16-31).
    let alpha=unpack2x16float(a.z>>16u).x;
    // A rough lobe's hit whose cone covers the radiance cache's cell there takes the cell's
    // outgoing radiance (RENDERLAB_RL_CACHE_HITS, reflection_cache_hits.wgsl), as inline hits do.
    let cached=rl_cache_hit(record,origin,cone.x,cone.y,sqrt(alpha));
    if cached.w>0.0 {
        textureStore(rl_hit_samples_out,at_slot,vec4<f32>(rl_finite(expose(cached.rgb)),1.0));
        return;
    }
    // The inline loop's per-hit state (rl_reflection_prepare): this hit's share of its
    // pixel's strata, and its path regularization.
    rl_indirect_open();
    rl_reflection_hit=true;
    rl_hit_stratum=a.z&0xffu;
    rl_hit_strata=max((a.z>>8u)&0xffu,1u);
    rl_hit_variance=8.0*cone.y*cone.y;
    let h=rl_hit_input(record,origin,d,cone.x,cone.y,pixel);
    // LC1 at the hit: its own cluster where that walk is exact, as inline hits take it.
    rl_hit_lights_prepare(h.input.world);
    rl_load_parameters(h.batch);
    let hs=surface(h.input);
    let radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);
    textureStore(rl_hit_samples_out,at_slot,vec4<f32>(radiance,1.0));
}
