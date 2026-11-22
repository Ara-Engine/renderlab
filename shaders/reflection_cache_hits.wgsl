// ---- cache-terminated reflection hits (RENDERLAB_RL_CACHE_HITS, src/program.rs) ----
// Composed after shaders/reflections.wgsl on traced stages (hardware and software reflection
// traces: the software seam passes the traversal's own distance) whose program reads the
// world radiance cache, only while the switch names a minimum roughness; every other module
// gets a stub that returns no value, so its hits shade as before.
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

// RENDERLAB_GI_BORROW (gi_cells.wgsl gi_borrow_*): what a cell `slot` at `level` without a bounce
// of its own adds from the nearest coarser cell around `world` in `bin` that has one (not a split
// cell), or 0. As gi_cache.wgsl gi_borrow_hit, on this module's binding.
fn rl_cache_borrow(world:vec3<f32>, level:u32, bin:u32, slot:u32) -> vec3<f32> {
    if !gi_borrow_on() { return vec3<f32>(0.0); }
    let r=rl_cache_outgoing[gi_borrow_at(slot)];
    if gi_borrow_own(r) { return vec3<f32>(0.0); }
    let last=min(level+1u+GI_BORROW_LEVELS,gi_word(GI_H_LEVELS));
    for(var l=level+1u;l<last;l+=1u) {
        let p=gi_find_pair(gi_key(l,bin,vec3<i32>(floor(gi_lattice(world,gi_edge(l),bin)))));
        if p==0xffffffffu || (gi_word(gi_hot(p)+15u)&GI_SPLIT_HOT)!=0u { continue; }
        let lw=rl_cache_outgoing[p].w;
        if lw<=0.0 || lw==0.5 || lw==1.5 { continue; }
        let lender=rl_cache_outgoing[gi_borrow_at(p)];
        if gi_borrow_own(lender) { return gi_borrow_radiance(r,lender); }
    }
    return vec3<f32>(0.0);
}
// The cached outgoing radiance (rgb, unexposed) and w 1, or w 0 to shade the hit: a hit
// `record` a distance `t` along `d` from `origin`, whose ray cone is `cone` metres wide there.
fn rl_cache_hit(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,t:f32,cone:f32,rough:f32)->vec4<f32> {
    if rough<RL_CACHE_ROUGH { return vec4<f32>(0.0); }
    return rl_cache_lookup(record,origin,d,t,cone);
}
fn rl_cache_lookup(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,t:f32,cone:f32)->vec4<f32> {
    if gi_word(GI_H_ENABLED)==0u { return vec4<f32>(0.0); }
    let world=fma(d,vec3<f32>(t),origin);
    let base=gi_level(world);
    if cone<gi_edge(base) { return vec4<f32>(0.0); }
    let hit=primary_decode(record);
    // A skinned or moved placement's cells lag its surface (they belong to where it was), so
    // its hits always take the full program.
    if batch_skinned(primary_batch_info(hit.batch)) || rl_cache_moved(hit.placement) { return vec4<f32>(0.0); }
    let level=rl_cache_level(world,cone);
    let bin=rl_cache_bin(record,origin,world);
    let slot=gi_find_pair(gi_key(level,bin,vec3<i32>(floor(gi_lattice(world,gi_edge(level),bin)))));
    // Only a settled value (w >= 1.0: 1.0 own bounce, 1.5 needs borrow); changing cells hold
    // 0.75 / 0.5 and absent or unshaded ones hold 0.
    if slot==0xffffffffu { return vec4<f32>(0.0); }
    let o=rl_cache_outgoing[slot];
    if o.w<1.0 { return vec4<f32>(0.0); }
    var rgb=o.rgb;
    if o.w==1.5 { rgb+=rl_cache_borrow(world,level,bin,slot); }
    return vec4<f32>(rgb,1.0);
}
// The final gather's probe-hit read (gi_receiver_pass.wgsl), its only hit path: the outgoing
// radiance (rgb, unexposed) of the hit's cell, settled or changing, w 0 when no cell there has a
// value or the cache is off. The cell is the one a pixel at the hit, seen from the ray's origin,
// reads: its bin from the interpolated vertex normal on the side the ray sees (gi_bin_source, as
// gi_prepare picks it). Levels in order: the one the cone covers, then the finer ones down to the
// hit's own (where pixels on screen made cells: a probe ray from the floor to the tower's side met
// only those). Never coarser than the cone: a coarser cell spans whatever lies within its edge on
// the same side, and around thin geometry that includes light from the other side. Glint's course
// underglow lit the top of the spinner's pad through the +Y cells that held both the thin pad
// and the surfaces beneath it (79/130/204 there with the underglow alone, against the reference's
// 0.8/1.2/1.6). Split cells above the own level are passed over for the same reason (gicut's
// slab-roof furnace: the roof's -Y cells at the cone level also held the lit underside 0.5 m
// below, and the slab's dark top read 0.35). A hit no level answers is left out of the probe's average (gi_receiver_pass.wgsl
// fg_lane). Movers read their cells too: those lag the surface by a frame or so, which a probe's
// hemisphere integral does not show. With no value it leaves the cell to request in rl_cache_missed, as
// the cache's external request list takes it (level | bin << 8, then the lattice cell; x
// 0xffffffff: none).
var<private> rl_cache_missed:vec4<u32>;
// The hit's geometric normal (unnormalized), for the reader's own use of the same triangle.
var<private> rl_cache_geometric:vec3<f32>;
// The slots the last rl_cache_cell used (0xffffffff: none): x the cell whose value it returned,
// y the cone level's cell when it exists. A reader that can report them (the final gather, through
// the cache's external list) keeps them alive: its reads count as the cache's own (gi_cache.wgsl
// GI_EXTERNAL_SEEN).
var<private> rl_cache_slots:vec2<u32>;
fn rl_cache_cell(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,t:f32,cone:f32)->vec4<f32> {
    rl_cache_slots=vec2<u32>(0xffffffffu);
    if gi_word(GI_H_ENABLED)==0u { return vec4<f32>(0.0); }
    let world=fma(d,vec3<f32>(t),origin);
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);
    let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let geometric=primary_geometric(ta.position,tb.position,tc.position,hit.placement);
    rl_cache_geometric=geometric;
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    let vertex=vertex_output(ta).normal*w.x+vertex_output(tb).normal*w.y+vertex_output(tc).normal*w.z;
    let bin=gi_bin(gi_bin_source(select(-geometric,geometric,dot(geometric,origin-world)>=0.0),vertex));
    let own=gi_level(world);
    let start=rl_cache_level(world,cone);
    var at_start=0xffffffffu;
    var at_own=0xffffffffu;
    for(var k=0u;k<=start-own;k+=1u) {
        let l=start-k;
        let slot=gi_find_pair(gi_key(l,bin,vec3<i32>(floor(gi_lattice(world,gi_edge(l),bin)))));
        if l==start { at_start=slot; }
        if l==own { at_own=slot; }
        if slot==0xffffffffu { continue; }
        // A split cell above the hit's own level is not read: its probe lines met two surface
        // layers facing its bin (a slab's underside and a roof's above it), and its value mixes
        // them. The cache's own lookup reads finer there too (gi_sample_refined).
        if l>own && (gi_word(gi_hot(slot)+15u)&GI_SPLIT_HOT)!=0u { continue; }
        let o=rl_cache_outgoing[slot];
        if o.w>0.0 {
            rl_cache_slots=vec2<u32>(slot,at_start);
            var rgb=o.rgb;
            if o.w==0.5 || o.w==1.5 { rgb+=rl_cache_borrow(world,l,bin,slot); }
            // Report w=0.5 while the hit cell is still ungathered or actively bouncing so final
            // gather probes avoid locking cold startup/cut zero-bounce cache frames into long history.
            let fixed_settle=gi_word(GI_H_FIXED_RAYS)!=0u && gi_word(GI_H_FIXED_SETTLE)!=0u;
            let cold=o.w<1.0 || o.w==1.5 || (fixed_settle && (gi_word(gi_cold(slot)+GI_C_STATE)&GI_SETTLED)==0u);
            return vec4<f32>(rgb,select(1.0,0.5,cold));
        }
    }
    rl_cache_slots.y=at_start;
    // The cell to ask for: the cone level's when it is absent; the hit's own level's when the cone
    // level's is an orphan (its probe found no surface at that level, as at a thin part's coarse
    // cells, so it never gets a value) or split, and the own one is absent; none while the cell
    // there only waits for its first gather.
    rl_cache_missed=vec4<u32>(0xffffffffu);
    var ask=0xffffffffu;
    if at_start==0xffffffffu { ask=start; }
    else if (gi_word(gi_cold(at_start)+GI_C_STATE)&GI_ORPHAN)!=0u && at_own==0xffffffffu { ask=own; }
    else if (gi_word(gi_hot(at_start)+15u)&GI_SPLIT_HOT)!=0u && at_own==0xffffffffu { ask=own; }
    if ask!=0xffffffffu {
        rl_cache_missed=vec4<u32>(ask|(bin<<8u),bitcast<vec3<u32>>(vec3<i32>(floor(gi_lattice(world,gi_edge(ask),bin)))));
    }
    return vec4<f32>(0.0);
}
// The level whose cells a cone `cone` metres wide at `world` covers, at least the point's own.
fn rl_cache_level(world:vec3<f32>,cone:f32)->u32 {
    let covered=u32(ceil(log2(max(cone,1e-6)/gi_header_f32(GI_H_MIN_EDGE))));
    return min(max(gi_level(world),covered),gi_word(GI_H_LEVELS)-1u);
}
// The cache bin of a hit `record` at `world`: its geometric normal turned toward `origin`.
fn rl_cache_bin(record:vec4<u32>,origin:vec3<f32>,world:vec3<f32>)->u32 {
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);
    let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let geometric=primary_geometric(ta.position,tb.position,tc.position,hit.placement);
    return gi_bin(select(-geometric,geometric,dot(geometric,origin-world)>=0.0));
}
// The placement's model changed since the previous frame (models holds both frames).
fn rl_cache_moved(slot:u32)->bool {
    let a=models[slot];let b=models[arrayLength(&models)/2u+slot];
    let eps=vec4<f32>(1e-5);
    return any(abs(a[0]-b[0])>eps)||any(abs(a[1]-b[1])>eps)
        ||any(abs(a[2]-b[2])>eps)||any(abs(a[3]-b[3])>eps);
}
// ---- end of cache-terminated reflection hits ----
