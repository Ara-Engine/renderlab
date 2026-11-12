// ---- cache-terminated reflection hits (RENDERLAB_RL_CACHE_HITS, src/program.rs) ----
// Composed after shaders/reflections.wgsl on traced stages whose program reads the world
// radiance cache, only while the switch names a minimum roughness; every other module gets
// a stub that returns no value, so its hits shade as before.
// A glossy hit whose ray cone at the hit is at least as wide as the cache's cell there
// reads that cell's outgoing radiance (gi_shade_entry.wgsl: the program's emission plus its
// Lambertian reflection of the sky, the bounce and the lights its visibility ray reached)
// instead of rebuilding the hit's receiver and running its `surface` and `shade`. The cell
// is found at the level gi_hit_level (gi_cache.wgsl) picks for a gather hit's cone, in the
// bin of the hit's geometric normal turned toward the ray. Smooth lobes (perceptual
// roughness under RL_CACHE_ROUGH), contacts (a cone narrower than the hit's own pixel
// level's cell), and absent or unshaded cells return w 0 and keep the full shading.
@group(3) @binding(28) var<storage,read> rl_cache_outgoing:array<vec4<f32>>;
const RL_CACHE_ROUGH:f32=RL_CACHE_ROUGH_MIN;

// The cached outgoing radiance (rgb, unexposed) and w 1, or w 0 to shade the hit: a hit
// `record` a distance `t` along `d` from `origin`, whose ray cone is `cone` metres wide there.
fn rl_cache_hit(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,t:f32,cone:f32,rough:f32)->vec4<f32> {
    if rough<RL_CACHE_ROUGH || gi_word(GI_H_ENABLED)==0u { return vec4<f32>(0.0); }
    let world=fma(d,vec3<f32>(t),origin);
    let base=gi_level(world);
    if cone<gi_edge(base) { return vec4<f32>(0.0); }
    let last=gi_word(GI_H_LEVELS)-1u;
    let level=min(max(base,u32(ceil(log2(cone/gi_header_f32(GI_H_MIN_EDGE))))),last);
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);
    let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let geometric=primary_geometric(ta.position,tb.position,tc.position,hit.placement);
    let bin=gi_bin(select(-geometric,geometric,dot(geometric,origin-world)>=0.0));
    let slot=gi_find_pair(gi_key(level,bin,vec3<i32>(floor(gi_lattice(world,gi_edge(level),bin)))));
    if slot==0xffffffffu { return vec4<f32>(0.0); }
    let o=rl_cache_outgoing[slot];
    if !(o.w>0.0) { return vec4<f32>(0.0); }
    return vec4<f32>(o.rgb,1.0);
}
// ---- end of cache-terminated reflection hits ----
