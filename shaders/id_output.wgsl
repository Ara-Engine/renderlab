// ---- Entity-ID output (docs/design-id-output.md, src/id_output.rs) ----
// One pass after the frame's opaque surfaces, temporal reconstruction and the glass raster:
// each output pixel's settled opaque surface and its frontmost surface of any kind, looked
// up in the per-placement id ranges (by the hit's primitive) as the embedder's 32-bit ids.
// 0 is no surface; 0xFFFF_FFFF a surface without an id.
//
// Settled surface: on reconstructed frames, the surface of temporal reconstruction's two
// slots (shaders/temporal_resolve.wgsl) that covers more of the pixel, so jitter never
// decides it. The previous frame's choice is kept while it is still one of the pixel's two
// surfaces and either nothing there moved (camera and both surfaces still) or it still
// covers at least half minus ID_MARGIN: a still image never flips, and a moving edge flips a
// pixel once. Other frames (no reconstruction: unjittered) read this frame's hit record.
//
// The slots name placements, not primitives. A still camera keeps the previous frame's
// primitive of an unchanged placement. Otherwise the placement's ids are voted over this
// frame's 3x3 samples showing it, with the previous frame's id kept until another leads by
// two samples: entities merged into one object hold still and flip once at a moving edge.

struct IdFrame {
    // Output size in pixels.
    size:vec2<u32>,
    // ID_* bits.
    flags:u32,
    // Opaque placements in the table; transparent draws' instances follow them.
    opaque:u32,
    // The trace tile size of the hit record's coverage table.
    tile:vec2<u32>,
    pad:vec2<u32>,
};
const ID_RECONSTRUCTED:u32=1u;
const ID_STILL_CAMERA:u32=2u;
const ID_PREVIOUS:u32=4u;
const ID_GLASS:u32=8u;
// Temporal reconstruction's slot words (temporal_resolve.wgsl TR1_ID, TR1_NONE).
const ID_SLOT:u32=0x7fffffu;
const ID_NONE_SLOT:u32=0xffffffffu;
// A surface without an id, and a primitive the pass does not know.
const ID_UNTAGGED:u32=0xffffffffu;
const ID_ANY:u32=0xffffffffu;
// The hysteresis of a moving pixel's choice, in coverage.
const ID_MARGIN:f32=0.125;

@group(0) @binding(0) var<uniform> id_frame:IdFrame;
// The frame's hit record (placement + 1, primitive, barycentrics) and trace tile coverage,
// on the internal grid.
@group(0) @binding(1) var id_hit:texture_2d<u32>;
@group(0) @binding(2) var id_tiles:texture_2d<u32>;
// Temporal reconstruction's written anchors and silhouette slots (reconstructed frames).
@group(0) @binding(3) var id_anchors:texture_2d<u32>;
@group(0) @binding(4) var id_layers:texture_2d<u32>;
// Per opaque placement, then per transparent instance: (first range, range count).
@group(0) @binding(5) var<storage,read> id_entries:array<vec2<u32>>;
// One bit per opaque placement: its transform or vertices changed since the last frame.
@group(0) @binding(6) var<storage,read> id_moved:array<u32>;
// The glass raster's nearest pane: transparent instance + 1, 0 for none.
@group(0) @binding(7) var id_glass:texture_2d<u32>;
// The previous frame's settled (placement + 1, primitive), and this frame's.
@group(0) @binding(8) var id_previous:texture_2d<u32>;
@group(0) @binding(9) var id_settled:texture_storage_2d<rg32uint,write>;
// Layer 0: opaque id; layer 1: front id.
@group(0) @binding(10) var id_out:texture_storage_2d_array<r32uint,write>;
// Sorted, disjoint (first primitive, count, id, 0) per entry.
@group(0) @binding(11) var<storage,read> id_ranges:array<vec4<u32>>;

// The id of `primitive` (ID_ANY: the entry's first range) of table entry `entry`.
fn id_lookup(entry:u32,primitive:u32)->u32 {
    let e=id_entries[entry];
    if e.y==0u {return ID_UNTAGGED;}
    if primitive==ID_ANY {return id_ranges[e.x].z;}
    // The last range starting at or before the primitive; halving bounds the loop.
    var lo=e.x;var hi=e.x+e.y;
    for(var k=0u;k<32u && hi-lo>1u;k+=1u) {
        let mid=(lo+hi)/2u;
        if id_ranges[mid].x<=primitive {lo=mid;} else {hi=mid;}
    }
    let r=id_ranges[lo];
    if primitive>=r.x && primitive-r.x<r.y {return r.z;}
    return ID_UNTAGGED;
}
fn id_tag(settled:vec2<u32>)->u32 {
    if settled.x==0u {return 0u;}
    if settled.x>id_frame.opaque {return ID_UNTAGGED;}
    return id_lookup(settled.x-1u,settled.y);
}
fn id_has_moved(placement:u32)->bool {
    if placement==0u || placement>id_frame.opaque {return false;}
    let i=placement-1u;
    return (id_moved[i/32u]&(1u<<(i%32u)))!=0u;
}
// The hit record at an internal pixel, as (placement + 1, primitive); uncovered trace tiles
// keep stale records and are background.
fn id_record(q:vec2<i32>)->vec2<u32> {
    let size=vec2<i32>(textureDimensions(id_hit));
    if any(q<vec2<i32>(0)) || any(q>=size) {return vec2<u32>(ID_NONE_SLOT,0u);}
    if textureLoad(id_tiles,vec2<u32>(q)/id_frame.tile,0).x==0u {return vec2<u32>(0u);}
    return textureLoad(id_hit,q,0).xy;
}

// The settled opaque placement + 1 at output pixel p.
fn id_settle(p:vec2<u32>,here:vec2<u32>)->u32 {
    if (id_frame.flags&ID_RECONSTRUCTED)==0u {return here.x;}
    let a=textureLoad(id_anchors,p,0).x&ID_SLOT;
    let l=textureLoad(id_layers,p,0);
    if l.x==ID_NONE_SLOT {return a;}
    let b=l.x&ID_SLOT;
    // The first slot's share of the pixel.
    let coverage=unpack2x16float(l.w).x;
    var choice=select(b,a,coverage>=0.5);
    if (id_frame.flags&ID_PREVIOUS)!=0u {
        let q=textureLoad(id_previous,p,0).x;
        if q==a || q==b {
            let share=select(1.0-coverage,coverage,q==a);
            let still=(id_frame.flags&ID_STILL_CAMERA)!=0u && !id_has_moved(a) && !id_has_moved(b);
            if still || share>=0.5-ID_MARGIN {choice=q;}
        }
    }
    return choice;
}

@compute @workgroup_size(8,8) fn id_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=id_frame.size) {return;}
    let p=id.xy;
    // The nearest internal sample (the output pixel itself at native resolution, decided
    // from the integer sizes).
    let internal=textureDimensions(id_hit);
    var q=vec2<i32>(p);
    if any(internal!=id_frame.size) {
        q=min(vec2<i32>((vec2<f32>(p)+vec2<f32>(0.5))*vec2<f32>(internal)/vec2<f32>(id_frame.size)),vec2<i32>(internal)-vec2<i32>(1));
    }
    let here=id_record(q);
    let placement=id_settle(p,here);
    let previous=textureLoad(id_previous,p,0).xy;
    var primitive=ID_ANY;
    let keep=(id_frame.flags&ID_PREVIOUS)!=0u && previous.x==placement;
    if keep && (id_frame.flags&ID_STILL_CAMERA)!=0u && !id_has_moved(placement) {
        primitive=previous.y;
    } else {
        // One placement can hold several entities (merged objects): its id follows its
        // primitive, which only this frame's jittered samples know. Vote by id over the 3x3
        // samples showing the placement: the previous frame's id stays until another id
        // leads it by two samples, so an edge between two entities of one object flips a
        // pixel once as it crosses.
        let previous_id=select(ID_UNTAGGED,id_lookup(placement-1u,previous.y),keep && placement!=0u && placement<=id_frame.opaque);
        var candidate=select(ID_ANY,here.y,here.x==placement);
        var candidate_id=0u;
        if candidate!=ID_ANY {candidate_id=id_tag(vec2<u32>(placement,candidate));}
        var kept=0u;var rival=0u;
        for(var i=0;i<9;i+=1) {
            let r=id_record(q+vec2<i32>(i%3,i/3)-vec2<i32>(1));
            if r.x!=placement {continue;}
            if candidate==ID_ANY {candidate=r.y;candidate_id=id_tag(vec2<u32>(placement,r.y));}
            let id=id_tag(vec2<u32>(placement,r.y));
            if keep && id==previous_id {kept+=1u;}
            if id==candidate_id {rival+=1u;}
        }
        primitive=candidate;
        if keep && (candidate==ID_ANY || candidate_id==previous_id || (kept>0u && rival<kept+2u)) {
            primitive=previous.y;
        }
    }
    let settled=vec2<u32>(placement,primitive);
    textureStore(id_settled,p,vec4<u32>(settled,0u,0u));
    let opaque=id_tag(settled);
    var front=opaque;
    if (id_frame.flags&ID_GLASS)!=0u {
        let g=textureLoad(id_glass,p,0).x;
        // Raster glass knows its instance, not its primitive: the instance's first range.
        if g!=0u {front=id_lookup(id_frame.opaque+g-1u,ID_ANY);}
    }
    textureStore(id_out,p,0,vec4<u32>(opaque,0u,0u,0u));
    textureStore(id_out,p,1,vec4<u32>(front,0u,0u,0u));
}

// ---- Handoff: one id layer, packed bit-exact into an Rgba8Unorm target ----
// R = bits 0-7 ... A = bits 24-31. Each byte k is written as (k + 0.25) / 255: Vulkan's float
// to unorm8 conversion need not round to nearest, and k / 255 can land a hair under k, which
// a truncating conversion stores as k - 1; a quarter step stores k under either rule.
@group(0) @binding(0) var handoff_ids:texture_2d_array<u32>;
// x: the layer. A vec4 is the 16 bytes the layout binds (a struct of u32 and vec3<u32> is 32).
@group(0) @binding(2) var<uniform> handoff:vec4<u32>;
@vertex fn handoff_vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn handoff_ids_fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    let id=textureLoad(handoff_ids,vec2<i32>(p.xy),i32(handoff.x),0).x;
    let bytes=vec4<f32>(vec4<u32>(id&0xffu,(id>>8u)&0xffu,(id>>16u)&0xffu,id>>24u));
    return (bytes+vec4<f32>(0.25))/255.0;
}
// ---- end of entity-ID output ----
