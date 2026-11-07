// N8k (docs/non-rt-n8k.md): the map-only sun visibility lookup, `VirtualLookup::Map`. Per pixel
// it reads light-space map data only: no triangle, vertex or adjacency is fetched. It shares
// virtual_shadow.wgsl's clipmap (top and peel layers of (height, triangle id) words, max-mips,
// page requests and cache) and adds, per texel at page-draw time (vmap_resolve):
//   - the light-space height plane (gradient) of the top and peel triangles and the facing of
//     their winding toward the sun, so heights are reconstructed between texel centres
//     (receiver-independent per-texel depth planes, as in adaptive depth bias);
//   - the top plane's prediction error at its same-placement neighbours: a planarity bound
//     that biases a receiver's own relief by what the map can actually resolve.
// Per pixel, for Interactive's eight directions:
//   - directions below the receiver's exact geometric tangent plane are blocked;
//   - the others march N8b's geometric height schedule through the receiver's level and the
//     coarser ones, skipping steps whose max-mip footprint lies below the ray (N8c);
//   - where a surface may be above the ray, the 2x2 texel centres around the sample rebuild it
//     from their planes. Surfaces under the receiver's tangent plane (plus their planarity
//     error) and the receiver's own triangle are ignored. The ray then rises to the lowest
//     surface over it (top or peel layer) until it is at a surface that still covers it
//     (blocked) or nothing covers it any more (it passed beside or under that surface). The
//     coverage is the 2x2 bilinear weight of covering texels (percentage-closer filtering).
// virtual_shadow_gpu.rs prepends PRIMARY_BATCHES and owns every layout below; the shared
// structures and helpers are copies of virtual_shadow.wgsl's and must keep its layouts.

const VSM_PAGE:u32=128u;
const VSM_NONE:u32=0xffffffffu;
const VSM_MAX_LEVELS:u32=8u;
const VSM_MAX_RECTS:u32=16u;
// primary_rays.wgsl PRIMARY_TILE.
const VSM_TRACE_TILE=vec2<u32>(8u,4u);
struct VsmLevel {origin:vec4<f32>,info:vec4<u32>};
// virtual_shadow.wgsl's VsmFrame, field for field.
struct VsmFrame {
    u:vec4<f32>,v:vec4<f32>,l:vec4<f32>,eye:vec4<f32>,depth:vec4<f32>,params:vec4<f32>,
    counts:vec4<u32>,misc:vec4<u32>,
    levels:array<VsmLevel,VSM_MAX_LEVELS>,
    rects:array<vec4<f32>,VSM_MAX_RECTS>,
    srects:array<vec4<f32>,VSM_MAX_RECTS>,
    horizon:array<vec4<u32>,16>,
    records:array<vec4<u32>,32>,
    n8e:vec4<u32>,
};
@group(0) @binding(0) var<uniform> vsm:VsmFrame;

fn vsm_light(world:vec3<f32>)->vec3<f32> {
    let rel=world-vsm.eye.xyz;
    return vec3<f32>(dot(rel,vsm.u.xyz),dot(rel,vsm.v.xyz),dot(world,vsm.l.xyz));
}
fn vsm_texel(level:u32)->f32 {return vsm.levels[level].origin.z;}
fn vsm_cap(level:u32)->f32 {return f32(VSM_PAGE-4u)*vsm_texel(level)/max(vsm.params.z,1e-3);}
fn vsm_reach(level:u32,height:f32)->f32 {return max(0.0,min(vsm_cap(level),vsm.depth.w-height));}
fn vsm_radius(level:u32,height:f32)->f32 {
    return vsm.params.z*vsm_reach(level,height)+3.0*vsm_texel(level);
}
fn vsm_word(cell:vec2<i32>,level:u32)->u32 {
    let side=vsm.counts.y*VSM_PAGE;
    return (level*side+u32(cell.y))*side+u32(cell.x);
}

// ---- The clipmap layers (virtual_shadow.wgsl's), as (id, normalized height bits) ----
@group(0) @binding(16) var<storage,read> vsm_top_words:array<vec2<u32>>;
@group(0) @binding(19) var<storage,read> vsm_peel_words:array<vec2<u32>>;
fn vsm_height(word:vec2<u32>)->f32 {return fma(bitcast<f32>(word.y),vsm.depth.z,vsm.depth.x);}

// ---- Per-texel planes (vmap_resolve): x top plane, y peel plane; and their errors ----
// A plane word is pack2x16float of the light-space height gradient (metres per metre of light
// xy), with bit 0 replaced by the winding's facing toward the sun. An error word is
// pack2x16float of the top and peel planes' largest prediction error at a same-placement
// 4-neighbour inside the page (metres; 0 without one). Two buffers, as each stays within the
// binding limit the layers' 8 bytes per texel already meet.
@group(0) @binding(40) var<storage,read> vmap_planes:array<vec2<u32>>;
@group(0) @binding(41) var<storage,read_write> vmap_planes_out:array<vec2<u32>>;
@group(0) @binding(42) var<storage,read> vmap_errors:array<u32>;
@group(0) @binding(43) var<storage,read_write> vmap_errors_out:array<u32>;
@group(0) @binding(29) var<storage,read> vsm_render:array<u32>;
struct VsmVec4s {data:array<vec4<f32>>};
struct VsmU32s {data:array<u32>};
@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<VsmVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<VsmU32s,PRIMARY_BATCHES>;
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;
// Gradients steeper than this (a surface seen nearly edge-on from the sun) are stored flat;
// their planarity error then carries the uncertainty.
const VMAP_STEEP:f32=64.0;

// The light-space gradient and facing (+1 toward the sun, -1 away) of map triangle `id`.
fn vmap_triangle_plane(id:u32)->vec3<f32> {
    let bits=vsm.misc.x;
    let placement=(id>>bits)-1u;
    let primitive=id&((1u<<bits)-1u);
    let batch=primary_placements[placement]&0x1fffffffu;
    let base=primitive*3u;
    let m=models[placement];
    let a=vsm_light((m*vec4<f32>(primary_vertices[batch].data[primary_indices[batch].data[base]*3u].xyz,1.0)).xyz);
    let b=vsm_light((m*vec4<f32>(primary_vertices[batch].data[primary_indices[batch].data[base+1u]*3u].xyz,1.0)).xyz);
    let c=vsm_light((m*vec4<f32>(primary_vertices[batch].data[primary_indices[batch].data[base+2u]*3u].xyz,1.0)).xyz);
    let n=cross(b-a,c-a);
    let facing=select(-1.0,1.0,n.z>0.0);
    if abs(n.z)*VMAP_STEEP<=length(n.xy) {return vec3<f32>(0.0,0.0,facing);}
    return vec3<f32>(-n.xy/n.z,facing);
}
fn vmap_pack(plane:vec3<f32>)->u32 {
    return (pack2x16float(plane.xy)&~1u)|u32(plane.z>0.0);
}
fn vmap_gradient(word:u32)->vec2<f32> {return unpack2x16float(word);}

// One workgroup per page slot: a page rasterized this frame gets every texel's planes and
// planarity errors; the other pages keep theirs, as their layer words are unchanged. Errors
// only look at neighbours inside the page, so they never read another page's words.
@compute @workgroup_size(256) fn vmap_resolve(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let slot=group.x;
    if vsm_render[slot]==0u {return;}
    let pages=vsm.counts.y;let per=pages*pages;
    let level=slot/per;let local=slot%per;
    let side=pages*VSM_PAGE;
    let corner=vec2<u32>(local%pages,local/pages)*VSM_PAGE;
    let texel=vsm_texel(level);
    let bits=vsm.misc.x;
    for(var k=li;k<VSM_PAGE*VSM_PAGE;k+=256u) {
        let inside=vec2<u32>(k%VSM_PAGE,k/VSM_PAGE);
        let t=corner+inside;
        let i=(level*side+t.y)*side+t.x;
        var out=vec2<u32>(0u);
        var errors=vec2<f32>(0.0);
        for(var layer=0u;layer<2u;layer+=1u) {
            var word=vsm_top_words[i];
            if layer==1u {word=vsm_peel_words[i];}
            if word.x==0u {continue;}
            let plane=vmap_triangle_plane(word.x);
            let packed=vmap_pack(plane);
            if layer==0u {out.x=packed;} else {out.y=packed;}
            let g=vmap_gradient(packed);
            let height=vsm_height(word);
            var error=0.0;
            for(var n=0u;n<4u;n+=1u) {
                let step=vec2<i32>(select(select(0,1,n==1u),-1,n==0u),select(select(0,1,n==3u),-1,n==2u));
                let at=vec2<i32>(inside)+step;
                if any(at<vec2<i32>(0)) || any(at>=vec2<i32>(i32(VSM_PAGE))) {continue;}
                let j=i32(i)+step.y*i32(side)+step.x;
                // The neighbour's surface of this layer's placement: its top, or its peel.
                var other=vsm_top_words[j];
                if other.x>>bits!=word.x>>bits {other=vsm_peel_words[j];}
                if other.x==0u || other.x>>bits!=word.x>>bits {continue;}
                error=max(error,abs(vsm_height(other)-(height+dot(g,vec2<f32>(step)*texel))));
            }
            if layer==0u {errors.x=error;} else {errors.y=error;}
        }
        vmap_planes_out[i]=out;
        vmap_errors_out[i]=pack2x16float(min(errors,vec2<f32>(65504.0)));
    }
}


// ---- N8k page-work helpers: mip dispatches and the top layer's min-mips ----
// The max-mip (virtual_shadow.wgsl vsm_mip) and min-mip dispatches of this frame: per mip
// 1..log2(side), (x, y, z) workgroups, all zero when no page was flagged for rendering (the
// caster cull's indirect arguments, which vsm_compact sets once a page is flagged), so camera
// motion inside the windows rebuilds no mip: their inputs did not change.
@group(0) @binding(37) var<storage,read> vmap_cull_args:array<u32,4>;
@group(0) @binding(46) var<storage,read_write> vmap_mip_args:array<u32>;
@compute @workgroup_size(16) fn vmap_mip_dispatch(@builtin(local_invocation_index) i:u32) {
    let side=vsm.counts.y*VSM_PAGE;
    let mip=i+1u;
    var n=vec3<u32>(0u);
    if (1u<<mip)<=side && vmap_cull_args[0]!=0u {
        let s=((side>>mip)+7u)/8u;
        n=vec3<u32>(s,s,vsm.counts.x);
    }
    vmap_mip_args[3u*i]=n.x;vmap_mip_args[3u*i+1u]=n.y;vmap_mip_args[3u*i+2u]=n.z;
}
// Min-mips of the top layer's heights per level, laid out as virtual_shadow.wgsl's max-mips
// (vsm_mip_base), rebuilt like them: a cell inside one page only where that page was
// rasterized this frame. Empty texels count as the lowest height.
struct VsmPass {level:u32,pad0:u32,pad1:u32,pad2:u32};
@group(3) @binding(30) var<uniform> vsm_pass:VsmPass;
@group(0) @binding(44) var<storage,read_write> vmap_minmips_out:array<f32>;
@compute @workgroup_size(8,8) fn vmap_minmip(@builtin(global_invocation_id) id:vec3<u32>) {
    let mip=vsm_pass.level;let level=id.z;
    if level>=vsm.counts.x {return;}
    let side=vsm.counts.y*VSM_PAGE;let s=side>>mip;
    if any(id.xy>=vec2<u32>(s)) {return;}
    if (1u<<mip)<=VSM_PAGE {
        let pages=vsm.counts.y;
        let page=(id.xy<<vec2<u32>(mip))/VSM_PAGE;
        if vsm_render[(level*pages+page.y)*pages+page.x]==0u {return;}
    }
    var h=3.4e38;
    for(var k=0u;k<4u;k+=1u) {
        let c=id.xy*2u+vec2<u32>(k&1u,k>>1u);
        if mip==1u {
            h=min(h,bitcast<f32>(vsm_top_words[(level*side+c.y)*side+c.x].y));
        } else {
            h=min(h,vmap_minmips_out[vsm_mip_base(level,mip-1u)+c.y*(2u*s)+c.x]);
        }
    }
    vmap_minmips_out[vsm_mip_base(level,mip)+id.y*s+id.x]=h;
}

// ---- N8k hybrid queue (VirtualLookup::Hybrid; virtual_shadow_hybrid.wgsl reads it) ----
// Word 0 the count, words 1-3 reserved, then two words per receiver: x | y << 16, and its
// ambiguous blocked directions (bits 0-7) | every direction it blocked (8-15).
@group(0) @binding(47) var<storage,read_write> vmap_queue:array<atomic<u32>>;
@group(0) @binding(48) var<storage,read_write> vmap_queue_dispatch:array<u32,4>;
// The exact pass's indirect dispatch over the queue, 32 lanes a workgroup. N8m
// (docs/non-rt-n8m.md): its lanes per queued receiver, chosen per frame. A queue as large as a camera-moving frame's runs one receiver per lane. A smaller one
// (a rolling frame's) is spread over up to VMAP_QUEUE_SPREAD lanes each, up to VMAP_QUEUE_LANES
// lanes in all, so fewer of its heavy, divergent walks share a warp. The spread goes in the
// queue's word 1 for vsm_lookup_queued.
override VMAP_QUEUE_SPREAD:u32=8u;
override VMAP_QUEUE_LANES:u32=8192u;
@compute @workgroup_size(1) fn vmap_queue_args() {
    let n=atomicLoad(&vmap_queue[0]);
    let spread=clamp(VMAP_QUEUE_LANES/max(n,1u),1u,max(VMAP_QUEUE_SPREAD,1u));
    atomicStore(&vmap_queue[1],spread);
    vmap_queue_dispatch[0]=(n*spread+31u)/32u;vmap_queue_dispatch[1]=1u;vmap_queue_dispatch[2]=1u;
}
fn vmap_hybrid()->bool {return (vsm.misc.w&256u)!=0u;}

// ---- Lookup ----
@group(0) @binding(7) var vsm_hit:texture_2d<u32>;
@group(0) @binding(8) var vsm_tiles:texture_2d<u32>;
@group(0) @binding(9) var vsm_world:texture_2d<f32>;
@group(0) @binding(10) var vsm_normal:texture_2d<f32>;
@group(0) @binding(15) var vsm_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(18) var vsm_level_in:texture_2d<u32>;
@group(0) @binding(35) var<storage,read> vsm_mips:array<f32>;
struct LightingUniform {eye:vec3<f32>,count:u32,environment:vec3<f32>,exposure:f32,map:vec4<f32>};
struct Light {position_range:vec4<f32>,travel_outer:vec4<f32>,rgb_inner:vec4<f32>,kind:vec4<u32>};
struct ShadowView {world_to_clip:mat4x4<f32>,coverage_bias:vec4<f32>,camera_depth:vec4<f32>,texel:vec4<f32>};
@group(3) @binding(0) var<uniform> lighting:LightingUniform;
@group(3) @binding(1) var<storage,read> lights:array<Light>;
@group(3) @binding(5) var<storage,read> shadow_views:array<ShadowView>;

// Screen constants (variant screens set them; the defaults are the shipped lookup).
// The bilinear coverage that blocks: 0.5 centres silhouettes, 1 needs all four centres. A
// surface of the receiver's own placement needs VMAP_OWN_COVER (its relief near the receiver is
// where the map's sub-texel error lies; docs/non-rt-n8k.md).
override VMAP_COVER:f32=0.5;
override VMAP_OWN_COVER:f32=0.5;
// Within VMAP_OWN_REACH texels (light xy) of the receiver, where an own surface's silhouette or
// fold cannot be told from the receiver's own relief, its crossings need VMAP_OWN_NEAR.
override VMAP_OWN_NEAR:f32=0.5;
override VMAP_OWN_REACH:f32=1.5;
// Height tolerances in texels: every reconstructed height, and the crossing test.
override VMAP_ABS:f32=0.02;
override VMAP_CROSS:f32=0.05;
// Multiple of a texel's planarity error that biases it against the receiver's filter planes.
override VMAP_CURVE:f32=1.0;
// Crossing refinements after the candidate sample.
override VMAP_REFINE:u32=4u;
// A direction is taken as entering the receiver's own surface when the cosine between it and
// the receiver's geometric normal is at most this (scan facets' normals tilt: a ray slightly
// below one facet's plane can still clear a convex crest, as the exact tests find).
override VMAP_FACING:f32=0.0;
// The own-surface floor (vmap_own_surface): 0 the receiver's facet only; 1 also the map's own
// rebuilt surface closest to the receiver at its point; 2 the highest of every plausible own
// surface the 2x2 centres around the receiver rebuild (an own surface then cannot block inside
// the receiver's bilinear cell). Plausible: within VMAP_PLAUS texels (plus its slope over a
// texel and its planarity error) of the receiver's height.
override VMAP_OWN:u32=2u;
override VMAP_PLAUS:f32=1.0;
// Plausibility 0: |gap| within VMAP_PLAUS texels times (1 + the surface's slope), plus its error;
// 1: |gap| within VMAP_PLAUS times (a quarter texel + the slope difference to the receiver's
// facet over the distance rebuilt), plus its error: an own overhang above the receiver is then
// not taken for the receiver's surface, while a steep receiver's own neighbours still are.
// 2: the surface's height above the receiver, rebuilt at the receiver from its plane or at its
// centre from the receiver's facet (whichever is lower), within VMAP_PLAUS quarter texels; no
// planarity error, which is largest at the folds and overhangs that do shadow the receiver.
override VMAP_PLAUS_MODE:u32=1u;
// The own-surface planes floor own surfaces only within this many texels (light xy) of the
// receiver: extrapolated farther, a steep plane would bury real occluders.
override VMAP_OWN_FAR:f32=2.0;
// The facing test also against the own-surface planes: a direction below the facet's plane is
// not taken as entering the receiver when it rises above any own-surface plane (1), above
// their mean gradient's plane (2), or above all of them (3); 0 off. Modes 1-3 freed umbra
// receivers' directions on the shadow side, whose plausible own planes belong to the sunlit
// side (docs/non-rt-n8k.md). 4 and 5 are 1 and 2 over the own planes whose winding faces the
// sun as the receiver's does (the plane word's facing bit, vmap_resolve). 6 is 4 for receivers
// seen from their winding's side with that side facing the sun only: the matched planes then
// face the sun too, and a ray rising above one leaves the solid under it (over a plane facing
// away, rising above it enters the solid). 7: the closest plausible own plane only (the map's
// source surface at the receiver), when it faces the sun: no-RT receivers are LOD triangles whose
// facets tilt from the source surface the film reference is traced on (docs/non-rt-n8k.md).
override VMAP_FACING_OWN:u32=6u;
// Mode 7 frees a direction only when it rises above that plane by at least this sine of
// elevation: a fully lit receiver's disk clears its surface, a terminator receiver's does not.
override VMAP_FACE_MARGIN:f32=0.0;
// Mode 8: mode 7's test (with the margin) frees facing blocks only when every other direction
// came out open (and there is one), as on a fully lit receiver; the freed directions are then
// marched. Receivers whose every direction faces away are left blocked: their closest own plane
// is often a sun-facing neighbour across a crease (docs/non-rt-n8k.md). Partial
// receivers keep their facing blocks, so the freeing does not lighten penumbrae.
// VMAP_LONE: with every other direction open, up to this many blocked directions whose blocks
// were all ambiguous are released (0 off).
override VMAP_LONE:u32=0u;
// VMAP_LONE's ambiguous blocks: near own surfaces only (default), or also partial coverages.
override VMAP_LONE_PARTIAL:bool=false;
// Mode 9: as 7, but the own plane must pass within VMAP_FACE_PLAUS texels of the receiver point
// itself (no slope or planarity allowance), so a sunlit surface across a light-space silhouette
// above a receiver facing away is not taken for its own; every receiver, all directions.
override VMAP_FACE_PLAUS:f32=0.5;
// Ring hybrid (with virtual_shadow_hybrid.wgsl's VSM_QUEUED_RING): queue only near-field
// ambiguous blocks (facing, an own surface near the receiver), with a far-field march of those
// directions that skips own surfaces within VMAP_FAR_REACH texels of the receiver; the exact ring
// then decides their near field.
override VMAP_RING:bool=false;
// Soft decisions (percentage-closer filtering at the crossing): coverage within VMAP_SOFT of the
// blocking threshold blocks a direction by (coverage - threshold + VMAP_SOFT) / (2 VMAP_SOFT); it
// keeps marching, and counts 1 minus its largest partial block (0: binary decisions).
override VMAP_SOFT:f32=0.0;
var<private> vmap_partial:f32;
// Per direction its largest partial block, 8 bits each (directions 0-3, 4-7).
var<private> vmap_soft0:u32;
var<private> vmap_soft1:u32;
fn vmap_soft_note(j:u32,w:f32) {
    let q=u32(round(clamp(w,0.0,1.0)*255.0));
    let shift=8u*(j&3u);
    var word=select(vmap_soft0,vmap_soft1,j>=4u);
    word=(word&~(255u<<shift))|(max((word>>shift)&255u,q)<<shift);
    if j<4u {vmap_soft0=word;} else {vmap_soft1=word;}
}
fn vmap_soft_of(j:u32)->f32 {return f32((select(vmap_soft0,vmap_soft1,j>=4u)>>(8u*(j&3u)))&255u)/255.0;}
override VMAP_FAR_REACH:f32=0.75;
var<private> vmap_far_mode:bool;
// VMAP_TRACE 3 marks a facing block as ambiguous when the direction lies within this cosine
// below the receiver's facet plane.
override VMAP_AMB_FACING:f32=0.05;
// Envelope floors only from own planes whose winding faces the sun as the receiver's does.
override VMAP_OWN_MATCH:bool=false;
// When the 2x2 centres around the receiver hold only its own-surface planes (or nothing), its
// contact and half-texel samples are clear: nothing there can cover them (vmap_own_surface).
// The march schedule: 0 N8b's geometric heights from `near`; 1 a contact sample at `near`, one
// where the cone's footprint reaches half a texel, then geometric (the samples in between all
// share the receiver's bilinear cell, and a ray under a surface there is already under it at
// the contact sample).
override VMAP_SCHEDULE:u32=1u;
// Screens: the peel layer, the per-texel planes (off: centre heights), the tangent-plane
// filter, the max-mip skip; VMAP_SKIP 1 writes lit after setup, 2 skips the march, 3 writes lit
// before any setup (dispatch and page work only); VMAP_TRACE 1 writes per-direction causes, 2
// why open directions stayed open, and VMAP_COST the 2x2 cover tests and max-mip tests instead
// of counts (vmap_lookup).
override VMAP_PEEL:bool=true;
override VMAP_PLANES:bool=true;
override VMAP_TANGENT:bool=true;
override VMAP_MIP:bool=true;
override VMAP_SKIP:u32=0u;
override VMAP_CLEAN:bool=true;
override VMAP_TRACE:u32=0u;
override VMAP_COST:bool=false;
// March samples per coarser level (N8b's continuation).
override VMAP_EXTRA_SAMPLES:u32=2u;
// The certain-umbra test (vmap_march): when the lowest top-layer centre over the cone's
// footprint, out to the height where its rays would meet it, is above every ray and the
// receiver's tangent plane by VMAP_UMBRA_PAD texels, all directions are blocked.
override VMAP_UMBRA:bool=true;
override VMAP_UMBRA_PAD:f32=0.25;
// The local caster ceiling: the march ends at the highest top-layer centre the cone's footprint
// can reach (two refinements, each at the finest level whose reach holds it), instead of at the
// scene's highest caster; and the receiver level's geometric steps (after the contact and
// half-texel samples) keep about this ratio, so a short range takes few steps. Schedule 1 only.
override VMAP_CEILING:bool=true;
override VMAP_RATIO:f32=1.75;

// fresh_shadows.wgsl's per-pixel R2 sequence (virtual_shadow.wgsl's copy), so the lookup
// evaluates Interactive's directions.
fn vsm_hash(input:u32)->u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
fn vsm_direction(index:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32)->vec3<f32> {
    let r=fract(vec2<f32>(f32(index))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}

// Per-pixel state, set once: the eight directions, the frame's active ones, and the receiver.
var<private> vmap_dirs:array<vec3<f32>,8>;
var<private> vmap_active:u32;
// The receiver: its triangle's map id (LOD receivers' triangles are not in the map, so theirs
// never matches) and placement + 1; its light-space point; its facet's tangent plane (height
// gradient) and whether it faces the sun, as rays of directions above it stay above it, so no
// surface under it can block them.
// winding_lit: the receiver's winding normal faces the sun.
struct VmapReceiver {own:u32,placement:u32,point:vec3<f32>,slope:vec2<f32>,facing:bool,winding_lit:bool};
var<private> vmap_r:VmapReceiver;
// The own-surface planes (height at the receiver's point, gradient) under which no surface
// of the receiver's placement blocks either (vmap_own_surface).
// Registers, not an array: a dynamically indexed private array lives in local memory, which
// the profile showed as 9% of the lookup's samples. Unused planes sit at -3.4e38.
var<private> vmap_own0:vec3<f32>;
var<private> vmap_own1:vec3<f32>;
var<private> vmap_own2:vec3<f32>;
var<private> vmap_own3:vec3<f32>;
var<private> vmap_own_count:u32;
// Bit i: own plane i's winding faces the sun as the receiver's does.
var<private> vmap_own_match:u32;
// The closest plausible own plane over all four centres (height at the receiver, gradient), and
// whether its winding faces the sun (VMAP_FACING_OWN 7).
var<private> vmap_own_best:vec3<f32>;
var<private> vmap_own_best_lit:bool;
// Mode 9's plane: the own plane closest to the receiver point within VMAP_FACE_PLAUS texels.
var<private> vmap_face_plane:vec3<f32>;
var<private> vmap_face_lit:bool;
var<private> vmap_clean:bool;
// The receiver level's texel (the own planes' reach).
var<private> vmap_texel0:f32;
// VMAP_COST counters: 2x2 cover tests and max-mip tests.
var<private> vmap_covers:u32;
var<private> vmap_mips:u32;
// The certain-umbra test decided the receiver (VMAP_TRACE 3).
var<private> vmap_umbra_hit:bool;
// The plane heights at light xy q under which a surface cannot block: x for any placement (the
// facet's tangent, when it faces the sun), y for the receiver's placement (also the own-surface
// planes, within VMAP_OWN_FAR texels of the receiver).
fn vmap_floors(q:vec2<f32>)->vec2<f32> {
    if !VMAP_TANGENT {return vec2<f32>(-3.4e38);}
    let d=q-vmap_r.point.xy;
    let facet=select(-3.4e38,vmap_r.point.z+dot(vmap_r.slope,d),vmap_r.facing);
    var own=facet;
    if dot(d,d)<=VMAP_OWN_FAR*VMAP_OWN_FAR*vmap_texel0*vmap_texel0 {
        own=max(max(facet,max(vmap_own0.x+dot(vmap_own0.yz,d),vmap_own1.x+dot(vmap_own1.yz,d))),
            max(vmap_own2.x+dot(vmap_own2.yz,d),vmap_own3.x+dot(vmap_own3.yz,d)));
    }
    return vec2<f32>(facet,own);
}

// The bilinear 2x2 texel centres around light xy `q` at `level`. A layer surface (top or
// peel) covers q when its height rebuilt there from its plane is at least `lo` (less `spread`
// per unit of its gradient, plus VMAP_CROSS texels when `spread` >= 0), and lies above the
// receiver's floors (vmap_floors) by more than its error. covered: the bilinear weight of
// centres with a covering surface; at: of those whose lowest covering surface is at most `hi`
// (plus the same slack), and own_at the share of it from the receiver's placement; next: the
// lowest covering surface above that, if any. The peel is read only under a covering top.
struct VmapCover {covered:f32,at:f32,own_at:f32,next:f32};
fn vmap_cover(level:u32,q:vec2<f32>,lo:f32,hi:f32,spread:f32)->VmapCover {
    if VMAP_COST {vmap_covers+=1u;}
    let texel=vsm_texel(level);
    let origin=vsm.levels[level].origin.xy;
    let side=i32(vsm.counts.y*VSM_PAGE);
    let x=(q-origin)/texel-vec2<f32>(0.5);
    let cell0=vec2<i32>(floor(x));
    let f=x-floor(x);
    let bias=f32(vsm.misc.z)*vsm.depth.z*5.960464477539063e-8;
    let cross=select(0.0,VMAP_CROSS*texel,spread>=0.0);
    let grow=max(spread,0.0);
    let floors=vmap_floors(q);
    let bits=vsm.misc.x;
    var out=VmapCover(0.0,0.0,0.0,3.4e38);
    for(var k=0u;k<4u;k+=1u) {
        let cell=cell0+vec2<i32>(i32(k&1u),i32(k>>1u));
        if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(side)) {continue;}
        let i=vsm_word(cell,level);
        let top=vsm_top_words[i];
        if top.x==0u {continue;}
        let w=select(1.0-f.x,f.x,(k&1u)!=0u)*select(1.0-f.y,f.y,(k&2u)!=0u);
        let off=q-(origin+(vec2<f32>(cell)+vec2<f32>(0.5))*texel);
        let planes=vmap_planes[i];
        let errors=unpack2x16float(vmap_errors[i]);
        var lowest=3.4e38;
        var slack=0.0;
        var own=false;
        for(var layer=0u;layer<select(1u,2u,VMAP_PEEL);layer+=1u) {
            var word=top;
            var packed=planes.x;
            var error=errors.x;
            if layer==1u {
                if lowest==3.4e38 {break;}
                word=vsm_peel_words[i];packed=planes.y;error=errors.y;
            }
            if word.x==0u || word.x==vmap_r.own {continue;}
            let g=select(vec2<f32>(0.0),vmap_gradient(packed),VMAP_PLANES);
            let height=vsm_height(word)+dot(g,off);
            let s=grow*length(g)+cross;
            let mine=word.x>>bits==vmap_r.placement;
            if vmap_far_mode && mine && distance(q,vmap_r.point.xy)<=VMAP_FAR_REACH*vmap_texel0 {continue;}
            let floor_q=select(floors.x,floors.y,mine);
            let uncertain=VMAP_ABS*texel+VMAP_CURVE*error*length(off)/texel+bias;
            if height<lo-s || height<=floor_q+uncertain {continue;}
            if height<lowest {lowest=height;slack=s;own=mine;}
        }
        if lowest<3.4e38 {
            out.covered+=w;
            if lowest<=hi+slack {out.at+=w;out.own_at+=select(0.0,w,own);} else {out.next=min(out.next,lowest);}
        }
    }
    return out;
}

// Whether the ray from light-space `o` rising with drift `d` (light xy per unit height) is
// blocked by a surface the map shows above its step from height `h0` to its sample at `h` (both
// above o) at `level`: a surface over the sample above the step's start is a candidate, and
// the ray rises to the lowest one, again and again, until it is at a surface that still covers
// it or nothing covers it any more. Rises past the level's reach are left to coarser levels.
// `first` is the candidate test at the sample, when the caller already has it.
// 0: open (no candidate); 1: blocked by a surface of the receiver's placement; 2: by
// another's; open after a candidate, 4: it passed beside or under it, 8: the rise left the
// level's reach, 12: an own surface near the receiver stayed over it.
fn vmap_blocked(level:u32,o:vec3<f32>,d:vec2<f32>,h:f32,first:VmapCover)->u32 {
    vmap_partial=0.0;
    if first.covered<VMAP_COVER {return 0u;}
    let cap=vsm_cap(level);
    let drift=length(d);
    var at=h;
    var rise=first.next-o.z;
    var own=false;
    var near=false;
    for(var i=0u;i<VMAP_REFINE;i+=1u) {
        if rise>cap {return 8u;}
        let spread=1.5*drift*abs(rise-at);
        at=rise;
        let c=vmap_cover(level,o.xy+d*at,o.z+at,o.z+at,spread);
        if c.covered<VMAP_COVER {return 4u;}
        own=c.at>0.0 && 2.0*c.own_at>=c.at;
        // Own surfaces close to the receiver (light xy) need VMAP_OWN_NEAR coverage there.
        near=own && distance(o.xy+d*at,vmap_r.point.xy)<=VMAP_OWN_REACH*vsm_texel(level);
        // Bits 4 and 5 (VMAP_TRACE 3): an own surface near the receiver, a partial coverage.
        let threshold=select(VMAP_COVER,select(VMAP_OWN_COVER,VMAP_OWN_NEAR,near),own);
        if c.at>=threshold+VMAP_SOFT {return select(2u,1u,own)|select(0u,16u,near)|select(32u,0u,c.at>=0.999);}
        if VMAP_SOFT>0.0 && c.at>threshold-VMAP_SOFT {vmap_partial=max(vmap_partial,(c.at-threshold+VMAP_SOFT)/(2.0*VMAP_SOFT));}
        rise=c.next-o.z;
    }
    // Still under a surface: it continues over the ray (N8b's continuity rule), except an own
    // surface close to the receiver, where the map cannot show it is one surface.
    if near {return 12u;}
    return select(2u,1u,own)|32u;
}

// virtual_shadow.wgsl's max-mip test: no top-layer texel centre within `radius` of `centre`
// at `level` is above `lower`. Conservative: false at window edges.
fn vsm_mip_base(level:u32,mip:u32)->u32 {
    let side=vsm.counts.y*VSM_PAGE;
    let s=side>>(mip-1u);
    return level*((side*side-1u)/3u)+(side*side-s*s)/3u;
}
@group(0) @binding(45) var<storage,read> vmap_minmips:array<f32>;
// The lowest top-layer centre within `radius` of `centre` at `level` (the min-mip cells
// covering it, so possibly over a little more), in metres (-3.4e38 when the footprint leaves
// the window).
fn vmap_mip_min(level:u32,centre:vec2<f32>,radius:f32)->f32 {
    let texel=vsm_texel(level);
    let side=vsm.counts.y*VSM_PAGE;
    let x=(centre-vsm.levels[level].origin.xy)/texel;
    let a=floor(x-vec2<f32>(radius/texel));let b=floor(x+vec2<f32>(radius/texel));
    if any(a<vec2<f32>(0.0)) || any(b>=vec2<f32>(f32(side))) {return -3.4e38;}
    let span=max(b.x-a.x,b.y-a.y)+1.0;
    let mip=u32(ceil(log2(span)));
    var h=0.0;
    if mip==0u {
        h=bitcast<f32>(vsm_top_words[(level*side+u32(a.y))*side+u32(a.x)].y);
    } else {
        let ia=vec2<u32>(a)>>vec2<u32>(mip);let ib=vec2<u32>(b)>>vec2<u32>(mip);
        let s=side>>mip;let base=vsm_mip_base(level,mip);
        h=min(min(vmap_minmips[base+ia.y*s+ia.x],vmap_minmips[base+ia.y*s+ib.x]),min(vmap_minmips[base+ib.y*s+ia.x],vmap_minmips[base+ib.y*s+ib.x]));
    }
    return fma(h,vsm.depth.z,vsm.depth.x);
}
// The highest top-layer centre within `radius` of `centre` at `level`, in metres (3.4e38 when
// the footprint leaves the window).
fn vmap_mip_max(level:u32,centre:vec2<f32>,radius:f32)->f32 {
    let texel=vsm_texel(level);
    let side=vsm.counts.y*VSM_PAGE;
    let x=(centre-vsm.levels[level].origin.xy)/texel;
    let a=floor(x-vec2<f32>(radius/texel));let b=floor(x+vec2<f32>(radius/texel));
    if any(a<vec2<f32>(0.0)) || any(b>=vec2<f32>(f32(side))) {return 3.4e38;}
    let span=max(b.x-a.x,b.y-a.y)+1.0;
    let mip=u32(ceil(log2(span)));
    var h=0.0;
    if mip==0u {
        h=bitcast<f32>(vsm_top_words[(level*side+u32(a.y))*side+u32(a.x)].y);
    } else {
        let ia=vec2<u32>(a)>>vec2<u32>(mip);let ib=vec2<u32>(b)>>vec2<u32>(mip);
        let s=side>>mip;let base=vsm_mip_base(level,mip);
        h=max(max(vsm_mips[base+ia.y*s+ia.x],vsm_mips[base+ia.y*s+ib.x]),max(vsm_mips[base+ib.y*s+ia.x],vsm_mips[base+ib.y*s+ib.x]));
    }
    return fma(h,vsm.depth.z,vsm.depth.x);
}
fn vsm_mip_clear(level:u32,centre:vec2<f32>,radius:f32,lower:f32)->bool {
    let texel=vsm_texel(level);
    let side=vsm.counts.y*VSM_PAGE;
    let x=(centre-vsm.levels[level].origin.xy)/texel;
    let a=floor(x-vec2<f32>(radius/texel));let b=floor(x+vec2<f32>(radius/texel));
    if any(a<vec2<f32>(0.0)) || any(b>=vec2<f32>(f32(side))) {return false;}
    let span=max(b.x-a.x,b.y-a.y)+1.0;
    let mip=u32(ceil(log2(span)));
    var h=0.0;
    if mip==0u {
        h=bitcast<f32>(vsm_top_words[(level*side+u32(a.y))*side+u32(a.x)].y);
    } else {
        let ia=vec2<u32>(a)>>vec2<u32>(mip);let ib=vec2<u32>(b)>>vec2<u32>(mip);
        let s=side>>mip;let base=vsm_mip_base(level,mip);
        h=max(max(vsm_mips[base+ia.y*s+ia.x],vsm_mips[base+ia.y*s+ib.x]),max(vsm_mips[base+ib.y*s+ia.x],vsm_mips[base+ib.y*s+ib.x]));
    }
    return fma(h,vsm.depth.z,vsm.depth.x)<=lower;
}

struct VmapPixel {valid:bool,world:vec3<f32>,geometric:vec3<f32>,epsilon:f32,thin:bool,level:u32,l:vec3<f32>};
// shadow_origin (scene.wgsl): opaque receivers stay on the viewed side; thin ones move toward
// the light.
fn vmap_origin(px:VmapPixel,toward_light:vec3<f32>)->vec3<f32> {
    let toward=select(lighting.eye-px.world,toward_light,px.thin);
    let normal=select(-px.geometric,px.geometric,dot(px.geometric,toward)>=0.0);
    return fma(normal,vec3<f32>(px.epsilon),px.world);
}
fn vmap_step(j:u32)->vec3<f32> {
    let d=vmap_dirs[j];
    return vec3<f32>(dot(d,vsm.u.xyz),dot(d,vsm.v.xyz),dot(d,vsm.l.xyz));
}
// One receiver's setup, as virtual_shadow.wgsl's vsm_prepare without the triangle read.
fn vmap_prepare(id:vec2<u32>)->VmapPixel {
    var out:VmapPixel;
    let size=textureDimensions(vsm_world);
    let p=vec2<i32>(id);
    let record=textureLoad(vsm_hit,p,0);
    let normal=textureLoad(vsm_normal,p,0);
    let light=lights[0];
    if record.x==0u || normal.w==0.0 || light.kind.z==0u {return out;}
    let world=textureLoad(vsm_world,p,0);
    // fresh_estimate's cascade coverage and light basis, so directions match Interactive.
    let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
    var layer=VSM_NONE;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;break;}
    }
    if layer==VSM_NONE {return out;}
    let view=shadow_views[layer];
    let l=-light.travel_outer.xyz;
    let rows=transpose(view.world_to_clip);let u=normalize(rows[0].xyz);let v=normalize(rows[1].xyz);
    let seed=vsm_hash(id.x+id.y*size.x);
    let shift=vec2<f32>(f32(seed>>8u),f32(vsm_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    let slope=view.texel.z;
    vmap_active=0xffu>>(8u-vsm.n8e.x);
    // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
    vmap_dirs[0]=vsm_direction(0u,shift,l,u,v,slope);vmap_dirs[1]=vsm_direction(1u,shift,l,u,v,slope);
    vmap_dirs[2]=vsm_direction(2u,shift,l,u,v,slope);vmap_dirs[3]=vsm_direction(3u,shift,l,u,v,slope);
    vmap_dirs[4]=vsm_direction(4u,shift,l,u,v,slope);vmap_dirs[5]=vsm_direction(5u,shift,l,u,v,slope);
    vmap_dirs[6]=vsm_direction(6u,shift,l,u,v,slope);vmap_dirs[7]=vsm_direction(7u,shift,l,u,v,slope);
    out.valid=true;
    out.world=world.xyz;
    out.geometric=normal.xyz;
    out.epsilon=world.w;
    out.thin=normal.w<0.0;
    out.level=textureLoad(vsm_level_in,p,0).x;
    out.l=l;
    // The receiver's map id: its placement + 1 and primitive (virtual_shadow.wgsl's
    // vsm_cluster_id). LOD receivers' primitives are not rasterized, so they never match.
    let slot=record.x-1u;
    vmap_r.placement=slot+1u;
    vmap_r.own=(vmap_r.placement<<vsm.misc.x)|record.y;
    // The tangent plane of the viewed side (the origin's side) in light space: a ray of a
    // direction above it stays above it, so no surface below it can block one.
    let toward=select(lighting.eye-world.xyz,l,out.thin);
    let n=select(-normal.xyz,normal.xyz,dot(normal.xyz,toward)>=0.0);
    let nl=vec3<f32>(dot(n,vsm.u.xyz),dot(n,vsm.v.xyz),dot(n,vsm.l.xyz));
    vmap_r.point=vsm_light(world.xyz);
    vmap_r.winding_lit=dot(normal.xyz,l)>0.0;
    vmap_r.facing=nl.z>0.05;
    vmap_r.slope=-nl.xy/max(nl.z,0.05);
    // A thin receiver's origin crosses to the far side for directions below its plane, whose
    // rays then do not stay above it.
    for(var j=0u;j<8u;j+=1u) {
        if out.thin && dot(n,vmap_dirs[j])<=0.0 {vmap_r.facing=false;}
    }
    return out;
}

fn vmap_own_store(k:u32,plane:vec3<f32>) {
    // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
    switch k {
        case 0u {vmap_own0=plane;} case 1u {vmap_own1=plane;}
        case 2u {vmap_own2=plane;} default {vmap_own3=plane;}
    }
}
// The receiver's own surface as the map rebuilds it at its light-space point p: per 2x2 centre
// around p at `level`, the receiver placement's layer surface whose height rebuilt at p is
// closest to the receiver's, if within VMAP_PLAUS of it. VMAP_OWN 1 keeps the closest of those,
// 2 all of them; their planes (height at p, gradient) floor same-placement surfaces, so the
// map's own sub-texel error cancels at the receiver instead of shadowing it.
fn vmap_own_surface(level:u32,p:vec2<f32>) {
    vmap_own_count=0u;
    vmap_own_match=0u;
    let none=vec3<f32>(-3.4e38,0.0,0.0);
    vmap_own0=none;vmap_own1=none;vmap_own2=none;vmap_own3=none;
    vmap_clean=false;
    if VMAP_OWN==0u {return;}
    var clean=true;
    var best_all=3.4e38;
    vmap_own_best_lit=false;
    var face_gap=3.4e38;
    vmap_face_lit=false;
    let texel=vsm_texel(level);
    vmap_texel0=texel;
    let origin=vsm.levels[level].origin.xy;
    let side=i32(vsm.counts.y*VSM_PAGE);
    let cell0=vec2<i32>(floor((p-origin)/texel-vec2<f32>(0.5)));
    let bits=vsm.misc.x;
    var best=3.4e38;
    for(var k=0u;k<4u;k+=1u) {
        let cell=cell0+vec2<i32>(i32(k&1u),i32(k>>1u));
        if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(side)) {continue;}
        let i=vsm_word(cell,level);
        let off=p-(origin+(vec2<f32>(cell)+vec2<f32>(0.5))*texel);
        let planes=vmap_planes[i];
        let errors=unpack2x16float(vmap_errors[i]);
        var gap=3.4e38;
        var plane=vec3<f32>(0.0);
        var top=false;
        var matched=false;
        var plane_lit=false;
        let empty=vsm_top_words[i].x==0u;
        for(var layer=0u;layer<2u;layer+=1u) {
            var word=vsm_top_words[i];var packed=planes.x;var error=errors.x;
            if layer==1u {word=vsm_peel_words[i];packed=planes.y;error=errors.y;}
            if word.x==0u || word.x>>bits!=vmap_r.placement {continue;}
            let g=vmap_gradient(packed);
            let height=vsm_height(word)+dot(g,off);
            var e=abs(height-vmap_r.point.z);
            if e<=VMAP_FACE_PLAUS*texel && e<face_gap {face_gap=e;vmap_face_plane=vec3<f32>(height,g);vmap_face_lit=(packed&1u)!=0u;}
            var bound=VMAP_PLAUS*texel*(1.0+length(g))+error;
            if VMAP_PLAUS_MODE==1u {bound=VMAP_PLAUS*(0.25*texel+length(g-vmap_r.slope)*length(off))+error;}
            if VMAP_PLAUS_MODE==2u {
                // Above the receiver by its plane at p, or by its centre over the facet's plane.
                e=max(min(height-vmap_r.point.z,vsm_height(word)-(vmap_r.point.z-dot(vmap_r.slope,off))),0.0);
                bound=VMAP_PLAUS*0.25*texel;
            }
            if e<gap && e<=bound {gap=e;plane=vec3<f32>(height,g);top=layer==0u;plane_lit=(packed&1u)!=0u;matched=plane_lit==vmap_r.winding_lit;}
        }
        // The cell is clean when its top is empty or is the own surface kept for it.
        clean=clean && (empty || (top && VMAP_OWN==2u && (matched || !VMAP_OWN_MATCH)));
        if gap==3.4e38 {continue;}
        if gap<best_all {best_all=gap;vmap_own_best=plane;vmap_own_best_lit=plane_lit;}
        // VMAP_OWN_MATCH: an own plane facing the sun otherwise than the receiver is kept for
        // the facing test only, never as a floor.
        if VMAP_OWN_MATCH && !matched {plane.x=-3.4e38;}
        if VMAP_OWN==1u {
            if gap<best {best=gap;vmap_own_store(0u,plane);vmap_own_count=1u;vmap_own_match=u32(matched);}
        } else {
            vmap_own_store(vmap_own_count,plane);
            vmap_own_match|=u32(matched)<<vmap_own_count;
            vmap_own_count+=1u;
        }
    }
    vmap_clean=clean;
}

// The march: heights above the origin through the receiver's level and every coarser one up
// to the highest caster (VMAP_SCHEDULE); per step the max-mip over the cone's footprint, then
// each open direction's sample. An opaque receiver's contact sample is computed once: its
// directions' samples all lie within a hair of its point. Returns the directions left open
// and, in bits 8-15, those the receiver's placement blocked; y: per open direction (bits j,
// 8+j, 16+j) whether a candidate was passed, left the reach, or stayed near (VMAP_TRACE 2); z:
// per blocked direction (bits j, 8+j) an own surface near the receiver, a partial coverage.
fn vmap_march(px:VmapPixel,open0:u32)->vec3<u32> {
    var open=open0;
    var own=0u;
    var why=0u;
    var amb=0u;
    let p=vsm_light(vmap_origin(px,px.l));
    let near=vsm.params.x;
    var drift=0.0;var offset=0.0;
    for(var j=0u;j<8u;j+=1u) {
        if (vmap_active&(1u<<j))!=0u {
            let s=vmap_step(j);
            drift=max(drift,length(s.xy)/max(s.z,1e-6));
            offset=max(offset,distance(vsm_light(vmap_origin(px,vmap_dirs[j])).xy,p.xy));
        }
    }
    var clear=max(vsm.depth.w-p.z,1.0001*near);
    let bias=f32(vsm.misc.z)*vsm.depth.z*5.960464477539063e-8;
    let samples=max(vsm.misc.y,1u);
    let level0=px.level;
    if VMAP_CEILING && VMAP_SCHEDULE==1u {
        // Every caster the cone can meet lies within its footprint at the ceiling; the level
        // whose reach holds the ceiling has its pages requested over that footprint (vsm_mark).
        for(var i=0u;i<2u;i+=1u) {
            var l=level0;
            while l+1u<vsm.counts.x && vsm_cap(l)<clear {l+=1u;}
            if VMAP_COST {vmap_mips+=1u;}
            let top=vmap_mip_max(l,p.xy,offset+drift*clear);
            if top>=3.0e38 {break;}
            clear=min(clear,max(top+bias-p.z,1.0001*near));
        }
    }
    // Schedule 1: where the cone's footprint reaches half a texel of the receiver's level.
    let half=max(0.5*vsm_texel(level0)/max(drift,1e-6),2.0*near);
    let clean=VMAP_CLEAN && vmap_clean && !px.thin;
    if VMAP_UMBRA && !clean && !px.thin {
        // Every ray stays within radius offset + drift h of p up to height h; if the top layer
        // is above it everywhere there (plus the bilinear support) up to the height where it
        // would meet it, each ray is under a surface until it reaches it: all are blocked. A
        // clean receiver lies on its own top surface, so the test cannot apply.
        let support=0.70711*vsm_texel(level0);
        let pad=VMAP_UMBRA_PAD*vsm_texel(level0)+bias;
        let rise=select(-3.4e38,length(vmap_r.slope),vmap_r.facing);
        let h0=vmap_mip_min(level0,p.xy,support);
        if VMAP_COST {vmap_mips+=1u;}
        if h0>p.z+pad && h0>vmap_r.point.z+rise*support+pad {
            let reach=h0-p.z;
            var l=level0;
            while l+1u<vsm.counts.x && vsm_cap(l)<reach {l+=1u;}
            let r1=offset+drift*reach+0.70711*vsm_texel(l);
            if VMAP_COST {vmap_mips+=1u;}
            let h1=vmap_mip_min(l,p.xy,r1);
            if h1>p.z+pad && h1>vmap_r.point.z+rise*r1+pad {vmap_umbra_hit=true;return vec3<u32>(0u,0u,0u);}
        }
    }
    var contact_cover=VmapCover(0.0,0.0,0.0,3.4e38);
    if !px.thin && !clean {contact_cover=vmap_cover(level0,p.xy,p.z,-3.4e38,-1.0);}
    var h=near;
    var previous=0.0;
    for(var level=level0;level<vsm.counts.x;level+=1u) {
        let end=min(max(vsm_cap(level),1.0001*near),clear);
        if end<=h {continue;}
        let custom=VMAP_SCHEDULE==1u && level==level0;
        let base=select(h,min(half,end),custom);
        var count=select(VMAP_EXTRA_SAMPLES,samples,level==level0);
        if custom && VMAP_CEILING {
            count=clamp(u32(ceil(log(max(end/base,1.0))/log(VMAP_RATIO)))+2u,3u,max(samples,3u));
        }
        let ratio=pow(end/base,1.0/f32(select(count,max(count,3u)-2u,custom)));
        for(var k=0u;k<count;k+=1u) {
            if custom && k==0u {
                h=near;
            } else if custom && k==1u {
                h=base;
            } else {
                h*=ratio;
            }
            let start=previous;
            previous=h;
            let contact=custom && k==0u && !px.thin;
            if clean && custom && k<2u {continue;}
            if contact && contact_cover.covered<VMAP_COVER {continue;}
            let radius=offset+drift*h;
            if !contact && VMAP_MIP {
                if VMAP_COST {vmap_mips+=1u;}
                if vsm_mip_clear(level,p.xy,radius,p.z+start+bias) {continue;}
            }
            for(var j=0u;j<8u;j+=1u) {
                if (open&(1u<<j))==0u {continue;}
                let s=vmap_step(j);
                let d=s.xy/s.z;
                var o=p;
                if px.thin {o=vsm_light(vmap_origin(px,vmap_dirs[j]));}
                var first=contact_cover;
                if !contact {first=vmap_cover(level,o.xy+d*h,o.z+start,-3.4e38,-1.0);}
                let cause=vmap_blocked(level,o,d,h,first);
                if VMAP_SOFT>0.0 && (cause&3u)==0u && vmap_partial>0.0 {vmap_soft_note(j,vmap_partial);}
                if (cause&3u)!=0u {open&=~(1u<<j);}
                if (cause&3u)==1u {own|=1u<<j;}
                if (cause&3u)==0u && cause>=4u {why|=1u<<(j+8u*((cause>>2u)-1u));}
                if (cause&16u)!=0u {amb|=1u<<j;}
                if (cause&32u)!=0u {amb|=1u<<(j+8u);}
            }
            if open==0u {return vec3<u32>(own<<8u,why,amb);}
            if h>=end {break;}
        }
        if h>=clear {break;}
    }
    return vec3<u32>(open|(own<<8u),why,amb);
}

fn vmap_visible(id:vec2<u32>,visible:u32) {
    let k=vsm.n8e.x;
    textureStore(vsm_output,vec2<i32>(id),vec4<u32>((256u*visible+k/2u)/k));
}
// virtual_shadow.wgsl's vsm_reached (N8f): whether this frame's moved casters can change pixel
// id's count under a kept camera.
fn vsm_reached(id:vec2<u32>)->bool {
    let c=vec2<f32>(id)+vec2<f32>(0.5);
    for(var i=0u;i<vsm.counts.w;i+=1u) {
        let s=vsm.srects[i];
        if all(c>=s.xy) && all(c<=s.zw) {return true;}
    }
    if textureLoad(vsm_hit,vec2<i32>(id),0).x==0u {return false;}
    let level=textureLoad(vsm_level_in,vec2<i32>(id),0).x;
    if level==VSM_NONE {return false;}
    let p=vsm_light(textureLoad(vsm_world,vec2<i32>(id),0).xyz);
    for(var l=level;l<vsm.counts.x;l+=1u) {
        let r=vec2<f32>(vsm_radius(l,p.z));
        for(var i=0u;i<vsm.counts.w;i+=1u) {
            let m=vsm.rects[i];
            if all(p.xy+r>=m.xy) && all(p.xy-r<=m.zw) {return true;}
        }
        if vsm_cap(l)>=vsm.depth.w-p.z {break;}
    }
    return false;
}
// One receiver's lookup: writes its count (or a screen's trace) and returns its hybrid queue
// word (ambiguous blocked directions | blocked directions << 8; 0: not queued).
fn vmap_pixel(id:vec2<u32>)->u32 {
    if VMAP_SKIP==3u {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));return 0u;}
    vmap_covers=0u;
    vmap_mips=0u;
    vmap_far_mode=false;
    vmap_soft0=0u;
    vmap_soft1=0u;
    let px=vmap_prepare(id);
    if !px.valid || VMAP_SKIP==1u {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));return 0u;}
    vmap_own_count=0u;
    if px.level!=VSM_NONE {vmap_own_surface(px.level,vmap_r.point.xy);}
    // Directions below the receiver's tangent plane enter its own surface (and, with
    // VMAP_FACING_OWN, below every own-surface plane: a light-space step s rises above a plane
    // of gradient g when s.z > g . s.xy).
    var facing=0u;
    var freeable=0u;
    if !px.thin {
        let toward=lighting.eye-px.world;
        let n=select(-px.geometric,px.geometric,dot(px.geometric,toward)>=0.0);
        for(var j=0u;j<8u;j+=1u) {
            if dot(n,vmap_dirs[j])>VMAP_FACING*length(n) {continue;}
            if VMAP_FACING_OWN!=0u && (vmap_own_count!=0u || VMAP_FACING_OWN==9u) {
                let s=vmap_step(j);
                let n=vmap_own_count;
                let a=vec4<f32>(s.z-dot(vmap_own0.yz,s.xy),select(-3.4e38,s.z-dot(vmap_own1.yz,s.xy),n>1u),
                    select(-3.4e38,s.z-dot(vmap_own2.yz,s.xy),n>2u),select(-3.4e38,s.z-dot(vmap_own3.yz,s.xy),n>3u));
                let mean=(vmap_own0.yz+select(vec2<f32>(0.0),vmap_own1.yz,n>1u)+select(vec2<f32>(0.0),vmap_own2.yz,n>2u)
                    +select(vec2<f32>(0.0),vmap_own3.yz,n>3u))/f32(n);
                let any_above=max(max(a.x,a.y),max(a.z,a.w))>0.0;
                let all_above=a.x>0.0 && (n<2u || a.y>0.0) && (n<3u || a.z>0.0) && (n<4u || a.w>0.0);
                var above=select(select(all_above,s.z>dot(mean,s.xy),VMAP_FACING_OWN==2u),any_above,VMAP_FACING_OWN==1u);
                if VMAP_FACING_OWN>=7u {
                    var g=vmap_own_best.yz;
                    var lit=vmap_own_best_lit;
                    if VMAP_FACING_OWN==9u {g=vmap_face_plane.yz;lit=vmap_face_lit;}
                    above=lit && s.z-dot(g,s.xy)>VMAP_FACE_MARGIN*sqrt(1.0+dot(g,g));
                    // Mode 8 decides after the march (freeable).
                    if VMAP_FACING_OWN==8u {
                        if above {freeable|=1u<<j;}
                        above=false;
                    }
                } else if VMAP_FACING_OWN>=4u {
                    let m=vmap_own_match;
                    let am=vec4<f32>(select(-3.4e38,a.x,(m&1u)!=0u),select(-3.4e38,a.y,(m&2u)!=0u),select(-3.4e38,a.z,(m&4u)!=0u),select(-3.4e38,a.w,(m&8u)!=0u));
                    let k=f32(countOneBits(m));
                    let gm=select(vec2<f32>(0.0),vmap_own0.yz,(m&1u)!=0u)+select(vec2<f32>(0.0),vmap_own1.yz,(m&2u)!=0u)
                        +select(vec2<f32>(0.0),vmap_own2.yz,(m&4u)!=0u)+select(vec2<f32>(0.0),vmap_own3.yz,(m&8u)!=0u);
                    above=select(m!=0u && s.z>dot(gm/max(k,1.0),s.xy),max(max(am.x,am.y),max(am.z,am.w))>0.0,VMAP_FACING_OWN!=5u);
                    if VMAP_FACING_OWN==6u {above=above && vmap_r.winding_lit && dot(px.geometric,toward)>=0.0;}
                }
                if above {continue;}
            }
            facing|=1u<<j;
        }
    }
    facing&=vmap_active;
    var blocked=facing;
    var own=0u;
    var why=0u;
    var amb=0u;
    vmap_umbra_hit=false;
    if VMAP_SKIP!=2u && px.level!=VSM_NONE && (vmap_active&~blocked)!=0u {
        let march=vmap_march(px,vmap_active&~blocked);
        own=march.x>>8u;
        blocked=vmap_active&~(march.x&0xffu);
        why=march.y;
        amb=march.z;
    }
    // Conditional freeing (mode 8, VMAP_LONE): only receivers whose other directions are open.
    freeable&=facing&vmap_active;
    if freeable!=0u && (vmap_active&~facing)!=0u && (blocked&~facing)==0u && px.level!=VSM_NONE {
        let again=vmap_march(px,freeable);
        let opened=freeable&(again.x&0xffu);
        blocked&=~opened;
        facing&=~opened;
    }
    if VMAP_LONE!=0u && facing==0u {
        let lone=blocked&select(amb&0xffu,(amb&0xffu)|((amb>>8u)&0xffu),VMAP_LONE_PARTIAL);
        if lone==blocked && countOneBits(lone)<=VMAP_LONE {blocked=0u;own=0u;}
    }
    let visible=countOneBits(vmap_active&~blocked);
    // The hybrid hands receivers with an ambiguous block to the exact walk: a facing block
    // within VMAP_AMB_FACING of the facet plane, a block by an own surface near the receiver,
    // or one at partial coverage (docs/non-rt-n8k.md sizes this set).
    var queued=0u;
    if vmap_hybrid() && VMAP_TRACE==0u && !VMAP_COST {
        var face=0u;
        if !px.thin {
            let toward=lighting.eye-px.world;
            let n=normalize(select(-px.geometric,px.geometric,dot(px.geometric,toward)>=0.0));
            for(var j=0u;j<8u;j+=1u) {
                if (facing&(1u<<j))!=0u && dot(n,vmap_dirs[j])>-VMAP_AMB_FACING {face|=1u<<j;}
            }
        }
        if VMAP_RING {
            let near=(face|(amb&0xffu))&vmap_active;
            if near!=0u && px.level!=VSM_NONE {
                vmap_far_mode=true;
                let far=vmap_march(px,near);
                vmap_far_mode=false;
                queued=near|(blocked<<8u)|((near&~(far.x&0xffu))<<16u);
            }
        } else {
            let flagged=face|(amb&0xffu)|((amb>>8u)&0xffu);
            if flagged!=0u {queued=flagged|(blocked<<8u);}
        }
    }
    if VMAP_COST {
        textureStore(vsm_output,vec2<i32>(id),vec4<u32>(min(vmap_covers,0xffffu)|min(vmap_mips,0xffffu)<<16u));
        return 0u;
    }
    if VMAP_TRACE==1u {
        // Visible directions (bits 0-3), and per direction what blocked it: the facing test
        // (4-11), a surface of the receiver's placement (12-19) or another's (20-27).
        textureStore(vsm_output,vec2<i32>(id),vec4<u32>(visible|facing<<4u|own<<12u|(blocked&~facing&~own)<<20u));
        return 0u;
    }
    if VMAP_TRACE==3u {
        // Visible directions (0-3); per direction, a facing block within VMAP_AMB_FACING of the
        // facet plane (4-11), a block by an own surface near the receiver (12-19), a block at
        // partial coverage (20-27); bit 28 the certain-umbra test decided it.
        var face=0u;
        if !px.thin {
            let toward=lighting.eye-px.world;
            let n=normalize(select(-px.geometric,px.geometric,dot(px.geometric,toward)>=0.0));
            for(var j=0u;j<8u;j+=1u) {
                if (facing&(1u<<j))!=0u && dot(n,vmap_dirs[j])>-VMAP_AMB_FACING {face|=1u<<j;}
            }
        }
        textureStore(vsm_output,vec2<i32>(id),vec4<u32>(visible|face<<4u|(amb&0xffu)<<12u|((amb>>8u)&0xffu)<<20u|u32(vmap_umbra_hit)<<28u));
        return 0u;
    }
    if VMAP_TRACE==2u {
        // Visible directions (bits 0-3), and per open direction whether it passed a candidate
        // (4-11), left a level's reach (12-19) or stayed under a near own surface (20-27);
        // none of these: no surface ever covered it. Bit 28: the clean-cell skip applied.
        let open=vmap_active&~blocked;
        textureStore(vsm_output,vec2<i32>(id),vec4<u32>(visible|(why&open)<<4u|((why>>8u)&open)<<12u|((why>>16u)&open)<<20u|u32(vmap_clean)<<28u));
        return 0u;
    }
    if VMAP_SOFT>0.0 {
        // Open directions count 1 minus their largest partial block.
        let open=vmap_active&~blocked;
        var sum=0.0;
        for(var j=0u;j<8u;j+=1u) {
            if (open&(1u<<j))!=0u {sum+=1.0-vmap_soft_of(j);}
        }
        let k=f32(vsm.n8e.x);
        textureStore(vsm_output,vec2<i32>(id),vec4<u32>(u32(round(256.0*sum/k))));
        return queued;
    }
    vmap_visible(id,visible);
    return queued;
}
// The hybrid's per-tile queue block.
var<workgroup> vmap_wg_count:atomic<u32>;
var<workgroup> vmap_wg_base:u32;
// The lookup pass, with virtual_shadow.wgsl's vsm_lookup schedule: on a rolling frame one
// trace tile of every refresh block (n8e.y), or on a casters-only frame (n8e.w) every tile's
// pixels the moved casters can reach; otherwise every tile. The hybrid appends each tile's
// queued receivers as one block, so the exact pass's waves hold neighbouring receivers.
@compute @workgroup_size(8,4) fn vmap_lookup(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let size=textureDimensions(vsm_world);
    let rolling=vsm.n8e.y;
    let block=vec2<u32>(rolling&15u,(rolling>>4u)&15u);
    let offset=vec2<u32>((rolling>>8u)&15u,(rolling>>12u)&15u);
    let casters=(vsm.n8e.w&1u)!=0u;
    var tile=group.xy;
    if (rolling>>16u)!=0u && !casters {tile=tile*block+offset;}
    let id=tile*VSM_TRACE_TILE+local.xy;
    var queued=0u;
    // No early return: the queue's barriers below need every invocation.
    if all(id<size) && textureLoad(vsm_tiles,tile,0).x!=0u && !(casters && any(tile%block!=offset) && !vsm_reached(id)) {
        queued=vmap_pixel(id);
    }
    if vmap_hybrid() {
        if li==0u {atomicStore(&vmap_wg_count,0u);}
        workgroupBarrier();
        var slot=0u;
        if queued!=0u {slot=atomicAdd(&vmap_wg_count,1u);}
        workgroupBarrier();
        if li==0u {
            let n=atomicLoad(&vmap_wg_count);
            vmap_wg_base=select(0u,atomicAdd(&vmap_queue[0],n),n!=0u);
        }
        let base=workgroupUniformLoad(&vmap_wg_base);
        if queued!=0u {
            atomicStore(&vmap_queue[4u+2u*(base+slot)],id.x|(id.y<<16u));
            atomicStore(&vmap_queue[5u+2u*(base+slot)],queued);
        }
    }
}
